use std::{collections::HashMap, pin::Pin, sync::Arc};

use bytes::Bytes;
use tokio::sync::Mutex;

use crate::{
    error::{AppError, AppResult},
    models::{
        supports_endpoint, ComboStrategy, EndpointKind, NormalizedRequest, OpenAiModelsResponse,
        ProviderChatAttempt, RoutingDebug, SettingsPayload,
    },
    repository::SqliteRepository,
    upstream::{fallback_error, tee_stream, BoxError, PassthroughHeaders, UpstreamClient, UpstreamResult},
    translate::{ProtocolFormat, TranslatorRegistry},
};

#[derive(Clone)]
pub struct RouterService {
    repository: Arc<SqliteRepository>,
    upstream: UpstreamClient,
    round_robin: Arc<Mutex<HashMap<String, usize>>>,
    translator: Arc<TranslatorRegistry>,
}

pub struct RoutedResponse {
    pub body: serde_json::Value,
    pub debug: RoutingDebug,
    pub is_stream: bool,
}

pub enum RoutedResult {
    /// Buffered JSON response
    Json(RoutedResponse),
    /// True SSE streaming relay
    Stream(RoutedStream),
}

pub struct RoutedStream {
    pub stream: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, BoxError>> + Send>>,
    pub debug: RoutingDebug,
    pub buffer: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl RouterService {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        Self {
            repository,
            upstream: UpstreamClient::new(),
            round_robin: Arc::new(Mutex::new(HashMap::new())),
            translator: Arc::new(TranslatorRegistry::new()),
        }
    }

    pub async fn list_models(&self) -> AppResult<OpenAiModelsResponse> {
        Ok(OpenAiModelsResponse {
            object: "list".to_string(),
            data: self.repository.get_openai_models_catalog().await?,
        })
    }

    pub async fn list_models_for_endpoint(
        &self,
        endpoint: EndpointKind,
    ) -> AppResult<OpenAiModelsResponse> {
        Ok(OpenAiModelsResponse {
            object: "list".to_string(),
            data: self.repository.get_models_catalog_for_endpoint(endpoint).await?,
        })
    }

    pub async fn get_settings(&self) -> AppResult<serde_json::Value> {
        self.repository.get_settings().await
    }

    pub async fn put_settings(&self, payload: SettingsPayload) -> AppResult<serde_json::Value> {
        self.repository.put_settings(&payload).await
    }

    pub fn normalize_audio_speech(
        &self,
        payload: serde_json::Value,
    ) -> AppResult<NormalizedRequest> {
        normalize_request(EndpointKind::AudioSpeech, payload)
    }

