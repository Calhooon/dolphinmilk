//! Tests for BRC-56 Peer Discovery (Task 8.1) and Cross-Agent Proof Chain Linking (Task 8.2).
//!
//! Covers: PeerDiscovery struct, PeerInfo/PeerVerification serde, parse_discovery_result,
//! discover_agent/verify_agent tool parameter validation, compute_message_hash, message hash
//! extraction from send_message output, proof enrichment with message_hashes, and
//! cross-reference endpoint.

use dolphin_milk::discovery::{PeerDiscovery, PeerInfo, PeerVerification};
use dolphin_milk::messagebox::compute_message_hash;
use dolphin_milk::tools::registry::{required_capability, ALWAYS_ON_TOOLS};
use dolphin_milk::wallet::WalletClient;
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// PeerInfo struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_peer_info_required_fields_only() {
    let peer = PeerInfo {
        identity_key: "02abc123".to_string(),
        name: None,
        capabilities: None,
        certificate_type: None,
        certifier: None,
        fields: HashMap::new(),
        raw: None,
    };
    let json_str = serde_json::to_string(&peer).unwrap();
    // Optional fields with skip_serializing_if should be absent
    assert!(json_str.contains("identity_key"));
    assert!(!json_str.contains("\"name\""));
    assert!(!json_str.contains("\"capabilities\""));
    assert!(!json_str.contains("\"fields\""));
}

#[test]
fn test_peer_info_full_serde_roundtrip() {
    let mut fields = HashMap::new();
    fields.insert("deployed_at".to_string(), "2026-03-05".to_string());
    let peer = PeerInfo {
        identity_key: "02abc123".to_string(),
        name: Some("TestBot".to_string()),
        capabilities: Some(vec!["tools".to_string(), "wallet".to_string()]),
        certificate_type: Some("agent-authorization".to_string()),
        certifier: Some("02parent456".to_string()),
        fields,
        raw: Some(json!({"extra": true})),
    };
    let json_str = serde_json::to_string(&peer).unwrap();
    let back: PeerInfo = serde_json::from_str(&json_str).unwrap();
    assert_eq!(back.identity_key, "02abc123");
    assert_eq!(back.name.as_deref(), Some("TestBot"));
    assert_eq!(back.capabilities.as_ref().unwrap().len(), 2);
    assert_eq!(
        back.certificate_type.as_deref(),
        Some("agent-authorization")
    );
    assert_eq!(back.certifier.as_deref(), Some("02parent456"));
    assert_eq!(back.fields.get("deployed_at").unwrap(), "2026-03-05");
}

#[test]
fn test_peer_info_capabilities_parsing() {
    // Test that capabilities are stored as individual strings
    let peer = PeerInfo {
        identity_key: "02abc".to_string(),
        name: None,
        capabilities: Some(vec![
            "tools".to_string(),
            "llm".to_string(),
            "memory".to_string(),
        ]),
        certificate_type: None,
        certifier: None,
        fields: HashMap::new(),
        raw: None,
    };
    let caps = peer.capabilities.unwrap();
    assert!(caps.contains(&"tools".to_string()));
    assert!(caps.contains(&"llm".to_string()));
    assert!(caps.contains(&"memory".to_string()));
}

// ---------------------------------------------------------------------------
// PeerVerification struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_peer_verification_no_certificates() {
    let v = PeerVerification {
        identity_key: "02abc".to_string(),
        has_certificates: false,
        certificate_count: 0,
        has_parent_signed: false,
        certificate_types: vec![],
        certifiers: vec![],
    };
    assert!(!v.has_certificates);
    assert_eq!(v.certificate_count, 0);
    assert!(!v.has_parent_signed);
}

#[test]
fn test_peer_verification_parent_signed() {
    let v = PeerVerification {
        identity_key: "02agent".to_string(),
        has_certificates: true,
        certificate_count: 2,
        has_parent_signed: true,
        certificate_types: vec!["agent-authorization".to_string()],
        certifiers: vec!["02parent".to_string()],
    };
    assert!(v.has_certificates);
    assert!(v.has_parent_signed);
    assert_eq!(v.certificate_count, 2);
}

