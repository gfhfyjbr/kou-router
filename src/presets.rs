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
            "Claude Code",
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
            "Codex",
            "https://chatgpt.com/backend-api/codex",
            "oauth",
            "bearer",
            Some("Bearer"),
            &[
                ("OpenAI-Beta", "responses=experimental"),
                ("originator", "codex_cli_rs"),
            ],
            &[(
                "responses",
                "https://chatgpt.com/backend-api/codex/responses",
            )],
            &[],
            &["responses"],
            Some("codex/gpt-5.5"),
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


pub const BUILTIN_OAUTH_PRESET_IDS: &[&str] = &["codex", "claude-oauth"];


fn host_from_base_url(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = without_scheme.split('/').next().unwrap_or("").trim();
    if host.is_empty() || host == "custom.local" {
        None
    } else {
        Some(host.to_string())
    }
}

fn is_custom_shell_line(provider: &crate::models::ProviderConnection) -> bool {
    if !provider.provider.eq_ignore_ascii_case("custom") {
        return false;
    }
    let name = provider.name.as_deref().unwrap_or("").trim();
    let base = provider.base_url.trim();
    // Canonical shell created by ensure. Old leftover custom *lines* used hostnames
    // and random model prefixes like `custom-cnw2`.
    name.eq_ignore_ascii_case("Custom API")
        || base.contains("custom.local")
        || provider.model_prefix == "custom"
}

fn custom_shell_connection() -> NewProviderConnection {
    NewProviderConnection {
        provider: "custom".to_string(),
        base_url: String::new(),
        api_key: None,
        auth_type: "apikey".to_string(),
        auth_header: "bearer".to_string(),
        auth_prefix: Some("Bearer".to_string()),
        extra_headers: Default::default(),
        endpoint_paths: Some(Default::default()),
        stream_endpoint_paths: Some(Default::default()),
        model_prefix: Some("custom".to_string()),
        name: Some("Custom API".to_string()),
        enabled: true,
        priority: Some(100),
        default_model: None,
        // shell line: account-level endpoints decide capabilities
        supported_endpoints: Some(vec![
            "chat".to_string(),
            "messages".to_string(),
            "responses".to_string(),
        ]),
        rate_limit_protection: false,
        protocol_format: None,
    }
}

/// Ensure the always-on provider lines exist in the DB (Codex, Claude, Custom API).
/// Also collapses leftover custom *provider lines* from the old flow into accounts
/// under the single Custom API shell line.
pub async fn ensure_builtin_provider_connections(
    repository: &crate::repository::SqliteRepository,
) -> AppResult<()> {
    for preset_id in BUILTIN_OAUTH_PRESET_IDS {
        if repository
            .find_provider_connection_by_provider(preset_id)
            .await?
            .is_some()
        {
            continue;
        }
        let create = import_request_to_provider(ImportProviderPresetRequest {
            preset_id: preset_id.to_string(),
            api_key: None,
            model_prefix: None,
            name: None,
            enabled: None,
            priority: None,
            rate_limit_protection: None,
        })?;
        repository.create_provider_connection(create).await?;
    }

    ensure_single_custom_api_line(repository).await?;
    normalize_builtin_line_names(repository).await?;
    Ok(())
}

fn canonical_line_name(provider: &str, name: Option<&str>) -> Option<&'static str> {
    let provider = provider.to_ascii_lowercase();
    let name_l = name.unwrap_or("").to_ascii_lowercase();
    if provider == "codex" || name_l == "codex oauth" || name_l == "codex" {
        return Some("Codex");
    }
    if matches!(
        provider.as_str(),
        "claude-oauth" | "claude" | "anthropic-oauth" | "anthropic"
    ) || name_l == "claude oauth"
        || name_l == "claude code"
    {
        return Some("Claude Code");
    }
    if provider == "custom" || name_l == "custom api" {
        return Some("Custom API");
    }
    None
}

async fn normalize_builtin_line_names(
    repository: &crate::repository::SqliteRepository,
) -> AppResult<()> {
    let providers = repository.list_provider_connections().await?;
    for provider in providers {
        let Some(canonical) = canonical_line_name(&provider.provider, provider.name.as_deref())
        else {
            continue;
        };
        let current = provider.name.as_deref().unwrap_or("").trim();
        if current != canonical {
            repository
                .update_provider_connection_name(&provider.id, canonical)
                .await?;
        }
    }
    Ok(())
}