    pub async fn route(
        &self,
        endpoint: EndpointKind,
        payload: serde_json::Value,
        suffix: Option<String>,
        passthrough_headers: Option<PassthroughHeaders>,
    ) -> AppResult<RoutedResult> {
        let normalized = normalize_request(endpoint, payload)?;
        let resolved_model = self.repository.resolve_alias(&normalized.model).await?;
        let candidate_models = self.expand_candidates(endpoint, &resolved_model).await?;
        let providers = self.repository.list_provider_connections().await?;
        let now = chrono::Utc::now();
        let providers: Vec<_> = providers
            .into_iter()
            .filter(|provider| {
                provider.enabled
                    && provider
                        .rate_limited_until
                        .map(|until| until < now)
                        .unwrap_or(true)
                    && provider
                        .circuit_open_until
                        .map(|until| until < now)
                        .unwrap_or(true)
                    && supports_endpoint(&provider.supported_endpoints, endpoint)
            })
            .collect();
        if providers.is_empty() {
            return Err(AppError::BadRequest(format!(
                "no enabled providers configured for {}",
                endpoint.as_str()
            )));
        }

        let mut tried = Vec::new();
        for candidate in candidate_models {
            let (prefix, raw_model) = split_model(&candidate)?;
            let mut matching: Vec<_> = if prefix.is_empty() {
                // Bare model name: match providers by model name heuristics
                find_providers_for_bare_model(&providers, &raw_model)
            } else {
                providers
                    .iter()
                    .filter(|provider| provider.model_prefix == prefix || provider.provider == prefix)
                    .cloned()
                    .collect()
            };
            matching.sort_by_key(|provider| provider.priority);

            if matching.is_empty() {
                tried.push(crate::models::ProviderChatAttempt {
                    provider_id: format!("unresolved:{}", if prefix.is_empty() { &raw_model } else { &prefix }),
                    model: raw_model.clone(),
                    status: 404,
                    body: format!("no provider found for model prefix and endpoint {}", endpoint.as_str()),
                });
                continue;
            }

            for provider in matching {
                // Protocol translation: detect formats
                let source_format = ProtocolFormat::detect_source(endpoint, &normalized.body);
                let target_format = ProtocolFormat::from_provider(&provider);

                // Translate request body if needed
                let translated_body = if source_format != target_format {
                    self.translator.translate_request(
                        source_format,
                        target_format,
                        &raw_model,
                        &normalized.body,
                        normalized.stream,
                    )?
                } else {
                    normalized.body.clone()
                };

                let result = self
                    .upstream
                    .execute(
                        &provider,
                        endpoint,
                        suffix.as_deref(),
                        &raw_model,
                        &translated_body,
                        normalized.inject_model,
                        passthrough_headers.as_ref(),
                    )
                    .await?;

                match result {
                    UpstreamResult::Streaming(streaming) => {
                        // 2xx streaming — relay immediately, no fallback possible
                        self.repository.mark_provider_success(&provider.id).await?;
                        tried.push(ProviderChatAttempt {
                            provider_id: provider.id.clone(),
                            model: candidate.clone(),
                            status: streaming.status.as_u16(),
                            body: "[streaming]".to_string(),
                        });
                        let (teed, buffer) = tee_stream(streaming.stream);
                        return Ok(RoutedResult::Stream(RoutedStream {
                            stream: teed,
                            debug: RoutingDebug {
                                requested_model: normalized.model,
                                resolved_model,
                                endpoint: endpoint.as_str().to_string(),
                                path_suffix: suffix,
                                tried,
                            },
                            buffer,
                        }));
                    }
                    UpstreamResult::Buffered(response) => {
                        let is_stream = response.is_stream;
                        let attempt = response.as_attempt(provider.id.clone(), candidate.clone());
                        if (200..300).contains(&attempt.status) {
                            self.repository.mark_provider_success(&provider.id).await?;

                            let body = if source_format == target_format {
                                // Same protocol — direct passthrough, no translation roundtrip
                                if is_stream || normalized.stream {
                                    serde_json::json!({
                                        "object": "stream.capture",
                                        "stream": true,
                                        "raw": &attempt.body,
                                    })
                                } else {
                                    serde_json::from_str(&attempt.body)
                                        .unwrap_or_else(|_| serde_json::json!({"raw": &attempt.body}))
                                }
                            } else {
                                // Different protocols — translate then adapt
                                let body_for_adapt = if target_format != ProtocolFormat::OpenAI {
                                    let parsed: serde_json::Value = serde_json::from_str(&attempt.body)
                                        .unwrap_or_else(|_| serde_json::json!({"raw": &attempt.body}));
                                    let translated = self.translator.translate_response(
                                        target_format,
                                        ProtocolFormat::OpenAI,
                                        &parsed,
                                    )?;
                                    serde_json::to_string(&translated)?
                                } else {
                                    attempt.body.clone()
                                };
                                adapt_success_body(endpoint, &body_for_adapt, normalized.stream, is_stream)?
                            };

                            tried.push(attempt);
                            return Ok(RoutedResult::Json(RoutedResponse {
                                body,
                                debug: RoutingDebug {
                                    requested_model: normalized.model,
                                    resolved_model,
                                    endpoint: endpoint.as_str().to_string(),
                                    path_suffix: suffix,
                                    tried,
                                },
                                is_stream,
                            }));
                        }

                        self.repository.mark_provider_failure(&provider.id, &attempt.body).await?;
                        let should_fallback = fallback_error(
                            reqwest::StatusCode::from_u16(attempt.status)
                                .unwrap_or(reqwest::StatusCode::BAD_GATEWAY),
                            &attempt.body,
                        );
                        tried.push(attempt);
                        if !should_fallback {
                            break;
                        }
                    }
                }
            }
        }

        Err(AppError::Upstream(format!(
            "all providers/combo targets failed for {}",
            endpoint.as_str()
        )))
    }

