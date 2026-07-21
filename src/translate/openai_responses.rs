use serde_json::{Value, json};

use crate::error::AppResult;

pub fn openai_request_to_responses(model: &str, body: &Value, stream: bool) -> AppResult<Value> {
    let mut out = serde_json::Map::new();
    out.insert("model".to_string(), json!(model));
    out.insert("stream".to_string(), json!(stream));

    if let Some(instructions) = body.get("instructions").cloned().or_else(|| {
        body.get("messages")
            .and_then(Value::as_array)
            .map(|messages| system_messages_to_instructions(messages))
            .filter(|text| !text.trim().is_empty())
            .map(Value::String)
    }) {
        out.insert("instructions".to_string(), instructions);
    }

    let input = body
        .get("input")
        .cloned()
        .or_else(|| body.get("messages").map(openai_messages_to_responses_input))
        .unwrap_or_else(|| json!([]));
    out.insert("input".to_string(), input);

    copy_if_present(
        &mut out,
        body,
        &[
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "prompt_cache_key",
            "prompt_cache_retention",
            "reasoning",
            "metadata",
            "store",
        ],
    );

    if let Some(max_tokens) = body
        .get("max_output_tokens")
        .or_else(|| body.get("max_completion_tokens"))
        .or_else(|| body.get("max_tokens"))
    {
        out.insert("max_output_tokens".to_string(), max_tokens.clone());
    }

    Ok(Value::Object(out))
}

pub fn claude_request_to_responses(model: &str, body: &Value, stream: bool) -> AppResult<Value> {
    let mut out = serde_json::Map::new();
    out.insert("model".to_string(), json!(model));
    out.insert("stream".to_string(), json!(stream));

    if let Some(system) = body.get("system") {
        out.insert(
            "instructions".to_string(),
            claude_system_to_instructions(system),
        );
    }

    let input = body
        .get("messages")
        .map(claude_messages_to_responses_input)
        .unwrap_or_else(|| json!([]));
    out.insert("input".to_string(), input);

    copy_if_present(
        &mut out,
        body,
        &[
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "metadata",
            "prompt_cache_key",
            "prompt_cache_retention",
        ],
    );

    if let Some(max_tokens) = body.get("max_tokens") {
        out.insert("max_output_tokens".to_string(), max_tokens.clone());
    }

    Ok(Value::Object(out))
}

pub fn responses_response_to_openai(body: &Value) -> AppResult<Value> {
    if body.get("choices").is_some() {
        return Ok(body.clone());
    }

    let text = responses_output_text(body);
    let finish_reason = match body.get("status").and_then(Value::as_str) {
        Some("completed") | None => "stop",
        Some("incomplete") => "length",
        Some("failed") => "error",
        Some(_) => "stop",
    };

    let mut out = serde_json::Map::new();
    out.insert(
        "id".to_string(),
        body.get("id")
            .cloned()
            .unwrap_or_else(|| json!("chatcmpl_kou_router")),
    );
    out.insert("object".to_string(), json!("chat.completion"));
    if let Some(model) = body.get("model").cloned() {
        out.insert("model".to_string(), model);
    }
    if let Some(created) = body.get("created").or_else(|| body.get("created_at")) {
        out.insert("created".to_string(), created.clone());
    }
    out.insert(
        "choices".to_string(),
        json!([{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": finish_reason
        }]),
    );
    if let Some(usage) = body.get("usage") {
        out.insert("usage".to_string(), responses_usage_to_openai(usage));
    }
    out.insert("raw_openai_responses".to_string(), body.clone());

    Ok(Value::Object(out))
}

fn copy_if_present(out: &mut serde_json::Map<String, Value>, body: &Value, keys: &[&str]) {
    for key in keys {
        if let Some(value) = body.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
}

fn openai_messages_to_responses_input(messages: &Value) -> Value {
    let Some(messages) = messages.as_array() else {
        return messages.clone();
    };
    Value::Array(
        messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) != Some("system"))
            .map(|message| {
                json!({
                    "type": "message",
                    "role": message.get("role").and_then(Value::as_str).unwrap_or("user"),
                    "content": response_input_content(
                        message.get("content").unwrap_or(&Value::Null)
                    )
                })
            })
            .collect(),
    )
}

