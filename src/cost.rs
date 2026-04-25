use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Pricing per million tokens for a model.
#[derive(Debug, Clone, Serialize)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: Option<f64>,
    pub cache_write_per_million: Option<f64>,
}

/// Extracted usage information from an API response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub total_tokens: u64,
}

/// Cost calculation result.
#[derive(Debug, Clone, Serialize)]
pub struct RequestCost {
    pub usage: UsageInfo,
    pub cost_usd: f64,
    pub model: String,
}

/// Get known model pricing.
///
/// Pricing data from major LLM providers (as of early 2025).
/// Unknown models default to a conservative estimate.
pub fn get_model_pricing(model: &str) -> ModelPricing {
    let lower = model.to_ascii_lowercase();

    // Strip provider prefix (e.g., "anthropic/claude-sonnet-4-20250514" → "claude-sonnet-4-20250514")
    let name = lower.rsplit('/').next().unwrap_or(&lower);

    // Claude models
    if name.starts_with("claude-sonnet-4")
        || name.starts_with("claude-3-5-sonnet")
        || name.starts_with("claude-3.5-sonnet")
    {
        return ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: Some(0.30),
            cache_write_per_million: Some(3.75),
        };
    }
    if name.starts_with("claude-opus-4")
        || name.starts_with("claude-3-opus")
        || name.starts_with("claude-3.5-opus")
    {
        return ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
            cache_read_per_million: Some(1.50),
            cache_write_per_million: Some(18.75),
        };
    }
    if name.starts_with("claude-3-haiku") || name.starts_with("claude-3.5-haiku") {
        return ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
            cache_read_per_million: Some(0.08),
            cache_write_per_million: Some(1.0),
        };
    }

    // OpenAI models
    if name.starts_with("gpt-4o-mini") {
        return ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
            cache_read_per_million: Some(0.075),
            cache_write_per_million: None,
        };
    }
    if name.starts_with("gpt-4o") || name.starts_with("chatgpt-4o") {
        return ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
            cache_read_per_million: Some(1.25),
            cache_write_per_million: None,
        };
    }
    if name.starts_with("gpt-4-turbo") {
        return ModelPricing {
            input_per_million: 10.0,
            output_per_million: 30.0,
            cache_read_per_million: None,
            cache_write_per_million: None,
        };
    }
    if name.starts_with("o1-mini") || name.starts_with("o3-mini") {
        return ModelPricing {
            input_per_million: 1.10,
            output_per_million: 4.40,
            cache_read_per_million: Some(0.55),
            cache_write_per_million: None,
        };
    }
    if name.starts_with("o1") || name.starts_with("o3") || name.starts_with("o4-mini") {
        return ModelPricing {
            input_per_million: 15.0,
            output_per_million: 60.0,
            cache_read_per_million: Some(7.50),
            cache_write_per_million: None,
        };
    }

    // Google Gemini
    if name.starts_with("gemini-2.0-flash") || name.starts_with("gemini-2.5-flash") {
        return ModelPricing {
            input_per_million: 0.075,
            output_per_million: 0.30,
            cache_read_per_million: None,
            cache_write_per_million: None,
        };
    }
    if name.starts_with("gemini-2.0-pro")
        || name.starts_with("gemini-2.5-pro")
        || name.starts_with("gemini-1.5-pro")
    {
        return ModelPricing {
            input_per_million: 1.25,
            output_per_million: 5.0,
            cache_read_per_million: None,
            cache_write_per_million: None,
        };
    }

    // DeepSeek
    if name.starts_with("deepseek-chat") || name.starts_with("deepseek-v") {
        return ModelPricing {
            input_per_million: 0.14,
            output_per_million: 0.28,
            cache_read_per_million: Some(0.014),
            cache_write_per_million: None,
        };
    }
    if name.starts_with("deepseek-reasoner") {
        return ModelPricing {
            input_per_million: 0.55,
            output_per_million: 2.19,
            cache_read_per_million: Some(0.14),
            cache_write_per_million: None,
        };
    }

    // Mistral
    if name.starts_with("mistral-large") || name.starts_with("mistral-medium") {
        return ModelPricing {
            input_per_million: 2.0,
            output_per_million: 6.0,
            cache_read_per_million: None,
            cache_write_per_million: None,
        };
    }
    if name.starts_with("mistral-small") || name.starts_with("codestral") {
        return ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.30,
            cache_read_per_million: None,
            cache_write_per_million: None,
        };
    }

    // Default conservative pricing for unknown models
    ModelPricing {
        input_per_million: 3.0,
        output_per_million: 15.0,
        cache_read_per_million: None,
        cache_write_per_million: None,
    }
}

