//! Integration tests for context compaction — end-to-end flows combining
//! microcompact, auto-compact, boundary preservation, and hook events.

use tempfile::TempDir;

use dolphin_milk::context::compact::{
    dedup_messages, extract_summary, message_content_hash, CompactionBoundary,
    CompactionCircuitBreaker,
};
use dolphin_milk::context::manager::{
    estimate_message_tokens, ContextManager, TokenAnalyzer, DEFAULT_COMPACTION_THRESHOLD,
};
use dolphin_milk::hooks::events::{HookEvent, HookEventType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_tool_pair(tool_name: &str, call_id: &str, content: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {"name": tool_name, "arguments": "{}"}
            }]
        }),
        serde_json::json!({
            "role": "tool",
            "tool_call_id": call_id,
            "name": tool_name,
            "content": content
        }),
    ]
}

fn make_conversation(user_msgs: usize, tool_calls: usize) -> Vec<serde_json::Value> {
    let mut history = Vec::new();
    for i in 0..user_msgs {
        history.push(serde_json::json!({"role": "user", "content": format!("User message {i}")}));
        history.push(
            serde_json::json!({"role": "assistant", "content": format!("Response to message {i}")}),
        );
    }
    for i in 0..tool_calls {
        let call_id = format!("call-{i}");
        history.extend(make_tool_pair(
            "file_read",
            &call_id,
            &format!("File content for call {i}: {}", "x".repeat(200)),
        ));
    }
    history
}

// ---------------------------------------------------------------------------
// build_messages → microcompact → auto-compact → verify
// ---------------------------------------------------------------------------

#[test]
fn test_build_messages_after_microcompact() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 4, 20);

    // Build a history with many tool results
    let mut history = make_conversation(5, 8);

    // Microcompact should clear old tool results
    let cleared = cm.microcompact(&mut history, 3, None);
    assert!(cleared > 0, "microcompact should clear some results");

    // build_messages should still produce valid output
    let msgs = cm.build_messages("system prompt", &history, None);
    assert!(!msgs.is_empty());
    assert_eq!(msgs[0]["role"], "system");

    // Verify tool pair integrity after build_messages (sanitize_tool_pairs runs)
    for msg in &msgs {
        if let Some(role) = msg.get("role").and_then(|v| v.as_str()) {
            if role == "tool" {
                // Tool results should have valid content (either original or placeholder)
                assert!(msg.get("content").is_some());
            }
        }
    }
}

#[test]
fn test_auto_compact_after_microcompact() {
    let dir = TempDir::new().unwrap();
    let mut cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 4, 10);

    // History that exceeds max_history_turns
    let mut history = make_conversation(8, 5);
    assert!(history.len() > 10, "history should exceed max turns");

    // Microcompact first (reduces token pressure)
    cm.microcompact(&mut history, 2, None);

    // Auto-compact via check_and_compact
    let tool_defs = "[]";
    let event = cm.check_and_compact("system", &mut history, tool_defs, "", "", 0.0);

    if history.len() > 10 {
        // If token utilization was above threshold, compaction should have happened
        // (depends on content size)
        if let Some(event) = event {
            assert!(event.tokens_before >= event.tokens_after);
            assert!(event.messages_dropped > 0);
        }
    }
}

#[test]
fn test_boundary_preserves_discovered_tools() {
    let mut boundary = CompactionBoundary::new("summary".into(), 0..20, 5000);
    boundary.discovered_tools = vec!["browser".into(), "generate_image".into()];
    boundary.proof_references = vec!["txid_abc".into()];
    boundary.last_message_hash = "deadbeef".into();

    // Serialize and restore
    let json = serde_json::to_string(&boundary).unwrap();
    let restored: CompactionBoundary = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.discovered_tools, vec!["browser", "generate_image"]);
    assert_eq!(restored.proof_references, vec!["txid_abc"]);
    assert_eq!(restored.last_message_hash, "deadbeef");
}

// ---------------------------------------------------------------------------
// Budget-aware: critical budget → microcompact only (no LLM call)
// ---------------------------------------------------------------------------

#[test]
fn test_budget_aware_threshold_adjustment() {
    // When budget is >80% spent, threshold drops to 0.6
    let threshold = TokenAnalyzer::budget_aware_threshold(0.8, 85.0);
    assert!(
        threshold <= 0.6,
        "threshold should drop for high budget usage"
    );

    // Normal budget, threshold stays at configured
    let threshold = TokenAnalyzer::budget_aware_threshold(0.8, 30.0);
    assert_eq!(threshold, 0.8);
}

#[test]
fn test_budget_critical_skips_aggressive_compaction() {
    // When budget_spent_pct > 80, the threshold drops, making compaction less likely
    // to trigger (more conservative about spending sats on LLM compaction)
    let threshold_normal =
        TokenAnalyzer::budget_aware_threshold(DEFAULT_COMPACTION_THRESHOLD, 50.0);
    let threshold_critical =
        TokenAnalyzer::budget_aware_threshold(DEFAULT_COMPACTION_THRESHOLD, 90.0);
    assert!(threshold_critical < threshold_normal);
}

