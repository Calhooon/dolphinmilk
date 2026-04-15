//! Tests for think module — ThinkResult, model detection, provider resolution,
//! and Claude/OpenAI format conversion.

use dolphin_milk::config::DmConfig;
use dolphin_milk::think::{
    convert_messages_for_claude, convert_tools_for_claude, is_claude_model, is_reasoning_model,
    resolve_endpoint, ThinkResult, CLAUDE_AGENT_URL, DEFAULT_MAX_TOKENS, DEFAULT_MODEL,
    OPENAI_AGENT_URL,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// ThinkResult basics (existing)
// ---------------------------------------------------------------------------

#[test]
fn test_think_result_defaults() {
    let r = ThinkResult {
        text: "hello".to_string(),
        model: "gpt-5-mini".to_string(),
        sats_paid: 0,
        sats_effective: 0,
        sats_refunded: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        finish_reason: String::new(),
        duration_ms: 0,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    assert_eq!(r.text, "hello");
    assert_eq!(r.model, "gpt-5-mini");
    assert_eq!(r.sats_paid, 0);
    assert_eq!(r.sats_effective, 0);
    assert_eq!(r.sats_refunded, 0);
    assert_eq!(r.prompt_tokens, 0);
    assert_eq!(r.completion_tokens, 0);
    assert_eq!(r.total_tokens, 0);
    assert!(r.finish_reason.is_empty());
    assert_eq!(r.duration_ms, 0);
}

#[test]
fn test_think_result_full() {
    let r = ThinkResult {
        text: "response".to_string(),
        model: "gpt-5.2".to_string(),
        sats_paid: 5000,
        sats_effective: 4800,
        sats_refunded: 200,
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        finish_reason: "stop".to_string(),
        duration_ms: 1234,
        tool_calls: vec![],
        payment_txid: Some("abc123def456".to_string()),
        refund_internalized: None,
        was_text_extracted: false,
    };
    assert_eq!(r.sats_paid, 5000);
    assert_eq!(r.sats_refunded, 200);
    assert_eq!(r.total_tokens, 150);
    assert_eq!(r.duration_ms, 1234);
}

#[test]
fn test_think_result_serialization() {
    let r = ThinkResult {
        text: "hello".to_string(),
        model: "gpt-5-mini".to_string(),
        sats_paid: 1000,
        sats_effective: 900,
        sats_refunded: 100,
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: 30,
        finish_reason: "stop".to_string(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: Some("txid123".to_string()),
        refund_internalized: None,
        was_text_extracted: false,
    };
    let json_str = serde_json::to_string(&r).unwrap();
    let deserialized: ThinkResult = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.text, "hello");
    assert_eq!(deserialized.sats_paid, 1000);
    assert_eq!(deserialized.sats_refunded, 100);
}

#[test]
fn test_think_result_serialization_skips_empty() {
    let r = ThinkResult {
        text: "hi".to_string(),
        model: "m".to_string(),
        sats_paid: 0,
        sats_effective: 0,
        sats_refunded: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        finish_reason: String::new(),
        duration_ms: 0,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    let json_str = serde_json::to_string(&r).unwrap();
    // Empty tool_calls and None payment_txid should be omitted
    assert!(!json_str.contains("tool_calls"));
    assert!(!json_str.contains("payment_txid"));
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn test_constants() {
    assert!(OPENAI_AGENT_URL.starts_with("https://"));
    assert!(OPENAI_AGENT_URL.contains("x402"));
    assert!(CLAUDE_AGENT_URL.starts_with("https://"));
    assert!(CLAUDE_AGENT_URL.contains("x402"));
    assert!(CLAUDE_AGENT_URL.contains("claude"));
    assert_ne!(OPENAI_AGENT_URL, CLAUDE_AGENT_URL);
    assert_eq!(DEFAULT_MODEL, "gpt-5-mini");
    assert_eq!(DEFAULT_MAX_TOKENS, 4096);
}

// ---------------------------------------------------------------------------
// is_reasoning_model tests (existing)
// ---------------------------------------------------------------------------

#[test]
fn test_reasoning_model_o_series() {
    assert!(is_reasoning_model("o1"));
    assert!(is_reasoning_model("o1-mini"));
    assert!(is_reasoning_model("o1-preview"));
    assert!(is_reasoning_model("o3"));
    assert!(is_reasoning_model("o3-mini"));
    assert!(is_reasoning_model("o4-mini"));
}

#[test]
fn test_reasoning_model_gpt5() {
    assert!(is_reasoning_model("gpt-5-mini"));
    assert!(is_reasoning_model("gpt-5-mini-2025-08-07"));
    assert!(is_reasoning_model("gpt-5.2"));
    assert!(is_reasoning_model("gpt-5.2-pro"));
    assert!(is_reasoning_model("gpt-5-nano"));
}

#[test]
fn test_reasoning_model_gpt41() {
    assert!(is_reasoning_model("gpt-4.1"));
    assert!(is_reasoning_model("gpt-4.1-mini"));
    assert!(is_reasoning_model("gpt-4.1-nano"));
}

#[test]
fn test_non_reasoning_models() {
    assert!(!is_reasoning_model("gpt-4o"));
    assert!(!is_reasoning_model("gpt-4o-mini"));
    assert!(!is_reasoning_model("gpt-3.5-turbo"));
    assert!(!is_reasoning_model("claude-sonnet-4-6"));
    assert!(!is_reasoning_model("claude-haiku-4-5"));
    assert!(!is_reasoning_model("llama-3"));
}

// ---------------------------------------------------------------------------
// is_claude_model tests (new)
// ---------------------------------------------------------------------------

#[test]
fn test_is_claude_model_positive() {
    assert!(is_claude_model("claude-sonnet-4-6"));
    assert!(is_claude_model("claude-opus-4-6"));
    assert!(is_claude_model("claude-haiku-4-5"));
    assert!(is_claude_model("claude-3-5-sonnet"));
    assert!(is_claude_model("claude-3-opus"));
    assert!(is_claude_model("claude-instant"));
}

#[test]
fn test_is_claude_model_negative() {
    assert!(!is_claude_model("gpt-5-mini"));
    assert!(!is_claude_model("gpt-4o"));
    assert!(!is_claude_model("o4-mini"));
    assert!(!is_claude_model("llama-3"));
    assert!(!is_claude_model(""));
    // "Claude" (capitalized) — strict prefix match
    assert!(!is_claude_model("Claude-3"));
}

// ---------------------------------------------------------------------------
// resolve_endpoint tests (new)
// ---------------------------------------------------------------------------

#[test]
fn test_resolve_endpoint_explicit_claude_provider() {
    let mut config = DmConfig::default();
    config.llm.default_provider = "claude-chat".into();
    assert_eq!(resolve_endpoint(&config, "gpt-5-mini"), CLAUDE_AGENT_URL);
}

#[test]
fn test_resolve_endpoint_explicit_claude_alias() {
    let mut config = DmConfig::default();
    config.llm.default_provider = "claude".into();
    assert_eq!(resolve_endpoint(&config, "gpt-5-mini"), CLAUDE_AGENT_URL);
}

#[test]
fn test_resolve_endpoint_claude_model_overrides_openai_provider() {
    // Claude models always route to Claude endpoint, even if provider is explicitly OpenAI
    let mut config = DmConfig::default();
    config.llm.default_provider = "openai-agent".into();
    assert_eq!(
        resolve_endpoint(&config, "claude-sonnet-4-6"),
        CLAUDE_AGENT_URL
    );
}

#[test]
fn test_resolve_endpoint_claude_model_overrides_openai_alias() {
    // Claude models always route to Claude endpoint, even if provider is explicitly OpenAI
    let mut config = DmConfig::default();
    config.llm.default_provider = "openai".into();
    assert_eq!(
        resolve_endpoint(&config, "claude-sonnet-4-6"),
        CLAUDE_AGENT_URL
    );
}

#[test]
fn test_resolve_endpoint_openai_model_honors_openai_provider() {
    let mut config = DmConfig::default();
    config.llm.default_provider = "openai-agent".into();
    assert_eq!(resolve_endpoint(&config, "gpt-5-mini"), OPENAI_AGENT_URL);
}

#[test]
fn test_resolve_endpoint_auto_detect_claude() {
    let mut config = DmConfig::default();
    config.llm.default_provider = "auto".into();
    assert_eq!(
        resolve_endpoint(&config, "claude-sonnet-4-6"),
        CLAUDE_AGENT_URL
    );
    assert_eq!(
        resolve_endpoint(&config, "claude-haiku-4-5"),
        CLAUDE_AGENT_URL
    );
    assert_eq!(
        resolve_endpoint(&config, "claude-opus-4-6"),
        CLAUDE_AGENT_URL
    );
}

#[test]
fn test_resolve_endpoint_auto_detect_openai() {
    let mut config = DmConfig::default();
    config.llm.default_provider = "unknown-provider".into();
    assert_eq!(resolve_endpoint(&config, "gpt-5-mini"), OPENAI_AGENT_URL);
    assert_eq!(resolve_endpoint(&config, "o4-mini"), OPENAI_AGENT_URL);
}

#[test]
fn test_resolve_endpoint_default_config() {
    let config = DmConfig::default();
    // Default provider is "openai-agent"
    assert_eq!(resolve_endpoint(&config, "gpt-5-mini"), OPENAI_AGENT_URL);
}

// ---------------------------------------------------------------------------
// convert_messages_for_claude tests (new)
// ---------------------------------------------------------------------------

#[test]
fn test_convert_messages_basic_user_assistant() {
    let messages = vec![
        json!({"role": "user", "content": "Hello"}),
        json!({"role": "assistant", "content": "Hi there!"}),
    ];

    let (system, claude_msgs) = convert_messages_for_claude(&messages);
    assert!(system.is_none());
    assert_eq!(claude_msgs.len(), 2);

    assert_eq!(claude_msgs[0]["role"], "user");
    assert_eq!(claude_msgs[0]["content"], "Hello");

    assert_eq!(claude_msgs[1]["role"], "assistant");
    let content = claude_msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Hi there!");
}

#[test]
fn test_convert_messages_extracts_system() {
    let messages = vec![
        json!({"role": "system", "content": "You are a helpful assistant."}),
        json!({"role": "user", "content": "Hello"}),
    ];

    let (system, claude_msgs) = convert_messages_for_claude(&messages);
    assert_eq!(system.unwrap(), "You are a helpful assistant.");
    assert_eq!(claude_msgs.len(), 1);
    assert_eq!(claude_msgs[0]["role"], "user");
}

#[test]
fn test_convert_messages_multiple_system() {
    let messages = vec![
        json!({"role": "system", "content": "Rule 1: Be helpful."}),
        json!({"role": "system", "content": "Rule 2: Be concise."}),
        json!({"role": "user", "content": "Hello"}),
    ];

    let (system, claude_msgs) = convert_messages_for_claude(&messages);
    let sys = system.unwrap();
    assert!(sys.contains("Rule 1: Be helpful."));
    assert!(sys.contains("Rule 2: Be concise."));
    assert_eq!(claude_msgs.len(), 1);
}

#[test]
fn test_convert_messages_tool_calls_on_assistant() {
    let messages = vec![
        json!({"role": "user", "content": "What's the weather?"}),
        json!({
            "role": "assistant",
            "content": "Let me check.",
            "tool_calls": [{
                "id": "call_abc123",
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "arguments": "{\"city\": \"NYC\"}"
                }
            }]
        }),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    assert_eq!(claude_msgs.len(), 2);

    // Assistant message should have text + tool_use blocks
    let content = claude_msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Let me check.");
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["id"], "call_abc123");
    assert_eq!(content[1]["name"], "get_weather");
    assert_eq!(content[1]["input"]["city"], "NYC");
}

#[test]
fn test_convert_messages_tool_results() {
    let messages = vec![
        json!({"role": "user", "content": "Check weather"}),
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "arguments": "{}"
                }
            }]
        }),
        json!({"role": "tool", "tool_call_id": "call_1", "content": "72°F sunny"}),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    assert_eq!(claude_msgs.len(), 3);

    // Third message should be a user message with tool_result content
    assert_eq!(claude_msgs[2]["role"], "user");
    let content = claude_msgs[2]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "tool_result");
    assert_eq!(content[0]["tool_use_id"], "call_1");
    assert_eq!(content[0]["content"], "72°F sunny");
}

