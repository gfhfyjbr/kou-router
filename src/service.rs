use std::{collections::HashMap, pin::Pin, sync::Arc};

use bytes::Bytes;
use chrono::{Duration, Utc};
use futures_util::StreamExt;
use tokio::sync::Mutex;

use crate::{
    error::{AppError, AppResult},
    fingerprint::{ClaudeCodeFingerprint, OAuthContext, ProviderKind},
    models::{
        ComboStrategy, EndpointKind, NewRequestDebugLog, NewResponseDebugLog, NormalizedRequest,
        OpenAiModelsResponse, ProviderAccount, ProviderAccountAuthMode,
        ProviderAccountRoutingStrategy, ProviderChatAttempt, RoutingDebug, RoutingProviderAccount,
        SettingsPayload, supports_endpoint,
    },
    oauth::OAuthService,
    proxy::ProxyClientCache,
    ratelimit::{RateLimitTracker, parse_rate_limit_headers},
    repository::SqliteRepository,
    retry::{RetryConfig, execute_with_retry},
    translate::{ProtocolFormat, TranslatorRegistry},
    upstream::{
        BoxError, PassthroughHeaders, UpstreamClient, UpstreamResult, fallback_error,
        prepare_upstream_request, provider_with_account_auth, tee_stream_boxerror, watchdog_stream,
    },
};

fn persist_request_debug_log(repository: Arc<SqliteRepository>, debug_log: NewRequestDebugLog) {
    tokio::spawn(async move {
        if let Err(err) = repository.insert_request_debug_log(debug_log).await {
            tracing::warn!(error = %err, "failed to persist request debug log");
        }
    });
}

fn persist_response_debug_log(repository: Arc<SqliteRepository>, debug_log: NewResponseDebugLog) {
    tokio::spawn(async move {
        if let Err(err) = repository.insert_response_debug_log(debug_log).await {
            tracing::warn!(error = %err, "failed to persist response debug log");
        }
    });
}

fn persist_codex_response_debug_log(
    repository: Arc<SqliteRepository>,
    request_id: String,
    provider_id: String,
    provider_account_id: Option<String>,
    model: String,
    sequence_no: i64,
    upstream_status: u16,
    raw_body: String,
    body: &serde_json::Value,
    endpoint: EndpointKind,
    provider_is_codex: bool,
) {
    let debug_log = build_response_debug_log(
        request_id,
        provider_id,
        provider_account_id,
        model,
        sequence_no,
        i64::from(upstream_status),
        raw_body,
        body,
        endpoint,
        provider_is_codex,
    );
    persist_response_debug_log(repository, debug_log);
}

fn persist_codex_stream_buffer_debug_log(
    repository: Arc<SqliteRepository>,
    request_id: String,
    provider_id: String,
    provider_account_id: Option<String>,
    model: String,
    sequence_no: i64,
    upstream_status: u16,
    buffer: Arc<std::sync::Mutex<Vec<u8>>>,
    endpoint: EndpointKind,
    provider_is_codex: bool,
) {
    tokio::spawn(async move {
        let mut last_len = None;
        let mut stable_polls = 0_u8;
        let mut raw_body = String::new();

        for _ in 0..240 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let snapshot = match buffer.lock() {
                Ok(buf) if !buf.is_empty() => String::from_utf8_lossy(&buf).to_string(),
                Ok(_) => continue,
                Err(_) => return,
            };
            if last_len == Some(snapshot.len()) {
                stable_polls += 1;
            } else {
                stable_polls = 0;
                last_len = Some(snapshot.len());
            }
            raw_body = snapshot;
            if stable_polls >= 2 {
                break;
            }
        }

        if raw_body.is_empty() {
            return;
        }

        let body = if is_sse_transcript(&raw_body) {
            reconstruct_responses_from_sse(&raw_body)
                .unwrap_or_else(|_| serde_json::json!({"raw": raw_body}))
        } else {
            serde_json::from_str(&raw_body).unwrap_or_else(|_| serde_json::json!({"raw": raw_body}))
        };

        let debug_log = build_response_debug_log(
            request_id,
            provider_id,
            provider_account_id,
            model,
            sequence_no,
            i64::from(upstream_status),
            raw_body,
            &body,
            endpoint,
            provider_is_codex,
        );

        if let Err(err) = repository.insert_response_debug_log(debug_log).await {
            tracing::warn!(error = %err, "failed to persist streamed response debug log");
        }
    });
}

#[derive(Clone)]
pub struct RouterService {
    repository: Arc<SqliteRepository>,
    upstream: UpstreamClient,
    round_robin: Arc<Mutex<HashMap<String, usize>>>,
    translator: Arc<TranslatorRegistry>,
    oauth: OAuthService,
    retry_config: RetryConfig,
    stream_idle_timeout: std::time::Duration,
    pub rate_limit_tracker: RateLimitTracker,
    fingerprint: ClaudeCodeFingerprint,
}

pub struct RoutedResponse {
    pub body: serde_json::Value,
    pub debug: RoutingDebug,
    pub is_stream: bool,
    pub response_headers: Vec<(String, String)>,
}

pub enum RoutedResult {
    /// Buffered JSON response
    Json(RoutedResponse),
    /// True SSE streaming relay
    Stream(RoutedStream),
    /// Raw passthrough relay preserving upstream bytes/content-type
    Raw(RawRoutedResponse),
}

pub struct RawRoutedResponse {
    pub status: reqwest::StatusCode,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    pub response_headers: Vec<(String, String)>,
}

pub struct RoutedStream {
    pub stream: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, BoxError>> + Send>>,
    pub debug: RoutingDebug,
    pub buffer: Arc<std::sync::Mutex<Vec<u8>>>,
    pub response_headers: Vec<(String, String)>,
}

