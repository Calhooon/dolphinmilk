//! HTTP server — axum-based REST API for the worm daemon.
//!
//! Endpoints:
//!   POST /task             — Submit a task, returns task ID
//!   GET  /status           — List all tasks with budget summary
//!   GET  /task/:id         — Get status of a specific task
//!   GET  /tasks            — List all tasks across sessions (memory + disk)
//!   GET  /task/:id/audit   — Full audit chain from transcript
//!   GET  /task/:id/proofs  — Proof details from transcript
//!   GET  /task/:id/conversation — Reconstructed conversation messages
//!   POST /message          — Forward a MessageBox message to the worm
//!   GET  /health           — Liveness check
//!   GET  /budget           — Budget report (BRC-31 auth-gated when parent key set)
//!   GET  /agent            — Agent identity and stats
//!   POST /.well-known/auth — BRC-31 Authrite handshake
//!   POST /chat             — Submit chat message (BRC-31 auth)
//!   GET  /chat/history     — Conversation from transcript JSONL (BRC-31 auth)
//!   GET  /ui/*             — Static file serving for Lit frontend

pub mod types;
pub use types::*;

mod task_spawner;
pub(crate) use task_spawner::spawn_task;

pub(crate) mod auth;
mod handlers;
pub mod pdf_template;

// Re-export audit search types for integration tests
pub use handlers::audit::{AuditSearchResponse, AuditSearchResult};

// Re-export marketplace types for integration tests
pub use handlers::marketplace::{PluginListResponse, PluginListing};

// Re-export revelation types for integration tests
pub use crate::audit::revelation::{Revelation, RevelationsListResponse};

pub mod ui_assets;

mod app_state;
pub use app_state::*;

