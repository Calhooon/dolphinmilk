//! Integration tests for the introspect tool: recent_proofs, task_costs,
//! task_detail, activity_summary. Pure filesystem tests — no wallet or HTTP needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use tempfile::TempDir;

use dolphin_milk::tools::introspect_tools::create_introspect_tool;

// ── Helpers ─────────────────────────────────────────────────────────────

fn create_test_workspace() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    fs::create_dir_all(workspace.join("tasks")).unwrap();
    (dir, workspace)
}

/// Write transcript events to `workspace/tasks/{task_id}/session.jsonl`.
fn write_task_transcript(workspace: &Path, task_id: &str, events: &[Value]) {
    let task_dir = workspace.join("tasks").join(task_id);
    fs::create_dir_all(&task_dir).unwrap();
    let mut content = String::new();
    for event in events {
        content.push_str(&serde_json::to_string(event).unwrap());
        content.push('\n');
    }
    fs::write(task_dir.join("session.jsonl"), content).unwrap();
}

/// Write budget entries to `workspace/budget.jsonl`.
fn write_budget_log(workspace: &Path, entries: &[Value]) {
    let mut content = String::new();
    for entry in entries {
        content.push_str(&serde_json::to_string(entry).unwrap());
        content.push('\n');
    }
    fs::write(workspace.join("budget.jsonl"), content).unwrap();
}

/// Helper: call the introspect tool with given params.
async fn call_introspect(workspace: &Path, params: Value) -> Value {
    let tool = create_introspect_tool(Arc::new(workspace.to_path_buf()));
    let result_str = (tool.execute)(params).await;
    serde_json::from_str(&result_str).unwrap_or_else(|_| json!({ "raw": result_str }))
}

// ── recent_proofs ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_recent_proofs_basic() {
    let (_dir, workspace) = create_test_workspace();

    let events = vec![
        json!({"type": "session_start", "ts": 1677123454.0, "id": "s1s2s3s4", "task": "Research BSV fees"}),
        json!({"type": "proof_created", "ts": 1677123456.789, "id": "a1b2c3d4", "txid": "abc123def456789", "proof_type": "Decision", "hash": "sha256aaa", "sats_cost": 200, "iteration": 1}),
        json!({"type": "proof_created", "ts": 1677123458.0, "id": "b2c3d4e5", "txid": "def456ghi789012", "proof_type": "TaskCompletion", "hash": "sha256bbb", "sats_cost": 200, "iteration": 2}),
        json!({"type": "session_end", "ts": 1677123460.0, "id": "e1e2e3e4", "status": "Complete", "iterations": 2}),
    ];
    write_task_transcript(&workspace, "task-001", &events);

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 5})).await;

    let proofs = result["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 2);
    // Most recent first
    assert_eq!(proofs[0]["txid"], "def456ghi789012");
    assert_eq!(proofs[1]["txid"], "abc123def456789");
    assert_eq!(proofs[0]["proof_type"], "TaskCompletion");
    assert_eq!(proofs[1]["proof_type"], "Decision");
}

#[tokio::test]
async fn test_recent_proofs_across_tasks() {
    let (_dir, workspace) = create_test_workspace();

    // Task 1: proof at ts=1000
    write_task_transcript(
        &workspace,
        "task-a",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "task a"}),
            json!({"type": "proof_created", "ts": 1000.0, "id": "p1", "txid": "tx-a", "proof_type": "Decision", "sats_cost": 200}),
        ],
    );

    // Task 2: proof at ts=2000
    write_task_transcript(
        &workspace,
        "task-b",
        &[
            json!({"type": "session_start", "ts": 2000.0, "id": "s2", "task": "task b"}),
            json!({"type": "proof_created", "ts": 2000.0, "id": "p2", "txid": "tx-b", "proof_type": "Decision", "sats_cost": 200}),
        ],
    );

    // Task 3: proof at ts=3000
    write_task_transcript(
        &workspace,
        "task-c",
        &[
            json!({"type": "session_start", "ts": 3000.0, "id": "s3", "task": "task c"}),
            json!({"type": "proof_created", "ts": 3000.0, "id": "p3", "txid": "tx-c", "proof_type": "Decision", "sats_cost": 200}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 10})).await;

    let proofs = result["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 3);
    // Sorted by timestamp descending: c, b, a
    assert_eq!(proofs[0]["txid"], "tx-c");
    assert_eq!(proofs[1]["txid"], "tx-b");
    assert_eq!(proofs[2]["txid"], "tx-a");
}

