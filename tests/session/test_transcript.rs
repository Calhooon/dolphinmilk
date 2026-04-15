//! Tests for transcript — JSONL session transcript.
//! Mirrors Python tests/test_transcript.py (15 tests).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use tempfile::TempDir;

use dolphin_milk::transcript::{Transcript, TranscriptEvent, EVENT_TYPES};

fn tmp_transcript() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.jsonl");
    (dir, path)
}

// -- TestTranscriptEvent --

#[test]
fn test_event_to_json() {
    let mut data = HashMap::new();
    data.insert("content".to_string(), Value::String("hi".to_string()));
    let e = TranscriptEvent::new("user", data);
    let json_str = serde_json::to_string(&e).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "user");
    assert_eq!(parsed["content"], "hi");
    assert!(parsed["ts"].as_f64().unwrap() > 0.0);
    assert!(!parsed["id"].as_str().unwrap().is_empty());
}

#[test]
fn test_event_from_json() {
    let json_str = r#"{"ts":1000.0,"type":"user","id":"abc","content":"hi"}"#;
    let e: TranscriptEvent = serde_json::from_str(json_str).unwrap();
    assert_eq!(e.ts, 1000.0);
    assert_eq!(e.event_type, "user");
    assert_eq!(e.id, "abc");
    assert_eq!(e.data["content"], "hi");
}

// -- TestTranscript --

#[test]
fn test_record_and_replay() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let mut session_data = HashMap::new();
    session_data.insert("task".to_string(), Value::String("test".to_string()));
    t.record("session_start", session_data);
    t.record_user("hello");
    t.record_system("you are a worm");

    let events = t.replay();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type, "session_start");
    assert_eq!(events[1].event_type, "user");
    assert_eq!(events[2].event_type, "system");
}

#[test]
fn test_persistence() {
    let (_dir, path) = tmp_transcript();

    // Write events
    {
        let mut t1 = Transcript::new(path.clone());
        t1.record_user("hello");
        t1.record_system("hi");
    }

    // Load from file
    let t2 = Transcript::new(path);
    assert_eq!(t2.event_count(), 2);
    let events = t2.replay();
    assert_eq!(events[0].event_type, "user");
    assert_eq!(events[1].event_type, "system");
}

#[test]
fn test_record_think_response() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let e = t.record_think_response(
        "hello world",
        "gpt-5-mini",
        1000,
        800,
        0,
        10,
        20,
        None,
        "stop",
        500,
    );
    assert_eq!(e.event_type, "think_response");
    assert_eq!(e.data["sats_paid"], json!(1000));
    assert_eq!(e.data["content"], "hello world");
}

#[test]
fn test_record_tool_call_and_result() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_tool_call("call_1", "execute_bash", &json!({"command": "ls"}));
    t.record_tool_result("call_1", "execute_bash", "file1.txt\nfile2.txt", true, 0);

    let events = t.replay();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "tool_call");
    assert_eq!(events[1].event_type, "tool_result");
}

#[test]
fn test_to_messages() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_system("you are a worm");
    t.record_user("hello");
    t.record_think_response(
        "hi there",
        "gpt-5-mini",
        0,
        0,
        0,
        0,
        0,
        Some(&[json!({"id": "tc_1", "type": "function", "function": {"name": "test", "arguments": "{}"}})]),
        "tool_calls",
        0,
    );
    t.record_tool_result("tc_1", "test", "result", true, 0);
    t.record_think_response("done", "gpt-5-mini", 0, 0, 0, 0, 0, None, "stop", 0);

    let msgs = t.to_messages();
    assert_eq!(msgs.len(), 5);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[2]["role"], "assistant");
    assert_eq!(msgs[3]["role"], "tool");
    assert_eq!(msgs[4]["role"], "assistant");
}

#[test]
fn test_orphaned_tool_result_skipped() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_user("hello");
    t.record_tool_result("orphan_1", "test", "orphan result", true, 0);
    t.record_think_response("done", "gpt-5-mini", 0, 0, 0, 0, 0, None, "stop", 0);

    let msgs = t.to_messages();
    // Orphaned tool result should be skipped
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[1]["role"], "assistant");
}

#[test]
fn test_total_sats_spent() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_think_response("a", "m", 100, 100, 0, 0, 0, None, "stop", 0);
    t.record_think_response("b", "m", 200, 200, 0, 0, 0, None, "stop", 0);
    assert_eq!(t.total_sats_spent(), 300);
}

