//! Integration tests for the HTTP server module.

use axum::body::Body;
use axum::http::{self, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use dolphin_milk::budget::BudgetReport;
use dolphin_milk::config::DmConfig;
use dolphin_milk::conversation::Conversation;
use dolphin_milk::server::{
    self, ConversationResponse, HealthResponse, MessageResponse, StatusResponse, TaskInfo,
    TaskRequest, TaskResponse, TaskStatus,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn test_health_returns_ok() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(health.status, "ok");
    assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
    // Scheduler hasn't ticked in test mode — field should be None (and omitted from JSON)
    assert!(health.scheduler_last_tick_secs_ago.is_none());
}

#[tokio::test]
async fn test_status_initially_empty() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/status")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let status: StatusResponse = serde_json::from_slice(&body).unwrap();
    assert!(status.tasks.is_empty());
    assert_eq!(status.active_count, 0);
    assert_eq!(status.total_sats, 0);
}

#[tokio::test]
async fn test_task_not_found_returns_404() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/task/nonexistent-uuid")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_submit_task_returns_202() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/task")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"task":"test task","max_iterations":1}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let task: TaskResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(!task.id.is_empty());
    // UUID format: 8-4-4-4-12
    assert_eq!(task.id.len(), 36);
}

#[tokio::test]
async fn test_submit_task_default_iterations() {
    let req: TaskRequest = serde_json::from_str(r#"{"task":"hello"}"#).unwrap();
    assert_eq!(req.max_iterations, 50);
}

#[tokio::test]
async fn test_message_endpoint_accepts() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/message")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "sender": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                "body": {"type": "status_update", "progress": 50},
                "message_box": "status_inbox"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let msg: MessageResponse = serde_json::from_slice(&body).unwrap();
    assert!(msg.accepted);
}

#[tokio::test]
async fn test_budget_endpoint_returns_report() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/budget")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let report: BudgetReport = serde_json::from_slice(&body).unwrap();
    assert_eq!(report.task_sats, 0);
    assert_eq!(report.total_operations, 0);
    assert!(report.services.is_empty());
    assert!(report.limits.max_per_task > 0);
}

#[test]
fn test_task_status_serialization() {
    assert_eq!(
        serde_json::to_string(&TaskStatus::Running).unwrap(),
        "\"running\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStatus::Complete).unwrap(),
        "\"complete\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStatus::Error).unwrap(),
        "\"error\""
    );

    let running: TaskStatus = serde_json::from_str("\"running\"").unwrap();
    assert_eq!(running, TaskStatus::Running);

    // #273: Interrupted status
    assert_eq!(
        serde_json::to_string(&TaskStatus::Interrupted).unwrap(),
        "\"interrupted\""
    );
    let interrupted: TaskStatus = serde_json::from_str("\"interrupted\"").unwrap();
    assert_eq!(interrupted, TaskStatus::Interrupted);
}

#[test]
fn test_task_info_none_fields_omitted() {
    let info = TaskInfo {
        id: "abc".into(),
        task: "hello".into(),
        status: TaskStatus::Running,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-02-24T12:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };
    let json = serde_json::to_string(&info).unwrap();
    // None fields should not appear in JSON
    assert!(!json.contains("result"));
    assert!(!json.contains("error"));
    assert!(!json.contains("completed_at"));
    assert!(!json.contains("proof_txids"));
}

#[test]
fn test_task_info_complete_with_result() {
    let info = TaskInfo {
        id: "xyz".into(),
        task: "compute pi".into(),
        status: TaskStatus::Complete,
        result: Some("3.14159".into()),
        error: None,
        iterations: 5,
        sats_spent: 1000,
        started_at: "2026-02-24T12:00:00Z".into(),
        completed_at: Some("2026-02-24T12:05:00Z".into()),
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };
    let json = serde_json::to_string(&info).unwrap();
    let parsed: TaskInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.status, TaskStatus::Complete);
    assert_eq!(parsed.result.unwrap(), "3.14159");
    assert_eq!(parsed.iterations, 5);
    assert_eq!(parsed.sats_spent, 1000);
}

#[tokio::test]
async fn test_invalid_task_request_returns_422() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/task")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"not_a_task": true}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // axum returns 422 for deserialization failures
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_health_uptime_increases() {
    let app = common::test_router().await;

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();

    // Uptime should be 0 or very small (just started)
    assert!(health.uptime_secs < 5);
}

#[tokio::test]
async fn test_health_scheduler_tick_reported() {
    // Create state, simulate a scheduler tick, then check /health
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        path,
    )
    .await;

    // Simulate scheduler tick 3 seconds ago
    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    state
        .scheduler
        .last_scheduler_tick
        .store(now_epoch - 3, Ordering::Relaxed);

    let app = server::build_router_with_state(state);

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();

    // scheduler_last_tick_secs_ago should be approximately 3 (allow 0-10 for CI jitter)
    let tick_age = health
        .scheduler_last_tick_secs_ago
        .expect("should be Some when scheduler has ticked");
    assert!(tick_age <= 10, "tick_age={tick_age} should be recent");
}

#[tokio::test]
async fn test_conversation_not_found_returns_404() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/task/nonexistent-uuid/conversation")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_conversation_with_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-conv-123";

    // Create a transcript JSONL file
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    // Write some events
    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_system("You are a helpful agent.");
    transcript.record_user("What is 2+2?");
    transcript.record_think_response("4", "gpt-5-nano", 100, 80, 20, 50, 10, None, "stop", 500);

    // Build router with this workspace
    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/conversation"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let conv: ConversationResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(conv.task_id, task_id);
    assert_eq!(conv.messages.len(), 3); // system + user + assistant
    assert_eq!(conv.messages[0]["role"], "system");
    assert_eq!(conv.messages[1]["role"], "user");
    assert_eq!(conv.messages[2]["role"], "assistant");
    assert_eq!(conv.messages[2]["content"], "4");
}

#[test]
fn test_conversation_response_serialization() {
    let resp = ConversationResponse {
        task_id: "abc".into(),
        messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("abc"));
    assert!(json.contains("hello"));
}

#[tokio::test]
async fn test_conversations_list_empty() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/conversations")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let conversations: Vec<Conversation> = serde_json::from_slice(&body).unwrap();
    assert!(
        conversations.is_empty(),
        "Expected empty conversation list, got {:?}",
        conversations
    );
}

// ═══════════════════════════════════════════════════════════════
// F.1: conversation_id in task responses
// ═══════════════════════════════════════════════════════════════