    async fn expand_candidates(
        &self,
        endpoint: EndpointKind,
        resolved_model: &str,
    ) -> AppResult<Vec<String>> {
        if !endpoint.is_chat_family() {
            return Ok(vec![resolved_model.to_string()]);
        }

        let Some(combo) = self.repository.find_combo_by_name(resolved_model).await? else {
            return Ok(vec![resolved_model.to_string()]);
        };
        if !combo.enabled {
            return Err(AppError::BadRequest(format!("combo {} is disabled", combo.name)));
        }

        Ok(match combo.strategy {
            ComboStrategy::Priority => combo.models,
            ComboStrategy::RoundRobin => {
                let picked = self.pick_round_robin(&combo.name, &combo.models).await?;
                let mut models = vec![picked.clone()];
                models.extend(combo.models.into_iter().filter(|m| m != &picked));
                models
            }
        })
    }

    async fn pick_round_robin(&self, combo_name: &str, models: &[String]) -> AppResult<String> {
        if models.is_empty() {
            return Err(AppError::BadRequest(
                "combo must include at least one model".into(),
            ));
        }
        let mut rr = self.round_robin.lock().await;
        let next = rr.entry(combo_name.to_string()).or_insert(0);
        let idx = *next % models.len();
        *next = next.wrapping_add(1);
        Ok(models[idx].clone())
    }
}

