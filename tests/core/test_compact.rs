//! Tests for context compaction — 9-section prompt, CompactionBoundary,
//! circuit breaker, and duplicate detection.

use dolphin_milk::context::compact::dedup_messages;
use dolphin_milk::context::compact::{
    extract_summary, message_content_hash, CompactionBoundary, CompactionCircuitBreaker,
    COMPACTION_PROMPT, COMPACTION_SECTIONS,
};

// ---------------------------------------------------------------------------
// 9-section prompt template
// ---------------------------------------------------------------------------

#[test]
fn test_compaction_prompt_has_9_sections() {
    assert_eq!(COMPACTION_SECTIONS.len(), 9);
}

#[test]
fn test_compaction_prompt_contains_all_section_names() {
    for section in &COMPACTION_SECTIONS {
        assert!(
            COMPACTION_PROMPT.contains(section),
            "Prompt missing section: {section}"
        );
    }
}

#[test]
fn test_compaction_prompt_has_analysis_tags() {
    assert!(COMPACTION_PROMPT.contains("<analysis>"));
    assert!(COMPACTION_PROMPT.contains("</analysis>"));
}

#[test]
fn test_compaction_prompt_has_summary_tags() {
    assert!(COMPACTION_PROMPT.contains("<summary>"));
    assert!(COMPACTION_PROMPT.contains("</summary>"));
}

#[test]
fn test_compaction_prompt_section_order() {
    let positions: Vec<usize> = COMPACTION_SECTIONS
        .iter()
        .filter_map(|s| COMPACTION_PROMPT.find(s))
        .collect();
    assert_eq!(positions.len(), 9, "all sections must be found");
    // Verify they appear in order
    for i in 1..positions.len() {
        assert!(
            positions[i] > positions[i - 1],
            "Section {} must appear after section {}",
            i + 1,
            i
        );
    }
}

#[test]
fn test_compaction_sections_list() {
    assert_eq!(COMPACTION_SECTIONS[0], "Primary Request");
    assert_eq!(COMPACTION_SECTIONS[1], "Key Technical Concepts");
    assert_eq!(COMPACTION_SECTIONS[2], "Files/Code");
    assert_eq!(COMPACTION_SECTIONS[3], "Errors/Fixes");
    assert_eq!(COMPACTION_SECTIONS[4], "Problem Solving");
    assert_eq!(COMPACTION_SECTIONS[5], "All User Messages");
    assert_eq!(COMPACTION_SECTIONS[6], "Pending Tasks");
    assert_eq!(COMPACTION_SECTIONS[7], "Current Work");
    assert_eq!(COMPACTION_SECTIONS[8], "Optional Next Step");
}

// ---------------------------------------------------------------------------
// extract_summary
// ---------------------------------------------------------------------------

#[test]
fn test_extract_summary_with_both_tags() {
    let response = "<analysis>thinking process</analysis>\n<summary>clean summary</summary>";
    assert_eq!(extract_summary(response), "clean summary");
}

#[test]
fn test_extract_summary_multiline() {
    let response =
        "<summary>\n## 1. Primary Request\nBuild it.\n\n## 2. Key Concepts\nRust.\n</summary>";
    let summary = extract_summary(response);
    assert!(summary.contains("Primary Request"));
    assert!(summary.contains("Key Concepts"));
    assert!(summary.contains("Rust"));
}

#[test]
fn test_extract_summary_analysis_only() {
    let response = "<analysis>just analysis</analysis>\nThe actual content here.";
    let summary = extract_summary(response);
    assert!(!summary.contains("just analysis"));
    assert!(summary.contains("actual content"));
}

#[test]
fn test_extract_summary_empty_summary_tags() {
    let response = "<analysis>thinking</analysis>\n<summary>\n</summary>";
    let summary = extract_summary(response);
    assert!(summary.is_empty() || summary.trim().is_empty());
}

#[test]
fn test_extract_summary_no_tags_passthrough() {
    let input = "Plain text without any tags";
    assert_eq!(extract_summary(input), input);
}

#[test]
fn test_extract_summary_nested_content() {
    let response =
        "<summary>## Files\n- `src/main.rs:42` — entry point\n- `<config>` tag in XML</summary>";
    let summary = extract_summary(response);
    assert!(summary.contains("src/main.rs:42"));
}

// ---------------------------------------------------------------------------
// CompactionBoundary serde
// ---------------------------------------------------------------------------

#[test]
fn test_boundary_serde_roundtrip() {
    let boundary = CompactionBoundary::new("test".into(), 0..5, 1000);
    let json = serde_json::to_string(&boundary).unwrap();
    let restored: CompactionBoundary = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.summary, "test");
    assert_eq!(restored.compacted_range, 0..5);
    assert_eq!(restored.token_savings, 1000);
}