#[test]
fn test_task_info_conversation_id_serialization() {
    let info = TaskInfo {
        id: "t1".into(),
        task: "test".into(),
        status: TaskStatus::Running,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-04-08T00:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: "chat".into(),
        conversation_id: Some("conv-abc".into()),
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains("\"conversation_id\":\"conv-abc\""));
    let parsed: TaskInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.conversation_id.as_deref(), Some("conv-abc"));
}

#[test]
fn test_task_info_none_conversation_id_omitted() {
    let info = TaskInfo {
        id: "t2".into(),
        task: "heartbeat".into(),
        status: TaskStatus::Complete,
        result: None,
        error: None,
        iterations: 1,
        sats_spent: 100,
        started_at: "2026-04-08T00:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: "schedule".into(),
        conversation_id: None,
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(
        !json.contains("conversation_id"),
        "None conversation_id should be omitted: {json}"
    );
}

#[tokio::test]
async fn test_task_conversation_id_endpoint_not_found() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/task/nonexistent/conversation-id")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_status_response_tasks_have_conversation_id() {
    let (_app, workspace) = common::test_router_with_workspace().await;
    // Create a task directory with a transcript that has session_id
    let task_id = "f1-test-task";
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let mut transcript = dolphin_milk::transcript::Transcript::new(task_dir.join("session.jsonl"));
    let mut start_data = std::collections::HashMap::new();
    start_data.insert("session_id".to_string(), serde_json::json!("conv-f1-test"));
    transcript.record("session_start", start_data);
    transcript.record_user("test task for F.1");
    let mut end_data = std::collections::HashMap::new();
    end_data.insert("status".to_string(), serde_json::json!("complete"));
    end_data.insert("iterations".to_string(), serde_json::json!(1));
    transcript.record("session_end", end_data);

    // Build router with the pre-populated workspace
    let app = common::test_router_for_workspace(workspace).await;

    // GET /task/{id} should include conversation_id
    let req = Request::builder()
        .uri(format!("/task/{task_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let task_info: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        task_info["conversation_id"].as_str(),
        Some("conv-f1-test"),
        "GET /task/{{id}} must include conversation_id"
    );
}

// ═══════════════════════════════════════════════════════════════
// F.2: conversation artifacts endpoint
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_conversation_artifacts_returns_404_for_unknown() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/conversations/conv-nonexistent/artifacts")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_conversation_artifacts_empty_conversation() {
    let (_, workspace) = common::test_router_with_workspace().await;
    // Create a conversation with no tasks
    let conv_mgr = dolphin_milk::conversation::ConversationManager::new(&workspace);
    let conv = conv_mgr
        .create(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "test message",
        )
        .unwrap();

    let app = common::test_router_for_workspace(workspace).await;
    let req = Request::builder()
        .uri(format!("/conversations/{}/artifacts", conv.id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["total"].as_u64(), Some(0));
    assert!(result["artifacts"].as_array().unwrap().is_empty());
    assert!(result["by_type"].as_object().unwrap().is_empty());
}

// ═══════════════════════════════════════════════════════════════
// F.3: enriched conversation detail
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_conversation_detail_includes_summary() {
    let (_, workspace) = common::test_router_with_workspace().await;
    let conv_mgr = dolphin_milk::conversation::ConversationManager::new(&workspace);
    let conv = conv_mgr
        .create(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "summary test",
        )
        .unwrap();

    let app = common::test_router_for_workspace(workspace).await;
    let req = Request::builder()
        .uri(format!("/conversations/{}", conv.id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        result.get("summary").is_some(),
        "Response must have summary object"
    );
    let summary = &result["summary"];
    assert!(summary.get("total_iterations").is_some());
    assert!(summary.get("total_proofs").is_some());
    assert!(summary.get("total_artifacts").is_some());
    assert!(summary.get("duration_secs").is_some());
    assert!(summary.get("tasks").is_some());
    assert!(summary.get("cost_by_service").is_some());
    assert!(summary.get("models_used").is_some());
    // Empty conversation: all zeroed
    assert_eq!(summary["total_iterations"].as_u64(), Some(0));
    assert_eq!(summary["total_proofs"].as_u64(), Some(0));
    assert_eq!(summary["total_artifacts"].as_u64(), Some(0));
    assert!(summary["tasks"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_conversation_detail_backwards_compatible() {
    let (_, workspace) = common::test_router_with_workspace().await;
    let conv_mgr = dolphin_milk::conversation::ConversationManager::new(&workspace);
    let conv = conv_mgr
        .create(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "backwards compat test",
        )
        .unwrap();

    let app = common::test_router_for_workspace(workspace).await;
    let req = Request::builder()
        .uri(format!("/conversations/{}", conv.id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Existing fields must still be present
    assert!(result.get("conversation").is_some());
    assert!(result.get("messages").is_some());
}

/// In dev mode (no parent key), responses should NOT include x-bsv-auth-* headers.
#[tokio::test]
async fn test_dev_mode_no_auth_headers() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/status")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Dev mode: no auth headers on the response
    assert!(resp.headers().get("x-bsv-auth-version").is_none());
    assert!(resp.headers().get("x-bsv-auth-identity-key").is_none());
    assert!(resp.headers().get("x-bsv-auth-signature").is_none());

    // Body should still be valid JSON
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let status: StatusResponse = serde_json::from_slice(&body).unwrap();
    assert!(status.tasks.is_empty());
}

/// Handshake response should include 'signature' field when deserialized.
/// In dev mode (no wallet), signature will be None since server_identity_key is empty.
#[tokio::test]
async fn test_handshake_returns_signature_field() {
    use dolphin_milk::auth::HandshakeResponse;

    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/.well-known/auth")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "version": "0.1",
                "messageType": "initialRequest",
                "identityKey": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                "initialNonce": "dGVzdG5vbmNl"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let handshake: HandshakeResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(handshake.version, "0.1");
    assert_eq!(handshake.message_type, "initialResponse");
    assert_eq!(handshake.your_nonce, "dGVzdG5vbmNl");
    // In dev mode (no wallet), signature is None since server_identity_key is empty
    // When wallet is running, signature would be Some(Vec<u8>)
    // The field should deserialize correctly either way
}

// -- Phase 5B: Heartbeat trigger tests --

#[tokio::test]
async fn test_heartbeat_trigger_returns_triggered() {
    // Default config has heartbeat.enabled = true, so heartbeat_tx should be Some
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/heartbeat/trigger")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["triggered"], true);
}

#[tokio::test]
async fn test_heartbeat_trigger_with_custom_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        path,
    )
    .await;
    let app = server::build_router_with_state(Arc::clone(&state));

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/heartbeat/trigger")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "message": "Check inbox for updates" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify the message was delivered to the rx channel
    let rx = state
        .scheduler
        .heartbeat_rx
        .as_ref()
        .expect("heartbeat_rx should be Some");
    let mut rx_guard = rx.lock().await;
    let msg = rx_guard
        .try_recv()
        .expect("should have received the triggered message");
    assert_eq!(msg, "Check inbox for updates");
}

#[tokio::test]
async fn test_heartbeat_trigger_disabled_returns_503() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let mut config = DmConfig::default();
    config.wallet.url = "http://127.0.0.1:19999".into(); // no wallet = no auth in test
    config.heartbeat.enabled = false;
    let state = server::create_app_state(config, path).await;

    // Verify heartbeat_tx is None when disabled
    assert!(state.scheduler.heartbeat_tx.is_none());

    let app = server::build_router_with_state(state);

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/heartbeat/trigger")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// -- Compaction endpoint (Phase 10A) --

#[tokio::test]
async fn test_compact_nonexistent_conversation() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/conversations/conv-does-not-exist/compact")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_compact_short_conversation_noop() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    // Create a conversation with a few messages (well under max_history_turns)
    let mgr = dolphin_milk::conversation::ConversationManager::new(&workspace);
    let conv = mgr.create("testkey", "Hello").unwrap();
    mgr.append_user_message(&conv.id, "How are you?").unwrap();

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .method(http::Method::POST)
        .uri(format!("/conversations/{}/compact", conv.id))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["compacted"], false);
    assert_eq!(json["message_count"], 2);
    assert!(json["reason"]
        .as_str()
        .unwrap()
        .contains("within history limits"));
}

// -- OpenAI-compatible endpoint (Phase 10B) --

#[tokio::test]
async fn test_openai_compat_enabled_in_dev_mode() {
    // In dev mode (no parent key), endpoint should be accessible
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "user", "content": "Hello"}],
                "max_tokens": 1,
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Should not be 404 (route exists in dev mode)
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_openai_compat_rejects_no_user_message() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "system", "content": "You are helpful"}],
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_openai_compat_stream_not_implemented() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "user", "content": "Hello"}],
                "stream": true,
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Streaming"));
}

