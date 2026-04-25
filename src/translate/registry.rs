use std::collections::HashMap;

use serde_json::Value;

use crate::error::AppResult;

use super::format::ProtocolFormat;

pub type RequestTranslatorFn = fn(model: &str, body: &Value, stream: bool) -> AppResult<Value>;
pub type ResponseTranslatorFn = fn(body: &Value) -> AppResult<Value>;
pub type StreamChunkTranslatorFn = fn(chunk_data: &str) -> AppResult<Option<String>>;

pub struct TranslatorRegistry {
    request: HashMap<(ProtocolFormat, ProtocolFormat), RequestTranslatorFn>,
    response: HashMap<(ProtocolFormat, ProtocolFormat), ResponseTranslatorFn>,
    stream_chunk: HashMap<(ProtocolFormat, ProtocolFormat), StreamChunkTranslatorFn>,
}

impl TranslatorRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            request: HashMap::new(),
            response: HashMap::new(),
            stream_chunk: HashMap::new(),
        };
        registry.register_all();
        registry
    }

    fn register_all(&mut self) {
        use super::{
            claude_to_openai, gemini_to_openai, ollama, openai_to_claude, openai_to_gemini, stream,
        };

        // OpenAI -> Claude
        self.request.insert(
            (ProtocolFormat::OpenAI, ProtocolFormat::Claude),
            openai_to_claude::translate_request,
        );
        // Claude -> OpenAI
        self.response.insert(
            (ProtocolFormat::Claude, ProtocolFormat::OpenAI),
            claude_to_openai::translate_response,
        );

        // OpenAI -> Gemini
        self.request.insert(
            (ProtocolFormat::OpenAI, ProtocolFormat::Gemini),
            openai_to_gemini::translate_request,
        );
        // Gemini -> OpenAI
        self.response.insert(
            (ProtocolFormat::Gemini, ProtocolFormat::OpenAI),
            gemini_to_openai::translate_response,
        );

        // OpenAI -> Ollama
        self.request.insert(
            (ProtocolFormat::OpenAI, ProtocolFormat::Ollama),
            ollama::translate_request_to_ollama,
        );
        // Ollama -> OpenAI
        self.response.insert(
            (ProtocolFormat::Ollama, ProtocolFormat::OpenAI),
            ollama::translate_response_to_openai,
        );

        // Stream chunk translators
        self.stream_chunk.insert(
            (ProtocolFormat::Claude, ProtocolFormat::OpenAI),
            stream::claude_chunk_to_openai,
        );
        self.stream_chunk.insert(
            (ProtocolFormat::Gemini, ProtocolFormat::OpenAI),
            stream::gemini_chunk_to_openai,
        );
    }

    /// Translate a request body from source format to target format.
    /// Uses hub-and-spoke: if no direct translator, goes source->OpenAI->target.
    pub fn translate_request(
        &self,
        source: ProtocolFormat,
        target: ProtocolFormat,
        model: &str,
        body: &Value,
        stream: bool,
    ) -> AppResult<Value> {
        if source == target {
            return Ok(body.clone());
        }

        // Try direct translator
        if let Some(translator) = self.request.get(&(source, target)) {
            return translator(model, body, stream);
        }

        // Hub-and-spoke: source -> OpenAI -> target
        if source != ProtocolFormat::OpenAI && target != ProtocolFormat::OpenAI {
            // First translate source -> OpenAI
            if let Some(to_openai) = self.response.get(&(source, ProtocolFormat::OpenAI)) {
                let openai_body = to_openai(body)?;
                // Then translate OpenAI -> target
                if let Some(from_openai) = self.request.get(&(ProtocolFormat::OpenAI, target)) {
                    return from_openai(model, &openai_body, stream);
                }
            }
        }

        // No translator found, passthrough
        Ok(body.clone())
    }

    /// Translate a response body from source format to target format.
    pub fn translate_response(
        &self,
        source: ProtocolFormat,
        target: ProtocolFormat,
        body: &Value,
    ) -> AppResult<Value> {
        if source == target {
            return Ok(body.clone());
        }

        // Try direct translator
        if let Some(translator) = self.response.get(&(source, target)) {
            return translator(body);
        }

        // Hub-and-spoke: source -> OpenAI -> target
        if source != ProtocolFormat::OpenAI && target != ProtocolFormat::OpenAI {
            if let Some(to_openai) = self.response.get(&(source, ProtocolFormat::OpenAI)) {
                let openai_body = to_openai(body)?;
                if let Some(from_openai) = self.response.get(&(ProtocolFormat::OpenAI, target)) {
                    return from_openai(&openai_body);
                }
                return Ok(openai_body);
            }
        }

        // No translator found, passthrough
        Ok(body.clone())
    }

    /// Get a stream chunk translator for the given format pair
    pub fn get_stream_chunk_translator(
        &self,
        source: ProtocolFormat,
        target: ProtocolFormat,
    ) -> Option<StreamChunkTranslatorFn> {
        if source == target {
            return None;
        }
        self.stream_chunk.get(&(source, target)).copied()
    }
}
