//! Tests for runner lifecycle types, factory functions, and basic DmLoop construction.
//!
//! Covers LoopState sub-structs, ContinuationState serialization, content_hash(),
//! ReplyObligation, ConversationChainBreak, create_loop(), DmLoop.run() with
//! cancellation, and runner constants.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dolphin_milk::config::DmConfig;
use dolphin_milk::runner::{
    content_hash, create_loop, create_loop_with_rate_limiter, AuthState, BudgetState,
    CommunicationState, ContinuationState, ConversationChainBreak, ExecutionState, LoopState,
    OnChainState, ReplyObligation, StorageState, MAX_INTENT_NUDGES, TOOL_INTENT_NUDGE,
};
use dolphin_milk::x402::circuit_breaker::CircuitBreakerRegistry;

fn test_wallet() -> std::sync::Arc<dyn dolphin_milk::wallet::WalletBackend + Send + Sync> {
    std::sync::Arc::new(dolphin_milk::wallet::HttpWalletClient::new(
        "http://localhost:3322",
        "http://localhost",
        30,
    ))
}

// ---------------------------------------------------------------------------
// 1. ExecutionState defaults
// ---------------------------------------------------------------------------

#[test]
fn execution_state_default_iteration_zero() {
    let s = ExecutionState::default();
    assert_eq!(s.iteration, 0);
}

#[test]
fn execution_state_default_done_false() {
    let s = ExecutionState::default();
    assert!(!s.done);
}

#[test]
fn execution_state_default_result_empty() {
    let s = ExecutionState::default();
    assert!(s.result.is_empty());
}

#[test]
fn execution_state_default_error_empty() {
    let s = ExecutionState::default();
    assert!(s.error.is_empty());
}

#[test]
fn execution_state_mutation() {
    let s = ExecutionState {
        iteration: 5,
        done: true,
        result: "answer".to_string(),
        error: "oops".to_string(),
    };
    assert_eq!(s.iteration, 5);
    assert!(s.done);
    assert_eq!(s.result, "answer");
    assert_eq!(s.error, "oops");
}

// ---------------------------------------------------------------------------
// 2. BudgetState defaults
// ---------------------------------------------------------------------------

#[test]
fn budget_state_default_sats_spent_zero() {
    let s = BudgetState::default();
    assert_eq!(s.sats_spent, 0);
}

#[test]
fn budget_state_default_cached_balance_zero() {
    let s = BudgetState::default();
    assert_eq!(s.cached_balance, 0);
}

#[test]
fn budget_state_default_over_budget_false() {
    let s = BudgetState::default();
    assert!(!s.over_budget);
}

// ---------------------------------------------------------------------------
// 3. CommunicationState defaults
// ---------------------------------------------------------------------------

#[test]
fn communication_state_default_pending_replies_empty() {
    let s = CommunicationState::default();
    assert!(s.pending_replies.is_empty());
}

#[test]
fn communication_state_default_nudge_count_zero() {
    let s = CommunicationState::default();
    assert_eq!(s.nudge_count, 0);
}

// ---------------------------------------------------------------------------
// 4. OnChainState defaults
// ---------------------------------------------------------------------------

#[test]
fn onchain_state_default_last_proof_hash_none() {
    let s = OnChainState::default();
    assert!(s.last_proof_hash.is_none());
}

#[test]
fn onchain_state_default_last_checkpoint_none() {
    let s = OnChainState::default();
    assert!(s.last_checkpoint.is_none());
}

#[test]
fn onchain_state_default_task_commitment_none() {
    let s = OnChainState::default();
    assert!(s.task_commitment.is_none());
}

#[test]
fn onchain_state_default_budget_allocation_none() {
    let s = OnChainState::default();
    assert!(s.budget_allocation.is_none());
}

#[test]
fn onchain_state_default_capability_declaration_none() {
    let s = OnChainState::default();
    assert!(s.capability_declaration.is_none());
}

