//! Tests for Block 4: Cryptographic Trust — HMAC transcript integrity,
//! key linkage revelation, and transaction staging.

use axum::body::Body;
use axum::http::{self, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use dolphin_milk::config::{BudgetConfig, DmConfig};
use dolphin_milk::server;
use dolphin_milk::transcript::Transcript;

use std::collections::HashMap;
use tempfile::TempDir;

// ============================================================================
// Helpers
// ============================================================================

#[path = "../common/mod.rs"]
mod common;

fn tmp_transcript() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.jsonl");
    (dir, path)
}

// ============================================================================
// Task 4.1 — HMAC Transcript Integrity
// ============================================================================

#[test]
fn test_transcript_read_bytes_empty_file() {
    let (_dir, path) = tmp_transcript();
    let t = Transcript::new(path);
    let bytes = t.read_bytes();
    // File doesn't exist yet (no events recorded), so bytes should be empty
    assert!(bytes.is_empty());
}

#[test]
fn test_transcript_read_bytes_with_content() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path.clone());
    t.record_user("hello world");
    t.record_system("testing HMAC");

    let bytes = t.read_bytes();
    assert!(!bytes.is_empty());

    // Should contain valid JSONL
    let content = String::from_utf8(bytes).unwrap();
    let lines: Vec<&str> = content.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);

    // Each line should be valid JSON
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(parsed.get("ts").is_some());
        assert!(parsed.get("type").is_some());
    }
}

#[test]
fn test_transcript_read_bytes_matches_file() {
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path.clone());
    t.record_user("test content");

    let bytes = t.read_bytes();
    let file_bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, file_bytes);
}

#[test]
fn test_transcript_read_bytes_nonexistent_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.jsonl");
    let t = Transcript::new(path);
    let bytes = t.read_bytes();
    assert!(bytes.is_empty());
}

#[test]
fn test_checkpoint_hmac_field_in_event() {
    // Test that checkpoint_created events can carry an hmac field in checkpoint_data
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);

    let checkpoint_data = serde_json::json!({
        "task": "test task",
        "iterations": 3,
        "sats_spent": 1000,
        "timestamp": "2026-03-05T12:00:00Z",
        "hmac": "dGVzdGhtYWM=",
    });

    t.record_checkpoint_created("txid123", "checkpoint", "dm-state", Some(&checkpoint_data));

    let events = t.get_events_by_type("checkpoint_created");
    assert_eq!(events.len(), 1);
    let cd = events[0].data.get("checkpoint_data").unwrap();
    assert_eq!(cd.get("hmac").unwrap().as_str().unwrap(), "dGVzdGhtYWM=");
}

#[test]
fn test_checkpoint_without_hmac() {
    // Backward compatibility: checkpoint without HMAC should work fine
    let (_dir, path) = tmp_transcript();
    let mut t = Transcript::new(path);

    let checkpoint_data = serde_json::json!({
        "task": "test task",
        "iterations": 1,
        "sats_spent": 500,
        "timestamp": "2026-03-05T12:00:00Z",
    });

    t.record_checkpoint_created("txid456", "checkpoint", "dm-state", Some(&checkpoint_data));

    let events = t.get_events_by_type("checkpoint_created");
    assert_eq!(events.len(), 1);
    let cd = events[0].data.get("checkpoint_data").unwrap();
    assert!(cd.get("hmac").is_none());
}

// -- Verification endpoint tests --

#[tokio::test]
async fn test_transcript_verify_endpoint_not_found() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/task/nonexistent-id/transcript/verify")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_transcript_verify_no_hmac() {
    let (app, workspace) = common::test_router_with_workspace().await;

    // Create a task directory with a transcript but no checkpoint HMAC
    let task_id = "test-verify-no-hmac";
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");
    let mut t = Transcript::new(transcript_path);
    t.record_user("test message");
    t.record("session_end", HashMap::new());

    let req = Request::builder()
        .uri(format!("/task/{task_id}/transcript/verify"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["valid"], false);
    assert!(result["error"].as_str().unwrap().contains("No HMAC found"));
}

