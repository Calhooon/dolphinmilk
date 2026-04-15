//! Tests for context — system prompt and context manager.
//! Mirrors Python tests/test_context.py (12 tests).

use tempfile::TempDir;

use dolphin_milk::context::manager::{
    compact_content, estimate_message_tokens, estimate_tokens, ContextManager,
};
use dolphin_milk::context::prompt::{build_system_prompt, PromptContext, ToolDesc};

fn default_ctx() -> PromptContext {
    PromptContext {
        identity_key: String::new(),
        balance_sats: 0,
        model: String::new(),
        tools: Vec::new(),
        memory_summary: String::new(),
        available_files: Vec::new(),
        task: String::new(),
        budget_remaining: 0,
        low_power: false,
        inbox_count: 0,
        skills_section: String::new(),
        workspace_path: String::new(),
        certificate_info: None,
        has_external_messages: false,
        identity_soul: None,
        auto_recall_ids: vec![],
        basket_health: std::collections::HashMap::new(),
        spendable_output_count: 0,
        instructions: String::new(),
        env_snapshot: None,
        working_memory_section: String::new(),
    }
}

// -- TestPromptBuilder --

#[test]
fn test_basic_prompt() {
    let ctx = default_ctx();
    let prompt = build_system_prompt(&ctx);
    assert!(
        prompt.contains("Dolphin Milk"),
        "should contain 'Dolphin Milk'"
    );
    assert!(prompt.contains("Identity"), "should contain 'Identity'");
    assert!(
        prompt.contains("Environment"),
        "should contain 'Environment'"
    );
    assert!(prompt.contains("Wallet"), "should contain 'Wallet'");
    assert!(
        prompt.contains("Working Principles"),
        "should contain 'Working Principles'"
    );
}

#[test]
fn test_identity_key_included() {
    let mut ctx = default_ctx();
    ctx.identity_key = "034aa44668fbc73ca5d490f0fa54b98b".to_string();
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("034aa44668fbc73ca5d490f0fa54b98b"));
}

#[test]
fn test_tools_section() {
    let mut ctx = default_ctx();
    ctx.tools = vec![
        ToolDesc {
            name: "execute_bash".to_string(),
            description: "Run shell commands".to_string(),
            category: "sandbox".to_string(),
            deferred: false,
            hint: None,
        },
        ToolDesc {
            name: "file_read".to_string(),
            description: "Read files".to_string(),
            category: "sandbox".to_string(),
            deferred: false,
            hint: None,
        },
    ];
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("execute_bash"));
    assert!(prompt.contains("file_read"));
}

#[test]
fn test_low_power_warning() {
    let mut ctx = default_ctx();
    ctx.low_power = true;
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("Low funds"));
}

#[test]
fn test_memory_section() {
    let mut ctx = default_ctx();
    ctx.memory_summary = "I know about BSV".to_string();
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("I know about BSV"));
}

#[test]
fn test_available_files() {
    let mut ctx = default_ctx();
    ctx.available_files = vec![
        "context/doc1.txt".to_string(),
        "context/doc2.txt".to_string(),
    ];
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("doc1.txt"));
    assert!(prompt.contains("doc2.txt"));
}

#[test]
fn test_self_signed_certificate_in_prompt() {
    use dolphin_milk::context::prompt::CertificateInfo;
    let mut ctx = default_ctx();
    ctx.certificate_info = Some(CertificateInfo {
        certifier: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".to_string(),
        self_signed: true,
        capabilities: "llm,tools,messaging,x402".to_string(),
        name: "dolphin-milk-agent".to_string(),
        budget_per_task: None,
        budget_per_hour: None,
        budget_per_day: None,
        budget_per_week: None,
        budget_per_month: None,
        budget_lifetime: None,
        budget_enforcement: None,
    });
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("self-signed"), "should mention self-signed");
    assert!(
        prompt.contains("dolphin-milk-agent"),
        "should include agent name"
    );
    assert!(prompt.contains("llm,tools"), "should include capabilities");
    assert!(prompt.contains("bootstrap"), "should mention bootstrap");
}

