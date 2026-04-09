use std::collections::BTreeMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnection {
    pub id: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    #[serde(default)]
    pub auth_prefix: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub endpoint_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub stream_endpoint_paths: BTreeMap<String, String>,
    pub model_prefix: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub priority: i64,
    pub default_model: Option<String>,
    #[serde(default = "default_supported_endpoints")]
    pub supported_endpoints: Vec<String>,
    #[serde(default)]
    pub rate_limit_protection: bool,
    pub last_error: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub last_error_type: Option<String>,
    pub last_error_source: Option<String>,
    pub rate_limited_until: Option<DateTime<Utc>>,
    pub circuit_open_until: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub backoff_level: i64,
    #[serde(default)]
    pub consecutive_use_count: i64,
    pub test_status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub protocol_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProviderConnection {
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    #[serde(default)]
    pub auth_prefix: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub endpoint_paths: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub stream_endpoint_paths: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub model_prefix: Option<String>,
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub priority: Option<i64>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub supported_endpoints: Option<Vec<String>>,
    #[serde(default)]
    pub rate_limit_protection: bool,
    #[serde(default)]
    pub protocol_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Combo {
    pub id: String,
    pub name: String,
    pub strategy: ComboStrategy,
    pub models: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCombo {
    pub name: String,
    #[serde(default)]
    pub strategy: ComboStrategy,
    pub models: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ComboStrategy {
    #[default]
    Priority,
    RoundRobin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    pub alias: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiModelsResponse {
    pub object: String,
    pub data: Vec<OpenAiModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiModel {
    pub id: String,
    pub object: String,
    pub owned_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_sizes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderChatAttempt {
    pub provider_id: String,
    pub model: String,
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDebug {
    pub requested_model: String,
    pub resolved_model: String,
    pub endpoint: String,
    pub path_suffix: Option<String>,
    pub tried: Vec<ProviderChatAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAliasRequest {
    pub alias: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsPayload {
    #[serde(flatten)]
    pub values: Value,
}

#[derive(Debug, Clone)]
pub struct NormalizedRequest {
    pub model: String,
    pub stream: bool,
    pub body: Value,
    pub inject_model: bool,
}

#[derive(Debug, Clone)]
pub struct AudioTranscriptionPayload {
    pub model: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
    pub text_fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    ChatCompletions,
    Completions,
    Messages,
    Responses,
    OllamaChat,
    Embeddings,
    ImagesGenerations,
    Moderations,
    Rerank,
    Search,
    AudioSpeech,
    AudioTranscriptions,
    MusicGenerations,
    VideosGenerations,
}

impl EndpointKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EndpointKind::ChatCompletions => "chat.completions",
            EndpointKind::Completions => "completions",
            EndpointKind::Messages => "messages",
            EndpointKind::Responses => "responses",
            EndpointKind::OllamaChat => "ollama.chat",
            EndpointKind::Embeddings => "embeddings",
            EndpointKind::ImagesGenerations => "images.generations",
            EndpointKind::Moderations => "moderations",
            EndpointKind::Rerank => "rerank",
            EndpointKind::Search => "search",
            EndpointKind::AudioSpeech => "audio.speech",
            EndpointKind::AudioTranscriptions => "audio.transcriptions",
            EndpointKind::MusicGenerations => "music.generations",
            EndpointKind::VideosGenerations => "videos.generations",
        }
    }

    pub fn capability(self) -> &'static str {
        match self {
            EndpointKind::ChatCompletions | EndpointKind::Completions => "chat",
            EndpointKind::Messages => "messages",
            EndpointKind::Responses => "responses",
            EndpointKind::OllamaChat => "ollama.chat",
            EndpointKind::Embeddings => "embeddings",
            EndpointKind::ImagesGenerations => "images",
            EndpointKind::Moderations => "moderations",
            EndpointKind::Rerank => "rerank",
            EndpointKind::Search => "search",
            EndpointKind::AudioSpeech => "audio.speech",
            EndpointKind::AudioTranscriptions => "audio.transcriptions",
            EndpointKind::MusicGenerations => "music",
            EndpointKind::VideosGenerations => "video",
        }
    }

    pub fn is_chat_family(self) -> bool {
        matches!(
            self,
            EndpointKind::ChatCompletions
                | EndpointKind::Completions
                | EndpointKind::Messages
                | EndpointKind::Responses
                | EndpointKind::OllamaChat
        )
    }
}

pub fn default_supported_endpoints() -> Vec<String> {
    vec![
        "chat".to_string(),
        "messages".to_string(),
        "responses".to_string(),
        "ollama.chat".to_string(),
        "embeddings".to_string(),
        "images".to_string(),
        "moderations".to_string(),
        "rerank".to_string(),
        "search".to_string(),
        "audio.speech".to_string(),
        "audio.transcriptions".to_string(),
        "music".to_string(),
        "video".to_string(),
    ]
}

pub fn default_auth_type() -> String {
    "apikey".to_string()
}

pub fn default_auth_header() -> String {
    "bearer".to_string()
}

pub fn supports_endpoint(capabilities: &[String], endpoint: EndpointKind) -> bool {
    if capabilities.is_empty() {
        return true;
    }

    let normalized: Vec<String> = capabilities
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect();

    normalized.iter().any(|value| value == endpoint.capability())
        || (endpoint.is_chat_family() && normalized.iter().any(|value| value == "chat"))
        || (endpoint == EndpointKind::ImagesGenerations
            && normalized.iter().any(|value| value == "images.generations"))
        || (matches!(endpoint, EndpointKind::AudioSpeech | EndpointKind::AudioTranscriptions)
            && normalized.iter().any(|value| value == "audio"))
        || (endpoint == EndpointKind::MusicGenerations
            && normalized.iter().any(|value| value == "music.generations"))
        || (endpoint == EndpointKind::VideosGenerations
            && (normalized.iter().any(|value| value == "videos")
                || normalized.iter().any(|value| value == "videos.generations")))
        || (endpoint == EndpointKind::VideosGenerations
            && normalized.iter().any(|value| value == "video.generations"))
        || (endpoint == EndpointKind::VideosGenerations
            && normalized.iter().any(|value| value == "video"))
}

fn default_true() -> bool {
    true
}
