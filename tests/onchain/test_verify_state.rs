//! Tests for R.1: verify_my_state — reading proof chain from blockchain
//! and verifying in-memory state consistency.
//!
//! Tests cover OP_RETURN hash parsing, verification result construction,
//! divergence detection, serde roundtrips, and mock wallet list_outputs.

use dolphin_milk::proofs::{
    build_op_return_script, parse_op_return_hash, verify_state_against_proofs, OnChainProof,
    StateDivergence, VerificationResult,
};

// ─────────────────────────────────────────────
// parse_op_return_hash
// ─────────────────────────────────────────────

#[test]
fn test_parse_op_return_hash_known_value() {
    // Build a known OP_RETURN script and verify parsing extracts the hash.
    let hash: [u8; 32] = [
        0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67,
        0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45,
        0x67, 0x89,
    ];
    let script = build_op_return_script(&hash);
    let parsed = parse_op_return_hash(&script);
    assert_eq!(parsed, Some(hex::encode(hash)));
}

#[test]
fn test_parse_op_return_hash_all_zeros() {
    let hash = [0u8; 32];
    let script = build_op_return_script(&hash);
    let parsed = parse_op_return_hash(&script);
    assert_eq!(parsed, Some("0".repeat(64)));
}

#[test]
fn test_parse_op_return_hash_rejects_short_script() {
    // A script that's too short should return None.
    assert_eq!(parse_op_return_hash("006a20"), None);
    assert_eq!(parse_op_return_hash("006a"), None);
    assert_eq!(parse_op_return_hash(""), None);
}

#[test]
fn test_parse_op_return_hash_rejects_wrong_prefix() {
    // Valid length but wrong opcodes.
    let mut bytes = vec![0x00, 0x6a, 0x20];
    bytes.extend_from_slice(&[0xAA; 32]);
    let hex_str = hex::encode(&bytes);
    // This should work (correct prefix)
    assert!(parse_op_return_hash(&hex_str).is_some());

    // Wrong first byte
    let mut bad = bytes.clone();
    bad[0] = 0x01;
    assert_eq!(parse_op_return_hash(&hex::encode(&bad)), None);

    // Wrong second byte
    let mut bad = bytes.clone();
    bad[1] = 0x00;
    assert_eq!(parse_op_return_hash(&hex::encode(&bad)), None);

    // Wrong third byte
    let mut bad = bytes;
    bad[2] = 0x21;
    assert_eq!(parse_op_return_hash(&hex::encode(&bad)), None);
}

#[test]
fn test_parse_op_return_hash_rejects_too_long() {
    // 36 bytes after prefix (should be 32)
    let mut bytes = vec![0x00, 0x6a, 0x20];
    bytes.extend_from_slice(&[0xBB; 33]);
    assert_eq!(parse_op_return_hash(&hex::encode(&bytes)), None);
}

#[test]
fn test_parse_op_return_hash_rejects_invalid_hex() {
    assert_eq!(parse_op_return_hash("not_hex"), None);
    assert_eq!(parse_op_return_hash("zzzz"), None);
}

// ─────────────────────────────────────────────
// verify_state_against_proofs — consistent
// ─────────────────────────────────────────────

#[test]
fn test_verify_state_consistent() {
    let proofs = vec![
        OnChainProof {
            txid: "tx1".to_string(),
            hash: "aabbccdd".to_string(),
        },
        OnChainProof {
            txid: "tx2".to_string(),
            hash: "11223344".to_string(),
        },
    ];
    let result = verify_state_against_proofs(
        &proofs,
        2,                // iteration
        1000,             // sats_spent
        Some("aabbccdd"), // matches first proof hash
    );
    assert!(result.consistent);
    assert_eq!(result.chain_length, 2);
    assert!(result.divergences.is_empty());
    assert_eq!(result.last_proof_hash, Some("aabbccdd".to_string()));
}

#[test]
fn test_verify_state_consistent_no_hash_tracking() {
    // When in-memory last_proof_hash is None, no hash divergence is reported.
    let proofs = vec![OnChainProof {
        txid: "tx1".to_string(),
        hash: "aabbccdd".to_string(),
    }];
    let result = verify_state_against_proofs(&proofs, 1, 500, None);
    assert!(result.consistent);
    assert_eq!(result.chain_length, 1);
}

// ─────────────────────────────────────────────
// verify_state_against_proofs — divergences
// ─────────────────────────────────────────────

