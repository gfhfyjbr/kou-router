use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    auth::AuthContext,
    cost::UsageInfo,
    error::{AppError, AppResult},
    models::EndpointKind,
    upstream::{PassthroughHeaders, PreparedUpstreamRequest},
};

const CACHE_HEADER: &str = "x-kou-response-cache";
const CACHE_TTL_HEADER: &str = "x-kou-response-cache-ttl";
const CACHE_NAMESPACE_HEADER: &str = "x-kou-response-cache-namespace";
const CACHE_ALLOW_NONDETERMINISTIC_HEADER: &str = "x-kou-response-cache-allow-nondeterministic";
const MAX_TTL_SECS: i64 = 3600;

#[derive(Debug, Clone, Default)]
pub struct ResponseCacheContext {
    pub read: bool,
    pub write: bool,
    pub ttl: Option<Duration>,
    pub namespace: String,
    pub allow_nondeterministic_chat: bool,
}

impl ResponseCacheContext {
    pub fn from_headers(headers: &HeaderMap, auth: &AuthContext) -> AppResult<Self> {
        let mode = header_value(headers, CACHE_HEADER)
            .map(str::trim)
            .map(str::to_ascii_lowercase);
        let Some(mode) = mode else {
            return Ok(Self {
                namespace: auth_namespace(auth, None),
                ..Self::default()
            });
        };

        let (read, write) = match mode.as_str() {
            "" | "0" | "false" | "off" | "none" | "no" => (false, false),
            "1" | "true" | "on" | "read-write" | "readwrite" | "rw" => (true, true),
            "read" | "read-only" | "readonly" => (true, false),
            "write" | "write-only" | "writeonly" => (false, true),
            other => {
                return Err(AppError::BadRequest(format!(
                    "invalid {CACHE_HEADER} value '{other}'"
                )));
            }
        };

        let namespace_override = header_value(headers, CACHE_NAMESPACE_HEADER)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let ttl = header_value(headers, CACHE_TTL_HEADER)
            .map(parse_ttl)
            .transpose()?;
        let allow_nondeterministic_chat =
            header_value(headers, CACHE_ALLOW_NONDETERMINISTIC_HEADER)
                .map(parse_bool_header)
                .transpose()?
                .unwrap_or(false);

        Ok(Self {
            read,
            write,
            ttl,
            namespace: auth_namespace(auth, namespace_override),
            allow_nondeterministic_chat,
        })
    }

    pub fn enabled(&self) -> bool {
        self.read || self.write
    }
}

#[derive(Debug, Clone)]
pub struct ResponseCacheIdentity {
    pub cache_key: String,
    pub request_body_hash: String,
}

#[derive(Debug, Clone)]
pub struct ResponseCachePolicy {
    pub cacheable: bool,
    pub reason: Option<String>,
}