#[test]
fn test_parent_signed_certificate_in_prompt() {
    use dolphin_milk::context::prompt::CertificateInfo;
    let mut ctx = default_ctx();
    ctx.certificate_info = Some(CertificateInfo {
        certifier: "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_string(),
        self_signed: false,
        capabilities: "llm,tools".to_string(),
        name: "prod-agent".to_string(),
        budget_per_task: None,
        budget_per_hour: None,
        budget_per_day: None,
        budget_per_week: None,
        budget_per_month: None,
        budget_lifetime: None,
        budget_enforcement: None,
    });
    let prompt = build_system_prompt(&ctx);
    assert!(
        prompt.contains("authorized by your parent"),
        "should mention parent authorization"
    );
    assert!(
        prompt.contains("0279be667ef9dcbbac55a06295ce870b"),
        "should include certifier key"
    );
    assert!(prompt.contains("prod-agent"), "should include agent name");
    assert!(
        prompt.contains("prove_identity"),
        "should mention prove_identity"
    );
}

#[test]
fn test_no_certificate_in_prompt() {
    let ctx = default_ctx();
    let prompt = build_system_prompt(&ctx);
    // The identity section should not mention cert details when no cert is provided.
    // Note: BRC-52 may appear in the static System Internals section, so we check
    // for the specific identity-section cert phrases instead.
    assert!(
        !prompt.contains("self-signed BRC-52 authorization"),
        "no self-signed cert info without certificate"
    );
    assert!(
        !prompt.contains("authorized by your parent"),
        "no parent cert info without certificate"
    );
}

// -- TestContextManager --

#[test]
fn test_estimate_tokens() {
    assert!(estimate_tokens("hello") > 0);
    assert_eq!(estimate_tokens(&"a".repeat(400)), 100); // ~4 chars per token
}

#[test]
fn test_estimate_message_tokens() {
    let msg = serde_json::json!({"role": "user", "content": "hello world"});
    let tokens = estimate_message_tokens(&msg);
    assert!(tokens > 0);
}

#[test]
fn test_build_messages_all_fit() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let history = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": "hi"}),
    ];
    let msgs = cm.build_messages("You are a worm", &history, None);
    assert_eq!(msgs.len(), 3); // system + 2 history
    assert_eq!(msgs[0]["role"], "system");
}

#[test]
fn test_build_messages_empty_history() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let msgs = cm.build_messages("You are a worm", &[], None);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "system");
}

#[test]
fn test_build_messages_truncation() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100, dir.path().join("ctx")); // Very small budget
    let history = vec![
        serde_json::json!({"role": "user", "content": "a".repeat(1000)}),
        serde_json::json!({"role": "assistant", "content": "b".repeat(1000)}),
        serde_json::json!({"role": "user", "content": "c".repeat(1000)}),
        serde_json::json!({"role": "assistant", "content": "d".repeat(1000)}),
        serde_json::json!({"role": "user", "content": "recent message"}),
    ];
    let msgs = cm.build_messages("system", &history, None);
    // Should still include system prompt and at least some messages
    assert!(!msgs.is_empty());
    assert_eq!(msgs[0]["role"], "system");
}

#[test]
fn test_should_offload() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));
    assert!(!cm.should_offload("short text"));
    assert!(cm.should_offload(&"x".repeat(20_000)));
}

#[test]
fn test_build_messages_max_history_turns() {
    let dir = TempDir::new().unwrap();
    // max_history_turns=5, min_recent_messages=2
    let cm = ContextManager::with_limits(100_000, dir.path().join("ctx"), 2, 5);
    // Create 10 messages — should be trimmed to last 5
    let history: Vec<serde_json::Value> = (0..10)
        .map(|i| serde_json::json!({"role": "user", "content": format!("msg{i}")}))
        .collect();
    let msgs = cm.build_messages("system", &history, None);
    // system + 5 history = 6
    assert_eq!(msgs.len(), 6);
    // First history message should be msg5 (index 5 of original)
    assert_eq!(msgs[1]["content"], "msg5");
    assert_eq!(msgs[5]["content"], "msg9");
}

#[test]
fn test_build_messages_with_limits() {
    let dir = TempDir::new().unwrap();
    // Confirm with_limits constructor works for min_recent_messages
    let cm = ContextManager::with_limits(100, dir.path().join("ctx"), 2, 40);
    let history: Vec<serde_json::Value> = (0..6)
        .map(|_| serde_json::json!({"role": "user", "content": "a".repeat(200)}))
        .collect();
    let msgs = cm.build_messages("sys", &history, None);
    // With tiny budget, should still keep at least the last 2 messages
    assert!(msgs.len() >= 3); // system + at least 2
}

