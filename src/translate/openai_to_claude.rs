use serde_json::{Value, json};

use super::common::*;
use crate::error::{AppError, AppResult};

const DEFAULT_MAX_TOKENS: u64 = 8192;

pub fn translate_request(model: &str, body: &Value, stream: bool) -> AppResult<Value> {
    let messages = body["messages"]
        .as_array()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            AppError::BadRequest("messages array is required and must not be empty".into())
        })?;

    // Partition system messages from the rest.
    let mut system_blocks: Vec<Value> = Vec::new();
    let mut claude_messages: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("");
        match role {
            "system" => {
                system_blocks.extend(openai_system_message_to_claude_blocks(msg));
            }
            "user" => {
                claude_messages.push(json!({
                    "role": "user",
                    "content": openai_content_to_claude_blocks(&msg["content"]),
                }));
            }
            "assistant" => {
                let mut blocks: Vec<Value> = Vec::new();

                // Text content, if present.
                if let Some(text) = msg["content"].as_str()
                    && !text.is_empty()
                {
                    blocks.push(json!({"type": "text", "text": text}));
                }

                // Tool calls become tool_use blocks.
                if msg.get("tool_calls").is_some() && !msg["tool_calls"].is_null() {
                    blocks.extend(openai_tool_calls_to_claude(&msg["tool_calls"]));
                }

                // If nothing was collected (empty assistant turn), emit an empty text block
                // so Claude still gets a well-formed message.
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": ""}));
                }

                claude_messages.push(json!({
                    "role": "assistant",
                    "content": Value::Array(blocks),
                }));
            }
            "tool" => {
                claude_messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": msg["tool_call_id"],
                        "content": msg["content"],
                    }],
                }));
            }
            _ => {
                // Unknown roles are silently dropped — Claude would reject them anyway.
            }
        }
    }

    // Build the system field.
    // If response_format requests JSON, append an instruction.
    if body.get("response_format").is_some()
        && body["response_format"]["type"].as_str() == Some("json_object")
    {
        system_blocks.push(json!({"type": "text", "text": "Respond only with valid JSON."}));
    }

    let mut result = json!({
        "model": model,
        "messages": claude_messages,
        "max_tokens": body["max_tokens"]
            .as_u64()
            .unwrap_or(DEFAULT_MAX_TOKENS),
        "stream": stream,
    });

    if !system_blocks.is_empty() {
        result["system"] = Value::Array(system_blocks);
    }

    if let Some(cache_control) = request_cache_control(body) {
        result["cache_control"] = cache_control;
    }

    // Pass-through numeric parameters.
    for key in &["temperature", "top_p", "top_k"] {
        if let Some(v) = body.get(*key)
            && !v.is_null()
        {
            result[*key] = v.clone();
        }
    }

    // Tools.
    if let Some(tools) = body.get("tools")
        && !tools.is_null()
    {
        let converted = openai_tools_to_claude(tools);
        if converted.as_array().is_some_and(|arr| !arr.is_empty()) {
            result["tools"] = converted;
        }
    }

    // Tool choice.
    if let Some(tc) = body.get("tool_choice")
        && !tc.is_null()
    {
        result["tool_choice"] = match tc.as_str() {
            Some("auto") => json!({"type": "auto"}),
            Some("required") => json!({"type": "any"}),
            Some("none") => {
                // Claude doesn't have a "none" tool_choice; omit tools instead.
                result.as_object_mut().unwrap().remove("tools");
                // Don't set tool_choice at all.
                Value::Null
            }
            _ => {
                // Object form: {"function":{"name":"X"}} -> {"type":"tool","name":"X"}
                if let Some(name) = tc.get("function").and_then(|f| f["name"].as_str()) {
                    json!({"type": "tool", "name": name})
                } else {
                    json!({"type": "auto"})
                }
            }
        };
        // Clean up null sentinel from the "none" branch.
        if result["tool_choice"].is_null() {
            result.as_object_mut().unwrap().remove("tool_choice");
        }
    }

    Ok(result)
}

fn request_cache_control(body: &Value) -> Option<Value> {
    body.get("cache_control")
        .or_else(|| body.get("anthropic_cache_control"))
        .or_else(|| body.get("claude_cache_control"))
        .cloned()
        .or_else(|| match body.get("prompt_cache") {
            Some(Value::Bool(true)) => Some(json!({"type": "ephemeral"})),
            Some(Value::Object(object)) => {
                if let Some(cache_control) = object.get("cache_control") {
                    Some(cache_control.clone())
                } else if object.get("type").is_some() || object.get("ttl").is_some() {
                    Some(Value::Object(object.clone()))
                } else {
                    None
                }
            }
            _ => None,
        })
}