#[test]
fn test_boundary_serde_with_all_fields() {
    let mut boundary = CompactionBoundary::new("full".into(), 10..50, 8000);
    boundary.preserved_context = vec!["user wants Rust".into(), "budget is 50000".into()];
    boundary.discovered_tools = vec!["browser".into(), "generate_image".into()];
    boundary.last_message_hash = "abc123def456".into();
    boundary.proof_references = vec!["txid_aaa".into(), "txid_bbb".into()];
    boundary.sats_saved = 500;

    let json = serde_json::to_string_pretty(&boundary).unwrap();
    let restored: CompactionBoundary = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.preserved_context.len(), 2);
    assert_eq!(restored.discovered_tools.len(), 2);
    assert_eq!(restored.proof_references.len(), 2);
    assert_eq!(restored.sats_saved, 500);
    assert_eq!(restored.last_message_hash, "abc123def456");
}

#[test]
fn test_boundary_empty_fields() {
    let boundary = CompactionBoundary::new(String::new(), 0..0, 0);
    let json = serde_json::to_string(&boundary).unwrap();
    let restored: CompactionBoundary = serde_json::from_str(&json).unwrap();
    assert!(restored.summary.is_empty());
    assert!(restored.discovered_tools.is_empty());
    assert_eq!(restored.compacted_range, 0..0);
}

#[test]
fn test_boundary_large_range() {
    let boundary = CompactionBoundary::new("big compaction".into(), 0..10000, 500000);
    let json = serde_json::to_string(&boundary).unwrap();
    let restored: CompactionBoundary = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.compacted_range, 0..10000);
    assert_eq!(restored.token_savings, 500000);
}

#[test]
fn test_boundary_timestamp_is_recent() {
    let boundary = CompactionBoundary::new("ts test".into(), 0..1, 100);
    let now = chrono::Utc::now();
    let diff = now - boundary.timestamp;
    assert!(diff.num_seconds() < 5, "timestamp should be recent");
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

#[test]
fn test_circuit_breaker_new() {
    let cb = CompactionCircuitBreaker::new();
    assert_eq!(cb.failure_count(), 0);
    assert!(!cb.should_fallback());
    assert!(cb.last_failure_reason().is_none());
}

#[test]
fn test_circuit_breaker_default() {
    let cb = CompactionCircuitBreaker::default();
    assert_eq!(cb.failure_count(), 0);
}

#[test]
fn test_circuit_breaker_one_failure() {
    let mut cb = CompactionCircuitBreaker::new();
    cb.record_failure("timeout");
    assert_eq!(cb.failure_count(), 1);
    assert!(!cb.should_fallback());
    assert_eq!(cb.last_failure_reason(), Some("timeout"));
}

#[test]
fn test_circuit_breaker_two_failures() {
    let mut cb = CompactionCircuitBreaker::new();
    cb.record_failure("error 1");
    cb.record_failure("error 2");
    assert_eq!(cb.failure_count(), 2);
    assert!(!cb.should_fallback());
}

#[test]
fn test_circuit_breaker_three_failures_triggers_fallback() {
    let mut cb = CompactionCircuitBreaker::new();
    cb.record_failure("a");
    cb.record_failure("b");
    cb.record_failure("c");
    assert!(cb.should_fallback());
    assert_eq!(cb.failure_count(), 3);
}

#[test]
fn test_circuit_breaker_success_resets() {
    let mut cb = CompactionCircuitBreaker::new();
    cb.record_failure("a");
    cb.record_failure("b");
    cb.record_success();
    assert_eq!(cb.failure_count(), 0);
    assert!(!cb.should_fallback());
    assert!(cb.last_failure_reason().is_none());
}

#[test]
fn test_circuit_breaker_five_failures() {
    let mut cb = CompactionCircuitBreaker::new();
    for i in 0..5 {
        cb.record_failure(&format!("err{i}"));
    }
    assert!(cb.should_fallback());
    assert_eq!(cb.failure_count(), 5);
    assert_eq!(cb.last_failure_reason(), Some("err4"));
}

#[test]
fn test_circuit_breaker_reset_after_fallback() {
    let mut cb = CompactionCircuitBreaker::new();
    cb.record_failure("a");
    cb.record_failure("b");
    cb.record_failure("c");
    assert!(cb.should_fallback());
    cb.record_success();
    assert!(!cb.should_fallback());
    assert_eq!(cb.failure_count(), 0);
}

#[test]
fn test_circuit_breaker_interleaved_success_failure() {
    let mut cb = CompactionCircuitBreaker::new();
    cb.record_failure("a");
    cb.record_failure("b");
    cb.record_success(); // reset
    cb.record_failure("c");
    assert_eq!(cb.failure_count(), 1);
    assert!(!cb.should_fallback());
}

// ---------------------------------------------------------------------------
// Duplicate detection
// ---------------------------------------------------------------------------

#[test]
fn test_dedup_no_duplicates() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": "hi"}),
        serde_json::json!({"role": "user", "content": "bye"}),
    ];
    let keep = dedup_messages(&messages);
    assert_eq!(keep, vec![0, 1, 2]);
}