#[tokio::test]
async fn test_recent_proofs_count_limit() {
    let (_dir, workspace) = create_test_workspace();

    let mut events: Vec<Value> =
        vec![json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "lots of proofs"})];
    for i in 0..10 {
        events.push(json!({
            "type": "proof_created",
            "ts": 1001.0 + i as f64,
            "id": format!("p{i}"),
            "txid": format!("tx-{i}"),
            "proof_type": "Decision",
            "sats_cost": 200,
            "iteration": i + 1
        }));
    }
    write_task_transcript(&workspace, "task-many", &events);

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 3})).await;

    let proofs = result["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 3);
    assert_eq!(result["total"], 10);
}

#[tokio::test]
async fn test_recent_proofs_empty() {
    let (_dir, workspace) = create_test_workspace();

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 5})).await;

    let proofs = result["proofs"].as_array().unwrap();
    assert!(proofs.is_empty());
    assert_eq!(result["total"], 0);
}

#[tokio::test]
async fn test_recent_proofs_includes_task_id() {
    let (_dir, workspace) = create_test_workspace();

    write_task_transcript(
        &workspace,
        "task-identified",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "test"}),
            json!({"type": "proof_created", "ts": 1001.0, "id": "p1", "txid": "tx-1", "proof_type": "Decision", "sats_cost": 200}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 5})).await;

    let proofs = result["proofs"].as_array().unwrap();
    assert_eq!(proofs[0]["task_id"], "task-identified");
}

// ── task_costs ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_task_costs_basic() {
    let (_dir, workspace) = create_test_workspace();

    // Task 1: 500 sats LLM
    write_task_transcript(
        &workspace,
        "task-cost1",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "task one"}),
            json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini", "prompt_tokens": 100, "completion_tokens": 50}),
            json!({"type": "session_end", "ts": 1002.0, "id": "e1", "status": "Complete", "iterations": 1}),
        ],
    );

    // Task 2: 800 sats LLM + 200 sats proof
    write_task_transcript(
        &workspace,
        "task-cost2",
        &[
            json!({"type": "session_start", "ts": 2000.0, "id": "s2", "task": "task two"}),
            json!({"type": "think_response", "ts": 2001.0, "id": "t2", "sats_effective": 800, "model": "gpt-5", "prompt_tokens": 200, "completion_tokens": 100}),
            json!({"type": "proof_created", "ts": 2002.0, "id": "p1", "txid": "proof1", "sats_cost": 200}),
            json!({"type": "session_end", "ts": 2003.0, "id": "e2", "status": "Complete", "iterations": 1}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "task_costs", "count": 10})).await;

    let tasks = result["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    // Each task has total_sats, model, status
    for task in tasks {
        assert!(task["total_sats"].as_u64().unwrap() > 0);
        assert!(task["model"].as_str().is_some());
        assert!(task["status"].as_str().is_some());
    }
}

