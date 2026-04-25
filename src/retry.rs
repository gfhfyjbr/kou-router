use std::time::Duration;

use rand::Rng;

use crate::{
    error::{AppResult, UpstreamErrorKind, classify_upstream_error},
    models::{EndpointKind, ProviderConnection},
    upstream::{
        PassthroughHeaders, PreparedUpstreamRequest, UpstreamClient, UpstreamResult,
    },
};

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts per provider (0 = no retries).
    pub max_retries: u32,
    /// Base delay in milliseconds for exponential backoff.
    pub base_delay_ms: u64,
    /// Maximum delay cap in milliseconds.
    pub max_delay_ms: u64,
    /// Whether to respect `Retry-After` headers from upstream.
    pub respect_retry_after: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 500,
            max_delay_ms: 16_000,
            respect_retry_after: true,
        }
    }
}

impl RetryConfig {
    /// Load from environment variables with defaults.
    pub fn from_env() -> Self {
        let max_retries = std::env::var("KOU_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let base_delay_ms = std::env::var("KOU_RETRY_BASE_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        let max_delay_ms = std::env::var("KOU_RETRY_MAX_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16_000);
        Self {
            max_retries,
            base_delay_ms,
            max_delay_ms,
            respect_retry_after: true,
        }
    }
}

/// Outcome of an execute-with-retry call.
pub struct RetryOutcome {
    pub result: UpstreamResult,
    /// Total number of attempts made (1 = no retries).
    pub attempts: u32,
    /// Total time spent sleeping between retries.
    pub total_retry_delay_ms: u64,
    /// Retry-After header value from the last response, if any.
    pub retry_after_secs: Option<u64>,
}

/// Execute an upstream request with retry logic.
///
/// Retries are only attempted for *buffered* (non-streaming) responses with retriable errors.
/// Streaming 2xx responses are returned immediately since the byte stream cannot be replayed.
#[allow(clippy::too_many_arguments)]
pub async fn execute_with_retry(
    client: &UpstreamClient,
    provider: &ProviderConnection,
    endpoint: EndpointKind,
    model: &str,
    prepared: &PreparedUpstreamRequest,
    passthrough_headers: Option<&PassthroughHeaders>,
    config: &RetryConfig,
) -> AppResult<RetryOutcome> {
    let mut attempts: u32 = 0;
    let mut total_delay: u64 = 0;
    let mut _last_retry_after: Option<u64> = None;

    loop {
        attempts += 1;

        let result = client
            .execute_prepared(provider, endpoint, model, prepared, passthrough_headers)
            .await;

        match result {
            Ok(UpstreamResult::Streaming(streaming)) => {
                // Streaming 2xx — cannot retry, return immediately
                return Ok(RetryOutcome {
                    result: UpstreamResult::Streaming(streaming),
                    attempts,
                    total_retry_delay_ms: total_delay,
                    retry_after_secs: None,
                });
            }
            Ok(UpstreamResult::Buffered(ref response)) => {
                let status = response.status;

                // Success — return immediately
                if status.is_success() {
                    return Ok(RetryOutcome {
                        result: result?,
                        attempts,
                        total_retry_delay_ms: total_delay,
                        retry_after_secs: None,
                    });
                }

                // Check retriability
                let axum_status = axum::http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
                let kind = classify_upstream_error(axum_status, &response.body);

                // Parse Retry-After if present
                let retry_after = if config.respect_retry_after {
                    parse_retry_after_from_headers(&response.response_headers)
                } else {
                    None
                };
                _last_retry_after = retry_after;

                // Max retries exceeded or non-retriable — return as-is
                if attempts > config.max_retries || !kind.is_retriable() {
                    return Ok(RetryOutcome {
                        result: result?,
                        attempts,
                        total_retry_delay_ms: total_delay,
                        retry_after_secs: retry_after,
                    });
                }

                // Calculate backoff delay
                let delay_ms = calculate_delay(config, attempts, retry_after, kind);
                total_delay += delay_ms;

                tracing::warn!(
                    provider_id = %provider.id,
                    model = %model,
                    attempt = attempts,
                    status = %status,
                    error_kind = ?kind,
                    delay_ms = delay_ms,
                    "retrying upstream request"
                );

                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            Err(e) => {
                // Connection-level errors (DNS, TCP, TLS) are retriable
                if attempts > config.max_retries {
                    return Err(e);
                }

                let delay_ms =
                    calculate_delay(config, attempts, None, UpstreamErrorKind::ConnectionError);
                total_delay += delay_ms;

                tracing::warn!(
                    provider_id = %provider.id,
                    model = %model,
                    attempt = attempts,
                    error = %e,
                    delay_ms = delay_ms,
                    "retrying after connection error"
                );

                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

/// Calculate backoff delay with jitter.
///
/// Formula: `min(base_delay * 2^(attempt-1) + jitter, max_delay)`
/// For 429 with Retry-After, use that value instead.
/// For 408 (timeout), use shorter backoff.
fn calculate_delay(
    config: &RetryConfig,
    attempt: u32,
    retry_after: Option<u64>,
    kind: UpstreamErrorKind,
) -> u64 {
    // If Retry-After is provided and kind is RateLimit, prefer it
    if let Some(retry_secs) = retry_after {
        if kind == UpstreamErrorKind::RateLimit {
            // Respect Retry-After but cap at max_delay
            return (retry_secs * 1000).min(config.max_delay_ms);
        }
    }

    // For timeout errors, use shorter backoff
    let base = if kind == UpstreamErrorKind::Timeout {
        config.base_delay_ms / 2
    } else {
        config.base_delay_ms
    };

    // Exponential backoff: base * 2^(attempt-1)
    let exp_delay = base.saturating_mul(1u64.checked_shl(attempt - 1).unwrap_or(u64::MAX));

    // Add jitter: random value in [0, base/2)
    let jitter = {
        let mut rng = rand::rng();
        rng.random_range(0..base.max(2) / 2)
    };

    (exp_delay + jitter).min(config.max_delay_ms)
}

/// Parse `Retry-After` from response headers.
/// Supports integer (seconds) format.
fn parse_retry_after_from_headers(headers: &[(String, String)]) -> Option<u64> {
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("retry-after") {
            // Try integer seconds first
            if let Ok(secs) = value.parse::<u64>() {
                return Some(secs);
            }
            // Try float seconds
            if let Ok(secs) = value.parse::<f64>() {
                return Some(secs.ceil() as u64);
            }
            // HTTP-date format is complex; skip for now
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_ms, 500);
        assert_eq!(config.max_delay_ms, 16_000);
        assert!(config.respect_retry_after);
    }

    #[test]
    fn test_calculate_delay_exponential() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 100,
            max_delay_ms: 10_000,
            respect_retry_after: true,
        };
        // Attempt 1: ~100ms (100 * 2^0 + jitter)
        let d1 = calculate_delay(&config, 1, None, UpstreamErrorKind::ServerError);
        assert!(d1 >= 100 && d1 <= 150, "d1={d1}");

        // Attempt 2: ~200ms (100 * 2^1 + jitter)
        let d2 = calculate_delay(&config, 2, None, UpstreamErrorKind::ServerError);
        assert!(d2 >= 200 && d2 <= 250, "d2={d2}");

        // Attempt 3: ~400ms (100 * 2^2 + jitter)
        let d3 = calculate_delay(&config, 3, None, UpstreamErrorKind::ServerError);
        assert!(d3 >= 400 && d3 <= 450, "d3={d3}");
    }

    #[test]
    fn test_calculate_delay_respects_max() {
        let config = RetryConfig {
            max_retries: 10,
            base_delay_ms: 1000,
            max_delay_ms: 5000,
            respect_retry_after: true,
        };
        let d = calculate_delay(&config, 8, None, UpstreamErrorKind::ServerError);
        assert!(d <= 5000, "delay should be capped at max_delay: {d}");
    }

    #[test]
    fn test_calculate_delay_retry_after() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            respect_retry_after: true,
        };
        let d = calculate_delay(&config, 1, Some(5), UpstreamErrorKind::RateLimit);
        assert_eq!(d, 5000, "should use Retry-After * 1000");
    }

    #[test]
    fn test_calculate_delay_timeout_shorter() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 200,
            max_delay_ms: 10_000,
            respect_retry_after: true,
        };
        let d = calculate_delay(&config, 1, None, UpstreamErrorKind::Timeout);
        // Timeout uses base/2 = 100, so delay ~100-149
        assert!(d >= 100 && d <= 150, "timeout delay should be shorter: {d}");
    }

    #[test]
    fn test_parse_retry_after_integer() {
        let headers = vec![("retry-after".to_string(), "30".to_string())];
        assert_eq!(parse_retry_after_from_headers(&headers), Some(30));
    }

    #[test]
    fn test_parse_retry_after_float() {
        let headers = vec![("retry-after".to_string(), "2.5".to_string())];
        assert_eq!(parse_retry_after_from_headers(&headers), Some(3));
    }

    #[test]
    fn test_parse_retry_after_missing() {
        let headers = vec![("x-other".to_string(), "42".to_string())];
        assert_eq!(parse_retry_after_from_headers(&headers), None);
    }

    #[test]
    fn test_parse_retry_after_case_insensitive() {
        let headers = vec![("Retry-After".to_string(), "10".to_string())];
        assert_eq!(parse_retry_after_from_headers(&headers), Some(10));
    }
}