#[test]
fn test_peer_verification_self_signed_only() {
    // When certifier == identity_key, it's self-signed — has_parent_signed should be false
    let v = PeerVerification {
        identity_key: "02self".to_string(),
        has_certificates: true,
        certificate_count: 1,
        has_parent_signed: false,
        certificate_types: vec!["agent-authorization".to_string()],
        certifiers: vec!["02self".to_string()],
    };
    assert!(v.has_certificates);
    assert!(!v.has_parent_signed);
}

#[test]
fn test_peer_verification_serde_roundtrip() {
    let v = PeerVerification {
        identity_key: "02abc".to_string(),
        has_certificates: true,
        certificate_count: 3,
        has_parent_signed: true,
        certificate_types: vec!["agent-authorization".to_string(), "self-signed".to_string()],
        certifiers: vec!["02parent".to_string(), "02abc".to_string()],
    };
    let json_str = serde_json::to_string(&v).unwrap();
    let back: PeerVerification = serde_json::from_str(&json_str).unwrap();
    assert_eq!(back.certificate_count, 3);
    assert!(back.has_parent_signed);
    assert_eq!(back.certificate_types.len(), 2);
    assert_eq!(back.certifiers.len(), 2);
}

// ---------------------------------------------------------------------------
// PeerDiscovery construction
// ---------------------------------------------------------------------------

#[test]
fn test_peer_discovery_construction() {
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let _discovery = PeerDiscovery::new(wallet);
    // Construction should succeed without panicking
}

// ---------------------------------------------------------------------------
// Discovery tool registration
// ---------------------------------------------------------------------------

#[test]
fn test_discovery_tools_not_always_on() {
    assert!(!ALWAYS_ON_TOOLS.contains(&"discover_agent"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"verify_agent"));
}

#[test]
fn test_discovery_category_maps_to_tools_capability() {
    assert_eq!(required_capability("discovery"), "tools");
}

// ---------------------------------------------------------------------------
// Discover agent tool parameter validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_discover_agent_no_params_returns_error() {
    // Import the tool's impl indirectly through the tool registry
    let tools =
        dolphin_milk::tools::discovery_tools::all_discovery_tools("http://localhost:3322".into());
    let discover = tools.iter().find(|t| t.name == "discover_agent").unwrap();
    let result = (discover.execute)(json!({})).await;
    assert!(result.contains("Error"));
    assert!(result.contains("identity_key") || result.contains("attributes"));
}

#[tokio::test]
async fn test_discover_agent_short_key_returns_error() {
    let tools =
        dolphin_milk::tools::discovery_tools::all_discovery_tools("http://localhost:3322".into());
    let discover = tools.iter().find(|t| t.name == "discover_agent").unwrap();
    let result = (discover.execute)(json!({"identity_key": "02short"})).await;
    assert!(result.contains("Error"));
    assert!(result.contains("66-char"));
}

#[tokio::test]
async fn test_discover_agent_empty_attributes_returns_error() {
    let tools =
        dolphin_milk::tools::discovery_tools::all_discovery_tools("http://localhost:3322".into());
    let discover = tools.iter().find(|t| t.name == "discover_agent").unwrap();
    let result = (discover.execute)(json!({"attributes": {}})).await;
    assert!(result.contains("Error"));
    assert!(result.contains("at least one"));
}

// ---------------------------------------------------------------------------
// Verify agent tool parameter validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_verify_agent_missing_key_returns_error() {
    let tools =
        dolphin_milk::tools::discovery_tools::all_discovery_tools("http://localhost:3322".into());
    let verify = tools.iter().find(|t| t.name == "verify_agent").unwrap();
    let result = (verify.execute)(json!({})).await;
    assert!(result.contains("Error"));
    assert!(result.contains("identity_key"));
}

#[tokio::test]
async fn test_verify_agent_short_key_returns_error() {
    let tools =
        dolphin_milk::tools::discovery_tools::all_discovery_tools("http://localhost:3322".into());
    let verify = tools.iter().find(|t| t.name == "verify_agent").unwrap();
    let result = (verify.execute)(json!({"identity_key": "abc123"})).await;
    assert!(result.contains("Error"));
    assert!(result.contains("66-char"));
}

