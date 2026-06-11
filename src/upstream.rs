use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::{
    Client, Method, RequestBuilder, StatusCode,
    header::{AUTHORIZATION, HeaderName, HeaderValue},
    multipart,
};
use serde_json::{Value, json};

use crate::{
    error::AppResult,
    models::{
        AudioTranscriptionPayload, EndpointKind, ProviderAccount, ProviderAccountAuthMode,
        ProviderChatAttempt, ProviderConnection,
    },
    proxy::ProxyClientCache,
    search::{SearchHttpMethod, build_search_request, normalize_search_response},
};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;
pub type BoxErrorStream = Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>;
pub type TeeBuffer = Arc<Mutex<Vec<u8>>>;
const CODEX_ORIGINATOR_HEADER: &str = "originator";
const CODEX_ORIGINATOR_VALUE: &str = "codex_cli_rs";
const OPENAI_BETA_HEADER: &str = "OpenAI-Beta";
const OPENAI_BETA_RESPONSES_VALUE: &str = "responses=experimental";

/// Headers from the incoming request that should be passed through to upstream.
/// Covers Claude Code and Codex session/turn headers that clients own.
#[derive(Debug, Clone, Default)]
pub struct PassthroughHeaders {
    pub headers: Vec<(String, String)>,
}

impl PassthroughHeaders {
    /// Extract passthrough headers from an incoming axum request's HeaderMap.
    pub fn from_header_map(headers: &axum::http::HeaderMap) -> Self {
        const PASSTHROUGH_NAMES: &[&str] = &[
            "anthropic-beta",
            "anthropic-version",
            "x-request-id",
            "x-client-request-id",
            "x-app",
            "user-agent",
            "x-claude-code-session-id",
            "x-claude-code-agent-id",
            "x-claude-code-parent-agent-id",
            "x-claude-remote-container-id",
            "x-claude-remote-session-id",
            "x-client-app",
            "x-anthropic-additional-protection",
            "anthropic-client-platform",
            "openai-beta",
            "originator",
            "session-id",
            "thread-id",
            "traceparent",
            "tracestate",
            "x-codex-beta-features",
            "x-codex-installation-id",
            "x-codex-parent-thread-id",
            "x-codex-turn-metadata",
            "x-codex-turn-state",
            "x-codex-window-id",
            "x-oai-attestation",
            "x-openai-internal-codex-responses-lite",
            "x-openai-memgen-request",
            "x-openai-subagent",
            "x-responsesapi-include-timing-metrics",
        ];
        let mut extracted = Vec::new();
        for &name in PASSTHROUGH_NAMES {
            if let Some(value) = headers.get(name)
                && let Ok(v) = value.to_str()
            {
                extracted.push((name.to_string(), v.to_string()));
            }
        }
        Self { headers: extracted }
    }

    /// Merge additional headers without overwriting existing ones.
    /// Used by fingerprint injection to add Claude Code headers.
    pub fn merge(&mut self, additional: Vec<(String, String)>) {
        for (name, value) in additional {
            if !self
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case(&name))
            {
                self.headers.push((name, value));
            }
        }
    }

    pub fn set_if_missing(&mut self, name: &str, value: &str) {
        if !self
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            self.headers.push((name.to_string(), value.to_string()));
        }
    }

    pub fn merge_csv_header(&mut self, name: &str, values: &str) {
        if values.trim().is_empty() {
            return;
        }
        if let Some((_, existing)) = self
            .headers
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            let mut merged: Vec<String> = existing
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
            for value in values
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !merged.iter().any(|item| item == value) {
                    merged.push(value.to_string());
                }
            }
            *existing = merged.join(",");
        } else {
            self.headers.push((name.to_string(), values.to_string()));
        }
    }

    pub fn ensure_claude_files_api_defaults(&mut self) {
        self.set_if_missing("anthropic-version", "2023-06-01");
        self.merge_csv_header("anthropic-beta", "files-api-2025-04-14,oauth-2025-04-20");
    }
}