#[test]
fn test_total_sats_spent_includes_tool_costs() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    // LLM inference: 100 sats
    t.record_think_response("a", "m", 100, 100, 0, 0, 0, None, "stop", 0);
    // Tool with payment: 50000 sats (e.g. image gen)
    t.record_tool_result(
        "call-1",
        "generate_image",
        r#"{"sats_paid": 50000}"#,
        true,
        50000,
    );
    // Tool without payment: 0 sats
    t.record_tool_result("call-2", "execute_bash", "output", true, 0);
    // Another LLM inference: 200 sats
    t.record_think_response("b", "m", 200, 200, 0, 0, 0, None, "stop", 0);
    // Total: 100 + 50000 + 200 = 50300
    assert_eq!(t.total_sats_spent(), 50300);
}

#[test]
fn test_total_sats_backward_compat_no_sats_field() {
    // Old transcripts without sats_paid on tool_result events should return 0 for those tools
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_think_response("a", "m", 100, 100, 0, 0, 0, None, "stop", 0);
    // sats_paid=0 means no sats_paid field is written (backward compat)
    t.record_tool_result("call-1", "execute_bash", "output", true, 0);
    assert_eq!(t.total_sats_spent(), 100);
}

#[test]
fn test_total_tokens() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_think_response("a", "m", 0, 0, 0, 10, 20, None, "stop", 0);
    t.record_think_response("b", "m", 0, 0, 0, 30, 40, None, "stop", 0);
    let (prompt, completion) = t.total_tokens();
    assert_eq!(prompt, 40);
    assert_eq!(completion, 60);
}

#[test]
fn test_event_count_and_last_event() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    assert_eq!(t.event_count(), 0);
    assert!(t.last_event().is_none());

    t.record_user("hello");
    assert_eq!(t.event_count(), 1);
    assert_eq!(t.last_event().unwrap().event_type, "user");
}

#[test]
fn test_get_events_by_type() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_user("a");
    t.record_system("b");
    t.record_user("c");

    let users = t.get_events_by_type("user");
    assert_eq!(users.len(), 2);
}

#[test]
fn test_corrupt_line_handled() {
    let (_dir, path) = tmp_transcript();
    // Write a valid line, a corrupt line, and another valid line
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&json!({"ts": 1.0, "type": "user", "id": "a", "content": "hi"}))
                .unwrap()
        )
        .unwrap();
        writeln!(f, "this is not json").unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(
                &json!({"ts": 2.0, "type": "system", "id": "b", "content": "ok"})
            )
            .unwrap()
        )
        .unwrap();
    }

    let t = Transcript::new(path);
    assert_eq!(t.event_count(), 2); // Corrupt line skipped
}

#[test]
fn test_event_types_constant() {
    assert!(EVENT_TYPES.contains(&"user"));
    assert!(EVENT_TYPES.contains(&"think_response"));
    assert!(EVENT_TYPES.contains(&"tool_call"));
    assert!(EVENT_TYPES.contains(&"session_start"));
    assert!(EVENT_TYPES.len() >= 10);
}

#[test]
fn test_record_budget() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let e = t.record_budget(1000, 500, 500, 20_000_000);
    assert_eq!(e.event_type, "budget_check");
    assert_eq!(e.data["balance"], json!(1000));
    assert_eq!(e.data["spent_session"], json!(500));
}

#[test]
fn test_record_error() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let mut ctx = HashMap::new();
    ctx.insert("type".to_string(), Value::String("budget".to_string()));
    let e = t.record_error("budget exceeded", Some(ctx));
    assert_eq!(e.event_type, "error");
    assert_eq!(e.data["error"], "budget exceeded");
}

// -- Proof data recording tests --

#[test]
fn test_record_proof_created_with_data() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let e = t.record_proof_created(
        "abc123",
        "decision",
        "deadbeef",
        Some("Iteration 1: response generated"),
        Some("2026-02-26T12:00:00Z"),
        Some(200),
        Some(1),
        Some("dm-proofs"),
        None,
    );
    assert_eq!(e.event_type, "proof_created");
    assert_eq!(e.data["txid"], "abc123");
    assert_eq!(e.data["proof_type"], "decision");
    assert_eq!(e.data["hash"], "deadbeef");
    assert_eq!(e.data["proof_data"], "Iteration 1: response generated");
    assert_eq!(e.data["proof_timestamp"], "2026-02-26T12:00:00Z");
    assert_eq!(e.data["sats_cost"], json!(200));
    assert_eq!(e.data["iteration"], json!(1));
    assert_eq!(e.data["basket"], "dm-proofs");
}