// Extracted handlers/types now live in:
//   app_state.rs         — AppState, TaskManager, AuthState, create_app_state, build_router, serve
//   auth.rs              — BRC-31 helpers (headermap_to_pairs, check_brc31_auth, signed_json_response, brc31_handshake)
//   handlers/tasks.rs    — task CRUD, audit, proofs, verify, receipts, events, cancel, serve_task_file
//   handlers/agent.rs    — health, get_agent, certificates
//   handlers/chat.rs     — chat, chat_history, openai_chat_completions
//   handlers/budget.rs   — get_budget, get_budget_detail, get_bsv_usd_rate, fetch_bsv_usd_rate
//   handlers/memory.rs   — list_memories, search_memories, get_memory
//   handlers/conversations.rs — list/detail/verify/sync/compact conversations
//   handlers/misc.rs     — receive_message, trigger_heartbeat, output, decrypt, schedules

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::BudgetReport;
    use crate::config::DmConfig;
    use axum::body::Body;
    use axum::http::{self, Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use std::path::PathBuf;
    use tower::ServiceExt;

    use crate::auth::server::HandshakeResponse;

    // Types moved to handler submodules — re-import for tests
    use super::handlers::tasks::{ProofVerification, VerifyResponse};
    use super::handlers::wallet_ops::{parse_push_drop_fields, DecryptResponse, OutputResponse};

    fn test_config() -> DmConfig {
        let mut cfg = DmConfig::default();
        cfg.wallet.url = "http://127.0.0.1:19999".to_string(); // unreachable — no auth in tests
        cfg
    }

    async fn test_router() -> Router {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        // Keep the tempdir alive by leaking it (test-only)
        std::mem::forget(dir);
        build_router(test_config(), path).await
    }

    #[tokio::test]
    async fn test_health() {
        let app = test_router().await;
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
    }

    #[tokio::test]
    async fn test_status_empty() {
        let app = test_router().await;
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
    async fn test_task_not_found() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/task/nonexistent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_submit_task() {
        let app = test_router().await;
        let req = Request::builder()
            .method(http::Method::POST)
            .uri("/task")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&TaskRequest {
                    task: "test task".into(),
                    max_iterations: 1,
                    tags: None,
                    model: None,
                })
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let task_resp: TaskResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(task_resp.status, TaskStatus::Running);
        assert!(!task_resp.id.is_empty());
    }

    #[tokio::test]
    async fn test_message_endpoint() {
        let app = test_router().await;
        let req = Request::builder()
            .method(http::Method::POST)
            .uri("/message")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "sender": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                    "body": {"type": "ping"},
                    "message_box": "status_inbox"
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let msg_resp: MessageResponse = serde_json::from_slice(&body).unwrap();
        assert!(msg_resp.accepted);
    }

    #[tokio::test]
    async fn test_budget_endpoint() {
        let app = test_router().await;
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
    }

    #[test]
    fn test_task_status_serde() {
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
    }

    #[test]
    fn test_task_info_serde() {
        let info = TaskInfo {
            id: "abc".into(),
            task: "hello".into(),
            status: TaskStatus::Complete,
            result: Some("world".into()),
            error: None,
            iterations: 3,
            sats_spent: 500,
            started_at: "2026-02-24T12:00:00Z".into(),
            completed_at: Some("2026-02-24T12:01:00Z".into()),
            proof_txids: Vec::new(),
            tags: Vec::new(),
            origin: String::new(),
            conversation_id: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("error")); // None fields skipped
        assert!(!json.contains("proof_txids")); // Empty vec skipped
        assert!(!json.contains("tags")); // Empty vec skipped
        let parsed: TaskInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "abc");
        assert_eq!(parsed.status, TaskStatus::Complete);
    }

    #[test]
    fn test_default_max_iterations() {
        let req: TaskRequest = serde_json::from_str(r#"{"task": "hello"}"#).unwrap();
        assert_eq!(req.max_iterations, 50);
    }

    #[tokio::test]
    async fn test_brc31_handshake_endpoint() {
        let app = test_router().await;
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
        // Server nonce should be base64 of 32 bytes
        assert!(!handshake.initial_nonce.is_empty());
    }

    #[tokio::test]
    async fn test_brc31_handshake_rejects_wrong_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);

        let mut config = DmConfig::default();
        config.parent.identity_key =
            "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".into();
        let state = create_app_state(config, path).await;
        let app = build_router_with_state(state);

        let req = Request::builder()
            .method(http::Method::POST)
            .uri("/.well-known/auth")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "version": "0.1",
                    "messageType": "initialRequest",
                    "identityKey": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
                    "initialNonce": "dGVzdA=="
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_chat_request_serde() {
        let req: ChatRequest = serde_json::from_str(r#"{"message": "hello"}"#).unwrap();
        assert_eq!(req.message, "hello");
        assert_eq!(req.max_iterations, 50);
        assert!(req.client_command_id.is_none());
    }

    #[tokio::test]
    async fn test_chat_request_serde_with_client_command_id() {
        let req: ChatRequest =
            serde_json::from_str(r#"{"message":"hello","client_command_id":"cmd-123"}"#).unwrap();
        assert_eq!(req.message, "hello");
        assert_eq!(req.client_command_id.as_deref(), Some("cmd-123"));
    }

    #[tokio::test]
    async fn test_chat_client_command_id_deduplicates() {
        let app = test_router().await;
        let body = serde_json::json!({
            "message": "dedupe me",
            "max_iterations": 1,
            "client_command_id": "cmd-dedupe-1"
        })
        .to_string();

        let req1 = Request::builder()
            .method(http::Method::POST)
            .uri("/chat")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();

        let resp1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let body1 = resp1.into_body().collect().await.unwrap().to_bytes();
        let data1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        let task_id_1 = data1.get("task_id").and_then(|v| v.as_str()).unwrap();
        let session_id_1 = data1.get("session_id").and_then(|v| v.as_str()).unwrap();

        let req2 = Request::builder()
            .method(http::Method::POST)
            .uri("/chat")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();

        let resp2 = app.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
        let data2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        let task_id_2 = data2.get("task_id").and_then(|v| v.as_str()).unwrap();
        let session_id_2 = data2.get("session_id").and_then(|v| v.as_str()).unwrap();

        assert_eq!(task_id_1, task_id_2);
        assert_eq!(session_id_1, session_id_2);
    }

    #[tokio::test]
    async fn test_chat_history_no_tasks() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/chat/history")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // No tasks exist, should 404
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_chat_history_specific_task_no_transcript() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/chat/history?task_id=nonexistent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let messages: Vec<ChatMessage> = serde_json::from_slice(&body).unwrap();
        assert!(messages.is_empty());
    }

    // -- Phase 3 endpoint tests --

    /// Build a router with a workspace that has a task transcript on disk.
    async fn test_router_with_task() -> (Router, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        std::mem::forget(dir);

        let task_id = "test-task-001".to_string();
        let task_dir = workspace.join(format!("tasks/{task_id}"));
        std::fs::create_dir_all(&task_dir).unwrap();

        // Write a small transcript with known events
        let mut transcript = crate::transcript::Transcript::new(task_dir.join("session.jsonl"));
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
        transcript.record_think_request(&[], "gpt-5-mini", 4096);
        transcript.record_think_response(
            "Pi is 3.14159",
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
        transcript.record_proof_created(
            "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "TaskCompletion",
            "deadbeef",
            Some("task:abc123"),
            Some("2026-02-26T12:00:00Z"),
            Some(200),
            Some(3),
            Some("dm-proofs"),
            None,
        );
        transcript.record_checkpoint_created(
            "1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd",
            "Checkpoint",
            "dm-state",
            Some(&serde_json::json!({"task": "test", "iterations": 3})),
        );

        let app = build_router(test_config(), workspace.clone()).await;
        (app, workspace, task_id)
    }

    #[tokio::test]
    async fn test_list_tasks_empty() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/tasks")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let list: TaskListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.total, 0);
        assert!(list.tasks.is_empty());
    }

    #[tokio::test]
    async fn test_list_tasks_with_disk_task() {
        let (app, _workspace, task_id) = test_router_with_task().await;
        let req = Request::builder()
            .uri("/tasks")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let list: TaskListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.total, 1);
        assert_eq!(list.tasks[0].id, task_id);
        assert_eq!(list.tasks[0].task, "compute pi to 5 digits");
        // Should have summed sats from think_response events
        assert_eq!(list.tasks[0].sats_spent, 140); // 80 + 60
                                                   // Should have found proof txids
        assert_eq!(list.tasks[0].proof_txids.len(), 2);
    }

    #[tokio::test]
    async fn test_audit_endpoint() {
        let (app, _workspace, task_id) = test_router_with_task().await;
        let req = Request::builder()
            .uri(format!("/task/{task_id}/audit"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let audit: AuditResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(audit.task_id, task_id);
        // 9 events: user, think_req, think_resp, tool_call, tool_result,
        //           think_req, think_resp, proof_created, checkpoint_created
        //           (no session_end — orphaned tasks are marked Interrupted, not patched)
        assert_eq!(audit.summary.total_events, 9);
        assert_eq!(audit.summary.iterations, 2); // 2 think_request events
        assert_eq!(audit.summary.tool_calls, 1);
        assert_eq!(audit.summary.proof_count, 2);
        assert_eq!(audit.summary.sats_spent, 140);
        assert!(audit.summary.duration_secs >= 0.0);
    }

    #[tokio::test]
    async fn test_audit_not_found() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/task/nonexistent/audit")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_proofs_endpoint() {
        let (app, _workspace, task_id) = test_router_with_task().await;
        let req = Request::builder()
            .uri(format!("/task/{task_id}/proofs"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let proofs: ProofsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(proofs.task_id, task_id);
        // BRC-18 OP_RETURN proofs (proof_created events)
        assert_eq!(proofs.proofs.len(), 1);
        assert_eq!(proofs.proofs[0].proof_type, "TaskCompletion");
        assert_eq!(proofs.proofs[0].hash, "deadbeef");
        // BRC-48 state tokens (checkpoint_created events)
        assert_eq!(proofs.checkpoints.len(), 1);
        assert_eq!(proofs.checkpoints[0].proof_type, "Checkpoint");
    }

    #[tokio::test]
    async fn test_proofs_not_found() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/task/nonexistent/proofs")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_agent_endpoint() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/agent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let agent: AgentResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(agent.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(agent.total_tasks, 0);
        assert_eq!(agent.total_sats_spent, 0);
        assert!(agent.tools.contains(&"execute_bash".to_string()));
        assert!(agent.tools.contains(&"memory_store".to_string()));
        assert!(agent.tools.contains(&"x402_call".to_string()));
        assert!(agent.tools.contains(&"discover_services".to_string()));
        // Phase 2: wallet consolidation reduced from 14→5 tools. Agent endpoint registers
        // sandbox(6) + wallet(5) + memory(2) + messagebox(2) + x402(3) = 18 tools.
        assert!(
            agent.tools.len() >= 16,
            "expected at least 16 tools, got {}",
            agent.tools.len()
        );

        // default_model should be present
        assert!(
            !agent.default_model.is_empty(),
            "default_model should be non-empty"
        );

        // available_models should contain the hardcoded fallback list (no x402 discovery in test)
        assert!(
            agent.available_models.len() >= 5,
            "expected at least 5 available models, got {}",
            agent.available_models.len()
        );
        // Check that known models are present
        let model_ids: Vec<&str> = agent
            .available_models
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert!(model_ids.contains(&"gpt-5-mini"), "missing gpt-5-mini");
        assert!(
            model_ids.contains(&"claude-sonnet-4-6"),
            "missing claude-sonnet-4-6"
        );
        // Check that each model has sensible fields
        for model in &agent.available_models {
            assert!(!model.id.is_empty(), "model id should not be empty");
            assert!(
                !model.provider.is_empty(),
                "model provider should not be empty"
            );
            assert!(
                model.context_window > 0,
                "context_window should be > 0 for {}",
                model.id
            );
            assert!(
                model.max_output_tokens > 0,
                "max_output_tokens should be > 0 for {}",
                model.id
            );
            assert!(
                model.max_input_tokens > 0,
                "max_input_tokens should be > 0 for {}",
                model.id
            );
        }
        // Check provider assignment
        let claude_model = agent
            .available_models
            .iter()
            .find(|m| m.id == "claude-sonnet-4-6")
            .unwrap();
        assert_eq!(claude_model.provider, "claude");
        let openai_model = agent
            .available_models
            .iter()
            .find(|m| m.id == "gpt-5-mini")
            .unwrap();
        assert_eq!(openai_model.provider, "openai");
    }

    #[test]
    fn test_task_summary_serde() {
        let summary = TaskSummary {
            id: "abc".into(),
            task: "hello".into(),
            status: "complete".into(),
            iterations: 3,
            sats_spent: 500,
            started_at: "2026-02-24T12:00:00Z".into(),
            completed_at: Some("2026-02-24T12:01:00Z".into()),
            proof_txids: Vec::new(),
            tokens: 1234,
            tags: Vec::new(),
            time_saved_minutes: 0,
            origin: String::new(),
            conversation_id: None,
        };
        let json = serde_json::to_string(&summary).unwrap();
        // Empty vec should be omitted
        assert!(!json.contains("proof_txids"));
        let parsed: TaskSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "abc");
        assert_eq!(parsed.status, "complete");
    }

    #[test]
    fn test_audit_summary_serde() {
        let summary = AuditSummary {
            total_events: 10,
            iterations: 2,
            sats_spent: 200,
            proof_count: 1,
            tool_calls: 3,
            duration_secs: 5.5,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: AuditSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total_events, 10);
        assert_eq!(parsed.duration_secs, 5.5);
    }

    #[test]
    fn test_proof_detail_serde() {
        let proof = ProofDetail {
            txid: "abcd1234".into(),
            proof_type: "TaskCompletion".into(),
            hash: "deadbeef".into(),
            timestamp: 1740000000.0,
            proof_data: None,
            proof_timestamp: None,
            checkpoint_data: None,
            sats_cost: None,
            basket: None,
            iteration: None,
            prev_hash: None,
        };
        let json = serde_json::to_string(&proof).unwrap();
        let parsed: ProofDetail = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.txid, "abcd1234");
        assert_eq!(parsed.proof_type, "TaskCompletion");
    }

    #[test]
    fn test_agent_response_serde() {
        let agent = AgentResponse {
            identity_key: "02abc".into(),
            balance: 1000,
            version: "0.1.0".into(),
            uptime_secs: 60,
            total_tasks: 5,
            total_sats_spent: 500,
            tools: vec!["execute_bash".into()],
            certificate_status: None,
            certificate_revoked: None,
            default_model: "gpt-5-mini".into(),
            available_models: vec![ModelInfo {
                id: "gpt-5-mini".into(),
                provider: "openai".into(),
                context_window: 400_000,
                max_output_tokens: 128_000,
                max_input_tokens: 272_000,
                supports_tools: true,
                supports_vision: true,
            }],
        };
        let json = serde_json::to_string(&agent).unwrap();
        let parsed: AgentResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.identity_key, "02abc");
        assert_eq!(parsed.balance, 1000);
        assert_eq!(parsed.tools.len(), 1);
        assert_eq!(parsed.default_model, "gpt-5-mini");
        assert_eq!(parsed.available_models.len(), 1);
        assert_eq!(parsed.available_models[0].id, "gpt-5-mini");
        assert_eq!(parsed.available_models[0].provider, "openai");
    }

    // -- parse_push_drop_fields tests --

    #[test]
    fn test_parse_push_drop_op_return() {
        // OP_FALSE OP_RETURN <32 bytes hash> — this is NOT PushDrop, should return None or empty
        // 006a20 + 64 hex = OP_0 OP_RETURN PUSH32
        let script = format!("006a20{}", "ab".repeat(32));
        let fields = parse_push_drop_fields(&script);
        // OP_0 pushes empty, OP_RETURN (0x6a) is an opcode > 75, so we stop
        assert!(fields.is_some());
        assert_eq!(fields.unwrap().len(), 1); // just the empty push from OP_0
    }

    #[test]
    fn test_parse_push_drop_two_fields() {
        // Simulate: PUSH(5 bytes "hello") PUSH(5 bytes "world") OP_2DROP <pubkey> OP_CHECKSIG
        let hello_hex = hex::encode(b"hello");
        let world_hex = hex::encode(b"world");
        // 05 = push 5 bytes, then data, 05 = push 5 bytes, then data, 6d = OP_2DROP
        let script = format!("05{}05{}6d", hello_hex, world_hex);
        let fields = parse_push_drop_fields(&script).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], hello_hex);
        assert_eq!(fields[1], world_hex);
    }

    #[test]
    fn test_parse_push_drop_with_op_pushdata1() {
        // 0x4c <len:1> <data...> OP_DROP
        let data = "aa".repeat(100); // 100 bytes
        let script = format!("4c64{}75", data); // 0x4c, len=100 (0x64), data, OP_DROP
        let fields = parse_push_drop_fields(&script).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], data);
    }

    #[test]
    fn test_parse_push_drop_empty_script() {
        let fields = parse_push_drop_fields("");
        assert!(fields.is_none());
    }

    #[test]
    fn test_parse_push_drop_invalid_hex() {
        let fields = parse_push_drop_fields("not-hex");
        assert!(fields.is_none());
    }

    // -- Tier 2+3 struct serde tests --

    #[test]
    fn test_proof_verification_serde() {
        let v = ProofVerification {
            txid: "abc123".to_string(),
            hash_recompute: "match".to_string(),
            on_chain_match: "not_found".to_string(),
            on_chain_hash: None,
            recomputed_hash: Some("deadbeef".to_string()),
            proof_type: Some("decision".to_string()),
            iteration: Some(3),
        };
        let json = serde_json::to_string(&v).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["txid"], "abc123");
        assert_eq!(parsed["hash_recompute"], "match");
        assert_eq!(parsed["on_chain_match"], "not_found");
        assert!(parsed.get("on_chain_hash").is_none() || parsed["on_chain_hash"].is_null());
        assert_eq!(parsed["recomputed_hash"], "deadbeef");
    }

    #[test]
    fn test_verify_response_serde() {
        let resp = VerifyResponse {
            task_id: "task-1".to_string(),
            verifications: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["task_id"], "task-1");
        assert!(parsed["verifications"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_output_response_serde() {
        let resp = OutputResponse {
            txid: "tx1".to_string(),
            basket: "dm-state".to_string(),
            locking_script: "deadbeef".to_string(),
            data_fields: Some(vec!["aabb".to_string()]),
            satoshis: 1000,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["txid"], "tx1");
        assert_eq!(parsed["basket"], "dm-state");
        assert_eq!(parsed["data_fields"][0], "aabb");
        assert_eq!(parsed["satoshis"], 1000);
    }

    #[test]
    fn test_output_response_no_fields_omitted() {
        let resp = OutputResponse {
            txid: "tx1".to_string(),
            basket: "dm-proofs".to_string(),
            locking_script: "006a20".to_string(),
            data_fields: None,
            satoshis: 200,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("data_fields").is_none() || parsed["data_fields"].is_null());
    }

    #[test]
    fn test_decrypt_response_serde() {
        let resp = DecryptResponse {
            plaintext: r#"{"task":"test"}"#.to_string(),
            parsed_json: Some(serde_json::json!({"task": "test"})),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["parsed_json"]["task"], "test");
    }

    #[tokio::test]
    async fn test_certificates_endpoint() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/certificates")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // In dev mode (no wallet), expect 200 or 500 depending on wallet availability.
        // Without a live wallet, the handler returns 500. Check it's a valid status code.
        let status = resp.status();
        assert!(
            status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
            "Expected 200 or 500, got {status}"
        );
    }

    #[test]
    fn test_certificate_status_serde() {
        let status = crate::certificates::CertificateStatus {
            status: "parent-signed".into(),
            certificate: Some(serde_json::json!({"certifier": "02parent"})),
            identity_key: Some("02agent".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "parent-signed");
        assert_eq!(parsed["identity_key"], "02agent");
    }

    #[test]
    fn test_agent_response_with_cert_status() {
        let resp = AgentResponse {
            identity_key: "02abc".into(),
            balance: 1000,
            version: "0.1.0".into(),
            uptime_secs: 60,
            total_tasks: 5,
            total_sats_spent: 500,
            tools: vec!["execute_bash".into()],
            certificate_status: Some("parent-signed".into()),
            certificate_revoked: None,
            default_model: "gpt-5-mini".into(),
            available_models: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["certificate_status"], "parent-signed");
    }

    #[test]
    fn test_agent_response_without_cert_status() {
        let resp = AgentResponse {
            identity_key: "02abc".into(),
            balance: 1000,
            version: "0.1.0".into(),
            uptime_secs: 60,
            total_tasks: 5,
            total_sats_spent: 500,
            tools: vec![],
            certificate_status: None,
            certificate_revoked: None,
            default_model: "gpt-5-mini".into(),
            available_models: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("certificate_status").is_none());
    }
}
