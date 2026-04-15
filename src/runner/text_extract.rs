//! Text extraction fallback for reasoning models.
//!
//! When reasoning models (gpt-5-mini, o4-mini) describe tool calls in text
//! instead of using the function calling API, these parsers extract JSON objects
//! and convert them into the standard OpenAI tool_call format.

use serde_json::Value;

/// Extract tool calls from LLM text when native function calling returns nothing.
///
/// Three strategies tried in order:
/// 1. JSON from markdown code blocks (```json ... ```)
/// 2. Inline JSON objects matching known tool names
/// 3. `{"tool_calls": [...]}` wrapper format
pub fn extract_tool_calls_from_text(text: &str, known_tools: &[String]) -> Vec<Value> {
    let mut calls = Vec::new();
    let mut call_counter = 0u32;

    // Strategy 1: Extract JSON from code blocks (```json ... ``` or ``` ... ```)
    for candidate in extract_code_block_json(text) {
        if let Some(extracted) = parse_tool_call_json(&candidate, known_tools) {
            for call in extracted {
                call_counter += 1;
                calls.push(wrap_as_tool_call(call_counter, &call));
            }
        }
    }

    // Strategy 2: Find top-level JSON objects in text that contain known tool names
    if calls.is_empty() {
        for json_str in find_json_objects(text) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&json_str) {
                if let Some(extracted) = parse_tool_call_json(&parsed, known_tools) {
                    for call in extracted {
                        call_counter += 1;
                        calls.push(wrap_as_tool_call(call_counter, &call));
                    }
                }
            }
        }
    }

    if !calls.is_empty() {
        tracing::info!(
            "Extracted {} tool call(s) from text (reasoning model fallback)",
            calls.len()
        );
    }

    calls
}

/// Extract JSON values from markdown code blocks in text.
fn extract_code_block_json(text: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let parts: Vec<&str> = text.split("```").collect();
    // Odd-indexed parts are inside code blocks
    for (i, part) in parts.iter().enumerate() {
        if i % 2 == 1 {
            let trimmed = part.trim_start_matches("json").trim();
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                results.push(parsed);
            }
        }
    }
    results
}