#[tokio::test]
async fn test_task_costs_sorted_recent_first() {
    let (_dir, workspace) = create_test_workspace();

    write_task_transcript(
        &workspace,
        "task-old",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "old task"}),
            json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 100, "model": "gpt-5-mini"}),
            json!({"type": "session_end", "ts": 1002.0, "id": "e1", "status": "Complete", "iterations": 1}),
        ],
    );

    write_task_transcript(
        &workspace,
        "task-middle",
        &[
            json!({"type": "session_start", "ts": 5000.0, "id": "s2", "task": "middle task"}),
            json!({"type": "think_response", "ts": 5001.0, "id": "t2", "sats_effective": 200, "model": "gpt-5-mini"}),
            json!({"type": "session_end", "ts": 5002.0, "id": "e2", "status": "Complete", "iterations": 1}),
        ],
    );

    write_task_transcript(
        &workspace,
        "task-new",
        &[
            json!({"type": "session_start", "ts": 9000.0, "id": "s3", "task": "new task"}),
            json!({"type": "think_response", "ts": 9001.0, "id": "t3", "sats_effective": 300, "model": "gpt-5"}),
            json!({"type": "session_end", "ts": 9002.0, "id": "e3", "status": "Complete", "iterations": 1}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "task_costs", "count": 10})).await;

    let tasks = result["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 3);
    // Most recent first by started_at
    let ts0 = tasks[0]["started_at"].as_f64().unwrap_or(0.0);
    let ts1 = tasks[1]["started_at"].as_f64().unwrap_or(0.0);
    let ts2 = tasks[2]["started_at"].as_f64().unwrap_or(0.0);
    assert!(ts0 >= ts1, "First task should be most recent");
    assert!(ts1 >= ts2, "Second task should be more recent than third");
}

#[tokio::test]
async fn test_task_costs_count_limit() {
    let (_dir, workspace) = create_test_workspace();

    for i in 0..10 {
        write_task_transcript(
            &workspace,
            &format!("task-{i}"),
            &[
                json!({"type": "session_start", "ts": (1000 + i * 100) as f64, "id": format!("s{i}"), "task": format!("task {i}")}),
                json!({"type": "think_response", "ts": (1001 + i * 100) as f64, "id": format!("t{i}"), "sats_effective": 100, "model": "gpt-5-mini"}),
                json!({"type": "session_end", "ts": (1002 + i * 100) as f64, "id": format!("e{i}"), "status": "Complete", "iterations": 1}),
            ],
        );
    }

    let result = call_introspect(&workspace, json!({"action": "task_costs", "count": 2})).await;

    let tasks = result["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
}

#[tokio::test]
async fn test_task_costs_includes_model() {
    let (_dir, workspace) = create_test_workspace();

    write_task_transcript(
        &workspace,
        "task-model",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "model test"}),
            json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "claude-sonnet-4-20250514", "prompt_tokens": 200, "completion_tokens": 100}),
            json!({"type": "session_end", "ts": 1002.0, "id": "e1", "status": "Complete", "iterations": 1}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "task_costs", "count": 5})).await;

    let tasks = result["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["model"], "claude-sonnet-4-20250514");
}

#[tokio::test]
async fn test_task_costs_empty() {
    let (_dir, workspace) = create_test_workspace();

    let result = call_introspect(&workspace, json!({"action": "task_costs", "count": 5})).await;

    let tasks = result["tasks"].as_array().unwrap();
    assert!(tasks.is_empty());
    assert_eq!(result["total"], 0);
}

// ── task_detail ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_task_detail_basic() {
    let (_dir, workspace) = create_test_workspace();

    let events = vec![
        json!({"type": "session_start", "ts": 1677123454.0, "id": "s1", "task": "Research BSV fees"}),
        json!({"type": "think_response", "ts": 1677123455.0, "id": "t1", "role": "assistant", "content": "I'll search...", "model": "gpt-5-mini", "sats_effective": 500, "prompt_tokens": 1200, "completion_tokens": 300}),
        json!({"type": "tool_result", "ts": 1677123457.0, "id": "tr1", "role": "tool", "call_id": "call-1", "name": "web_fetch", "content": "result data", "success": true, "sats_paid": 100}),
        json!({"type": "session_end", "ts": 1677123460.0, "id": "e1", "status": "Complete", "iterations": 1}),
    ];
    write_task_transcript(&workspace, "task-detail-1", &events);

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-detail-1"}),
    )
    .await;

    assert_eq!(result["task_id"], "task-detail-1");
    assert_eq!(result["task"], "Research BSV fees");
    assert_eq!(result["model"], "gpt-5-mini");
    assert_eq!(result["status"], "Complete");
    assert_eq!(result["total_sats"], 600); // 500 LLM + 100 tool
    assert_eq!(result["prompt_tokens"], 1200);
    assert_eq!(result["completion_tokens"], 300);
}