#[test]
fn onchain_state_default_conversation_chain_verified_none() {
    let s = OnChainState::default();
    assert!(s.conversation_chain_verified.is_none());
}

#[test]
fn onchain_state_default_conversation_chain_break_none() {
    let s = OnChainState::default();
    assert!(s.conversation_chain_break.is_none());
}

#[test]
fn onchain_state_default_basket_health_empty() {
    let s = OnChainState::default();
    assert!(s.basket_health.is_empty());
}

// ---------------------------------------------------------------------------
// 5. StorageState defaults
// ---------------------------------------------------------------------------

#[test]
fn storage_state_default_task_empty() {
    let s = StorageState::default();
    assert!(s.task.is_empty());
}

#[test]
fn storage_state_default_task_id_empty() {
    let s = StorageState::default();
    assert!(s.task_id.is_empty());
}

#[test]
fn storage_state_default_offloaded_results_empty() {
    let s = StorageState::default();
    assert!(s.offloaded_results.is_empty());
}

#[test]
fn storage_state_default_offload_counter_zero() {
    let s = StorageState::default();
    assert_eq!(s.offload_counter, 0);
}

// ---------------------------------------------------------------------------
// 6. AuthState defaults
// ---------------------------------------------------------------------------

#[test]
fn auth_state_default_external_origin_false() {
    let s = AuthState::default();
    assert!(!s.external_origin);
}

#[test]
fn auth_state_default_capabilities_none() {
    let s = AuthState::default();
    assert!(s.capabilities.is_none());
}

#[test]
fn auth_state_default_cert_hash_none() {
    let s = AuthState::default();
    assert!(s.cert_hash.is_none());
}

#[test]
fn auth_state_default_cert_type_none() {
    let s = AuthState::default();
    assert!(s.cert_type.is_none());
}

// ---------------------------------------------------------------------------
// 7. LoopState composite defaults
// ---------------------------------------------------------------------------

#[test]
fn loop_state_default_all_sub_structs_at_defaults() {
    let ls = LoopState::default();
    // Spot-check one field from each sub-struct
    assert_eq!(ls.exec.iteration, 0);
    assert_eq!(ls.budget.sats_spent, 0);
    assert!(ls.comms.pending_replies.is_empty());
    assert!(ls.onchain.last_proof_hash.is_none());
    assert!(ls.storage.task.is_empty());
    assert!(!ls.auth.external_origin);
}

// ---------------------------------------------------------------------------
// 8. ReplyObligation
// ---------------------------------------------------------------------------

const SENDER_KEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

#[test]
fn reply_obligation_construction() {
    let ob = ReplyObligation {
        sender_key: SENDER_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    };
    assert_eq!(ob.sender_key, SENDER_KEY);
    assert_eq!(ob.message_box, "status_inbox");
}

#[test]
fn reply_obligation_clone_preserves_fields() {
    let ob = ReplyObligation {
        sender_key: SENDER_KEY.to_string(),
        message_box: "general_inbox".to_string(),
    };
    let cloned = ob.clone();
    assert_eq!(cloned.sender_key, ob.sender_key);
    assert_eq!(cloned.message_box, ob.message_box);
}

// ---------------------------------------------------------------------------
// 9. ConversationChainBreak
// ---------------------------------------------------------------------------

#[test]
fn conversation_chain_break_construction() {
    let brk = ConversationChainBreak {
        conversation_id: "conv-123".to_string(),
        first_break_seq: 5,
        expected_hash: "aabb".to_string(),
        actual_hash: "ccdd".to_string(),
        total_breaks: 2,
    };
    assert_eq!(brk.conversation_id, "conv-123");
    assert_eq!(brk.first_break_seq, 5);
    assert_eq!(brk.expected_hash, "aabb");
    assert_eq!(brk.actual_hash, "ccdd");
    assert_eq!(brk.total_breaks, 2);
}