/// Extract usage information from a response body.
///
/// Supports:
/// - OpenAI format: `{"usage": {"prompt_tokens": N, "completion_tokens": N}}`
/// - Claude format: `{"usage": {"input_tokens": N, "output_tokens": N, "cache_creation_input_tokens": N, "cache_read_input_tokens": N}}`
/// - Gemini format: `{"usageMetadata": {"promptTokenCount": N, "candidatesTokenCount": N}}`
pub fn extract_usage(body: &Value) -> Option<UsageInfo> {
    // OpenAI format
    if let Some(usage) = body.get("usage") {
        let input = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64());
        let cache_creation = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64());
        let total = usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(input + output);

        if input == 0 && output == 0 {
            return None;
        }

        return Some(UsageInfo {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            total_tokens: total,
        });
    }

    // Gemini format
    if let Some(meta) = body.get("usageMetadata") {
        let input = meta
            .get("promptTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = meta
            .get("candidatesTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let total = meta
            .get("totalTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(input + output);

        if input == 0 && output == 0 {
            return None;
        }

        return Some(UsageInfo {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: total,
        });
    }

    None
}

/// Calculate USD cost for a request.
pub fn calculate_cost(model: &str, usage: &UsageInfo) -> f64 {
    let pricing = get_model_pricing(model);

    let input_cost = (usage.input_tokens as f64 / 1_000_000.0) * pricing.input_per_million;
    let output_cost = (usage.output_tokens as f64 / 1_000_000.0) * pricing.output_per_million;

    let cache_read_cost = usage
        .cache_read_tokens
        .zip(pricing.cache_read_per_million)
        .map(|(tokens, price)| (tokens as f64 / 1_000_000.0) * price)
        .unwrap_or(0.0);

    let cache_write_cost = usage
        .cache_creation_tokens
        .zip(pricing.cache_write_per_million)
        .map(|(tokens, price)| (tokens as f64 / 1_000_000.0) * price)
        .unwrap_or(0.0);

    input_cost + output_cost + cache_read_cost + cache_write_cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- extract_usage ---

    #[test]
    fn test_extract_openai_usage() {
        let body = json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        });
        let usage = extract_usage(&body).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
        assert!(usage.cache_read_tokens.is_none());
    }

    #[test]
    fn test_extract_claude_usage() {
        let body = json!({
            "usage": {
                "input_tokens": 200,
                "output_tokens": 80,
                "cache_creation_input_tokens": 10,
                "cache_read_input_tokens": 50
            }
        });
        let usage = extract_usage(&body).unwrap();
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 80);
        assert_eq!(usage.cache_read_tokens, Some(50));
        assert_eq!(usage.cache_creation_tokens, Some(10));
        assert_eq!(usage.total_tokens, 280);
    }

    #[test]
    fn test_extract_gemini_usage() {
        let body = json!({
            "usageMetadata": {
                "promptTokenCount": 150,
                "candidatesTokenCount": 75,
                "totalTokenCount": 225
            }
        });
        let usage = extract_usage(&body).unwrap();
        assert_eq!(usage.input_tokens, 150);
        assert_eq!(usage.output_tokens, 75);
        assert_eq!(usage.total_tokens, 225);
    }

    #[test]
    fn test_extract_no_usage() {
        let body = json!({"choices": [{"message": {"content": "hello"}}]});
        assert!(extract_usage(&body).is_none());
    }

    #[test]
    fn test_extract_zero_tokens_returns_none() {
        let body = json!({"usage": {"prompt_tokens": 0, "completion_tokens": 0}});
        assert!(extract_usage(&body).is_none());
    }

    // --- calculate_cost ---

    #[test]
    fn test_calculate_cost_sonnet() {
        let usage = UsageInfo {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: 2_000_000,
        };
        let cost = calculate_cost("anthropic/claude-sonnet-4-20250514", &usage);
        // $3 input + $15 output = $18
        assert!((cost - 18.0).abs() < 0.01, "cost={cost}");
    }

    #[test]
    fn test_calculate_cost_gpt4o_mini() {
        let usage = UsageInfo {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: 2_000_000,
        };
        let cost = calculate_cost("openai/gpt-4o-mini", &usage);
        // $0.15 input + $0.60 output = $0.75
        assert!((cost - 0.75).abs() < 0.01, "cost={cost}");
    }

    #[test]
    fn test_calculate_cost_with_cache() {
        let usage = UsageInfo {
            input_tokens: 500_000,
            output_tokens: 100_000,
            cache_read_tokens: Some(200_000),
            cache_creation_tokens: Some(50_000),
            total_tokens: 850_000,
        };
        let cost = calculate_cost("anthropic/claude-sonnet-4-20250514", &usage);
        // input: 0.5M * $3 = $1.50
        // output: 0.1M * $15 = $1.50
        // cache_read: 0.2M * $0.30 = $0.06
        // cache_write: 0.05M * $3.75 = $0.1875
        // total: $3.2475
        assert!((cost - 3.2475).abs() < 0.01, "cost={cost}");
    }

    #[test]
    fn test_calculate_cost_unknown_model() {
        let usage = UsageInfo {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: 1500,
        };
        let cost = calculate_cost("some-unknown/model-v2", &usage);
        // default: $3/$15 per M
        // input: 0.001M * $3 = $0.003
        // output: 0.0005M * $15 = $0.0075
        // total: $0.0105
        assert!(cost > 0.0 && cost < 0.02, "cost={cost}");
    }

    #[test]
    fn test_known_pricing_deepseek() {
        let pricing = get_model_pricing("deepseek/deepseek-chat");
        assert!((pricing.input_per_million - 0.14).abs() < 0.01);
        assert!((pricing.output_per_million - 0.28).abs() < 0.01);
    }

    #[test]
    fn test_known_pricing_gemini_flash() {
        let pricing = get_model_pricing("gemini/gemini-2.0-flash-exp");
        assert!((pricing.input_per_million - 0.075).abs() < 0.01);
        assert!((pricing.output_per_million - 0.30).abs() < 0.01);
    }

    #[test]
    fn test_pricing_strips_prefix() {
        // "anthropic/claude-sonnet-4-20250514" should match same as "claude-sonnet-4-20250514"
        let p1 = get_model_pricing("anthropic/claude-sonnet-4-20250514");
        let p2 = get_model_pricing("claude-sonnet-4-20250514");
        assert!((p1.input_per_million - p2.input_per_million).abs() < 0.01);
    }
}