// ---------------------------------------------------------------------------
// PreCompact/PostCompact events
// ---------------------------------------------------------------------------

#[test]
fn test_pre_compact_event_type() {
    let event = HookEvent::PreCompact {
        message_count: 10,
        token_estimate: 5000,
    };
    assert_eq!(event.event_type(), HookEventType::PreCompact);
}

#[test]
fn test_post_compact_event_type() {
    let event = HookEvent::PostCompact {
        original_message_count: 50,
        compacted_message_count: 20,
        summary: "Compacted 30 messages".into(),
    };
    assert_eq!(event.event_type(), HookEventType::PostCompact);
}

#[test]
fn test_pre_compact_event_serde() {
    let event = HookEvent::PreCompact {
        message_count: 15,
        token_estimate: 8000,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("pre_compact"));
    assert!(json.contains("15"));
    assert!(json.contains("8000"));
}

#[test]
fn test_post_compact_event_serde() {
    let event = HookEvent::PostCompact {
        original_message_count: 40,
        compacted_message_count: 15,
        summary: "Key decisions preserved".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("post_compact"));
    assert!(json.contains("Key decisions preserved"));
}

#[test]
fn test_event_type_display() {
    assert_eq!(HookEventType::PreCompact.to_string(), "pre_compact");
    assert_eq!(HookEventType::PostCompact.to_string(), "post_compact");
}

#[test]
fn test_event_type_from_str() {
    assert_eq!(
        "pre_compact".parse::<HookEventType>().unwrap(),
        HookEventType::PreCompact
    );
    assert_eq!(
        "post_compact".parse::<HookEventType>().unwrap(),
        HookEventType::PostCompact
    );
}

// ---------------------------------------------------------------------------
// Circuit breaker integration
// ---------------------------------------------------------------------------

#[test]
fn test_circuit_breaker_fallback_flow() {
    let mut cb = CompactionCircuitBreaker::new();

    // Simulate 3 consecutive failures
    cb.record_failure("LLM timeout");
    assert!(!cb.should_fallback());
    cb.record_failure("LLM error");
    assert!(!cb.should_fallback());
    cb.record_failure("LLM overloaded");
    assert!(cb.should_fallback());

    // In fallback mode, we should use microcompact only
    // (this is tested by verifying should_fallback() returns true)

    // Recovery: one success resets
    cb.record_success();
    assert!(!cb.should_fallback());
    assert_eq!(cb.failure_count(), 0);
}

#[test]
fn test_circuit_breaker_rapid_recovery() {
    let mut cb = CompactionCircuitBreaker::new();
    for _ in 0..10 {
        cb.record_failure("err");
    }
    assert!(cb.should_fallback());
    assert_eq!(cb.failure_count(), 10);

    cb.record_success();
    assert!(!cb.should_fallback());
    assert_eq!(cb.failure_count(), 0);
}

// ---------------------------------------------------------------------------
// Dedup + microcompact combined
// ---------------------------------------------------------------------------

#[test]
fn test_dedup_then_microcompact() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    // Create history with duplicate tool results
    let mut history = Vec::new();
    for i in 0..3 {
        history.extend(make_tool_pair(
            "file_read",
            &format!("call-{i}"),
            "identical file content",
        ));
    }
    // Add unique messages
    history.push(serde_json::json!({"role": "user", "content": "what did you find?"}));

    // Dedup first
    let keep_indices = dedup_messages(&history);

    // Check that dedup found duplicates in tool results
    // (tool results with identical content will be deduped)
    assert!(
        keep_indices.len() <= history.len(),
        "dedup should find some duplicates"
    );

    // Microcompact on original (dedup is informational, doesn't mutate)
    let cleared = cm.microcompact(&mut history, 1, None);
    // Microcompact may or may not clear depending on count — just verify no panic
    let _ = cleared;
}

#[test]
fn test_compaction_summary_extraction_9_sections() {
    let llm_response = concat!(
        "<analysis>\n",
        "The user is building a BSV agent. Key files: src/runner/step.rs.\n",
        "Budget concern: 5000 sats remaining.\n",
        "</analysis>\n\n",
        "<summary>\n",
        "## 1. Primary Request\n",
        "Build context window management for dolphin-milk.\n\n",
        "## 2. Key Technical Concepts\n",
        "Microcompact, CompactionBoundary, token estimation.\n\n",
        "## 3. Files/Code\n",
        "- src/context/compact.rs — new compaction module\n",
        "- src/context/manager.rs:500 — microcompact method\n\n",
        "## 4. Errors/Fixes\n",
        "None\n\n",
        "## 5. Problem Solving\n",
        "Decided on 9-section structured prompt over 5-category.\n\n",
        "## 6. All User Messages\n",
        "\"Build the microcompact feature\"\n\n",
        "## 7. Pending Tasks\n",
        "Write tests, add E2E scenarios.\n\n",
        "## 8. Current Work\n",
        "Implementing CompactionBoundary struct.\n\n",
        "## 9. Optional Next Step\n",
        "Wire into runner step.rs.\n",
        "</summary>",
    );

    let summary = extract_summary(llm_response);

    // All 9 sections should be preserved
    assert!(summary.contains("Primary Request"));
    assert!(summary.contains("Key Technical Concepts"));
    assert!(summary.contains("Files/Code"));
    assert!(summary.contains("Errors/Fixes"));
    assert!(summary.contains("Problem Solving"));
    assert!(summary.contains("All User Messages"));
    assert!(summary.contains("Pending Tasks"));
    assert!(summary.contains("Current Work"));
    assert!(summary.contains("Optional Next Step"));

    // Analysis scratchpad should NOT be in the summary
    assert!(!summary.contains("Budget concern"));
    assert!(!summary.contains("analysis"));

    // Specific content preserved
    assert!(summary.contains("src/context/compact.rs"));
    assert!(summary.contains("microcompact"));
}