#[test]
fn test_offload_to_file() {
    let dir = TempDir::new().unwrap();
    let mut cm = ContextManager::new(128_000, dir.path().join("ctx"));
    let path = cm.offload_to_file("big content here", "test");
    assert!(std::path::Path::new(&path).exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "big content here");
    assert_eq!(cm.offloaded_files().len(), 1);
}

// -- sanitize_tool_pairs (called from build_messages) --

#[test]
fn test_build_messages_removes_orphaned_tool_result() {
    // Tool result with no matching assistant tool_calls should be stripped
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let history = vec![
        // Tool result with no preceding assistant message that has tool_calls
        serde_json::json!({
            "role": "tool",
            "tool_call_id": "call-99",
            "content": "result",
        }),
        serde_json::json!({"role": "user", "content": "hello"}),
    ];
    let msgs = cm.build_messages("system", &history, None);
    // The orphaned tool message should be removed
    let roles: Vec<&str> = msgs
        .iter()
        .filter_map(|m| m.get("role").and_then(|v| v.as_str()))
        .collect();
    assert!(
        !roles.contains(&"tool"),
        "orphaned tool result must be stripped: {roles:?}"
    );
}

#[test]
fn test_build_messages_removes_assistant_with_missing_tool_results() {
    // Assistant message with tool_calls but results missing should be stripped
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let history = vec![
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "execute_bash", "arguments": "{}"}}],
        }),
        // No tool result follows
        serde_json::json!({"role": "user", "content": "next question"}),
    ];
    let msgs = cm.build_messages("system", &history, None);
    let roles: Vec<&str> = msgs
        .iter()
        .filter_map(|m| m.get("role").and_then(|v| v.as_str()))
        .collect();
    assert!(
        !roles.contains(&"assistant") || {
            // If assistant is present, it must not have tool_calls
            msgs.iter()
                .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
                .all(|m| m.get("tool_calls").is_none())
        },
        "assistant with unmatched tool_calls must be stripped: {roles:?}"
    );
}

#[test]
fn test_build_messages_preserves_complete_tool_pairs() {
    // Paired assistant+tool messages should NOT be stripped
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let history = vec![
        serde_json::json!({"role": "user", "content": "run ls"}),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "execute_bash", "arguments": "{}"}}],
        }),
        serde_json::json!({
            "role": "tool",
            "tool_call_id": "call-1",
            "content": "file.txt",
        }),
        serde_json::json!({"role": "assistant", "content": "Done."}),
    ];
    let msgs = cm.build_messages("system", &history, None);
    // system + all 4 history messages = 5
    assert_eq!(msgs.len(), 5, "complete tool pair must be preserved");
}

#[test]
fn test_build_messages_preserves_assistant_without_tool_calls() {
    // Plain assistant message (no tool_calls) should never be removed
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let history = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": "hi there"}),
    ];
    let msgs = cm.build_messages("system", &history, None);
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2]["content"], "hi there");
}

// -- compact_content / compact_large_messages tests --

#[test]
fn test_compact_content_strips_markdown_base64_image() {
    let content = "Here is the image:\n![sunrise](data:image/jpeg;base64,/9j/4AAQSkZJRgABAQ...very-long-base64)\nDone.";
    let result = compact_content(content);
    assert!(
        !result.contains("data:image"),
        "data URI should be stripped"
    );
    assert!(
        !result.contains("/9j/"),
        "base64 content should be stripped"
    );
    assert!(result.contains("sunrise"), "alt text should be preserved");
    assert!(
        result.contains("Done."),
        "surrounding text should be preserved"
    );
}

#[test]
fn test_compact_content_strips_bare_data_uri() {
    let content = "The image URL is data:image/png;base64,iVBORw0KGgo...longbase64 and more text.";
    let result = compact_content(content);
    assert!(
        !result.contains("data:image"),
        "bare data URI should be stripped"
    );
    assert!(
        result.contains("and more text"),
        "surrounding text should be preserved"
    );
}

#[test]
fn test_compact_content_preserves_https_urls() {
    let content = "Here is the image:\n![sunrise](https://replicate.delivery/example.jpg)\nDone.";
    let result = compact_content(content);
    assert_eq!(result, content, "https URLs should be preserved unchanged");
}

