//! Tests for context fork — isolated context for memory-intensive operations.

use dolphin_milk::context::fork::{is_memory_intensive_skill, ContextFork, ForkResult};
use dolphin_milk::context::prompt::{PromptContext, ToolDesc};

fn default_ctx() -> PromptContext {
    PromptContext {
        identity_key: "test-key-abc".to_string(),
        balance_sats: 5000,
        model: "test-model".to_string(),
        tools: vec![ToolDesc {
            name: "memory_search".to_string(),
            description: "Search memory".to_string(),
            category: "memory".to_string(),
            deferred: false,
            hint: None,
        }],
        memory_summary: "Original summary: agent knows about BSV".to_string(),
        available_files: vec!["README.md".to_string()],
        task: "Analyze all memories".to_string(),
        budget_remaining: 3000,
        low_power: false,
        inbox_count: 0,
        skills_section: String::new(),
        workspace_path: "/tmp/test".to_string(),
        certificate_info: None,
        has_external_messages: false,
        identity_soul: None,
        auto_recall_ids: vec!["mem-1".to_string()],
        basket_health: std::collections::HashMap::new(),
        spendable_output_count: 0,
        instructions: String::new(),
        env_snapshot: None,
        working_memory_section: String::new(),
    }
}

// -- Fork preserves original context --

#[test]
fn test_fork_preserves_original_context() {
    let ctx = default_ctx();
    let msgs = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": "hi there"}),
    ];

    let mut fork = ContextFork::new(&ctx, &msgs);

    // Mutate forked context
    fork.context.memory_summary = "HUGE memory dump with 1000 entries".to_string();
    fork.context.balance_sats = 0;
    fork.context.task = "Different task".to_string();
    fork.push_message(serde_json::json!({"role": "system", "content": "injected"}));

    // Original unchanged
    assert_eq!(
        ctx.memory_summary,
        "Original summary: agent knows about BSV"
    );
    assert_eq!(ctx.balance_sats, 5000);
    assert_eq!(ctx.task, "Analyze all memories");
    assert_eq!(msgs.len(), 2);

    // Fork mutated
    assert!(fork.context.memory_summary.contains("HUGE"));
    assert_eq!(fork.context.balance_sats, 0);
    assert_eq!(fork.message_count(), 3);
}

#[test]
fn test_fork_preserves_tools_and_files() {
    let ctx = default_ctx();
    let fork = ContextFork::new(&ctx, &[]);

    assert_eq!(fork.context.tools.len(), 1);
    assert_eq!(fork.context.tools[0].name, "memory_search");
    assert_eq!(fork.context.available_files.len(), 1);
    assert_eq!(fork.context.identity_key, "test-key-abc");
}

#[test]
fn test_fork_preserves_auto_recall_ids() {
    let ctx = default_ctx();
    let fork = ContextFork::new(&ctx, &[]);
    assert_eq!(fork.context.auto_recall_ids, vec!["mem-1".to_string()]);
}

// -- Fork results are extractable --

#[test]
fn test_fork_record_and_extract_results() {
    let ctx = default_ctx();
    let mut fork = ContextFork::new(&ctx, &[]);

    fork.record_result("recall", "Found 42 entries about BSV transactions");
    fork.record_result("summary", "Key themes: fees, proofs, micropayments");

    let results = fork.results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].label, "recall");
    assert!(results[0].content.contains("42 entries"));
    assert_eq!(results[1].label, "summary");
}

#[test]
fn test_fork_into_results() {
    let ctx = default_ctx();
    let mut fork = ContextFork::new(&ctx, &[]);

    fork.record_result("test", "data");
    fork.record_result("test2", "data2");

    let results = fork.into_results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].content, "data");
    assert_eq!(results[1].content, "data2");
}

#[test]
fn test_fork_summary_formatting() {
    let ctx = default_ctx();
    let mut fork = ContextFork::new(&ctx, &[]);

    fork.record_result("memory_dump", "42 entries found");
    fork.record_result("analysis", "BSV focus");

    let summary = fork.summary();
    assert!(summary.contains("[memory_dump]"));
    assert!(summary.contains("[analysis]"));
    assert!(summary.contains("42 entries found"));
    assert!(summary.contains("BSV focus"));
}

#[test]
fn test_fork_empty_summary() {
    let ctx = default_ctx();
    let fork = ContextFork::new(&ctx, &[]);
    assert_eq!(fork.summary(), "");
}

// -- Memory injection --

#[test]
fn test_inject_memory_summary() {
    let ctx = default_ctx();
    let mut fork = ContextFork::new(&ctx, &[]);

    let big_summary = "entry ".repeat(500);
    fork.inject_memory_summary(big_summary.clone());

    assert_eq!(fork.context.memory_summary, big_summary);
    assert_eq!(
        ctx.memory_summary,
        "Original summary: agent knows about BSV"
    );
}

// -- Memory-intensive skill detection --

#[test]
fn test_memory_intensive_skill_detected() {
    let yaml = "name: deep-recall\nmemory_intensive: true\nauto_activate: false";
    assert!(is_memory_intensive_skill(yaml));
}

#[test]
fn test_non_memory_intensive_skill() {
    let yaml = "name: simple\nauto_activate: true\ntools: [x402_call]";
    assert!(!is_memory_intensive_skill(yaml));
}

#[test]
fn test_memory_intensive_explicit_false() {
    let yaml = "name: test\nmemory_intensive: false";
    assert!(!is_memory_intensive_skill(yaml));
}

#[test]
fn test_memory_intensive_invalid_yaml() {
    assert!(!is_memory_intensive_skill("not: valid: yaml: [[["));
}

// -- Message manipulation --

#[test]
fn test_fork_messages_cloned() {
    let ctx = default_ctx();
    let msgs = vec![serde_json::json!({"role": "user", "content": "original"})];

    let fork = ContextFork::new(&ctx, &msgs);

    // Fork has same messages
    assert_eq!(fork.messages().len(), 1);
    assert_eq!(fork.messages()[0]["content"], "original");
}

#[test]
fn test_fork_push_message() {
    let ctx = default_ctx();
    let mut fork = ContextFork::new(&ctx, &[]);

    assert_eq!(fork.message_count(), 0);

    fork.push_message(serde_json::json!({"role": "system", "content": "full memory dump..."}));
    fork.push_message(serde_json::json!({"role": "user", "content": "summarize it"}));

    assert_eq!(fork.message_count(), 2);
}

// -- ForkResult struct --

#[test]
fn test_fork_result_debug() {
    let result = ForkResult {
        label: "test".to_string(),
        content: "data".to_string(),
    };
    let debug = format!("{:?}", result);
    assert!(debug.contains("test"));
    assert!(debug.contains("data"));
}

#[test]
fn test_fork_result_clone() {
    let result = ForkResult {
        label: "test".to_string(),
        content: "data".to_string(),
    };
    let cloned = result.clone();
    assert_eq!(cloned.label, "test");
    assert_eq!(cloned.content, "data");
}
