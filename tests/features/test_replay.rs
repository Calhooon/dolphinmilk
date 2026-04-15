//! Tests for replay infrastructure — ReplayViewer, ForkExecutor, and server routes.

use serde_json::{json, Value};
use std::collections::HashMap;
use tempfile::TempDir;

use dolphin_milk::replay::{ForkExecutor, ForkParams, ReplayViewer};
use dolphin_milk::transcript::Transcript;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temporary transcript with realistic events.
fn create_test_transcript() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let task_dir = dir.path().join("tasks/test-task-123");
    std::fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("session.jsonl");

    let mut t = Transcript::new(path.clone());

    // session_start
    let mut data = HashMap::new();
    data.insert("task".to_string(), json!("Test task"));
    t.record("session_start", data);

    // user message
    t.record_user("What is the capital of France?");

    // think_request
    t.record_think_request(&[], "gpt-5-mini", 4096);

    // think_response with tool calls
    t.record_think_response(
        "Let me search for that.",
        "gpt-5-mini",
        500, // sats_paid
        450, // sats_effective
        50,  // sats_refunded
        100, // prompt_tokens
        50,  // completion_tokens
        Some(&[json!({
            "id": "call_1",
            "type": "function",
            "function": {
                "name": "web_search",
                "arguments": "{\"query\": \"capital of France\"}"
            }
        })]),
        "stop",
        250, // duration_ms
    );

    // tool_call
    t.record_tool_call(
        "call_1",
        "web_search",
        &json!({"query": "capital of France"}),
    );

    // tool_result
    t.record_tool_result(
        "call_1",
        "web_search",
        "Paris is the capital of France.",
        true,
        100,
    );

    // budget_check
    t.record_budget(50000, 550, 550, 20000000);

    // think_request (second iteration)
    t.record_think_request(&[], "gpt-5-mini", 4096);

    // think_response (final answer)
    t.record_think_response(
        "The capital of France is Paris.",
        "gpt-5-mini",
        400,
        380,
        20,
        200,
        30,
        None,
        "stop",
        180,
    );

    // session_end
    let mut end_data = HashMap::new();
    end_data.insert("iterations".to_string(), json!(2));
    end_data.insert("sats_spent".to_string(), json!(930));
    t.record("session_end", end_data);

    (dir, path)
}

/// Create a minimal transcript with just a few events.
fn create_minimal_transcript() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.jsonl");

    let mut t = Transcript::new(path.clone());
    t.record_user("Hello world");
    t.record_system("You are a helpful assistant");

    (dir, path)
}

// ---------------------------------------------------------------------------
// ReplayViewer tests
// ---------------------------------------------------------------------------

#[test]
fn test_replay_viewer_reads_correct_event_count() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    // session_start + user + think_request + think_response + tool_call +
    // tool_result + budget_check + think_request + think_response + session_end = 10
    assert_eq!(viewer.event_count(), 10);
}

#[test]
fn test_replay_viewer_build_timeline() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    assert_eq!(timeline.total_events, 10);
    assert_eq!(timeline.total_iterations, 2); // two think_request events
                                              // sats: 450 (first think_response effective) + 100 (tool_result) + 380 (second think_response effective) = 930
    assert_eq!(timeline.total_sats, 930);
    assert_eq!(timeline.events.len(), 10);
    assert_eq!(timeline.task_id, "test-task-123");
}

#[test]
fn test_replay_viewer_event_types_in_timeline() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    let event_types: Vec<&str> = timeline
        .events
        .iter()
        .map(|e| e.event_type.as_str())
        .collect();
    assert_eq!(
        event_types,
        vec![
            "session_start",
            "user",
            "think_request",
            "think_response",
            "tool_call",
            "tool_result",
            "budget_check",
            "think_request",
            "think_response",
            "session_end",
        ]
    );
}

#[test]
fn test_replay_viewer_iteration_tracking() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    // session_start and user are iteration 0 (before first think_request)
    assert_eq!(timeline.events[0].iteration, 0);
    assert_eq!(timeline.events[1].iteration, 0);
    // After first think_request, iteration is 1
    assert_eq!(timeline.events[2].iteration, 1);
    assert_eq!(timeline.events[3].iteration, 1);
    // After second think_request, iteration is 2
    assert_eq!(timeline.events[7].iteration, 2);
    assert_eq!(timeline.events[8].iteration, 2);
}