#[test]
fn test_dedup_identical_user_messages() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "same question"}),
        serde_json::json!({"role": "assistant", "content": "answer"}),
        serde_json::json!({"role": "user", "content": "same question"}),
    ];
    let keep = dedup_messages(&messages);
    assert_eq!(keep, vec![0, 1]); // second "same question" deduped
}

#[test]
fn test_dedup_different_roles_same_content() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "echo"}),
        serde_json::json!({"role": "assistant", "content": "echo"}),
    ];
    let keep = dedup_messages(&messages);
    assert_eq!(keep, vec![0, 1]); // different roles = not duplicates
}

#[test]
fn test_dedup_empty_messages() {
    let keep = dedup_messages(&[]);
    assert!(keep.is_empty());
}

#[test]
fn test_dedup_tool_results() {
    let messages = vec![
        serde_json::json!({"role": "tool", "content": "identical output"}),
        serde_json::json!({"role": "tool", "content": "identical output"}),
        serde_json::json!({"role": "tool", "content": "different output"}),
    ];
    let keep = dedup_messages(&messages);
    assert_eq!(keep, vec![0, 2]);
}

#[test]
fn test_dedup_keeps_first_occurrence() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "first"}),
        serde_json::json!({"role": "user", "content": "second"}),
        serde_json::json!({"role": "user", "content": "first"}),
        serde_json::json!({"role": "user", "content": "third"}),
    ];
    let keep = dedup_messages(&messages);
    assert_eq!(keep, vec![0, 1, 3]);
}

#[test]
fn test_dedup_long_content_uses_first_500_chars() {
    let base = "a".repeat(500);
    let msg1_content = format!("{}SUFFIX1", base);
    let msg2_content = format!("{}SUFFIX2", base);
    let messages = vec![
        serde_json::json!({"role": "user", "content": msg1_content}),
        serde_json::json!({"role": "user", "content": msg2_content}),
    ];
    let keep = dedup_messages(&messages);
    // Both have same first 500 chars, so msg2 is deduped
    assert_eq!(keep, vec![0]);
}

#[test]
fn test_dedup_all_unique() {
    let messages: Vec<_> = (0..10)
        .map(|i| serde_json::json!({"role": "user", "content": format!("message {i}")}))
        .collect();
    let keep = dedup_messages(&messages);
    assert_eq!(keep.len(), 10);
}

#[test]
fn test_dedup_all_identical() {
    let messages: Vec<_> = (0..5)
        .map(|_| serde_json::json!({"role": "user", "content": "identical"}))
        .collect();
    let keep = dedup_messages(&messages);
    assert_eq!(keep, vec![0]); // only first kept
}

#[test]
fn test_dedup_missing_content_field() {
    let messages = vec![
        serde_json::json!({"role": "system"}),
        serde_json::json!({"role": "system"}),
    ];
    let keep = dedup_messages(&messages);
    // Both have empty content, same role → dedup
    assert_eq!(keep, vec![0]);
}

// ---------------------------------------------------------------------------
// message_content_hash
// ---------------------------------------------------------------------------

#[test]
fn test_message_hash_deterministic() {
    let msg = serde_json::json!({"role": "user", "content": "test"});
    let h1 = message_content_hash(&msg);
    let h2 = message_content_hash(&msg);
    assert_eq!(h1, h2);
}

#[test]
fn test_message_hash_length() {
    let msg = serde_json::json!({"role": "user", "content": "hello"});
    assert_eq!(message_content_hash(&msg).len(), 64); // SHA-256 hex = 64 chars
}

#[test]
fn test_message_hash_different_for_different_content() {
    let m1 = serde_json::json!({"role": "user", "content": "abc"});
    let m2 = serde_json::json!({"role": "user", "content": "def"});
    assert_ne!(message_content_hash(&m1), message_content_hash(&m2));
}

#[test]
fn test_message_hash_includes_role() {
    let m1 = serde_json::json!({"role": "user", "content": "same"});
    let m2 = serde_json::json!({"role": "assistant", "content": "same"});
    assert_ne!(message_content_hash(&m1), message_content_hash(&m2));
}

#[test]
fn test_message_hash_empty_content() {
    let msg = serde_json::json!({"role": "system", "content": ""});
    let hash = message_content_hash(&msg);
    assert_eq!(hash.len(), 64);
}
