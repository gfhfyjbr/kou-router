use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::{
    header::{HeaderName, HeaderValue, AUTHORIZATION},
    multipart, Client, Method, RequestBuilder, StatusCode,
};
use serde_json::{json, Value};

use crate::{
    error::AppResult,
    models::{AudioTranscriptionPayload, EndpointKind, ProviderConnection, ProviderChatAttempt},
    search::{build_search_request, normalize_search_response, SearchHttpMethod},
};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

/// Headers from the incoming request that should be passed through to upstream.
/// Primarily for Anthropic-specific headers that Claude Code sends.
#[derive(Debug, Clone, Default)]
pub struct PassthroughHeaders {
    pub headers: Vec<(String, String)>,
}

impl PassthroughHeaders {
    /// Extract passthrough headers from an incoming axum request's HeaderMap.
    /// Captures: anthropic-beta, anthropic-version, x-client-request-id
    pub fn from_header_map(headers: &axum::http::HeaderMap) -> Self {
        const PASSTHROUGH_NAMES: &[&str] = &[
            "anthropic-beta",
            "anthropic-version",
            "x-client-request-id",
        ];
        let mut extracted = Vec::new();
        for &name in PASSTHROUGH_NAMES {
            if let Some(value) = headers.get(name) {
                if let Ok(v) = value.to_str() {
                    extracted.push((name.to_string(), v.to_string()));
                }
            }
        }
        Self { headers: extracted }
    }
}

/// Result of upstream execution — either buffered or streaming
pub enum UpstreamResult {
    /// Non-streaming or error response (fully buffered)
    Buffered(ProviderResponse),
    /// True SSE stream relay
    Streaming(StreamingProviderResponse),
}

pub struct StreamingProviderResponse {
    pub status: StatusCode,
    pub provider_id: String,
    pub model: String,
    pub stream: BoxStream,
}

pub fn tee_stream(
    stream: BoxStream,
) -> (
    Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
    Arc<Mutex<Vec<u8>>>,
) {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = buffer.clone();
    let teed = stream.map(move |result| match result {
        Ok(bytes) => {
            if let Ok(mut buf) = buf_clone.lock() {
                buf.extend_from_slice(&bytes);
            }
            Ok(bytes)
        }
        Err(e) => Err(Box::new(e) as BoxError),
    });
    (Box::pin(teed), buffer)
}

#[derive(Clone)]
pub struct UpstreamClient {
    client: Client,
}