#[test]
fn test_replay_viewer_cumulative_cost() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    // Before any spending, cumulative should be 0
    assert_eq!(timeline.events[0].cumulative_sats, 0); // session_start
    assert_eq!(timeline.events[1].cumulative_sats, 0); // user
    assert_eq!(timeline.events[2].cumulative_sats, 0); // think_request
                                                       // First think_response: 450 sats_effective
    assert_eq!(timeline.events[3].cumulative_sats, 450);
    assert_eq!(timeline.events[3].event_sats, 450);
    // tool_call: no cost
    assert_eq!(timeline.events[4].cumulative_sats, 450);
    // tool_result: 100 sats
    assert_eq!(timeline.events[5].cumulative_sats, 550);
    assert_eq!(timeline.events[5].event_sats, 100);
    // budget_check: no cost
    assert_eq!(timeline.events[6].cumulative_sats, 550);
    // Second think_response: 380 sats
    assert_eq!(timeline.events[8].cumulative_sats, 930);
}

#[test]
fn test_replay_viewer_cost_timeline() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    // Only events with cost > 0 appear in cost_timeline
    assert_eq!(timeline.cost_timeline.len(), 3);
    assert_eq!(timeline.cost_timeline[0].event_type, "think_response");
    assert_eq!(timeline.cost_timeline[0].cumulative_sats, 450);
    assert_eq!(timeline.cost_timeline[1].event_type, "tool_result");
    assert_eq!(timeline.cost_timeline[1].cumulative_sats, 550);
    assert_eq!(timeline.cost_timeline[2].event_type, "think_response");
    assert_eq!(timeline.cost_timeline[2].cumulative_sats, 930);
}

#[test]
fn test_replay_viewer_tool_usage() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    assert_eq!(timeline.tool_usage.len(), 1);
    assert_eq!(timeline.tool_usage[0].name, "web_search");
    assert_eq!(timeline.tool_usage[0].iteration, 1);
    assert_eq!(timeline.tool_usage[0].success, Some(true));
    assert_eq!(timeline.tool_usage[0].sats_paid, 100);
    assert!(timeline.tool_usage[0].result_index.is_some());
}

#[test]
fn test_replay_viewer_seeking_get_event() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);

    // Seek to event 0
    let event = viewer.get_event(0).unwrap();
    assert_eq!(event.index, 0);
    assert_eq!(event.event_type, "session_start");
    assert_eq!(event.cumulative_sats, 0);

    // Seek to event 5 (tool_result)
    let event = viewer.get_event(5).unwrap();
    assert_eq!(event.index, 5);
    assert_eq!(event.event_type, "tool_result");
    assert_eq!(event.cumulative_sats, 550);
    assert_eq!(event.tool_name, Some("web_search".to_string()));

    // Seek to event 8 (second think_response)
    let event = viewer.get_event(8).unwrap();
    assert_eq!(event.index, 8);
    assert_eq!(event.event_type, "think_response");
    assert_eq!(event.iteration, 2);
    assert_eq!(event.cumulative_sats, 930);
    assert_eq!(event.model, Some("gpt-5-mini".to_string()));

    // Out of bounds
    assert!(viewer.get_event(100).is_none());
}

#[test]
fn test_replay_viewer_get_events_range() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);

    let range = viewer.get_events_range(2, 5);
    assert_eq!(range.len(), 3);
    assert_eq!(range[0].index, 2);
    assert_eq!(range[0].event_type, "think_request");
    assert_eq!(range[1].index, 3);
    assert_eq!(range[1].event_type, "think_response");
    assert_eq!(range[2].index, 4);
    assert_eq!(range[2].event_type, "tool_call");
}

#[test]
fn test_replay_viewer_range_clamped() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);

    // Request beyond end — should clamp
    let range = viewer.get_events_range(8, 100);
    assert_eq!(range.len(), 2); // events 8, 9
    assert_eq!(range[0].index, 8);
    assert_eq!(range[1].index, 9);
}

#[test]
fn test_replay_viewer_empty_transcript() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.jsonl");
    std::fs::write(&path, "").unwrap();

    let viewer = ReplayViewer::new(&path);
    assert_eq!(viewer.event_count(), 0);
    let timeline = viewer.build_timeline();
    assert_eq!(timeline.total_events, 0);
    assert_eq!(timeline.total_sats, 0);
    assert_eq!(timeline.total_iterations, 0);
    assert_eq!(timeline.duration_secs, 0.0);
}