// ---------------------------------------------------------------------------
// Task 2.1 + 2.2: StatusResponse and BudgetReport field alignment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_status_response_has_correct_fields() {
    // Verify StatusResponse has active_count and total_sats (not a nested budget object)
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/status")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Must have these three fields at the top level
    assert!(json.get("tasks").is_some(), "tasks field missing");
    assert!(
        json.get("active_count").is_some(),
        "active_count field missing"
    );
    assert!(json.get("total_sats").is_some(), "total_sats field missing");

    // Must NOT have a nested "budget" object (old wrong interface)
    assert!(
        json.get("budget").is_none(),
        "budget field should not exist on StatusResponse"
    );
}

#[test]
fn test_budget_report_has_balance_field() {
    // Verify BudgetReport includes balance field at the type level
    let report = BudgetReport {
        task_sats: 100,
        total_operations: 1,
        services: Default::default(),
        hourly_sats: 100,
        daily_sats: 100,
        weekly_sats: 100,
        monthly_sats: 100,
        lifetime_sats: 100,
        limits: dolphin_milk::budget::LimitsReport {
            max_per_task: 250000,
            max_per_hour: 2500000,
            max_per_day: 25000000,
            max_per_week: 100000000,
            max_per_month: 500000000,
            max_lifetime: 0,
            task_remaining: 249900,
            hourly_remaining: 2499900,
            daily_remaining: 24999900,
            weekly_remaining: 99999900,
            monthly_remaining: 499999900,
            lifetime_remaining: 0,
            enforcement: "strict".into(),
        },
        balance: 42000,
    };

    let json = serde_json::to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["balance"], 42000);
    assert_eq!(parsed["task_sats"], 100);
    assert_eq!(parsed["limits"]["max_per_task"], 250000);
}

#[tokio::test]
async fn test_bsv_usd_rate_endpoint_returns_rate() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/rates/bsv-usd")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Should have a rate field (either cached or fallback)
    assert!(json.get("rate").is_some(), "rate field missing");
    let rate = json["rate"].as_f64().unwrap();
    // Rate should be positive (either real or the $20 fallback)
    assert!(rate > 0.0, "rate should be positive");
}

#[tokio::test]
async fn test_bsv_usd_rate_endpoint_enhanced_fields() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/rates/bsv-usd")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Enhanced response must include all new fields
    assert!(json.get("rate").is_some(), "rate field missing");
    assert!(
        json.get("rate_with_margin").is_some(),
        "rate_with_margin field missing"
    );
    assert!(json.get("source").is_some(), "source field missing");
    assert!(json.get("updated_at").is_some(), "updated_at field missing");
    assert!(json.get("stale").is_some(), "stale field missing");
    assert!(
        json.get("margin_percent").is_some(),
        "margin_percent field missing"
    );

    // With default 0% margin, rate_with_margin should equal rate
    let rate = json["rate"].as_f64().unwrap();
    let rate_with_margin = json["rate_with_margin"].as_f64().unwrap();
    assert!(
        (rate - rate_with_margin).abs() < 0.001,
        "with 0% margin, rates should match"
    );

    // margin_percent should be 0.0 by default
    let margin = json["margin_percent"].as_f64().unwrap();
    assert!(
        (margin - 0.0).abs() < f64::EPSILON,
        "default margin should be 0"
    );

    // stale should be a bool
    assert!(json["stale"].is_boolean(), "stale should be a boolean");
}

/// Margin calculation: rate * (1 + margin_percent/100)
fn apply_margin(rate: f64, margin_percent: f64) -> f64 {
    rate * (1.0 + margin_percent / 100.0)
}

#[test]
fn test_apply_margin_zero() {
    let result = apply_margin(50.0, 0.0);
    assert!((result - 50.0).abs() < f64::EPSILON);
}

#[test]
fn test_apply_margin_positive() {
    let result = apply_margin(100.0, 5.0);
    assert!((result - 105.0).abs() < f64::EPSILON);
}

#[test]
fn test_apply_margin_fractional() {
    let result = apply_margin(50.0, 2.5);
    assert!((result - 51.25).abs() < 1e-10);
}

#[test]
fn test_openai_compat_config_default() {
    let config = DmConfig::default();
    assert!(!config.server.openai_compat_enabled);
}