impl UpstreamClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn execute(
        &self,
        provider: &ProviderConnection,
        endpoint: EndpointKind,
        suffix: Option<&str>,
        model: &str,
        payload: &Value,
        inject_model: bool,
        passthrough_headers: Option<&PassthroughHeaders>,
    ) -> AppResult<UpstreamResult> {
        let stream_requested = payload
            .get("stream")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let path = endpoint_path(provider, endpoint, suffix, stream_requested);
        let url = build_url(&provider.base_url, &path);
        let mut request_body = payload.clone();
        if inject_model {
            request_body["model"] = Value::String(model.to_string());
        }

        if endpoint == EndpointKind::Search {
            let spec = build_search_request(provider, &request_body, &url)?;
            let builder = match spec.method {
                SearchHttpMethod::Get => self.client.request(Method::GET, spec.url),
                SearchHttpMethod::Post => {
                    let builder = self.client.request(Method::POST, spec.url);
                    if let Some(body) = spec.body {
                        builder.json(&body)
                    } else {
                        builder
                    }
                }
            };
            let builder = apply_passthrough_headers(builder, passthrough_headers);
            let response = apply_provider_headers(builder, provider).send().await?;
            let status = response.status();
            let body = response.text().await?;
            let body = if (200..300).contains(&status.as_u16()) {
                let query = request_body
                    .get("query")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let search_type = request_body
                    .get("search_type")
                    .or_else(|| request_body.get("provider"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("web");
                let json_body: Value = serde_json::from_str(&body)?;
                serde_json::to_string(&normalize_search_response(&provider.provider, query, search_type, json_body))?
            } else {
                body
            };
            Ok(UpstreamResult::Buffered(ProviderResponse {
                status,
                body,
                is_stream: false,
            }))
        } else {
            let builder = self.client.request(Method::POST, url).json(&request_body);
            let builder = apply_passthrough_headers(builder, passthrough_headers);
            let response = apply_provider_headers(builder, provider).send().await?;
            let status = response.status();
            let is_stream = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("text/event-stream"))
                .unwrap_or(false);

            if is_stream && status.is_success() {
                // True streaming — relay chunks directly to client
                Ok(UpstreamResult::Streaming(StreamingProviderResponse {
                    status,
                    provider_id: provider.id.clone(),
                    model: model.to_string(),
                    stream: Box::pin(response.bytes_stream()),
                }))
            } else if is_stream {
                // Error response on stream request — buffer all chunks
                let mut stream = response.bytes_stream();
                let mut body = String::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    body.push_str(&String::from_utf8_lossy(&chunk));
                }
                Ok(UpstreamResult::Buffered(ProviderResponse {
                    status,
                    body,
                    is_stream: true,
                }))
            } else {
                let body = response.text().await?;
                Ok(UpstreamResult::Buffered(ProviderResponse {
                    status,
                    body,
                    is_stream: false,
                }))
            }
        }
    }

    pub async fn execute_audio_speech(
        &self,
        provider: &ProviderConnection,
        model: &str,
        payload: &Value,
    ) -> AppResult<AudioResponse> {
        let path = endpoint_path(provider, EndpointKind::AudioSpeech, None, false);
        let url = build_url(&provider.base_url, &path);
        let mut request_body = payload.clone();
        request_body["model"] = Value::String(model.to_string());

        let builder = self.client.request(Method::POST, url).json(&request_body);
        let response = apply_provider_headers(builder, provider).send().await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = response.bytes().await?.to_vec();

        Ok(AudioResponse {
            status,
            content_type,
            bytes,
        })
    }

    pub async fn execute_audio_transcription(
        &self,
        provider: &ProviderConnection,
        model: &str,
        payload: &AudioTranscriptionPayload,
    ) -> AppResult<AudioResponse> {
        let path = endpoint_path(provider, EndpointKind::AudioTranscriptions, None, false);
        let url = build_url(&provider.base_url, &path);

        let file_part = multipart::Part::bytes(payload.bytes.clone())
            .file_name(payload.filename.clone())
            .mime_str(payload.content_type.as_deref().unwrap_or("application/octet-stream"))?;
        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", model.to_string());
        for (key, value) in &payload.text_fields {
            if key != "model" && key != "file" {
                form = form.text(key.clone(), value.clone());
            }
        }

        let builder = self.client.request(Method::POST, url).multipart(form);
        let response = apply_provider_headers(builder, provider).send().await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();
        let bytes = response.bytes().await?.to_vec();

        Ok(AudioResponse {
            status,
            content_type,
            bytes,
        })
    }
}

pub struct ProviderResponse {
    pub status: StatusCode,
    pub body: String,
    pub is_stream: bool,
}

impl ProviderResponse {
    pub fn as_attempt(self, provider_id: String, model: String) -> ProviderChatAttempt {
        ProviderChatAttempt {
            provider_id,
            model,
            status: self.status.as_u16(),
            body: self.body,
        }
    }
}

pub struct AudioResponse {
    pub status: StatusCode,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl AudioResponse {
    pub fn as_attempt(&self, provider_id: String, model: String) -> ProviderChatAttempt {
        ProviderChatAttempt {
            provider_id,
            model,
            status: self.status.as_u16(),
            body: self.body_preview(),
        }
    }

    pub fn body_preview(&self) -> String {
        String::from_utf8_lossy(&self.bytes).chars().take(4000).collect()
    }
}

pub fn fallback_error(status: StatusCode, body: &str) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || body.to_ascii_lowercase().contains("rate limit")
        || body.to_ascii_lowercase().contains("quota")
}

pub fn openai_error(message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": "upstream_error"
        }
    })
}

fn apply_passthrough_headers(mut builder: RequestBuilder, headers: Option<&PassthroughHeaders>) -> RequestBuilder {
    if let Some(pt) = headers {
        for (name, value) in &pt.headers {
            if let (Ok(hn), Ok(hv)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                builder = builder.header(hn, hv);
            }
        }
    }
    builder
}

fn apply_provider_headers(mut builder: RequestBuilder, provider: &ProviderConnection) -> RequestBuilder {
    if let Some(api_key) = &provider.api_key {
        let header = provider.auth_header.trim().to_ascii_lowercase();
        match header.as_str() {
            "none" => {}
            "bearer" | "authorization" => {
                let prefix = provider.auth_prefix.as_deref().unwrap_or("Bearer");
                builder = builder.header(AUTHORIZATION, format!("{prefix} {api_key}"));
            }
            "token" => {
                builder = builder.header(AUTHORIZATION, format!("Token {api_key}"));
            }
            "basic" => {
                builder = builder.header(AUTHORIZATION, format!("Basic {api_key}"));
            }
            "x-api-key" | "xi-api-key" | "x-goog-api-key" | "x-subscription-token" => {
                builder = builder.header(header.as_str(), api_key);
            }
            other => {
                let name = HeaderName::from_bytes(other.as_bytes());
                if let Ok(name) = name {
                    let value = provider
                        .auth_prefix
                        .as_deref()
                        .map(|prefix| format!("{prefix} {api_key}"))
                        .unwrap_or_else(|| api_key.clone());
                    if let Ok(value) = HeaderValue::from_str(&value) {
                        builder = builder.header(name, value);
                    }
                }
            }
        }
    }

    for (key, value) in &provider.extra_headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            builder = builder.header(name, value);
        }
    }

    builder
}