#[tokio::test]
async fn test_task_detail_proofs() {
    let (_dir, workspace) = create_test_workspace();

    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "proof task"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini", "prompt_tokens": 100, "completion_tokens": 50}),
        json!({"type": "proof_created", "ts": 1002.0, "id": "p1", "txid": "abc123", "proof_type": "Decision", "hash": "hash1", "sats_cost": 200}),
        json!({"type": "proof_created", "ts": 1003.0, "id": "p2", "txid": "def456", "proof_type": "TaskCompletion", "hash": "hash2", "sats_cost": 200}),
        json!({"type": "session_end", "ts": 1004.0, "id": "e1", "status": "Complete", "iterations": 1}),
    ];
    write_task_transcript(&workspace, "task-with-proofs", &events);

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-with-proofs"}),
    )
    .await;

    let proofs = result["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 2);
    // Check that txids are present
    let txids: Vec<&str> = proofs.iter().filter_map(|p| p["txid"].as_str()).collect();
    assert!(txids.contains(&"abc123"));
    assert!(txids.contains(&"def456"));
}

#[tokio::test]
async fn test_task_detail_tools_used() {
    let (_dir, workspace) = create_test_workspace();

    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "tool task"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini"}),
        json!({"type": "tool_result", "ts": 1002.0, "id": "tr1", "name": "file_read", "sats_paid": 0, "success": true}),
        json!({"type": "tool_result", "ts": 1003.0, "id": "tr2", "name": "web_fetch", "sats_paid": 0, "success": true}),
        json!({"type": "tool_result", "ts": 1004.0, "id": "tr3", "name": "file_read", "sats_paid": 0, "success": true}),
        json!({"type": "session_end", "ts": 1005.0, "id": "e1", "status": "Complete", "iterations": 1}),
    ];
    write_task_transcript(&workspace, "task-tools", &events);

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-tools"}),
    )
    .await;

    let tools = result["tools_used"].as_array().unwrap();
    // file_read appears twice but should be deduped
    assert!(tools.contains(&json!("file_read")));
    assert!(tools.contains(&json!("web_fetch")));
    assert_eq!(tools.len(), 2);
}

#[tokio::test]
async fn test_task_detail_not_found() {
    let (_dir, workspace) = create_test_workspace();

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "nonexistent-task"}),
    )
    .await;

    // Result should contain an error string
    let raw = result.to_string();
    assert!(
        raw.contains("not found") || raw.contains("Error"),
        "Expected error for missing task, got: {raw}"
    );
}

#[tokio::test]
async fn test_task_detail_requires_task_id() {
    let (_dir, workspace) = create_test_workspace();

    let result = call_introspect(&workspace, json!({"action": "task_detail"})).await;

    let raw = result.to_string();
    assert!(
        raw.contains("task_id") && raw.contains("required"),
        "Expected task_id required error, got: {raw}"
    );
}

// ── activity_summary ────────────────────────────────────────────────────