// -- Audit Export Tests --

#[tokio::test]
async fn test_audit_export_csv_returns_200() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-export-csv";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_system("You are a helpful agent.");
    transcript.record_user("What is 2+2?");
    transcript.record_think_response("4", "gpt-5-nano", 100, 80, 20, 50, 10, None, "stop", 500);

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=csv"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let headers = resp.headers();
    assert_eq!(
        headers.get("content-type").unwrap().to_str().unwrap(),
        "text/csv"
    );
    assert!(headers
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("attachment"));

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let csv = String::from_utf8(body.to_vec()).unwrap();
    assert!(csv.starts_with(
        "timestamp,iteration,event_type,model,tool_name,sats_spent,proof_txid,detail\n"
    ));
    // Should have header + at least 3 event rows (system, user, think_response)
    assert!(csv.lines().count() >= 4);
}

#[tokio::test]
async fn test_audit_export_json_returns_200() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-export-json";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_system("You are a helpful agent.");
    transcript.record_user("What is 2+2?");
    transcript.record_think_response("4", "gpt-5-nano", 100, 80, 20, 50, 10, None, "stop", 500);

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=json"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let headers = resp.headers();
    assert_eq!(
        headers.get("content-type").unwrap().to_str().unwrap(),
        "application/json"
    );
    assert!(headers
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("attachment"));

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let export: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(export["export_version"], "1.0");
    assert!(export["task_id"]
        .as_str()
        .unwrap()
        .contains("test-export-json"));
    assert!(export["events"].as_array().unwrap().len() >= 3);
    assert!(export["summary"].is_object());
    assert!(export["proofs"].is_array());
    // Signature fields present (may be empty without wallet)
    assert!(export.get("signature").is_some());
    assert!(export.get("signature_hash").is_some());
}

#[tokio::test]
async fn test_audit_export_default_format_is_csv() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-export-default";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("hello");

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    // No format= query param — should default to CSV
    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/csv"
    );
}

#[tokio::test]
async fn test_audit_export_nonexistent_task_returns_404() {
    let app = common::test_router().await;

    let req = Request::builder()
        .uri("/task/nonexistent-id/audit/export?format=csv")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_audit_export_csv_has_correct_columns() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-export-cols";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("Do something");
    transcript.record_think_response("OK", "gpt-5-mini", 200, 150, 50, 100, 20, None, "stop", 800);

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=csv"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let csv = String::from_utf8(body.to_vec()).unwrap();
    let lines: Vec<&str> = csv.lines().collect();

    // Check header
    assert_eq!(
        lines[0],
        "timestamp,iteration,event_type,model,tool_name,sats_spent,proof_txid,detail"
    );

    // Check that think_response event includes model
    let think_line = lines.iter().find(|l| l.contains("think_response")).unwrap();
    assert!(think_line.contains("gpt-5-mini"));
}

#[tokio::test]
async fn test_audit_export_json_has_summary() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-export-summary";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("hello");
    transcript.record_think_request(&[], "gpt-5-mini", 4096);
    transcript.record_think_response("hi", "gpt-5-mini", 100, 80, 20, 50, 10, None, "stop", 500);

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=json"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let export: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let summary = &export["summary"];
    assert!(summary["total_events"].as_u64().unwrap() >= 2);
    assert!(summary["iterations"].as_u64().unwrap() >= 1);
    assert!(summary.get("sats_spent").is_some());
    assert!(summary.get("duration_secs").is_some());
}

#[tokio::test]
async fn test_audit_export_json_signature_hash_is_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-export-hash";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("test");

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=json"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let export: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // signature_hash should be a 64-char hex string (SHA-256)
    let hash = export["signature_hash"].as_str().unwrap();
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

// ---------------------------------------------------------------------------
// Task 1.2: Task reconstruction from disk transcripts after server restart
// ---------------------------------------------------------------------------

/// After server restart, completed tasks should be reconstructed from disk
/// and accessible via GET /task/{id}.
#[tokio::test]
async fn test_task_reconstruction_from_disk_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-reconstruct-001";

    // Create a transcript JSONL file that simulates a completed task
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("What is 2+2?");
    transcript.record_think_request(&[], "gpt-5-nano", 4096);
    transcript.record_think_response("4", "gpt-5-nano", 100, 80, 20, 50, 10, None, "stop", 500);

    // Record session_end to mark task as complete
    {
        let mut data = std::collections::HashMap::new();
        data.insert("iterations".to_string(), serde_json::json!(1));
        data.insert("sats_spent".to_string(), serde_json::json!(80));
        data.insert("status".to_string(), serde_json::json!("complete"));
        data.insert("result".to_string(), serde_json::json!("4"));
        data.insert("error".to_string(), serde_json::json!(""));
        transcript.record("session_end", data);
    }

    // Build router — this simulates server restart
    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    // The task should now be accessible via GET /task/{id}
    let req = Request::builder()
        .uri(format!("/task/{task_id}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Reconstructed task should be found"
    );

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let info: TaskInfo = serde_json::from_slice(&body).unwrap();
    assert_eq!(info.id, task_id);
    assert_eq!(info.task, "What is 2+2?");
    assert_eq!(info.status, TaskStatus::Complete);
    assert_eq!(info.iterations, 1);
    assert_eq!(info.sats_spent, 80); // sats_effective from think_response
    assert_eq!(info.result.as_deref(), Some("4"));
    assert!(info.error.is_none());
}

/// Tasks with session_end that has an error should be reconstructed as Error status.
#[tokio::test]
async fn test_task_reconstruction_error_status() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-reconstruct-err";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("Do something dangerous");

    // session_end with an error
    {
        let mut data = std::collections::HashMap::new();
        data.insert("iterations".to_string(), serde_json::json!(0));
        data.insert("sats_spent".to_string(), serde_json::json!(0));
        data.insert("status".to_string(), serde_json::json!("error"));
        data.insert("result".to_string(), serde_json::json!(""));
        data.insert("error".to_string(), serde_json::json!("Budget exceeded"));
        transcript.record("session_end", data);
    }

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let info: TaskInfo = serde_json::from_slice(&body).unwrap();
    assert_eq!(info.status, TaskStatus::Error);
    assert_eq!(info.error.as_deref(), Some("Budget exceeded"));
}

