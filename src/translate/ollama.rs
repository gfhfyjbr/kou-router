use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppResult;

/// Translate an OpenAI chat/completions request into Ollama's /api/chat format.
pub fn translate_request_to_ollama(model: &str, body: &Value, stream: bool) -> AppResult<Value> {
    let mut options = json!({});

    if let Some(temp) = body.get("temperature") {
        options["temperature"] = temp.clone();
    }
    if let Some(top_p) = body.get("top_p") {
        options["top_p"] = top_p.clone();
    }
    if let Some(max_tokens) = body.get("max_tokens") {
        options["num_predict"] = max_tokens.clone();
    }
    if let Some(stop) = body.get("stop") {
        options["stop"] = stop.clone();
    }

    let mut request = json!({
        "model": model,
        "stream": stream,
    });

    if let Some(messages) = body.get("messages") {
        request["messages"] = messages.clone();
    }

    if let Some(tools) = body.get("tools") {
        request["tools"] = tools.clone();
    }

    // Only include options if any were set.
    if options.as_object().map_or(false, |o| !o.is_empty()) {
        request["options"] = options;
    }

    Ok(request)
}

/// Translate an Ollama /api/chat response into OpenAI chat/completions format.
pub fn translate_response_to_openai(body: &Value) -> AppResult<Value> {
    let message = body.get("message").cloned().unwrap_or_else(|| json!({}));

    let content = message
        .get("content")
        .cloned()
        .unwrap_or(Value::String(String::new()));

    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("assistant");

    let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
    let finish_reason = if done { json!("stop") } else { Value::Null };

    let mut oai_message = json!({
        "role": role,
        "content": content,
    });

    if let Some(tool_calls) = message.get("tool_calls") {
        oai_message["tool_calls"] = tool_calls.clone();
    }

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let prompt_tokens = body
        .get("prompt_eval_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = body
        .get("eval_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let id = format!("chatcmpl-ollama-{}", Uuid::new_v4());

    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": oai_message,
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
    fn test_request_options_mapping() {
        let body = json!({
            "temperature": 0.7,
            "top_p": 0.9,
            "max_tokens": 128,
            "stop": ["\n"],
            "messages": []
        });
        let result = translate_request_to_ollama("llama3", &body, false).unwrap();
        let opts = &result["options"];
        assert_eq!(opts["temperature"], json!(0.7));
        assert_eq!(opts["top_p"], json!(0.9));
        assert_eq!(opts["num_predict"], json!(128));
        assert_eq!(opts["stop"], json!(["\n"]));
    }

    #[test]
    fn test_request_tools_passthrough() {
        let tools = json!([{"type": "function", "function": {"name": "get_weather"}}]);
        let body = json!({ "tools": tools, "messages": [] });
        let result = translate_request_to_ollama("llama3", &body, false).unwrap();
        assert_eq!(result["tools"], tools);
    }

    #[test]
    fn test_request_messages_passthrough() {
        let msgs = json!([{"role": "user", "content": "hello"}]);
        let body = json!({ "messages": msgs });
        let result = translate_request_to_ollama("llama3", &body, false).unwrap();
        assert_eq!(result["messages"], msgs);
    }

    #[test]
    fn test_request_model_and_stream() {
        let body = json!({ "messages": [] });
        let result = translate_request_to_ollama("mistral", &body, true).unwrap();
        assert_eq!(result["model"], json!("mistral"));
        assert_eq!(result["stream"], json!(true));

        let result2 = translate_request_to_ollama("llama3", &body, false).unwrap();
        assert_eq!(result2["model"], json!("llama3"));
        assert_eq!(result2["stream"], json!(false));
    }

    #[test]
    fn test_response_basic() {
        let body = json!({
            "message": { "role": "assistant", "content": "hi" },
            "done": true
        });
        let result = translate_response_to_openai(&body).unwrap();
        let msg = &result["choices"][0]["message"];
        assert_eq!(msg["content"], json!("hi"));
        assert_eq!(msg["role"], json!("assistant"));
    }

    #[test]
    fn test_response_done_flag() {
        let done_body = json!({
            "message": { "role": "assistant", "content": "" },
            "done": true
        });
        let result = translate_response_to_openai(&done_body).unwrap();
        assert_eq!(result["choices"][0]["finish_reason"], json!("stop"));

        let not_done_body = json!({
            "message": { "role": "assistant", "content": "" },
            "done": false
        });
        let result2 = translate_response_to_openai(&not_done_body).unwrap();
        assert!(result2["choices"][0]["finish_reason"].is_null());
    }

    #[test]
    fn test_response_usage() {
        let body = json!({
            "message": { "role": "assistant", "content": "" },
            "done": true,
            "prompt_eval_count": 10,
            "eval_count": 20
        });
        let result = translate_response_to_openai(&body).unwrap();
        let usage = &result["usage"];
        assert_eq!(usage["prompt_tokens"], json!(10));
        assert_eq!(usage["completion_tokens"], json!(20));
        assert_eq!(usage["total_tokens"], json!(30));
    }

    #[test]
    fn test_request_no_options_if_empty() {
        let body = json!({ "messages": [] });
        let result = translate_request_to_ollama("llama3", &body, false).unwrap();
        assert!(result.get("options").is_none(), "options key should be absent when no option fields set");
    }

}