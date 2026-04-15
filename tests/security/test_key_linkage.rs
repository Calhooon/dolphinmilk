//! Tests for BRC-69 key linkage revelation — structured format, hashing,
//! audit logging, and CLI subcommand parsing.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use clap::Parser;
use dolphin_milk::audit::revelation::{
    compute_revelation_hash, Revelation, RevelationLog, RevelationsListResponse,
};
use dolphin_milk::cli::{AuditAction, Cli, Command};
use std::path::PathBuf;

#[path = "../common/mod.rs"]
mod common;

// ============================================================================
// Test 1: Revelation struct serializes correctly
// ============================================================================

#[test]
fn test_revelation_creation_counterparty() {
    let r = Revelation::counterparty(
        "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab",
        "02verifier_key_hex",
        serde_json::json!({"linkage": "data"}),
    );

    assert_eq!(r.revelation_type, "counterparty");
    assert_eq!(
        r.counterparty,
        "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab"
    );
    assert_eq!(r.requested_by, "02verifier_key_hex");
    assert!(r.protocol_id.is_none());
    assert!(r.key_id.is_none());
    assert!(!r.id.is_empty());
    assert!(!r.timestamp.is_empty());
    assert!(!r.revelation_hash.is_empty());
    assert_eq!(r.revelation_hash.len(), 64); // SHA-256 hex

    // Verify it serializes to JSON
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["revelation_type"], "counterparty");
    assert!(json.get("protocol_id").is_none()); // skip_serializing_if None
    assert!(json.get("key_id").is_none());
}

#[test]
fn test_revelation_creation_specific() {
    let r = Revelation::specific(
        "02counterparty_key",
        "[2, \"worm message signature\"]",
        "message",
        "02verifier_key",
        serde_json::json!({"specific": "linkage_data"}),
    );

    assert_eq!(r.revelation_type, "specific");
    assert_eq!(r.counterparty, "02counterparty_key");
    assert_eq!(
        r.protocol_id.as_deref(),
        Some("[2, \"worm message signature\"]")
    );
    assert_eq!(r.key_id.as_deref(), Some("message"));
    assert_eq!(r.requested_by, "02verifier_key");
    assert_eq!(r.revelation_hash.len(), 64);

    // Serializes with protocol_id and key_id present
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["protocol_id"], "[2, \"worm message signature\"]");
    assert_eq!(json["key_id"], "message");
}

#[test]
fn test_revelation_serde_roundtrip() {
    let r = Revelation::counterparty("02abc", "02def", serde_json::json!({"test": true}));
    let json_str = serde_json::to_string(&r).unwrap();
    let r2: Revelation = serde_json::from_str(&json_str).unwrap();

    assert_eq!(r.id, r2.id);
    assert_eq!(r.revelation_type, r2.revelation_type);
    assert_eq!(r.counterparty, r2.counterparty);
    assert_eq!(r.requested_by, r2.requested_by);
    assert_eq!(r.timestamp, r2.timestamp);
    assert_eq!(r.revelation_hash, r2.revelation_hash);
    assert_eq!(r.linkage_data, r2.linkage_data);
}

// ============================================================================
// Test 2: Revelation hash is deterministic
// ============================================================================

#[test]
fn test_revelation_hash_deterministic() {
    let data = serde_json::json!({"key": "value"});
    let ts = "2026-03-25T12:00:00Z";

    let hash1 = compute_revelation_hash("counterparty", "02abc", None, None, "02def", ts, &data);
    let hash2 = compute_revelation_hash("counterparty", "02abc", None, None, "02def", ts, &data);

    assert_eq!(hash1, hash2);
    assert_eq!(hash1.len(), 64); // SHA-256 hex
}

#[test]
fn test_revelation_hash_changes_with_type() {
    let data = serde_json::json!({"key": "value"});
    let ts = "2026-03-25T12:00:00Z";

    let hash_counterparty =
        compute_revelation_hash("counterparty", "02abc", None, None, "02def", ts, &data);
    let hash_specific = compute_revelation_hash(
        "specific",
        "02abc",
        Some("proto"),
        Some("key"),
        "02def",
        ts,
        &data,
    );

    assert_ne!(hash_counterparty, hash_specific);
}

#[test]
fn test_revelation_hash_changes_with_counterparty() {
    let data = serde_json::json!({"key": "value"});
    let ts = "2026-03-25T12:00:00Z";

    let hash1 = compute_revelation_hash("counterparty", "02abc", None, None, "02def", ts, &data);
    let hash2 = compute_revelation_hash("counterparty", "02xyz", None, None, "02def", ts, &data);

    assert_ne!(hash1, hash2);
}

#[test]
fn test_revelation_hash_changes_with_data() {
    let ts = "2026-03-25T12:00:00Z";

    let hash1 = compute_revelation_hash(
        "counterparty",
        "02abc",
        None,
        None,
        "02def",
        ts,
        &serde_json::json!({"key": "value1"}),
    );
    let hash2 = compute_revelation_hash(
        "counterparty",
        "02abc",
        None,
        None,
        "02def",
        ts,
        &serde_json::json!({"key": "value2"}),
    );

    assert_ne!(hash1, hash2);
}