#[test]
fn conversation_chain_break_clone() {
    let brk = ConversationChainBreak {
        conversation_id: "conv-abc".to_string(),
        first_break_seq: 1,
        expected_hash: "aa".to_string(),
        actual_hash: "bb".to_string(),
        total_breaks: 1,
    };
    let cloned = brk.clone();
    assert_eq!(cloned.conversation_id, brk.conversation_id);
    assert_eq!(cloned.total_breaks, brk.total_breaks);
}

// ---------------------------------------------------------------------------
// 10. ContinuationState serialization
// ---------------------------------------------------------------------------

#[test]
fn continuation_state_round_trip_with_wake_at() {
    let state = ContinuationState {
        id: "cont-1".to_string(),
        task: "research BSV".to_string(),
        transcript_path: std::path::PathBuf::from("/tmp/session.jsonl"),
        iteration: 3,
        reason: "waiting for data".to_string(),
        wake_at: Some("2026-03-26T12:00:00Z".to_string()),
        created_at: "2026-03-26T11:00:00Z".to_string(),
        last_proof_hash: Some("abcd1234".to_string()),
    };
    let json = serde_json::to_string(&state).unwrap();
    let deserialized: ContinuationState = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, "cont-1");
    assert_eq!(deserialized.task, "research BSV");
    assert_eq!(
        deserialized.transcript_path.to_str().unwrap(),
        "/tmp/session.jsonl"
    );
    assert_eq!(deserialized.iteration, 3);
    assert_eq!(deserialized.reason, "waiting for data");
    assert_eq!(
        deserialized.wake_at.as_deref(),
        Some("2026-03-26T12:00:00Z")
    );
    assert_eq!(deserialized.created_at, "2026-03-26T11:00:00Z");
    assert_eq!(deserialized.last_proof_hash.as_deref(), Some("abcd1234"));
}

#[test]
fn continuation_state_round_trip_without_wake_at() {
    let state = ContinuationState {
        id: "cont-2".to_string(),
        task: "summarize findings".to_string(),
        transcript_path: std::path::PathBuf::from("/workspace/session.jsonl"),
        iteration: 0,
        reason: "immediate resume".to_string(),
        wake_at: None,
        created_at: "2026-03-26T10:00:00Z".to_string(),
        last_proof_hash: None,
    };
    let json = serde_json::to_string(&state).unwrap();
    let deserialized: ContinuationState = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, "cont-2");
    assert!(deserialized.wake_at.is_none());
    assert!(deserialized.last_proof_hash.is_none());
}

#[test]
fn continuation_state_skip_serializing_none_last_proof_hash() {
    let state = ContinuationState {
        id: "cont-3".to_string(),
        task: "test".to_string(),
        transcript_path: std::path::PathBuf::from("/tmp/test.jsonl"),
        iteration: 1,
        reason: "test".to_string(),
        wake_at: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        last_proof_hash: None,
    };
    let json = serde_json::to_string(&state).unwrap();
    // last_proof_hash has skip_serializing_if = "Option::is_none"
    assert!(!json.contains("last_proof_hash"));
}

#[test]
fn continuation_state_includes_last_proof_hash_when_some() {
    let state = ContinuationState {
        id: "cont-4".to_string(),
        task: "test".to_string(),
        transcript_path: std::path::PathBuf::from("/tmp/test.jsonl"),
        iteration: 1,
        reason: "test".to_string(),
        wake_at: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        last_proof_hash: Some("deadbeef".to_string()),
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("last_proof_hash"));
    assert!(json.contains("deadbeef"));
}

#[test]
fn continuation_state_deserialize_from_json_literal() {
    let json = r#"{
        "id": "cont-5",
        "task": "check balance",
        "transcript_path": "/data/session.jsonl",
        "iteration": 7,
        "reason": "scheduled wake",
        "wake_at": "2026-04-01T06:00:00Z",
        "created_at": "2026-03-31T23:00:00Z",
        "last_proof_hash": "ff00ff00"
    }"#;
    let state: ContinuationState = serde_json::from_str(json).unwrap();
    assert_eq!(state.id, "cont-5");
    assert_eq!(state.iteration, 7);
    assert_eq!(state.wake_at.as_deref(), Some("2026-04-01T06:00:00Z"));
    assert_eq!(state.last_proof_hash.as_deref(), Some("ff00ff00"));
}

