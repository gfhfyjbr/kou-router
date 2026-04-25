use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Parsed rate limit information from upstream response headers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RateLimitInfo {
    /// Remaining requests/tokens allowed.
    pub remaining: Option<u64>,
    /// Total limit.
    pub limit: Option<u64>,
    /// When the rate limit resets.
    pub reset_at: Option<DateTime<Utc>>,
    /// Utilization percentage (0.0-100.0).
    pub utilization_pct: Option<f64>,
    /// Retry-After value in seconds, if present.
    pub retry_after_secs: Option<u64>,
}

/// Per-provider in-memory rate limit state.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderRateLimitState {
    pub provider_id: String,
    pub last_remaining: Option<u64>,
    pub last_limit: Option<u64>,
    pub last_utilization_pct: Option<f64>,
    pub last_reset_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// Thread-safe in-memory store for rate limit state per provider.
#[derive(Debug, Clone, Default)]
pub struct RateLimitTracker {
    state: Arc<RwLock<HashMap<String, ProviderRateLimitState>>>,
    /// Utilization percentage threshold for warning (default: 85%).
    warn_threshold: f64,
}

impl RateLimitTracker {
    pub fn new() -> Self {
        let warn_threshold: f64 = std::env::var("KOU_RATELIMIT_WARN_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(85.0);
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
            warn_threshold,
        }
    }

    /// Update rate limit state for a provider based on parsed headers.
    pub fn update(&self, provider_id: &str, info: &RateLimitInfo) {
        if info.remaining.is_none() && info.limit.is_none() {
            return; // No rate limit info to track
        }
        let mut state = self.state.write().unwrap();
        let entry =
            state
                .entry(provider_id.to_string())
                .or_insert_with(|| ProviderRateLimitState {
                    provider_id: provider_id.to_string(),
                    last_remaining: None,
                    last_limit: None,
                    last_utilization_pct: None,
                    last_reset_at: None,
                    updated_at: Utc::now(),
                });
        if let Some(remaining) = info.remaining {
            entry.last_remaining = Some(remaining);
        }
        if let Some(limit) = info.limit {
            entry.last_limit = Some(limit);
        }
        entry.last_utilization_pct = info.utilization_pct;
        entry.last_reset_at = info.reset_at;
        entry.updated_at = Utc::now();
    }

    /// Check if a provider is near its rate limit (above threshold).
    pub fn is_near_limit(&self, provider_id: &str) -> bool {
        let state = self.state.read().unwrap();
        state
            .get(provider_id)
            .and_then(|s| s.last_utilization_pct)
            .map(|pct| pct >= self.warn_threshold)
            .unwrap_or(false)
    }

    /// Get current state for all providers (for monitoring endpoint).
    pub fn get_all_states(&self) -> Vec<ProviderRateLimitState> {
        let state = self.state.read().unwrap();
        state.values().cloned().collect()
    }
}