#[test]
fn test_replay_viewer_model_metadata() {
    let (_dir, path) = create_test_transcript();
    let viewer = ReplayViewer::new(&path);
    let timeline = viewer.build_timeline();

    // think_request should have model
    assert_eq!(timeline.events[2].model, Some("gpt-5-mini".to_string()));
    // think_response should have model
    assert_eq!(timeline.events[3].model, Some("gpt-5-mini".to_string()));
    // user event should NOT have model
    assert_eq!(timeline.events[1].model, None);
}

// ---------------------------------------------------------------------------
// ForkExecutor tests
// ---------------------------------------------------------------------------

#[test]
fn test_fork_executor_reconstructs_messages() {
    let (_dir, path) = create_test_transcript();

    let params = ForkParams {
        event_index: 6, // up to but not including budget_check
        message: None,
        model: None,
        max_iterations: None,
    };

    let result = ForkExecutor::prepare_fork(&path, &params).unwrap();

    assert_eq!(result.original_task, "What is the capital of France?");
    assert_eq!(result.events_consumed, 6);

    // Should have: user, assistant (with tool_calls), tool result = 3 messages
    // (session_start, think_request, tool_call are not message-producing events)
    assert_eq!(result.prior_messages.len(), 3);

    // First message is user
    assert_eq!(result.prior_messages[0]["role"], "user");
    assert_eq!(
        result.prior_messages[0]["content"],
        "What is the capital of France?"
    );

    // Second message is assistant with tool calls
    assert_eq!(result.prior_messages[1]["role"], "assistant");
    assert!(result.prior_messages[1]["tool_calls"].is_array());

    // Third message is tool result
    assert_eq!(result.prior_messages[2]["role"], "tool");
    assert_eq!(result.prior_messages[2]["tool_call_id"], "call_1");
}

#[test]
fn test_fork_executor_iteration_and_cost() {
    let (_dir, path) = create_test_transcript();

    let params = ForkParams {
        event_index: 7, // up to budget_check (includes think_request + think_response + tool_result)
        message: None,
        model: None,
        max_iterations: None,
    };

    let result = ForkExecutor::prepare_fork(&path, &params).unwrap();
    assert_eq!(result.fork_iteration, 1); // one think_request seen
    assert_eq!(result.original_sats_at_fork, 550); // 450 + 100
}

#[test]
fn test_fork_executor_at_start() {
    let (_dir, path) = create_test_transcript();

    let params = ForkParams {
        event_index: 0, // fork from the very beginning
        message: None,
        model: None,
        max_iterations: None,
    };

    let result = ForkExecutor::prepare_fork(&path, &params).unwrap();
    assert_eq!(result.prior_messages.len(), 0); // no messages yet
    assert_eq!(result.fork_iteration, 0);
    assert_eq!(result.original_sats_at_fork, 0);
}

#[test]
fn test_fork_executor_at_end() {
    let (_dir, path) = create_test_transcript();

    let params = ForkParams {
        event_index: 10, // after all events
        message: None,
        model: None,
        max_iterations: None,
    };

    let result = ForkExecutor::prepare_fork(&path, &params).unwrap();
    assert_eq!(result.events_consumed, 10);
    assert_eq!(result.fork_iteration, 2);
    assert_eq!(result.original_sats_at_fork, 930);
    // user + assistant(1) + tool + assistant(2) = 4 messages
    assert_eq!(result.prior_messages.len(), 4);
}

#[test]
fn test_fork_executor_nonexistent_transcript() {
    let params = ForkParams {
        event_index: 0,
        message: None,
        model: None,
        max_iterations: None,
    };

    let result = ForkExecutor::prepare_fork(std::path::Path::new("/nonexistent.jsonl"), &params);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn test_fork_executor_index_beyond_length() {
    let (_dir, path) = create_minimal_transcript();

    let params = ForkParams {
        event_index: 999,
        message: None,
        model: None,
        max_iterations: None,
    };

    let result = ForkExecutor::prepare_fork(&path, &params);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("exceeds"));
}

#[test]
fn test_fork_executor_with_custom_message() {
    let (_dir, path) = create_test_transcript();

    let params = ForkParams {
        event_index: 6,
        message: Some("Try again with a different approach".to_string()),
        model: Some("claude-sonnet-4-6".to_string()),
        max_iterations: Some(5),
    };

    let result = ForkExecutor::prepare_fork(&path, &params).unwrap();
    // The fork result preserves the original task regardless of the override message
    assert_eq!(result.original_task, "What is the capital of France?");
}

