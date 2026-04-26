use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    models::NewProviderConnection,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPreset {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub auth_type: String,
    pub auth_header: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_prefix: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub endpoint_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub stream_endpoint_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub supported_endpoints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    pub source: String,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportProviderPresetRequest {
    pub preset_id: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub model_prefix: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub rate_limit_protection: Option<bool>,
}

pub fn provider_presets() -> Vec<ProviderPreset> {
    vec![
        preset(
            "openai",
            "OpenAI",
            "https://api.openai.com/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &[
                "chat",
                "responses",
                "embeddings",
                "images",
                "moderations",
                "audio.speech",
                "audio.transcriptions",
            ],
            Some("openai/gpt-4o-mini"),
            "Нативный OpenAI-style preset. Ближе всего к текущему generic pipeline.",
        ),
        preset(
            "anthropic",
            "Anthropic API Key",
            "https://api.anthropic.com/v1",
            "apikey",
            "x-api-key",
            None,
            &[("Anthropic-Version", "2023-06-01")],
            &[("messages", "/messages")],
            &[],
            &["chat", "messages", "responses", "files"],
            Some("anthropic/claude-sonnet-4.6"),
            "Anthropic preset for Claude-compatible server routing: OpenAI chat/responses requests translate to Messages API, plus raw passthrough support for count_tokens and Files API content downloads.",
        ),
        preset(
            "openrouter",
            "OpenRouter",
            "https://openrouter.ai/api/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("openrouter/openai/gpt-4o-mini"),
            "OpenAI-compatible chat provider из OmniRoute.",
        ),
        preset(
            "deepseek",
            "DeepSeek",
            "https://api.deepseek.com/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("deepseek/deepseek-chat"),
            "OpenAI-compatible chat provider из OmniRoute.",
        ),
        preset(
            "groq",
            "Groq",
            "https://api.groq.com/openai/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("groq/llama-3.3-70b-versatile"),
            "OpenAI-compatible chat provider из OmniRoute.",
        ),
        preset(
            "xai",
            "xAI",
            "https://api.x.ai/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat", "images"],
            Some("xai/grok-3-mini"),
            "xAI chat/image surface как в OmniRoute registries.",
        ),
        preset(
            "mistral",
            "Mistral",
            "https://api.mistral.ai/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat", "embeddings"],
            Some("mistral/mistral-small-latest"),
            "Mistral chat/embeddings preset по мотивам OmniRoute.",
        ),
        preset(
            "together",
            "Together",
            "https://api.together.xyz/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat", "images", "embeddings", "rerank"],
            Some("together/meta-llama/Llama-3.3-70B-Instruct-Turbo"),
            "Together surface объединён из chat/image/embedding/rerank registries OmniRoute.",
        ),
        preset(
            "fireworks",
            "Fireworks",
            "https://api.fireworks.ai/inference/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[("images.generations", "/images/generations")],
            &[],
            &["chat", "images", "embeddings", "rerank"],
            Some("fireworks/accounts/fireworks/models/llama-v3p1-70b-instruct"),
            "Fireworks preset из нескольких OmniRoute registries.",
        ),
        preset(
            "cohere",
            "Cohere",
            "https://api.cohere.com/v2",
            "apikey",
            "bearer",
            None,
            &[],
            &[("rerank", "/rerank")],
            &[],
            &["rerank"],
            Some("cohere/rerank-v3.5"),
            "Rerank-focused preset как в OmniRoute.",
        ),
        preset(
            "nvidia",
            "NVIDIA",
            "https://integrate.api.nvidia.com/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &[
                "chat",
                "rerank",
                "embeddings",
                "audio.speech",
                "audio.transcriptions",
            ],
            Some("nvidia/meta/llama-3.1-70b-instruct"),
            "Сводный NVIDIA preset по OmniRoute registries.",
        ),
        preset(
            "nebius",
            "Nebius",
            "https://api.studio.nebius.com/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[(
                "images.generations",
                "https://api.tokenfactory.nebius.com/v1/images/generations",
            )],
            &[],
            &["chat", "embeddings", "images", "rerank"],
            Some("nebius/meta-llama/Meta-Llama-3.1-70B-Instruct"),
            "Nebius chat + image/embedding/rerank footprint из OmniRoute.",
        ),
        preset(
            "hyperbolic",
            "Hyperbolic",
            "https://api.hyperbolic.xyz/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[
                (
                    "images.generations",
                    "https://api.hyperbolic.xyz/v1/image/generation",
                ),
                (
                    "audio.speech",
                    "https://api.hyperbolic.xyz/v1/audio/generation",
                ),
            ],
            &[],
            &["chat", "images", "audio.speech"],
            Some("hyperbolic/meta-llama/Llama-3.1-70B-Instruct"),
            "Часть Hyperbolic endpoint'ов в OmniRoute требует custom payload transformers; preset хранит URLs как scaffold.",
        ),
        preset(
            "huggingface",
            "Hugging Face Inference",
            "https://api-inference.huggingface.co",
            "apikey",
            "bearer",
            None,
            &[],
            &[
                (
                    "audio.transcriptions",
                    "https://api-inference.huggingface.co/models",
                ),
                (
                    "audio.speech",
                    "https://api-inference.huggingface.co/models",
                ),
            ],
            &[],
            &["chat", "audio.speech", "audio.transcriptions"],
            Some("huggingface/meta-llama/Llama-3.1-8B-Instruct"),
            "HF preset по OmniRoute. ASR/TTS paths пока scaffold-уровня, под transformers batch позже.",
        ),
        preset(
            "vertex",
            "Google Vertex AI",
            "https://aiplatform.googleapis.com/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("vertex/gemini-2.5-pro"),
            "Vertex chat preset из OmniRoute registry.",
        ),
        preset(
            "alibaba",
            "Alibaba",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("alibaba/qwen-plus"),
            "Alibaba compatible-mode preset из OmniRoute.",
        ),
        preset(
            "cloudflare-ai",
            "Cloudflare AI Gateway",
            "https://api.cloudflare.com/client/v4/accounts",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("cloudflare-ai/@cf/meta/llama-3.1-8b-instruct"),
            "Cloudflare AI preset из OmniRoute registry. Часто требует account-specific URL finishing.",
        ),
        preset(
            "aimlapi",
            "AIMLAPI",
            "https://api.aimlapi.com/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("aimlapi/openai/gpt-4o-mini"),
            "OpenAI-compatible chat preset из OmniRoute.",
        ),
        preset(
            "pollinations",
            "Pollinations",
            "https://text.pollinations.ai/openai",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("pollinations/openai-large"),
            "Pollinations preset из OmniRoute.",
        ),
        preset(
            "glm",
            "GLM",
            "https://open.bigmodel.cn/api/paas/v4",
            "apikey",
            "x-api-key",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("glm/glm-4.5"),
            "GLM preset из OmniRoute provider registry.",
        ),
        preset(
            "kimi",
            "Kimi",
            "https://api.moonshot.ai/v1",
            "apikey",
            "bearer",
            None,
            &[],
            &[],
            &[],
            &["chat"],
            Some("kimi/kimi-k2.5"),
            "Kimi API-key preset из OmniRoute.",
        ),
        preset(
            "serper-search",
            "Serper Search",
            "https://google.serper.dev",
            "apikey",
            "x-api-key",
            None,
            &[],
            &[("search", "https://google.serper.dev/search")],
            &[],
            &["search"],
            Some("serper-search/web"),
            "Search provider preset по OmniRoute search registry.",
        ),
        preset(
            "brave-search",
            "Brave Search",
            "https://api.search.brave.com",
            "apikey",
            "x-subscription-token",
            None,
            &[],
            &[("search", "https://api.search.brave.com/res/v1/web/search")],
            &[],
            &["search"],
            Some("brave-search/web"),
            "Search provider preset по OmniRoute search registry.",
        ),
        preset(
            "exa-search",
            "Exa Search",
            "https://api.exa.ai",
            "apikey",
            "x-api-key",
            None,
            &[],
            &[("search", "https://api.exa.ai/search")],
            &[],
            &["search"],
            Some("exa-search/web"),
            "Search provider preset по OmniRoute search registry.",
        ),
        preset(
            "tavily-search",
            "Tavily Search",
            "https://api.tavily.com",
            "apikey",
            "bearer",
            None,
            &[],
            &[("search", "https://api.tavily.com/search")],
            &[],
            &["search"],
            Some("tavily-search/web"),
            "Search provider preset по OmniRoute search registry.",
        ),
        preset(
            "perplexity-search",
            "Perplexity Search",
            "https://api.perplexity.ai",
            "apikey",
            "bearer",
            None,
            &[],
            &[("search", "https://api.perplexity.ai/search")],
            &[],
            &["search"],
            Some("perplexity-search/web"),
            "Search provider preset по OmniRoute search registry.",
        ),
        preset(
            "claude-oauth",
            "Claude OAuth",
            "https://api.anthropic.com/v1",
            "oauth",
            "bearer",
            Some("Bearer"),
            &[
                ("Anthropic-Version", "2023-06-01"),
                (
                    "Anthropic-Beta",
                    "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14",
                ),
            ],
            &[("messages", "/messages")],
            &[],
            &["chat", "messages", "responses", "files"],
            Some("claude-oauth/claude-sonnet-4.6"),
            "Claude Code OAuth preset with native authorization-code exchange, OpenAI chat/responses compatibility via Messages translation, and Claude-compatible Files API content proxying for managed provider accounts.",
        ),
        preset(
            "antigravity",
            "Antigravity OAuth Scaffold",
            "https://cloudcode-pa.googleapis.com",
            "oauth",
            "bearer",
            Some("Bearer"),
            &[],
            &[(
                "chat",
                "https://cloudcode-pa.googleapis.com/v1internal:generateContent",
            )],
            &[(
                "chat",
                "https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse",
            )],
            &["chat"],
            Some("antigravity/gemini-3.1-pro-high"),
            "OmniRoute Antigravity scaffold. Требует OAuth/token-refresh и Gemini-style body translator для полной parity.",
        ),
        preset(
            "codex",
            "Codex OAuth",
            "https://chatgpt.com/backend-api/codex",
            "oauth",
            "bearer",
            Some("Bearer"),
            &[
                ("Openai-Beta", "responses=experimental"),
                ("Version", "0.124.0"),
                (
                    "User-Agent",
                    "codex_cli_rs/0.124.0 (Mac OS; arm64) ghostty/1.3.1",
                ),
            ],
            &[(
                "responses",
                "https://chatgpt.com/backend-api/codex/responses",
            )],
            &[],
            &["responses"],
            Some("codex/gpt-5.3-codex"),
            "Codex OAuth preset for managed ChatGPT-backed accounts using the native Codex responses endpoint and OAuth refresh flow.",
        ),
        preset(
            "github-copilot",
            "GitHub Copilot OAuth Scaffold",
            "https://api.githubcopilot.com",
            "oauth",
            "bearer",
            Some("Bearer"),
            &[
                ("copilot-integration-id", "vscode-chat"),
                ("editor-version", "vscode/1.110.0"),
                ("editor-plugin-version", "copilot-chat/0.38.0"),
                ("x-github-api-version", "2025-04-01"),
            ],
            &[
                ("chat", "https://api.githubcopilot.com/chat/completions"),
                ("responses", "https://api.githubcopilot.com/responses"),
            ],
            &[],
            &["chat", "responses"],
            Some("github-copilot/gpt-4.1"),
            "GitHub Copilot scaffold из OmniRoute. OAuth и provider-specific response nuances ещё впереди.",
        ),
    ]
}