#[test]
fn test_convert_messages_multiple_tool_results_grouped() {
    let messages = vec![
        json!({"role": "user", "content": "Do two things"}),
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "tool_a", "arguments": "{}"}},
                {"id": "call_2", "type": "function", "function": {"name": "tool_b", "arguments": "{}"}}
            ]
        }),
        json!({"role": "tool", "tool_call_id": "call_1", "content": "result_a"}),
        json!({"role": "tool", "tool_call_id": "call_2", "content": "result_b"}),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    // user, assistant, user (grouped tool results)
    assert_eq!(claude_msgs.len(), 3);

    let tool_result_msg = &claude_msgs[2];
    assert_eq!(tool_result_msg["role"], "user");
    let content = tool_result_msg["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "tool_result");
    assert_eq!(content[0]["tool_use_id"], "call_1");
    assert_eq!(content[1]["type"], "tool_result");
    assert_eq!(content[1]["tool_use_id"], "call_2");
}

#[test]
fn test_convert_messages_tool_results_then_user() {
    // Tool results followed by a user message should NOT be grouped
    let messages = vec![
        json!({"role": "user", "content": "check"}),
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "t", "arguments": "{}"}}]
        }),
        json!({"role": "tool", "tool_call_id": "c1", "content": "done"}),
        json!({"role": "user", "content": "now do something else"}),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    assert_eq!(claude_msgs.len(), 4);
    // Tool result flushed before user message
    assert_eq!(claude_msgs[2]["role"], "user");
    assert!(claude_msgs[2]["content"].is_array()); // tool_result block
    assert_eq!(claude_msgs[3]["role"], "user");
    assert_eq!(claude_msgs[3]["content"], "now do something else"); // plain user
}

