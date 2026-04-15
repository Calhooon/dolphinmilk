//! Tests for the rich message type system (session::message).
//!
//! Validates Message::from_event() conversion for all 18 event types,
//! SessionState reconstruction from message sequences, and edge cases
//! (empty transcripts, interrupted sessions, truncation detection).

use dolphin_milk::session::message::{Message, SessionState};
use dolphin_milk::session::transcript::{Transcript, TranscriptEvent};
use serde_json::Value;
use std::collections::HashMap;

// ─── Helpers ───────────────────────────────────────────────────────────

fn make_event(event_type: &str, data: Vec<(&str, Value)>) -> TranscriptEvent {
    let mut map = HashMap::new();
    for (k, v) in data {
        map.insert(k.to_string(), v);
    }
    TranscriptEvent {
        ts: 1000.0,
        event_type: event_type.to_string(),
        id: "test1234".to_string(),
        data: map,
    }
}

fn s(val: &str) -> Value {
    Value::String(val.to_string())
}

fn n(val: u64) -> Value {
    serde_json::json!(val)
}

// ─── from_event conversion tests ───────────────────────────────────────

#[test]
fn test_system_message() {
    let event = make_event(
        "system",
        vec![("content", s("hello")), ("role", s("system"))],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::System { content, ts } => {
            assert_eq!(content, "hello");
            assert_eq!(ts, 1000.0);
        }
        _ => panic!("Expected System variant"),
    }
}

#[test]
fn test_user_message() {
    let event = make_event(
        "user",
        vec![("content", s("What is 2+2?")), ("role", s("user"))],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::User { content, ts } => {
            assert_eq!(content, "What is 2+2?");
            assert_eq!(ts, 1000.0);
        }
        _ => panic!("Expected User variant"),
    }
}