fn claude_messages_to_responses_input(messages: &Value) -> Value {
    let Some(messages) = messages.as_array() else {
        return messages.clone();
    };
    Value::Array(
        messages
            .iter()
            .map(|message| {
                json!({
                    "type": "message",
                    "role": message.get("role").and_then(Value::as_str).unwrap_or("user"),
                    "content": response_input_content(
                        message.get("content").unwrap_or(&Value::Null)
                    )
                })
            })
            .collect(),
    )
}

fn response_input_content(content: &Value) -> Value {
    match content {
        Value::String(text) => json!([{"type": "input_text", "text": text}]),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| match item.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("input_image") => item.clone(),
                    Some("text") => item
                        .get("text")
                        .and_then(Value::as_str)
                        .map(|text| json!({"type": "input_text", "text": text}))
                        .unwrap_or_else(|| item.clone()),
                    _ => item.clone(),
                })
                .collect(),
        ),
        Value::Null => Value::Array(Vec::new()),
        other => json!([{"type": "input_text", "text": other.to_string()}]),
    }
}

fn system_messages_to_instructions(messages: &[Value]) -> String {
    messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("system"))
        .filter_map(|message| message_text(message.get("content").unwrap_or(&Value::Null)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn claude_system_to_instructions(system: &Value) -> Value {
    match system {
        Value::String(_) => system.clone(),
        Value::Array(_) => Value::String(message_text(system).unwrap_or_default()),
        other => Value::String(other.to_string()),
    }
}

fn message_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn responses_output_text(body: &Value) -> String {
    body.get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|content| {
            content
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| content.get("delta").and_then(Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("")
}

fn responses_usage_to_openai(usage: &Value) -> Value {
    json!({
        "prompt_tokens": usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .cloned()
            .unwrap_or_else(|| json!(0)),
        "completion_tokens": usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .cloned()
            .unwrap_or_else(|| json!(0)),
        "total_tokens": usage
            .get("total_tokens")
            .cloned()
            .unwrap_or_else(|| json!(0)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_chat_request_becomes_responses_input() {
        let body = json!({
            "messages": [
                {"role": "system", "content": "Be exact."},
                {"role": "user", "content": "ping"}
            ],
            "max_tokens": 10,
            "prompt_cache_key": "shared-prefix",
            "prompt_cache_retention": "24h"
        });

        let result = openai_request_to_responses("gpt-5.5", &body, false).unwrap();

        assert_eq!(result["model"], "gpt-5.5");
        assert_eq!(result["instructions"], "Be exact.");
        assert_eq!(result["input"][0]["role"], "user");
        assert_eq!(result["input"][0]["content"][0]["text"], "ping");
        assert_eq!(result["max_output_tokens"], 10);
        assert_eq!(result["prompt_cache_key"], "shared-prefix");
        assert_eq!(result["prompt_cache_retention"], "24h");
    }

    #[test]
    fn claude_messages_request_becomes_responses_input() {
        let body = json!({
            "system": [{"type": "text", "text": "Be exact."}],
            "messages": [{"role": "user", "content": [{"type": "text", "text": "ping"}]}],
            "max_tokens": 10,
            "prompt_cache_key": "claude-prefix"
        });

        let result = claude_request_to_responses("gpt-5.5", &body, false).unwrap();

        assert_eq!(result["instructions"], "Be exact.");
        assert_eq!(result["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(result["input"][0]["content"][0]["text"], "ping");
        assert_eq!(result["max_output_tokens"], 10);
        assert_eq!(result["prompt_cache_key"], "claude-prefix");
    }

    #[test]
    fn responses_response_becomes_openai_chat() {
        let body = json!({
            "id": "resp_1",
            "model": "gpt-5.5",
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "pong"}]
            }],
            "usage": {"input_tokens": 3, "output_tokens": 4, "total_tokens": 7}
        });

        let result = responses_response_to_openai(&body).unwrap();

        assert_eq!(result["id"], "resp_1");
        assert_eq!(result["choices"][0]["message"]["content"], "pong");
        assert_eq!(result["usage"]["prompt_tokens"], 3);
        assert_eq!(result["usage"]["completion_tokens"], 4);
    }
}