#[test]
fn test_convert_messages_empty_assistant() {
    let messages = vec![
        json!({"role": "user", "content": "hi"}),
        json!({"role": "assistant", "content": ""}),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    // Empty assistant should still have a text block (Claude requires non-empty content)
    let content = claude_msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    // Must be non-empty — Claude rejects empty text content blocks
    let text = content[0]["text"].as_str().unwrap();
    assert!(!text.is_empty(), "text content block must be non-empty");
}

#[test]
fn test_convert_messages_empty_user_content() {
    let messages = vec![json!({"role": "user", "content": ""})];
    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    // Empty user content should be replaced with placeholder
    let content = claude_msgs[0]["content"].as_str().unwrap();
    assert!(
        !content.is_empty(),
        "user content must be non-empty for Claude API"
    );
}

#[test]
fn test_convert_messages_empty_input() {
    let messages: Vec<serde_json::Value> = vec![];
    let (system, claude_msgs) = convert_messages_for_claude(&messages);
    assert!(system.is_none());
    assert!(claude_msgs.is_empty());
}

#[test]
fn test_convert_messages_system_interleaved() {
    // System messages can appear anywhere — all should be extracted
    let messages = vec![
        json!({"role": "system", "content": "First system"}),
        json!({"role": "user", "content": "Hello"}),
        json!({"role": "system", "content": "Second system"}),
        json!({"role": "assistant", "content": "Hi"}),
    ];

    let (system, claude_msgs) = convert_messages_for_claude(&messages);
    let sys = system.unwrap();
    assert!(sys.contains("First system"));
    assert!(sys.contains("Second system"));
    // Only user and assistant remain
    assert_eq!(claude_msgs.len(), 2);
}

#[test]
fn test_convert_messages_multiple_tool_use_on_assistant() {
    let messages = vec![
        json!({"role": "user", "content": "Do three things"}),
        json!({
            "role": "assistant",
            "content": "I'll help with all three.",
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "tool_a", "arguments": "{\"x\": 1}"}},
                {"id": "c2", "type": "function", "function": {"name": "tool_b", "arguments": "{\"y\": 2}"}},
                {"id": "c3", "type": "function", "function": {"name": "tool_c", "arguments": "{\"z\": 3}"}}
            ]
        }),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    let content = claude_msgs[1]["content"].as_array().unwrap();
    // 1 text block + 3 tool_use blocks
    assert_eq!(content.len(), 4);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["name"], "tool_a");
    assert_eq!(content[2]["type"], "tool_use");
    assert_eq!(content[2]["name"], "tool_b");
    assert_eq!(content[3]["type"], "tool_use");
    assert_eq!(content[3]["name"], "tool_c");
}

