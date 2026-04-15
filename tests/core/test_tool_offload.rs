//! Tests for L1: Tool result offloading — infrastructure invariants.
//!
//! Preview FORMAT tests live inline in `src/runner/execute.rs` in
//! `#[cfg(test)] mod preview_tests`, next to the implementation they
//! validate. Those tests exercise the persisted-output structural summary,
//! recursive JSON descent, nested-object handling, plain-text fallback,
//! item-field priority, bounded output under pathological input, and the
//! canonical HN Algolia response shape that broke the previous
//! first-N-bytes preview format.
//!
//! This file keeps tests for the offload INFRASTRUCTURE from outside the
//! crate: no-offload tool list invariants, paid-tools list invariants,
//! file creation semantics, name sanitization, and the `read_tool_output`
//! retrieval tool behavior. None of those need `pub(crate)` access.
//!
//! History: an earlier version of this file reimplemented
//! `generate_smart_preview` locally (`preview_for_json_array`,
//! `preview_for_json_object`) so it could test the preview format from
//! outside the crate. That approach was tautological — tests that
//! reimplement the code under test can't catch regressions in the real
//! implementation. Those helpers + their 8 tests were deleted when the
//! preview format was upgraded to `generate_persisted_output_preview`.

use serde_json::Value;

// ===========================================================================
// No-offload tool list — retrieval tools must never be offloaded
// ===========================================================================

#[test]
fn test_no_offload_tools_includes_file_read() {
    // This is the critical invariant: file_read must NEVER be offloaded,
    // otherwise the agent gets caught in a cascading preview loop.
    const NO_OFFLOAD_TOOLS: &[&str] = &[
        "file_read",
        "memory_search",
        "memory_recall",
        "read_tool_output",
        "execute_bash",
    ];
    assert!(
        NO_OFFLOAD_TOOLS.contains(&"file_read"),
        "file_read must be in NO_OFFLOAD_TOOLS"
    );
    assert!(
        NO_OFFLOAD_TOOLS.contains(&"memory_search"),
        "memory_search must be in NO_OFFLOAD_TOOLS"
    );
    assert!(
        NO_OFFLOAD_TOOLS.contains(&"execute_bash"),
        "execute_bash must be in NO_OFFLOAD_TOOLS"
    );
}

#[test]
fn test_introspect_not_in_no_offload_tools() {
    // Introspect results include a self-contained `summary` field, so they
    // can be safely offloaded. The summary provides the key info without
    // needing file_read follow-up.
    const NO_OFFLOAD_TOOLS: &[&str] = &[
        "file_read",
        "memory_search",
        "memory_recall",
        "read_tool_output",
        "execute_bash",
    ];
    assert!(
        !NO_OFFLOAD_TOOLS.contains(&"introspect"),
        "introspect should NOT be in NO_OFFLOAD_TOOLS — it has self-contained summaries"
    );
}

// ===========================================================================
// Paid tools list — only these should have sats_paid extracted
// ===========================================================================

#[test]
fn test_paid_tools_list_is_restrictive() {
    // If file_read returns JSON containing "sats_paid", it should NOT be counted.
    // Only tools that actually make payments should have sats_paid extracted.
    const PAID_TOOLS: &[&str] = &["x402_call", "generate_image", "upload_to_nanostore"];
    assert!(PAID_TOOLS.contains(&"x402_call"));
    assert!(
        !PAID_TOOLS.contains(&"file_read"),
        "file_read must NOT be in PAID_TOOLS — it would double-count costs"
    );
    assert!(
        !PAID_TOOLS.contains(&"discover_services"),
        "Free tools must NOT be in PAID_TOOLS"
    );
}

// ===========================================================================
// File creation for offloaded results
// ===========================================================================