pub fn find_provider_preset(id: &str) -> Option<ProviderPreset> {
    provider_presets()
        .into_iter()
        .find(|preset| preset.id.eq_ignore_ascii_case(id))
}

pub fn import_request_to_provider(
    input: ImportProviderPresetRequest,
) -> AppResult<NewProviderConnection> {
    let preset = find_provider_preset(&input.preset_id)
        .ok_or_else(|| AppError::NotFound(format!("provider preset {}", input.preset_id)))?;

    Ok(NewProviderConnection {
        provider: preset.id.clone(),
        base_url: preset.base_url,
        api_key: input.api_key,
        auth_type: preset.auth_type,
        auth_header: preset.auth_header,
        auth_prefix: preset.auth_prefix,
        extra_headers: preset.extra_headers,
        endpoint_paths: Some(preset.endpoint_paths),
        stream_endpoint_paths: Some(preset.stream_endpoint_paths),
        model_prefix: Some(input.model_prefix.unwrap_or_else(|| preset.id.clone())),
        name: input.name.or(Some(preset.display_name)),
        enabled: input.enabled.unwrap_or(true),
        priority: input.priority,
        default_model: preset.default_model,
        supported_endpoints: Some(preset.supported_endpoints),
        rate_limit_protection: input.rate_limit_protection.unwrap_or(false),
        protocol_format: detect_preset_protocol(&preset.id),
    })
}