#[tokio::test]
async fn test_activity_summary_basic() {
    let (_dir, workspace) = create_test_workspace();

    // Task 1: 500 sats LLM + 1 proof (200 sats)
    write_task_transcript(
        &workspace,
        "task-s1",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "task one"}),
            json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini"}),
            json!({"type": "proof_created", "ts": 1002.0, "id": "p1", "txid": "tx1", "sats_cost": 200}),
        ],
    );

    // Task 2: 800 sats LLM + 2 proofs
    write_task_transcript(
        &workspace,
        "task-s2",
        &[
            json!({"type": "session_start", "ts": 2000.0, "id": "s2", "task": "task two"}),
            json!({"type": "think_response", "ts": 2001.0, "id": "t2", "sats_effective": 800, "model": "gpt-5"}),
            json!({"type": "proof_created", "ts": 2002.0, "id": "p2", "txid": "tx2", "sats_cost": 200}),
            json!({"type": "proof_created", "ts": 2003.0, "id": "p3", "txid": "tx3", "sats_cost": 200}),
        ],
    );

    // Task 3: 300 sats LLM, no proofs
    write_task_transcript(
        &workspace,
        "task-s3",
        &[
            json!({"type": "session_start", "ts": 3000.0, "id": "s3", "task": "task three"}),
            json!({"type": "think_response", "ts": 3001.0, "id": "t3", "sats_effective": 300, "model": "gpt-5-mini"}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "activity_summary"})).await;

    assert_eq!(result["total_tasks"], 3);
    assert_eq!(result["total_proofs"], 3);
    // Total sats: 500 + 200 + 800 + 200 + 200 + 300 = 2200
    assert_eq!(result["total_sats_spent"], 2200);
}

#[tokio::test]
async fn test_activity_summary_empty() {
    let (_dir, workspace) = create_test_workspace();

    let result = call_introspect(&workspace, json!({"action": "activity_summary"})).await;

    assert_eq!(result["total_tasks"], 0);
    assert_eq!(result["total_proofs"], 0);
    assert_eq!(result["total_sats_spent"], 0);
}

#[tokio::test]
async fn test_activity_summary_service_breakdown() {
    let (_dir, workspace) = create_test_workspace();

    write_task_transcript(
        &workspace,
        "task-breakdown",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "breakdown test"}),
            json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini"}),
            json!({"type": "proof_created", "ts": 1002.0, "id": "p1", "txid": "tx1", "sats_cost": 200}),
        ],
    );

    // Also write budget.jsonl for complementary data
    write_budget_log(
        &workspace,
        &[
            json!({"timestamp": "2026-03-23T10:00:00Z", "service": "llm", "operation": "think", "sats": 500}),
            json!({"timestamp": "2026-03-23T10:01:00Z", "service": "proofs", "operation": "create_proof", "sats": 200}),
            json!({"timestamp": "2026-03-23T10:02:00Z", "service": "llm", "operation": "think", "sats": 300}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "activity_summary"})).await;

    // Transcript-derived services
    let services = &result["services"];
    assert_eq!(services["llm"], 500);
    assert_eq!(services["proofs"], 200);

    // Budget journal services
    let budget = &result["budget_journal"];
    assert_eq!(budget["total_sats"], 1000);
    assert_eq!(budget["services"]["llm"], 800); // 500 + 300
    assert_eq!(budget["services"]["proofs"], 200);
}

// ── edge cases ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_malformed_jsonl_skipped() {
    let (_dir, workspace) = create_test_workspace();

    let task_dir = workspace.join("tasks").join("task-bad");
    fs::create_dir_all(&task_dir).unwrap();
    let content = r#"{"type": "proof_created", "ts": 1000.0, "txid": "good-tx", "sats_cost": 200}
not valid json at all
{malformed also
{"type": "proof_created", "ts": 1001.0, "txid": "also-good", "sats_cost": 200}
"#;
    fs::write(task_dir.join("session.jsonl"), content).unwrap();

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 10})).await;

    let proofs = result["proofs"].as_array().unwrap();
    // Should have 2 valid proofs, malformed lines skipped
    assert_eq!(proofs.len(), 2);
    assert_eq!(result["total"], 2);
}