// ---------------------------------------------------------------------------
// 11. content_hash()
// ---------------------------------------------------------------------------

#[test]
fn content_hash_deterministic() {
    let h1 = content_hash("Hello, world!");
    let h2 = content_hash("Hello, world!");
    assert_eq!(h1, h2);
}

#[test]
fn content_hash_different_inputs_different_hashes() {
    let h1 = content_hash("Hello, world!");
    let h2 = content_hash("Hello, World!");
    assert_ne!(h1, h2);
}

#[test]
fn content_hash_returns_64_char_hex() {
    let h = content_hash("test");
    assert_eq!(h.len(), 64, "SHA-256 hex digest should be 64 characters");
    assert!(
        h.chars().all(|c| c.is_ascii_hexdigit()),
        "should be hex characters only"
    );
}

#[test]
fn content_hash_empty_string() {
    let h = content_hash("");
    // SHA-256 of empty string is well-known
    assert_eq!(
        h,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn content_hash_known_value() {
    // SHA-256("abc") is well-known
    let h = content_hash("abc");
    assert_eq!(
        h,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// ---------------------------------------------------------------------------
// 12. Constants
// ---------------------------------------------------------------------------

#[test]
fn max_intent_nudges_is_two() {
    assert_eq!(MAX_INTENT_NUDGES, 2);
}

#[test]
fn tool_intent_nudge_is_non_empty() {
    assert!(!TOOL_INTENT_NUDGE.is_empty());
}

#[test]
fn tool_intent_nudge_mentions_tool_call() {
    // The nudge message should instruct the LLM to make an actual tool call
    let lower = TOOL_INTENT_NUDGE.to_lowercase();
    assert!(
        lower.contains("tool call"),
        "nudge should mention 'tool call'"
    );
}

// ---------------------------------------------------------------------------
// 13. create_loop() factory function
// ---------------------------------------------------------------------------

#[test]
fn create_loop_returns_worm_loop_with_default_state() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("task-workspace");
    let config = DmConfig::default();

    let wl = create_loop(config, workspace.clone(), None, None, test_wallet());

    // Verify initial state
    assert_eq!(wl.state.exec.iteration, 0);
    assert!(!wl.state.exec.done);
    assert_eq!(wl.state.budget.sats_spent, 0);
    assert!(wl.state.comms.pending_replies.is_empty());
    assert!(wl.prior_messages.is_none());
}

#[test]
fn create_loop_creates_workspace_directory() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("new-workspace");
    let config = DmConfig::default();

    // workspace doesn't exist yet
    assert!(!workspace.exists());

    let _wl = create_loop(config, workspace.clone(), None, None, test_wallet());

    // create_loop should create the workspace directory
    assert!(workspace.exists());
}

#[test]
fn create_loop_with_custom_memory_dir() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let memory_dir = dir.path().join("custom-memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let config = DmConfig::default();

    let _wl = create_loop(config, workspace, Some(memory_dir), None, test_wallet());
    // Should not panic — memory dir is used successfully
}

#[test]
fn create_loop_with_rate_limiter_factory() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let config = DmConfig::default();
    let circuit_breakers = Arc::new(CircuitBreakerRegistry::new());

    let wl = create_loop_with_rate_limiter(
        config,
        workspace,
        None,
        None,
        test_wallet(),
        None,
        circuit_breakers,
        None,
    );

    assert!(wl.rate_limiter.is_none());
    assert_eq!(wl.state.exec.iteration, 0);
}

// ---------------------------------------------------------------------------
// 14. DmLoop.run() — cancellation tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn worm_loop_run_immediate_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("cancel-workspace");
    let config = DmConfig::default();

    let mut wl = create_loop(config, workspace, None, None, test_wallet());

    // Set cancel flag before run
    let cancel = Arc::new(AtomicBool::new(true));
    wl.run("test task", 10, None, cancel).await;

    // Should exit quickly with 0 iterations
    assert_eq!(wl.state.exec.iteration, 0);
    assert!(wl.state.exec.done);
    assert!(wl.state.exec.error.contains("Cancelled"));
}