fn endpoint_path(
    provider: &ProviderConnection,
    endpoint: EndpointKind,
    suffix: Option<&str>,
    stream_requested: bool,
) -> String {
    if stream_requested {
        if let Some(path) = provider_path_override(&provider.stream_endpoint_paths, endpoint, suffix) {
            return path;
        }
    }
    if let Some(path) = provider_path_override(&provider.endpoint_paths, endpoint, suffix) {
        return path;
    }

    let default_path = match endpoint {
        EndpointKind::ChatCompletions | EndpointKind::Completions => "/chat/completions".to_string(),
        EndpointKind::Messages => "/messages".to_string(),
        EndpointKind::Responses => "/responses".to_string(),
        EndpointKind::OllamaChat => "/api/chat".to_string(),
        EndpointKind::Embeddings => "/embeddings".to_string(),
        EndpointKind::ImagesGenerations => "/images/generations".to_string(),
        EndpointKind::MusicGenerations => "/music/generations".to_string(),
        EndpointKind::VideosGenerations => "/videos/generations".to_string(),
        EndpointKind::Moderations => "/moderations".to_string(),
        EndpointKind::Rerank => "/rerank".to_string(),
        EndpointKind::Search => "/search".to_string(),
        EndpointKind::AudioSpeech => "/audio/speech".to_string(),
        EndpointKind::AudioTranscriptions => "/audio/transcriptions".to_string(),
    };
    append_suffix_if_needed(endpoint, default_path, suffix)
}

fn provider_path_override(
    overrides: &std::collections::BTreeMap<String, String>,
    endpoint: EndpointKind,
    suffix: Option<&str>,
) -> Option<String> {
    for key in [Some(endpoint.as_str()), Some(endpoint.capability()), chat_family_override_key(endpoint)]
        .into_iter()
        .flatten()
    {
        if let Some(path) = overrides.get(key) {
            return Some(append_suffix_if_needed(endpoint, path.clone(), suffix));
        }
    }
    None
}

fn chat_family_override_key(endpoint: EndpointKind) -> Option<&'static str> {
    if endpoint.is_chat_family() {
        Some("chat")
    } else {
        None
    }
}

fn append_suffix_if_needed(endpoint: EndpointKind, base_path: String, suffix: Option<&str>) -> String {
    if endpoint == EndpointKind::Responses {
        return if base_path.contains("{suffix}") {
            base_path.replace("{suffix}", suffix.unwrap_or(""))
        } else {
            append_path_suffix(&base_path, suffix.unwrap_or(""))
        };
    }
    base_path
}


fn append_path_suffix(base_path: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return base_path.to_string();
    }
    if suffix.starts_with('/') {
        format!("{base_path}{suffix}")
    } else {
        format!("{base_path}/{suffix}")
    }
}