#[test]
fn test_convert_messages_full_conversation_with_tool_cycle() {
    // Simulate a full multi-turn with tool use
    let messages = vec![
        json!({"role": "system", "content": "You are helpful."}),
        json!({"role": "user", "content": "Search for X"}),
        json!({
            "role": "assistant",
            "content": "Searching...",
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "search", "arguments": "{\"q\": \"X\"}"}}]
        }),
        json!({"role": "tool", "tool_call_id": "c1", "content": "Found: X is great"}),
        json!({"role": "assistant", "content": "I found that X is great."}),
        json!({"role": "user", "content": "Thanks!"}),
    ];

    let (system, claude_msgs) = convert_messages_for_claude(&messages);
    assert_eq!(system.unwrap(), "You are helpful.");
    // user, assistant (with tool_use), user (tool_result), assistant, user
    assert_eq!(claude_msgs.len(), 5);
    assert_eq!(claude_msgs[0]["role"], "user");
    assert_eq!(claude_msgs[1]["role"], "assistant");
    assert_eq!(claude_msgs[2]["role"], "user"); // tool_result
    assert_eq!(claude_msgs[3]["role"], "assistant");
    assert_eq!(claude_msgs[4]["role"], "user");
}

#[test]
fn test_convert_messages_assistant_with_tool_calls_no_text() {
    // Assistant with tool_calls but null/missing content
    let messages = vec![
        json!({"role": "user", "content": "run tool"}),
        json!({
            "role": "assistant",
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "t", "arguments": "{}"}}]
        }),
    ];

    let (_, claude_msgs) = convert_messages_for_claude(&messages);
    let content = claude_msgs[1]["content"].as_array().unwrap();
    // Should have only tool_use block (no empty text block since content was null/missing)
    assert!(!content.is_empty());
    // The last block should be tool_use
    assert_eq!(content.last().unwrap()["type"], "tool_use");
}

