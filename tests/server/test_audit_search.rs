//! Integration tests for GET /audit/search endpoint.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use dolphin_milk::server::AuditSearchResponse;
use std::path::PathBuf;

#[path = "../common/mod.rs"]
mod common;

/// Create a workspace with two task transcripts containing known events.
fn create_test_workspace() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    std::mem::forget(dir);

    // Task 1: has think_response with gpt-5-mini model and a tool call to execute_bash
    let task1_dir = workspace.join("tasks/task-alpha");
    std::fs::create_dir_all(&task1_dir).unwrap();
    let mut t1 = dolphin_milk::transcript::Transcript::new(task1_dir.join("session.jsonl"));
    t1.record_user("compute pi to 5 digits");
    t1.record_think_request(&[], "gpt-5-mini", 4096);
    t1.record_think_response(
        "I'll compute pi using a bash command",
        "gpt-5-mini",
        100,
        80,
        20,
        50,
        30,
        None,
        "stop",
        500,
    );
    t1.record_tool_call(
        "call-1",
        "execute_bash",
        &serde_json::json!({"command": "echo 3.14159"}),
    );
    t1.record_tool_result("call-1", "execute_bash", "3.14159", true, 0);
    t1.record_think_request(&[], "gpt-5-mini", 4096);
    t1.record_think_response(
        "Pi is approximately 3.14159",
        "gpt-5-mini",
        80,
        60,
        20,
        40,
        20,
        None,
        "stop",
        300,
    );

    // Task 2: has a memory_search tool call and an error event
    let task2_dir = workspace.join("tasks/task-beta");
    std::fs::create_dir_all(&task2_dir).unwrap();
    let mut t2 = dolphin_milk::transcript::Transcript::new(task2_dir.join("session.jsonl"));
    t2.record_user("search for knowledge about BSV");
    t2.record_think_request(&[], "claude-sonnet-4-20250514", 8192);
    t2.record_think_response(
        "Let me search memory for BSV knowledge",
        "claude-sonnet-4-20250514",
        200,
        180,
        20,
        100,
        50,
        None,
        "stop",
        800,
    );
    t2.record_tool_call(
        "call-2",
        "memory_search",
        &serde_json::json!({"query": "BSV micropayments"}),
    );
    t2.record_tool_result(
        "call-2",
        "memory_search",
        "Found 3 results about BSV",
        true,
        0,
    );
    t2.record_error("Payment failed: insufficient funds", None);

    workspace
}

// ---------------------------------------------------------------------------
// Test 1: Endpoint exists and returns 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_endpoint_exists() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=pi")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Test 2: Empty/missing query returns 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_empty_query_returns_error() {
    let app = common::test_router().await;

    // No q param at all
    let req = Request::builder()
        .uri("/audit/search")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Empty q param
    let req = Request::builder()
        .uri("/audit/search?q=")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Whitespace-only q param
    let req = Request::builder()
        .uri("/audit/search?q=%20%20")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Test 3: Search finds think events by model name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_finds_think_event() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=gpt-5-mini")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(search.query, "gpt-5-mini");
    // gpt-5-mini appears in think_request (2x model field) and think_response (2x model field)
    // from task-alpha only
    assert!(
        search.total >= 4,
        "expected >=4 matches for gpt-5-mini, got {}",
        search.total
    );

    // All results should be from task-alpha
    for result in &search.results {
        assert_eq!(result.task_id, "task-alpha");
    }
}

// ---------------------------------------------------------------------------
// Test 4: Search finds tool_call events by tool name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_finds_tool_call() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=memory_search")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert!(
        search.total >= 1,
        "expected at least 1 match for memory_search, got {}",
        search.total
    );

    // Should find the tool_call or tool_result event
    let has_tool_event = search.results.iter().any(|r| {
        (r.event_type == "tool_call" || r.event_type == "tool_result") && r.task_id == "task-beta"
    });
    assert!(
        has_tool_event,
        "expected to find tool event for memory_search"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Query matching nothing returns empty array
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_no_results() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=xyzzy_nonexistent_query_12345")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(search.total, 0);
    assert!(search.results.is_empty());
    assert_eq!(search.query, "xyzzy_nonexistent_query_12345");
}

