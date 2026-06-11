use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("classified upstream error")]
    ClassifiedUpstream { status: StatusCode, body: Value },
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl AppError {
    /// HTTP status this error maps to (mirrors the IntoResponse impl).
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::ClassifiedUpstream { status, .. } => *status,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
            AppError::Database(_) | AppError::Http(_) | AppError::Json(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::ClassifiedUpstream { status, body } => (status, Json(body)).into_response(),
            other => {
                let status = match &other {
                    AppError::NotFound(_) => StatusCode::NOT_FOUND,
                    AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
                    AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
                    AppError::Database(_) | AppError::Http(_) | AppError::Json(_) => {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                    AppError::ClassifiedUpstream { .. } => unreachable!(),
                };
                let body = Json(json!({
                    "error": {
                        "message": other.to_string(),
                        "type": "kou_router_error"
                    }
                }));
                (status, body).into_response()
            }
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ── Upstream error classification ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamErrorKind {
    RateLimit,
    Overloaded,
    PromptTooLong,
    InvalidRequest,
    AuthenticationFailed,
    ModelNotFound,
    ContentFiltered,
    Timeout,
    ServerError,
    ConnectionError,
    Unknown,
}

impl UpstreamErrorKind {
    /// Whether this error kind is retriable (used by retry engine).
    pub fn is_retriable(self) -> bool {
        matches!(
            self,
            UpstreamErrorKind::RateLimit
                | UpstreamErrorKind::Overloaded
                | UpstreamErrorKind::Timeout
                | UpstreamErrorKind::ServerError
                | UpstreamErrorKind::ConnectionError
        )
    }

    /// Map to HTTP status code for the client response.
    pub fn http_status(self) -> StatusCode {
        match self {
            UpstreamErrorKind::RateLimit => StatusCode::TOO_MANY_REQUESTS,
            UpstreamErrorKind::Overloaded => StatusCode::SERVICE_UNAVAILABLE,
            UpstreamErrorKind::PromptTooLong => StatusCode::BAD_REQUEST,
            UpstreamErrorKind::InvalidRequest => StatusCode::BAD_REQUEST,
            UpstreamErrorKind::AuthenticationFailed => StatusCode::UNAUTHORIZED,
            UpstreamErrorKind::ModelNotFound => StatusCode::NOT_FOUND,
            UpstreamErrorKind::ContentFiltered => StatusCode::BAD_REQUEST,
            UpstreamErrorKind::Timeout => StatusCode::GATEWAY_TIMEOUT,
            UpstreamErrorKind::ServerError => StatusCode::BAD_GATEWAY,
            UpstreamErrorKind::ConnectionError => StatusCode::BAD_GATEWAY,
            UpstreamErrorKind::Unknown => StatusCode::BAD_GATEWAY,
        }
    }
}

/// Classify an upstream error from HTTP status + response body.
pub fn classify_upstream_error(status: StatusCode, body: &str) -> UpstreamErrorKind {
    let lower = body.to_ascii_lowercase();

    // Status-based classification first
    match status.as_u16() {
        429 => return UpstreamErrorKind::RateLimit,
        401 | 403 => return UpstreamErrorKind::AuthenticationFailed,
        404 => {
            if lower.contains("model") {
                return UpstreamErrorKind::ModelNotFound;
            }
            return UpstreamErrorKind::ModelNotFound;
        }
        408 => return UpstreamErrorKind::Timeout,
        529 => return UpstreamErrorKind::Overloaded,
        _ => {}
    }

    // Body pattern classification
    if lower.contains("prompt is too long")
        || lower.contains("context_length_exceeded")
        || lower.contains("max_tokens")
        || lower.contains("maximum context length")
        || lower.contains("too many tokens")
    {
        return UpstreamErrorKind::PromptTooLong;
    }

    if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("quota")
        || lower.contains("too many requests")
    {
        return UpstreamErrorKind::RateLimit;
    }

    if lower.contains("overloaded")
        || lower.contains("capacity")
        || lower.contains("temporarily unavailable")
    {
        return UpstreamErrorKind::Overloaded;
    }

    if lower.contains("content_filter")
        || lower.contains("content filter")
        || lower.contains("safety")
        || lower.contains("blocked")
        || lower.contains("content_policy")
    {
        return UpstreamErrorKind::ContentFiltered;
    }

    if lower.contains("authentication")
        || lower.contains("invalid.*key")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("incorrect api key")
    {
        return UpstreamErrorKind::AuthenticationFailed;
    }

    // Status-based fallback
    match status.as_u16() {
        400 => UpstreamErrorKind::InvalidRequest,
        500 | 502 | 503 => UpstreamErrorKind::ServerError,
        _ if status.is_server_error() => UpstreamErrorKind::ServerError,
        _ => UpstreamErrorKind::Unknown,
    }
}

/// Build an enriched error response JSON body.
pub fn enriched_error_response(
    kind: UpstreamErrorKind,
    status: StatusCode,
    body: &str,
    provider_id: &str,
    retry_after_secs: Option<u64>,
) -> Value {
    let message = classified_upstream_message(kind, body);

    let mut error_obj = json!({
        "error": {
            "message": message,
            "type": kind,
            "upstream_status": status.as_u16(),
            "provider_id": provider_id,
            "retriable": kind.is_retriable()
        }
    });

    if let Some(secs) = retry_after_secs {
        error_obj["error"]["retry_after_secs"] = json!(secs);
    }

    error_obj
}

fn classified_upstream_message(kind: UpstreamErrorKind, body: &str) -> String {
    let classified = format!("upstream provider returned {} error", kind_label(kind));
    match extract_upstream_error_text(body) {
        Some(upstream_text) => format!("{classified}: {upstream_text}"),
        None => classified,
    }
}

fn extract_upstream_error_text(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| {
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
        })
        .or_else(|| Some(trimmed.to_string()))
}

fn kind_label(kind: UpstreamErrorKind) -> &'static str {
    match kind {
        UpstreamErrorKind::RateLimit => "rate limit",
        UpstreamErrorKind::Overloaded => "overloaded",
        UpstreamErrorKind::PromptTooLong => "prompt too long",
        UpstreamErrorKind::InvalidRequest => "invalid request",
        UpstreamErrorKind::AuthenticationFailed => "authentication",
        UpstreamErrorKind::ModelNotFound => "model not found",
        UpstreamErrorKind::ContentFiltered => "content filtered",
        UpstreamErrorKind::Timeout => "timeout",
        UpstreamErrorKind::ServerError => "server",
        UpstreamErrorKind::ConnectionError => "connection",
        UpstreamErrorKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_429_rate_limit() {
        assert_eq!(
            classify_upstream_error(StatusCode::TOO_MANY_REQUESTS, ""),
            UpstreamErrorKind::RateLimit
        );
    }

    #[test]
    fn test_classify_529_overloaded() {
        let status = StatusCode::from_u16(529).unwrap();
        assert_eq!(
            classify_upstream_error(status, ""),
            UpstreamErrorKind::Overloaded
        );
    }

    #[test]
    fn test_classify_401_auth() {
        assert_eq!(
            classify_upstream_error(StatusCode::UNAUTHORIZED, ""),
            UpstreamErrorKind::AuthenticationFailed
        );
    }

    #[test]
    fn test_classify_403_auth() {
        assert_eq!(
            classify_upstream_error(StatusCode::FORBIDDEN, ""),
            UpstreamErrorKind::AuthenticationFailed
        );
    }

    #[test]
    fn test_classify_404_model_not_found() {
        assert_eq!(
            classify_upstream_error(StatusCode::NOT_FOUND, "model not found"),
            UpstreamErrorKind::ModelNotFound
        );
    }

    #[test]
    fn test_classify_400_prompt_too_long() {
        assert_eq!(
            classify_upstream_error(StatusCode::BAD_REQUEST, "prompt is too long for this model"),
            UpstreamErrorKind::PromptTooLong
        );
        assert_eq!(
            classify_upstream_error(
                StatusCode::BAD_REQUEST,
                "{\"error\": {\"code\": \"context_length_exceeded\"}}"
            ),
            UpstreamErrorKind::PromptTooLong
        );
    }

    #[test]
    fn test_classify_400_content_filter() {
        assert_eq!(
            classify_upstream_error(StatusCode::BAD_REQUEST, "content_filter triggered"),
            UpstreamErrorKind::ContentFiltered
        );
    }

    #[test]
    fn test_classify_400_rate_limit_in_body() {
        assert_eq!(
            classify_upstream_error(StatusCode::BAD_REQUEST, "Hit rate limit on this endpoint"),
            UpstreamErrorKind::RateLimit
        );
    }

    #[test]
    fn test_classify_400_generic() {
        assert_eq!(
            classify_upstream_error(StatusCode::BAD_REQUEST, "invalid request body"),
            UpstreamErrorKind::InvalidRequest
        );
    }

    #[test]
    fn test_classify_502_server_error() {
        assert_eq!(
            classify_upstream_error(StatusCode::BAD_GATEWAY, ""),
            UpstreamErrorKind::ServerError
        );
    }

    #[test]
    fn test_classify_500_overloaded_in_body() {
        assert_eq!(
            classify_upstream_error(StatusCode::INTERNAL_SERVER_ERROR, "server is overloaded"),
            UpstreamErrorKind::Overloaded
        );
    }

    #[test]
    fn test_retriable_kinds() {
        assert!(UpstreamErrorKind::RateLimit.is_retriable());
        assert!(UpstreamErrorKind::Overloaded.is_retriable());
        assert!(UpstreamErrorKind::Timeout.is_retriable());
        assert!(UpstreamErrorKind::ServerError.is_retriable());
        assert!(UpstreamErrorKind::ConnectionError.is_retriable());
        assert!(!UpstreamErrorKind::InvalidRequest.is_retriable());
        assert!(!UpstreamErrorKind::AuthenticationFailed.is_retriable());
        assert!(!UpstreamErrorKind::PromptTooLong.is_retriable());
        assert!(!UpstreamErrorKind::ContentFiltered.is_retriable());
        assert!(!UpstreamErrorKind::ModelNotFound.is_retriable());
    }

    #[test]
    fn test_enriched_error_response_format() {
        let body = enriched_error_response(
            UpstreamErrorKind::RateLimit,
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\": {\"message\": \"Rate limit exceeded\"}}",
            "anthropic-main",
            Some(30),
        );
        assert_eq!(body["error"]["type"], "rate_limit");
        assert_eq!(body["error"]["upstream_status"], 429);
        assert_eq!(body["error"]["provider_id"], "anthropic-main");
        assert_eq!(body["error"]["retriable"], true);
        assert_eq!(body["error"]["retry_after_secs"], 30);
        assert_eq!(
            body["error"]["message"],
            "upstream provider returned rate limit error: Rate limit exceeded"
        );
    }

    #[test]
    fn test_enriched_error_response_plain_text_body() {
        let body = enriched_error_response(
            UpstreamErrorKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "bad input",
            "openai-main",
            None,
        );
        assert_eq!(
            body["error"]["message"],
            "upstream provider returned invalid request error: bad input"
        );
    }

    #[test]
    fn test_enriched_error_response_no_retry_after() {
        let body = enriched_error_response(
            UpstreamErrorKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "bad input",
            "openai-main",
            None,
        );
        assert_eq!(body["error"]["retriable"], false);
        assert!(body["error"].get("retry_after_secs").is_none());
    }

    #[test]
    fn test_enriched_error_response_empty_body_falls_back_to_classified_message() {
        let body = enriched_error_response(
            UpstreamErrorKind::ServerError,
            StatusCode::BAD_GATEWAY,
            "   ",
            "provider-main",
            None,
        );
        assert_eq!(
            body["error"]["message"],
            "upstream provider returned server error"
        );
    }
}
