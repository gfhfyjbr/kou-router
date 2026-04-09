use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

#[allow(unused_imports)]
use super::common::*;

/// Convert an OpenAI chat/completions request to Gemini generateContent format.
///
/// Model is not included in the body — Gemini routes by URL.
/// Stream flag is also URL-level; neither appears in the output.
pub fn translate_request(_model: &str, body: &Value, _stream: bool) -> AppResult<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::BadRequest("missing messages array".into()))?;

    let mut system_parts: Vec<Value> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
        match role {
            "system" => {
                let text = extract_text_content(msg);
                if !text.is_empty() {
                    system_parts.push(json!({ "text": text }));
                }
            }
            "user" => {
                let parts = openai_content_to_gemini_parts(&msg["content"]);
                contents.push(json!({ "role": "user", "parts": parts }));
            }
            "assistant" => {
                let mut parts = Vec::new();

                let text = extract_text_content(msg);
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }

                // Each OpenAI tool_call becomes a Gemini functionCall part
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for call in tool_calls {
                        if let Some(part) = tool_call_to_function_call(call) {
                            parts.push(part);
                        }
                    }
                }

                if !parts.is_empty() {
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
            }
            "tool" => {
                let part = tool_msg_to_function_response(msg);

                // Gemini requires consecutive function responses in a single message.
                let merge = contents
                    .last()
                    .and_then(|c| c.get("role")?.as_str())
                    == Some("function");

                if merge {
                    // Safe: we just confirmed `last()` exists and has "parts".
                    if let Some(parts) = contents
                        .last_mut()
                        .and_then(|c| c.get_mut("parts"))
                        .and_then(Value::as_array_mut)
                    {
                        parts.push(part);
                    }
                } else {
                    contents.push(json!({ "role": "function", "parts": [part] }));
                }
            }
            _ => {} // ignore unknown roles
        }
    }

    let mut result = json!({ "contents": contents });

    // Concatenated system messages
    if !system_parts.is_empty() {
        result["systemInstruction"] = json!({ "parts": system_parts });
    }

    // Tools: OpenAI wraps each in {type: "function", function: {...}};
    // Gemini wants a single object with a functionDeclarations array.
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let decls: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let func = t.get("function")?;
                Some(json!({
                    "name": func.get("name").cloned().unwrap_or(json!("")),
                    "description": func.get("description").cloned().unwrap_or(json!("")),
                    "parameters": func.get("parameters").cloned()
                        .unwrap_or(json!({ "type": "object" }))
                }))
            })
            .collect();

        if !decls.is_empty() {
            result["tools"] = json!([{ "functionDeclarations": decls }]);
        }
    }

    // tool_choice -> toolConfig
    if let Some(tc) = body.get("tool_choice") {
        if let Some(config) = translate_tool_choice(tc) {
            result["toolConfig"] = config;
        }
    }

    // generationConfig
    let gen_config = build_generation_config(body);
    if !gen_config.as_object().map_or(true, |m| m.is_empty()) {
        result["generationConfig"] = gen_config;
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the plain-text portion of an OpenAI message's `content` field.
/// Handles both the string form and the array-of-parts form.
fn extract_text_content(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                if p.get("type")?.as_str()? == "text" {
                    p.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Convert OpenAI content (string or multimodal array) to Gemini `parts`.
fn openai_content_to_gemini_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => vec![json!({ "text": text })],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                match item.get("type")?.as_str()? {
                    "text" => {
                        let t = item.get("text").and_then(Value::as_str).unwrap_or("");
                        Some(json!({ "text": t }))
                    }
                    "image_url" => {
                        let url = item
                            .get("image_url")
                            .and_then(|v| v.get("url"))
                            .and_then(Value::as_str)?;
                        image_url_to_gemini_part(url)
                    }
                    _ => None,
                }
            })
            .collect(),
        Value::Null => vec![json!({ "text": "" })],
        _ => vec![json!({ "text": content.to_string() })],
    }
}