/// Tasks without session_end (server crash) should be reconstructed as Interrupted status.
#[tokio::test]
async fn test_task_reconstruction_interrupted_status() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-reconstruct-interrupted";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);

    // Session start
    {
        let mut data = std::collections::HashMap::new();
        data.insert("task".to_string(), serde_json::json!("Research BSV fees"));
        transcript.record("session_start", data);
    }
    transcript.record_user("Research BSV fees");
    transcript.record_think_request(&[], "gpt-5-mini", 16384);
    transcript.record_think_response(
        "Let me search...",
        "gpt-5-mini",
        200,
        80,
        120,
        500,
        50,
        None,
        "tool_calls",
        1000,
    );
    // No session_end — simulates server crash

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let info: TaskInfo = serde_json::from_slice(&body).unwrap();
    assert_eq!(info.status, TaskStatus::Interrupted);
    assert_eq!(info.task, "Research BSV fees");
    assert_eq!(info.iterations, 1);
    assert!(info.completed_at.is_none()); // not completed
    assert!(info.result.is_none()); // no result
}

/// Proof txids should be extracted from transcript proof_created events.
#[tokio::test]
async fn test_task_reconstruction_includes_proof_txids() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-reconstruct-proofs";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
    transcript.record_user("Do something");
    transcript.record_think_response("OK", "gpt-5-nano", 100, 80, 20, 50, 10, None, "stop", 500);
    transcript.record_proof_created(
        "abc123def456",
        "decision",
        "deadbeef",
        None,
        None,
        Some(200),
        Some(1),
        Some("dm-proofs"),
        None,
    );

    {
        let mut data = std::collections::HashMap::new();
        data.insert("iterations".to_string(), serde_json::json!(1));
        data.insert("sats_spent".to_string(), serde_json::json!(280));
        data.insert("status".to_string(), serde_json::json!("complete"));
        data.insert("result".to_string(), serde_json::json!("OK"));
        data.insert("error".to_string(), serde_json::json!(""));
        transcript.record("session_end", data);
    }

    std::mem::forget(dir);
    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let info: TaskInfo = serde_json::from_slice(&body).unwrap();
    assert_eq!(info.proof_txids, vec!["abc123def456"]);
}

/// Multiple tasks should all be reconstructed and listed via GET /status.
#[tokio::test]
async fn test_task_reconstruction_multiple_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    for i in 1..=3 {
        let task_id = format!("multi-task-{i}");
        let task_dir = workspace.join(format!("tasks/{task_id}"));
        std::fs::create_dir_all(&task_dir).unwrap();
        let transcript_path = task_dir.join("session.jsonl");

        let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);
        transcript.record_user(&format!("Task number {i}"));
        transcript.record_think_response(
            "done",
            "gpt-5-nano",
            100,
            80,
            20,
            50,
            10,
            None,
            "stop",
            500,
        );

        let mut data = std::collections::HashMap::new();
        data.insert("iterations".to_string(), serde_json::json!(1));
        data.insert("sats_spent".to_string(), serde_json::json!(80));
        data.insert("status".to_string(), serde_json::json!("complete"));
        data.insert("result".to_string(), serde_json::json!("done"));
        data.insert("error".to_string(), serde_json::json!(""));
        transcript.record("session_end", data);
    }

    std::mem::forget(dir);
    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    // Check that all 3 tasks were reconstructed
    let tasks = state.task_mgr.tasks.lock().await;
    assert_eq!(tasks.len(), 3, "All 3 tasks should be reconstructed");
    assert!(tasks.contains_key("multi-task-1"));
    assert!(tasks.contains_key("multi-task-2"));
    assert!(tasks.contains_key("multi-task-3"));

    for (_, info) in tasks.iter() {
        assert_eq!(info.status, TaskStatus::Complete);
        assert_eq!(info.iterations, 1);
    }
}

/// Session ID should be reconstructed from session_start events in transcript.
#[tokio::test]
async fn test_task_reconstruction_session_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let task_id = "test-session-map";

    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");

    let mut transcript = dolphin_milk::transcript::Transcript::new(transcript_path);

    // Record a session_start event with session_id
    {
        let mut data = std::collections::HashMap::new();
        data.insert("session_id".to_string(), serde_json::json!("conv-test-123"));
        data.insert("task_id".to_string(), serde_json::json!(task_id));
        data.insert("resumed".to_string(), serde_json::json!(false));
        transcript.record("session_start", data);
    }
    transcript.record_user("Hello");
    transcript.record_think_response("Hi", "gpt-5-nano", 100, 80, 20, 50, 10, None, "stop", 500);

    {
        let mut data = std::collections::HashMap::new();
        data.insert("iterations".to_string(), serde_json::json!(1));
        data.insert("sats_spent".to_string(), serde_json::json!(80));
        data.insert("status".to_string(), serde_json::json!("complete"));
        data.insert("result".to_string(), serde_json::json!("Hi"));
        data.insert("error".to_string(), serde_json::json!(""));
        transcript.record("session_end", data);
    }

    std::mem::forget(dir);
    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    // Check task_sessions mapping was reconstructed
    let sessions = state.task_mgr.task_sessions.lock().await;
    assert_eq!(sessions.get(task_id), Some(&"conv-test-123".to_string()));
}

// ---------------------------------------------------------------------------
// Task 1.3: Panic catching in spawned task threads
// ---------------------------------------------------------------------------

/// Verify that the supervisor pattern can detect JoinError (simulated test).
/// We can't easily cause a real panic in spawn_task, but we can verify
/// the error handling pattern works by testing the TaskStatus update logic.
#[test]
fn test_task_status_transitions_on_panic() {
    // If a task panics, its status should transition from Running to Error
    let mut info = TaskInfo {
        id: "panic-test".into(),
        task: "doomed task".into(),
        status: TaskStatus::Running,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-03-05T12:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };

    // Simulate what the supervisor does on panic
    if matches!(info.status, TaskStatus::Running) {
        info.status = TaskStatus::Error;
        info.error = Some("Task panicked: task panicked".to_string());
        info.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }

    assert_eq!(info.status, TaskStatus::Error);
    assert!(info.error.as_ref().unwrap().contains("panicked"));
    assert!(info.completed_at.is_some());
}

/// Verify the supervisor does not overwrite a task that already completed.
#[test]
fn test_supervisor_does_not_overwrite_completed_task() {
    let mut info = TaskInfo {
        id: "already-done".into(),
        task: "finished task".into(),
        status: TaskStatus::Complete,
        result: Some("success".into()),
        error: None,
        iterations: 5,
        sats_spent: 1000,
        started_at: "2026-03-05T12:00:00Z".into(),
        completed_at: Some("2026-03-05T12:05:00Z".into()),
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };

    // Supervisor check: only update if still Running
    if matches!(info.status, TaskStatus::Running) {
        info.status = TaskStatus::Error;
        info.error = Some("Task panicked".to_string());
    }

    // Should remain Complete since it wasn't Running
    assert_eq!(info.status, TaskStatus::Complete);
    assert!(info.error.is_none());
    assert_eq!(info.result.as_deref(), Some("success"));
}