#[tokio::test]
async fn test_missing_fields_handled() {
    let (_dir, workspace) = create_test_workspace();

    // Events with missing optional fields — should not panic
    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1"}),
        // think_response without model, sats, or tokens
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "role": "assistant", "content": "response"}),
        // tool_result without sats_paid
        json!({"type": "tool_result", "ts": 1002.0, "id": "tr1", "name": "file_read", "success": true}),
        // proof_created without txid
        json!({"type": "proof_created", "ts": 1003.0, "id": "p1", "proof_type": "Decision"}),
        json!({"type": "session_end", "ts": 1004.0, "id": "e1"}),
    ];
    write_task_transcript(&workspace, "task-sparse", &events);

    // All actions should complete without panicking
    let result1 = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 5})).await;
    assert!(result1["proofs"].is_array());

    let result2 = call_introspect(&workspace, json!({"action": "task_costs", "count": 5})).await;
    assert!(result2["tasks"].is_array());

    let result3 = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-sparse"}),
    )
    .await;
    assert_eq!(result3["task_id"], "task-sparse");

    let result4 = call_introspect(&workspace, json!({"action": "activity_summary"})).await;
    assert_eq!(result4["total_tasks"], 1);
}

#[tokio::test]
async fn test_unknown_action_returns_error() {
    let (_dir, workspace) = create_test_workspace();

    let result = call_introspect(&workspace, json!({"action": "invalid_action"})).await;

    let raw = result.to_string();
    assert!(
        raw.contains("Error") || raw.contains("unknown action"),
        "Expected error for unknown action, got: {raw}"
    );
}

#[tokio::test]
async fn test_default_count_used() {
    let (_dir, workspace) = create_test_workspace();

    // Create many tasks
    for i in 0..10 {
        write_task_transcript(
            &workspace,
            &format!("task-def-{i}"),
            &[
                json!({"type": "session_start", "ts": (1000 + i * 100) as f64, "id": format!("s{i}"), "task": format!("task {i}")}),
                json!({"type": "think_response", "ts": (1001 + i * 100) as f64, "id": format!("t{i}"), "sats_effective": 100, "model": "gpt-5-mini"}),
                json!({"type": "session_end", "ts": (1002 + i * 100) as f64, "id": format!("e{i}"), "status": "Complete", "iterations": 1}),
            ],
        );
    }

    // Call without count param — should use DEFAULT_COUNT (5)
    let result = call_introspect(&workspace, json!({"action": "task_costs"})).await;

    let tasks = result["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 5);
}

#[tokio::test]
async fn test_tool_registration() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Arc::new(dir.path().to_path_buf());
    let tool = create_introspect_tool(ws);

    assert_eq!(tool.name, "introspect");
    assert_eq!(tool.category, "introspect");
    assert!(tool.cleanup.is_none());
    assert!(tool.description.contains("proofs"));
    assert!(tool.description.contains("costs"));
    assert!(tool.description.contains("task_detail"));
    assert!(tool.description.contains("activity_summary"));
}

#[tokio::test]
async fn test_task_costs_includes_tool_spending() {
    let (_dir, workspace) = create_test_workspace();

    write_task_transcript(
        &workspace,
        "task-tool-spend",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "tool spending"}),
            json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini"}),
            json!({"type": "tool_result", "ts": 1002.0, "id": "tr1", "name": "x402_call", "sats_paid": 150, "success": true}),
            json!({"type": "proof_created", "ts": 1003.0, "id": "p1", "txid": "tx1", "sats_cost": 200}),
            json!({"type": "session_end", "ts": 1004.0, "id": "e1", "status": "Complete", "iterations": 1}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "task_costs", "count": 5})).await;

    let tasks = result["tasks"].as_array().unwrap();
    // 500 (LLM) + 150 (tool) + 200 (proof) = 850
    assert_eq!(tasks[0]["total_sats"], 850);
}