// ---------------------------------------------------------------------------
// convert_tools_for_claude tests (new)
// ---------------------------------------------------------------------------

#[test]
fn test_convert_tools_basic() {
    let openai_tools = vec![json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the weather for a city",
            "parameters": {
                "type": "object",
                "properties": {
                    "city": {"type": "string", "description": "City name"}
                },
                "required": ["city"]
            }
        }
    })];

    let claude_tools = convert_tools_for_claude(&openai_tools);
    assert_eq!(claude_tools.len(), 1);
    assert_eq!(claude_tools[0]["name"], "get_weather");
    assert_eq!(claude_tools[0]["description"], "Get the weather for a city");
    assert_eq!(claude_tools[0]["input_schema"]["type"], "object");
    assert_eq!(
        claude_tools[0]["input_schema"]["properties"]["city"]["type"],
        "string"
    );
    // Should NOT have "type": "function" wrapper
    assert!(claude_tools[0].get("type").is_none());
    assert!(claude_tools[0].get("function").is_none());
}

#[test]
fn test_convert_tools_multiple() {
    let openai_tools = vec![
        json!({
            "type": "function",
            "function": {"name": "tool_a", "description": "Desc A", "parameters": {"type": "object", "properties": {}}}
        }),
        json!({
            "type": "function",
            "function": {"name": "tool_b", "description": "Desc B", "parameters": {"type": "object", "properties": {}}}
        }),
        json!({
            "type": "function",
            "function": {"name": "tool_c", "description": "Desc C", "parameters": {"type": "object", "properties": {}}}
        }),
    ];

    let claude_tools = convert_tools_for_claude(&openai_tools);
    assert_eq!(claude_tools.len(), 3);
    assert_eq!(claude_tools[0]["name"], "tool_a");
    assert_eq!(claude_tools[1]["name"], "tool_b");
    assert_eq!(claude_tools[2]["name"], "tool_c");
}

#[test]
fn test_convert_tools_missing_description() {
    let openai_tools = vec![json!({
        "type": "function",
        "function": {"name": "tool_x", "parameters": {"type": "object", "properties": {}}}
    })];

    let claude_tools = convert_tools_for_claude(&openai_tools);
    assert_eq!(claude_tools.len(), 1);
    assert_eq!(claude_tools[0]["name"], "tool_x");
    assert_eq!(claude_tools[0]["description"], "");
}

#[test]
fn test_convert_tools_missing_parameters() {
    let openai_tools = vec![json!({
        "type": "function",
        "function": {"name": "simple_tool", "description": "A simple tool"}
    })];

    let claude_tools = convert_tools_for_claude(&openai_tools);
    assert_eq!(claude_tools.len(), 1);
    // Should default to empty object schema
    assert_eq!(claude_tools[0]["input_schema"]["type"], "object");
}

#[test]
fn test_convert_tools_empty() {
    let openai_tools: Vec<serde_json::Value> = vec![];
    let claude_tools = convert_tools_for_claude(&openai_tools);
    assert!(claude_tools.is_empty());
}

#[test]
fn test_convert_tools_invalid_format_skipped() {
    // Malformed tool without "function" key should be skipped
    let openai_tools = vec![
        json!({"type": "function", "function": {"name": "good", "description": "ok", "parameters": {"type": "object", "properties": {}}}}),
        json!({"type": "something_else", "data": "no function key"}),
        json!({"type": "function", "function": {"name": "also_good", "description": "fine", "parameters": {"type": "object", "properties": {}}}}),
    ];

    let claude_tools = convert_tools_for_claude(&openai_tools);
    assert_eq!(claude_tools.len(), 2);
    assert_eq!(claude_tools[0]["name"], "good");
    assert_eq!(claude_tools[1]["name"], "also_good");
}

#[test]
fn test_convert_tools_complex_parameters() {
    let openai_tools = vec![json!({
        "type": "function",
        "function": {
            "name": "complex_tool",
            "description": "A tool with complex params",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "count": {"type": "integer", "minimum": 0},
                    "options": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "nested": {
                        "type": "object",
                        "properties": {
                            "inner": {"type": "boolean"}
                        }
                    }
                },
                "required": ["name"]
            }
        }
    })];

    let claude_tools = convert_tools_for_claude(&openai_tools);
    let schema = &claude_tools[0]["input_schema"];
    assert_eq!(schema["properties"]["name"]["type"], "string");
    assert_eq!(schema["properties"]["count"]["minimum"], 0);
    assert_eq!(schema["properties"]["options"]["type"], "array");
    assert_eq!(schema["required"][0], "name");
}