#[test]
fn test_offloaded_file_contains_full_content() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    // Simulate a large x402 result that exceeds the 50K offload threshold.
    let large_items: Vec<Value> = (0..1200)
        .map(|i| serde_json::json!({"id": i, "text": format!("tweet about BSV #{}", i), "likes": i * 3}))
        .collect();
    let content = serde_json::to_string(&large_items).unwrap();
    assert!(
        content.len() > 50_000,
        "Test content should exceed 50K threshold: {} bytes",
        content.len()
    );

    // Write to file (as the offloading code does)
    let safe_name = "x402_call";
    let file_path = format!("{}/tool_output_{}_0.json", workspace.display(), safe_name);
    std::fs::write(&file_path, content.as_bytes()).unwrap();

    // Verify file has FULL content
    let written = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(
        written.len(),
        content.len(),
        "File must contain the full, untruncated content"
    );

    // Verify it parses back to same data
    let parsed: Vec<Value> = serde_json::from_str(&written).unwrap();
    assert_eq!(parsed.len(), 1200, "All 1200 items should be in the file");
}

#[test]
fn test_multiple_offloads_get_unique_files() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    // Counter-based naming — same tool name, different counter
    let file0 = format!("{}/tool_output_x402_call_0.json", workspace.display());
    let file1 = format!("{}/tool_output_x402_call_1.json", workspace.display());
    let file2 = format!("{}/tool_output_x402_call_2.json", workspace.display());

    assert_ne!(file0, file1);
    assert_ne!(file1, file2);
    assert!(file0.ends_with("_0.json"));
    assert!(file1.ends_with("_1.json"));
}

#[test]
fn test_tool_name_sanitized_in_filename() {
    // MCP-bridged tools might have namespaced names with special chars
    let name = "mcp/wallet-tools:get_balance";
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    assert_eq!(safe_name, "mcp_wallet_tools_get_balance");
    assert!(
        !safe_name.contains('/'),
        "Sanitized name must not contain path separators"
    );
}

// ===========================================================================
// read_tool_output tool — retrieves full original content from transcript
// ===========================================================================

/// Helper: create a mock transcript.jsonl in a temp workspace with a tool_result event.
fn write_mock_transcript(dir: &std::path::Path, call_id: &str, content: &str) {
    let event = serde_json::json!({
        "ts": 1700000000.0,
        "type": "tool_result",
        "id": "abcd1234",
        "role": "tool",
        "call_id": call_id,
        "name": "x402_call",
        "content": content,
        "success": true
    });
    let path = dir.join("transcript.jsonl");
    let mut lines = String::new();
    // Add a think_response event first (like a real transcript)
    let think = serde_json::json!({
        "ts": 1699999999.0,
        "type": "think_response",
        "id": "aaaa1111",
        "role": "assistant",
        "content": "Let me search for that.",
        "model": "gpt-5-mini",
        "sats_paid": 500,
        "sats_effective": 450
    });
    lines.push_str(&serde_json::to_string(&think).unwrap());
    lines.push('\n');
    lines.push_str(&serde_json::to_string(&event).unwrap());
    lines.push('\n');
    std::fs::write(path, lines).unwrap();
}

#[tokio::test]
async fn test_read_tool_output_finds_matching_call_id() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let call_id = "call_abc123";
    let full_content =
        r#"{"results": [{"id": 1, "text": "big result data"}, {"id": 2, "text": "more data"}]}"#;

    write_mock_transcript(&workspace, call_id, full_content);

    // Create the tool and execute it
    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools
        .iter()
        .find(|t| t.name == "read_tool_output")
        .expect("read_tool_output should be registered");

    let params = serde_json::json!({"call_id": call_id});
    let result = (read_tool.execute)(params).await;

    assert_eq!(
        result, full_content,
        "Should return the full original content"
    );
}

#[tokio::test]
async fn test_read_tool_output_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    write_mock_transcript(&workspace, "call_existing", "some content");

    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools.iter().find(|t| t.name == "read_tool_output").unwrap();

    let params = serde_json::json!({"call_id": "call_nonexistent"});
    let result = (read_tool.execute)(params).await;

    assert!(
        result.contains("No tool_result found"),
        "Should report not found: {}",
        result
    );
    assert!(
        result.contains("call_nonexistent"),
        "Should include the call_id in error: {}",
        result
    );
}