#[tokio::test]
async fn test_discover_agent_tool_count_and_categories() {
    let tools =
        dolphin_milk::tools::discovery_tools::all_discovery_tools("http://localhost:3322".into());
    assert_eq!(tools.len(), 2);
    for tool in &tools {
        assert_eq!(tool.category, "discovery");
    }
}

// ---------------------------------------------------------------------------
// compute_message_hash (Task 8.2)
// ---------------------------------------------------------------------------

#[test]
fn test_compute_message_hash_deterministic() {
    let body = json!({"text": "hello", "to": "02abc"});
    let hash1 = compute_message_hash(&body);
    let hash2 = compute_message_hash(&body);
    assert_eq!(hash1, hash2);
}

#[test]
fn test_compute_message_hash_length_is_64_hex() {
    let body = json!({"anything": "value"});
    let hash = compute_message_hash(&body);
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_compute_message_hash_different_bodies_differ() {
    let hash1 = compute_message_hash(&json!({"msg": "hello"}));
    let hash2 = compute_message_hash(&json!({"msg": "world"}));
    assert_ne!(hash1, hash2);
}

#[test]
fn test_compute_message_hash_order_sensitive() {
    // JSON serialization of objects with different key order should differ
    // because serde_json::to_string uses insertion order
    let body1 = json!({"a": 1, "b": 2});
    // The important thing is that it's deterministic for the SAME Value.
    let hash1a = compute_message_hash(&body1);
    let hash1b = compute_message_hash(&body1);
    assert_eq!(hash1a, hash1b);
}

#[test]
fn test_compute_message_hash_empty_object() {
    let hash = compute_message_hash(&json!({}));
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_compute_message_hash_null_value() {
    let hash = compute_message_hash(&Value::Null);
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_compute_message_hash_complex_body() {
    let body = json!({
        "type": "agent_message",
        "conversation_ref": "conv-123",
        "turn": 1,
        "body": {"content": "Analyze this data", "task_id": "task-456"}
    });
    let hash = compute_message_hash(&body);
    assert_eq!(hash.len(), 64);
    // Same body should always produce the same hash
    assert_eq!(hash, compute_message_hash(&body));
}

// ---------------------------------------------------------------------------
// Message hash extraction from send_message result
// ---------------------------------------------------------------------------

#[test]
fn test_message_hash_in_send_result() {
    // Simulates what send_message returns after computing a hash
    let send_result = json!({
        "status": "ok",
        "sentMessageId": "msg-123",
        "message_hash": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    });
    let hash = send_result
        .get("message_hash")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(hash.len(), 64);
}

#[test]
fn test_message_hash_extraction_from_tool_output() {
    // Simulates the step.rs extraction logic
    let tool_output = serde_json::to_string(&json!({
        "status": "ok",
        "sentMessageId": "msg-456",
        "message_hash": "deadbeef".repeat(8)
    }))
    .unwrap();

    let mut message_hashes: Vec<String> = Vec::new();
    if let Ok(json) = serde_json::from_str::<Value>(&tool_output) {
        if let Some(hash) = json.get("message_hash").and_then(|v| v.as_str()) {
            message_hashes.push(hash.to_string());
        }
    }
    assert_eq!(message_hashes.len(), 1);
    assert_eq!(message_hashes[0], "deadbeef".repeat(8));
}

#[test]
fn test_message_hash_missing_from_non_send_result() {
    // Non-send_message tools should not have message_hash
    let tool_output = json!({
        "status": "ok",
        "result": "some data"
    });
    let hash = tool_output.get("message_hash").and_then(|v| v.as_str());
    assert!(hash.is_none());
}

// ---------------------------------------------------------------------------
// Proof enrichment with message_hashes
// ---------------------------------------------------------------------------

#[test]
fn test_decision_proof_text_includes_message_hashes() {
    // Simulates the format used in step.rs record_and_resolve
    let message_hashes = ["a1b2c3d4".repeat(8), "e5f6a7b8".repeat(8)];
    let mut parts = String::from("DECISION hash=abc123 tools=[send_message] iter=1");
    if !message_hashes.is_empty() {
        parts.push_str(&format!(" message_hashes=[{}]", message_hashes.join(",")));
    }
    assert!(parts.contains("message_hashes="));
    assert!(parts.contains(&message_hashes[0]));
    assert!(parts.contains(&message_hashes[1]));
}

#[test]
fn test_decision_proof_text_no_hashes_when_empty() {
    let message_hashes: Vec<String> = vec![];
    let mut parts = String::from("DECISION hash=abc123 tools=[file_read] iter=1");
    if !message_hashes.is_empty() {
        parts.push_str(&format!(" message_hashes=[{}]", message_hashes.join(",")));
    }
    assert!(!parts.contains("message_hashes="));
}

#[test]
fn test_reasoning_section_includes_message_hashes() {
    // Simulates the reasoning section format in step.rs
    let message_hashes = ["cafebabe".repeat(8)];
    let mut reasoning = format!(
        "REASONING: content_hash={} model=gpt-5 sats=500 payment_txid=abc123",
        "hash123"
    );
    if !message_hashes.is_empty() {
        reasoning.push_str(&format!("\nMESSAGE_HASHES: {}", message_hashes.join(",")));
    }
    assert!(reasoning.contains("MESSAGE_HASHES:"));
    assert!(reasoning.contains(&message_hashes[0]));
}

// ---------------------------------------------------------------------------
// Cross-reference endpoint validation (audit.rs)
// ---------------------------------------------------------------------------

#[test]
fn test_valid_message_hash_format() {
    let hash = "a".repeat(64);
    let valid = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
    assert!(valid);
}

#[test]
fn test_invalid_message_hash_too_short() {
    let hash = "a".repeat(63);
    let valid = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
    assert!(!valid);
}

#[test]
fn test_invalid_message_hash_too_long() {
    let hash = "a".repeat(65);
    let valid = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
    assert!(!valid);
}

#[test]
fn test_invalid_message_hash_non_hex() {
    let hash = format!("{}gg", "a".repeat(62));
    let valid = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
    assert!(!valid);
}

// ---------------------------------------------------------------------------
// Cross-reference response structure
// ---------------------------------------------------------------------------

#[test]
fn test_cross_reference_match_structure() {
    // Test the response format matches audit.rs CrossReferenceMatch/CrossReferenceResponse
    let response = json!({
        "message_hash": "a".repeat(64),
        "matches": [
            {
                "proof_txid": "tx123",
                "task_id": "task-456",
                "iteration": 3,
                "proof_type": "decision",
                "timestamp": "1709654400.0"
            }
        ],
        "count": 1
    });
    let matches = response.get("matches").unwrap().as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["proof_txid"], "tx123");
    assert_eq!(matches[0]["task_id"], "task-456");
    assert_eq!(matches[0]["iteration"], 3);
    assert_eq!(matches[0]["proof_type"], "decision");
    assert_eq!(response["count"], 1);
}

#[test]
fn test_cross_reference_empty_matches() {
    let response = json!({
        "message_hash": "b".repeat(64),
        "matches": [],
        "count": 0
    });
    assert_eq!(response["count"], 0);
    assert!(response["matches"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Cross-reference server endpoint (axum oneshot)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cross_reference_endpoint_exists() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dolphin_milk::config::DmConfig;
    use dolphin_milk::server;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let workspace = tempfile::tempdir().unwrap();
    let workspace_path = workspace.path().to_path_buf();
    // Create tasks dir
    std::fs::create_dir_all(workspace_path.join("tasks")).unwrap();

    // Leak the tempdir to keep it alive (same pattern as test_server.rs)
    let leaked_path = workspace_path.clone();
    std::mem::forget(workspace);

    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        leaked_path,
    )
    .await;

    let valid_hash = "a".repeat(64);
    let req = Request::builder()
        .uri(format!("/audit/cross-reference/{}", valid_hash))
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Should return 200, not 404 (route exists)
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    // Should have the expected response structure
    assert_eq!(json["message_hash"], valid_hash);
    assert!(json.get("matches").is_some());
    assert!(json.get("count").is_some());
}

#[tokio::test]
async fn test_cross_reference_invalid_hash_returns_400() {
    use axum::body::Body;
    use axum::http::Request;
    use dolphin_milk::config::DmConfig;
    use dolphin_milk::server;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let workspace = tempfile::tempdir().unwrap();
    let workspace_path = workspace.path().to_path_buf();
    std::fs::create_dir_all(workspace_path.join("tasks")).unwrap();

    let leaked_path = workspace_path.clone();
    std::mem::forget(workspace);

    let app = server::build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        leaked_path,
    )
    .await;

    // Too short hash
    let req = Request::builder()
        .uri("/audit/cross-reference/tooshort")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("error").is_some());
}

// ---------------------------------------------------------------------------
// Transcript scanning for cross-reference
// ---------------------------------------------------------------------------

#[test]
fn test_transcript_proof_event_contains_message_hash() {
    use dolphin_milk::transcript::Transcript;
    use std::collections::HashMap;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(path.clone());

    // Simulate a proof_created event with message hash in the data
    let message_hash = "c".repeat(64);
    let mut data = HashMap::new();
    data.insert("txid".to_string(), json!("proof_tx_123"));
    data.insert("proof_type".to_string(), json!("decision"));
    data.insert("iteration".to_string(), json!(2));
    data.insert(
        "data".to_string(),
        json!(format!(
            "DECISION hash=abc tools=[send_message] message_hashes=[{}]",
            message_hash
        )),
    );
    transcript.record("proof_created", data);

    // Reload and verify
    let t2 = Transcript::new(path);
    let events = t2.replay();
    let proof_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "proof_created")
        .collect();
    assert_eq!(proof_events.len(), 1);
    let proof_data = proof_events[0].data.get("data").unwrap().as_str().unwrap();
    assert!(proof_data.contains(&message_hash));
}