#[test]
fn test_compact_content_mixed_urls() {
    let content = "Good URL: ![a](https://example.com/img.jpg)\nBad: ![b](data:image/jpeg;base64,/9j/AAAA)\nEnd.";
    let result = compact_content(content);
    assert!(
        result.contains("https://example.com/img.jpg"),
        "https URL preserved"
    );
    assert!(!result.contains("data:image"), "data URI stripped");
    assert!(result.contains("End."), "trailing text preserved");
}

#[test]
fn test_compact_content_truncates_oversized() {
    let big = "x".repeat(20_000);
    let result = compact_content(&big);
    assert!(result.len() < big.len(), "should be truncated");
    assert!(
        result.contains("truncated"),
        "should have truncation marker"
    );
}

#[test]
fn test_compact_content_passthrough_small() {
    let small = "This is a normal message.";
    let result = compact_content(small);
    assert_eq!(result, small, "small content should pass through unchanged");
}

#[test]
fn test_build_messages_compacts_assistant_base64() {
    // The critical bug: base64 in assistant messages was not compacted
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(100_000, dir.path().join("ctx"));
    let base64_content = format!(
        "Here is your image:\n![sunrise](data:image/jpeg;base64,{})\nEnjoy!",
        "/9j/".repeat(100_000) // ~400K chars
    );
    let history = vec![
        serde_json::json!({"role": "user", "content": "generate an image"}),
        serde_json::json!({"role": "assistant", "content": base64_content}),
    ];
    let msgs = cm.build_messages("system", &history, None);
    let assistant_content = msgs[2]["content"].as_str().unwrap();
    assert!(
        !assistant_content.contains("data:image"),
        "data URI must be stripped from assistant"
    );
    assert!(
        assistant_content.len() < 20_000,
        "assistant content must be compacted"
    );
}

#[test]
fn test_build_messages_compacts_tool_results() {
    // Existing behavior: tool results should still be compacted.
    // Use a small max_tokens so the dynamic per-message limit (~8K) is below 20K content.
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(30_000, dir.path().join("ctx"));
    let big_output = "x".repeat(20_000);
    let history = vec![
        serde_json::json!({"role": "user", "content": "run ls"}),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "execute_bash", "arguments": "{}"}}],
        }),
        serde_json::json!({
            "role": "tool",
            "tool_call_id": "call-1",
            "content": big_output,
        }),
    ];
    let msgs = cm.build_messages("system", &history, None);
    let tool_content = msgs[3]["content"].as_str().unwrap();
    assert!(
        tool_content.len() < big_output.len(),
        "tool result should be compacted"
    );
    assert!(
        tool_content.contains("truncated"),
        "should have truncation marker"
    );
}

// -- Phase 6: Compaction tests --

#[test]
fn test_needs_compaction_threshold() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 8, 10);

    assert!(
        !cm.needs_compaction(5),
        "below max should not need compaction"
    );
    assert!(
        !cm.needs_compaction(10),
        "at max should not need compaction"
    );
    assert!(cm.needs_compaction(11), "above max should need compaction");
    assert!(
        cm.needs_compaction(100),
        "well above max should need compaction"
    );
}

#[test]
fn test_compaction_summary_accessors() {
    let dir = TempDir::new().unwrap();
    let mut cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 8, 10);

    assert!(cm.compaction_summary().is_none(), "initially None");
    cm.set_compaction_summary("Summary of earlier conversation.".to_string());
    assert_eq!(
        cm.compaction_summary(),
        Some("Summary of earlier conversation.")
    );
    assert_eq!(cm.max_history_turns(), 10);
}

#[test]
fn test_compaction_summary_prepended_in_messages() {
    let dir = TempDir::new().unwrap();
    let mut cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 2, 3);

    cm.set_compaction_summary("User asked about X. Decision was Y.".to_string());

    // Build 5 messages — exceeds max_history_turns of 3, so oldest 2 are dropped
    let history: Vec<serde_json::Value> = (0..5)
        .map(|i| serde_json::json!({"role": "user", "content": format!("msg {i}")}))
        .collect();

    let messages = cm.build_messages("system prompt", &history, None);

    // First message is system prompt
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "system prompt");

    // Second message should be the compaction summary
    assert_eq!(messages[1]["role"], "system");
    let summary_content = messages[1]["content"].as_str().unwrap();
    assert!(
        summary_content.contains("2 messages compacted"),
        "should mention dropped count, got: {summary_content}"
    );
    assert!(
        summary_content.contains("User asked about X"),
        "should contain summary text"
    );

    // Remaining messages are the kept history (last 3)
    assert_eq!(messages[2]["content"], "msg 2");
    assert_eq!(messages[3]["content"], "msg 3");
    assert_eq!(messages[4]["content"], "msg 4");
}