impl RouterService {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        Self::with_clients(repository, ProxyClientCache::new())
    }

    pub fn with_clients(repository: Arc<SqliteRepository>, clients: ProxyClientCache) -> Self {
        let stream_idle_timeout_secs: u64 = std::env::var("KOU_STREAM_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90);
        Self {
            oauth: OAuthService::with_clients(repository.clone(), clients.clone()),
            repository,
            upstream: UpstreamClient::with_clients(clients),
            round_robin: Arc::new(Mutex::new(HashMap::new())),
            translator: Arc::new(TranslatorRegistry::new()),
            retry_config: RetryConfig::from_env(),
            stream_idle_timeout: std::time::Duration::from_secs(stream_idle_timeout_secs),
            rate_limit_tracker: RateLimitTracker::new(),
            fingerprint: ClaudeCodeFingerprint::new(),
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
            data: self
                .repository
                .get_models_catalog_for_endpoint(endpoint)
                .await?,
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
        request_id: String,
    ) -> AppResult<RoutedResult> {
        let normalized = normalize_request(endpoint, payload)?;
        let resolved_model = self.repository.resolve_alias(&normalized.model).await?;
        let candidate_models = self.expand_candidates(endpoint, &resolved_model).await?;
        let providers = self.repository.list_provider_connections().await?;
        let now = Utc::now();
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
        let mut debug_sequence_no = 0_i64;
        for candidate in candidate_models {
            let (prefix, raw_model) = split_model(&candidate)?;
            let mut matching: Vec<_> = if prefix.is_empty() {
                find_providers_for_bare_model(&providers, &raw_model)
            } else {
                providers
                    .iter()
                    .filter(|provider| {
                        provider.model_prefix == prefix || provider.provider == prefix
                    })
                    .cloned()
                    .collect()
            };
            matching.sort_by(|a, b| {
                let a_near_limit =
                    a.rate_limit_protection && self.rate_limit_tracker.is_near_limit(&a.id);
                let b_near_limit =
                    b.rate_limit_protection && self.rate_limit_tracker.is_near_limit(&b.id);
                a_near_limit
                    .cmp(&b_near_limit)
                    .then(a.priority.cmp(&b.priority))
            });

            if matching.is_empty() {
                tried.push(ProviderChatAttempt {
                    provider_id: format!(
                        "unresolved:{}",
                        if prefix.is_empty() {
                            &raw_model
                        } else {
                            &prefix
                        }
                    ),
                    model: raw_model.clone(),
                    status: 404,
                    body: format!(
                        "no provider found for model prefix and endpoint {}",
                        endpoint.as_str()
                    ),
                    account: None,
                });
                continue;
            }

            for provider in matching {
                let source_format = ProtocolFormat::detect_source(endpoint, &normalized.body);
                let target_format = ProtocolFormat::from_provider(&provider);
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
                let mut provider_body = translated_body.clone();
                maybe_adapt_codex_responses_request(
                    endpoint,
                    &provider,
                    &mut provider_body,
                    &normalized.body,
                );

                let execution_targets = self.execution_targets(&provider).await?;
                if execution_targets.is_empty() {
                    continue;
                }

                for (resolved_provider, selected_account) in execution_targets {
                    let account_debug = selected_account.as_ref().map(RoutingProviderAccount::from);
                    let provider_kind = classify_provider(&provider);
                    let oauth_ctx = build_oauth_context(selected_account.as_ref(), provider_kind);
                    let (final_body, final_pt) = if self
                        .fingerprint
                        .needs_injection(&target_format, &passthrough_headers)
                    {
                        let mut body = provider_body.clone();
                        self.fingerprint
                            .inject_body(&mut body, &normalized.body, oauth_ctx.as_ref());
                        let mut pt = passthrough_headers.clone().unwrap_or_default();
                        pt.merge(
                            self.fingerprint
                                .generate_headers(&raw_model, provider_kind, oauth_ctx.as_ref()),
                        );
                        (body, Some(pt))
                    } else {
                        (provider_body.clone(), passthrough_headers.clone())
                    };

                    let prepared_request = prepare_upstream_request(
                        &resolved_provider,
                        endpoint,
                        suffix.as_deref(),
                        &raw_model,
                        &final_body,
                        normalized.inject_model,
                    );
                    debug_sequence_no += 1;
                    let sequence_no = debug_sequence_no;
                    persist_request_debug_log(
                        self.repository.clone(),
                        NewRequestDebugLog {
                            request_id: request_id.clone(),
                            provider_id: provider.id.clone(),
                            provider_account_id: selected_account
                                .as_ref()
                                .map(|account| account.id.clone()),
                            model: candidate.clone(),
                            endpoint: endpoint.as_str().to_string(),
                            sequence_no,
                            raw_body: serde_json::to_string(&prepared_request.request_body)?,
                        },
                    );

                    let retry_outcome = match self
                        .execute_with_oauth_retry(
                            &provider,
                            &resolved_provider,
                            selected_account.as_ref(),
                            endpoint,
                            &raw_model,
                            &prepared_request,
                            final_pt.as_ref(),
                            selected_account
                                .as_ref()
                                .and_then(|account| account.proxy_url.as_deref()),
                        )
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            if matches!(err, AppError::BadRequest(_))
                                && let Some(account) = selected_account.as_ref()
                            {
                                tracing::warn!(
                                    account_id = %account.id,
                                    error = %err,
                                    "skipping account after invalid proxy configuration"
                                );
                            }
                            self.repository
                                .mark_provider_failure(&provider.id, &err.to_string())
                                .await?;
                            if let Some(account) = selected_account.as_ref() {
                                self.repository
                                    .mark_provider_account_failure(&account.id, &err.to_string())
                                    .await?;
                            }
                            tried.push(ProviderChatAttempt {
                                provider_id: provider.id.clone(),
                                model: candidate.clone(),
                                status: 502,
                                body: err.to_string(),
                                account: account_debug.clone(),
                            });
                            continue;
                        }
                    };
                    let result = retry_outcome.result;
                    let retry_after_secs = retry_outcome.retry_after_secs;

                    match result {
                        UpstreamResult::Streaming(streaming) => {
                            if endpoint == EndpointKind::Responses
                                && is_codex_provider(&provider)
                                && !normalized.stream
                            {
                                let resp_headers = streaming.response_headers;
                                let rl_info = parse_rate_limit_headers(&resp_headers);
                                self.rate_limit_tracker.update(&provider.id, &rl_info);
                                let mut watched =
                                    watchdog_stream(streaming.stream, self.stream_idle_timeout);
                                let mut transcript = String::new();
                                while let Some(chunk) = watched.next().await {
                                    let chunk =
                                        chunk.map_err(|err| AppError::Upstream(err.to_string()))?;
                                    transcript.push_str(&String::from_utf8_lossy(&chunk));
                                }
                                let attempt = ProviderChatAttempt {
                                    provider_id: provider.id.clone(),
                                    model: candidate.clone(),
                                    status: streaming.status.as_u16(),
                                    body: transcript.clone(),
                                    account: account_debug.clone(),
                                };
                                let (body, routed_is_stream) = adapt_responses_success_body(
                                    &transcript,
                                    normalized.stream,
                                    /*upstream_stream*/ true,
                                )?;
                                if let Some(error_message) = responses_stream_error_message(&body) {
                                    self.repository
                                        .mark_provider_failure(&provider.id, &error_message)
                                        .await?;
                                    if let Some(account) = selected_account.as_ref() {
                                        self.repository
                                            .mark_provider_account_failure(
                                                &account.id,
                                                &error_message,
                                            )
                                            .await?;
                                    }
                                    let kind = crate::error::classify_upstream_error(
                                        axum::http::StatusCode::BAD_GATEWAY,
                                        &error_message,
                                    );
                                    let upstream_error_body =
                                        serde_json::to_string(&serde_json::json!({
                                            "error": body.get("error").cloned().unwrap_or_else(|| {
                                                serde_json::json!({"message": error_message})
                                            }),
                                        }))?;
                                    let enriched_body = crate::error::enriched_error_response(
                                        kind,
                                        axum::http::StatusCode::BAD_GATEWAY,
                                        &upstream_error_body,
                                        &provider.id,
                                        retry_after_secs,
                                    );
                                    persist_codex_response_debug_log(
                                        self.repository.clone(),
                                        request_id.clone(),
                                        provider.id.clone(),
                                        selected_account.as_ref().map(|account| account.id.clone()),
                                        candidate.clone(),
                                        sequence_no,
                                        streaming.status.as_u16(),
                                        transcript.clone(),
                                        &body,
                                        endpoint,
                                        is_codex_provider(&provider),
                                    );
                                    tried.push(attempt);
                                    return Err(AppError::ClassifiedUpstream {
                                        status: kind.http_status(),
                                        body: enriched_body,
                                    });
                                }
                                persist_codex_response_debug_log(
                                    self.repository.clone(),
                                    request_id.clone(),
                                    provider.id.clone(),
                                    selected_account.as_ref().map(|account| account.id.clone()),
                                    candidate.clone(),
                                    sequence_no,
                                    streaming.status.as_u16(),
                                    transcript.clone(),
                                    &body,
                                    endpoint,
                                    is_codex_provider(&provider),
                                );
                                self.repository.mark_provider_success(&provider.id).await?;
                                if let Some(account) = selected_account.as_ref() {
                                    self.repository
                                        .mark_provider_account_success(&account.id)
                                        .await?;
                                }
                                let (usage, cost_usd) = {
                                    let usage_info = crate::cost::extract_usage(&body);
                                    let cost = usage_info.as_ref().map(|usage| {
                                        crate::cost::calculate_cost(&candidate, usage)
                                    });
                                    (usage_info, cost)
                                };
                                tried.push(attempt);
                                return Ok(RoutedResult::Json(RoutedResponse {
                                    body,
                                    debug: RoutingDebug {
                                        request_id: request_id.clone(),
                                        requested_model: normalized.model.clone(),
                                        resolved_model: resolved_model.clone(),
                                        endpoint: endpoint.as_str().to_string(),
                                        path_suffix: suffix.clone(),
                                        tried,
                                        usage,
                                        cost_usd,
                                    },
                                    is_stream: routed_is_stream,
                                    response_headers: resp_headers,
                                }));
                            }
                            self.repository.mark_provider_success(&provider.id).await?;
                            if let Some(account) = selected_account.as_ref() {
                                self.repository
                                    .mark_provider_account_success(&account.id)
                                    .await?;
                            }
                            tried.push(ProviderChatAttempt {
                                provider_id: provider.id.clone(),
                                model: candidate.clone(),
                                status: streaming.status.as_u16(),
                                body: "[streaming]".to_string(),
                                account: account_debug.clone(),
                            });
                            let rl_info = parse_rate_limit_headers(&streaming.response_headers);
                            self.rate_limit_tracker.update(&provider.id, &rl_info);
                            let watched =
                                watchdog_stream(streaming.stream, self.stream_idle_timeout);
                            let (teed, buffer) = tee_stream_boxerror(watched);
                            persist_codex_stream_buffer_debug_log(
                                self.repository.clone(),
                                request_id.clone(),
                                provider.id.clone(),
                                selected_account.as_ref().map(|account| account.id.clone()),
                                candidate.clone(),
                                sequence_no,
                                streaming.status.as_u16(),
                                buffer.clone(),
                                endpoint,
                                is_codex_provider(&provider),
                            );
                            return Ok(RoutedResult::Stream(RoutedStream {
                                stream: teed,
                                debug: RoutingDebug {
                                    request_id: request_id.clone(),
                                    requested_model: normalized.model.clone(),
                                    resolved_model: resolved_model.clone(),
                                    endpoint: endpoint.as_str().to_string(),
                                    path_suffix: suffix.clone(),
                                    tried,
                                    usage: None,
                                    cost_usd: None,
                                },
                                buffer,
                                response_headers: streaming.response_headers,
                            }));
                        }
                        UpstreamResult::Buffered(mut response) => {
                            let is_stream = response.is_stream;
                            let resp_headers = std::mem::take(&mut response.response_headers);
                            let mut attempt =
                                response.as_attempt(provider.id.clone(), candidate.clone());
                            attempt.account = account_debug.clone();
                            if (200..300).contains(&attempt.status) {
                                let mut routed_is_stream = is_stream;
                                let body = if source_format == target_format {
                                    if endpoint == EndpointKind::Responses {
                                        let (body, body_is_stream) = adapt_responses_success_body(
                                            &attempt.body,
                                            normalized.stream,
                                            is_stream,
                                        )?;
                                        routed_is_stream = body_is_stream;
                                        body
                                    } else if is_stream || normalized.stream {
                                        serde_json::json!({
                                            "object": "stream.capture",
                                            "stream": true,
                                            "raw": &attempt.body,
                                        })
                                    } else {
                                        serde_json::from_str(&attempt.body).unwrap_or_else(
                                            |_| serde_json::json!({"raw": &attempt.body}),
                                        )
                                    }
                                } else {
                                    let body_for_adapt = if target_format != ProtocolFormat::OpenAI
                                    {
                                        let parsed: serde_json::Value =
                                            serde_json::from_str(&attempt.body).unwrap_or_else(
                                                |_| serde_json::json!({"raw": &attempt.body}),
                                            );
                                        let translated = self.translator.translate_response(
                                            target_format,
                                            ProtocolFormat::OpenAI,
                                            &parsed,
                                        )?;
                                        serde_json::to_string(&translated)?
                                    } else {
                                        attempt.body.clone()
                                    };
                                    if endpoint == EndpointKind::Responses {
                                        let (body, body_is_stream) = adapt_responses_success_body(
                                            &body_for_adapt,
                                            normalized.stream,
                                            is_stream,
                                        )?;
                                        routed_is_stream = body_is_stream;
                                        body
                                    } else {
                                        adapt_success_body(
                                            endpoint,
                                            &body_for_adapt,
                                            normalized.stream,
                                            is_stream,
                                        )?
                                    }
                                };

                                if let Some(error_message) = responses_stream_error_message(&body) {
                                    self.repository
                                        .mark_provider_failure(&provider.id, &error_message)
                                        .await?;
                                    if let Some(account) = selected_account.as_ref() {
                                        self.repository
                                            .mark_provider_account_failure(
                                                &account.id,
                                                &error_message,
                                            )
                                            .await?;
                                    }
                                    let kind = crate::error::classify_upstream_error(
                                        axum::http::StatusCode::BAD_GATEWAY,
                                        &error_message,
                                    );
                                    let upstream_error_body =
                                        serde_json::to_string(&serde_json::json!({
                                            "error": body.get("error").cloned().unwrap_or_else(|| {
                                                serde_json::json!({"message": error_message})
                                            }),
                                        }))?;
                                    let enriched_body = crate::error::enriched_error_response(
                                        kind,
                                        axum::http::StatusCode::BAD_GATEWAY,
                                        &upstream_error_body,
                                        &provider.id,
                                        retry_after_secs,
                                    );
                                    persist_codex_response_debug_log(
                                        self.repository.clone(),
                                        request_id.clone(),
                                        provider.id.clone(),
                                        selected_account.as_ref().map(|account| account.id.clone()),
                                        candidate.clone(),
                                        sequence_no,
                                        attempt.status,
                                        attempt.body.clone(),
                                        &body,
                                        endpoint,
                                        is_codex_provider(&provider),
                                    );
                                    tried.push(attempt);
                                    return Err(AppError::ClassifiedUpstream {
                                        status: kind.http_status(),
                                        body: enriched_body,
                                    });
                                }

                                persist_codex_response_debug_log(
                                    self.repository.clone(),
                                    request_id.clone(),
                                    provider.id.clone(),
                                    selected_account.as_ref().map(|account| account.id.clone()),
                                    candidate.clone(),
                                    sequence_no,
                                    attempt.status,
                                    attempt.body.clone(),
                                    &body,
                                    endpoint,
                                    is_codex_provider(&provider),
                                );
                                self.repository.mark_provider_success(&provider.id).await?;
                                if let Some(account) = selected_account.as_ref() {
                                    self.repository
                                        .mark_provider_account_success(&account.id)
                                        .await?;
                                }
                                let rl_info = parse_rate_limit_headers(&resp_headers);
                                self.rate_limit_tracker.update(&provider.id, &rl_info);

                                let (usage, cost_usd) = {
                                    let usage_info = crate::cost::extract_usage(&body);
                                    let cost = usage_info.as_ref().map(|usage| {
                                        crate::cost::calculate_cost(&candidate, usage)
                                    });
                                    (usage_info, cost)
                                };

                                tried.push(attempt);
                                return Ok(RoutedResult::Json(RoutedResponse {
                                    body,
                                    debug: RoutingDebug {
                                        request_id: request_id.clone(),
                                        requested_model: normalized.model.clone(),
                                        resolved_model: resolved_model.clone(),
                                        endpoint: endpoint.as_str().to_string(),
                                        path_suffix: suffix.clone(),
                                        tried,
                                        usage,
                                        cost_usd,
                                    },
                                    is_stream: routed_is_stream,
                                    response_headers: resp_headers,
                                }));
                            }

                            self.repository
                                .mark_provider_failure(&provider.id, &attempt.body)
                                .await?;
                            if let Some(account) = selected_account.as_ref() {
                                self.repository
                                    .mark_provider_account_failure(&account.id, &attempt.body)
                                    .await?;
                            }
                            let upstream_status = reqwest::StatusCode::from_u16(attempt.status)
                                .unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
                            let should_fallback = fallback_error(upstream_status, &attempt.body);
                            let last_attempt_body = attempt.body.clone();
                            let last_attempt_status = attempt.status;
                            let last_provider_id = provider.id.clone();
                            tried.push(attempt);
                            if !should_fallback {
                                let kind = crate::error::classify_upstream_error(
                                    axum::http::StatusCode::from_u16(last_attempt_status)
                                        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                                    &last_attempt_body,
                                );
                                let enriched_body = crate::error::enriched_error_response(
                                    kind,
                                    axum::http::StatusCode::from_u16(last_attempt_status)
                                        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                                    &last_attempt_body,
                                    &last_provider_id,
                                    retry_after_secs,
                                );
                                let parsed_last_attempt_body =
                                    serde_json::from_str::<serde_json::Value>(&last_attempt_body)
                                        .unwrap_or_else(
                                            |_| serde_json::json!({"raw": last_attempt_body}),
                                        );
                                persist_codex_response_debug_log(
                                    self.repository.clone(),
                                    request_id.clone(),
                                    last_provider_id.clone(),
                                    selected_account.as_ref().map(|account| account.id.clone()),
                                    candidate.clone(),
                                    sequence_no,
                                    last_attempt_status,
                                    last_attempt_body.clone(),
                                    &parsed_last_attempt_body,
                                    endpoint,
                                    is_codex_provider(&provider),
                                );
                                return Err(AppError::ClassifiedUpstream {
                                    status: kind.http_status(),
                                    body: enriched_body,
                                });
                            }
                        }
                    }
                }
            }
        }

        if let Some(last) = tried.last() {
            let kind = crate::error::classify_upstream_error(
                axum::http::StatusCode::from_u16(last.status)
                    .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                &last.body,
            );
            let enriched_body = crate::error::enriched_error_response(
                kind,
                axum::http::StatusCode::from_u16(last.status)
                    .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                &last.body,
                &last.provider_id,
                None,
            );
            return Err(AppError::ClassifiedUpstream {
                status: kind.http_status(),
                body: enriched_body,
            });
        }

        Err(AppError::Upstream(format!(
            "all providers/combo targets failed for {}",
            endpoint.as_str()
        )))
    }

    pub async fn proxy_raw_to_provider(
        &self,
        request_id: String,
        endpoint: EndpointKind,
        method: reqwest::Method,
        upstream_path: &str,
        payload: Option<&serde_json::Value>,
        passthrough_headers: Option<PassthroughHeaders>,
    ) -> AppResult<RawRoutedResponse> {
        let model = payload
            .and_then(|value| value.get("model"))
            .and_then(|value| value.as_str())
            .unwrap_or("");

        let providers = self.repository.list_provider_connections().await?;
        let now = Utc::now();
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
            })
            .collect();

        let mut matching: Vec<_> = if model.is_empty() {
            providers
                .iter()
                .filter(|provider| {
                    supports_endpoint(&provider.supported_endpoints, endpoint)
                        && is_anthropic_compatible_provider(provider)
                })
                .cloned()
                .collect()
        } else {
            let (prefix, raw_model) = split_model(model)?;
            if prefix.is_empty() {
                find_providers_for_bare_model(&providers, &raw_model)
            } else {
                providers
                    .iter()
                    .filter(|provider| {
                        provider.model_prefix == prefix || provider.provider == prefix
                    })
                    .cloned()
                    .collect()
            }
        };
        matching.sort_by_key(|provider| provider.priority);

        if matching.is_empty() {
            return Err(AppError::BadRequest(if model.is_empty() {
                format!("no provider found for endpoint '{}'", endpoint.as_str())
            } else {
                format!("no provider found for model '{}'", model)
            }));
        }

        let mut debug_sequence_no = 0_i64;
        let mut last_error = None;
        for provider in matching {
            let execution_targets = self.execution_targets(&provider).await?;
            if execution_targets.is_empty() {
                continue;
            }

            for (resolved_provider, selected_account) in execution_targets {
                debug_sequence_no += 1;
                let sequence_no = debug_sequence_no;
                let raw_request = payload.cloned().unwrap_or(serde_json::Value::Null);
                persist_request_debug_log(
                    self.repository.clone(),
                    NewRequestDebugLog {
                        request_id: request_id.clone(),
                        provider_id: provider.id.clone(),
                        provider_account_id: selected_account
                            .as_ref()
                            .map(|account| account.id.clone()),
                        model: model.to_string(),
                        endpoint: upstream_path.to_string(),
                        sequence_no,
                        raw_body: serde_json::to_string(&raw_request)?,
                    },
                );

                let upstream_response = self
                    .upstream
                    .execute_raw_proxy(
                        &resolved_provider,
                        method.clone(),
                        upstream_path,
                        payload,
                        passthrough_headers.as_ref(),
                        selected_account
                            .as_ref()
                            .and_then(|account| account.proxy_url.as_deref()),
                    )
                    .await;
                let (status, resp_headers, content_type, body) = match upstream_response {
                    Ok(response) => response,
                    Err(err) => {
                        if matches!(err, AppError::BadRequest(_))
                            && let Some(account) = selected_account.as_ref()
                        {
                            tracing::warn!(
                                account_id = %account.id,
                                error = %err,
                                "skipping account after invalid proxy configuration"
                            );
                        }
                        self.repository
                            .mark_provider_failure(&provider.id, &err.to_string())
                            .await?;
                        if let Some(account) = selected_account.as_ref() {
                            self.repository
                                .mark_provider_account_failure(&account.id, &err.to_string())
                                .await?;
                        }
                        last_error = Some(RawRoutedResponse {
                            status: reqwest::StatusCode::BAD_GATEWAY,
                            body: err.to_string().into_bytes(),
                            content_type: Some("text/plain".to_owned()),
                            response_headers: Vec::new(),
                        });
                        continue;
                    }
                };
                let body_text = String::from_utf8_lossy(&body).to_string();
                let parsed_body = serde_json::from_slice::<serde_json::Value>(&body)
                    .unwrap_or_else(|_| serde_json::json!({"raw": body_text.clone()}));
                persist_response_debug_log(
                    self.repository.clone(),
                    build_response_debug_log(
                        request_id.clone(),
                        provider.id.clone(),
                        selected_account.as_ref().map(|account| account.id.clone()),
                        model.to_string(),
                        sequence_no,
                        i64::from(status.as_u16()),
                        body_text.clone(),
                        &parsed_body,
                        endpoint,
                        is_codex_provider(&provider),
                    ),
                );
                if status.is_success() {
                    self.repository.mark_provider_success(&provider.id).await?;
                    if let Some(account) = selected_account.as_ref() {
                        self.repository
                            .mark_provider_account_success(&account.id)
                            .await?;
                    }
                    return Ok(RawRoutedResponse {
                        status,
                        body,
                        content_type,
                        response_headers: resp_headers,
                    });
                }

                self.repository
                    .mark_provider_failure(&provider.id, &body_text)
                    .await?;
                if let Some(account) = selected_account.as_ref() {
                    self.repository
                        .mark_provider_account_failure(&account.id, &body_text)
                        .await?;
                }
                let should_fallback = fallback_error(status, &body_text);
                last_error = Some(RawRoutedResponse {
                    status,
                    body,
                    content_type,
                    response_headers: resp_headers,
                });
                if !should_fallback && let Some(last_error) = last_error {
                    return Ok(last_error);
                }
            }
        }

        last_error.ok_or_else(|| {
            AppError::BadRequest(if model.is_empty() {
                format!("no provider found for endpoint '{}'", endpoint.as_str())
            } else {
                format!("no provider found for model '{}'", model)
            })
        })
    }

    async fn execution_targets(
        &self,
        provider: &crate::models::ProviderConnection,
    ) -> AppResult<Vec<(crate::models::ProviderConnection, Option<ProviderAccount>)>> {
        let accounts = self
            .repository
            .list_selectable_provider_accounts(&provider.id)
            .await?;
        if accounts.is_empty() {
            if provider.auth_type.eq_ignore_ascii_case("oauth") {
                return Ok(Vec::new());
            }

            return if self
                .repository
                .list_provider_accounts(&provider.id)
                .await?
                .is_empty()
            {
                Ok(vec![(provider.clone(), None)])
            } else {
                Ok(Vec::new())
            };
        }

        let routing = self
            .repository
            .get_provider_account_routing_config(&provider.id)
            .await?;
        let accounts = self
            .order_provider_accounts(&provider.id, accounts, routing.strategy)
            .await;
        let mut targets = Vec::new();
        for account in accounts {
            if let Some(account) = self.prepare_provider_account(account).await? {
                targets.push((
                    provider_with_account_auth(provider, &account, &provider.provider),
                    Some(account),
                ));
            }
        }
        Ok(targets)
    }

    async fn order_provider_accounts(
        &self,
        provider_connection_id: &str,
        mut accounts: Vec<ProviderAccount>,
        strategy: ProviderAccountRoutingStrategy,
    ) -> Vec<ProviderAccount> {
        if strategy == ProviderAccountRoutingStrategy::RoundRobin && accounts.len() > 1 {
            let mut rr = self.round_robin.lock().await;
            let next = rr
                .entry(format!("provider-account:{provider_connection_id}"))
                .or_insert(0);
            let idx = *next % accounts.len();
            *next = next.wrapping_add(1);
            accounts.rotate_left(idx);
        }
        accounts
    }

    async fn prepare_provider_account(
        &self,
        account: ProviderAccount,
    ) -> AppResult<Option<ProviderAccount>> {
        match account.auth_mode {
            ProviderAccountAuthMode::ApiKey => {
                Ok(has_secret(account.api_key.as_deref()).then_some(account))
            }
            ProviderAccountAuthMode::OAuth => {
                let refresh_deadline = Utc::now() + Duration::minutes(5);
                let needs_refresh = account
                    .expires_at
                    .map(|expires_at| expires_at <= refresh_deadline)
                    .unwrap_or(!has_secret(account.access_token.as_deref()));
                let account = if needs_refresh {
                    match self.oauth.refresh_provider_account(&account.id).await {
                        Ok(account) => account,
                        Err(err) => {
                            self.repository
                                .record_provider_account_refresh_error(
                                    &account.id,
                                    &err.to_string(),
                                )
                                .await?;
                            return Ok(None);
                        }
                    }
                } else {
                    account
                };

                if has_secret(account.access_token.as_deref()) {
                    Ok(Some(account))
                } else {
                    self.repository
                        .record_provider_account_refresh_error(
                            &account.id,
                            "provider account has no access token after refresh",
                        )
                        .await?;
                    Ok(None)
                }
            }
        }
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
            return Err(AppError::BadRequest(format!(
                "combo {} is disabled",
                combo.name
            )));
        }

        Ok(match combo.strategy {
            ComboStrategy::Priority => combo.models,
            ComboStrategy::RoundRobin => {
                let picked = self.pick_round_robin(&combo.name, &combo.models).await?;
                let mut models = vec![picked.clone()];
                models.extend(combo.models.into_iter().filter(|model| model != &picked));
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

    /// Execute with retry, then if upstream returned 401 on an OAuth account,
    /// force-refresh the token and retry once. Mirrors Claude Code's `withOAuth401Retry`.
    ///
    /// `base_provider` is the unresolved ProviderConnection (without the access token baked in).
    /// `resolved_provider` already has the access token from the previous resolution; if a
    /// refresh happens, we rebuild it via `provider_with_account_auth`.
    #[allow(clippy::too_many_arguments)]
    async fn execute_with_oauth_retry(
        &self,
        base_provider: &crate::models::ProviderConnection,
        resolved_provider: &crate::models::ProviderConnection,
        account: Option<&ProviderAccount>,
        endpoint: EndpointKind,
        model: &str,
        prepared: &crate::upstream::PreparedUpstreamRequest,
        passthrough: Option<&PassthroughHeaders>,
        proxy_url: Option<&str>,
    ) -> AppResult<crate::retry::RetryOutcome> {
        let outcome = execute_with_retry(
            &self.upstream,
            resolved_provider,
            endpoint,
            model,
            prepared,
            passthrough,
            proxy_url,
            &self.retry_config,
        )
        .await?;

        let needs_oauth_refresh = matches!(
            (account, &outcome.result),
            (Some(a), UpstreamResult::Buffered(resp))
                if a.auth_mode == ProviderAccountAuthMode::OAuth && resp.status.as_u16() == 401
        );
        if !needs_oauth_refresh {
            return Ok(outcome);
        }
        let account = match account {
            Some(a) => a,
            None => return Ok(outcome),
        };

        tracing::warn!(
            account_id = %account.id,
            "upstream returned 401 on OAuth account; force-refreshing token and retrying once"
        );
        let fresh_account = match self.oauth.refresh_provider_account(&account.id).await {
            Ok(fresh) => fresh,
            Err(err) => {
                tracing::warn!(
                    account_id = %account.id,
                    error = %err,
                    "oauth force-refresh failed after 401; returning original outcome"
                );
                return Ok(outcome);
            }
        };
        let refreshed_provider =
            provider_with_account_auth(base_provider, &fresh_account, &base_provider.provider);
        execute_with_retry(
            &self.upstream,
            &refreshed_provider,
            endpoint,
            model,
            prepared,
            passthrough,
            proxy_url,
            &self.retry_config,
        )
        .await
    }

}

fn has_secret(value: Option<&str>) -> bool {
    value.is_some_and(|secret| !secret.trim().is_empty())
}
fn is_codex_provider(provider: &crate::models::ProviderConnection) -> bool {
    provider.provider.eq_ignore_ascii_case("codex")
}

fn maybe_adapt_codex_responses_request(
    endpoint: EndpointKind,
    provider: &crate::models::ProviderConnection,
    body: &mut serde_json::Value,
    normalized_body: &serde_json::Value,
) {
    if endpoint != EndpointKind::Responses || !is_codex_provider(provider) {
        return;
    }

    let instructions = body
        .get("instructions")
        .is_none()
        .then(|| codex_response_instructions(body, normalized_body));
    let rewritten_input = (!codex_input_is_response_items(body.get("input")))
        .then(|| codex_response_input(body, normalized_body));

    let Some(object) = body.as_object_mut() else {
        return;
    };
    if let Some(instructions) = instructions {
        object.insert(
            "instructions".to_string(),
            serde_json::Value::String(instructions),
        );
    }
    if let Some(input) = rewritten_input {
        object.insert("input".to_string(), input);
    }
    object.remove("max_tokens");
    object.remove("max_output_tokens");
    object.remove("max_completion_tokens");
    object.insert("stream".to_string(), serde_json::Value::Bool(true));
    object.remove("messages");
    object.insert("store".to_string(), serde_json::Value::Bool(false));
}

fn codex_response_instructions(
    body: &serde_json::Value,
    normalized_body: &serde_json::Value,
) -> String {
    body.get("instructions")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            normalized_body
                .get("messages")
                .and_then(serde_json::Value::as_array)
                .map(|messages| {
                    messages
                        .iter()
                        .filter(|message| {
                            message.get("role").and_then(serde_json::Value::as_str)
                                == Some("system")
                        })
                        .filter_map(|message| {
                            codex_message_text(
                                message.get("content").unwrap_or(&serde_json::Value::Null),
                            )
                        })
                        .filter(|text| !text.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n\n")
                })
        })
        .unwrap_or_default()
}

fn codex_response_input(
    body: &serde_json::Value,
    normalized_body: &serde_json::Value,
) -> serde_json::Value {
    body.get("messages")
        .or_else(|| normalized_body.get("messages"))
        .and_then(serde_json::Value::as_array)
        .map(|messages| codex_response_input_from_messages(messages))
        .filter(|items| items.as_array().is_some_and(|entries| !entries.is_empty()))
        .unwrap_or_else(|| {
            serde_json::Value::Array(vec![codex_response_message_item(
                "user",
                body.get("input").unwrap_or(&serde_json::Value::Null),
            )])
        })
}

fn codex_input_is_response_items(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Array(items)) => {
            items.iter().all(|item| item.get("type").is_some())
        }
        Some(serde_json::Value::Object(object)) => object.get("type").is_some(),
        _ => false,
    }
}