#[tokio::test]
async fn worm_loop_run_max_iterations_zero() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("maxiter-workspace");
    let config = DmConfig::default();

    let mut wl = create_loop(config, workspace, None, None, test_wallet());
    let cancel = Arc::new(AtomicBool::new(false));

    wl.run("test task", 0, None, cancel).await;

    // With max_iterations=0, the loop body never executes
    assert_eq!(wl.state.exec.iteration, 0);
    assert!(wl.state.exec.error.contains("max iterations"));
}

#[tokio::test]
async fn worm_loop_run_sets_task_in_state() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("task-state-workspace");
    let config = DmConfig::default();

    let mut wl = create_loop(config, workspace, None, None, test_wallet());
    let cancel = Arc::new(AtomicBool::new(true));

    wl.run("What is BSV?", 10, None, cancel).await;

    // setup_task() should set the task description in storage state
    assert_eq!(wl.state.storage.task, "What is BSV?");
}

// ---------------------------------------------------------------------------
// 15. DmLoop field access after construction
// ---------------------------------------------------------------------------

#[test]
fn worm_loop_config_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("config-workspace");
    let mut config = DmConfig::default();
    config.budget.max_per_task = 12345;

    let wl = create_loop(config, workspace, None, None, test_wallet());

    assert_eq!(wl.config.budget.max_per_task, 12345);
}

#[test]
fn worm_loop_workspace_path_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("path-workspace");
    let config = DmConfig::default();

    let wl = create_loop(config, workspace.clone(), None, None, test_wallet());

    assert_eq!(wl.workspace, workspace);
}

#[test]
fn worm_loop_prior_messages_initially_none() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("prior-workspace");
    let config = DmConfig::default();

    let wl = create_loop(config, workspace, None, None, test_wallet());

    assert!(wl.prior_messages.is_none());
}

#[test]
fn worm_loop_rate_limiter_initially_none() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("rl-workspace");
    let config = DmConfig::default();

    let wl = create_loop(config, workspace, None, None, test_wallet());

    assert!(wl.rate_limiter.is_none());
}

#[test]
fn worm_loop_metrics_initially_none() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("metrics-workspace");
    let config = DmConfig::default();

    let wl = create_loop(config, workspace, None, None, test_wallet());

    assert!(wl.metrics.is_none());
}

#[test]
fn worm_loop_approval_tools_from_config() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("approval-workspace");
    let mut config = DmConfig::default();
    config.tool_approval.require = vec!["dangerous_tool".to_string()];

    let wl = create_loop(config, workspace, None, None, test_wallet());

    assert_eq!(wl.approval_tools, vec!["dangerous_tool"]);
}

#[test]
fn worm_loop_state_is_default_after_construction() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("state-workspace");
    let config = DmConfig::default();

    let wl = create_loop(config, workspace, None, None, test_wallet());

    // Full default state verification
    assert_eq!(wl.state.exec.iteration, 0);
    assert!(!wl.state.exec.done);
    assert!(wl.state.exec.result.is_empty());
    assert!(wl.state.exec.error.is_empty());
    assert_eq!(wl.state.budget.sats_spent, 0);
    assert!(!wl.state.budget.over_budget);
    assert!(wl.state.comms.pending_replies.is_empty());
    assert_eq!(wl.state.comms.nudge_count, 0);
    assert!(wl.state.onchain.last_proof_hash.is_none());
    assert!(wl.state.storage.task.is_empty());
    assert!(!wl.state.auth.external_origin);
}