async fn ensure_single_custom_api_line(
    repository: &crate::repository::SqliteRepository,
) -> AppResult<()> {
    let customs = repository
        .list_provider_connections_by_provider("custom")
        .await?;

    let mut shell = if let Some(shell) = customs.iter().find(|p| is_custom_shell_line(p)).cloned() {
        shell
    } else {
        repository
            .create_provider_connection(custom_shell_connection())
            .await?
    };

    // Never show a fake shell URL on the Custom API card.
    if shell.base_url.contains("custom.local") || shell.base_url.trim().is_empty() {
        if shell.base_url.contains("custom.local") {
            repository
                .update_provider_connection_base_url(&shell.id, "")
                .await?;
            shell.base_url.clear();
        }
    }

    for orphan in customs.into_iter().filter(|p| p.id != shell.id) {
        // Re-home connection-level endpoint config as an account under the shell.
        let label = orphan
            .name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| host_from_base_url(&orphan.base_url))
            .unwrap_or_else(|| "custom endpoint".to_string());

        let supported = if orphan.supported_endpoints.is_empty() {
            None
        } else {
            Some(orphan.supported_endpoints.clone())
        };

        // Only migrate if this orphan looks like a usable endpoint (has base_url).
        if !orphan.base_url.trim().is_empty()
            && !orphan.base_url.contains("custom.local")
        {
            repository
                .create_provider_account(crate::models::NewProviderAccount {
                    provider_connection_id: shell.id.clone(),
                    label: Some(label),
                    auth_mode: crate::models::ProviderAccountAuthMode::ApiKey,
                    api_key: orphan.api_key.clone(),
                    access_token: None,
                    refresh_token: None,
                    expires_at: None,
                    scopes: None,
                    remote_account_id: None,
                    remote_email: None,
                    is_fedramp: false,
                    enabled: orphan.enabled,
                    priority: Some(orphan.priority),
                    proxy_url: None,
                    base_url: Some(orphan.base_url.clone()),
                    protocol_format: orphan.protocol_format.clone(),
                    supported_endpoints: supported,
                })
                .await?;
        }

        // Move any accounts that already lived under the orphan line.
        let orphan_accounts = repository.list_provider_accounts(&orphan.id).await?;
        for account in orphan_accounts {
            repository
                .create_provider_account(crate::models::NewProviderAccount {
                    provider_connection_id: shell.id.clone(),
                    label: account.label,
                    auth_mode: account.auth_mode,
                    api_key: account.api_key,
                    access_token: account.access_token,
                    refresh_token: account.refresh_token,
                    expires_at: account.expires_at,
                    scopes: Some(account.scopes),
                    remote_account_id: account.remote_account_id,
                    remote_email: account.remote_email,
                    is_fedramp: account.is_fedramp,
                    enabled: account.enabled,
                    priority: Some(account.priority),
                    proxy_url: account.proxy_url,
                    base_url: account.base_url.or(Some(orphan.base_url.clone())),
                    protocol_format: account.protocol_format.or(orphan.protocol_format.clone()),
                    supported_endpoints: account
                        .supported_endpoints
                        .or_else(|| {
                            if orphan.supported_endpoints.is_empty() {
                                None
                            } else {
                                Some(orphan.supported_endpoints.clone())
                            }
                        }),
                })
                .await?;
        }

        repository.delete_provider_connection(&orphan.id).await?;
    }

    Ok(())
}

/// Map a Custom API standard id into protocol_format + supported_endpoints.
pub fn custom_api_standard_config(standard: &str) -> AppResult<(Option<String>, Vec<String>)> {
    match standard.trim().to_ascii_lowercase().as_str() {
        "openai-responses" => Ok((Some("openai-responses".to_string()), vec!["responses".to_string()])),
        "openai-completions" => Ok((Some("openai".to_string()), vec!["chat".to_string()])),
        "anthropic-messages" => Ok((Some("claude".to_string()), vec!["messages".to_string()])),
        other => Err(AppError::BadRequest(format!(
            "unsupported custom api standard '{other}' (expected openai-responses, openai-completions, anthropic-messages)"
        ))),
    }
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
    use super::{custom_api_standard_config, detect_preset_protocol};

    #[test]
    fn test_detect_preset_protocol_codex_responses() {
        assert_eq!(
            detect_preset_protocol("codex"),
            Some("openai-responses".to_string())
        );
    }

    #[test]
    fn test_custom_api_standard_config_maps_known_values() {
        let (fmt, endpoints) = custom_api_standard_config("openai-responses").unwrap();
        assert_eq!(fmt.as_deref(), Some("openai-responses"));
        assert_eq!(endpoints, vec!["responses".to_string()]);

        let (fmt, endpoints) = custom_api_standard_config("openai-completions").unwrap();
        assert_eq!(fmt.as_deref(), Some("openai"));
        assert_eq!(endpoints, vec!["chat".to_string()]);

        let (fmt, endpoints) = custom_api_standard_config("anthropic-messages").unwrap();
        assert_eq!(fmt.as_deref(), Some("claude"));
        assert_eq!(endpoints, vec!["messages".to_string()]);
    }

    #[test]
    fn test_custom_api_standard_config_rejects_unknown() {
        assert!(custom_api_standard_config("soap").is_err());
    }
}