/// Parse rate limit information from upstream response headers.
///
/// Supports multiple header formats:
///   - `anthropic-ratelimit-requests-remaining` / `anthropic-ratelimit-requests-limit`
///   - `anthropic-ratelimit-tokens-remaining` / `anthropic-ratelimit-tokens-limit`
///   - `x-ratelimit-remaining` / `x-ratelimit-limit` (OpenAI)
///   - `x-ratelimit-remaining-requests` / `x-ratelimit-limit-requests`
///   - `retry-after`
pub fn parse_rate_limit_headers(headers: &[(String, String)]) -> RateLimitInfo {
    let mut info = RateLimitInfo::default();

    // Helper: find header value case-insensitively
    let find = |target: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(target))
            .map(|(_, value)| value.as_str())
    };

    // --- Remaining ---
    // Try Anthropic format first (more specific)
    let remaining = find("anthropic-ratelimit-requests-remaining")
        .or_else(|| find("anthropic-ratelimit-tokens-remaining"))
        .or_else(|| find("x-ratelimit-remaining-requests"))
        .or_else(|| find("x-ratelimit-remaining-tokens"))
        .or_else(|| find("x-ratelimit-remaining"));

    if let Some(val) = remaining {
        info.remaining = val.parse().ok();
    }

    // --- Limit ---
    let limit = find("anthropic-ratelimit-requests-limit")
        .or_else(|| find("anthropic-ratelimit-tokens-limit"))
        .or_else(|| find("x-ratelimit-limit-requests"))
        .or_else(|| find("x-ratelimit-limit-tokens"))
        .or_else(|| find("x-ratelimit-limit"));

    if let Some(val) = limit {
        info.limit = val.parse().ok();
    }

    // --- Calculate utilization ---
    if let (Some(remaining), Some(limit)) = (info.remaining, info.limit) {
        if limit > 0 {
            let used = limit.saturating_sub(remaining);
            info.utilization_pct = Some((used as f64 / limit as f64) * 100.0);
        }
    }

    // --- Reset ---
    let reset = find("anthropic-ratelimit-requests-reset")
        .or_else(|| find("anthropic-ratelimit-tokens-reset"))
        .or_else(|| find("x-ratelimit-reset-requests"))
        .or_else(|| find("x-ratelimit-reset-tokens"))
        .or_else(|| find("x-ratelimit-reset"));

    if let Some(val) = reset {
        // Try epoch seconds first
        if let Ok(epoch) = val.parse::<i64>() {
            info.reset_at = DateTime::from_timestamp(epoch, 0);
        }
        // Try RFC3339 / ISO 8601
        else if let Ok(dt) = val.parse::<DateTime<Utc>>() {
            info.reset_at = Some(dt);
        }
    }

    // --- Retry-After ---
    if let Some(val) = find("retry-after") {
        if let Ok(secs) = val.parse::<u64>() {
            info.retry_after_secs = Some(secs);
        } else if let Ok(secs) = val.parse::<f64>() {
            info.retry_after_secs = Some(secs.ceil() as u64);
        }
    }

    info
}

/// Anthropic unified rate limit information (new format).
/// These headers provide per-window (5h/7d) utilization tracking
/// and overage management.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UnifiedRateLimitInfo {
    /// Overall quota status: "ok" | "warning" | "exceeded"
    pub status: Option<String>,
    /// When the rate limit resets (ISO 8601)
    pub reset_at: Option<DateTime<Utc>>,
    /// Whether a fallback model is available
    pub fallback_available: bool,
    /// Representative claim (the most constrained resource)
    pub representative_claim: Option<String>,
    /// 5-hour window utilization percentage
    pub utilization_5h: Option<f64>,
    /// 7-day window utilization percentage
    pub utilization_7d: Option<f64>,
    /// 5-hour window reset time
    pub reset_5h: Option<DateTime<Utc>>,
    /// 7-day window reset time
    pub reset_7d: Option<DateTime<Utc>>,
    /// Whether 5h threshold was surpassed
    pub surpassed_5h: bool,
    /// Whether 7d threshold was surpassed
    pub surpassed_7d: bool,
    /// Overage status
    pub overage_status: Option<String>,
    /// Overage reset time
    pub overage_reset: Option<DateTime<Utc>>,
    /// Reason overage is disabled
    pub overage_disabled_reason: Option<String>,
    /// Whether currently using overage allowance
    pub is_using_overage: bool,
}