fn normalize_request(
    endpoint: EndpointKind,
    mut payload: serde_json::Value,
) -> AppResult<NormalizedRequest> {
    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

    match endpoint {
        EndpointKind::Completions => {
            if object.get("messages").is_none() {
                let prompt = object
                    .remove("prompt")
                    .ok_or_else(|| AppError::BadRequest("missing prompt".into()))?;
                let prompt_text = match prompt {
                    serde_json::Value::Array(values) => values
                        .into_iter()
                        .map(|value| value.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    serde_json::Value::String(text) => text,
                    other => other.to_string(),
                };
                object.insert(
                    "messages".to_string(),
                    serde_json::json!([{"role": "user", "content": prompt_text}]),
                );
            }
        }
        EndpointKind::Messages => {
            if object.get("messages").is_none() {
                if let Some(input) = object.get("input").cloned() {
                    object.insert("messages".to_string(), input);
                }
            }
            object
                .entry("stream".to_string())
                .or_insert(serde_json::Value::Bool(false));
        }
        EndpointKind::Responses => {
            if object.get("input").is_some() && object.get("messages").is_none() {
                let input = object.get("input").cloned().unwrap_or_default();
                let messages = match input {
                    serde_json::Value::String(text) => {
                        serde_json::json!([{"role": "user", "content": text}])
                    }
                    serde_json::Value::Array(items) => serde_json::Value::Array(
                        items
                            .into_iter()
                            .map(|item| {
                                if item.get("role").is_some() {
                                    item
                                } else {
                                    serde_json::json!({"role": "user", "content": item})
                                }
                            })
                            .collect(),
                    ),
                    other => serde_json::json!([{"role": "user", "content": other}]),
                };
                object.insert("messages".to_string(), messages);
            }
        }
        EndpointKind::OllamaChat => {
            if object.get("messages").is_none() {
                return Err(AppError::BadRequest(
                    "ollama chat requires messages".into(),
                ));
            }
        }
        EndpointKind::Embeddings => {
            if object.get("input").is_none() {
                return Err(AppError::BadRequest(
                    "embeddings request requires input".into(),
                ));
            }
        }
        EndpointKind::ImagesGenerations => {
            if object.get("prompt").is_none() {
                return Err(AppError::BadRequest(
                    "image generation request requires prompt".into(),
                ));
            }
        }
        EndpointKind::MusicGenerations => {
            if object.get("prompt").is_none() {
                return Err(AppError::BadRequest(
                    "music generation request requires prompt".into(),
                ));
            }
        }
        EndpointKind::VideosGenerations => {
            if object.get("prompt").is_none() {
                return Err(AppError::BadRequest(
                    "video generation request requires prompt".into(),
                ));
            }
        }
        EndpointKind::Moderations => {
            if object.get("input").is_none() {
                return Err(AppError::BadRequest(
                    "moderation request requires input".into(),
                ));
            }
            object
                .entry("model".to_string())
                .or_insert(serde_json::Value::String("openai/omni-moderation-latest".to_string()));
        }
        EndpointKind::Rerank => {
            if object.get("query").is_none() {
                return Err(AppError::BadRequest(
                    "rerank request requires query".into(),
                ));
            }
            let documents = object
                .get("documents")
                .and_then(|value| value.as_array())
                .ok_or_else(|| AppError::BadRequest("rerank request requires documents array".into()))?;
            if documents.is_empty() {
                return Err(AppError::BadRequest(
                    "rerank request requires at least one document".into(),
                ));
            }
        }
        EndpointKind::Search => {
            if object.get("query").is_none() {
                return Err(AppError::BadRequest(
                    "search request requires query".into(),
                ));
            }
            object
                .entry("model".to_string())
                .or_insert(serde_json::Value::String("search/web".to_string()));
            object
                .entry("provider".to_string())
                .or_insert(serde_json::Value::String("web".to_string()));
            let search_type_default = object
                .get("provider")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::String("web".to_string()));
            object
                .entry("search_type".to_string())
                .or_insert(search_type_default);
        }
        EndpointKind::AudioSpeech => {
            if object.get("input").is_none() {
                return Err(AppError::BadRequest(
                    "audio speech request requires input".into(),
                ));
            }
            object
                .entry("voice".to_string())
                .or_insert(serde_json::Value::String("alloy".to_string()));
        }
        EndpointKind::AudioTranscriptions => {
            return Err(AppError::BadRequest(
                "audio transcriptions require multipart/form-data".into(),
            ));
        }
        EndpointKind::ChatCompletions => {}
    }

    let model = object
        .get("model")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::BadRequest("missing model".into()))?
        .to_string();
    let stream = object
        .get("stream")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let inject_model = !matches!(endpoint, EndpointKind::Search);
    Ok(NormalizedRequest {
        model,
        stream,
        body: payload,
        inject_model,
    })
}

fn adapt_success_body(
    endpoint: EndpointKind,
    body: &str,
    stream_requested: bool,
    upstream_stream: bool,
) -> AppResult<serde_json::Value> {
    if upstream_stream || stream_requested {
        return Ok(serde_json::json!({
            "object": "stream.capture",
            "stream": true,
            "raw": body,
        }));
    }

    let json_body: serde_json::Value = serde_json::from_str(body)?;
    match endpoint {
        EndpointKind::Responses => Ok(adapt_to_responses(json_body)),
        EndpointKind::Messages => Ok(adapt_to_messages(json_body)),
        EndpointKind::OllamaChat => Ok(adapt_to_ollama(json_body)),
        EndpointKind::ChatCompletions
        | EndpointKind::Completions
        | EndpointKind::Embeddings
        | EndpointKind::ImagesGenerations
        | EndpointKind::MusicGenerations
        | EndpointKind::VideosGenerations
        | EndpointKind::Moderations
        | EndpointKind::Rerank
        | EndpointKind::Search => Ok(json_body),
        EndpointKind::AudioSpeech | EndpointKind::AudioTranscriptions => Err(AppError::BadRequest(
            "audio endpoints should not use JSON success adapter".into(),
        )),
    }
}

fn adapt_to_responses(body: serde_json::Value) -> serde_json::Value {
    if body.get("output").is_some() {
        return body;
    }
    let text = extract_assistant_text(&body);
    serde_json::json!({
        "id": body.get("id").cloned().unwrap_or_else(|| serde_json::json!("resp_kou_router")),
        "object": "response",
        "model": body.get("model").cloned().unwrap_or_else(|| serde_json::json!(null)),
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}]
        }],
        "raw_openai_chat": body,
    })
}