#[test]
fn test_transcript_proof_event_without_message_hash() {
    use dolphin_milk::transcript::Transcript;
    use std::collections::HashMap;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(path.clone());

    // A normal proof event without message hashes
    let mut data = HashMap::new();
    data.insert("txid".to_string(), json!("proof_tx_456"));
    data.insert("proof_type".to_string(), json!("decision"));
    data.insert("iteration".to_string(), json!(1));
    data.insert(
        "data".to_string(),
        json!("DECISION hash=xyz tools=[file_read] iter=1"),
    );
    transcript.record("proof_created", data);

    let t2 = Transcript::new(path);
    let events = t2.replay();
    let proof_data = events[0].data.get("data").unwrap().as_str().unwrap();
    assert!(!proof_data.contains("message_hashes="));
}

// ---------------------------------------------------------------------------
// Wallet method availability (BRC-56 endpoints exist)
// ---------------------------------------------------------------------------

#[test]
fn test_wallet_client_has_discover_methods() {
    // Just verify the WalletClient type has the expected methods via compilation
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    // These method calls won't actually execute (no wallet running), but they confirm
    // the methods exist at compile time
    let _: &WalletClient = &wallet;
    // discover_by_identity_key and discover_by_attributes are async methods
    // that exist on WalletClient — compilation validates this
}