// ---------------------------------------------------------------------------
// Server route tests (axum oneshot)
// ---------------------------------------------------------------------------

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn test_replay_route_returns_structured_data() {
    let (app, workspace) = common::test_router_with_workspace().await;

    // Create a task transcript in the workspace
    let task_id = "replay-test-001";
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut t = Transcript::new(transcript_path);
    t.record_user("Test replay");
    t.record_think_request(&[], "gpt-5-mini", 4096);
    t.record_think_response(
        "The answer is 42.",
        "gpt-5-mini",
        300,
        280,
        20,
        100,
        50,
        None,
        "stop",
        200,
    );
    let mut end_data = HashMap::new();
    end_data.insert("iterations".to_string(), json!(1));
    t.record("session_end", end_data);

    let req = Request::builder()
        .uri(format!("/task/{task_id}/replay"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let data: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(data["task_id"], task_id);
    assert_eq!(data["total_events"], 4); // user + think_request + think_response + session_end
    assert_eq!(data["total_iterations"], 1);
    assert_eq!(data["total_sats"], 280); // sats_effective from think_response

    // Check events array structure
    let events = data["events"].as_array().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0]["event_type"], "user");
    assert_eq!(events[1]["event_type"], "think_request");
    assert_eq!(events[2]["event_type"], "think_response");

    // Check cost_timeline
    let cost_timeline = data["cost_timeline"].as_array().unwrap();
    assert_eq!(cost_timeline.len(), 1); // only one spending event
    assert_eq!(cost_timeline[0]["cumulative_sats"], 280);

    // Check tool_usage (should be empty)
    let tool_usage = data["tool_usage"].as_array().unwrap();
    assert_eq!(tool_usage.len(), 0);
}

#[tokio::test]
async fn test_replay_route_404_missing_task() {
    let (app, _workspace) = common::test_router_with_workspace().await;

    let req = Request::builder()
        .uri("/task/nonexistent/replay")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_fork_route_creates_new_task() {
    let (app, workspace) = common::test_router_with_workspace().await;

    // Create a source task transcript
    let source_task_id = "fork-source-001";
    let task_dir = workspace.join(format!("tasks/{source_task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut t = Transcript::new(transcript_path);
    t.record_user("Test fork source");
    t.record_think_request(&[], "gpt-5-mini", 4096);
    t.record_think_response(
        "Some response",
        "gpt-5-mini",
        300,
        280,
        20,
        100,
        50,
        None,
        "stop",
        200,
    );

    let fork_body = json!({
        "event_index": 2,
        "message": "Fork test"
    });

    let req = Request::builder()
        .method("POST")
        .uri(format!("/task/{source_task_id}/fork"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&fork_body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let data: Value = serde_json::from_slice(&body).unwrap();

    // Should have a new task_id and session_id
    assert!(data["task_id"].as_str().is_some());
    assert!(data["session_id"].as_str().is_some());
    assert_eq!(data["original_task"], "Test fork source");
    assert_eq!(data["events_consumed"], 2);
    assert_eq!(data["fork_iteration"], 1); // one think_request before index 2

    // Verify fork context file was written
    let fork_task_id = data["task_id"].as_str().unwrap();
    let fork_meta_path = workspace.join(format!("tasks/{fork_task_id}/fork_context.json"));
    assert!(fork_meta_path.exists());

    let fork_meta: Value =
        serde_json::from_str(&std::fs::read_to_string(&fork_meta_path).unwrap()).unwrap();
    assert_eq!(fork_meta["source_task_id"], source_task_id);
    assert_eq!(fork_meta["event_index"], 2);
}

#[tokio::test]
async fn test_fork_route_404_missing_task() {
    let (app, _workspace) = common::test_router_with_workspace().await;

    let fork_body = json!({
        "event_index": 0
    });

    let req = Request::builder()
        .method("POST")
        .uri("/task/nonexistent/fork")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&fork_body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_fork_route_bad_event_index() {
    let (app, workspace) = common::test_router_with_workspace().await;

    // Create a source task with 2 events
    let source_task_id = "fork-bad-index";
    let task_dir = workspace.join(format!("tasks/{source_task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut t = Transcript::new(transcript_path);
    t.record_user("Short task");
    t.record_system("Done");

    let fork_body = json!({
        "event_index": 999
    });

    let req = Request::builder()
        .method("POST")
        .uri(format!("/task/{source_task_id}/fork"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&fork_body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert!(data["error"].as_str().unwrap().contains("exceeds"));
}