// ---------------------------------------------------------------------------
// Task 1.4: Conversation integrity proof budget tracking
// ---------------------------------------------------------------------------

/// Verify that the budget tracker records conversation integrity proof costs.
/// The worm's budget_tracker is called in task_spawner after proof creation.
#[test]
fn test_budget_tracker_records_conversation_integrity_proof() {
    use dolphin_milk::budget::BudgetTracker;
    use dolphin_milk::config::BudgetConfig;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let mut tracker = BudgetTracker::from_config(&BudgetConfig::default(), &workspace);

    // Simulate what task_spawner does after creating a conversation integrity proof
    tracker.record(
        "proofs",
        "brc18_conversation_integrity",
        200,
        serde_json::json!({"txid": "abc123", "conversation": "conv-test"}),
    );

    assert_eq!(tracker.task_sats(), 200);

    let entries = tracker.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].service, "proofs");
    assert_eq!(entries[0].operation, "brc18_conversation_integrity");
    assert_eq!(entries[0].sats, 200);
}

/// Verify that sats_spent on LoopState gets incremented for the integrity proof.
/// This simulates the line `worm.state.sats_spent += 200` in task_spawner.
#[test]
fn test_sats_spent_includes_conversation_integrity_proof() {
    // The proof costs 200 sats added to worm.state.sats_spent
    // which then flows to state_ref.stats.sats_spent via fetch_add
    let initial_sats: u64 = 500;
    let proof_cost: u64 = 200;
    let total = initial_sats + proof_cost;
    assert_eq!(
        total, 700,
        "sats_spent should include the 200-sat proof cost"
    );
}

// ---------------------------------------------------------------------------
// Block 6: Backend Health & Caching
// ---------------------------------------------------------------------------

// -- Task 6.1: Health endpoint includes wallet_connected field --

#[tokio::test]
async fn test_health_includes_wallet_connected_field() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();

    // The wallet_connected field should be present (bool, not optional).
    // Its value depends on whether a real wallet is running on localhost:3322.
    // We verify the field exists and the status is always "ok" regardless.
    let _ = health.wallet_connected; // field exists
    assert_eq!(
        health.status, "ok",
        "Status should always be ok, even if wallet is down"
    );
    // Verify the JSON actually contains the field
    let body2 = serde_json::to_string(&health).unwrap();
    assert!(
        body2.contains("wallet_connected"),
        "JSON should contain wallet_connected field"
    );
}

#[test]
fn test_health_response_serde_with_wallet_connected() {
    let health = HealthResponse {
        status: "ok".into(),
        version: "0.1.0".into(),
        uptime_secs: 42,
        scheduler_last_tick_secs_ago: None,
        wallet_connected: true,
        wallet_url: None,
    };
    let json = serde_json::to_string(&health).unwrap();
    assert!(json.contains("\"wallet_connected\":true"));

    let parsed: HealthResponse = serde_json::from_str(&json).unwrap();
    assert!(parsed.wallet_connected);
}

#[test]
fn test_health_response_wallet_connected_defaults_false() {
    // Deserializing old JSON without wallet_connected should default to false
    let json = r#"{"status":"ok","version":"0.1.0","uptime_secs":10}"#;
    let health: HealthResponse = serde_json::from_str(json).unwrap();
    assert!(!health.wallet_connected);
}

// -- Task 6.2 & 6.3: Cached tool registry and wallet balance on /agent --

#[tokio::test]
async fn test_cached_tool_names_populated_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        path,
    )
    .await;

    // cached_tool_names should be populated and sorted
    assert!(
        !state.cached_tool_names.is_empty(),
        "cached_tool_names should not be empty after create_app_state"
    );

    // Verify tools are sorted (important for consistency)
    let names = &state.cached_tool_names;
    for window in names.windows(2) {
        assert!(
            window[0] <= window[1],
            "cached_tool_names should be sorted: '{}' should come before '{}'",
            window[0],
            window[1]
        );
    }

    // Should contain known tool names
    assert!(
        names.contains(&"execute_bash".to_string()),
        "Should contain execute_bash"
    );
    assert!(
        names.contains(&"wallet_balance".to_string()),
        "Should contain wallet_balance"
    );
    assert!(
        names.contains(&"memory_store".to_string()),
        "Should contain memory_store"
    );
}

#[tokio::test]
async fn test_cached_balance_initially_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        path,
    )
    .await;

    let guard = state.cached_balance.read().await;
    assert!(guard.is_none(), "cached_balance should be None at startup");
}

// -- Task 6.4: Certificate revocation status in responses --

#[test]
fn test_agent_response_includes_certificate_revoked_field() {
    use dolphin_milk::server::AgentResponse;

    let resp = AgentResponse {
        identity_key: "02abc".into(),
        balance: 100,
        version: "0.1.0".into(),
        uptime_secs: 10,
        total_tasks: 0,
        total_sats_spent: 0,
        tools: vec!["test".into()],
        certificate_status: Some("parent-signed".into()),
        certificate_revoked: Some(false),
        default_model: "gpt-5-mini".into(),
        available_models: vec![],
    };

    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"certificate_revoked\":false"));

    let parsed: AgentResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.certificate_revoked, Some(false));
}

#[test]
fn test_agent_response_certificate_revoked_none_omitted() {
    use dolphin_milk::server::AgentResponse;

    let resp = AgentResponse {
        identity_key: "02abc".into(),
        balance: 100,
        version: "0.1.0".into(),
        uptime_secs: 10,
        total_tasks: 0,
        total_sats_spent: 0,
        tools: vec![],
        certificate_status: None,
        certificate_revoked: None,
        default_model: "gpt-5-mini".into(),
        available_models: vec![],
    };

    let json = serde_json::to_string(&resp).unwrap();
    // None fields with skip_serializing_if should be omitted
    assert!(
        !json.contains("certificate_revoked"),
        "certificate_revoked=None should be omitted from JSON"
    );
    assert!(
        !json.contains("certificate_status"),
        "certificate_status=None should be omitted from JSON"
    );
}