#[test]
fn test_think_request() {
    let event = make_event(
        "think_request",
        vec![
            ("model", s("gpt-5-mini")),
            ("max_tokens", n(16384)),
            ("message_count", n(5)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ThinkRequest {
            model,
            max_tokens,
            message_count,
            ..
        } => {
            assert_eq!(model, "gpt-5-mini");
            assert_eq!(max_tokens, 16384);
            assert_eq!(message_count, 5);
        }
        _ => panic!("Expected ThinkRequest variant"),
    }
}

#[test]
fn test_think_response() {
    let event = make_event(
        "think_response",
        vec![
            ("content", s("4")),
            ("model", s("gpt-5-mini")),
            ("sats_paid", n(263120)),
            ("sats_effective", n(15805)),
            ("sats_refunded", n(247315)),
            ("prompt_tokens", n(7500)),
            ("completion_tokens", n(74)),
            ("finish_reason", s("stop")),
            ("duration_ms", n(24813)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ThinkResponse {
            content,
            model,
            sats_paid,
            sats_effective,
            sats_refunded,
            prompt_tokens,
            completion_tokens,
            finish_reason,
            duration_ms,
            tool_calls,
            ..
        } => {
            assert_eq!(content, "4");
            assert_eq!(model, "gpt-5-mini");
            assert_eq!(sats_paid, 263120);
            assert_eq!(sats_effective, 15805);
            assert_eq!(sats_refunded, 247315);
            assert_eq!(prompt_tokens, 7500);
            assert_eq!(completion_tokens, 74);
            assert_eq!(finish_reason, "stop");
            assert_eq!(duration_ms, 24813);
            assert!(tool_calls.is_empty());
        }
        _ => panic!("Expected ThinkResponse variant"),
    }
}

#[test]
fn test_think_response_with_tool_calls() {
    let tc = serde_json::json!([{
        "id": "call_abc",
        "type": "function",
        "function": { "name": "wallet_balance", "arguments": "{}" }
    }]);
    let event = make_event(
        "think_response",
        vec![
            ("content", s("")),
            ("model", s("gpt-5-mini")),
            ("sats_paid", n(100)),
            ("sats_effective", n(50)),
            ("sats_refunded", n(50)),
            ("prompt_tokens", n(100)),
            ("completion_tokens", n(10)),
            ("finish_reason", s("tool_calls")),
            ("duration_ms", n(1000)),
            ("tool_calls", tc),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ThinkResponse { tool_calls, .. } => {
            assert_eq!(tool_calls.len(), 1);
        }
        _ => panic!("Expected ThinkResponse variant"),
    }
}

#[test]
fn test_tool_call() {
    let args = serde_json::json!({"query": "test"});
    let event = make_event(
        "tool_call",
        vec![
            ("call_id", s("call_123")),
            ("name", s("memory_search")),
            ("arguments", args.clone()),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ToolCall {
            call_id,
            name,
            arguments,
            ..
        } => {
            assert_eq!(call_id, "call_123");
            assert_eq!(name, "memory_search");
            assert_eq!(arguments, args);
        }
        _ => panic!("Expected ToolCall variant"),
    }
}

#[test]
fn test_tool_result() {
    let event = make_event(
        "tool_result",
        vec![
            ("call_id", s("call_123")),
            ("name", s("memory_search")),
            ("content", s("Found 3 results")),
            ("success", Value::Bool(true)),
            ("sats_paid", n(0)),
            ("role", s("tool")),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ToolResult {
            call_id,
            name,
            content,
            success,
            sats_paid,
            ..
        } => {
            assert_eq!(call_id, "call_123");
            assert_eq!(name, "memory_search");
            assert_eq!(content, "Found 3 results");
            assert!(success);
            assert_eq!(sats_paid, 0);
        }
        _ => panic!("Expected ToolResult variant"),
    }
}

#[test]
fn test_budget_check() {
    let event = make_event(
        "budget_check",
        vec![
            ("balance", n(50000000)),
            ("spent_session", n(15000)),
            ("spent_hour", n(15000)),
            ("task_limit", n(20000000)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::BudgetCheck {
            balance,
            spent_session,
            spent_hour,
            task_limit,
            ..
        } => {
            assert_eq!(balance, 50000000);
            assert_eq!(spent_session, 15000);
            assert_eq!(spent_hour, 15000);
            assert_eq!(task_limit, 20000000);
        }
        _ => panic!("Expected BudgetCheck variant"),
    }
}

#[test]
fn test_session_start() {
    let event = make_event("session_start", vec![("task", s("What is 2+2?"))]);
    let msg = Message::from_event(&event);
    match msg {
        Message::SessionStart { task, .. } => {
            assert_eq!(task, "What is 2+2?");
        }
        _ => panic!("Expected SessionStart variant"),
    }
}

#[test]
fn test_session_end() {
    let event = make_event(
        "session_end",
        vec![
            ("iterations", n(3)),
            ("sats_spent", n(45000)),
            ("result", s("The answer is 4")),
            ("error", s("")),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::SessionEnd {
            iterations,
            sats_spent,
            result,
            error,
            ..
        } => {
            assert_eq!(iterations, 3);
            assert_eq!(sats_spent, 45000);
            assert_eq!(result, "The answer is 4");
            assert!(error.is_empty());
        }
        _ => panic!("Expected SessionEnd variant"),
    }
}

#[test]
fn test_error_event() {
    let event = make_event("error", vec![("error", s("Budget exceeded"))]);
    let msg = Message::from_event(&event);
    match msg {
        Message::Error { error, .. } => {
            assert_eq!(error, "Budget exceeded");
        }
        _ => panic!("Expected Error variant"),
    }
}

#[test]
fn test_proof_created() {
    let event = make_event(
        "proof_created",
        vec![
            ("txid", s("abc123")),
            ("proof_type", s("decision")),
            ("hash", s("def456")),
            ("prev_hash", s("ghi789")),
            ("basket", s("worm-proofs")),
            ("iteration", n(2)),
            ("sats_cost", n(200)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ProofCreated {
            txid,
            proof_type,
            hash,
            prev_hash,
            basket,
            iteration,
            sats_cost,
            ..
        } => {
            assert_eq!(txid, "abc123");
            assert_eq!(proof_type, "decision");
            assert_eq!(hash, "def456");
            assert_eq!(prev_hash, Some("ghi789".to_string()));
            assert_eq!(basket, Some("worm-proofs".to_string()));
            assert_eq!(iteration, Some(2));
            assert_eq!(sats_cost, Some(200));
        }
        _ => panic!("Expected ProofCreated variant"),
    }
}

#[test]
fn test_checkpoint_created() {
    let event = make_event(
        "checkpoint_created",
        vec![
            ("txid", s("ckpt_tx")),
            ("token_type", s("task_commitment")),
            ("basket", s("worm-state")),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::CheckpointCreated {
            txid,
            token_type,
            basket,
            checkpoint_data,
            ..
        } => {
            assert_eq!(txid, "ckpt_tx");
            assert_eq!(token_type, "task_commitment");
            assert_eq!(basket, "worm-state");
            assert!(checkpoint_data.is_none());
        }
        _ => panic!("Expected CheckpointCreated variant"),
    }
}

#[test]
fn test_memory_stored() {
    let event = make_event(
        "memory_stored",
        vec![
            ("memory_id", s("mem_001")),
            ("category", s("knowledge")),
            ("tags", serde_json::json!(["bsv", "fees"])),
            ("content_preview", s("BSV transaction fees are...")),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::MemoryStored {
            memory_id,
            category,
            tags,
            content_preview,
            ..
        } => {
            assert_eq!(memory_id, "mem_001");
            assert_eq!(category, "knowledge");
            assert_eq!(tags, vec!["bsv", "fees"]);
            assert_eq!(content_preview, "BSV transaction fees are...");
        }
        _ => panic!("Expected MemoryStored variant"),
    }
}

#[test]
fn test_unknown_event() {
    let event = make_event("future_event_type", vec![("key", s("value"))]);
    let msg = Message::from_event(&event);
    match msg {
        Message::Unknown {
            event_type, data, ..
        } => {
            assert_eq!(event_type, "future_event_type");
            assert_eq!(data.get("key").unwrap().as_str().unwrap(), "value");
        }
        _ => panic!("Expected Unknown variant"),
    }
}

#[test]
fn test_continuation_save() {
    let event = make_event(
        "continuation_save",
        vec![
            ("continuation_id", s("cont_001")),
            ("reason", s("waiting for approval")),
            ("wake_at", s("2026-04-08T12:00:00Z")),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ContinuationSave {
            continuation_id,
            reason,
            wake_at,
            ..
        } => {
            assert_eq!(continuation_id, "cont_001");
            assert_eq!(reason, "waiting for approval");
            assert_eq!(wake_at, Some("2026-04-08T12:00:00Z".to_string()));
        }
        _ => panic!("Expected ContinuationSave variant"),
    }
}

#[test]
fn test_continuation_resume() {
    let event = make_event(
        "continuation_resume",
        vec![
            ("continuation_id", s("cont_001")),
            ("original_task", s("research BSV fees")),
            ("paused_iteration", n(5)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ContinuationResume {
            continuation_id,
            original_task,
            paused_iteration,
            ..
        } => {
            assert_eq!(continuation_id, "cont_001");
            assert_eq!(original_task, "research BSV fees");
            assert_eq!(paused_iteration, 5);
        }
        _ => panic!("Expected ContinuationResume variant"),
    }
}

#[test]
fn test_loop_warning() {
    let event = make_event(
        "loop_warning",
        vec![("message", s("Repetitive behavior detected"))],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::LoopWarning { message, .. } => {
            assert_eq!(message, "Repetitive behavior detected");
        }
        _ => panic!("Expected LoopWarning variant"),
    }
}

#[test]
fn test_receipt_stored() {
    let event = make_event(
        "receipt_stored",
        vec![
            ("path", s("/workspace/beef/tx123.beef")),
            ("txid", s("tx123")),
            ("sats", n(200)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ReceiptStored {
            path, txid, sats, ..
        } => {
            assert_eq!(path, "/workspace/beef/tx123.beef");
            assert_eq!(txid, "tx123");
            assert_eq!(sats, 200);
        }
        _ => panic!("Expected ReceiptStored variant"),
    }
}

#[test]
fn test_skill_activated() {
    let event = make_event(
        "skill_activated",
        vec![("skill_name", s("x402")), ("context", s("auto_activate"))],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::SkillActivated {
            skill_name,
            context,
            ..
        } => {
            assert_eq!(skill_name, "x402");
            assert_eq!(context, "auto_activate");
        }
        _ => panic!("Expected SkillActivated variant"),
    }
}

// ─── State checkpoint tests (#273) ─────────────────────────────────────

#[test]
fn test_state_checkpoint_event() {
    let event = make_event(
        "state_checkpoint",
        vec![
            ("iteration", n(10)),
            ("sats_spent", n(150000)),
            ("last_proof_hash", s("abc123")),
            (
                "tools_used",
                serde_json::json!(["wallet_balance", "memory_search"]),
            ),
            ("model", s("gpt-5-mini")),
            ("checkpoint_type", s("state_snapshot")),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::StateCheckpoint {
            iteration,
            sats_spent,
            last_proof_hash,
            tools_used,
            model,
            ..
        } => {
            assert_eq!(iteration, 10);
            assert_eq!(sats_spent, 150000);
            assert_eq!(last_proof_hash, Some("abc123".to_string()));
            assert_eq!(tools_used, vec!["wallet_balance", "memory_search"]);
            assert_eq!(model, "gpt-5-mini");
        }
        _ => panic!("Expected StateCheckpoint variant"),
    }
}

#[test]
fn test_transcript_record_state_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(path);

    transcript.record_state_checkpoint(
        5,
        50000,
        Some("hash_abc"),
        &["wallet_balance".to_string()],
        "gpt-5-mini",
    );

    let events = transcript.replay();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "state_checkpoint");
    assert_eq!(
        events[0].data.get("iteration").unwrap().as_u64().unwrap(),
        5
    );
    assert_eq!(
        events[0].data.get("sats_spent").unwrap().as_u64().unwrap(),
        50000
    );
}

// ─── Interrupted session recovery tests (#273) ────────────────────────

#[test]
fn test_interrupted_session_with_checkpoint() {
    // Simulate a crash after iteration 10 with checkpoint at iteration 10
    let messages = vec![
        Message::SessionStart {
            task: "long research task".to_string(),
            ts: 1.0,
        },
        Message::ThinkRequest {
            model: "gpt-5".to_string(),
            max_tokens: 16384,
            message_count: 3,
            ts: 2.0,
        },
        Message::ThinkResponse {
            content: "searching...".to_string(),
            model: "gpt-5".to_string(),
            sats_paid: 100,
            sats_effective: 25000,
            sats_refunded: 75,
            prompt_tokens: 5000,
            completion_tokens: 100,
            finish_reason: "tool_calls".to_string(),
            duration_ms: 10000,
            tool_calls: vec![],
            ts: 3.0,
        },
        Message::StateCheckpoint {
            iteration: 5,
            sats_spent: 125000,
            last_proof_hash: Some("hash_at_5".to_string()),
            tools_used: vec!["web_search".to_string(), "memory_store".to_string()],
            model: "gpt-5".to_string(),
            ts: 4.0,
        },
        // More iterations...
        Message::ThinkRequest {
            model: "gpt-5".to_string(),
            max_tokens: 16384,
            message_count: 8,
            ts: 5.0,
        },
        Message::ThinkResponse {
            content: "found results".to_string(),
            model: "gpt-5".to_string(),
            sats_paid: 100,
            sats_effective: 30000,
            sats_refunded: 70,
            prompt_tokens: 8000,
            completion_tokens: 200,
            finish_reason: "stop".to_string(),
            duration_ms: 15000,
            tool_calls: vec![],
            ts: 6.0,
        },
        // No session_end — crash
    ];

    let state = SessionState::from_messages(&messages);
    assert!(state.is_interrupted());
    assert_eq!(state.iterations, 2);
    assert_eq!(state.sats_spent, 55000); // 25000 + 30000
    assert_eq!(state.task, "long research task");
    assert!(!state.completed);
    assert!(state.error.is_none());
}

// ─── Message methods ───────────────────────────────────────────────────

#[test]
fn test_ts_accessor() {
    let event = make_event("user", vec![("content", s("hello"))]);
    let msg = Message::from_event(&event);
    assert_eq!(msg.ts(), 1000.0);
}

#[test]
fn test_is_terminal() {
    let end = make_event(
        "session_end",
        vec![
            ("iterations", n(1)),
            ("sats_spent", n(100)),
            ("result", s("")),
            ("error", s("")),
        ],
    );
    assert!(Message::from_event(&end).is_terminal());

    let user = make_event("user", vec![("content", s("hi"))]);
    assert!(!Message::from_event(&user).is_terminal());
}

#[test]
fn test_is_truncated() {
    let truncated = make_event(
        "think_response",
        vec![
            ("content", s("partial...")),
            ("model", s("gpt-5")),
            ("finish_reason", s("length")),
            ("sats_effective", n(100)),
        ],
    );
    assert!(Message::from_event(&truncated).is_truncated());

    let normal = make_event(
        "think_response",
        vec![
            ("content", s("done")),
            ("model", s("gpt-5")),
            ("finish_reason", s("stop")),
            ("sats_effective", n(100)),
        ],
    );
    assert!(!Message::from_event(&normal).is_truncated());
}

#[test]
fn test_finish_reason_accessor() {
    let resp = make_event(
        "think_response",
        vec![
            ("content", s("")),
            ("model", s("gpt-5")),
            ("finish_reason", s("tool_calls")),
            ("sats_effective", n(100)),
        ],
    );
    assert_eq!(
        Message::from_event(&resp).finish_reason(),
        Some("tool_calls")
    );

    let user = make_event("user", vec![("content", s("hi"))]);
    assert_eq!(Message::from_event(&user).finish_reason(), None);
}

// ─── SessionState reconstruction tests ─────────────────────────────────

#[test]
fn test_empty_session_state() {
    let state = SessionState::from_messages(&[]);
    assert_eq!(state.iterations, 0);
    assert_eq!(state.sats_spent, 0);
    assert!(state.task.is_empty());
    assert!(!state.completed);
    assert!(state.is_interrupted());
    assert!(state.last_proof_hash.is_none());
}

#[test]
fn test_complete_session_state() {
    let messages = vec![
        Message::SessionStart {
            task: "What is 2+2?".to_string(),
            ts: 1.0,
        },
        Message::ThinkRequest {
            model: "gpt-5-mini".to_string(),
            max_tokens: 16384,
            message_count: 3,
            ts: 2.0,
        },
        Message::ThinkResponse {
            content: "4".to_string(),
            model: "gpt-5-mini".to_string(),
            sats_paid: 263000,
            sats_effective: 15000,
            sats_refunded: 248000,
            prompt_tokens: 7500,
            completion_tokens: 74,
            finish_reason: "stop".to_string(),
            duration_ms: 25000,
            tool_calls: vec![],
            ts: 3.0,
        },
        Message::BudgetCheck {
            balance: 49000000,
            spent_session: 15000,
            spent_hour: 15000,
            task_limit: 20000000,
            ts: 4.0,
        },
        Message::ProofCreated {
            txid: "proof_tx1".to_string(),
            proof_type: "decision".to_string(),
            hash: "hash_abc".to_string(),
            prev_hash: None,
            basket: Some("worm-proofs".to_string()),
            iteration: Some(1),
            sats_cost: Some(200),
            ts: 5.0,
        },
        Message::SessionEnd {
            iterations: 1,
            sats_spent: 15200,
            result: "4".to_string(),
            error: String::new(),
            ts: 6.0,
        },
    ];

    let state = SessionState::from_messages(&messages);
    assert_eq!(state.task, "What is 2+2?");
    assert_eq!(state.iterations, 1);
    assert_eq!(state.sats_spent, 15200); // session_end total preferred
    assert!(state.completed);
    assert!(!state.is_interrupted());
    assert_eq!(state.last_proof_hash, Some("hash_abc".to_string()));
    assert_eq!(state.proof_txids, vec!["proof_tx1"]);
    assert_eq!(state.last_balance, 49000000);
    assert_eq!(state.last_model, Some("gpt-5-mini".to_string()));
    assert!(state.error.is_none());
}

#[test]
fn test_interrupted_session_state() {
    // Simulate a crash: session_start + think_request + think_response, no session_end
    let messages = vec![
        Message::SessionStart {
            task: "Research BSV fees".to_string(),
            ts: 1.0,
        },
        Message::ThinkRequest {
            model: "gpt-5".to_string(),
            max_tokens: 16384,
            message_count: 3,
            ts: 2.0,
        },
        Message::ThinkResponse {
            content: "Let me search...".to_string(),
            model: "gpt-5".to_string(),
            sats_paid: 500000,
            sats_effective: 30000,
            sats_refunded: 470000,
            prompt_tokens: 10000,
            completion_tokens: 200,
            finish_reason: "tool_calls".to_string(),
            duration_ms: 35000,
            tool_calls: vec![serde_json::json!({"id": "call_1", "type": "function"})],
            ts: 3.0,
        },
        Message::ToolCall {
            call_id: "call_1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({"query": "BSV fees"}),
            ts: 4.0,
        },
        Message::ToolResult {
            call_id: "call_1".to_string(),
            name: "web_search".to_string(),
            content: "Results found".to_string(),
            success: true,
            sats_paid: 5000,
            ts: 5.0,
        },
    ];

    let state = SessionState::from_messages(&messages);
    assert_eq!(state.task, "Research BSV fees");
    assert_eq!(state.iterations, 1);
    assert_eq!(state.sats_spent, 35000); // 30000 LLM + 5000 tool
    assert!(!state.completed);
    assert!(state.is_interrupted());
    assert_eq!(state.tool_call_count, 1);
    assert_eq!(state.tools_used, vec!["web_search"]);
    assert_eq!(state.last_finish_reason, Some("tool_calls".to_string()));
}

#[test]
fn test_truncation_detection() {
    let messages = vec![
        Message::ThinkRequest {
            model: "gpt-5".to_string(),
            max_tokens: 16384,
            message_count: 3,
            ts: 1.0,
        },
        Message::ThinkResponse {
            content: "This is a very long response that got cut off...".to_string(),
            model: "gpt-5".to_string(),
            sats_paid: 100,
            sats_effective: 50,
            sats_refunded: 50,
            prompt_tokens: 100,
            completion_tokens: 16384,
            finish_reason: "length".to_string(),
            duration_ms: 30000,
            tool_calls: vec![],
            ts: 2.0,
        },
    ];

    let state = SessionState::from_messages(&messages);
    assert!(state.was_truncated());
    assert_eq!(state.truncation_count, 1);
    assert_eq!(state.last_finish_reason, Some("length".to_string()));
}

#[test]
fn test_multi_iteration_state() {
    let messages = vec![
        Message::SessionStart {
            task: "Complex task".to_string(),
            ts: 1.0,
        },
        // Iteration 1
        Message::ThinkRequest {
            model: "gpt-5-mini".to_string(),
            max_tokens: 16384,
            message_count: 3,
            ts: 2.0,
        },
        Message::ThinkResponse {
            content: "".to_string(),
            model: "gpt-5-mini".to_string(),
            sats_paid: 100,
            sats_effective: 10000,
            sats_refunded: 0,
            prompt_tokens: 100,
            completion_tokens: 50,
            finish_reason: "tool_calls".to_string(),
            duration_ms: 5000,
            tool_calls: vec![],
            ts: 3.0,
        },
        Message::ToolCall {
            call_id: "c1".to_string(),
            name: "wallet_balance".to_string(),
            arguments: Value::Null,
            ts: 4.0,
        },
        Message::ToolResult {
            call_id: "c1".to_string(),
            name: "wallet_balance".to_string(),
            content: "50000000".to_string(),
            success: true,
            sats_paid: 0,
            ts: 5.0,
        },
        // Iteration 2
        Message::ThinkRequest {
            model: "gpt-5-mini".to_string(),
            max_tokens: 16384,
            message_count: 5,
            ts: 6.0,
        },
        Message::ThinkResponse {
            content: "Your balance is 50M sats.".to_string(),
            model: "gpt-5-mini".to_string(),
            sats_paid: 200,
            sats_effective: 8000,
            sats_refunded: 0,
            prompt_tokens: 150,
            completion_tokens: 30,
            finish_reason: "stop".to_string(),
            duration_ms: 4000,
            tool_calls: vec![],
            ts: 7.0,
        },
        Message::ProofCreated {
            txid: "proof1".to_string(),
            proof_type: "decision".to_string(),
            hash: "hash1".to_string(),
            prev_hash: None,
            basket: None,
            iteration: Some(1),
            sats_cost: None,
            ts: 8.0,
        },
        Message::ProofCreated {
            txid: "proof2".to_string(),
            proof_type: "decision".to_string(),
            hash: "hash2".to_string(),
            prev_hash: Some("hash1".to_string()),
            basket: None,
            iteration: Some(2),
            sats_cost: None,
            ts: 9.0,
        },
        Message::SessionEnd {
            iterations: 2,
            sats_spent: 18000,
            result: "Your balance is 50M sats.".to_string(),
            error: String::new(),
            ts: 10.0,
        },
    ];

    let state = SessionState::from_messages(&messages);
    assert_eq!(state.iterations, 2);
    assert_eq!(state.sats_spent, 18000);
    assert_eq!(state.last_proof_hash, Some("hash2".to_string()));
    assert_eq!(state.proof_txids, vec!["proof1", "proof2"]);
    assert_eq!(state.tool_call_count, 1);
    assert_eq!(state.tools_used, vec!["wallet_balance"]);
    assert!(state.completed);
    assert!(!state.was_truncated());
    assert_eq!(state.last_finish_reason, Some("stop".to_string()));
}

#[test]
fn test_session_end_with_error() {
    let messages = vec![
        Message::SessionStart {
            task: "fail task".to_string(),
            ts: 1.0,
        },
        Message::Error {
            error: "Budget exceeded".to_string(),
            context: None,
            ts: 2.0,
        },
        Message::SessionEnd {
            iterations: 1,
            sats_spent: 500,
            result: String::new(),
            error: "Budget exceeded".to_string(),
            ts: 3.0,
        },
    ];

    let state = SessionState::from_messages(&messages);
    assert!(state.completed);
    assert_eq!(state.error, Some("Budget exceeded".to_string()));
}

#[test]
fn test_tool_dedup_in_tools_used() {
    let messages = vec![
        Message::ToolCall {
            call_id: "c1".to_string(),
            name: "memory_search".to_string(),
            arguments: Value::Null,
            ts: 1.0,
        },
        Message::ToolCall {
            call_id: "c2".to_string(),
            name: "memory_search".to_string(),
            arguments: Value::Null,
            ts: 2.0,
        },
        Message::ToolCall {
            call_id: "c3".to_string(),
            name: "wallet_balance".to_string(),
            arguments: Value::Null,
            ts: 3.0,
        },
    ];

    let state = SessionState::from_messages(&messages);
    assert_eq!(state.tool_call_count, 3);
    assert_eq!(state.tools_used.len(), 2); // deduplicated
    assert!(state.tools_used.contains(&"memory_search".to_string()));
    assert!(state.tools_used.contains(&"wallet_balance".to_string()));
}

// ─── Transcript integration tests ──────────────────────────────────────

#[test]
fn test_transcript_to_typed_messages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(path);

    transcript.record_user("What is 2+2?");
    transcript.record_think_request(&[], "gpt-5-mini", 16384);
    transcript.record_think_response("4", "gpt-5-mini", 100, 50, 50, 100, 10, None, "stop", 1000);

    let messages = transcript.to_typed_messages();
    assert_eq!(messages.len(), 3);

    assert!(matches!(messages[0], Message::User { .. }));
    assert!(matches!(messages[1], Message::ThinkRequest { .. }));
    assert!(matches!(messages[2], Message::ThinkResponse { .. }));
}

#[test]
fn test_transcript_reconstruct_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(path);

    // Record a complete session
    let mut start_data = HashMap::new();
    start_data.insert("task".to_string(), Value::String("test task".to_string()));
    transcript.record("session_start", start_data);

    transcript.record_user("test task");
    transcript.record_think_request(&[], "gpt-5-mini", 16384);
    transcript.record_think_response(
        "done",
        "gpt-5-mini",
        200,
        100,
        100,
        500,
        50,
        None,
        "stop",
        2000,
    );
    transcript.record_budget(49000000, 100, 100, 20000000);

    let mut end_data = HashMap::new();
    end_data.insert("iterations".to_string(), serde_json::json!(1));
    end_data.insert("sats_spent".to_string(), serde_json::json!(100));
    end_data.insert("result".to_string(), Value::String("done".to_string()));
    end_data.insert("error".to_string(), Value::String(String::new()));
    transcript.record("session_end", end_data);

    let state = transcript.reconstruct_state();
    assert_eq!(state.task, "test task");
    assert_eq!(state.iterations, 1);
    assert_eq!(state.sats_spent, 100);
    assert!(state.completed);
    assert_eq!(state.last_balance, 49000000);
    assert_eq!(state.last_model, Some("gpt-5-mini".to_string()));
}

// ─── Missing/default field handling ────────────────────────────────────

#[test]
fn test_missing_fields_default_gracefully() {
    // ThinkResponse with minimal fields — should not panic
    let event = make_event("think_response", vec![("content", s("hi"))]);
    let msg = Message::from_event(&event);
    match msg {
        Message::ThinkResponse {
            sats_paid,
            sats_effective,
            prompt_tokens,
            finish_reason,
            model,
            ..
        } => {
            assert_eq!(sats_paid, 0);
            assert_eq!(sats_effective, 0);
            assert_eq!(prompt_tokens, 0);
            assert!(finish_reason.is_empty());
            assert!(model.is_empty());
        }
        _ => panic!("Expected ThinkResponse"),
    }
}

#[test]
fn test_tool_result_missing_sats_paid() {
    let event = make_event(
        "tool_result",
        vec![
            ("call_id", s("c1")),
            ("name", s("file_read")),
            ("content", s("file contents")),
            ("success", Value::Bool(true)),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::ToolResult { sats_paid, .. } => {
            assert_eq!(sats_paid, 0); // defaults to 0 when absent
        }
        _ => panic!("Expected ToolResult"),
    }
}

#[test]
fn test_checkpoint_with_data() {
    let data = serde_json::json!({
        "budget_cap": 20000000,
        "model": "gpt-5-mini",
        "skills_hash": "abc123"
    });
    let event = make_event(
        "checkpoint_created",
        vec![
            ("txid", s("ckpt_1")),
            ("token_type", s("task_commitment")),
            ("basket", s("worm-state")),
            ("checkpoint_data", data.clone()),
        ],
    );
    let msg = Message::from_event(&event);
    match msg {
        Message::CheckpointCreated {
            checkpoint_data, ..
        } => {
            assert_eq!(checkpoint_data.unwrap(), data);
        }
        _ => panic!("Expected CheckpointCreated"),
    }
}