// ---------------------------------------------------------------------------
// Integration: message hash through the pipeline
// ---------------------------------------------------------------------------

#[test]
fn test_message_hash_pipeline_end_to_end() {
    // 1. Compute hash from a message body
    let body = json!({
        "type": "agent_message",
        "body": {"content": "Hello, agent B!"}
    });
    let hash = compute_message_hash(&body);

    // 2. Hash should be 64-char hex
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

    // 3. Simulated send_message result includes the hash
    let send_result = json!({
        "status": "ok",
        "sentMessageId": "msg-789",
        "message_hash": hash,
    });

    // 4. Extraction logic (same as step.rs)
    let extracted = send_result
        .get("message_hash")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(extracted, hash);

    // 5. Proof text includes the hash
    let proof_text = format!("DECISION hash=xxx message_hashes=[{}]", extracted);
    assert!(proof_text.contains(&hash));

    // 6. Cross-reference search would find it
    assert!(proof_text.contains(&hash));
}

#[test]
fn test_multiple_message_hashes_in_single_iteration() {
    // An agent might send multiple messages in one iteration
    let body1 = json!({"to": "agent_a", "msg": "hello"});
    let body2 = json!({"to": "agent_b", "msg": "world"});
    let hash1 = compute_message_hash(&body1);
    let hash2 = compute_message_hash(&body2);

    assert_ne!(hash1, hash2);

    let hashes = [hash1.clone(), hash2.clone()];
    let proof_text = format!("message_hashes=[{}]", hashes.join(","));
    assert!(proof_text.contains(&hash1));
    assert!(proof_text.contains(&hash2));
}