fn adapt_to_messages(body: serde_json::Value) -> serde_json::Value {
    let text = extract_assistant_text(&body);
    serde_json::json!({
        "id": body.get("id").cloned().unwrap_or_else(|| serde_json::json!("msg_kou_router")),
        "type": "message",
        "role": "assistant",
        "model": body.get("model").cloned().unwrap_or_else(|| serde_json::json!(null)),
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "raw_openai_chat": body,
    })
}

fn adapt_to_ollama(body: serde_json::Value) -> serde_json::Value {
    let text = extract_assistant_text(&body);
    serde_json::json!({
        "model": body.get("model").cloned().unwrap_or_else(|| serde_json::json!("unknown")),
        "message": {"role": "assistant", "content": text},
        "done": true,
        "raw_openai_chat": body,
    })
}

fn extract_assistant_text(body: &serde_json::Value) -> String {
    body.get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(ToString::to_string)
        .unwrap_or_default()
}

fn split_model(value: &str) -> AppResult<(String, String)> {
    if let Some((prefix, model)) = value.split_once('/') {
        if prefix.is_empty() || model.is_empty() {
            return Err(AppError::BadRequest(format!("invalid model identifier: {value}")));
        }
        Ok((prefix.to_string(), model.to_string()))
    } else {
        // Bare model name (no prefix) — e.g. "claude-sonnet-4-20250514"
        // Return empty prefix; the caller will match by provider name/model heuristics
        Ok((String::new(), value.to_string()))
    }
}

/// Find matching providers for a bare model name (no prefix).
/// Matches by: known model name prefixes (claude- → anthropic, gpt- → openai, etc.),
/// or by provider's default_model containing the model name.
fn find_providers_for_bare_model(
    providers: &[crate::models::ProviderConnection],
    model: &str,
) -> Vec<crate::models::ProviderConnection> {
    let inferred_provider = infer_provider_from_model_name(model);

    let mut matching: Vec<_> = providers
        .iter()
        .filter(|p| {
            // Match by inferred provider name
            if let Some(ref inferred) = inferred_provider {
                if p.provider == *inferred || p.model_prefix == *inferred {
                    return true;
                }
            }
            // Match by default_model containing the bare model name
            if let Some(ref default) = p.default_model {
                if default == model || default.ends_with(&format!("/{model}")) {
                    return true;
                }
            }
            false
        })
        .cloned()
        .collect();

    // If no match by heuristics, try all providers (let upstream reject if wrong)
    if matching.is_empty() && inferred_provider.is_none() {
        matching = providers.to_vec();
    }

    matching
}