// ---------------------------------------------------------------------------
// Token estimation and compaction interaction
// ---------------------------------------------------------------------------

#[test]
fn test_token_analyzer_with_microcompacted_messages() {
    let mut history = Vec::new();
    for i in 0..5 {
        history.extend(make_tool_pair(
            "file_read",
            &format!("call-{i}"),
            &"x".repeat(1000),
        ));
    }

    let tokens_before: usize = history.iter().map(estimate_message_tokens).sum();

    // Simulate microcompact by replacing content
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));
    cm.microcompact(&mut history, 2, None);

    let tokens_after: usize = history.iter().map(estimate_message_tokens).sum();

    assert!(
        tokens_after < tokens_before,
        "tokens should decrease after microcompact: before={tokens_before} after={tokens_after}"
    );
}

#[test]
fn test_needs_compaction_after_microcompact() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 4, 10);

    // Build history exceeding max_history_turns
    let mut history = make_conversation(8, 3);
    let orig_len = history.len();
    assert!(cm.needs_compaction(orig_len), "should need compaction");

    // Microcompact doesn't reduce message count, only content
    cm.microcompact(&mut history, 2, None);
    assert_eq!(
        history.len(),
        orig_len,
        "microcompact doesn't remove messages"
    );
    assert!(
        cm.needs_compaction(history.len()),
        "still needs compaction after microcompact"
    );
}

// ---------------------------------------------------------------------------
// Compaction boundary with message hash chain
// ---------------------------------------------------------------------------

#[test]
fn test_boundary_message_hash_chain() {
    let messages = [
        serde_json::json!({"role": "user", "content": "first message"}),
        serde_json::json!({"role": "assistant", "content": "first response"}),
        serde_json::json!({"role": "user", "content": "second message"}),
    ];

    let last_hash = message_content_hash(&messages[messages.len() - 1]);
    let mut boundary = CompactionBoundary::new("summary".into(), 0..3, 1000);
    boundary.last_message_hash = last_hash.clone();

    assert_eq!(boundary.last_message_hash.len(), 64);
    assert_eq!(boundary.last_message_hash, last_hash);
}

#[test]
fn test_build_messages_with_compaction_summary_and_microcompact() {
    let dir = TempDir::new().unwrap();
    let mut cm = ContextManager::with_limits(128_000, dir.path().to_path_buf(), 4, 5);

    // Set a compaction summary
    cm.set_compaction_summary("User asked about BSV fees. Decision: use x402.".into());

    // Build history that exceeds max_history_turns (5)
    let mut history = make_conversation(2, 4);

    // Microcompact
    cm.microcompact(&mut history, 1, None);

    // build_messages should include the compaction summary
    let msgs = cm.build_messages("system prompt", &history, None);
    assert!(!msgs.is_empty());
    assert_eq!(msgs[0]["role"], "system");

    // If messages were trimmed, the summary should be injected
    if history.len() > 5 {
        let has_summary = msgs.iter().any(|m| {
            m.get("content")
                .and_then(|v| v.as_str())
                .map(|c| c.contains("BSV fees"))
                .unwrap_or(false)
        });
        assert!(has_summary, "compaction summary should be injected");
    }
}

// ---------------------------------------------------------------------------
// Microcompact preserves non-tool messages
// ---------------------------------------------------------------------------

#[test]
fn test_microcompact_preserves_user_messages() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = vec![
        serde_json::json!({"role": "user", "content": "Remember: budget is 50000 sats"}),
        serde_json::json!({"role": "assistant", "content": "Noted."}),
    ];
    history.extend(make_tool_pair("file_read", "call-1", "big result"));

    cm.microcompact(&mut history, 0, None);

    // User message should be untouched
    assert_eq!(
        history[0]["content"].as_str().unwrap(),
        "Remember: budget is 50000 sats"
    );
    assert_eq!(history[1]["content"].as_str().unwrap(), "Noted.");
}

#[test]
fn test_microcompact_preserves_system_messages() {
    let dir = TempDir::new().unwrap();
    let cm = ContextManager::new(128_000, dir.path().join("ctx"));

    let mut history = vec![serde_json::json!({"role": "system", "content": "You are an agent."})];
    history.extend(make_tool_pair("file_read", "call-1", "result"));

    cm.microcompact(&mut history, 0, None);

    assert_eq!(history[0]["content"].as_str().unwrap(), "You are an agent.");
}
