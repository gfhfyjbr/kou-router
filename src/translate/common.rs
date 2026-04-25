use serde_json::{Value, json};

/// Map OpenAI finish_reason to Claude stop_reason
pub fn openai_finish_to_claude_stop(reason: &str) -> &str {
    match reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "content_filter" => "end_turn",
        _ => "end_turn",
    }
}

/// Map Claude stop_reason to OpenAI finish_reason
pub fn claude_stop_to_openai_finish(reason: &str) -> &str {
    match reason {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "stop_sequence" => "stop",
        _ => "stop",
    }
}

/// Map Gemini finish reason to OpenAI finish_reason
pub fn gemini_finish_to_openai(reason: &str) -> &str {
    match reason {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        "SAFETY" => "content_filter",
        "RECITATION" => "content_filter",
        _ => "stop",
    }
}

/// Map OpenAI finish_reason to Gemini finish reason
pub fn openai_finish_to_gemini(reason: &str) -> &str {
    match reason {
        "stop" => "STOP",
        "length" => "MAX_TOKENS",
        "content_filter" => "SAFETY",
        _ => "STOP",
    }
}

/// Convert OpenAI message content (string or array) to Claude content blocks
pub fn openai_content_to_claude_blocks(content: &Value) -> Value {
    match content {
        Value::String(text) => json!([{"type": "text", "text": text}]),
        Value::Array(items) => {
            let blocks: Vec<Value> = items
                .iter()
                .filter_map(|item| {
                    let item_type = item.get("type")?.as_str()?;
                    match item_type {
                        "text" => Some(json!({
                            "type": "text",
                            "text": item.get("text").and_then(|v| v.as_str()).unwrap_or("")
                        })),
                        "image_url" => {
                            let url = item
                                .get("image_url")
                                .and_then(|v| v.get("url"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if url.starts_with("data:") {
                                // data:image/png;base64,... -> extract media_type and base64
                                if let Some((meta, data)) = url.split_once(',') {
                                    let media_type = meta
                                        .strip_prefix("data:")
                                        .and_then(|s| s.split(';').next())
                                        .unwrap_or("image/png");
                                    Some(json!({
                                        "type": "image",
                                        "source": {
                                            "type": "base64",
                                            "media_type": media_type,
                                            "data": data
                                        }
                                    }))
                                } else {
                                    None
                                }
                            } else {
                                Some(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "url",
                                        "url": url
                                    }
                                }))
                            }
                        }
                        _ => Some(item.clone()),
                    }
                })
                .collect();
            Value::Array(blocks)
        }
        Value::Null => json!([]),
        _ => json!([{"type": "text", "text": content.to_string()}]),
    }
}