/// Infer provider name from well-known model name prefixes.
fn infer_provider_from_model_name(model: &str) -> Option<String> {
    let prefixes: &[(&str, &str)] = &[
        ("claude-", "anthropic"),
        ("gpt-", "openai"),
        ("o1-", "openai"),
        ("o3-", "openai"),
        ("o4-", "openai"),
        ("chatgpt-", "openai"),
        ("gemini-", "gemini"),
        ("gemma-", "gemini"),
        ("llama-", "meta"),
        ("mistral-", "mistral"),
        ("codestral-", "mistral"),
        ("command-", "cohere"),
        ("deepseek-", "deepseek"),
    ];
    for (prefix, provider) in prefixes {
        if model.starts_with(prefix) {
            return Some(provider.to_string());
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::models::EndpointKind;

    // --- normalize_request ---

    #[test]
    fn test_normalize_chat_completions() {
        let payload = json!({"model": "openai/gpt-4", "messages": [{"role": "user", "content": "hi"}]});
        let result = normalize_request(EndpointKind::ChatCompletions, payload).unwrap();
        assert_eq!(result.model, "openai/gpt-4");
        assert!(!result.stream);
        assert!(result.inject_model);
        assert!(result.body.get("messages").is_some());
    }

    #[test]
    fn test_normalize_completions_prompt_to_messages() {
        let payload = json!({"model": "openai/gpt-4", "prompt": "hello"});
        let result = normalize_request(EndpointKind::Completions, payload).unwrap();
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hello");
    }

    #[test]
    fn test_normalize_completions_array_prompt() {
        let payload = json!({"model": "openai/gpt-4", "prompt": ["a", "b"]});
        let result = normalize_request(EndpointKind::Completions, payload).unwrap();
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["content"], "a\nb");
    }

    #[test]
    fn test_normalize_messages_input_to_messages() {
        let input = json!([{"role": "user", "content": "hi"}]);
        let payload = json!({"model": "anthropic/claude", "input": input});
        let result = normalize_request(EndpointKind::Messages, payload).unwrap();
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn test_normalize_responses_input_string() {
        let payload = json!({"model": "openai/gpt-4", "input": "hello"});
        let result = normalize_request(EndpointKind::Responses, payload).unwrap();
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hello");
    }

    #[test]
    fn test_normalize_responses_input_array() {
        let payload = json!({"model": "openai/gpt-4", "input": [
            {"role": "user", "content": "hi"},
            "plain text"
        ]});
        let result = normalize_request(EndpointKind::Responses, payload).unwrap();
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hi");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "plain text");
    }

    #[test]
    fn test_normalize_embeddings_requires_input() {
        let payload = json!({"model": "openai/embed"});
        assert!(normalize_request(EndpointKind::Embeddings, payload).is_err());
    }

    #[test]
    fn test_normalize_images_requires_prompt() {
        let payload = json!({"model": "openai/dall-e"});
        assert!(normalize_request(EndpointKind::ImagesGenerations, payload).is_err());
    }

    #[test]
    fn test_normalize_music_requires_prompt() {
        let payload = json!({"model": "suno/music"});
        assert!(normalize_request(EndpointKind::MusicGenerations, payload).is_err());
    }

    #[test]
    fn test_normalize_video_requires_prompt() {
        let payload = json!({"model": "runway/gen"});
        assert!(normalize_request(EndpointKind::VideosGenerations, payload).is_err());
    }

    #[test]
    fn test_normalize_moderations_default_model() {
        let payload = json!({"input": "test text"});
        let result = normalize_request(EndpointKind::Moderations, payload).unwrap();
        assert_eq!(result.model, "openai/omni-moderation-latest");
    }

    #[test]
    fn test_normalize_moderations_missing_input() {
        let payload = json!({"model": "openai/mod"});
        assert!(normalize_request(EndpointKind::Moderations, payload).is_err());
    }

    #[test]
    fn test_normalize_rerank_requires_query_and_docs() {
        // Missing query
        let payload = json!({"model": "cohere/rerank", "documents": ["a"]});
        assert!(normalize_request(EndpointKind::Rerank, payload).is_err());

        // Missing documents
        let payload = json!({"model": "cohere/rerank", "query": "q"});
        assert!(normalize_request(EndpointKind::Rerank, payload).is_err());

        // Empty documents
        let payload = json!({"model": "cohere/rerank", "query": "q", "documents": []});
        assert!(normalize_request(EndpointKind::Rerank, payload).is_err());
    }

    #[test]
    fn test_normalize_search_defaults() {
        let payload = json!({"query": "rust async"});
        let result = normalize_request(EndpointKind::Search, payload).unwrap();
        assert_eq!(result.model, "search/web");
        assert_eq!(result.body["provider"], "web");
        assert_eq!(result.body["search_type"], "web");
        assert!(!result.inject_model);
    }

    #[test]
    fn test_normalize_search_missing_query() {
        let payload = json!({"model": "search/web"});
        assert!(normalize_request(EndpointKind::Search, payload).is_err());
    }

    #[test]
    fn test_normalize_audio_speech_requires_input() {
        let payload = json!({"model": "openai/tts"});
        assert!(normalize_request(EndpointKind::AudioSpeech, payload).is_err());
    }

    #[test]
    fn test_normalize_audio_speech_default_voice() {
        let payload = json!({"model": "openai/tts", "input": "hello"});
        let result = normalize_request(EndpointKind::AudioSpeech, payload).unwrap();
        assert_eq!(result.body["voice"], "alloy");
    }

    #[test]
    fn test_normalize_audio_transcriptions_error() {
        let payload = json!({"model": "openai/whisper"});
        assert!(normalize_request(EndpointKind::AudioTranscriptions, payload).is_err());
    }

    #[test]
    fn test_normalize_missing_model() {
        let payload = json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(normalize_request(EndpointKind::ChatCompletions, payload).is_err());
    }

    #[test]
    fn test_normalize_invalid_body() {
        let payload = serde_json::Value::String("not an object".into());
        assert!(normalize_request(EndpointKind::ChatCompletions, payload).is_err());
    }

    #[test]
    fn test_normalize_ollama_requires_messages() {
        let payload = json!({"model": "ollama/llama"});
        assert!(normalize_request(EndpointKind::OllamaChat, payload).is_err());
    }

    // --- split_model ---

    #[test]
    fn test_split_model_valid() {
        let (provider, model) = split_model("provider/model").unwrap();
        assert_eq!(provider, "provider");
        assert_eq!(model, "model");
    }

    #[test]
    fn test_split_model_no_slash() {
        // Bare model names now return empty prefix instead of error
        let (provider, model) = split_model("claude-sonnet-4-20250514").unwrap();
        assert_eq!(provider, "");
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_split_model_empty_parts() {
        assert!(split_model("/model").is_err());
        assert!(split_model("provider/").is_err());
    }

    #[test]
    fn test_split_model_bare_gpt() {
        let (provider, model) = split_model("gpt-4o").unwrap();
        assert_eq!(provider, "");
        assert_eq!(model, "gpt-4o");
    }

    // --- infer_provider_from_model_name ---

    #[test]
    fn test_infer_provider_claude() {
        assert_eq!(infer_provider_from_model_name("claude-sonnet-4-20250514"), Some("anthropic".to_string()));
    }

    #[test]
    fn test_infer_provider_gpt() {
        assert_eq!(infer_provider_from_model_name("gpt-4o"), Some("openai".to_string()));
    }

    #[test]
    fn test_infer_provider_unknown() {
        assert_eq!(infer_provider_from_model_name("some-random-model"), None);
    }

    // --- adapt / extract ---

    #[test]
    fn test_adapt_to_responses() {
        let body = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{"message": {"role": "assistant", "content": "Hello!"}}]
        });
        let result = adapt_to_responses(body);
        assert_eq!(result["object"], "response");
        let output = result["output"].as_array().unwrap();
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["role"], "assistant");
        let content = output[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "output_text");
        assert_eq!(content[0]["text"], "Hello!");
    }

    #[test]
    fn test_adapt_to_responses_native() {
        let body = json!({"output": [{"type": "message"}], "id": "resp_123"});
        let result = adapt_to_responses(body.clone());
        assert_eq!(result, body);
    }

    #[test]
    fn test_adapt_to_messages() {
        let body = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{"message": {"role": "assistant", "content": "Hello!"}}]
        });
        let result = adapt_to_messages(body);
        assert_eq!(result["type"], "message");
        assert_eq!(result["role"], "assistant");
        let content = result["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Hello!");
    }

    #[test]
    fn test_adapt_to_ollama() {
        let body = json!({
            "model": "gpt-4",
            "choices": [{"message": {"role": "assistant", "content": "Hello!"}}]
        });
        let result = adapt_to_ollama(body);
        assert_eq!(result["model"], "gpt-4");
        assert_eq!(result["message"]["role"], "assistant");
        assert_eq!(result["message"]["content"], "Hello!");
        assert_eq!(result["done"], true);
    }

    #[test]
    fn test_extract_assistant_text() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "extracted text"}}]
        });
        assert_eq!(extract_assistant_text(&body), "extracted text");

        // Missing content yields empty string
        assert_eq!(extract_assistant_text(&json!({"choices": []})), "");
        assert_eq!(extract_assistant_text(&json!({})), "");
    }
}