impl ResponseCachePolicy {
    pub fn allow() -> Self {
        Self {
            cacheable: true,
            reason: None,
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Self {
            cacheable: false,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewResponseCacheEntry {
    pub cache_key: String,
    pub endpoint: String,
    pub upstream_endpoint: String,
    pub requested_model: String,
    pub resolved_model: String,
    pub provider_id: String,
    pub provider_account_id: Option<String>,
    pub namespace: String,
    pub request_body_hash: String,
    pub response_body: String,
    pub response_headers: Vec<(String, String)>,
    pub status: i64,
    pub is_stream: bool,
    pub usage: Option<UsageInfo>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ResponseCacheEntry {
    pub cache_key: String,
    pub endpoint: String,
    pub upstream_endpoint: String,
    pub requested_model: String,
    pub resolved_model: String,
    pub provider_id: String,
    pub provider_account_id: Option<String>,
    pub namespace: String,
    pub request_body_hash: String,
    pub response_body: String,
    pub response_headers: Vec<(String, String)>,
    pub status: i64,
    pub is_stream: bool,
    pub usage: Option<UsageInfo>,
    pub expires_at: DateTime<Utc>,
    pub hit_count: i64,
}

#[allow(clippy::too_many_arguments)]
pub fn build_identity(
    endpoint: EndpointKind,
    upstream_endpoint: EndpointKind,
    requested_model: &str,
    resolved_model: &str,
    provider_id: &str,
    provider_account_id: Option<&str>,
    namespace: &str,
    prepared: &PreparedUpstreamRequest,
    client_stream_requested: bool,
) -> ResponseCacheIdentity {
    let request_body_hash = canonical_json_hash(&prepared.request_body);
    let account = provider_account_id.unwrap_or("");
    let key_material = format!(
        "v1\nendpoint={}\nupstream_endpoint={}\nrequested_model={}\nresolved_model={}\nprovider_id={}\nprovider_account_id={}\nnamespace={}\npath={}\nclient_stream={}\nbody={}",
        endpoint.as_str(),
        upstream_endpoint.as_str(),
        requested_model,
        resolved_model,
        provider_id,
        account,
        namespace,
        prepared.path,
        client_stream_requested,
        request_body_hash,
    );
    ResponseCacheIdentity {
        cache_key: sha256_hex(key_material.as_bytes()),
        request_body_hash,
    }
}

pub fn evaluate_policy(
    endpoint: EndpointKind,
    request_body: &Value,
    passthrough: Option<&PassthroughHeaders>,
    allow_nondeterministic_chat: bool,
) -> ResponseCachePolicy {
    if !matches!(
        endpoint,
        EndpointKind::ChatCompletions
            | EndpointKind::Completions
            | EndpointKind::Messages
            | EndpointKind::Responses
            | EndpointKind::OllamaChat
            | EndpointKind::Embeddings
            | EndpointKind::Moderations
            | EndpointKind::Rerank
    ) {
        return ResponseCachePolicy::deny("endpoint is not response-cacheable");
    }

    if endpoint.is_chat_family()
        && !allow_nondeterministic_chat
        && !temperature_is_zero(request_body)
    {
        return ResponseCachePolicy::deny("chat-family requests require temperature: 0");
    }

    if truthy_field(request_body, "store") {
        return ResponseCachePolicy::deny("store:true is not response-cacheable");
    }

    for field in [
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "web_search_options",
        "file_search",
        "files",
        "attachments",
        "previous_response_id",
    ] {
        if non_empty_field(request_body, field) {
            return ResponseCachePolicy::deny(format!("{field} is not response-cacheable"));
        }
    }

    for field in [
        "image_url",
        "input_image",
        "input_audio",
        "modalities",
        "audio",
        "video",
    ] {
        if contains_key_recursive(request_body, field) {
            return ResponseCachePolicy::deny(format!("{field} content is not response-cacheable"));
        }
    }

    if let Some(header) = live_passthrough_header(passthrough) {
        return ResponseCachePolicy::deny(format!("{header} passthrough header is live context"));
    }

    ResponseCachePolicy::allow()
}

pub fn cacheable_response_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    const SAFE_EXACT: &[&str] = &["openai-model", "x-reasoning-included"];
    headers
        .iter()
        .filter(|(name, _)| SAFE_EXACT.contains(&name.to_ascii_lowercase().as_str()))
        .cloned()
        .collect()
}

pub fn canonical_json_hash(value: &Value) -> String {
    let mut canonical = String::new();
    write_canonical_json(value, &mut canonical);
    sha256_hex(canonical.as_bytes())
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn parse_ttl(value: &str) -> AppResult<Duration> {
    let value = value.trim();
    let (number, multiplier) = if let Some(raw) = value.strip_suffix('s') {
        (raw, 1)
    } else if let Some(raw) = value.strip_suffix('m') {
        (raw, 60)
    } else if let Some(raw) = value.strip_suffix('h') {
        (raw, 3600)
    } else {
        (value, 1)
    };
    let amount = number
        .parse::<i64>()
        .map_err(|_| AppError::BadRequest(format!("invalid {CACHE_TTL_HEADER} value '{value}'")))?;
    let seconds = amount.saturating_mul(multiplier);
    if seconds <= 0 || seconds > MAX_TTL_SECS {
        return Err(AppError::BadRequest(format!(
            "{CACHE_TTL_HEADER} must be between 1s and {MAX_TTL_SECS}s"
        )));
    }
    Ok(Duration::seconds(seconds))
}

fn parse_bool_header(value: &str) -> AppResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(AppError::BadRequest(format!(
            "invalid {CACHE_ALLOW_NONDETERMINISTIC_HEADER} value '{other}'"
        ))),
    }
}

fn auth_namespace(auth: &AuthContext, override_namespace: Option<&str>) -> String {
    let base = match auth {
        AuthContext::ApiKey { key_id, .. } => format!("api_key:{key_id}"),
        AuthContext::Admin { username } => format!("admin:{username}"),
        AuthContext::Anonymous => "anonymous".to_string(),
    };
    match override_namespace {
        Some(namespace) => format!("{base}:{namespace}"),
        None => base,
    }
}

fn temperature_is_zero(body: &Value) -> bool {
    body.get("temperature")
        .and_then(Value::as_f64)
        .is_some_and(|value| value == 0.0)
}

fn truthy_field(body: &Value, key: &str) -> bool {
    body.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn non_empty_field(body: &Value, key: &str) -> bool {
    match body.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(false)) => false,
        Some(Value::String(value)) => !value.trim().is_empty() && value != "none",
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::Object(values)) => !values.is_empty(),
        Some(_) => true,
    }
}

fn contains_key_recursive(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(name, value)| name == key || contains_key_recursive(value, key)),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_key_recursive(value, key)),
        _ => false,
    }
}