#[test]
fn test_compaction_summary_not_prepended_when_unset() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 2, 3);

    // No compaction summary set — should not inject a summary message
    let history: Vec<serde_json::Value> = (0..5)
        .map(|i| serde_json::json!({"role": "user", "content": format!("msg {i}")}))
        .collect();

    let messages = cm.build_messages("system prompt", &history, None);

    // First is system prompt, then the last 3 history messages (no summary injected)
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "system prompt");
    assert_eq!(messages[1]["content"], "msg 2");
    assert_eq!(messages[2]["content"], "msg 3");
    assert_eq!(messages[3]["content"], "msg 4");
    assert_eq!(messages.len(), 4, "system + 3 history, no summary");
}

#[test]
fn test_compaction_summary_not_injected_when_history_fits() {
    let dir = TempDir::new().unwrap();
    let mut cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 2, 10);

    cm.set_compaction_summary("This should not appear.".to_string());

    // Only 3 messages — below max_history_turns of 10, no trimming
    let history: Vec<serde_json::Value> = (0..3)
        .map(|i| serde_json::json!({"role": "user", "content": format!("msg {i}")}))
        .collect();

    let messages = cm.build_messages("system prompt", &history, None);

    // No trimming happened, so no summary injected
    assert_eq!(messages.len(), 4, "system + 3 history, no summary");
    // Verify none of the messages contain the summary
    for msg in &messages {
        if let Some(content) = msg["content"].as_str() {
            assert!(
                !content.contains("This should not appear"),
                "summary should not be injected when history fits"
            );
        }
    }
}

// -- Output token reservation --

#[test]
fn test_output_token_reservation_default() {
    // Default ContextManager deducts 16K output tokens from the 128K context window.
    // The effective input budget is ~112K tokens.
    let dir = TempDir::new().unwrap();
    // Use with_output_tokens to set high history turn limit so truncation is purely token-based.
    let cm = ContextManager::with_output_tokens(128_000, dir.path().join("ctx"), 8, 500, 16_384);

    let system = "system prompt";
    // Each message ~254 tokens (1000 chars / 4 + 4 overhead). 450 messages = ~114,300 tokens.
    let history: Vec<serde_json::Value> = (0..450)
        .map(|i| serde_json::json!({"role": "user", "content": format!("{}: {}", i, "x".repeat(996))}))
        .collect();

    let msgs = cm.build_messages(system, &history, None);
    // With output token reservation (budget = 128K - 16K = 111,616), not all 450 fit
    assert!(
        msgs.len() < 451,
        "should have truncated some messages due to output token reservation, got {}",
        msgs.len()
    );
    // But with an explicit budget of 128K (no deduction), all should fit
    let msgs_full = cm.build_messages(system, &history, Some(128_000));
    assert_eq!(
        msgs_full.len(),
        451, // system + 450 history
        "with explicit full budget, all messages should fit"
    );
}

#[test]
fn test_with_output_tokens_constructor() {
    // Verify the with_output_tokens constructor allows custom output reservation.
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::with_output_tokens(1000, dir.path().join("ctx"), 2, 40, 500);

    // System prompt ~4 tokens, remaining budget = 1000 - 500 - 4 = ~496 tokens
    let system = "sys";
    // Each message ~26 tokens (100 chars / 4 + 4 overhead)
    let history: Vec<serde_json::Value> = (0..30)
        .map(|i| serde_json::json!({"role": "user", "content": format!("{i}: {}", "a".repeat(88))}))
        .collect();

    let msgs = cm.build_messages(system, &history, None);
    // With 496 token budget after system, at ~26 tokens each, should fit ~19 messages
    assert!(
        msgs.len() < 31,
        "should have truncated some messages, got {}",
        msgs.len()
    );
    assert!(
        msgs.len() > 3,
        "should still include some messages, got {}",
        msgs.len()
    );
}