/// Parse Anthropic unified rate limit headers (new format).
/// These complement the legacy headers and provide richer quota info.
pub fn parse_unified_rate_limit_headers(
    headers: &[(String, String)],
) -> Option<UnifiedRateLimitInfo> {
    let find = |target: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(target))
            .map(|(_, value)| value.as_str())
    };

    // Only parse if the unified status header is present
    let status = find("anthropic-ratelimit-unified-status")?;

    let mut info = UnifiedRateLimitInfo {
        status: Some(status.to_string()),
        ..Default::default()
    };

    // Reset time
    if let Some(val) = find("anthropic-ratelimit-unified-reset") {
        info.reset_at = val.parse::<DateTime<Utc>>().ok();
    }

    // Fallback availability
    info.fallback_available = find("anthropic-ratelimit-unified-fallback")
        .map(|v| v == "available")
        .unwrap_or(false);

    // Representative claim
    info.representative_claim =
        find("anthropic-ratelimit-unified-representative-claim").map(|v| v.to_string());

    // 5h window
    if let Some(val) = find("anthropic-ratelimit-unified-5h-utilization") {
        info.utilization_5h = val.parse::<f64>().ok();
    }
    if let Some(val) = find("anthropic-ratelimit-unified-5h-reset") {
        info.reset_5h = val.parse::<DateTime<Utc>>().ok();
    }
    info.surpassed_5h = find("anthropic-ratelimit-unified-5h-surpassed-threshold")
        .map(|v| v == "true")
        .unwrap_or(false);

    // 7d window
    if let Some(val) = find("anthropic-ratelimit-unified-7d-utilization") {
        info.utilization_7d = val.parse::<f64>().ok();
    }
    if let Some(val) = find("anthropic-ratelimit-unified-7d-reset") {
        info.reset_7d = val.parse::<DateTime<Utc>>().ok();
    }
    info.surpassed_7d = find("anthropic-ratelimit-unified-7d-surpassed-threshold")
        .map(|v| v == "true")
        .unwrap_or(false);

    // Overage info
    info.overage_status = find("anthropic-ratelimit-unified-overage-status").map(|v| v.to_string());
    if let Some(val) = find("anthropic-ratelimit-unified-overage-reset") {
        info.overage_reset = val.parse::<DateTime<Utc>>().ok();
    }
    info.overage_disabled_reason =
        find("anthropic-ratelimit-unified-overage-disabled-reason").map(|v| v.to_string());
    info.is_using_overage = find("anthropic-ratelimit-unified-is-using-overage")
        .map(|v| v == "true")
        .unwrap_or(false);

    Some(info)
}