#[test]
fn test_record_proof_created_without_data_backward_compat() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let e = t.record_proof_created(
        "abc123", "decision", "deadbeef", None, None, None, None, None, None,
    );
    assert_eq!(e.event_type, "proof_created");
    assert_eq!(e.data["txid"], "abc123");
    assert_eq!(e.data["proof_type"], "decision");
    assert_eq!(e.data["hash"], "deadbeef");
    // Optional fields should not be present
    assert!(!e.data.contains_key("proof_data"));
    assert!(!e.data.contains_key("proof_timestamp"));
    assert!(!e.data.contains_key("sats_cost"));
    assert!(!e.data.contains_key("iteration"));
    assert!(!e.data.contains_key("basket"));
}

#[test]
fn test_record_proof_data_persists_across_replay() {
    let (_dir, path) = tmp_transcript();
    {
        let mut t = Transcript::new(path.clone());
        t.record_proof_created(
            "tx1",
            "budget_snapshot",
            "aabb",
            Some("TASK_SATS: 1000\nITERATIONS: 5"),
            Some("2026-02-26T12:00:00Z"),
            Some(200),
            Some(5),
            Some("dm-proofs"),
            None,
        );
    }

    let t2 = Transcript::new(path);
    let events = t2.replay();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].data["proof_data"],
        "TASK_SATS: 1000\nITERATIONS: 5"
    );
    assert_eq!(events[0].data["proof_timestamp"], "2026-02-26T12:00:00Z");
    assert_eq!(events[0].data["sats_cost"], json!(200));
}

#[test]
fn test_record_checkpoint_created_with_data() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let checkpoint = json!({
        "task": "compute pi",
        "iterations": 3,
        "sats_spent": 600,
    });
    let e = t.record_checkpoint_created("tx99", "checkpoint", "dm-state", Some(&checkpoint));
    assert_eq!(e.event_type, "checkpoint_created");
    assert_eq!(e.data["txid"], "tx99");
    assert_eq!(e.data["token_type"], "checkpoint");
    assert_eq!(e.data["basket"], "dm-state");
    assert_eq!(e.data["checkpoint_data"]["task"], "compute pi");
    assert_eq!(e.data["checkpoint_data"]["iterations"], 3);
}

// -- events_since tests --

#[test]
fn test_events_since_returns_all_from_zero() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_user("a");
    t.record_user("b");
    t.record_user("c");

    let slice = t.events_since(0);
    assert_eq!(slice.len(), 3);
    assert_eq!(slice[0].data["content"], "a");
}

#[test]
fn test_events_since_returns_partial_slice() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_user("a");
    t.record_user("b");
    t.record_user("c");
    t.record_user("d");

    let slice = t.events_since(2);
    assert_eq!(slice.len(), 2);
    assert_eq!(slice[0].data["content"], "c");
    assert_eq!(slice[1].data["content"], "d");
}

#[test]
fn test_events_since_beyond_returns_empty() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_user("a");

    let slice = t.events_since(5);
    assert!(slice.is_empty());

    let slice2 = t.events_since(1);
    assert!(slice2.is_empty());
}

#[test]
fn test_events_since_empty_transcript() {
    let (_dir, path) = tmp_transcript();
    let t = Transcript::new(path);

    let slice = t.events_since(0);
    assert!(slice.is_empty());
}

#[test]
fn test_record_checkpoint_created_without_data_backward_compat() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    let e = t.record_checkpoint_created("tx99", "checkpoint", "dm-state", None);
    assert_eq!(e.event_type, "checkpoint_created");
    assert_eq!(e.data["txid"], "tx99");
    assert!(!e.data.contains_key("checkpoint_data"));
}

// -- get_tool_result_content tests --

#[test]
fn test_get_tool_result_content_found() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_tool_result("call_123", "x402_call", "full result data here", true, 500);
    t.record_tool_result("call_456", "file_read", "different content", true, 0);

    let result = t.get_tool_result_content("call_123");
    assert_eq!(result, Some("full result data here".to_string()));

    let result2 = t.get_tool_result_content("call_456");
    assert_eq!(result2, Some("different content".to_string()));
}

#[test]
fn test_get_tool_result_content_not_found() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    t.record_tool_result("call_123", "x402_call", "some content", true, 0);

    let result = t.get_tool_result_content("call_nonexistent");
    assert_eq!(result, None);
}

#[test]
fn test_get_tool_result_content_empty_transcript() {
    let (_dir, path) = tmp_transcript();
    let t = Transcript::new(path);
    let result = t.get_tool_result_content("call_any");
    assert_eq!(result, None);
}

#[test]
fn test_get_tool_result_content_returns_first_match() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);
    // Same call_id recorded twice (shouldn't normally happen, but test determinism)
    t.record_tool_result("call_dup", "tool_a", "first content", true, 0);
    t.record_tool_result("call_dup", "tool_a", "second content", true, 0);

    let result = t.get_tool_result_content("call_dup");
    assert_eq!(
        result,
        Some("first content".to_string()),
        "Should return the first match"
    );
}