#[test]
fn test_explicit_budget_overrides_output_reservation() {
    // When budget_tokens is explicitly provided, it should be used as-is
    // (not reduced by output_tokens).
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::with_output_tokens(100_000, dir.path().join("ctx"), 2, 40, 50_000);

    let system = "sys";
    let history = vec![serde_json::json!({"role": "user", "content": "hello"})];

    // With explicit budget of 100_000, messages should fit easily
    let msgs = cm.build_messages(system, &history, Some(100_000));
    assert_eq!(msgs.len(), 2, "system + 1 history message");
}

// -- Microcompact tests --

fn make_tool_history(tool_name: &str, count: usize) -> Vec<serde_json::Value> {
    let mut history = Vec::new();
    for i in 0..count {
        // Assistant message with tool_calls
        history.push(serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": format!("call-{tool_name}-{i}"),
                "type": "function",
                "function": {"name": tool_name, "arguments": "{}"}
            }]
        }));
        // Tool result
        history.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": format!("call-{tool_name}-{i}"),
            "name": tool_name,
            "content": format!("result for {tool_name} call {i}")
        }));
    }
    history
}

#[test]
fn test_microcompact_clears_old_results() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("file_read", 5);
    let cleared = cm.microcompact(&mut history, 2, None);
    assert!(cleared > 0, "should have cleared some results");

    // Last 2 tool results should still have their content
    let tool_msgs: Vec<_> = history
        .iter()
        .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("tool"))
        .collect();
    let last_two_content: Vec<_> = tool_msgs
        .iter()
        .rev()
        .take(2)
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    for c in &last_two_content {
        assert!(
            !c.contains("cleared"),
            "recent results should not be cleared"
        );
    }
}

#[test]
fn test_microcompact_respects_keep_recent() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("execute_bash", 4);
    let cleared = cm.microcompact(&mut history, 3, None);
    // 4 results, keep 3, so 1 should be cleared
    assert_eq!(cleared, 1);
}

#[test]
fn test_microcompact_ignores_non_compactable_tools() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    // "send_message" is not in COMPACTABLE_TOOLS
    let mut history = make_tool_history("send_message", 10);
    let cleared = cm.microcompact(&mut history, 3, None);
    assert_eq!(cleared, 0, "non-compactable tools should not be cleared");
}

#[test]
fn test_microcompact_identifies_compactable_tools() {
    for tool in ContextManager::COMPACTABLE_TOOLS {
        let dir = TempDir::new().unwrap();
        let cm = ContextManager::new(128_000, dir.path().join("ctx"));
        let mut history = make_tool_history(tool, 5);
        let cleared = cm.microcompact(&mut history, 2, None);
        assert!(
            cleared > 0,
            "tool '{}' should be compactable but was not cleared",
            tool
        );
    }
}

#[test]
fn test_microcompact_time_based_trigger() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("file_read", 4);
    // Add an assistant message at the end
    history.push(serde_json::json!({"role": "assistant", "content": "Done processing."}));

    // With a time gap > 60min, should clear more aggressively
    let cleared = cm.microcompact(&mut history, 2, Some(90));
    assert!(cleared > 0, "time-based trigger should clear results");
}

#[test]
fn test_microcompact_no_time_trigger_below_threshold() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("file_read", 3);
    // 3 results with keep_recent=3 should not be cleared even with time gap
    let cleared = cm.microcompact(&mut history, 3, Some(30));
    assert_eq!(cleared, 0, "below count threshold, no clearing");
}

#[test]
fn test_microcompact_already_cleared_not_double_counted() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("file_read", 5);
    let cleared1 = cm.microcompact(&mut history, 2, None);
    assert!(cleared1 > 0);
    // Run again — already-cleared results should not be counted
    let cleared2 = cm.microcompact(&mut history, 2, None);
    assert_eq!(cleared2, 0, "second pass should not clear anything new");
}

#[test]
fn test_microcompact_placeholder_text() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("file_read", 4);
    cm.microcompact(&mut history, 1, None);

    // Check that cleared messages have the placeholder
    let cleared_msgs: Vec<_> = history
        .iter()
        .filter(|m| {
            m.get("content")
                .and_then(|v| v.as_str())
                .map(|c| c.contains("Content cleared"))
                .unwrap_or(false)
        })
        .collect();
    assert!(!cleared_msgs.is_empty());
    for msg in &cleared_msgs {
        assert!(msg["content"].as_str().unwrap().contains("search_tools"));
    }
}