fn openai_system_message_to_claude_blocks(message: &Value) -> Vec<Value> {
    let content = message.get("content").unwrap_or(&Value::Null);
    let mut blocks = match content {
        Value::String(text) => vec![json!({"type": "text", "text": text})],
        Value::Array(items) => {
            let value = openai_content_to_claude_blocks(&Value::Array(items.clone()));
            value.as_array().cloned().unwrap_or_default()
        }
        Value::Null => Vec::new(),
        other => vec![json!({"type": "text", "text": other.to_string()})],
    };

    if message.get("cache_control").is_some()
        && !blocks
            .iter()
            .any(|block| block.get("cache_control").is_some())
        && let Some(last) = blocks.last_mut()
    {
        copy_cache_control(message, last);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_user_message() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let msgs = result["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        let content = msgs[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
    }

    #[test]
    fn test_system_message_extracted() {
        let body = json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "hi"}
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let system = result["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "You are helpful.");
        // System messages must not appear in the messages array.
        let msgs = result["messages"].as_array().unwrap();
        assert!(msgs.iter().all(|m| m["role"] != "system"));
    }

    #[test]
    fn test_multiple_system_messages_become_blocks() {
        let body = json!({
            "messages": [
                {"role": "system", "content": "First."},
                {"role": "system", "content": "Second."},
                {"role": "user", "content": "go"}
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["system"][0]["text"], "First.");
        assert_eq!(result["system"][1]["text"], "Second.");
    }

    #[test]
    fn test_prompt_cache_control_passthrough() {
        let body = json!({
            "messages": [
                {
                    "role": "system",
                    "content": "Stable system.",
                    "cache_control": {"type": "ephemeral"}
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Stable prefix",
                            "cache_control": {"type": "ephemeral", "ttl": "1h"}
                        }
                    ]
                }
            ],
            "tools": [{
                "type": "function",
                "cache_control": {"type": "ephemeral"},
                "function": {
                    "name": "lookup",
                    "description": "Lookup",
                    "parameters": {"type": "object"}
                }
            }],
            "prompt_cache": {"type": "ephemeral"}
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["cache_control"]["type"], "ephemeral");
        assert_eq!(result["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            result["messages"][0]["content"][0]["cache_control"]["ttl"],
            "1h"
        );
        assert_eq!(result["tools"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_assistant_with_text() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello back"}
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let msgs = result["messages"].as_array().unwrap();
        let assistant = &msgs[1];
        assert_eq!(assistant["role"], "assistant");
        let blocks = assistant["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "hello back");
    }

    #[test]
    fn test_assistant_with_tool_calls() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Paris\"}"
                        }
                    }]
                }
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let assistant = &result["messages"][1];
        let blocks = assistant["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "tool_use");
        assert_eq!(blocks[0]["name"], "get_weather");
        assert_eq!(blocks[0]["id"], "call_1");
        assert_eq!(blocks[0]["input"]["city"], "Paris");
    }

    #[test]
    fn test_tool_role_converted() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": "ok"},
                {
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "result text"
                }
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let tool_msg = &result["messages"][2];
        assert_eq!(tool_msg["role"], "user");
        let content = tool_msg["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[0]["content"], "result text");
    }

    #[test]
    fn test_json_response_format_appends_instruction() {
        let body = json!({
            "messages": [{"role": "user", "content": "give json"}],
            "response_format": {"type": "json_object"}
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let system_text = result["system"][0]["text"].as_str().unwrap();
        assert!(system_text.contains("Respond only with valid JSON."));
    }

    #[test]
    fn test_tools_converted() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "search",
                    "description": "Search the web",
                    "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
                }
            }]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "search");
        assert_eq!(tools[0]["description"], "Search the web");
        assert!(tools[0].get("input_schema").is_some());
    }

    #[test]
    fn test_tool_choice_auto() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type":"function","function":{"name":"f","description":"","parameters":{"type":"object"}}}],
            "tool_choice": "auto"
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["tool_choice"]["type"], "auto");
    }

    #[test]
    fn test_tool_choice_required() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type":"function","function":{"name":"f","description":"","parameters":{"type":"object"}}}],
            "tool_choice": "required"
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["tool_choice"]["type"], "any");
    }

    #[test]
    fn test_tool_choice_none_removes_tools() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type":"function","function":{"name":"f","description":"","parameters":{"type":"object"}}}],
            "tool_choice": "none"
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert!(result.get("tools").is_none() || result["tools"].is_null());
        assert!(result.get("tool_choice").is_none() || result["tool_choice"].is_null());
    }

    #[test]
    fn test_image_content_base64() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "what is this?"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc123"}}
                ]
            }]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let content = result["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "abc123");
    }

    #[test]
    fn test_numeric_params_passthrough() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.7,
            "top_p": 0.9
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["temperature"], 0.7);
        assert_eq!(result["top_p"], 0.9);
    }

    #[test]
    fn test_max_tokens_default() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["max_tokens"], 8192);
    }

    #[test]
    fn test_stream_flag_passed() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}]
        });
        let result_stream = translate_request("claude-3-opus", &body, true).unwrap();
        assert_eq!(result_stream["stream"], true);
        let result_no_stream = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result_no_stream["stream"], false);
    }

    // --- Edge cases ---

    #[test]
    fn test_unknown_role_silently_dropped() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "alien", "content": "bzzz"}
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let msgs = result["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn test_empty_assistant_turn_produces_empty_text_block() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": ""}
            ]
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        let blocks = result["messages"][1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "");
    }

    #[test]
    fn test_tool_choice_object_with_function_name() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type":"function","function":{"name":"search","description":"","parameters":{"type":"object"}}}],
            "tool_choice": {"function": {"name": "search"}}
        });
        let result = translate_request("claude-3-opus", &body, false).unwrap();
        assert_eq!(result["tool_choice"]["type"], "tool");
        assert_eq!(result["tool_choice"]["name"], "search");
    }
}