#[tokio::test]
async fn test_task_detail_per_iteration_breakdown() {
    let (_dir, workspace) = create_test_workspace();

    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "multi-iter"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini", "prompt_tokens": 100, "completion_tokens": 50}),
        json!({"type": "tool_result", "ts": 1002.0, "id": "tr1", "name": "file_read", "sats_paid": 0, "success": true}),
        json!({"type": "think_response", "ts": 1003.0, "id": "t2", "sats_effective": 600, "model": "gpt-5-mini", "prompt_tokens": 200, "completion_tokens": 100}),
        json!({"type": "tool_result", "ts": 1004.0, "id": "tr2", "name": "web_fetch", "sats_paid": 50, "success": true}),
        json!({"type": "session_end", "ts": 1005.0, "id": "e1", "status": "Complete", "iterations": 2}),
    ];
    write_task_transcript(&workspace, "task-multi-iter", &events);

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-multi-iter"}),
    )
    .await;

    let per_iter = result["per_iteration"].as_array().unwrap();
    assert_eq!(per_iter.len(), 2);
    // First iteration should include LLM cost
    assert_eq!(per_iter[0]["iteration"], 1);
    assert!(per_iter[0]["sats"].as_u64().unwrap() >= 500);
    // Second iteration should include LLM cost + tool cost
    assert_eq!(per_iter[1]["iteration"], 2);
}

#[tokio::test]
async fn test_activity_summary_budget_journal_only() {
    let (_dir, workspace) = create_test_workspace();

    // No tasks, but budget.jsonl has entries
    write_budget_log(
        &workspace,
        &[
            json!({"timestamp": "2026-03-23T10:00:00Z", "service": "llm", "operation": "think", "sats": 450}),
            json!({"timestamp": "2026-03-23T10:01:00Z", "service": "proofs", "operation": "create_proof", "sats": 200}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "activity_summary"})).await;

    assert_eq!(result["total_tasks"], 0);
    let budget = &result["budget_journal"];
    assert_eq!(budget["total_sats"], 650);
}

#[tokio::test]
async fn test_session_jsonl_fallback() {
    // Tests that the tool reads session.jsonl (legacy name) when transcript.jsonl doesn't exist
    let (_dir, workspace) = create_test_workspace();

    // Write to session.jsonl (the legacy/standard name used by this project)
    write_task_transcript(
        &workspace,
        "task-legacy",
        &[
            json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "legacy task"}),
            json!({"type": "proof_created", "ts": 1001.0, "id": "p1", "txid": "legacy-tx", "proof_type": "Decision", "sats_cost": 200}),
        ],
    );

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 5})).await;

    let proofs = result["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0]["txid"], "legacy-tx");
}

#[tokio::test]
async fn test_count_clamped_to_max() {
    let (_dir, workspace) = create_test_workspace();

    // count=100 should be clamped to MAX_COUNT (20)
    let result =
        call_introspect(&workspace, json!({"action": "recent_proofs", "count": 100})).await;

    // With no data, just verify it doesn't error out
    assert!(result["proofs"].is_array());
    assert_eq!(result["total"], 0);
}

#[tokio::test]
async fn test_task_detail_duration_ms() {
    let (_dir, workspace) = create_test_workspace();

    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "timed task"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini"}),
        json!({"type": "session_end", "ts": 1010.0, "id": "e1", "status": "Complete", "iterations": 1}),
    ];
    write_task_transcript(&workspace, "task-timed", &events);

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-timed"}),
    )
    .await;

    // Duration should be (1010 - 1000) * 1000 = 10000 ms
    assert_eq!(result["duration_ms"], 10000);
}

// ── Summary field tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_recent_proofs_summary_field() {
    let (_dir, workspace) = create_test_workspace();
    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "test"}),
        json!({"type": "proof_created", "ts": 1001.0, "id": "p1", "txid": "abc", "proof_type": "Decision", "hash": "deadbeef01234567", "sats_cost": 200, "iteration": 1}),
        json!({"type": "proof_created", "ts": 1002.0, "id": "p2", "txid": "def", "proof_type": "TaskCompletion", "hash": "cafebabe01234567", "sats_cost": 200, "iteration": 2}),
    ];
    write_task_transcript(&workspace, "task-sum", &events);

    let result = call_introspect(&workspace, json!({"action": "recent_proofs", "count": 5})).await;
    let summary = result["summary"].as_str().expect("summary field missing");
    assert!(summary.contains("2 of 2 proofs"), "summary: {summary}");
    // Should include proof types from the first 3 (only 2 here)
    assert!(summary.contains("TaskCompletion"), "summary: {summary}");
    assert!(summary.contains("Decision"), "summary: {summary}");
}