// ============================================================================
// Test 3: RevelationLog record — writing to disk
// ============================================================================

#[test]
fn test_revelation_log_record() {
    let dir = tempfile::tempdir().unwrap();
    let log = RevelationLog::from_dir(dir.path().to_path_buf());

    let r = Revelation::counterparty("02abc", "02def", serde_json::json!({"test": true}));

    log.record(&r).unwrap();

    // Verify file exists
    let path = dir.path().join(format!("{}.json", r.id));
    assert!(path.exists());

    // Verify content is valid JSON
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: Revelation = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed.id, r.id);
    assert_eq!(parsed.revelation_hash, r.revelation_hash);
}

#[test]
fn test_revelation_log_record_multiple() {
    let dir = tempfile::tempdir().unwrap();
    let log = RevelationLog::from_dir(dir.path().to_path_buf());

    let r1 = Revelation::counterparty("02abc", "02def", serde_json::json!({"first": true}));
    let r2 = Revelation::specific(
        "02ghi",
        "proto",
        "key",
        "02jkl",
        serde_json::json!({"second": true}),
    );

    log.record(&r1).unwrap();
    log.record(&r2).unwrap();

    // Both files exist
    assert!(dir.path().join(format!("{}.json", r1.id)).exists());
    assert!(dir.path().join(format!("{}.json", r2.id)).exists());
}

// ============================================================================
// Test 4: RevelationLog list — reading back revelations sorted
// ============================================================================

#[test]
fn test_revelation_log_list() {
    let dir = tempfile::tempdir().unwrap();
    let log = RevelationLog::from_dir(dir.path().to_path_buf());

    // Write revelations with known timestamps for ordering
    let mut r1 = Revelation::counterparty("02first", "02verifier", serde_json::json!({"order": 1}));
    r1.timestamp = "2026-03-25T10:00:00Z".to_string();

    let mut r2 =
        Revelation::counterparty("02second", "02verifier", serde_json::json!({"order": 2}));
    r2.timestamp = "2026-03-25T12:00:00Z".to_string();

    let mut r3 = Revelation::specific(
        "02third",
        "proto",
        "key",
        "02verifier",
        serde_json::json!({"order": 3}),
    );
    r3.timestamp = "2026-03-25T11:00:00Z".to_string();

    // Write out of order
    log.record(&r2).unwrap();
    log.record(&r3).unwrap();
    log.record(&r1).unwrap();

    let list = log.list();
    assert_eq!(list.len(), 3);

    // Verify sorted by timestamp ascending
    assert_eq!(list[0].counterparty, "02first");
    assert_eq!(list[1].counterparty, "02third");
    assert_eq!(list[2].counterparty, "02second");
}

// ============================================================================
// Test 5: RevelationLog empty — returns empty vec
// ============================================================================

#[test]
fn test_revelation_log_empty() {
    let dir = tempfile::tempdir().unwrap();
    let log = RevelationLog::from_dir(dir.path().to_path_buf());
    let list = log.list();
    assert!(list.is_empty());
}

#[test]
fn test_revelation_log_nonexistent_dir() {
    let log = RevelationLog::from_dir(PathBuf::from("/nonexistent/path/revelations"));
    let list = log.list();
    assert!(list.is_empty());
}

#[test]
fn test_revelation_log_skips_non_json() {
    let dir = tempfile::tempdir().unwrap();
    let log = RevelationLog::from_dir(dir.path().to_path_buf());

    // Write a revelation
    let r = Revelation::counterparty("02abc", "02def", serde_json::json!({"test": true}));
    log.record(&r).unwrap();

    // Also write a non-JSON file
    std::fs::write(dir.path().join("notes.txt"), "not a revelation").unwrap();

    let list = log.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, r.id);
}

// ============================================================================
// Test 6: CLI audit subcommand parsing
// ============================================================================

#[test]
fn test_audit_reveal_linkage_parse() {
    let cli = Cli::try_parse_from(["dolphin-milk", "audit", "reveal-linkage", "02abcdef"]).unwrap();

    match cli.command {
        Some(Command::Audit(args)) => match args.action {
            AuditAction::RevealLinkage {
                counterparty,
                verifier,
                protocol,
                key_id,
                workspace,
            } => {
                assert_eq!(counterparty, "02abcdef");
                assert!(verifier.is_none());
                assert!(protocol.is_none());
                assert!(key_id.is_none());
                assert!(workspace.is_none());
            }
            _ => panic!("Expected RevealLinkage"),
        },
        _ => panic!("Expected Audit command"),
    }
}