// ---------------------------------------------------------------------------
// Test 6: Pagination with limit and offset
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_pagination() {
    let workspace = create_test_workspace();

    // First, get total count
    let app = common::test_router_for_workspace(workspace.clone()).await;
    let req = Request::builder()
        .uri("/audit/search?q=gpt-5-mini&limit=100")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let full: AuditSearchResponse = serde_json::from_slice(&body).unwrap();
    let total = full.total;
    assert!(total >= 2, "need at least 2 results to test pagination");

    // Now request with limit=1
    let app = common::test_router_for_workspace(workspace.clone()).await;
    let req = Request::builder()
        .uri("/audit/search?q=gpt-5-mini&limit=1&offset=0")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let page1: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(page1.results.len(), 1);
    assert_eq!(page1.total, total); // total stays the same
    assert_eq!(page1.limit, 1);
    assert_eq!(page1.offset, 0);

    // Page 2 with offset=1
    let app = common::test_router_for_workspace(workspace).await;
    let req = Request::builder()
        .uri("/audit/search?q=gpt-5-mini&limit=1&offset=1")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let page2: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(page2.results.len(), 1);
    assert_eq!(page2.offset, 1);

    // Results should be different between page 1 and page 2
    // (different events or at least different timestamps)
    assert_ne!(
        page1.results[0].timestamp, page2.results[0].timestamp,
        "Paginated pages should return different events"
    );
}

// ---------------------------------------------------------------------------
// Test 7: Finds events from multiple task transcripts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_across_tasks() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    // Search for "think_response" which appears in both tasks as event type.
    // Actually, let's search for something that appears in both transcripts.
    // Both tasks have "user" events — search for content from user messages.
    // Better: search for a common string. Both have think_request events.
    // Let's search for "stop" which is the finish_reason in think_response of both tasks.
    let req = Request::builder()
        .uri("/audit/search?q=stop")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    // Both tasks have think_response events with finish_reason "stop"
    let task_ids: std::collections::HashSet<&str> =
        search.results.iter().map(|r| r.task_id.as_str()).collect();

    assert!(
        task_ids.contains("task-alpha"),
        "expected results from task-alpha"
    );
    assert!(
        task_ids.contains("task-beta"),
        "expected results from task-beta"
    );
    assert!(
        task_ids.len() >= 2,
        "expected results from at least 2 tasks"
    );
}

// ---------------------------------------------------------------------------
// Test 8: Result format includes required fields
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_result_format() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=execute_bash")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();

    // Verify the raw JSON structure has the required fields
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Top-level fields
    assert!(json.get("query").is_some(), "missing 'query' field");
    assert!(json.get("results").is_some(), "missing 'results' field");
    assert!(json.get("total").is_some(), "missing 'total' field");
    assert!(json.get("limit").is_some(), "missing 'limit' field");
    assert!(json.get("offset").is_some(), "missing 'offset' field");

    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "expected at least one result");

    // Each result should have task_id, event_type, timestamp, matched_content
    for result in results {
        assert!(result.get("task_id").is_some(), "missing 'task_id'");
        assert!(result.get("event_type").is_some(), "missing 'event_type'");
        assert!(result.get("timestamp").is_some(), "missing 'timestamp'");
        assert!(
            result.get("matched_content").is_some(),
            "missing 'matched_content'"
        );

        // task_id should be a non-empty string
        assert!(!result["task_id"].as_str().unwrap().is_empty());
        // event_type should be a known transcript event type
        assert!(!result["event_type"].as_str().unwrap().is_empty());
        // timestamp should be a positive number
        assert!(result["timestamp"].as_f64().unwrap() > 0.0);
        // matched_content should contain the search term
        let content = result["matched_content"].as_str().unwrap().to_lowercase();
        assert!(
            content.contains("execute_bash"),
            "matched_content should contain the query, got: {content}"
        );
    }

    // Also verify the typed deserialization works
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(search.query, "execute_bash");
    assert_eq!(search.limit, 20); // default
    assert_eq!(search.offset, 0); // default
}

// ---------------------------------------------------------------------------
// Test 9 (bonus): Case-insensitive search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_case_insensitive() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    // Search for "GPT-5-MINI" (uppercase) should find "gpt-5-mini" events
    let req = Request::builder()
        .uri("/audit/search?q=GPT-5-MINI")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert!(
        search.total >= 1,
        "case-insensitive search should find results"
    );
}

// ---------------------------------------------------------------------------
// Test 10 (bonus): Limit capped at 100
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_limit_capped() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=stop&limit=999")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    // Limit should be capped to 100
    assert_eq!(search.limit, 100);
}

// ---------------------------------------------------------------------------
// Test 11 (bonus): Search finds error events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audit_search_finds_error_events() {
    let workspace = create_test_workspace();
    let app = common::test_router_for_workspace(workspace).await;

    let req = Request::builder()
        .uri("/audit/search?q=insufficient+funds")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search: AuditSearchResponse = serde_json::from_slice(&body).unwrap();

    assert!(search.total >= 1, "should find the error event");
    let error_hit = search.results.iter().find(|r| r.event_type == "error");
    assert!(error_hit.is_some(), "should find an error event type");
    assert_eq!(error_hit.unwrap().task_id, "task-beta");
}