#[test]
fn test_microcompact_mixed_tools() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = Vec::new();
    history.extend(make_tool_history("file_read", 4));
    history.extend(make_tool_history("execute_bash", 4));
    history.extend(make_tool_history("send_message", 4)); // not compactable

    let cleared = cm.microcompact(&mut history, 2, None);
    // file_read: 4 - 2 = 2 cleared. execute_bash: 4 - 2 = 2 cleared. send_message: 0 cleared.
    assert_eq!(cleared, 4, "should clear 2 from each compactable tool");
}

#[test]
fn test_microcompact_empty_history() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));
    let mut history: Vec<serde_json::Value> = vec![];
    let cleared = cm.microcompact(&mut history, 3, None);
    assert_eq!(cleared, 0);
}

#[test]
fn test_microcompact_count_based_only_one_tool() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("web_fetch", 6);
    let cleared = cm.microcompact(&mut history, 3, None);
    assert_eq!(cleared, 3, "6 results - keep 3 = clear 3");
}

#[test]
fn test_compactable_tools_list() {
    // Verify the expected tools are in the list
    assert!(ContextManager::COMPACTABLE_TOOLS.contains(&"file_read"));
    assert!(ContextManager::COMPACTABLE_TOOLS.contains(&"execute_bash"));
    assert!(ContextManager::COMPACTABLE_TOOLS.contains(&"web_fetch"));
    // memory_search removed from compactable tools — its results should persist
    assert!(!ContextManager::COMPACTABLE_TOOLS.contains(&"memory_search"));
    assert!(ContextManager::COMPACTABLE_TOOLS.contains(&"x402_call"));
    assert!(ContextManager::COMPACTABLE_TOOLS.contains(&"file_write"));
    assert!(ContextManager::COMPACTABLE_TOOLS.contains(&"file_search"));
    // These should NOT be compactable
    assert!(!ContextManager::COMPACTABLE_TOOLS.contains(&"send_message"));
    assert!(!ContextManager::COMPACTABLE_TOOLS.contains(&"wallet_balance"));
}

#[test]
fn test_microcompact_keep_recent_zero() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = make_tool_history("file_read", 3);
    let cleared = cm.microcompact(&mut history, 0, None);
    assert_eq!(cleared, 3, "keep_recent=0 should clear all");
}

#[test]
fn test_microcompact_with_user_messages_interspersed() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = Vec::new();
    history.push(serde_json::json!({"role": "user", "content": "read the first file"}));
    history.extend(make_tool_history("file_read", 2));
    history.push(serde_json::json!({"role": "user", "content": "now read another"}));
    history.extend(make_tool_history("file_read", 2));

    // 4 file_read results total, keep 2
    let cleared = cm.microcompact(&mut history, 2, None);
    assert_eq!(cleared, 2, "should clear 2 oldest of 4 file_read results");

    // Verify user messages are untouched
    assert_eq!(
        history[0]["content"].as_str().unwrap(),
        "read the first file"
    );
}

#[test]
fn test_microcompact_tool_without_name_field() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    // Tool result without name field — should still resolve via assistant tool_calls
    let mut history = vec![
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": "call-anon",
                "type": "function",
                "function": {"name": "file_read", "arguments": "{}"}
            }]
        }),
        serde_json::json!({
            "role": "tool",
            "tool_call_id": "call-anon",
            "content": "file content here"
        }),
    ];
    // Add more to exceed keep threshold
    history.extend(make_tool_history("file_read", 3));

    let cleared = cm.microcompact(&mut history, 2, None);
    assert!(
        cleared > 0,
        "should resolve tool name from assistant and clear"
    );
}

#[test]
fn test_microcompact_default_keep_recent_constant() {
    assert_eq!(ContextManager::MICROCOMPACT_KEEP_RECENT, 3);
}

#[test]
fn test_microcompact_placeholder_constant() {
    assert!(ContextManager::MICROCOMPACT_PLACEHOLDER.contains("Content cleared"));
    assert!(ContextManager::MICROCOMPACT_PLACEHOLDER.contains("search_tools"));
    assert!(ContextManager::MICROCOMPACT_PLACEHOLDER.contains("file_read"));
}

#[test]
fn test_microcompact_large_history_performance() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    // 50 tool calls — should handle without issue
    let mut history = make_tool_history("execute_bash", 50);
    let cleared = cm.microcompact(&mut history, 5, None);
    assert_eq!(cleared, 45, "50 - 5 = 45 cleared");
}
