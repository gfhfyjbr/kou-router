//! Живые списки моделей с системных роутов провайдеров.
//!
//! Вместо хардкода `{prefix}/default` роутер спрашивает сами провайдеры —
//! теми же роутами, что их официальные CLI:
//! - codex: `GET {base}/models?client_version=…` (Codex CLI),
//!   ответ `{"models":[{"slug","visibility",…}]}`;
//! - claude: `GET {base}/models?limit=1000` (Anthropic Models API),
//!   ответ `{"data":[{"id",…}]}`.
//! - custom OpenAI-compatible: `GET {account.base_url}/models`.
//!
//! Ошибки не фатальны: при сбое подмешивается preset-каталог.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::models::ProviderConnection;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Версия Codex CLI для query-параметра `client_version`.
const CODEX_CLIENT_VERSION: &str = "0.55.0";

/// Провайдеры, у которых есть системный роут списка моделей.
pub fn supports(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "codex" | "claude-oauth" | "claude" | "anthropic-oauth"
    )
}

/// Preset slugs used when live fetch fails or no OAuth token is available.
pub fn preset_slugs(provider: &str) -> &'static [&'static str] {
    match provider.to_ascii_lowercase().as_str() {
        "codex" => &[
            "gpt-5.5",
            "gpt-5-codex",
            "gpt-5",
            "o4-mini",
            "codex-mini-latest",
        ],
        "claude-oauth" | "claude" | "anthropic-oauth" | "anthropic" => &[
            "claude-opus-4-1",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
            "claude-sonnet-4-20250514",
        ],
        _ => &[],
    }
}

pub async fn fetch(
    client: &reqwest::Client,
    provider: &ProviderConnection,
    access_token: &str,
) -> AppResult<Vec<String>> {
    match provider.provider.to_ascii_lowercase().as_str() {
        "codex" => fetch_codex(client, provider, access_token).await,
        "claude-oauth" | "claude" | "anthropic-oauth" => {
            fetch_claude(client, provider, access_token).await
        }
        other => Err(AppError::BadRequest(format!(
            "provider '{other}' has no native models route"
        ))),
    }
}

/// Best-effort OpenAI-compatible models list for a custom endpoint base URL.
pub async fn fetch_openai_compatible(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> AppResult<Vec<String>> {
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/models") {
        base.to_string()
    } else {
        format!("{base}/models")
    };

    let mut request = client.get(&url);
    if let Some(key) = api_key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(key);
    }

    let response = request
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|err| AppError::Upstream(format!("custom models fetch failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "custom models fetch failed: status {}",
            response.status()
        )));
    }

    let parsed: OpenAiModels = response
        .json()
        .await
        .map_err(|err| AppError::Upstream(format!("custom models response invalid: {err}")))?;

    Ok(parsed
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|id| !id.is_empty())
        .collect())
}

#[derive(Debug, Deserialize)]
struct CodexModels {
    models: Vec<CodexModel>,
}

#[derive(Debug, Deserialize)]
struct CodexModel {
    slug: String,
    #[serde(default)]
    visibility: Option<String>,
}

async fn fetch_codex(
    client: &reqwest::Client,
    provider: &ProviderConnection,
    access_token: &str,
) -> AppResult<Vec<String>> {
    let base = provider.base_url.trim_end_matches('/');
    let version = std::env::var("KOU_CODEX_CLIENT_VERSION")
        .unwrap_or_else(|_| CODEX_CLIENT_VERSION.to_string());
    let url = format!("{base}/models?client_version={version}");

    let response = client
        .get(&url)
        .bearer_auth(access_token)
        .header("originator", "codex_cli_rs")
        .header("OpenAI-Beta", "responses=experimental")
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|err| AppError::Upstream(format!("codex models fetch failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "codex models fetch failed: status {}",
            response.status()
        )));
    }
    let parsed: CodexModels = response
        .json()
        .await
        .map_err(|err| AppError::Upstream(format!("codex models response invalid: {err}")))?;

    Ok(parsed
        .models
        .into_iter()
        .filter(|model| model.visibility.as_deref() != Some("hide"))
        .map(|model| model.slug)
        .collect())
}

#[derive(Debug, Deserialize)]
struct AnthropicModels {
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModels {
    #[serde(default)]
    data: Vec<OpenAiModelId>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelId {
    id: String,
}

async fn fetch_claude(
    client: &reqwest::Client,
    provider: &ProviderConnection,
    access_token: &str,
) -> AppResult<Vec<String>> {
    let base = provider.base_url.trim_end_matches('/');
    let url = format!("{base}/models?limit=1000");

    // extra_headers пресета несут anthropic-version и oauth beta-флаги
    let mut request = client.get(&url).bearer_auth(access_token);
    for (name, value) in &provider.extra_headers {
        request = request.header(name, value);
    }

    let response = request
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|err| AppError::Upstream(format!("claude models fetch failed: {err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "claude models fetch failed: status {}",
            response.status()
        )));
    }
    let parsed: AnthropicModels = response
        .json()
        .await
        .map_err(|err| AppError::Upstream(format!("claude models response invalid: {err}")))?;

    Ok(parsed.data.into_iter().map(|model| model.id).collect())
}