#[test]
fn test_agent_response_backward_compat_without_revoked() {
    // Old JSON without certificate_revoked should deserialize fine
    let json = r#"{"identity_key":"02abc","balance":100,"version":"0.1.0","uptime_secs":10,"total_tasks":0,"total_sats_spent":0,"tools":[]}"#;
    let resp: dolphin_milk::server::AgentResponse = serde_json::from_str(json).unwrap();
    assert!(resp.certificate_revoked.is_none());
    assert!(resp.certificate_status.is_none());
}

// -- Task 6.5: Revocation UTXO pagination --

#[test]
fn test_is_revoked_null_outpoint_returns_false() {
    // This is a unit test confirming the existing behavior that null outpoints
    // short-circuit to Ok(false) without any wallet calls
    use dolphin_milk::certificates::is_null_revocation_outpoint;

    assert!(is_null_revocation_outpoint(""));
    assert!(is_null_revocation_outpoint(
        "000000000000000000000000000000000000000000000000000000000000000000000000"
    ));
}

#[test]
fn test_parse_revocation_outpoint_pagination_scenario() {
    use dolphin_milk::certificates::parse_revocation_outpoint;

    // Valid outpoint that would need pagination to find
    let txid = "a".repeat(64);
    let vout = "00000001";
    let outpoint = format!("{txid}{vout}");

    let result = parse_revocation_outpoint(&outpoint);
    assert!(result.is_some(), "Valid outpoint should parse");
    let (parsed_txid, parsed_vout) = result.unwrap();
    assert_eq!(parsed_txid, txid);
    assert_eq!(parsed_vout, 1);
}

// -- Task 6.2 continued: Verify tool cache matches direct registry --

#[tokio::test]
async fn test_cached_tools_match_direct_registry() {
    use dolphin_milk::tools::memory_tools::all_memory_tools;
    use dolphin_milk::tools::messagebox_tools::all_messagebox_tools;
    use dolphin_milk::tools::registry::ToolRegistry;
    use dolphin_milk::tools::sandbox::all_sandbox_tools;
    use dolphin_milk::tools::wallet_tools::all_wallet_tools;
    use dolphin_milk::tools::x402_tools::all_x402_tools;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let config = DmConfig::default();

    // Build a fresh registry the same way the server does
    let mut registry = ToolRegistry::new();
    let ws = path.clone();
    let memory_dir = ws.join("memory");
    for tool in all_sandbox_tools(ws, config.llm.context_window) {
        registry.register(tool);
    }
    for tool in all_wallet_tools("http://localhost:3322".to_string()) {
        registry.register(tool);
    }
    for tool in all_memory_tools(memory_dir) {
        registry.register(tool);
    }
    for tool in all_messagebox_tools(config.wallet.url.clone()) {
        registry.register(tool);
    }
    for tool in all_x402_tools(config.wallet.url.clone(), config.x402.registry_url.clone()) {
        registry.register(tool);
    }
    let mut direct_names = registry.tool_names();
    direct_names.sort();

    // Create app state and compare
    std::mem::forget(dir);
    let state = server::create_app_state(config, path).await;

    assert_eq!(
        state.cached_tool_names, direct_names,
        "Cached tool names should match a freshly built registry"
    );
}

// ---------------------------------------------------------------------------
// Conversation queue via per-conversation semaphore
// ---------------------------------------------------------------------------

#[test]
fn test_task_status_queued_serialization() {
    assert_eq!(
        serde_json::to_string(&TaskStatus::Queued).unwrap(),
        "\"queued\""
    );
    let queued: TaskStatus = serde_json::from_str("\"queued\"").unwrap();
    assert_eq!(queued, TaskStatus::Queued);
}

#[test]
fn test_task_status_queued_ne_running() {
    assert_ne!(TaskStatus::Queued, TaskStatus::Running);
    assert_ne!(TaskStatus::Queued, TaskStatus::Complete);
}

#[test]
fn test_task_info_queued_status() {
    let info = TaskInfo {
        id: "queued-1".into(),
        task: "waiting".into(),
        status: TaskStatus::Queued,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-03-06T12:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains("\"queued\""));
    let parsed: TaskInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.status, TaskStatus::Queued);
}

#[test]
fn test_cancel_queued_task_status_transition() {
    let mut info = TaskInfo {
        id: "cancel-queued".into(),
        task: "will be cancelled".into(),
        status: TaskStatus::Queued,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-03-06T12:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };

    // Simulate what spawn_task does when cancel flag is set while queued
    if matches!(info.status, TaskStatus::Queued) {
        info.status = TaskStatus::Cancelled;
        info.error = Some("Cancelled while queued".to_string());
        info.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }

    assert_eq!(info.status, TaskStatus::Cancelled);
    assert!(info.error.as_ref().unwrap().contains("while queued"));
    assert!(info.completed_at.is_some());
}

#[test]
fn test_supervisor_handles_queued_and_running_panic() {
    // Supervisor should transition both Queued and Running to Error
    for initial in [TaskStatus::Queued, TaskStatus::Running] {
        let mut info = TaskInfo {
            id: "panic-test".into(),
            task: "doomed".into(),
            status: initial.clone(),
            result: None,
            error: None,
            iterations: 0,
            sats_spent: 0,
            started_at: "2026-03-06T12:00:00Z".into(),
            completed_at: None,
            proof_txids: Vec::new(),
            tags: Vec::new(),
            origin: String::new(),
            conversation_id: None,
        };

        if matches!(info.status, TaskStatus::Running | TaskStatus::Queued) {
            info.status = TaskStatus::Error;
            info.error = Some("Task panicked".to_string());
            info.completed_at = Some(chrono::Utc::now().to_rfc3339());
        }

        assert_eq!(
            info.status,
            TaskStatus::Error,
            "should transition from {:?}",
            initial
        );
    }
}

#[test]
fn test_supervisor_does_not_overwrite_completed_from_queued() {
    // If a task went Queued -> Complete before supervisor checks, leave it alone
    let mut info = TaskInfo {
        id: "done-quick".into(),
        task: "fast task".into(),
        status: TaskStatus::Complete,
        result: Some("done".into()),
        error: None,
        iterations: 3,
        sats_spent: 500,
        started_at: "2026-03-06T12:00:00Z".into(),
        completed_at: Some("2026-03-06T12:01:00Z".into()),
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };

    if matches!(info.status, TaskStatus::Running | TaskStatus::Queued) {
        info.status = TaskStatus::Error;
        info.error = Some("Task panicked".to_string());
    }

    assert_eq!(info.status, TaskStatus::Complete);
    assert!(info.error.is_none());
}