// ---------------------------------------------------------------------------
// Claude response parsing edge cases (via ThinkResult normalization)
// These test the data structures that parse_claude_response would produce.
// ---------------------------------------------------------------------------

#[test]
fn test_claude_tool_call_normalization_format() {
    // Verify the normalized tool_call format matches what the runner expects
    let normalized_tool_call = json!({
        "id": "toolu_abc123",
        "type": "function",
        "function": {
            "name": "execute_bash",
            "arguments": "{\"command\": \"ls\"}"
        }
    });

    // Runner extracts these fields (runner.rs:720-731)
    let call_id = normalized_tool_call["id"].as_str().unwrap();
    let function = normalized_tool_call.get("function").unwrap();
    let name = function["name"].as_str().unwrap();
    let arguments_str = function["arguments"].as_str().unwrap();
    let arguments: serde_json::Value = serde_json::from_str(arguments_str).unwrap();

    assert_eq!(call_id, "toolu_abc123");
    assert_eq!(name, "execute_bash");
    assert_eq!(arguments["command"], "ls");
}

#[test]
fn test_think_result_with_claude_tool_calls() {
    // ThinkResult with Claude-style tool IDs should serialize/deserialize fine
    let r = ThinkResult {
        text: "".to_string(),
        model: "claude-sonnet-4-6".to_string(),
        sats_paid: 3000,
        sats_effective: 2800,
        sats_refunded: 200,
        prompt_tokens: 50,
        completion_tokens: 30,
        total_tokens: 80,
        finish_reason: "tool_calls".to_string(),
        duration_ms: 800,
        tool_calls: vec![
            json!({
                "id": "toolu_01ABC",
                "type": "function",
                "function": {"name": "memory_search", "arguments": "{\"query\": \"test\"}"}
            }),
            json!({
                "id": "toolu_02DEF",
                "type": "function",
                "function": {"name": "file_read", "arguments": "{\"path\": \"/tmp/x\"}"}
            }),
        ],
        payment_txid: Some("beef123".to_string()),
        refund_internalized: None,
        was_text_extracted: false,
    };

    let json_str = serde_json::to_string(&r).unwrap();
    let deserialized: ThinkResult = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.tool_calls.len(), 2);
    assert_eq!(deserialized.model, "claude-sonnet-4-6");
    assert_eq!(deserialized.finish_reason, "tool_calls");
    assert_eq!(deserialized.tool_calls[0]["id"], "toolu_01ABC");
    assert_eq!(deserialized.tool_calls[1]["function"]["name"], "file_read");
}

#[test]
fn test_claude_stop_reason_mapping() {
    // Verify the expected mapping from Claude stop_reason to OpenAI finish_reason
    let mappings = vec![
        ("end_turn", "stop"),
        ("tool_use", "tool_calls"),
        ("max_tokens", "length"),
        ("stop_sequence", "stop"),
    ];
    for (claude_reason, expected_openai) in mappings {
        let mapped = match claude_reason {
            "end_turn" => "stop",
            "tool_use" => "tool_calls",
            "max_tokens" => "length",
            "stop_sequence" => "stop",
            other => other,
        };
        assert_eq!(
            mapped, expected_openai,
            "Mapping failed for {claude_reason}"
        );
    }
}

// ---------------------------------------------------------------------------
// Round-trip: OpenAI messages → Claude → verify structure
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_complex_conversation() {
    // Full agent conversation: system + user + assistant with tools + tool results + final
    let messages = vec![
        json!({"role": "system", "content": "You are a coding assistant."}),
        json!({"role": "user", "content": "Find all Python files"}),
        json!({
            "role": "assistant",
            "content": "I'll search for Python files.",
            "tool_calls": [{
                "id": "call_search",
                "type": "function",
                "function": {"name": "file_search", "arguments": "{\"pattern\": \"*.py\"}"}
            }]
        }),
        json!({"role": "tool", "tool_call_id": "call_search", "content": "Found: main.py, test.py"}),
        json!({"role": "assistant", "content": "I found 2 Python files: main.py and test.py."}),
        json!({"role": "user", "content": "Read main.py"}),
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": "call_read",
                "type": "function",
                "function": {"name": "file_read", "arguments": "{\"path\": \"main.py\"}"}
            }]
        }),
        json!({"role": "tool", "tool_call_id": "call_read", "content": "print('hello world')"}),
        json!({"role": "assistant", "content": "The file contains a simple hello world program."}),
    ];

    let (system, claude_msgs) = convert_messages_for_claude(&messages);

    // System extracted
    assert_eq!(system.unwrap(), "You are a coding assistant.");

    // Verify message count: user, assistant, user(tool_result), assistant, user, assistant, user(tool_result), assistant
    assert_eq!(claude_msgs.len(), 8);

    // Verify alternating roles work for Claude
    let roles: Vec<&str> = claude_msgs
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        vec![
            "user",
            "assistant",
            "user",
            "assistant",
            "user",
            "assistant",
            "user",
            "assistant"
        ]
    );

    // Verify tool_use is in assistant content blocks
    let assistant_1_content = claude_msgs[1]["content"].as_array().unwrap();
    assert!(assistant_1_content.iter().any(|b| b["type"] == "tool_use"));

    // Verify tool_result is in user content blocks
    let tool_result_1 = claude_msgs[2]["content"].as_array().unwrap();
    assert_eq!(tool_result_1[0]["type"], "tool_result");
    assert_eq!(tool_result_1[0]["tool_use_id"], "call_search");
}