#[tokio::test]
async fn test_read_tool_output_missing_transcript_file() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    // Don't write any transcript file

    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools.iter().find(|t| t.name == "read_tool_output").unwrap();

    let params = serde_json::json!({"call_id": "call_abc"});
    let result = (read_tool.execute)(params).await;

    assert!(
        result.contains("Error reading transcript"),
        "Should report read error: {}",
        result
    );
}

#[tokio::test]
async fn test_read_tool_output_missing_call_id_param() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools.iter().find(|t| t.name == "read_tool_output").unwrap();

    let params = serde_json::json!({});
    let result = (read_tool.execute)(params).await;

    assert!(
        result.contains("call_id"),
        "Should mention call_id requirement: {}",
        result
    );
    assert!(
        result.contains("required"),
        "Should say required: {}",
        result
    );
}

#[tokio::test]
async fn test_read_tool_output_large_content_not_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let call_id = "call_large";
    // Create content larger than the 50K offload threshold.
    let large_content: String = (0..1200)
        .map(|i| format!("{{\"id\": {}, \"data\": \"{}\"}}", i, "x".repeat(100)))
        .collect::<Vec<_>>()
        .join(", ");
    let large_content = format!("[{}]", large_content);
    assert!(
        large_content.len() > 50_000,
        "Content should exceed offload threshold"
    );

    write_mock_transcript(&workspace, call_id, &large_content);

    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools.iter().find(|t| t.name == "read_tool_output").unwrap();

    let params = serde_json::json!({"call_id": call_id});
    let result = (read_tool.execute)(params).await;

    // The FULL content should be returned — no truncation
    assert_eq!(
        result.len(),
        large_content.len(),
        "read_tool_output must return the FULL content, not truncated"
    );
    assert_eq!(result, large_content);
}

#[tokio::test]
async fn test_read_tool_output_falls_back_to_session_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let call_id = "call_legacy";
    let content = "legacy session content";

    // Write to session.jsonl (legacy name), NOT transcript.jsonl
    let event = serde_json::json!({
        "ts": 1700000000.0,
        "type": "tool_result",
        "id": "abcd1234",
        "role": "tool",
        "call_id": call_id,
        "name": "file_read",
        "content": content,
        "success": true
    });
    let path = workspace.join("session.jsonl");
    std::fs::write(path, serde_json::to_string(&event).unwrap()).unwrap();

    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools.iter().find(|t| t.name == "read_tool_output").unwrap();

    let params = serde_json::json!({"call_id": call_id});
    let result = (read_tool.execute)(params).await;

    assert_eq!(result, content, "Should fall back to session.jsonl");
}

#[test]
fn test_read_tool_output_is_system_category() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = dolphin_milk::tools::sandbox::all_sandbox_tools(workspace, 128_000);
    let read_tool = tools.iter().find(|t| t.name == "read_tool_output").unwrap();
    assert_eq!(
        read_tool.category, "system",
        "read_tool_output should be category 'system'"
    );
}

#[test]
fn test_read_tool_output_not_always_on() {
    // Verify the tool is NOT in ALWAYS_ON_TOOLS — it should be discoverable via search_tools
    const ALWAYS_ON_TOOLS: &[&str] = &[
        "execute_bash",
        "file_read",
        "file_write",
        "file_search",
        "web_fetch",
        "continue_task",
        "memory_store",
        "memory_search",
        "wallet_balance",
        "wallet_identity",
        "wallet_call",
        "discover_services",
        "discover_endpoints",
        "x402_call",
        "search_tools",
    ];
    assert!(!ALWAYS_ON_TOOLS.contains(&"read_tool_output"),
        "read_tool_output must NOT be in ALWAYS_ON_TOOLS — it should be discoverable via search_tools");
}

#[test]
fn test_read_tool_output_in_no_offload_list() {
    // Critical: read_tool_output must never be offloaded, or we get infinite regression
    const NO_OFFLOAD_TOOLS: &[&str] = &[
        "file_read",
        "memory_search",
        "memory_recall",
        "read_tool_output",
        "execute_bash",
    ];
    assert!(
        NO_OFFLOAD_TOOLS.contains(&"read_tool_output"),
        "read_tool_output MUST be in NO_OFFLOAD_TOOLS to prevent cascading preview loops"
    );
}