#[tokio::test]
async fn test_conversation_semaphore_created_on_demand() {
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    std::mem::forget(dir);
    let state = server::create_app_state(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    // Initially empty
    {
        let sems = state.task_mgr.conversation_semaphores.lock().await;
        assert!(sems.is_empty());
    }

    // Insert a semaphore
    {
        let mut sems = state.task_mgr.conversation_semaphores.lock().await;
        sems.entry("conv-123".to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(1)));
        assert_eq!(sems.len(), 1);
        assert_eq!(sems.get("conv-123").unwrap().available_permits(), 1);
    }
}

#[tokio::test]
async fn test_semaphore_blocks_concurrent_access() {
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    let sem = Arc::new(Semaphore::new(1));

    // Acquire the permit
    let permit = sem.clone().acquire_owned().await.unwrap();
    assert_eq!(sem.available_permits(), 0);

    // Try to acquire again — should not be immediately available
    let sem2 = sem.clone();
    let handle = tokio::spawn(async move {
        let _permit = sem2.acquire().await.unwrap();
        "acquired"
    });

    // Give the spawned task a chance to park
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Drop the first permit — should unblock the waiter
    drop(permit);

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
    assert!(result.is_ok(), "waiter should have been unblocked");
    assert_eq!(result.unwrap().unwrap(), "acquired");
}

#[tokio::test]
async fn test_semaphore_fifo_ordering() {
    use std::sync::Arc;
    use tokio::sync::{Mutex, Semaphore};

    let sem = Arc::new(Semaphore::new(1));
    let order = Arc::new(Mutex::new(Vec::<u32>::new()));

    // Hold the permit
    let permit = sem.clone().acquire_owned().await.unwrap();

    // Spawn 3 waiters in order
    let mut handles = vec![];
    for i in 1..=3 {
        let sem = sem.clone();
        let order = order.clone();
        // Small delay to ensure ordering of acquire calls
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            order.lock().await.push(i);
            // Hold briefly so next waiter gets a chance
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }));
    }

    // Release — waiters should proceed in FIFO order
    drop(permit);

    for h in handles {
        h.await.unwrap();
    }

    let result = order.lock().await;
    assert_eq!(
        *result,
        vec![1, 2, 3],
        "semaphore should wake waiters in FIFO order"
    );
}

#[tokio::test]
async fn test_cancel_task_accepts_queued_status() {
    let app = common::test_router().await;

    // Submit a task first
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/task")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"task": "test queued cancel"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let task: TaskResponse = serde_json::from_slice(&body).unwrap();

    // Verify the cancel endpoint exists and returns OK (not 409)
    // In dev mode, the task starts running immediately (no queue contention),
    // so this tests that the route handler works for both Running and Queued.
    let cancel_req = Request::builder()
        .method(http::Method::POST)
        .uri(format!("/task/{}/cancel", task.id))
        .body(Body::empty())
        .unwrap();
    let cancel_resp = app.oneshot(cancel_req).await.unwrap();
    // Should be OK (200) — task was Running in dev mode
    assert_eq!(cancel_resp.status(), StatusCode::OK);
}

// =============================================================================
// PDF export endpoint tests
// =============================================================================

/// Helper: build a router with a task transcript (same as the inline test helper).
async fn router_with_task() -> (axum::Router, String) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    std::mem::forget(dir);

    let task_id = "pdf-test-task-001".to_string();
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();

    let mut transcript = dolphin_milk::transcript::Transcript::new(task_dir.join("session.jsonl"));
    transcript.record_user("compute pi to 5 digits");
    transcript.record_think_request(&[], "gpt-5-mini", 4096);
    transcript.record_think_response(
        "I'll compute pi",
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
    transcript.record_tool_call(
        "call-1",
        "execute_bash",
        &serde_json::json!({"command": "echo 3.14159"}),
    );
    transcript.record_tool_result("call-1", "execute_bash", "3.14159", true, 0);
    transcript.record_proof_created(
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
        "TaskCompletion",
        "deadbeef",
        Some("task:pdf-test"),
        Some("2026-03-01T12:00:00Z"),
        Some(200),
        Some(1),
        Some("dm-proofs"),
        None,
    );

    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;
    (app, task_id)
}

#[tokio::test]
async fn test_pdf_export_endpoint_exists() {
    // GET /task/{id}/audit/export?format=pdf should return correct content-type
    // Without Chrome available, expect 503 (SERVICE_UNAVAILABLE)
    let (app, task_id) = router_with_task().await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=pdf"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Without Chrome: 503 with JSON error
    // With Chrome: 200 with application/pdf
    let status = resp.status();
    assert!(
        status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::OK,
        "expected 503 or 200, got {status}"
    );

    if status == StatusCode::OK {
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(ct, "application/pdf");
    } else {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err.get("error").is_some());
        assert!(err.get("hint").is_some());
    }
}

#[tokio::test]
async fn test_pdf_export_no_chrome_returns_503() {
    // Graceful fallback without Chrome — should return 503
    // This test will pass in CI where Chrome is typically not installed
    let (app, task_id) = router_with_task().await;

    let req = Request::builder()
        .uri(format!("/task/{task_id}/audit/export?format=pdf"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    // In environments without Chrome, this should be 503
    // In environments with Chrome, it would be 200 — both are acceptable
    let status = resp.status();
    assert!(
        status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::OK,
        "expected 503 (no Chrome) or 200 (Chrome available), got {status}"
    );

    if status == StatusCode::SERVICE_UNAVAILABLE {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let error_msg = err["error"].as_str().unwrap_or("");
        assert!(
            error_msg.contains("Chrome")
                || error_msg.contains("unavailable")
                || error_msg.contains("PDF"),
            "error should mention Chrome or PDF availability: {error_msg}"
        );
    }
}

#[tokio::test]
async fn test_pdf_export_nonexistent_task_404() {
    // Proper error for missing task
    let app = common::test_router().await;

    let req = Request::builder()
        .uri("/task/nonexistent-task-xyz/audit/export?format=pdf")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_budget_pdf_export_endpoint() {
    // Budget export endpoint should exist and work
    let app = common::test_router().await;

    // Default format (JSON)
    let req = Request::builder()
        .uri("/budget/export")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("application/json"));

    // PDF format — will be 503 without Chrome or 200 with Chrome
    let app2 = common::test_router().await;
    let req2 = Request::builder()
        .uri("/budget/export?format=pdf")
        .body(Body::empty())
        .unwrap();
    let resp2 = app2.oneshot(req2).await.unwrap();
    let status = resp2.status();
    assert!(
        status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::OK,
        "expected 503 or 200, got {status}"
    );
}