#[test]
fn test_verify_state_iteration_divergence() {
    // Empty chain but iteration > 0 means proofs are missing.
    let result = verify_state_against_proofs(&[], 5, 2000, None);
    assert!(!result.consistent);
    assert_eq!(result.chain_length, 0);
    assert_eq!(result.divergences.len(), 1);
    assert_eq!(result.divergences[0].field, "chain_length");
    assert_eq!(result.divergences[0].in_memory, "5");
    assert_eq!(result.divergences[0].on_chain, "0");
}

#[test]
fn test_verify_state_hash_divergence() {
    let proofs = vec![OnChainProof {
        txid: "tx1".to_string(),
        hash: "on_chain_hash".to_string(),
    }];
    let result = verify_state_against_proofs(&proofs, 1, 500, Some("in_memory_hash"));
    assert!(!result.consistent);
    assert_eq!(result.divergences.len(), 1);
    assert_eq!(result.divergences[0].field, "last_proof_hash");
    assert_eq!(result.divergences[0].in_memory, "in_memory_hash");
    assert_eq!(result.divergences[0].on_chain, "on_chain_hash");
}

#[test]
fn test_verify_state_multiple_divergences() {
    // Empty chain + mismatched hash = 2 divergences... but actually if chain is empty,
    // there's no chain hash to compare. So we only get the chain_length divergence.
    let result = verify_state_against_proofs(&[], 3, 1000, Some("abc123"));
    assert!(!result.consistent);
    // chain_length divergence only — no on-chain hash to compare against
    assert_eq!(result.divergences.len(), 1);
    assert_eq!(result.divergences[0].field, "chain_length");
}

// ─────────────────────────────────────────────
// verify_state_against_proofs — empty chain
// ─────────────────────────────────────────────

#[test]
fn test_verify_state_empty_chain_iteration_zero() {
    // Empty chain at iteration 0 is consistent (task hasn't started proofs yet).
    let result = verify_state_against_proofs(&[], 0, 0, None);
    assert!(result.consistent);
    assert_eq!(result.chain_length, 0);
    assert!(result.divergences.is_empty());
    assert!(result.last_proof_hash.is_none());
}

// ─────────────────────────────────────────────
// Serde roundtrips
// ─────────────────────────────────────────────

#[test]
fn test_verification_result_serde() {
    let result = VerificationResult {
        consistent: false,
        chain_length: 42,
        divergences: vec![StateDivergence {
            field: "chain_length".to_string(),
            in_memory: "10".to_string(),
            on_chain: "42".to_string(),
        }],
        last_proof_hash: Some("deadbeef".to_string()),
    };
    let json = serde_json::to_string(&result).unwrap();
    let back: VerificationResult = serde_json::from_str(&json).unwrap();
    assert!(!back.consistent);
    assert_eq!(back.chain_length, 42);
    assert_eq!(back.divergences.len(), 1);
    assert_eq!(back.divergences[0].field, "chain_length");
    assert_eq!(back.last_proof_hash, Some("deadbeef".to_string()));
}

#[test]
fn test_on_chain_proof_serde() {
    let proof = OnChainProof {
        txid: "abc123def456".to_string(),
        hash: "deadbeefcafebabe".to_string(),
    };
    let json = serde_json::to_string(&proof).unwrap();
    assert!(json.contains("abc123def456"));
    assert!(json.contains("deadbeefcafebabe"));
    let back: OnChainProof = serde_json::from_str(&json).unwrap();
    assert_eq!(back.txid, "abc123def456");
    assert_eq!(back.hash, "deadbeefcafebabe");
}

#[test]
fn test_state_divergence_serde() {
    let div = StateDivergence {
        field: "test_field".to_string(),
        in_memory: "mem_val".to_string(),
        on_chain: "chain_val".to_string(),
    };
    let json = serde_json::to_string(&div).unwrap();
    let back: StateDivergence = serde_json::from_str(&json).unwrap();
    assert_eq!(back.field, "test_field");
    assert_eq!(back.in_memory, "mem_val");
    assert_eq!(back.on_chain, "chain_val");
}

// ─────────────────────────────────────────────
// Mock wallet — read_proof_chain via mockito
// ─────────────────────────────────────────────

