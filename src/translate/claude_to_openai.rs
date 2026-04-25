use serde_json::{Value, json};

use super::common::*;
use crate::error::AppResult;

/// Convert an Anthropic Messages API response to OpenAI chat/completions format.
pub fn translate_response(body: &Value) -> AppResult<Value> {
    let content_blocks = body.get("content");

    // Extract text from all text blocks.
    let text_content = content_blocks
        .and_then(|v| v.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type")?.as_str()? == "text" {
                        Some(b.get("text")?.as_str().unwrap_or(""))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    // Extract tool_use blocks -> OpenAI tool_calls.
    let tool_calls = content_blocks
        .map(|c| claude_tool_use_to_openai(c))
        .unwrap_or_default();

    // Build message object. Include content even when tool_calls are present (both can coexist).
    let mut message = json!({
        "role": "assistant",
    });

    // Content: set to the concatenated text, or null if empty and tool_calls present.
    if !text_content.is_empty() {
        message["content"] = json!(text_content);
    } else if tool_calls.is_empty() {
        // No text, no tools — default to empty string.
        message["content"] = json!("");
    } else {
        message["content"] = Value::Null;
    }

    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    // finish_reason
    let finish_reason = body
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(claude_stop_to_openai_finish)
        .unwrap_or("stop");

    // usage
    let usage = body.get("usage");
    let prompt_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // id — prefix with chatcmpl- if not already prefixed.
    let raw_id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let id = if raw_id.starts_with("chatcmpl-") {
        raw_id.to_string()
    } else {
        format!("chatcmpl-{raw_id}")
    };

    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");

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
            "total_tokens": prompt_tokens + completion_tokens,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_text_response() {
        let body = json!({
            "id": "msg_1",
            "model": "claude-3",
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        });
        let res = translate_response(&body).unwrap();
        assert_eq!(res["choices"][0]["message"]["content"], "hi");
        assert_eq!(res["choices"][0]["message"]["role"], "assistant");
        assert!(res["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn test_tool_use_response() {
        let body = json!({
            "id": "msg_2",
            "model": "claude-3",
            "content": [{
                "type": "tool_use",
                "id": "1",
                "name": "fn",
                "input": {}
            }],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        });
        let res = translate_response(&body).unwrap();
        let msg = &res["choices"][0]["message"];
        // No text blocks → content is null when tool_calls present
        assert!(msg["content"].is_null());
        let tc = msg["tool_calls"].as_array().expect("tool_calls present");
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0]["type"], "function");
        assert_eq!(tc[0]["function"]["name"], "fn");
        assert_eq!(tc[0]["id"], "1");
    }

    #[test]
    fn test_text_and_tool_use() {
        let body = json!({
            "id": "msg_3",
            "model": "claude-3",
            "content": [
                {"type": "text", "text": "thinking"},
                {"type": "tool_use", "id": "2", "name": "calc", "input": {"x": 1}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 5, "output_tokens": 10}
        });
        let res = translate_response(&body).unwrap();
        let msg = &res["choices"][0]["message"];
        assert_eq!(msg["content"], "thinking");
        let tc = msg["tool_calls"].as_array().expect("tool_calls present");
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0]["function"]["name"], "calc");
    }

    #[test]
    fn test_stop_reason_mapping() {
        let make = |reason: &str| {
            json!({
                "id": "msg_x",
                "model": "claude-3",
                "content": [{"type": "text", "text": ""}],
                "stop_reason": reason,
                "usage": {"input_tokens": 0, "output_tokens": 0}
            })
        };
        let cases = [
            ("end_turn", "stop"),
            ("max_tokens", "length"),
            ("tool_use", "tool_calls"),
        ];
        for (claude_reason, expected) in cases {
            let res = translate_response(&make(claude_reason)).unwrap();
            assert_eq!(
                res["choices"][0]["finish_reason"], expected,
                "stop_reason '{claude_reason}' should map to '{expected}'"
            );
        }
    }

    #[test]
    fn test_usage_mapping() {
        let body = json!({
            "id": "msg_u",
            "model": "claude-3",
            "content": [],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });
        let res = translate_response(&body).unwrap();
        let usage = &res["usage"];
        assert_eq!(usage["prompt_tokens"], 10);
        assert_eq!(usage["completion_tokens"], 20);
        assert_eq!(usage["total_tokens"], 30);
    }

    #[test]
    fn test_id_prefix() {
        let make = |id: &str| {
            json!({
                "id": id,
                "model": "claude-3",
                "content": [],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 0, "output_tokens": 0}
            })
        };
        let res1 = translate_response(&make("msg_123")).unwrap();
        assert_eq!(res1["id"], "chatcmpl-msg_123");

        let res2 = translate_response(&make("chatcmpl-x")).unwrap();
        assert_eq!(res2["id"], "chatcmpl-x");
    }

    #[test]
    fn test_empty_content() {
        // No content blocks at all
        let body = json!({
            "id": "msg_e",
            "model": "claude-3",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let res = translate_response(&body).unwrap();
        assert_eq!(res["choices"][0]["message"]["content"], "");
        assert!(res["choices"][0]["message"].get("tool_calls").is_none());
    }
}
