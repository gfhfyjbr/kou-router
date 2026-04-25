use serde_json::{Value, json};
use uuid::Uuid;

use super::common::*;
use crate::error::{AppError, AppResult};

/// Convert a Google Gemini generateContent response to OpenAI chat/completions format.
pub fn translate_response(body: &Value) -> AppResult<Value> {
    let candidates = body
        .get("candidates")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| AppError::BadRequest("gemini response missing candidates".into()))?;

    let candidate = &candidates[0];
    let parts = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());

    // Separate text parts from functionCall parts.
    let mut text_pieces: Vec<&str> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    if let Some(parts) = parts {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                text_pieces.push(text);
            } else if let Some(fc) = part.get("functionCall") {
                let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let empty_obj = json!({});
                let args = fc.get("args").unwrap_or(&empty_obj);
                let args_str = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(json!({
                    "id": format!("call_{}", Uuid::new_v4()),
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args_str,
                    }
                }));
            }
        }
    }

    // Build message.
    let mut message = json!({ "role": "assistant" });

    if !text_pieces.is_empty() {
        message["content"] = json!(text_pieces.concat());
    } else if tool_calls.is_empty() {
        // No text and no tools — default to empty string.
        message["content"] = json!("");
    } else {
        message["content"] = Value::Null;
    }

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    // finish_reason
    let finish_reason = candidate
        .get("finishReason")
        .and_then(|v| v.as_str())
        .map(gemini_finish_to_openai)
        .unwrap_or("stop");

    // usage — Gemini uses usageMetadata at the top level.
    let usage_meta = body.get("usageMetadata");
    let prompt_tokens = usage_meta
        .and_then(|u| u.get("promptTokenCount"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = usage_meta
        .and_then(|u| u.get("candidatesTokenCount"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_tokens = usage_meta
        .and_then(|u| u.get("totalTokenCount"))
        .and_then(|v| v.as_u64())
        .unwrap_or(prompt_tokens + completion_tokens);

    let model = body
        .get("modelVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("gemini");

    let id = format!("chatcmpl-gemini-{}", Uuid::new_v4());

    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_text_candidate() {
        let body = json!({
            "candidates": [{
                "content": { "parts": [{"text": "hello"}] },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-1.5-pro"
        });
        let result = translate_response(&body).unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "hello");
        assert_eq!(result["choices"][0]["message"]["role"], "assistant");
        assert_eq!(result["choices"][0]["index"], 0);
        assert_eq!(result["model"], "gemini-1.5-pro");
        assert_eq!(result["object"], "chat.completion");
    }

    #[test]
    fn test_function_call_candidate() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "NYC"}
                        }
                    }]
                },
                "finishReason": "STOP"
            }]
        });
        let result = translate_response(&body).unwrap();
        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        let tc = &tool_calls[0];
        assert!(tc["id"].as_str().unwrap().starts_with("call_"));
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
        let args: serde_json::Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "NYC");
        // content should be null when only tool calls present
        assert!(result["choices"][0]["message"]["content"].is_null());
    }

    #[test]
    fn test_finish_reason_mapping() {
        let cases = [
            ("STOP", "stop"),
            ("MAX_TOKENS", "length"),
            ("SAFETY", "content_filter"),
        ];
        for (gemini_reason, expected) in cases {
            let body = json!({
                "candidates": [{
                    "content": { "parts": [{"text": "x"}] },
                    "finishReason": gemini_reason
                }]
            });
            let result = translate_response(&body).unwrap();
            assert_eq!(
                result["choices"][0]["finish_reason"].as_str().unwrap(),
                expected,
                "mapping {} -> {}",
                gemini_reason,
                expected
            );
        }
    }

    #[test]
    fn test_usage_metadata() {
        let body = json!({
            "candidates": [{
                "content": { "parts": [{"text": "ok"}] },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 10,
                "totalTokenCount": 15
            }
        });
        let result = translate_response(&body).unwrap();
        assert_eq!(result["usage"]["prompt_tokens"], 5);
        assert_eq!(result["usage"]["completion_tokens"], 10);
        assert_eq!(result["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_empty_candidates_error() {
        let body = json!({ "candidates": [] });
        let result = translate_response(&body);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_candidates_key_error() {
        let body = json!({ "modelVersion": "gemini-1.5-pro" });
        let result = translate_response(&body);
        assert!(result.is_err());
    }

    #[test]
    fn test_text_and_function_call() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "Let me check the weather."},
                        {"functionCall": {"name": "get_weather", "args": {"city": "SF"}}}
                    ]
                },
                "finishReason": "STOP"
            }]
        });
        let result = translate_response(&body).unwrap();
        let msg = &result["choices"][0]["message"];
        assert_eq!(msg["content"], "Let me check the weather.");
        let tool_calls = msg["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        assert!(tool_calls[0]["id"].as_str().unwrap().starts_with("call_"));
    }
}