#[tokio::test]
async fn test_read_proof_chain_mock() {
    use dolphin_milk::proofs::read_proof_chain;
    use dolphin_milk::wallet::WalletClient;

    let mut server = mockito::Server::new_async().await;
    let server_url = server.url();

    // Build two known OP_RETURN scripts
    let hash1 = [0xAAu8; 32];
    let hash2 = [0xBBu8; 32];
    let script1 = build_op_return_script(&hash1);
    let script2 = build_op_return_script(&hash2);

    let response_body = serde_json::json!({
        "outputs": [
            {
                "lockingScript": script1,
                "outpoint": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890.0",
                "satoshis": 0,
            },
            {
                "lockingScript": script2,
                "outpoint": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef.0",
                "satoshis": 0,
            },
        ]
    });

    let _mock = server
        .mock("POST", "/listOutputs")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let wallet = WalletClient::new(&server_url, "http://localhost", 30);
    let proofs = read_proof_chain(&wallet, 10).await.unwrap();

    assert_eq!(proofs.len(), 2);
    assert_eq!(proofs[0].hash, hex::encode(hash1));
    assert_eq!(
        proofs[0].txid,
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
    );
    assert_eq!(proofs[1].hash, hex::encode(hash2));
    assert_eq!(
        proofs[1].txid,
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
    );
}

#[tokio::test]
async fn test_read_proof_chain_empty_basket() {
    use dolphin_milk::proofs::read_proof_chain;
    use dolphin_milk::wallet::WalletClient;

    let mut server = mockito::Server::new_async().await;
    let server_url = server.url();

    let response_body = serde_json::json!({
        "outputs": []
    });

    let _mock = server
        .mock("POST", "/listOutputs")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let wallet = WalletClient::new(&server_url, "http://localhost", 30);
    let proofs = read_proof_chain(&wallet, 10).await.unwrap();

    assert!(proofs.is_empty());
}

#[tokio::test]
async fn test_read_proof_chain_respects_limit() {
    use dolphin_milk::proofs::read_proof_chain;
    use dolphin_milk::wallet::WalletClient;

    let mut server = mockito::Server::new_async().await;
    let server_url = server.url();

    // Return 5 proofs but request limit of 2
    let mut outputs = Vec::new();
    for i in 0..5u8 {
        let hash = [i + 1; 32];
        let script = build_op_return_script(&hash);
        outputs.push(serde_json::json!({
            "lockingScript": script,
            "outpoint": format!("{}.0", hex::encode([i + 1; 32])),
            "satoshis": 0,
        }));
    }

    let response_body = serde_json::json!({ "outputs": outputs });

    let _mock = server
        .mock("POST", "/listOutputs")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let wallet = WalletClient::new(&server_url, "http://localhost", 30);
    let proofs = read_proof_chain(&wallet, 2).await.unwrap();

    assert_eq!(proofs.len(), 2);
}

#[tokio::test]
async fn test_read_proof_chain_skips_non_op_return() {
    use dolphin_milk::proofs::read_proof_chain;
    use dolphin_milk::wallet::WalletClient;

    let mut server = mockito::Server::new_async().await;
    let server_url = server.url();

    let hash = [0xCCu8; 32];
    let valid_script = build_op_return_script(&hash);

    let response_body = serde_json::json!({
        "outputs": [
            {
                // Valid OP_RETURN
                "lockingScript": valid_script,
                "outpoint": "aabb.0",
                "satoshis": 0,
            },
            {
                // Not an OP_RETURN (PushDrop or other script)
                "lockingScript": "76a914aabbccdd88ac",
                "outpoint": "ccdd.0",
                "satoshis": 1,
            },
        ]
    });

    let _mock = server
        .mock("POST", "/listOutputs")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let wallet = WalletClient::new(&server_url, "http://localhost", 30);
    let proofs = read_proof_chain(&wallet, 10).await.unwrap();

    // Only the valid OP_RETURN should be parsed
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0].hash, hex::encode(hash));
}

// ─────────────────────────────────────────────
// Config: verify_interval
// ─────────────────────────────────────────────

#[test]
fn test_lifecycle_config_verify_interval_default() {
    let config = dolphin_milk::config::LifecycleConfig::default();
    assert_eq!(config.verify_interval, 10);
}

#[test]
fn test_lifecycle_config_verify_interval_toml() {
    let toml_str = r#"
    [lifecycle]
    verify_interval = 5
    "#;
    let config: dolphin_milk::config::DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.lifecycle.verify_interval, 5);
}

#[test]
fn test_lifecycle_config_verify_interval_zero_disables() {
    let toml_str = r#"
    [lifecycle]
    verify_interval = 0
    "#;
    let config: dolphin_milk::config::DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.lifecycle.verify_interval, 0);
}