fn live_passthrough_header(passthrough: Option<&PassthroughHeaders>) -> Option<String> {
    let headers = passthrough?;
    headers.headers.iter().find_map(|(name, _)| {
        let lowered = name.to_ascii_lowercase();
        let live = lowered == "session-id"
            || lowered == "thread-id"
            || lowered == "traceparent"
            || lowered == "tracestate"
            || lowered.starts_with("x-codex-")
            || lowered.starts_with("x-claude-")
            || lowered.starts_with("x-openai-")
            || lowered == "anthropic-beta"
            || lowered == "anthropic-version";
        live.then_some(name.clone())
    })
}

fn write_canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => out.push_str(&value.to_string()),
        Value::String(value) => out.push_str(&serde_json::to_string(value).unwrap_or_default()),
        Value::Array(values) => {
            out.push('[');
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_canonical_json(value, out);
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_by_key(|(left, _)| *left);
            out.push('{');
            for (idx, (key, value)) in entries.into_iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                write_canonical_json(value, out);
            }
            out.push('}');
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_hash_ignores_object_order() {
        let left = json!({"b": [2, {"d": 4, "c": 3}], "a": 1});
        let right = json!({"a": 1, "b": [2, {"c": 3, "d": 4}]});
        assert_eq!(canonical_json_hash(&left), canonical_json_hash(&right));
    }

    #[test]
    fn policy_requires_temperature_zero_for_chat() {
        let body = json!({"model": "x", "messages": [], "temperature": 0});
        assert!(evaluate_policy(EndpointKind::ChatCompletions, &body, None, false).cacheable);

        let body = json!({"model": "x", "messages": []});
        assert!(!evaluate_policy(EndpointKind::ChatCompletions, &body, None, false).cacheable);
        assert!(evaluate_policy(EndpointKind::ChatCompletions, &body, None, true).cacheable);
    }

    #[test]
    fn policy_rejects_tools_and_live_headers() {
        let body = json!({"model": "x", "messages": [], "temperature": 0, "tools": [{"type": "function"}]});
        assert!(!evaluate_policy(EndpointKind::ChatCompletions, &body, None, false).cacheable);

        let body = json!({"model": "x", "messages": [], "temperature": 0});
        let passthrough = PassthroughHeaders {
            headers: vec![("x-codex-turn-state".to_string(), "live".to_string())],
        };
        assert!(
            !evaluate_policy(
                EndpointKind::ChatCompletions,
                &body,
                Some(&passthrough),
                false
            )
            .cacheable
        );
    }
}