/// Also enrich legacy RateLimitInfo with unified headers when available.
/// If unified status is "exceeded", set utilization to 100%.
pub fn enrich_with_unified(info: &mut RateLimitInfo, headers: &[(String, String)]) {
    if let Some(unified) = parse_unified_rate_limit_headers(headers) {
        // Use 5h utilization as primary utilization if legacy is missing
        if info.utilization_pct.is_none() {
            info.utilization_pct = unified.utilization_5h;
        }
        // If unified says exceeded, force 100% utilization
        if unified.status.as_deref() == Some("exceeded") {
            info.utilization_pct = Some(100.0);
        }
        // Use unified reset if legacy is missing
        if info.reset_at.is_none() {
            info.reset_at = unified.reset_at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_anthropic_headers() {
        let headers = vec![
            (
                "anthropic-ratelimit-requests-remaining".to_string(),
                "95".to_string(),
            ),
            (
                "anthropic-ratelimit-requests-limit".to_string(),
                "100".to_string(),
            ),
        ];
        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.remaining, Some(95));
        assert_eq!(info.limit, Some(100));
        assert!((info.utilization_pct.unwrap() - 5.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_openai_headers() {
        let headers = vec![
            ("x-ratelimit-remaining".to_string(), "50".to_string()),
            ("x-ratelimit-limit".to_string(), "200".to_string()),
        ];
        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.remaining, Some(50));
        assert_eq!(info.limit, Some(200));
        assert!((info.utilization_pct.unwrap() - 75.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_retry_after() {
        let headers = vec![("retry-after".to_string(), "30".to_string())];
        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.retry_after_secs, Some(30));
    }

    #[test]
    fn test_parse_empty_headers() {
        let headers: Vec<(String, String)> = vec![];
        let info = parse_rate_limit_headers(&headers);
        assert!(info.remaining.is_none());
        assert!(info.limit.is_none());
        assert!(info.utilization_pct.is_none());
        assert!(info.retry_after_secs.is_none());
    }

    #[test]
    fn test_tracker_update_and_check() {
        let tracker = RateLimitTracker {
            state: Arc::new(RwLock::new(HashMap::new())),
            warn_threshold: 85.0,
        };

        // Provider at 90% utilization — should be near limit
        tracker.update(
            "provider-1",
            &RateLimitInfo {
                remaining: Some(10),
                limit: Some(100),
                utilization_pct: Some(90.0),
                ..Default::default()
            },
        );
        assert!(tracker.is_near_limit("provider-1"));

        // Provider at 50% — should NOT be near limit
        tracker.update(
            "provider-2",
            &RateLimitInfo {
                remaining: Some(50),
                limit: Some(100),
                utilization_pct: Some(50.0),
                ..Default::default()
            },
        );
        assert!(!tracker.is_near_limit("provider-2"));

        // Unknown provider
        assert!(!tracker.is_near_limit("unknown"));
    }

    #[test]
    fn test_tracker_get_all_states() {
        let tracker = RateLimitTracker {
            state: Arc::new(RwLock::new(HashMap::new())),
            warn_threshold: 85.0,
        };

        tracker.update(
            "p1",
            &RateLimitInfo {
                remaining: Some(50),
                limit: Some(100),
                utilization_pct: Some(50.0),
                ..Default::default()
            },
        );
        tracker.update(
            "p2",
            &RateLimitInfo {
                remaining: Some(10),
                limit: Some(100),
                utilization_pct: Some(90.0),
                ..Default::default()
            },
        );

        let states = tracker.get_all_states();
        assert_eq!(states.len(), 2);
    }

    #[test]
    fn test_utilization_calculation() {
        let headers = vec![
            ("x-ratelimit-remaining".to_string(), "0".to_string()),
            ("x-ratelimit-limit".to_string(), "100".to_string()),
        ];
        let info = parse_rate_limit_headers(&headers);
        assert!((info.utilization_pct.unwrap() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_openai_specific_headers() {
        let headers = vec![
            (
                "x-ratelimit-remaining-requests".to_string(),
                "20".to_string(),
            ),
            ("x-ratelimit-limit-requests".to_string(), "60".to_string()),
        ];
        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.remaining, Some(20));
        assert_eq!(info.limit, Some(60));
    }

    #[test]
    fn test_parse_unified_headers() {
        let headers = vec![
            (
                "anthropic-ratelimit-unified-status".to_string(),
                "warning".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-5h-utilization".to_string(),
                "85.5".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-7d-utilization".to_string(),
                "60.0".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-fallback".to_string(),
                "available".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-representative-claim".to_string(),
                "tokens_5h".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-5h-surpassed-threshold".to_string(),
                "true".to_string(),
            ),
        ];
        let info = parse_unified_rate_limit_headers(&headers).unwrap();
        assert_eq!(info.status.as_deref(), Some("warning"));
        assert!((info.utilization_5h.unwrap() - 85.5).abs() < 0.1);
        assert!((info.utilization_7d.unwrap() - 60.0).abs() < 0.1);
        assert!(info.fallback_available);
        assert_eq!(info.representative_claim.as_deref(), Some("tokens_5h"));
        assert!(info.surpassed_5h);
        assert!(!info.surpassed_7d);
    }

    #[test]
    fn test_parse_unified_headers_absent() {
        let headers = vec![("retry-after".to_string(), "30".to_string())];
        assert!(parse_unified_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn test_enrich_with_unified() {
        let headers = vec![
            (
                "anthropic-ratelimit-unified-status".to_string(),
                "exceeded".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-5h-utilization".to_string(),
                "95.0".to_string(),
            ),
        ];
        let mut info = RateLimitInfo::default();
        enrich_with_unified(&mut info, &headers);
        // unified exceeded -> 100% utilization
        assert!((info.utilization_pct.unwrap() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_enrich_preserves_legacy() {
        let headers = vec![
            (
                "anthropic-ratelimit-unified-status".to_string(),
                "ok".to_string(),
            ),
            (
                "anthropic-ratelimit-unified-5h-utilization".to_string(),
                "50.0".to_string(),
            ),
        ];
        let mut info = RateLimitInfo {
            utilization_pct: Some(30.0),
            ..Default::default()
        };
        enrich_with_unified(&mut info, &headers);
        // Legacy utilization should be preserved (not overwritten)
        assert!((info.utilization_pct.unwrap() - 30.0).abs() < 0.1);
    }
}