// ---------------------------------------------------------------------------
// Cross-check: Claude models aren't flagged as reasoning
// ---------------------------------------------------------------------------

#[test]
fn test_claude_models_not_reasoning() {
    assert!(!is_reasoning_model("claude-sonnet-4-6"));
    assert!(!is_reasoning_model("claude-opus-4-6"));
    assert!(!is_reasoning_model("claude-haiku-4-5"));
    assert!(!is_reasoning_model("claude-3-5-sonnet"));
}

// ---------------------------------------------------------------------------
// Task 2.3: refund_internalized field on ThinkResult
// ---------------------------------------------------------------------------

#[test]
fn test_think_result_refund_internalized_none_skipped() {
    let r = ThinkResult {
        text: "hi".into(),
        model: "m".into(),
        sats_paid: 0,
        sats_effective: 0,
        sats_refunded: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        finish_reason: String::new(),
        duration_ms: 0,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    let json_str = serde_json::to_string(&r).unwrap();
    // None should be omitted from JSON (skip_serializing_if)
    assert!(!json_str.contains("refund_internalized"));
}

#[test]
fn test_think_result_refund_internalized_true() {
    let r = ThinkResult {
        text: "ok".into(),
        model: "gpt-5-mini".into(),
        sats_paid: 1000,
        sats_effective: 800,
        sats_refunded: 200,
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: 30,
        finish_reason: "stop".into(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: Some("tx123".into()),
        refund_internalized: Some(true),
        was_text_extracted: false,
    };
    let json_str = serde_json::to_string(&r).unwrap();
    assert!(json_str.contains("\"refund_internalized\":true"));

    let deserialized: ThinkResult = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.refund_internalized, Some(true));
}

#[test]
fn test_think_result_refund_internalized_false() {
    let r = ThinkResult {
        text: "ok".into(),
        model: "gpt-5-mini".into(),
        sats_paid: 1000,
        sats_effective: 800,
        sats_refunded: 200,
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: 30,
        finish_reason: "stop".into(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: Some("tx123".into()),
        refund_internalized: Some(false),
        was_text_extracted: false,
    };
    let json_str = serde_json::to_string(&r).unwrap();
    assert!(json_str.contains("\"refund_internalized\":false"));

    let deserialized: ThinkResult = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.refund_internalized, Some(false));
}

#[test]
fn test_think_result_refund_internalized_default_on_deserialize() {
    // Old JSON without refund_internalized should deserialize with None
    let json_str = r#"{"text":"hi","model":"m","sats_paid":0,"sats_effective":0,"sats_refunded":0,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"finish_reason":"","duration_ms":0}"#;
    let r: ThinkResult = serde_json::from_str(json_str).unwrap();
    assert_eq!(r.refund_internalized, None);
}

// ---------------------------------------------------------------------------
// Task 2.4: sats_effective should not exceed sats_paid (sanity invariant)
// ---------------------------------------------------------------------------

#[test]
fn test_sats_effective_lte_sats_paid_normal() {
    // Normal case: sats_effective <= sats_paid
    let r = ThinkResult {
        text: "response".into(),
        model: "gpt-5-mini".into(),
        sats_paid: 1000,
        sats_effective: 800,
        sats_refunded: 200,
        prompt_tokens: 50,
        completion_tokens: 50,
        total_tokens: 100,
        finish_reason: "stop".into(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    assert!(r.sats_effective <= r.sats_paid);
}

#[test]
fn test_sats_effective_equals_sats_paid_no_refund() {
    // When no refund, sats_effective == sats_paid
    let r = ThinkResult {
        text: "response".into(),
        model: "gpt-5-mini".into(),
        sats_paid: 500,
        sats_effective: 500,
        sats_refunded: 0,
        prompt_tokens: 50,
        completion_tokens: 50,
        total_tokens: 100,
        finish_reason: "stop".into(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    assert!(r.sats_effective <= r.sats_paid);
    assert_eq!(r.sats_effective, r.sats_paid);
}

#[test]
fn test_sats_effective_greater_than_sats_paid_is_anomaly() {
    // This should be flagged as anomalous in the runner
    let r = ThinkResult {
        text: "response".into(),
        model: "gpt-5-mini".into(),
        sats_paid: 100,
        sats_effective: 200, // anomaly!
        sats_refunded: 0,
        prompt_tokens: 50,
        completion_tokens: 50,
        total_tokens: 100,
        finish_reason: "stop".into(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    // The anomaly condition the runner checks
    assert!(r.sats_effective > r.sats_paid && r.sats_paid > 0);
}

#[test]
fn test_sats_effective_anomaly_zero_paid_is_not_flagged() {
    // When sats_paid is 0, the condition should NOT trigger
    // (sats_paid > 0 guard prevents spurious warnings for free calls)
    let r = ThinkResult {
        text: "response".into(),
        model: "gpt-5-mini".into(),
        sats_paid: 0,
        sats_effective: 0,
        sats_refunded: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        finish_reason: "stop".into(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: None,
        refund_internalized: None,
        was_text_extracted: false,
    };
    // The runner condition: sats_effective > sats_paid && sats_paid > 0
    assert!(!(r.sats_effective > r.sats_paid && r.sats_paid > 0));
}

// ===========================================================================
// Extended Thinking & Reasoning Effort (Issue #172 Parts C+D)
// ===========================================================================

use dolphin_milk::think::{build_claude_body, build_openai_body};

#[test]
fn test_build_openai_body_with_reasoning_effort() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_openai_body(&messages, "o4-mini", 4096, None, None, Some("high"));
    // reasoning_effort should be present for reasoning models
    assert_eq!(body["reasoning_effort"], json!("high"));
    // Reasoning models use max_completion_tokens
    assert_eq!(body["max_completion_tokens"], json!(4096));
    // max_tokens is also sent for x402 proxy compatibility
    assert_eq!(body["max_tokens"], json!(4096));
}

#[test]
fn test_build_openai_body_reasoning_effort_skipped_for_non_reasoning() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_openai_body(&messages, "gpt-4o", 4096, None, None, Some("high"));
    // reasoning_effort should NOT be present for non-reasoning models
    assert!(body.get("reasoning_effort").is_none());
    // Non-reasoning models use max_tokens
    assert_eq!(body["max_tokens"], json!(4096));
}

#[test]
fn test_build_claude_body_with_thinking() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_claude_body(
        &messages,
        "claude-sonnet-4-6",
        4096,
        None,
        None,
        Some(10000),
    );
    // thinking field should be present
    assert_eq!(body["thinking"]["type"], json!("enabled"));
    assert_eq!(body["thinking"]["budget_tokens"], json!(10000));
    // max_tokens should be adjusted upward: budget(10000) + 1024 = 11024 > 4096
    assert_eq!(body["max_tokens"], json!(11024));
}

#[test]
fn test_build_claude_body_without_thinking() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_claude_body(&messages, "claude-sonnet-4-6", 4096, None, None, None);
    // No thinking field when budget is None
    assert!(body.get("thinking").is_none());
    // max_tokens unchanged
    assert_eq!(body["max_tokens"], json!(4096));
}

#[test]
fn test_build_claude_body_thinking_no_max_tokens_adjustment_when_sufficient() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    // max_tokens (65000) already >= budget(10000) + 1024 = 11024
    let body = build_claude_body(
        &messages,
        "claude-sonnet-4-6",
        65000,
        None,
        None,
        Some(10000),
    );
    assert_eq!(body["thinking"]["type"], json!("enabled"));
    // max_tokens should remain at 65000 since it's already sufficient
    assert_eq!(body["max_tokens"], json!(65000));
}

#[test]
fn test_build_openai_body_reasoning_effort_none() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_openai_body(&messages, "o4-mini", 4096, None, None, None);
    // reasoning_effort should not be present when None
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn test_build_openai_body_reasoning_effort_low() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_openai_body(&messages, "gpt-5-mini", 4096, None, None, Some("low"));
    assert_eq!(body["reasoning_effort"], json!("low"));
}

#[test]
fn test_build_openai_body_reasoning_effort_medium() {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let body = build_openai_body(&messages, "gpt-5.2", 4096, None, None, Some("medium"));
    assert_eq!(body["reasoning_effort"], json!("medium"));
}