/// Convert a data-URI or plain URL into the appropriate Gemini part.
fn image_url_to_gemini_part(url: &str) -> Option<Value> {
    if url.starts_with("data:") {
        let (meta, data) = url.split_once(',')?;
        let mime = meta
            .strip_prefix("data:")
            .and_then(|s| s.split(';').next())
            .unwrap_or("image/png");
        Some(json!({ "inlineData": { "mimeType": mime, "data": data } }))
    } else {
        // Gemini fileData — works with GCS URIs; for public URLs the
        // caller / proxy layer may need to fetch-and-inline, but we pass
        // through as-is so the upstream can decide.
        Some(json!({ "fileData": { "mimeType": "image/png", "fileUri": url } }))
    }
}

/// Convert a single OpenAI tool_call object to a Gemini functionCall part.
fn tool_call_to_function_call(call: &Value) -> Option<Value> {
    let func = call.get("function")?;
    let name = func.get("name")?.as_str()?;
    let args_str = func.get("arguments").and_then(Value::as_str).unwrap_or("{}");
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
    Some(json!({ "functionCall": { "name": name, "args": args } }))
}

/// Convert an OpenAI tool-role message to a Gemini functionResponse part.
fn tool_msg_to_function_response(msg: &Value) -> Value {
    let name = msg
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let raw = msg.get("content").and_then(Value::as_str).unwrap_or("{}");
    // Gemini expects a JSON object as the response body.
    // If the content isn't valid JSON, wrap the raw string so it's always an object.
    let response: Value =
        serde_json::from_str(raw).unwrap_or_else(|_| json!({ "result": raw }));
    json!({ "functionResponse": { "name": name, "response": response } })
}

/// Map OpenAI `tool_choice` to Gemini `toolConfig`.
fn translate_tool_choice(tc: &Value) -> Option<Value> {
    match tc {
        Value::String(s) => {
            let mode = match s.as_str() {
                "auto" => "AUTO",
                "none" => "NONE",
                "required" => "ANY",
                _ => return None,
            };
            Some(json!({ "functionCallingConfig": { "mode": mode } }))
        }
        Value::Object(obj) => {
            // {type: "function", function: {name: "..."}} -> ANY with allowedFunctionNames
            let name = obj.get("function")?.get("name")?.as_str()?;
            Some(json!({
                "functionCallingConfig": {
                    "mode": "ANY",
                    "allowedFunctionNames": [name]
                }
            }))
        }
        _ => None,
    }
}