fn codex_response_input_from_messages(messages: &[serde_json::Value]) -> serde_json::Value {
    serde_json::Value::Array(
        messages
            .iter()
            .filter_map(|message| {
                let role = message
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("user");
                if role == "system" {
                    return None;
                }

                Some(codex_response_message_item(
                    role,
                    message.get("content").unwrap_or(&serde_json::Value::Null),
                ))
            })
            .collect(),
    )
}

fn codex_response_message_item(role: &str, content: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "role": role,
        "content": codex_response_message_content(content),
    })
}

fn codex_response_message_content(content: &serde_json::Value) -> serde_json::Value {
    match content {
        serde_json::Value::String(text) => {
            serde_json::json!([{"type": "input_text", "text": text}])
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(
                    |item| match item.get("type").and_then(serde_json::Value::as_str) {
                        Some("input_text") | Some("input_image") => item.clone(),
                        Some("text") => item
                            .get("text")
                            .and_then(serde_json::Value::as_str)
                            .map(|text| serde_json::json!({"type": "input_text", "text": text}))
                            .unwrap_or_else(|| item.clone()),
                        _ => item.clone(),
                    },
                )
                .collect(),
        ),
        serde_json::Value::Null => serde_json::Value::Array(Vec::new()),
        other => serde_json::json!([{"type": "input_text", "text": other.to_string()}]),
    }
}