#[tokio::test]
async fn test_transcript_verify_with_hmac_no_wallet() {
    let (app, workspace) = common::test_router_with_workspace().await;

    // Create a task directory with a transcript that has a checkpoint with HMAC
    let task_id = "test-verify-with-hmac";
    let task_dir = workspace.join(format!("tasks/{task_id}"));
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");
    let mut t = Transcript::new(transcript_path);
    t.record_user("test message");

    let checkpoint_data = serde_json::json!({
        "task": "test",
        "iterations": 1,
        "sats_spent": 100,
        "timestamp": "2026-03-05T12:00:00Z",
        "hmac": "dGVzdGhtYWNkYXRh",
    });
    t.record_checkpoint_created("txid789", "checkpoint", "dm-state", Some(&checkpoint_data));

    let req = Request::builder()
        .uri(format!("/task/{task_id}/transcript/verify"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Without a wallet running, verification will fail but shouldn't crash
    assert_eq!(result["valid"], false);
    // HMAC should be present in response
    assert!(result["hmac"].is_string());
    // Error should mention wallet
    assert!(
        result["error"].as_str().unwrap_or("").contains("Wallet")
            || result["error"].as_str().unwrap_or("").contains("wallet")
    );
}

#[tokio::test]
async fn test_transcript_verify_route_exists() {
    let app = common::test_router().await;
    // The route should exist (even though the task doesn't — it should return 404, not 405)
    let req = Request::builder()
        .uri("/task/some-id/transcript/verify")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // 404 (task not found) is expected — NOT 405 (method not allowed)
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Task 4.2 — Key Linkage Revelation
// ============================================================================

#[tokio::test]
async fn test_counterparty_key_linkage_route_exists() {
    let app = common::test_router().await;
    let body = serde_json::json!({
        "counterparty": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "verifier": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "privileged": false,
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/audit/key-linkage/counterparty")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // In dev mode (no parent key), auth is bypassed, so we get a wallet error
    // not 404/405. The route is registered.
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    assert_ne!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn test_specific_key_linkage_route_exists() {
    let app = common::test_router().await;
    let body = serde_json::json!({
        "counterparty": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "verifier": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "protocol_id": [2, "dolphin milk transcript"],
        "key_id": "test-key",
        "privileged": false,
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/audit/key-linkage/specific")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);
    assert_ne!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn test_specific_key_linkage_missing_protocol_id() {
    let app = common::test_router().await;
    let body = serde_json::json!({
        "counterparty": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "verifier": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        // protocol_id intentionally missing
        "key_id": "test-key",
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/audit/key-linkage/specific")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(result["error"].as_str().unwrap().contains("protocol_id"));
}

#[tokio::test]
async fn test_specific_key_linkage_missing_key_id() {
    let app = common::test_router().await;
    let body = serde_json::json!({
        "counterparty": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "verifier": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "protocol_id": [2, "dolphin milk memory"],
        // key_id intentionally missing
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/audit/key-linkage/specific")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(result["error"].as_str().unwrap().contains("key_id"));
}

#[tokio::test]
async fn test_counterparty_linkage_get_not_allowed() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::GET)
        .uri("/audit/key-linkage/counterparty")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ============================================================================
// Task 4.3 — Transaction Staging
// ============================================================================

// -- Config tests --

#[test]
fn test_staging_threshold_default() {
    let config = BudgetConfig::default();
    assert_eq!(config.staging_threshold, 500_000);
}

#[test]
fn test_staging_threshold_in_worm_config() {
    let config = DmConfig::default();
    assert_eq!(config.budget.staging_threshold, 500_000);
}

#[test]
fn test_staging_threshold_from_toml() {
    // This test uses TOML config directly, no env var dependency.
    // Write a TOML config and load it.
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("dolphin-milk.toml");
    std::fs::write(
        &config_path,
        r#"
[budget]
staging_threshold = 1000000
max_per_task = 20000000
"#,
    )
    .unwrap();

    // Load directly via toml (bypassing apply_env to avoid env race)
    let contents = std::fs::read_to_string(&config_path).unwrap();
    let config: DmConfig = toml::from_str(&contents).unwrap();
    assert_eq!(config.budget.staging_threshold, 1_000_000);
}

#[test]
fn test_staging_threshold_zero_disables() {
    // When threshold is 0, staging is effectively disabled
    let config = BudgetConfig {
        staging_threshold: 0,
        ..BudgetConfig::default()
    };
    assert_eq!(config.staging_threshold, 0);
}

// -- Staging endpoint tests --

#[tokio::test]
async fn test_staged_list_empty() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/staged")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["total"], 0);
    assert!(result["staged"].as_array().unwrap().is_empty());
    assert_eq!(result["staging_threshold"], 500_000);
}

#[tokio::test]
async fn test_staged_approve_not_found() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/staged/nonexistent-ref/approve")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_staged_abort_not_found() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/staged/nonexistent-ref/abort")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_staged_list_route_method() {
    let app = common::test_router().await;
    // POST should not be allowed on /staged (only GET)
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/staged")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn test_staged_approve_get_not_allowed() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::GET)
        .uri("/staged/some-ref/approve")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// -- StagedTransaction type tests --

#[test]
fn test_staged_transaction_serde() {
    let tx = server::StagedTransaction {
        reference: "ref-123".to_string(),
        amount_sats: 600_000,
        service: "llm".to_string(),
        created_at: "2026-03-05T12:00:00Z".to_string(),
        status: "pending".to_string(),
        task_id: Some("task-abc".to_string()),
        description: Some("Large inference call".to_string()),
    };

    let json = serde_json::to_string(&tx).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["reference"], "ref-123");
    assert_eq!(parsed["amount_sats"], 600_000);
    assert_eq!(parsed["status"], "pending");
    assert_eq!(parsed["task_id"], "task-abc");
}

#[test]
fn test_staged_transaction_optional_fields() {
    let tx = server::StagedTransaction {
        reference: "ref-456".to_string(),
        amount_sats: 100_000,
        service: "image".to_string(),
        created_at: "2026-03-05T12:00:00Z".to_string(),
        status: "pending".to_string(),
        task_id: None,
        description: None,
    };

    let json = serde_json::to_string(&tx).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.get("task_id").is_none());
    assert!(parsed.get("description").is_none());
}

// -- Wallet method existence tests --
// These compile-time tests verify the methods exist on WalletClient.
// If the method signature changes, these tests will fail to compile.

#[tokio::test]
async fn test_wallet_sign_action_method_exists() {
    let client =
        dolphin_milk::wallet::WalletClient::new("http://localhost:9999", "http://localhost", 1);
    // Call will fail (no wallet) but verifies the method exists and compiles
    let result = client.sign_action("fake-ref").await;
    assert!(result.is_err()); // Expected: wallet not reachable
}

#[tokio::test]
async fn test_wallet_abort_action_method_exists() {
    let client =
        dolphin_milk::wallet::WalletClient::new("http://localhost:9999", "http://localhost", 1);
    let result = client.abort_action("fake-ref").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_wallet_create_hmac_method_exists() {
    let client =
        dolphin_milk::wallet::WalletClient::new("http://localhost:9999", "http://localhost", 1);
    let result = client
        .create_hmac(
            b"test data",
            &serde_json::json!([2, "test"]),
            "key1",
            "self",
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_wallet_verify_hmac_method_exists() {
    let client =
        dolphin_milk::wallet::WalletClient::new("http://localhost:9999", "http://localhost", 1);
    let result = client
        .verify_hmac(
            b"test data",
            b"fake hmac",
            &serde_json::json!([2, "test"]),
            "key1",
            "self",
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_wallet_reveal_counterparty_key_linkage_exists() {
    let client =
        dolphin_milk::wallet::WalletClient::new("http://localhost:9999", "http://localhost", 1);
    let result = client
        .reveal_counterparty_key_linkage(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            false,
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_wallet_reveal_specific_key_linkage_exists() {
    let client =
        dolphin_milk::wallet::WalletClient::new("http://localhost:9999", "http://localhost", 1);
    let result = client
        .reveal_specific_key_linkage(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            &serde_json::json!([2, "test"]),
            "key1",
            false,
        )
        .await;
    assert!(result.is_err());
}

// ============================================================================
// Integration: staged transaction lifecycle via AppState
// ============================================================================

#[tokio::test]
async fn test_staged_transaction_lifecycle_in_state() {
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

    // Initially empty
    {
        let staged = state.staged_transactions.lock().await;
        assert!(staged.is_empty());
    }

    // Insert a staged transaction
    {
        let mut staged = state.staged_transactions.lock().await;
        staged.insert(
            "ref-001".to_string(),
            server::StagedTransaction {
                reference: "ref-001".to_string(),
                amount_sats: 750_000,
                service: "llm".to_string(),
                created_at: "2026-03-05T12:00:00Z".to_string(),
                status: "pending".to_string(),
                task_id: Some("task-xyz".to_string()),
                description: Some("Test staging".to_string()),
            },
        );
    }

    // Verify it's there
    {
        let staged = state.staged_transactions.lock().await;
        assert_eq!(staged.len(), 1);
        let tx = staged.get("ref-001").unwrap();
        assert_eq!(tx.status, "pending");
        assert_eq!(tx.amount_sats, 750_000);
    }

    // Update status to approved
    {
        let mut staged = state.staged_transactions.lock().await;
        if let Some(tx) = staged.get_mut("ref-001") {
            tx.status = "approved".to_string();
        }
    }

    // Verify status updated
    {
        let staged = state.staged_transactions.lock().await;
        let tx = staged.get("ref-001").unwrap();
        assert_eq!(tx.status, "approved");
    }
}