/// Extract text from Claude content blocks
pub fn claude_blocks_to_text(content: &Value) -> String {
    match content {
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// Convert OpenAI tool_calls array to Claude tool_use content blocks
pub fn openai_tool_calls_to_claude(tool_calls: &Value) -> Vec<Value> {
    tool_calls
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let func = call.get("function")?;
                    let name = func.get("name")?.as_str()?;
                    let args_str = func.get("arguments")?.as_str().unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    Some(json!({
                        "type": "tool_use",
                        "id": call.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        "name": name,
                        "input": input
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Convert Claude tool_use blocks to OpenAI tool_calls array
pub fn claude_tool_use_to_openai(content: &Value) -> Vec<Value> {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    if block.get("type")?.as_str()? != "tool_use" {
                        return None;
                    }
                    Some(json!({
                        "id": block.get("id").cloned().unwrap_or(json!("")),
                        "type": "function",
                        "function": {
                            "name": block.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                            "arguments": serde_json::to_string(
                                block.get("input").unwrap_or(&json!({}))
                            ).unwrap_or_else(|_| "{}".to_string())
                        }
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Convert OpenAI tools to Claude tools format
pub fn openai_tools_to_claude(tools: &Value) -> Value {
    match tools.as_array() {
        Some(tools) => Value::Array(
            tools
                .iter()
                .filter_map(|tool| {
                    let func = tool.get("function")?;
                    Some(json!({
                        "name": func.get("name").cloned().unwrap_or(json!("")),
                        "description": func.get("description").cloned().unwrap_or(json!("")),
                        "input_schema": func.get("parameters").cloned().unwrap_or(json!({"type": "object"}))
                    }))
                })
                .collect(),
        ),
        None => json!([]),
    }
}

/// Convert Claude tools to OpenAI tools format
pub fn claude_tools_to_openai(tools: &Value) -> Value {
    match tools.as_array() {
        Some(tools) => Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.get("name").cloned().unwrap_or(json!("")),
                            "description": tool.get("description").cloned().unwrap_or(json!("")),
                            "parameters": tool.get("input_schema").cloned().unwrap_or(json!({"type": "object"}))
                        }
                    })
                })
                .collect(),
        ),
        None => json!([]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_openai_finish_to_claude() {
        assert_eq!(openai_finish_to_claude_stop("stop"), "end_turn");
        assert_eq!(openai_finish_to_claude_stop("length"), "max_tokens");
        assert_eq!(openai_finish_to_claude_stop("tool_calls"), "tool_use");
        assert_eq!(openai_finish_to_claude_stop("function_call"), "tool_use");
        assert_eq!(openai_finish_to_claude_stop("content_filter"), "end_turn");
        assert_eq!(openai_finish_to_claude_stop("unknown"), "end_turn");
    }

    #[test]
    fn test_claude_stop_to_openai() {
        assert_eq!(claude_stop_to_openai_finish("end_turn"), "stop");
        assert_eq!(claude_stop_to_openai_finish("max_tokens"), "length");
        assert_eq!(claude_stop_to_openai_finish("tool_use"), "tool_calls");
        assert_eq!(claude_stop_to_openai_finish("stop_sequence"), "stop");
        assert_eq!(claude_stop_to_openai_finish("unknown"), "stop");
    }

    #[test]
    fn test_gemini_finish_mappings() {
        assert_eq!(gemini_finish_to_openai("STOP"), "stop");
        assert_eq!(gemini_finish_to_openai("MAX_TOKENS"), "length");
        assert_eq!(gemini_finish_to_openai("SAFETY"), "content_filter");
        assert_eq!(gemini_finish_to_openai("RECITATION"), "content_filter");
        assert_eq!(gemini_finish_to_openai("unknown"), "stop");
    }

    #[test]
    fn test_openai_finish_to_gemini() {
        assert_eq!(openai_finish_to_gemini("stop"), "STOP");
        assert_eq!(openai_finish_to_gemini("length"), "MAX_TOKENS");
        assert_eq!(openai_finish_to_gemini("content_filter"), "SAFETY");
        assert_eq!(openai_finish_to_gemini("unknown"), "STOP");
    }

    #[test]
    fn test_openai_content_to_claude_blocks_string() {
        let input = json!("hello");
        let result = openai_content_to_claude_blocks(&input);
        assert_eq!(result, json!([{"type": "text", "text": "hello"}]));
    }

    #[test]
    fn test_openai_content_to_claude_blocks_array() {
        let input = json!([
            {"type": "text", "text": "hi"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
        ]);
        let result = openai_content_to_claude_blocks(&input);
        assert_eq!(
            result,
            json!([
                {"type": "text", "text": "hi"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}
            ])
        );
    }

    #[test]
    fn test_openai_content_to_claude_blocks_url_image() {
        let input = json!([
            {"type": "image_url", "image_url": {"url": "https://example.com/img.png"}}
        ]);
        let result = openai_content_to_claude_blocks(&input);
        assert_eq!(
            result,
            json!([
                {"type": "image", "source": {"type": "url", "url": "https://example.com/img.png"}}
            ])
        );
    }

    #[test]
    fn test_openai_content_to_claude_blocks_null() {
        let input = Value::Null;
        let result = openai_content_to_claude_blocks(&input);
        assert_eq!(result, json!([]));
    }

    #[test]
    fn test_claude_blocks_to_text() {
        let input = json!([
            {"type": "text", "text": "hello"},
            {"type": "text", "text": " world"}
        ]);
        assert_eq!(claude_blocks_to_text(&input), "hello world");
    }

    #[test]
    fn test_claude_blocks_to_text_string() {
        let input = json!("direct");
        assert_eq!(claude_blocks_to_text(&input), "direct");
    }

    #[test]
    fn test_openai_tool_calls_to_claude() {
        let input = json!([{
            "id": "1",
            "function": {"name": "fn", "arguments": "{\"a\":1}"}
        }]);
        let result = openai_tool_calls_to_claude(&input);
        assert_eq!(
            result,
            vec![json!({
                "type": "tool_use",
                "id": "1",
                "name": "fn",
                "input": {"a": 1}
            })]
        );
    }

    #[test]
    fn test_claude_tool_use_to_openai() {
        let input = json!([{
            "type": "tool_use",
            "id": "1",
            "name": "fn",
            "input": {"a": 1}
        }]);
        let result = claude_tool_use_to_openai(&input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["id"], "1");
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["function"]["name"], "fn");
        let args: Value =
            serde_json::from_str(result[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args, json!({"a": 1}));
    }

    #[test]
    fn test_openai_tools_to_claude() {
        let input = json!([{
            "type": "function",
            "function": {
                "name": "fn",
                "description": "desc",
                "parameters": {"type": "object"}
            }
        }]);
        let result = openai_tools_to_claude(&input);
        assert_eq!(
            result,
            json!([{
                "name": "fn",
                "description": "desc",
                "input_schema": {"type": "object"}
            }])
        );
    }

    #[test]
    fn test_claude_tools_to_openai() {
        let input = json!([{
            "name": "fn",
            "description": "desc",
            "input_schema": {"type": "object"}
        }]);
        let result = claude_tools_to_openai(&input);
        assert_eq!(
            result,
            json!([{
                "type": "function",
                "function": {
                    "name": "fn",
                    "description": "desc",
                    "parameters": {"type": "object"}
                }
            }])
        );
    }

    #[test]
    fn test_empty_tool_calls() {
        let input = json!([]);
        assert!(openai_tool_calls_to_claude(&input).is_empty());
        assert!(claude_tool_use_to_openai(&input).is_empty());
        assert_eq!(openai_tools_to_claude(&input), json!([]));
        assert_eq!(claude_tools_to_openai(&input), json!([]));
    }
}