fn codex_message_text(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(
                    |item| match item.get("type").and_then(serde_json::Value::as_str) {
                        Some("text") | Some("input_text") | Some("output_text") => item
                            .get("text")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        _ => item.as_str().map(str::to_string),
                    },
                )
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() { None } else { Some(text) }
        }
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
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
            if object.get("messages").is_none()
                && let Some(input) = object.get("input").cloned()
            {
                object.insert("messages".to_string(), input);
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
        EndpointKind::Files => {
            return Err(AppError::BadRequest(
                "files endpoints should use raw proxy routing".into(),
            ));
        }
        EndpointKind::OllamaChat => {
            if object.get("messages").is_none() {
                return Err(AppError::BadRequest("ollama chat requires messages".into()));
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
                .or_insert(serde_json::Value::String(
                    "openai/omni-moderation-latest".to_string(),
                ));
        }
        EndpointKind::Rerank => {
            if object.get("query").is_none() {
                return Err(AppError::BadRequest("rerank request requires query".into()));
            }
            let documents = object
                .get("documents")
                .and_then(|value| value.as_array())
                .ok_or_else(|| {
                    AppError::BadRequest("rerank request requires documents array".into())
                })?;
            if documents.is_empty() {
                return Err(AppError::BadRequest(
                    "rerank request requires at least one document".into(),
                ));
            }
        }
        EndpointKind::Search => {
            if object.get("query").is_none() {
                return Err(AppError::BadRequest("search request requires query".into()));
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

fn adapt_responses_success_body(
    body: &str,
    stream_requested: bool,
    _upstream_stream: bool,
) -> AppResult<(serde_json::Value, bool)> {
    if is_sse_transcript(body) {
        if stream_requested {
            return Ok((serde_json::json!({"raw": body}), true));
        }
        return Ok((reconstruct_responses_from_sse(body)?, false));
    }

    let json_body: serde_json::Value = serde_json::from_str(body)?;
    Ok((adapt_to_responses(json_body), false))
}

fn is_sse_transcript(body: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("event:") || line.starts_with("data:")
    })
}

fn parse_sse_events(body: &str) -> Vec<(Option<String>, String)> {
    let mut events = Vec::new();
    let mut event_name = None;
    let mut data_lines: Vec<String> = Vec::new();

    let flush_event = |events: &mut Vec<(Option<String>, String)>,
                       event_name: &mut Option<String>,
                       data_lines: &mut Vec<String>| {
        if event_name.is_none() && data_lines.is_empty() {
            return;
        }
        events.push((event_name.take(), data_lines.join("\n")));
        data_lines.clear();
    };

    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            flush_event(&mut events, &mut event_name, &mut data_lines);
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    flush_event(&mut events, &mut event_name, &mut data_lines);
    events
}

fn merge_response_fields(
    target: &mut serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, value) in object {
        if matches!(
            key.as_str(),
            "id" | "object" | "model" | "status" | "usage" | "error" | "output" | "created_at"
        ) {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn reconstruct_responses_from_sse(body: &str) -> AppResult<serde_json::Value> {
    let mut response = serde_json::Map::new();
    response.insert(
        "object".to_string(),
        serde_json::Value::String("response".to_string()),
    );
    let mut output_items = Vec::new();
    let mut synthesized_text = String::new();

    for (event_name, payload) in parse_sse_events(body) {
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if let Some(response_value) = value.get("response") {
            merge_response_fields(&mut response, response_value);
        } else if value.get("id").is_some()
            || value.get("status").is_some()
            || value.get("output").is_some()
        {
            merge_response_fields(&mut response, &value);
        }

        if event_name
            .as_deref()
            .is_some_and(|name| name.ends_with("output_item.done"))
            && let Some(item) = value.get("item").cloned()
        {
            output_items.push(item);
        }

        if let Some(delta) = value.get("delta").and_then(serde_json::Value::as_str) {
            synthesized_text.push_str(delta);
        } else if synthesized_text.is_empty()
            && let Some(text) = value.get("text").and_then(serde_json::Value::as_str)
        {
            synthesized_text.push_str(text);
        }
    }

    if response.get("output").is_none() {
        let output = if !output_items.is_empty() {
            serde_json::Value::Array(output_items)
        } else {
            serde_json::json!([{
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": synthesized_text,
                }],
            }])
        };
        response.insert("output".to_string(), output);
    }
    response
        .entry("id".to_string())
        .or_insert_with(|| serde_json::json!("resp_kou_router"));
    response
        .entry("model".to_string())
        .or_insert(serde_json::Value::Null);
    response
        .entry("status".to_string())
        .or_insert_with(|| serde_json::json!("completed"));
    response
        .entry("usage".to_string())
        .or_insert(serde_json::Value::Null);
    response
        .entry("error".to_string())
        .or_insert(serde_json::Value::Null);

    Ok(serde_json::Value::Object(response))
}

fn responses_stream_error_message(body: &serde_json::Value) -> Option<String> {
    if body.get("status").and_then(serde_json::Value::as_str) != Some("failed") {
        return None;
    }
    let error = body.get("error")?;
    if error.is_null() {
        return Some("responses stream failed".to_string());
    }
    error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| error.as_str().map(str::to_string))
        .or_else(|| Some("responses stream failed".to_string()))
}

fn build_response_debug_log(
    request_id: String,
    provider_id: String,
    provider_account_id: Option<String>,
    model: String,
    sequence_no: i64,
    upstream_status: i64,
    raw_body: String,
    body: &serde_json::Value,
    endpoint: EndpointKind,
    provider_is_codex: bool,
) -> NewResponseDebugLog {
    let (reasoning_summary_json, obfuscation_count) =
        if endpoint == EndpointKind::Responses && provider_is_codex {
            (
                extract_reasoning_summary_json(&raw_body, body),
                count_obfuscation_occurrences(&raw_body),
            )
        } else {
            (None, 0)
        };

    NewResponseDebugLog {
        request_id,
        provider_id,
        provider_account_id,
        model,
        endpoint: endpoint.as_str().to_string(),
        sequence_no,
        upstream_status,
        obfuscation_count,
        reasoning_summary_json,
        raw_body,
    }
}

fn extract_reasoning_summary_json(raw_body: &str, body: &serde_json::Value) -> Option<String> {
    let mut summaries = Vec::new();
    collect_reasoning_summaries(body, &mut summaries);
    if summaries.is_empty() {
        for (_event_name, payload) in parse_sse_events(raw_body) {
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
                continue;
            };
            collect_reasoning_summaries(&value, &mut summaries);
        }
    }
    if summaries.is_empty() {
        None
    } else {
        serde_json::to_string(&summaries).ok()
    }
}