/// Collect generation-config fields from the OpenAI body.
fn build_generation_config(body: &Value) -> Value {
    let mut cfg = json!({});

    if let Some(v) = body.get("max_tokens") {
        cfg["maxOutputTokens"] = v.clone();
    }
    // Some newer clients send max_completion_tokens instead.
    if cfg.get("maxOutputTokens").is_none() {
        if let Some(v) = body.get("max_completion_tokens") {
            cfg["maxOutputTokens"] = v.clone();
        }
    }
    if let Some(v) = body.get("temperature") {
        cfg["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        cfg["topP"] = v.clone();
    }
    if let Some(stop) = body.get("stop") {
        match stop {
            Value::String(s) => cfg["stopSequences"] = json!([s]),
            Value::Array(_) => cfg["stopSequences"] = stop.clone(),
            _ => {}
        }
    }

    cfg
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_messages() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "Hello" }
            ]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let contents = result["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "Hello");
    }

    #[test]
    fn test_system_instruction() {
        let body = json!({
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hi" }
            ]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let sys = &result["systemInstruction"]["parts"];
        let parts = sys.as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "You are helpful.");
        // system message must NOT appear in contents
        let contents = result["contents"].as_array().unwrap();
        assert!(contents.iter().all(|c| c["role"] != "system"));
    }

    #[test]
    fn test_assistant_model_role() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "Hi" },
                { "role": "assistant", "content": "Hello!" }
            ]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let contents = result["contents"].as_array().unwrap();
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "Hello!");
    }

    #[test]
    fn test_tools_to_function_declarations() {
        let body = json!({
            "messages": [{ "role": "user", "content": "weather" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
                }
            }]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0]["name"], "get_weather");
        assert_eq!(decls[0]["description"], "Get weather");
        assert_eq!(decls[0]["parameters"]["type"], "object");
    }

    #[test]
    fn test_tool_choice_auto() {
        let body = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "tool_choice": "auto"
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        assert_eq!(
            result["toolConfig"]["functionCallingConfig"]["mode"],
            "AUTO"
        );
    }

    #[test]
    fn test_tool_choice_none() {
        let body = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "tool_choice": "none"
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        assert_eq!(
            result["toolConfig"]["functionCallingConfig"]["mode"],
            "NONE"
        );
    }

    #[test]
    fn test_tool_choice_required() {
        let body = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "tool_choice": "required"
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        assert_eq!(
            result["toolConfig"]["functionCallingConfig"]["mode"],
            "ANY"
        );
    }

    #[test]
    fn test_tool_choice_object_with_function_name() {
        let body = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "tool_choice": {
                "type": "function",
                "function": { "name": "get_weather" }
            }
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let config = &result["toolConfig"]["functionCallingConfig"];
        assert_eq!(config["mode"], "ANY");
        let allowed = config["allowedFunctionNames"].as_array().unwrap();
        assert_eq!(allowed, &[json!("get_weather")]);
    }

    #[test]
    fn test_generation_config() {
        let body = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "temperature": 0.7,
            "max_tokens": 256,
            "top_p": 0.9,
            "stop": ["END", "STOP"]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let gc = &result["generationConfig"];
        assert_eq!(gc["temperature"], 0.7);
        assert_eq!(gc["maxOutputTokens"], 256);
        assert_eq!(gc["topP"], 0.9);
        assert_eq!(gc["stopSequences"], json!(["END", "STOP"]));
    }

    #[test]
    fn test_max_completion_tokens_fallback() {
        let body = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "max_completion_tokens": 512
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        assert_eq!(result["generationConfig"]["maxOutputTokens"], 512);
    }

    #[test]
    fn test_image_data_uri() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,abc123" } }
                ]
            }]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let part = &result["contents"][0]["parts"][0];
        assert_eq!(part["inlineData"]["mimeType"], "image/png");
        assert_eq!(part["inlineData"]["data"], "abc123");
    }

    #[test]
    fn test_image_url() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image_url", "image_url": { "url": "https://example.com/img.png" } }
                ]
            }]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let part = &result["contents"][0]["parts"][0];
        assert_eq!(part["fileData"]["mimeType"], "image/png");
        assert_eq!(part["fileData"]["fileUri"], "https://example.com/img.png");
    }

    #[test]
    fn test_tool_call_to_function_call() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "weather" },
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Tokyo\"}"
                        }
                    }]
                }
            ]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let contents = result["contents"].as_array().unwrap();
        // assistant → model
        assert_eq!(contents[1]["role"], "model");
        let fc = &contents[1]["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "get_weather");
        assert_eq!(fc["args"]["city"], "Tokyo");
    }

    #[test]
    fn test_tool_response_merge() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "multi-tool" },
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        { "id": "c1", "type": "function", "function": { "name": "a", "arguments": "{}" } },
                        { "id": "c2", "type": "function", "function": { "name": "b", "arguments": "{}" } }
                    ]
                },
                { "role": "tool", "name": "a", "content": "{\"r\":1}" },
                { "role": "tool", "name": "b", "content": "{\"r\":2}" }
            ]
        });
        let result = translate_request("gemini-pro", &body, false).unwrap();
        let contents = result["contents"].as_array().unwrap();
        // user, model, function (merged)
        assert_eq!(contents.len(), 3);
        let func_msg = &contents[2];
        assert_eq!(func_msg["role"], "function");
        let parts = func_msg["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["name"], "a");
        assert_eq!(parts[1]["functionResponse"]["name"], "b");
    }
}