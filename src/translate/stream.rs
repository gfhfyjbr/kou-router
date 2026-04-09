use serde_json::{json, Value};
use uuid::Uuid;

use super::common::{claude_stop_to_openai_finish, gemini_finish_to_openai};
use crate::error::{AppError, AppResult};

/// Translate a Claude SSE chunk into OpenAI SSE format.
///
/// `chunk_data` is the raw SSE text for one event, potentially containing both
/// `event:` and `data:` lines. Returns `Ok(None)` for events that have no
/// OpenAI equivalent (ping, content_block_start, etc.).
pub fn claude_chunk_to_openai(chunk_data: &str) -> AppResult<Option<String>> {
    let mut event_type: Option<&str> = None;
    let mut data_line: Option<&str> = None;

    for line in chunk_data.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_line = Some(rest.trim());
        }
    }

    let event = match event_type {
        Some(e) => e,
        // No event field — nothing to translate.
        None => return Ok(None),
    };

    let id = Uuid::new_v4().to_string();

    match event {
        "content_block_delta" => {
            let data_str = data_line.ok_or_else(|| {
                AppError::BadRequest("content_block_delta missing data".into())
            })?;
            let data: Value = serde_json::from_str(data_str)?;

            let delta_type = data
                .pointer("/delta/type")
                .and_then(Value::as_str)
                .unwrap_or("");

            if delta_type != "text_delta" {
                return Ok(None);
            }

            let text = data
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .unwrap_or("");

            let openai = json!({
                "id": format!("chatcmpl-{id}"),
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": { "content": text },
                    "finish_reason": null
                }]
            });

            Ok(Some(format!("data: {}\n\n", openai)))
        }

        "message_delta" => {
            let data_str = data_line.ok_or_else(|| {
                AppError::BadRequest("message_delta missing data".into())
            })?;
            let data: Value = serde_json::from_str(data_str)?;

            if let Some(stop_reason) = data
                .pointer("/delta/stop_reason")
                .and_then(Value::as_str)
            {
                let finish = claude_stop_to_openai_finish(stop_reason);
                let openai = json!({
                    "id": format!("chatcmpl-{id}"),
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": finish
                    }]
                });
                Ok(Some(format!("data: {}\n\n", openai)))
            } else {
                Ok(None)
            }
        }

        "message_stop" => {
            let openai = json!({
                "id": format!("chatcmpl-{id}"),
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]
            });
            Ok(Some(format!("data: {}\n\ndata: [DONE]\n\n", openai)))
        }

        // ping, message_start, content_block_start, content_block_stop, etc.
        _ => Ok(None),
    }
}

/// Translate a Gemini SSE chunk into OpenAI SSE format.
///
/// `chunk_data` is a raw SSE line, typically `data: {...}`.
/// Returns `Ok(None)` when the line contains no translatable content.
pub fn gemini_chunk_to_openai(chunk_data: &str) -> AppResult<Option<String>> {
    let json_str = chunk_data
        .lines()
        .find_map(|line| line.trim().strip_prefix("data:").map(str::trim))
        .unwrap_or(chunk_data.trim());

    if json_str.is_empty() || json_str == "[DONE]" {
        return Ok(None);
    }

    let data: Value = serde_json::from_str(json_str)?;

    let text = data
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str);

    let finish_reason = data
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str);

    // Nothing useful in this chunk.
    if text.is_none() && finish_reason.is_none() {
        return Ok(None);
    }

    let id = Uuid::new_v4().to_string();
    let mut out = String::new();

    // Build the delta object: include content only when text is present.
    let delta = match text {
        Some(t) => json!({ "content": t }),
        None => json!({}),
    };

    let mapped_finish = finish_reason.map(gemini_finish_to_openai);

    let openai = json!({
        "id": format!("chatcmpl-{id}"),
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": mapped_finish
        }]
    });

    out.push_str(&format!("data: {}\n\n", openai));

    if finish_reason.is_some() {
        out.push_str("data: [DONE]\n\n");
    }

    Ok(Some(out))
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // ── Claude chunk tests ──────────────────────────────────────

    #[test]
    fn test_claude_text_delta() {
        let input = "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}";
        let result = claude_chunk_to_openai(input).unwrap().unwrap();
        assert!(result.starts_with("data: "));
        assert!(result.ends_with("\n\n"));
        let json_str = result.strip_prefix("data: ").unwrap().trim();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["object"], "chat.completion.chunk");
        assert_eq!(v["choices"][0]["delta"]["content"], "hello");
        assert!(v["choices"][0]["finish_reason"].is_null());
        assert!(v["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    #[test]
    fn test_claude_message_delta_stop() {
        let input = "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"}}";
        let result = claude_chunk_to_openai(input).unwrap().unwrap();
        let json_str = result.strip_prefix("data: ").unwrap().trim();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        assert_eq!(v["choices"][0]["delta"], json!({}));
    }

    #[test]
    fn test_claude_message_stop() {
        let input = "event: message_stop\ndata: {}";
        let result = claude_chunk_to_openai(input).unwrap().unwrap();
        assert!(result.contains("data: "));
        assert!(result.contains("[DONE]"));
        // Parse the first SSE frame (before the [DONE] line)
        let first_frame = result.split("\n\n").next().unwrap();
        let json_str = first_frame.strip_prefix("data: ").unwrap();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_claude_ping_ignored() {
        let input = "event: ping\ndata: {}";
        assert!(claude_chunk_to_openai(input).unwrap().is_none());
    }

    #[test]
    fn test_claude_content_block_start_ignored() {
        let input = "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}";
        assert!(claude_chunk_to_openai(input).unwrap().is_none());
    }

    #[test]
    fn test_claude_no_event_ignored() {
        let input = "some random line with no event prefix";
        assert!(claude_chunk_to_openai(input).unwrap().is_none());
    }

    #[test]
    fn test_claude_non_text_delta_ignored() {
        let input = "event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}";
        assert!(claude_chunk_to_openai(input).unwrap().is_none());
    }

    // ── Gemini chunk tests ──────────────────────────────────────

    #[test]
    fn test_gemini_text_chunk() {
        let input = r#"data: {"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}"#;
        let result = gemini_chunk_to_openai(input).unwrap().unwrap();
        let json_str = result.strip_prefix("data: ").unwrap().trim();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["choices"][0]["delta"]["content"], "hi");
        assert!(v["choices"][0]["finish_reason"].is_null());
        assert!(!result.contains("[DONE]"));
    }

    #[test]
    fn test_gemini_finish_chunk() {
        let input = r#"data: {"candidates":[{"finishReason":"STOP","content":{"parts":[]}}]}"#;
        let result = gemini_chunk_to_openai(input).unwrap().unwrap();
        assert!(result.contains("[DONE]"));
        let first_frame = result.split("\n\n").next().unwrap();
        let json_str = first_frame.strip_prefix("data: ").unwrap();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_gemini_done_ignored() {
        let input = "data: [DONE]";
        assert!(gemini_chunk_to_openai(input).unwrap().is_none());
    }

    #[test]
    fn test_gemini_empty_chunk() {
        assert!(gemini_chunk_to_openai("").unwrap().is_none());
    }
}