#[tokio::test]
async fn test_recent_proofs_summary_empty() {
    let (_dir, workspace) = create_test_workspace();
    let result = call_introspect(&workspace, json!({"action": "recent_proofs"})).await;
    let summary = result["summary"].as_str().expect("summary field missing");
    assert_eq!(summary, "No proofs found.");
}

#[tokio::test]
async fn test_task_costs_summary_field() {
    let (_dir, workspace) = create_test_workspace();
    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "cost check"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500, "model": "gpt-5-mini"}),
        json!({"type": "session_end", "ts": 1002.0, "id": "e1", "status": "Complete", "iterations": 1}),
    ];
    write_task_transcript(&workspace, "task-cost", &events);

    let result = call_introspect(&workspace, json!({"action": "task_costs"})).await;
    let summary = result["summary"].as_str().expect("summary field missing");
    assert!(summary.contains("1 of 1 tasks"), "summary: {summary}");
    assert!(summary.contains("500 sats"), "summary: {summary}");
    assert!(summary.contains("task-cost"), "summary: {summary}");
    assert!(summary.contains("Complete"), "summary: {summary}");
}

#[tokio::test]
async fn test_task_detail_summary_field() {
    let (_dir, workspace) = create_test_workspace();
    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "Analyze BSV fees"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 800, "model": "gpt-5-mini"}),
        json!({"type": "tool_result", "ts": 1002.0, "id": "r1", "name": "web_fetch", "sats_paid": 0, "success": true}),
        json!({"type": "proof_created", "ts": 1003.0, "id": "p1", "txid": "tx1", "proof_type": "Decision", "sats_cost": 200}),
        json!({"type": "session_end", "ts": 1004.0, "id": "e1", "status": "Complete", "iterations": 1}),
    ];
    write_task_transcript(&workspace, "task-detail-sum", &events);

    let result = call_introspect(
        &workspace,
        json!({"action": "task_detail", "task_id": "task-detail-sum"}),
    )
    .await;
    let summary = result["summary"].as_str().expect("summary field missing");
    assert!(summary.contains("task-detail-sum"), "summary: {summary}");
    assert!(summary.contains("Analyze BSV fees"), "summary: {summary}");
    assert!(summary.contains("Complete"), "summary: {summary}");
    assert!(summary.contains("1000 sats"), "summary: {summary}"); // 800 + 200
    assert!(summary.contains("1 proofs"), "summary: {summary}");
    assert!(summary.contains("web_fetch"), "summary: {summary}");
}

#[tokio::test]
async fn test_activity_summary_summary_field() {
    let (_dir, workspace) = create_test_workspace();
    let events = vec![
        json!({"type": "session_start", "ts": 1000.0, "id": "s1", "task": "task one"}),
        json!({"type": "think_response", "ts": 1001.0, "id": "t1", "sats_effective": 500}),
        json!({"type": "proof_created", "ts": 1002.0, "id": "p1", "txid": "tx1", "sats_cost": 200}),
    ];
    write_task_transcript(&workspace, "task-as1", &events);

    let result = call_introspect(&workspace, json!({"action": "activity_summary"})).await;
    let summary = result["summary"].as_str().expect("summary field missing");
    assert!(summary.contains("1 tasks"), "summary: {summary}");
    assert!(summary.contains("1 proofs"), "summary: {summary}");
    assert!(summary.contains("700 sats"), "summary: {summary}");
    assert!(summary.contains("llm"), "summary: {summary}");
}