fn passthrough_contains(headers: Option<&PassthroughHeaders>, name: &str) -> bool {
    headers.is_some_and(|passthrough| {
        passthrough
            .headers
            .iter()
            .any(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
    })
}

fn provider_extra_contains(provider: &ProviderConnection, name: &str) -> bool {
    provider
        .extra_headers
        .keys()
        .any(|header_name| header_name.eq_ignore_ascii_case(name))
}

fn skip_codex_extra_header(provider: &ProviderConnection, name: &str, value: &str) -> bool {
    is_codex_responses_provider(provider)
        && (name.eq_ignore_ascii_case("version")
            || name.eq_ignore_ascii_case("user-agent")
                && value.starts_with("codex_cli_rs/0.124.0 "))
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
    pub response_headers: Vec<(String, String)>,
}

pub fn tee_stream(stream: BoxStream) -> (BoxErrorStream, TeeBuffer) {
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

/// Wrap a stream with idle timeout watchdog.
/// If no data arrives within `idle_timeout`, the stream terminates with an SSE error event.
pub fn watchdog_stream(
    stream: BoxStream,
    idle_timeout: std::time::Duration,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>> {
    let stream = async_stream::stream! {
        let mut inner = Box::pin(stream);
        loop {
            match tokio::time::timeout(idle_timeout, inner.next()).await {
                Ok(Some(Ok(bytes))) => yield Ok(bytes),
                Ok(Some(Err(e))) => {
                    yield Err(Box::new(e) as BoxError);
                    break;
                }
                Ok(None) => break, // stream ended naturally
                Err(_timeout) => {
                    // Idle timeout — send error SSE event and terminate
                    let timeout_secs = idle_timeout.as_secs();
                    let error_event = format!(
                        "data: {{\"error\": {{\"message\": \"stream idle timeout after {}s\", \"type\": \"stream_timeout\"}}}}\n\n",
                        timeout_secs
                    );
                    tracing::warn!(
                        timeout_secs = timeout_secs,
                        "SSE stream idle timeout, closing connection"
                    );
                    yield Ok(Bytes::from(error_event));
                    break;
                }
            }
        }
    };
    Box::pin(stream)
}

/// Like `tee_stream` but accepts `BoxError` streams (for use after watchdog).
pub fn tee_stream_boxerror(stream: BoxErrorStream) -> (BoxErrorStream, TeeBuffer) {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = buffer.clone();
    let teed = stream.map(move |result| match result {
        Ok(bytes) => {
            if let Ok(mut buf) = buf_clone.lock() {
                buf.extend_from_slice(&bytes);
            }
            Ok(bytes)
        }
        Err(e) => Err(e),
    });
    (Box::pin(teed), buffer)
}

#[derive(Clone)]
pub struct UpstreamClient {
    clients: ProxyClientCache,
}

impl Default for UpstreamClient {
    fn default() -> Self {
        Self::new()
    }
}

pub fn provider_with_account_auth(
    provider: &ProviderConnection,
    account: &ProviderAccount,
    provider_id: &str,
) -> ProviderConnection {
    let mut resolved = provider.clone();
    match account.auth_mode {
        ProviderAccountAuthMode::ApiKey => {
            resolved.api_key = account.api_key.clone();
        }
        ProviderAccountAuthMode::OAuth => {
            resolved.api_key = account.access_token.clone();
            if provider_id.eq_ignore_ascii_case("codex") {
                resolved
                    .extra_headers
                    .entry("ChatGPT-Account-ID".to_string())
                    .or_insert_with(|| account.remote_account_id.clone().unwrap_or_default());
                if resolved
                    .extra_headers
                    .get("ChatGPT-Account-ID")
                    .is_some_and(String::is_empty)
                {
                    resolved.extra_headers.remove("ChatGPT-Account-ID");
                }
                // FedRAMP-enrolled workspaces must advertise themselves so the
                // backend routes them through the FedRAMP edge, matching the
                // upstream `BearerAuthProvider::add_auth_headers` behavior.
                if account.is_fedramp {
                    resolved
                        .extra_headers
                        .entry("X-OpenAI-Fedramp".to_string())
                        .or_insert_with(|| "true".to_string());
                }
            }
        }
    }
    resolved
}

fn is_codex_responses_provider(provider: &ProviderConnection) -> bool {
    provider.provider.eq_ignore_ascii_case("codex")
        || provider.model_prefix.eq_ignore_ascii_case("codex")
        || provider.base_url.contains("/backend-api/codex")
}

fn force_codex_responses_stream(
    provider: &ProviderConnection,
    endpoint: EndpointKind,
    request_body: &mut Value,
) {
    if endpoint != EndpointKind::Responses || !is_codex_responses_provider(provider) {
        return;
    }
    if let Some(object) = request_body.as_object_mut() {
        object.insert("stream".to_string(), Value::Bool(true));
    }
}

#[derive(Debug, Clone)]
pub struct PreparedUpstreamRequest {
    pub request_body: Value,
    pub path: String,
    pub url: String,
}

pub fn prepare_upstream_request(
    provider: &ProviderConnection,
    endpoint: EndpointKind,
    suffix: Option<&str>,
    model: &str,
    payload: &Value,
    inject_model: bool,
) -> PreparedUpstreamRequest {
    let mut request_body = payload.clone();
    if inject_model {
        request_body["model"] = Value::String(model.to_string());
    }
    force_codex_responses_stream(provider, endpoint, &mut request_body);
    let stream_requested = request_body
        .get("stream")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let path = endpoint_path(provider, endpoint, suffix, stream_requested);
    let url = build_url(&provider.base_url, &path);
    PreparedUpstreamRequest {
        request_body,
        path,
        url,
    }
}

impl UpstreamClient {
    pub fn new() -> Self {
        Self::with_clients(ProxyClientCache::new())
    }

    pub fn with_clients(clients: ProxyClientCache) -> Self {
        Self { clients }
    }

    fn client_for(&self, proxy_url: Option<&str>) -> AppResult<Client> {
        self.clients.for_optional(proxy_url)
    }

    /// Extract response headers that should be forwarded back to the client.
    /// Captures rate-limit headers, retry-after, request-id, and Codex turn state from upstream.
    fn extract_response_headers(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
        const FORWARD_PREFIXES: &[&str] = &["anthropic-ratelimit-", "x-ratelimit-", "x-codex-"];
        const FORWARD_EXACT: &[&str] = &[
            "retry-after",
            "request-id",
            "x-request-id",
            "openai-model",
            "x-reasoning-included",
        ];
        let mut result = Vec::new();
        for (name, value) in headers.iter() {
            let name = name.as_str();
            let matched = FORWARD_EXACT.contains(&name)
                || FORWARD_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix));
            if matched && let Ok(value) = value.to_str() {
                result.push((name.to_string(), value.to_string()));
            }
        }
        result
    }

    fn is_streaming_response(endpoint: EndpointKind, content_type: Option<&str>) -> bool {
        let Some(content_type) = content_type
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        let content_type = content_type.to_ascii_lowercase();
        if content_type.contains("text/event-stream") || content_type.contains("event-stream") {
            return true;
        }
        endpoint == EndpointKind::Responses
            && !content_type.contains("application/json")
            && !content_type.contains("application/problem+json")
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        provider: &ProviderConnection,
        endpoint: EndpointKind,
        suffix: Option<&str>,
        model: &str,
        payload: &Value,
        inject_model: bool,
        passthrough_headers: Option<&PassthroughHeaders>,
        proxy_url: Option<&str>,
    ) -> AppResult<UpstreamResult> {
        let prepared =
            prepare_upstream_request(provider, endpoint, suffix, model, payload, inject_model);
        self.execute_prepared(
            provider,
            endpoint,
            model,
            &prepared,
            passthrough_headers,
            proxy_url,
        )
        .await
    }

    pub async fn execute_prepared(
        &self,
        provider: &ProviderConnection,
        endpoint: EndpointKind,
        model: &str,
        prepared: &PreparedUpstreamRequest,
        passthrough_headers: Option<&PassthroughHeaders>,
        proxy_url: Option<&str>,
    ) -> AppResult<UpstreamResult> {
        let client = self.client_for(proxy_url)?;
        let request_body = &prepared.request_body;
        if endpoint == EndpointKind::Search {
            let spec = build_search_request(provider, request_body, &prepared.url)?;
            let builder = match spec.method {
                SearchHttpMethod::Get => client.request(Method::GET, spec.url),
                SearchHttpMethod::Post => {
                    let builder = client.request(Method::POST, spec.url);
                    if let Some(body) = spec.body {
                        builder.json(&body)
                    } else {
                        builder
                    }
                }
            };
            let builder = apply_provider_headers(builder, provider, passthrough_headers);
            let builder = apply_passthrough_headers(builder, passthrough_headers);
            let response = builder.send().await?;
            let status = response.status();
            let resp_headers = Self::extract_response_headers(response.headers());
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
                serde_json::to_string(&normalize_search_response(
                    &provider.provider,
                    query,
                    search_type,
                    json_body,
                ))?
            } else {
                body
            };
            Ok(UpstreamResult::Buffered(ProviderResponse {
                status,
                body,
                is_stream: false,
                response_headers: resp_headers,
            }))
        } else {
            let builder = client
                .request(Method::POST, &prepared.url)
                .json(request_body);
            let builder = apply_provider_headers(builder, provider, passthrough_headers);
            let builder = apply_passthrough_headers(builder, passthrough_headers);
            let response = builder.send().await?;
            let status = response.status();
            let is_stream = Self::is_streaming_response(
                endpoint,
                response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok()),
            );
            let resp_headers = Self::extract_response_headers(response.headers());

            if is_stream && status.is_success() {
                Ok(UpstreamResult::Streaming(StreamingProviderResponse {
                    status,
                    provider_id: provider.id.clone(),
                    model: model.to_string(),
                    stream: Box::pin(response.bytes_stream()),
                    response_headers: resp_headers,
                }))
            } else if is_stream {
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
                    response_headers: resp_headers,
                }))
            } else {
                let body = response.text().await?;
                Ok(UpstreamResult::Buffered(ProviderResponse {
                    status,
                    body,
                    is_stream: false,
                    response_headers: resp_headers,
                }))
            }
        }
    }

    /// Sends a raw request to an arbitrary path on the provider's `base_url`.
    ///
    /// Used for passthrough endpoints like `/v1/messages/count_tokens` where
    /// the upstream bytes and content type must be preserved.
    pub async fn execute_raw_proxy(
        &self,
        provider: &ProviderConnection,
        method: Method,
        path: &str,
        payload: Option<&Value>,
        passthrough_headers: Option<&PassthroughHeaders>,
        proxy_url: Option<&str>,
    ) -> AppResult<(StatusCode, Vec<(String, String)>, Option<String>, Vec<u8>)> {
        let url = build_url(&provider.base_url, path);
        let client = self.client_for(proxy_url)?;
        let builder = client.request(method, url);
        let builder = if let Some(payload) = payload {
            builder.json(payload)
        } else {
            builder
        };
        let builder = apply_provider_headers(builder, provider, passthrough_headers);
        let builder = apply_passthrough_headers(builder, passthrough_headers);
        let response = builder.send().await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let resp_headers = Self::extract_response_headers(response.headers());
        let body = response.bytes().await?.to_vec();
        Ok((status, resp_headers, content_type, body))
    }

    pub async fn execute_audio_speech(
        &self,
        provider: &ProviderConnection,
        model: &str,
        payload: &Value,
        proxy_url: Option<&str>,
    ) -> AppResult<AudioResponse> {
        let path = endpoint_path(provider, EndpointKind::AudioSpeech, None, false);
        let url = build_url(&provider.base_url, &path);
        let mut request_body = payload.clone();
        request_body["model"] = Value::String(model.to_string());

        let client = self.client_for(proxy_url)?;
        let builder = client.request(Method::POST, url).json(&request_body);
        let response = apply_provider_headers(builder, provider, None)
            .send()
            .await?;
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
        proxy_url: Option<&str>,
    ) -> AppResult<AudioResponse> {
        let path = endpoint_path(provider, EndpointKind::AudioTranscriptions, None, false);
        let url = build_url(&provider.base_url, &path);

        let file_part = multipart::Part::bytes(payload.bytes.clone())
            .file_name(payload.filename.clone())
            .mime_str(
                payload
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
            )?;
        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", model.to_string());
        for (key, value) in &payload.text_fields {
            if key != "model" && key != "file" {
                form = form.text(key.clone(), value.clone());
            }
        }

        let client = self.client_for(proxy_url)?;
        let builder = client.request(Method::POST, url).multipart(form);
        let response = apply_provider_headers(builder, provider, None)
            .send()
            .await?;
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
    pub response_headers: Vec<(String, String)>,
}

