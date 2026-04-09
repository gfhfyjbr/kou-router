use crate::models::{EndpointKind, ProviderConnection};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolFormat {
    OpenAI,
    OpenAIResponses,
    Claude,
    Gemini,
    Ollama,
}

impl ProtocolFormat {
    /// Detect source format from endpoint kind + request body heuristics
    pub fn detect_source(endpoint: EndpointKind, _body: &Value) -> Self {
        match endpoint {
            EndpointKind::Messages => ProtocolFormat::Claude,
            EndpointKind::Responses => ProtocolFormat::OpenAIResponses,
            EndpointKind::OllamaChat => ProtocolFormat::Ollama,
            EndpointKind::ChatCompletions
            | EndpointKind::Completions
            | EndpointKind::Embeddings
            | EndpointKind::ImagesGenerations
            | EndpointKind::MusicGenerations
            | EndpointKind::VideosGenerations
            | EndpointKind::Moderations
            | EndpointKind::Rerank
            | EndpointKind::Search
            | EndpointKind::AudioSpeech
            | EndpointKind::AudioTranscriptions => ProtocolFormat::OpenAI,
        }
    }

    /// Determine target format from provider configuration
    pub fn from_provider(provider: &ProviderConnection) -> Self {
        // Use explicit protocol_format if set
        if let Some(ref fmt) = provider.protocol_format {
            return match fmt.as_str() {
                "claude" | "anthropic" => ProtocolFormat::Claude,
                "gemini" | "vertex" => ProtocolFormat::Gemini,
                "ollama" => ProtocolFormat::Ollama,
                "openai-responses" => ProtocolFormat::OpenAIResponses,
                _ => ProtocolFormat::OpenAI,
            };
        }
        // Auto-detect from provider name
        match provider.provider.as_str() {
            "anthropic" => ProtocolFormat::Claude,
            "vertex" | "gemini" => ProtocolFormat::Gemini,
            "ollama" => ProtocolFormat::Ollama,
            _ => ProtocolFormat::OpenAI,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProtocolFormat::OpenAI => "openai",
            ProtocolFormat::OpenAIResponses => "openai-responses",
            ProtocolFormat::Claude => "claude",
            ProtocolFormat::Gemini => "gemini",
            ProtocolFormat::Ollama => "ollama",
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::models::EndpointKind;

    fn make_provider(provider: &str, protocol_format: Option<&str>) -> crate::models::ProviderConnection {
        crate::models::ProviderConnection {
            id: String::new(),
            provider: provider.to_string(),
            base_url: String::new(),
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: Default::default(),
            endpoint_paths: Default::default(),
            stream_endpoint_paths: Default::default(),
            model_prefix: String::new(),
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
            protocol_format: protocol_format.map(String::from),
        }
    }

    #[test]
    fn test_detect_source_messages() {
        let body = json!({});
        assert_eq!(ProtocolFormat::detect_source(EndpointKind::Messages, &body), ProtocolFormat::Claude);
    }

    #[test]
    fn test_detect_source_responses() {
        let body = json!({});
        assert_eq!(ProtocolFormat::detect_source(EndpointKind::Responses, &body), ProtocolFormat::OpenAIResponses);
    }

    #[test]
    fn test_detect_source_ollama() {
        let body = json!({});
        assert_eq!(ProtocolFormat::detect_source(EndpointKind::OllamaChat, &body), ProtocolFormat::Ollama);
    }

    #[test]
    fn test_detect_source_chat() {
        let body = json!({});
        assert_eq!(ProtocolFormat::detect_source(EndpointKind::ChatCompletions, &body), ProtocolFormat::OpenAI);
    }

    #[test]
    fn test_detect_source_embeddings() {
        let body = json!({});
        assert_eq!(ProtocolFormat::detect_source(EndpointKind::Embeddings, &body), ProtocolFormat::OpenAI);
    }

    #[test]
    fn test_from_provider_explicit_claude() {
        let p = make_provider("openai", Some("claude"));
        assert_eq!(ProtocolFormat::from_provider(&p), ProtocolFormat::Claude);
    }

    #[test]
    fn test_from_provider_explicit_gemini() {
        let p = make_provider("openai", Some("gemini"));
        assert_eq!(ProtocolFormat::from_provider(&p), ProtocolFormat::Gemini);
    }

    #[test]
    fn test_from_provider_auto_anthropic() {
        let p = make_provider("anthropic", None);
        assert_eq!(ProtocolFormat::from_provider(&p), ProtocolFormat::Claude);
    }

    #[test]
    fn test_from_provider_auto_gemini() {
        let p = make_provider("gemini", None);
        assert_eq!(ProtocolFormat::from_provider(&p), ProtocolFormat::Gemini);
    }

    #[test]
    fn test_from_provider_auto_ollama() {
        let p = make_provider("ollama", None);
        assert_eq!(ProtocolFormat::from_provider(&p), ProtocolFormat::Ollama);
    }

    #[test]
    fn test_from_provider_default_openai() {
        let p = make_provider("openrouter", None);
        assert_eq!(ProtocolFormat::from_provider(&p), ProtocolFormat::OpenAI);
    }
}