fn build_url(base_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", base_url.trim_end_matches('/'), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use serde_json::json;
    use crate::models::EndpointKind;
    use std::collections::BTreeMap;

    fn make_provider() -> crate::models::ProviderConnection {
        crate::models::ProviderConnection {
            id: String::new(),
            provider: "test".to_string(),
            base_url: "https://api.test.com".to_string(),
            api_key: Some("sk-test-key".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: BTreeMap::new(),
            stream_endpoint_paths: BTreeMap::new(),
            model_prefix: "test".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: None,
            supported_endpoints: vec![],
            rate_limit_protection: false,
            last_error: None,
            last_error_at: None,
            last_error_type: None,
            last_error_source: None,
            rate_limited_until: None,
            circuit_open_until: None,
            last_used_at: None,
            backoff_level: 0,
            consecutive_use_count: 0,
            test_status: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            protocol_format: None,
        }
    }

    #[test]
    fn test_endpoint_path_chat_default() {
        let p = make_provider();
        assert_eq!(endpoint_path(&p, EndpointKind::ChatCompletions, None, false), "/chat/completions");
    }

    #[test]
    fn test_endpoint_path_embeddings_default() {
        let p = make_provider();
        assert_eq!(endpoint_path(&p, EndpointKind::Embeddings, None, false), "/embeddings");
    }

    #[test]
    fn test_endpoint_path_messages_default() {
        let p = make_provider();
        assert_eq!(endpoint_path(&p, EndpointKind::Messages, None, false), "/messages");
    }

    #[test]
    fn test_endpoint_path_all_defaults() {
        let p = make_provider();
        let cases: Vec<(EndpointKind, &str)> = vec![
            (EndpointKind::ChatCompletions, "/chat/completions"),
            (EndpointKind::Completions, "/chat/completions"),
            (EndpointKind::Messages, "/messages"),
            (EndpointKind::Responses, "/responses"),
            (EndpointKind::OllamaChat, "/api/chat"),
            (EndpointKind::Embeddings, "/embeddings"),
            (EndpointKind::ImagesGenerations, "/images/generations"),
            (EndpointKind::MusicGenerations, "/music/generations"),
            (EndpointKind::VideosGenerations, "/videos/generations"),
            (EndpointKind::Moderations, "/moderations"),
            (EndpointKind::Rerank, "/rerank"),
            (EndpointKind::Search, "/search"),
            (EndpointKind::AudioSpeech, "/audio/speech"),
            (EndpointKind::AudioTranscriptions, "/audio/transcriptions"),
        ];
        for (kind, expected) in cases {
            assert_eq!(endpoint_path(&p, kind, None, false), expected, "failed for {:?}", kind);
        }
    }

    #[test]
    fn test_endpoint_path_override() {
        let mut p = make_provider();
        p.endpoint_paths.insert("chat.completions".to_string(), "/v1/custom".to_string());
        assert_eq!(endpoint_path(&p, EndpointKind::ChatCompletions, None, false), "/v1/custom");
    }

    #[test]
    fn test_endpoint_path_stream_override() {
        let mut p = make_provider();
        p.stream_endpoint_paths.insert("chat.completions".to_string(), "/v1/stream-chat".to_string());
        // stream_requested=true should use the stream override
        assert_eq!(endpoint_path(&p, EndpointKind::ChatCompletions, None, true), "/v1/stream-chat");
        // stream_requested=false should fall through to default
        assert_eq!(endpoint_path(&p, EndpointKind::ChatCompletions, None, false), "/chat/completions");
    }

    #[test]
    fn test_endpoint_path_responses_suffix() {
        let p = make_provider();
        assert_eq!(endpoint_path(&p, EndpointKind::Responses, Some("native/path"), false), "/responses/native/path");
    }

    #[test]
    fn test_endpoint_path_responses_placeholder_suffix() {
        let mut p = make_provider();
        p.endpoint_paths.insert("responses".to_string(), "/v1/responses/{suffix}".to_string());
        assert_eq!(endpoint_path(&p, EndpointKind::Responses, Some("abc"), false), "/v1/responses/abc");
        // Empty suffix replaces placeholder with empty string
        assert_eq!(endpoint_path(&p, EndpointKind::Responses, None, false), "/v1/responses/");
    }

    #[test]
    fn test_build_url_relative() {
        assert_eq!(build_url("https://api.test.com", "/chat/completions"), "https://api.test.com/chat/completions");
    }

    #[test]
    fn test_build_url_absolute() {
        assert_eq!(build_url("https://api.test.com", "http://other.com/path"), "http://other.com/path");
        assert_eq!(build_url("https://api.test.com", "https://other.com/path"), "https://other.com/path");
    }

    #[test]
    fn test_fallback_error_429() {
        assert!(fallback_error(StatusCode::TOO_MANY_REQUESTS, ""));
    }

    #[test]
    fn test_fallback_error_500() {
        assert!(fallback_error(StatusCode::INTERNAL_SERVER_ERROR, ""));
    }

    #[test]
    fn test_fallback_error_502() {
        assert!(fallback_error(StatusCode::BAD_GATEWAY, ""));
    }

    #[test]
    fn test_fallback_error_rate_limit_body() {
        assert!(fallback_error(StatusCode::BAD_REQUEST, "Hit rate limit on this endpoint"));
    }

    #[test]
    fn test_fallback_error_quota_body() {
        assert!(fallback_error(StatusCode::BAD_REQUEST, "You have exceeded your quota"));
    }

    #[test]
    fn test_fallback_error_400() {
        assert!(!fallback_error(StatusCode::BAD_REQUEST, "invalid request body"));
    }

    #[test]
    fn test_openai_error_format() {
        let err = openai_error("something went wrong");
        assert_eq!(err["error"]["message"], "something went wrong");
        assert_eq!(err["error"]["type"], "upstream_error");
    }
}