#[test]
fn test_audit_reveal_linkage_with_options_parse() {
    let cli = Cli::try_parse_from([
        "dolphin-milk",
        "audit",
        "reveal-linkage",
        "02abcdef",
        "--verifier",
        "02verifier",
        "--protocol",
        "dolphin milk message signature",
        "--key-id",
        "message",
        "--workspace",
        "/tmp/test",
    ])
    .unwrap();

    match cli.command {
        Some(Command::Audit(args)) => match args.action {
            AuditAction::RevealLinkage {
                counterparty,
                verifier,
                protocol,
                key_id,
                workspace,
            } => {
                assert_eq!(counterparty, "02abcdef");
                assert_eq!(verifier.as_deref(), Some("02verifier"));
                assert_eq!(protocol.as_deref(), Some("dolphin milk message signature"));
                assert_eq!(key_id.as_deref(), Some("message"));
                assert_eq!(workspace.as_deref(), Some("/tmp/test"));
            }
            _ => panic!("Expected RevealLinkage"),
        },
        _ => panic!("Expected Audit command"),
    }
}

#[test]
fn test_audit_list_revelations_parse() {
    let cli = Cli::try_parse_from(["dolphin-milk", "audit", "list-revelations"]).unwrap();

    match cli.command {
        Some(Command::Audit(args)) => match args.action {
            AuditAction::ListRevelations { workspace } => {
                assert!(workspace.is_none());
            }
            _ => panic!("Expected ListRevelations"),
        },
        _ => panic!("Expected Audit command"),
    }
}

#[test]
fn test_audit_list_revelations_with_workspace_parse() {
    let cli = Cli::try_parse_from([
        "dolphin-milk",
        "audit",
        "list-revelations",
        "--workspace",
        "/custom/path",
    ])
    .unwrap();

    match cli.command {
        Some(Command::Audit(args)) => match args.action {
            AuditAction::ListRevelations { workspace } => {
                assert_eq!(workspace.as_deref(), Some("/custom/path"));
            }
            _ => panic!("Expected ListRevelations"),
        },
        _ => panic!("Expected Audit command"),
    }
}

// ============================================================================
// Test 7: GET /audit/revelations endpoint
// ============================================================================

#[tokio::test]
async fn test_revelations_endpoint_exists() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/audit/revelations")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: RevelationsListResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(result.count, 0);
    assert!(result.revelations.is_empty());
}

#[tokio::test]
async fn test_revelations_endpoint_with_data() {
    let (app, workspace) = common::test_router_with_workspace().await;

    // Write a revelation directly to the workspace
    let log = RevelationLog::new(&workspace.to_string_lossy());
    let r = Revelation::counterparty("02abc", "02def", serde_json::json!({"test": true}));
    log.record(&r).unwrap();

    let req = Request::builder()
        .uri("/audit/revelations")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let result: RevelationsListResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(result.count, 1);
    assert_eq!(result.revelations[0].id, r.id);
    assert_eq!(result.revelations[0].counterparty, "02abc");
}

// ============================================================================
// Test 8: Unique revelation IDs
// ============================================================================

#[test]
fn test_revelation_ids_are_unique() {
    let r1 = Revelation::counterparty("02a", "02b", serde_json::json!({}));
    let r2 = Revelation::counterparty("02a", "02b", serde_json::json!({}));
    assert_ne!(r1.id, r2.id);
}

// ============================================================================
// Test 9: RevelationsListResponse serde
// ============================================================================

#[test]
fn test_revelations_list_response_serde() {
    let resp = RevelationsListResponse {
        revelations: vec![],
        count: 0,
    };

    let json = serde_json::to_string(&resp).unwrap();
    let parsed: RevelationsListResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.count, 0);
    assert!(parsed.revelations.is_empty());
}

// ============================================================================
// Test 10: Revelation hash includes all fields
// ============================================================================

#[test]
fn test_revelation_hash_includes_requester() {
    let data = serde_json::json!({"key": "value"});
    let ts = "2026-03-25T12:00:00Z";

    let hash1 = compute_revelation_hash(
        "counterparty",
        "02abc",
        None,
        None,
        "02requester_a",
        ts,
        &data,
    );
    let hash2 = compute_revelation_hash(
        "counterparty",
        "02abc",
        None,
        None,
        "02requester_b",
        ts,
        &data,
    );

    assert_ne!(hash1, hash2);
}

#[test]
fn test_revelation_hash_includes_timestamp() {
    let data = serde_json::json!({"key": "value"});

    let hash1 = compute_revelation_hash(
        "counterparty",
        "02abc",
        None,
        None,
        "02def",
        "2026-03-25T12:00:00Z",
        &data,
    );
    let hash2 = compute_revelation_hash(
        "counterparty",
        "02abc",
        None,
        None,
        "02def",
        "2026-03-25T13:00:00Z",
        &data,
    );

    assert_ne!(hash1, hash2);
}

// ============================================================================
// Test 11: Workspace auto-creation
// ============================================================================

#[test]
fn test_revelation_log_creates_dir() {
    let dir = tempfile::tempdir().unwrap();
    let log_dir = dir.path().join("sub").join("audit_revelations");
    assert!(!log_dir.exists());

    let _log = RevelationLog::from_dir(log_dir.clone());
    assert!(log_dir.exists());
}