/// Find balanced JSON objects `{...}` in text using brace counting.
fn find_json_objects(text: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut depth = 1i32;
            let mut in_string = false;
            let mut escape = false;
            let start = i;
            i += 1;
            while i < bytes.len() && depth > 0 {
                if escape {
                    escape = false;
                } else if bytes[i] == b'\\' && in_string {
                    escape = true;
                } else if bytes[i] == b'"' {
                    in_string = !in_string;
                } else if !in_string {
                    if bytes[i] == b'{' {
                        depth += 1;
                    } else if bytes[i] == b'}' {
                        depth -= 1;
                    }
                }
                i += 1;
            }
            if depth == 0 {
                objects.push(text[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    objects
}

/// Try to interpret a JSON value as one or more tool calls.
///
/// Accepts formats:
/// - `{"name": "tool", "arguments": {...}}` — single call
/// - `[{"name": "tool", "arguments": {...}}, ...]` — array of calls
/// - `{"tool_calls": [...]}` — wrapped array
fn parse_tool_call_json(value: &Value, known_tools: &[String]) -> Option<Vec<(String, Value)>> {
    let mut calls = Vec::new();

    match value {
        Value::Array(arr) => {
            for item in arr {
                if let Some(pair) = extract_single_call(item, known_tools) {
                    calls.push(pair);
                }
            }
        }
        Value::Object(obj) => {
            // Check for {"tool_calls": [...]} wrapper
            if let Some(inner) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                for item in inner {
                    if let Some(pair) = extract_single_call(item, known_tools) {
                        calls.push(pair);
                    }
                }
            } else if let Some(pair) = extract_single_call(value, known_tools) {
                calls.push(pair);
            }
        }
        _ => {}
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

/// Extract a single (name, arguments) pair from a JSON object.
fn extract_single_call(value: &Value, known_tools: &[String]) -> Option<(String, Value)> {
    let obj = value.as_object()?;

    // Format 1: {"name": "tool", "arguments": {...}}
    if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
        if known_tools.iter().any(|t| t == name) {
            let args = obj
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            return Some((name.to_string(), args));
        }
    }

    // Format 2: OpenAI nested {"function": {"name": "tool", "arguments": "..."}}
    if let Some(func) = obj.get("function").and_then(|v| v.as_object()) {
        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
            if known_tools.iter().any(|t| t == name) {
                let args_raw = func
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let args = if let Some(s) = args_raw.as_str() {
                    serde_json::from_str(s).unwrap_or(serde_json::json!({}))
                } else {
                    args_raw
                };
                return Some((name.to_string(), args));
            }
        }
    }

    None
}

/// Wrap extracted (name, arguments) into OpenAI tool_call format.
fn wrap_as_tool_call(counter: u32, call: &(String, Value)) -> Value {
    let (name, arguments) = call;
    let args_str = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string());
    serde_json::json!({
        "id": format!("text-extracted-{counter}"),
        "type": "function",
        "function": {
            "name": name,
            "arguments": args_str,
        }
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<String> {
        vec![
            "send_message".into(),
            "check_inbox".into(),
            "memory_search".into(),
        ]
    }

    #[test]
    fn test_extract_from_json_code_block() {
        let text = r#"I'll call send_message now.

```json
{"name": "send_message", "arguments": {"recipient": "03ef3231", "body": {"result": "42"}}}
```"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert_eq!(calls.len(), 1);
        let func = calls[0].get("function").unwrap();
        assert_eq!(func.get("name").unwrap().as_str().unwrap(), "send_message");
    }

    #[test]
    fn test_extract_from_json_array_code_block() {
        let text = r#"Calling two tools:

```json
[
  {"name": "send_message", "arguments": {"recipient": "abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123abc123ab", "body": {"text": "hi"}}},
  {"name": "check_inbox", "arguments": {}}
]
```"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_extract_from_inline_json() {
        let text = r#"I need to search memory: {"name": "memory_search", "arguments": {"query": "hello"}}"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert_eq!(calls.len(), 1);
        let func = calls[0].get("function").unwrap();
        assert_eq!(func.get("name").unwrap().as_str().unwrap(), "memory_search");
    }

    #[test]
    fn test_extract_from_tool_calls_wrapper() {
        let text = r#"```json
{"tool_calls": [{"name": "send_message", "arguments": {"recipient": "key", "body": {}}}]}
```"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn test_extract_openai_nested_format() {
        let text = r#"```json
{"function": {"name": "send_message", "arguments": "{\"recipient\": \"key\"}"}}
```"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn test_extract_ignores_unknown_tools() {
        let text = r#"{"name": "unknown_tool", "arguments": {}}"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_no_json_in_text() {
        let text = "I'll call send_message for each inbox item now.\n(1/9)\n- recipient: 03ef";
        let calls = extract_tool_calls_from_text(text, &tools());
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_empty_text() {
        let calls = extract_tool_calls_from_text("", &tools());
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extracted_call_has_correct_format() {
        let text =
            r#"{"name": "send_message", "arguments": {"recipient": "abc", "body": {"x": 1}}}"#;
        let calls = extract_tool_calls_from_text(text, &tools());
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(
            call.get("id").unwrap().as_str().unwrap(),
            "text-extracted-1"
        );
        assert_eq!(call.get("type").unwrap().as_str().unwrap(), "function");
        let func = call.get("function").unwrap();
        assert_eq!(func.get("name").unwrap().as_str().unwrap(), "send_message");
        // arguments should be a JSON string (OpenAI format)
        assert!(func.get("arguments").unwrap().as_str().is_some());
    }

    // -- find_json_objects --

    #[test]
    fn test_find_json_objects_simple() {
        let objects = find_json_objects(r#"text {"a": 1} more {"b": 2}"#);
        assert_eq!(objects.len(), 2);
    }

    #[test]
    fn test_find_json_objects_nested() {
        let objects = find_json_objects(r#"{"a": {"b": {"c": 1}}}"#);
        assert_eq!(objects.len(), 1);
        assert!(objects[0].contains("\"c\": 1"));
    }

    #[test]
    fn test_find_json_objects_with_strings() {
        let objects = find_json_objects(r#"{"key": "value with { braces }"}"#);
        assert_eq!(objects.len(), 1);
    }

    #[test]
    fn test_find_json_objects_none() {
        let objects = find_json_objects("no json here at all");
        assert!(objects.is_empty());
    }
}