fn preset(
    id: &str,
    display_name: &str,
    base_url: &str,
    auth_type: &str,
    auth_header: &str,
    auth_prefix: Option<&str>,
    extra_headers: &[(&str, &str)],
    endpoint_paths: &[(&str, &str)],
    stream_endpoint_paths: &[(&str, &str)],
    supported_endpoints: &[&str],
    default_model: Option<&str>,
    notes: &str,
) -> ProviderPreset {
    ProviderPreset {
        id: id.to_string(),
        display_name: display_name.to_string(),
        base_url: base_url.to_string(),
        auth_type: auth_type.to_string(),
        auth_header: auth_header.to_string(),
        auth_prefix: auth_prefix.map(ToString::to_string),
        extra_headers: map_from_pairs(extra_headers),
        endpoint_paths: map_from_pairs(endpoint_paths),
        stream_endpoint_paths: map_from_pairs(stream_endpoint_paths),
        supported_endpoints: supported_endpoints
            .iter()
            .map(|value| value.to_string())
            .collect(),
        default_model: default_model.map(ToString::to_string),
        source: "diegosouzapw/OmniRoute".to_string(),
        notes: notes.to_string(),
    }
}

fn map_from_pairs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn detect_preset_protocol(preset_id: &str) -> Option<String> {
    match preset_id {
        "anthropic" | "claude-oauth" => Some("claude".to_string()),
        "codex" => Some("openai-responses".to_string()),
        "vertex" => Some("gemini".to_string()),
        "ollama" => Some("ollama".to_string()),
        _ => None, // default: OpenAI-compatible
    }
}

#[cfg(test)]
mod tests {
    use super::detect_preset_protocol;

    #[test]
    fn test_detect_preset_protocol_codex_responses() {
        assert_eq!(
            detect_preset_protocol("codex"),
            Some("openai-responses".to_string())
        );
    }
}