impl ProviderResponse {
    pub fn as_attempt(self, provider_id: String, model: String) -> ProviderChatAttempt {
        ProviderChatAttempt {
            provider_id,
            model,
            status: self.status.as_u16(),
            body: self.body,
            account: None,
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
            account: None,
        }
    }

    pub fn body_preview(&self) -> String {
        String::from_utf8_lossy(&self.bytes)
            .chars()
            .take(4000)
            .collect()
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

fn apply_passthrough_headers(
    mut builder: RequestBuilder,
    headers: Option<&PassthroughHeaders>,
) -> RequestBuilder {
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

fn apply_provider_headers(
    mut builder: RequestBuilder,
    provider: &ProviderConnection,
    passthrough_headers: Option<&PassthroughHeaders>,
) -> RequestBuilder {
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

    if is_codex_responses_provider(provider) {
        if !provider_extra_contains(provider, CODEX_ORIGINATOR_HEADER)
            && !passthrough_contains(passthrough_headers, CODEX_ORIGINATOR_HEADER)
        {
            builder = builder.header(CODEX_ORIGINATOR_HEADER, CODEX_ORIGINATOR_VALUE);
        }
        if !provider_extra_contains(provider, OPENAI_BETA_HEADER)
            && !passthrough_contains(passthrough_headers, OPENAI_BETA_HEADER)
        {
            builder = builder.header(OPENAI_BETA_HEADER, OPENAI_BETA_RESPONSES_VALUE);
        }
    }

    for (key, value) in &provider.extra_headers {
        if passthrough_contains(passthrough_headers, key)
            || skip_codex_extra_header(provider, key, value)
        {
            continue;
        }
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
    if stream_requested
        && let Some(path) =
            provider_path_override(&provider.stream_endpoint_paths, endpoint, suffix)
    {
        return path;
    }
    if let Some(path) = provider_path_override(&provider.endpoint_paths, endpoint, suffix) {
        return path;
    }

    let default_path = match endpoint {
        EndpointKind::ChatCompletions | EndpointKind::Completions => {
            "/chat/completions".to_string()
        }
        EndpointKind::Messages => "/messages".to_string(),
        EndpointKind::Responses => "/responses".to_string(),
        EndpointKind::Files => "/files".to_string(),
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
    for key in [
        Some(endpoint.as_str()),
        Some(endpoint.capability()),
        chat_family_override_key(endpoint),
    ]
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

fn append_suffix_if_needed(
    endpoint: EndpointKind,
    base_path: String,
    suffix: Option<&str>,
) -> String {
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
    use crate::models::EndpointKind;
    #[allow(unused_imports)]
    use serde_json::json;
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
        assert_eq!(
            endpoint_path(&p, EndpointKind::ChatCompletions, None, false),
            "/chat/completions"
        );
    }

    #[test]
    fn test_endpoint_path_embeddings_default() {
        let p = make_provider();
        assert_eq!(
            endpoint_path(&p, EndpointKind::Embeddings, None, false),
            "/embeddings"
        );
    }

    #[test]
    fn test_endpoint_path_messages_default() {
        let p = make_provider();
        assert_eq!(
            endpoint_path(&p, EndpointKind::Messages, None, false),
            "/messages"
        );
    }

    #[test]
    fn test_endpoint_path_all_defaults() {
        let p = make_provider();
        let cases: Vec<(EndpointKind, &str)> = vec![
            (EndpointKind::ChatCompletions, "/chat/completions"),
            (EndpointKind::Completions, "/chat/completions"),
            (EndpointKind::Messages, "/messages"),
            (EndpointKind::Responses, "/responses"),
            (EndpointKind::Files, "/files"),
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
            assert_eq!(
                endpoint_path(&p, kind, None, false),
                expected,
                "failed for {:?}",
                kind
            );
        }
    }

    #[test]
    fn test_endpoint_path_override() {
        let mut p = make_provider();
        p.endpoint_paths
            .insert("chat.completions".to_string(), "/v1/custom".to_string());
        assert_eq!(
            endpoint_path(&p, EndpointKind::ChatCompletions, None, false),
            "/v1/custom"
        );
    }

    #[test]
    fn test_endpoint_path_stream_override() {
        let mut p = make_provider();
        p.stream_endpoint_paths.insert(
            "chat.completions".to_string(),
            "/v1/stream-chat".to_string(),
        );
        // stream_requested=true should use the stream override
        assert_eq!(
            endpoint_path(&p, EndpointKind::ChatCompletions, None, true),
            "/v1/stream-chat"
        );
        // stream_requested=false should fall through to default
        assert_eq!(
            endpoint_path(&p, EndpointKind::ChatCompletions, None, false),
            "/chat/completions"
        );
    }

    #[test]
    fn test_force_codex_responses_stream_overrides_non_stream_request() {
        let mut provider = make_provider();
        provider.provider = "codex".to_string();
        provider.model_prefix = "codex".to_string();
        provider.base_url = "https://chatgpt.com/backend-api/codex".to_string();

        let mut body = serde_json::json!({
            "model": "codex/gpt-5.5",
            "stream": false
        });

        force_codex_responses_stream(&provider, EndpointKind::Responses, &mut body);

        assert_eq!(body["stream"], serde_json::json!(true));
    }

    #[test]
    fn test_endpoint_path_responses_suffix() {
        let p = make_provider();
        assert_eq!(
            endpoint_path(&p, EndpointKind::Responses, Some("native/path"), false),
            "/responses/native/path"
        );
    }

    #[test]
    fn test_endpoint_path_responses_placeholder_suffix() {
        let mut p = make_provider();
        p.endpoint_paths.insert(
            "responses".to_string(),
            "/v1/responses/{suffix}".to_string(),
        );
        assert_eq!(
            endpoint_path(&p, EndpointKind::Responses, Some("abc"), false),
            "/v1/responses/abc"
        );
        // Empty suffix replaces placeholder with empty string
        assert_eq!(
            endpoint_path(&p, EndpointKind::Responses, None, false),
            "/v1/responses/"
        );
    }

    #[test]
    fn test_build_url_relative() {
        assert_eq!(
            build_url("https://api.test.com", "/chat/completions"),
            "https://api.test.com/chat/completions"
        );
    }

    #[test]
    fn test_is_streaming_response_accepts_responses_octet_stream() {
        assert!(UpstreamClient::is_streaming_response(
            EndpointKind::Responses,
            Some("application/octet-stream"),
        ));
        assert!(!UpstreamClient::is_streaming_response(
            EndpointKind::ChatCompletions,
            Some("application/octet-stream"),
        ));
    }

    #[test]
    fn test_is_streaming_response_rejects_json() {
        assert!(!UpstreamClient::is_streaming_response(
            EndpointKind::Responses,
            Some("application/json; charset=utf-8"),
        ));
    }

    #[test]
    fn test_build_url_absolute() {
        assert_eq!(
            build_url("https://api.test.com", "http://other.com/path"),
            "http://other.com/path"
        );
        assert_eq!(
            build_url("https://api.test.com", "https://other.com/path"),
            "https://other.com/path"
        );
    }

    #[test]
    fn test_passthrough_headers_merge_csv_header_deduplicates() {
        let mut headers = PassthroughHeaders {
            headers: vec![("anthropic-beta".to_string(), "oauth-2025-04-20".to_string())],
        };
        headers.merge_csv_header("anthropic-beta", "files-api-2025-04-14,oauth-2025-04-20");
        let value = headers
            .headers
            .iter()
            .find(|(name, _)| name == "anthropic-beta")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert_eq!(value, "oauth-2025-04-20,files-api-2025-04-14");
    }

    #[test]
    fn test_passthrough_headers_ensure_claude_files_api_defaults() {
        let mut headers = PassthroughHeaders::default();
        headers.ensure_claude_files_api_defaults();
        assert!(
            headers
                .headers
                .iter()
                .any(|(name, value)| name == "anthropic-version" && value == "2023-06-01")
        );
        assert!(headers.headers.iter().any(|(name, value)| {
            name == "anthropic-beta"
                && value.contains("files-api-2025-04-14")
                && value.contains("oauth-2025-04-20")
        }));
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
        assert!(fallback_error(
            StatusCode::BAD_REQUEST,
            "Hit rate limit on this endpoint"
        ));
    }

    #[test]
    fn test_fallback_error_quota_body() {
        assert!(fallback_error(
            StatusCode::BAD_REQUEST,
            "You have exceeded your quota"
        ));
    }

    #[test]
    fn test_fallback_error_400() {
        assert!(!fallback_error(
            StatusCode::BAD_REQUEST,
            "invalid request body"
        ));
    }

    #[test]
    fn test_openai_error_format() {
        let err = openai_error("something went wrong");
        assert_eq!(err["error"]["message"], "something went wrong");
        assert_eq!(err["error"]["type"], "upstream_error");
    }
}