fn collect_reasoning_summaries(value: &serde_json::Value, summaries: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("type").and_then(serde_json::Value::as_str) == Some("reasoning")
                && let Some(summary) = map.get("summary")
            {
                summaries.push(summary.clone());
            }
            for child in map.values() {
                collect_reasoning_summaries(child, summaries);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_reasoning_summaries(item, summaries);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn count_obfuscation_occurrences(haystack: &str) -> i64 {
    haystack.match_indices("\"obfuscation\"").count() as i64
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
        EndpointKind::Files => Err(AppError::BadRequest(
            "files endpoints should not use JSON success adapter".into(),
        )),
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
        "model": body.get("model").cloned().unwrap_or(serde_json::Value::Null),
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
        "model": body.get("model").cloned().unwrap_or(serde_json::Value::Null),
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
            return Err(AppError::BadRequest(format!(
                "invalid model identifier: {value}"
            )));
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
            if let Some(ref inferred) = inferred_provider
                && (p.provider == *inferred
                    || p.model_prefix == *inferred
                    || (*inferred == "anthropic" && is_anthropic_compatible_provider(p)))
            {
                return true;
            }
            if let Some(ref default) = p.default_model
                && (default == model || default.ends_with(&format!("/{model}")))
            {
                return true;
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

/// Check if a provider connection speaks the Anthropic/Claude protocol.
fn is_anthropic_compatible_provider(provider: &crate::models::ProviderConnection) -> bool {
    ProtocolFormat::from_provider(provider) == ProtocolFormat::Claude
}

/// Classify a provider connection into the kind enum used by fingerprint header logic.
fn classify_provider(provider: &crate::models::ProviderConnection) -> ProviderKind {
    let name = provider.provider.to_ascii_lowercase();
    let url = provider.base_url.to_ascii_lowercase();

    if matches!(
        name.as_str(),
        "anthropic" | "claude-oauth" | "anthropic-oauth" | "claude"
    ) || url.contains("api.anthropic.com")
    {
        return ProviderKind::FirstParty;
    }
    if name == "foundry" || url.contains("services.ai.azure.com") {
        return ProviderKind::Foundry;
    }
    if name == "vertex" || url.contains("aiplatform.googleapis.com") {
        return ProviderKind::Vertex;
    }
    if name == "bedrock" || url.contains("amazonaws.com") {
        return ProviderKind::Bedrock;
    }
    ProviderKind::Other
}

/// Build an `OAuthContext` for fingerprint injection from the selected account + provider kind.
/// Returns None when the account is api-key or absent (no OAuth context to propagate).
///
/// `is_oauth_subscriber` is true only for Anthropic 1P + OAuth account (the precondition for
/// the `oauth-2025-04-20` Anthropic beta). Foundry/Vertex/Bedrock OAuth, even if it existed,
/// is not Anthropic-issued and would not unlock the same beta.
fn build_oauth_context(
    account: Option<&ProviderAccount>,
    kind: ProviderKind,
) -> Option<OAuthContext> {
    let account = account?;
    if account.auth_mode != ProviderAccountAuthMode::OAuth {
        return None;
    }
    Some(OAuthContext {
        account_uuid: account.remote_account_id.clone(),
        is_oauth_subscriber: kind == ProviderKind::FirstParty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EndpointKind;
    use serde_json::json;

    // --- normalize_request ---

    #[test]
    fn test_normalize_chat_completions() {
        let payload =
            json!({"model": "openai/gpt-4", "messages": [{"role": "user", "content": "hi"}]});
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
    fn test_codex_responses_request_adapts_string_input() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "codex".to_string(),
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            api_key: None,
            auth_type: "oauth".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: Some("Bearer".to_string()),
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "codex".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: Some("codex/gpt-5.3-codex".to_string()),
            supported_endpoints: vec!["responses".to_string()],
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
            protocol_format: Some("openai-responses".to_string()),
        };
        let normalized = normalize_request(
            EndpointKind::Responses,
            json!({
                "model": "codex/gpt-5.3-codex",
                "input": "hello",
                "stream": false
            }),
        )
        .unwrap();
        assert!(!normalized.stream);
        let mut body = normalized.body.clone();

        maybe_adapt_codex_responses_request(
            EndpointKind::Responses,
            &provider,
            &mut body,
            &normalized.body,
        );

        assert_eq!(body["instructions"], "");
        assert_eq!(
            body["input"],
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }])
        );
        assert_eq!(body["stream"], json!(true));
        assert!(body.get("messages").is_none());
        assert_eq!(body["store"], json!(false));
    }

    #[test]
    fn test_codex_responses_request_preserves_native_input_and_instructions() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "codex".to_string(),
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            api_key: None,
            auth_type: "oauth".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: Some("Bearer".to_string()),
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "codex".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: Some("codex/gpt-5.3-codex".to_string()),
            supported_endpoints: vec!["responses".to_string()],
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
            protocol_format: Some("openai-responses".to_string()),
        };
        let native_input = json!([{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hi"}]
        }]);
        let mut body = json!({
            "model": "codex/gpt-5.3-codex",
            "instructions": "keep me",
            "input": native_input.clone(),
            "store": true
        });
        let normalized_body = body.clone();

        maybe_adapt_codex_responses_request(
            EndpointKind::Responses,
            &provider,
            &mut body,
            &normalized_body,
        );

        assert_eq!(body["instructions"], "keep me");
        assert_eq!(body["input"], native_input);
        assert_eq!(body["stream"], json!(true));
        assert!(body.get("messages").is_none());
        assert_eq!(body["store"], json!(false));
    }

    #[test]
    fn test_codex_responses_request_drops_token_limit_fields() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "codex".to_string(),
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            api_key: None,
            auth_type: "oauth".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: Some("Bearer".to_string()),
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "codex".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: Some("codex/gpt-5.3-codex".to_string()),
            supported_endpoints: vec!["responses".to_string()],
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
            protocol_format: Some("openai-responses".to_string()),
        };

        let mut body = json!({
            "model": "codex/gpt-5.3-codex",
            "input": "hello",
            "max_output_tokens": 512,
            "max_tokens": 256,
            "max_completion_tokens": 128
        });
        let normalized_body = body.clone();

        maybe_adapt_codex_responses_request(
            EndpointKind::Responses,
            &provider,
            &mut body,
            &normalized_body,
        );

        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());
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
        assert_eq!(
            infer_provider_from_model_name("claude-sonnet-4-20250514"),
            Some("anthropic".to_string())
        );
    }

    #[test]
    fn test_infer_provider_gpt() {
        assert_eq!(
            infer_provider_from_model_name("gpt-4o"),
            Some("openai".to_string())
        );
    }

    #[test]
    fn test_infer_provider_unknown() {
        assert_eq!(infer_provider_from_model_name("some-random-model"), None);
    }

    #[test]
    fn test_find_providers_for_bare_claude_model_matches_claude_oauth_protocol() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "claude-oauth".to_string(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key: None,
            auth_type: "oauth".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_prefix: None,
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "claude-oauth".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: Some("claude-oauth/claude-sonnet-4.6".to_string()),
            supported_endpoints: vec!["messages".to_string()],
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
            protocol_format: Some("claude".to_string()),
        };

        let matching = find_providers_for_bare_model(&[provider], "claude-sonnet-4.6");

        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].provider, "claude-oauth");
    }

    #[test]
    fn test_supports_endpoint_chat_family_matches_chat_capability() {
        assert!(supports_endpoint(
            &["chat".to_string()],
            EndpointKind::ChatCompletions
        ));
        assert!(supports_endpoint(
            &["chat".to_string()],
            EndpointKind::Responses
        ));
        assert!(supports_endpoint(
            &["chat".to_string()],
            EndpointKind::Messages
        ));
    }

    #[test]
    fn test_supports_endpoint_files_capability() {
        assert!(supports_endpoint(
            &["files".to_string()],
            EndpointKind::Files
        ));
        assert!(!supports_endpoint(
            &["messages".to_string()],
            EndpointKind::Files
        ));
    }

    #[test]
    fn test_find_providers_for_bare_claude_model_does_not_imply_files_support() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "claude-oauth".to_string(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key: None,
            auth_type: "oauth".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_prefix: None,
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "claude-oauth".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: Some("claude-oauth/claude-sonnet-4.6".to_string()),
            supported_endpoints: vec!["messages".to_string()],
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
            protocol_format: Some("claude".to_string()),
        };
        let matching =
            find_providers_for_bare_model(std::slice::from_ref(&provider), "claude-sonnet-4.6");
        assert_eq!(matching.len(), 1);
        assert!(!supports_endpoint(
            &matching[0].supported_endpoints,
            EndpointKind::Files
        ));
    }

    #[test]
    fn test_is_anthropic_compatible_provider_for_explicit_claude_protocol() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "custom".to_string(),
            base_url: "https://example.test/v1".to_string(),
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "custom".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: None,
            supported_endpoints: vec!["messages".to_string()],
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
            protocol_format: Some("claude".to_string()),
        };

        assert!(is_anthropic_compatible_provider(&provider));
    }

    #[test]
    fn test_is_anthropic_compatible_provider_rejects_openai_protocol() {
        let provider = crate::models::ProviderConnection {
            id: String::new(),
            provider: "openai-compatible".to_string(),
            base_url: "https://example.test/v1".to_string(),
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "openai-compatible".to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: None,
            supported_endpoints: vec!["messages".to_string()],
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
        };

        assert!(!is_anthropic_compatible_provider(&provider));
    }

    // --- adapt / extract ---

    #[test]
    fn test_adapt_responses_success_body_reconstructs_sse_transcript() {
        let body = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\",\"model\":\"codex-mini\",\"status\":\"in_progress\"}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\" world\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"model\":\"codex-mini\",\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2,\"total_tokens\":3}}}\n\n"
        );

        let (result, is_stream) = adapt_responses_success_body(body, false, true).unwrap();

        assert!(!is_stream);
        assert_eq!(result["id"], "resp_123");
        assert_eq!(result["object"], "response");
        assert_eq!(result["model"], "codex-mini");
        assert_eq!(result["status"], "completed");
        assert_eq!(result["output"][0]["content"][0]["text"], "Hello world");
        assert_eq!(result["usage"]["total_tokens"], 3);
    }

    #[test]
    fn test_adapt_responses_success_body_preserves_sse_for_streaming() {
        let body = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n"
        );

        let (result, is_stream) = adapt_responses_success_body(
            body, /*stream_requested*/ true, /*upstream_stream*/ true,
        )
        .unwrap();

        assert!(is_stream);
        assert_eq!(result["raw"], body);
    }

    #[test]
    fn test_responses_stream_error_message_detects_failed_response() {
        let body = json!({
            "status": "failed",
            "error": {"message": "rate limit exceeded"}
        });

        assert_eq!(
            responses_stream_error_message(&body),
            Some("rate limit exceeded".to_string())
        );
    }

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
    fn test_build_codex_response_debug_log_extracts_reasoning_and_obfuscation() {
        let raw = concat!(
            "event: response.output_item.done\n",
            "data: {\"item\":{\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"planned\"}]}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"delta\":\"hi\",\"obfuscation\":\"abc\"}\n\n"
        );
        let body = reconstruct_responses_from_sse(raw).unwrap();

        let log = build_response_debug_log(
            "req_123".to_string(),
            "provider_123".to_string(),
            Some("account_123".to_string()),
            "codex/gpt-5.3-codex".to_string(),
            7,
            200,
            raw.to_string(),
            &body,
            EndpointKind::Responses,
            true,
        );

        assert_eq!(log.request_id, "req_123");
        assert_eq!(log.provider_account_id, Some("account_123".to_string()));
        assert_eq!(log.sequence_no, 7);
        assert_eq!(log.obfuscation_count, 1);
        assert_eq!(
            log.reasoning_summary_json,
            Some("[[{\"text\":\"planned\",\"type\":\"summary_text\"}]]".to_string())
        );
    }

    #[test]
    fn test_build_response_debug_log_allows_failed_codex_responses() {
        let body = json!({"status": "failed", "error": {"message": "bad request"}});

        let log = build_response_debug_log(
            "req_failed".to_string(),
            "provider_123".to_string(),
            None,
            "codex/gpt-5.3-codex".to_string(),
            3,
            400,
            "{\"status\":\"failed\"}".to_string(),
            &body,
            EndpointKind::Responses,
            true,
        );

        assert_eq!(log.sequence_no, 3);
        assert_eq!(log.upstream_status, 400);
        assert_eq!(log.raw_body, "{\"status\":\"failed\"}");
    }

    #[test]
    fn test_build_response_debug_log_skips_codex_enrichment_for_other_paths() {
        let body = json!({"output": []});

        let log = build_response_debug_log(
            "req_123".to_string(),
            "provider_123".to_string(),
            None,
            "openai/gpt-4.1".to_string(),
            2,
            200,
            "{}".to_string(),
            &body,
            EndpointKind::ChatCompletions,
            false,
        );

        assert_eq!(log.sequence_no, 2);
        assert_eq!(log.reasoning_summary_json, None);
        assert_eq!(log.obfuscation_count, 0);
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
    #[tokio::test]
    async fn test_execution_targets_skips_oauth_provider_without_accounts() {
        let pool = crate::db::init_db("sqlite::memory:").await.unwrap();
        let repository = Arc::new(crate::repository::SqliteRepository::new(pool));
        let service = RouterService::new(repository.clone());

        let provider = repository
            .create_provider_connection(crate::models::NewProviderConnection {
                provider: "codex".to_string(),
                base_url: "https://chatgpt.com/backend-api/codex".to_string(),
                api_key: None,
                auth_type: "oauth".to_string(),
                auth_header: "bearer".to_string(),
                auth_prefix: Some("Bearer".to_string()),
                extra_headers: Default::default(),
                endpoint_paths: Some(Default::default()),
                stream_endpoint_paths: Some(Default::default()),
                model_prefix: Some("codex".to_string()),
                name: None,
                enabled: true,
                priority: Some(0),
                default_model: Some("codex/gpt-5.3-codex".to_string()),
                supported_endpoints: Some(vec!["responses".to_string()]),
                rate_limit_protection: false,
                protocol_format: Some("openai-responses".to_string()),
            })
            .await
            .unwrap();

        let targets = service.execution_targets(&provider).await.unwrap();
        assert!(targets.is_empty());
    }

    #[tokio::test]
    async fn test_execution_targets_allows_apikey_provider_without_accounts() {
        let pool = crate::db::init_db("sqlite::memory:").await.unwrap();
        let repository = Arc::new(crate::repository::SqliteRepository::new(pool));
        let service = RouterService::new(repository.clone());

        let provider = repository
            .create_provider_connection(crate::models::NewProviderConnection {
                provider: "openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                api_key: Some("sk-test".to_string()),
                auth_type: "apikey".to_string(),
                auth_header: "bearer".to_string(),
                auth_prefix: Some("Bearer".to_string()),
                extra_headers: Default::default(),
                endpoint_paths: Some(Default::default()),
                stream_endpoint_paths: Some(Default::default()),
                model_prefix: Some("openai".to_string()),
                name: None,
                enabled: true,
                priority: Some(0),
                default_model: Some("openai/gpt-4.1".to_string()),
                supported_endpoints: Some(vec!["responses".to_string()]),
                rate_limit_protection: false,
                protocol_format: Some("openai-responses".to_string()),
            })
            .await
            .unwrap();

        let targets = service.execution_targets(&provider).await.unwrap();
        assert_eq!(targets.len(), 1);
        assert!(targets[0].1.is_none());
        assert_eq!(targets[0].0.id, provider.id);
    }

    fn make_provider(name: &str, base_url: &str) -> crate::models::ProviderConnection {
        crate::models::ProviderConnection {
            id: "p".into(),
            provider: name.into(),
            base_url: base_url.into(),
            api_key: None,
            auth_type: "apikey".into(),
            auth_header: "bearer".into(),
            auth_prefix: None,
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: "".into(),
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
    fn test_classify_provider_anthropic_is_first_party() {
        let p = make_provider("anthropic", "https://api.anthropic.com");
        assert_eq!(classify_provider(&p), ProviderKind::FirstParty);
    }

    #[test]
    fn test_classify_provider_claude_oauth_is_first_party() {
        let p = make_provider("claude-oauth", "https://api.anthropic.com");
        assert_eq!(classify_provider(&p), ProviderKind::FirstParty);
    }

    #[test]
    fn test_classify_provider_foundry() {
        let p = make_provider("foundry", "https://east-us.services.ai.azure.com");
        assert_eq!(classify_provider(&p), ProviderKind::Foundry);
    }

    #[test]
    fn test_classify_provider_vertex() {
        let p = make_provider("vertex", "https://us-east4-aiplatform.googleapis.com");
        assert_eq!(classify_provider(&p), ProviderKind::Vertex);
    }

    #[test]
    fn test_classify_provider_bedrock() {
        let p = make_provider("bedrock", "https://bedrock-runtime.us-east-1.amazonaws.com");
        assert_eq!(classify_provider(&p), ProviderKind::Bedrock);
    }

    #[test]
    fn test_classify_provider_other() {
        let p = make_provider("openrouter", "https://openrouter.ai/api/v1");
        assert_eq!(classify_provider(&p), ProviderKind::Other);
    }

    fn make_account(
        auth_mode: ProviderAccountAuthMode,
        remote_account_id: Option<&str>,
    ) -> ProviderAccount {
        ProviderAccount {
            id: "a".into(),
            provider_connection_id: "p".into(),
            label: None,
            auth_mode,
            api_key: None,
            access_token: Some("tok".into()),
            refresh_token: Some("rt".into()),
            expires_at: None,
            scopes: vec![],
            remote_account_id: remote_account_id.map(str::to_string),
            remote_email: None,
            enabled: true,
            priority: 0,
            last_used_at: None,
            rate_limited_until: None,
            circuit_open_until: None,
            last_error: None,
            last_error_type: None,
            last_refresh_at: None,
            refresh_error: None,
            backoff_level: 0,
            consecutive_use_count: 0,
            proxy_url: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_build_oauth_context_oauth_first_party() {
        let acc = make_account(ProviderAccountAuthMode::OAuth, Some("acc-uuid"));
        let ctx = build_oauth_context(Some(&acc), ProviderKind::FirstParty).unwrap();
        assert_eq!(ctx.account_uuid.as_deref(), Some("acc-uuid"));
        assert!(ctx.is_oauth_subscriber);
    }

    #[test]
    fn test_build_oauth_context_oauth_3p_not_subscriber() {
        let acc = make_account(ProviderAccountAuthMode::OAuth, Some("acc-uuid"));
        let ctx = build_oauth_context(Some(&acc), ProviderKind::Vertex).unwrap();
        assert_eq!(ctx.account_uuid.as_deref(), Some("acc-uuid"));
        // Vertex OAuth is not Anthropic-subscriber
        assert!(!ctx.is_oauth_subscriber);
    }

    #[test]
    fn test_build_oauth_context_apikey_returns_none() {
        let acc = make_account(ProviderAccountAuthMode::ApiKey, Some("acc-uuid"));
        assert!(build_oauth_context(Some(&acc), ProviderKind::FirstParty).is_none());
    }

    #[test]
    fn test_build_oauth_context_no_account_returns_none() {
        assert!(build_oauth_context(None, ProviderKind::FirstParty).is_none());
    }
}
