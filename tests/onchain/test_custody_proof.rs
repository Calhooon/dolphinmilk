//! Tests for custody proof (Issue #205).
//!
//! Verifies:
//!   - ProofType::Custody display and serde
//!   - custody_proof() convenience function data format
//!   - Proof chain linking with prev_hash
//!   - Self vs external customer key
//!   - Certificate serial presence/absence
//!   - All expected fields in proof data

use dolphin_milk::proofs::{custody_proof, ProofCommitment, ProofType};

// ─────────────────────────────────────────────
// ProofType::Custody — Display
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_type_display() {
    assert_eq!(ProofType::Custody.to_string(), "custody");
}

// ─────────────────────────────────────────────
// ProofType::Custody — Serde roundtrip
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_serde() {
    let json = serde_json::to_string(&ProofType::Custody).unwrap();
    assert_eq!(json, r#""Custody""#);
    let back: ProofType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProofType::Custody);
}

// ─────────────────────────────────────────────
// custody_proof() — basic creation
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_creation() {
    let tools = vec!["memory_search".to_string(), "file_read".to_string()];
    let commitment = custody_proof(
        Some("02abc123"),
        "aabbccdd",
        5,
        120,
        &tools,
        15000,
        "deadbeef",
        Some("prev1234"),
        3,
        "02agentkey",
        Some("cert-serial-001"),
        None,
    );

    assert_eq!(commitment.proof_type, ProofType::Custody);
    assert!(commitment.data.contains("CUSTODY_PROOF: v1"));
    assert!(commitment.data.contains("CUSTOMER_KEY: 02abc123"));
    assert!(commitment.data.contains("TASK_HASH: sha256:aabbccdd"));
    assert!(commitment.data.contains("ITERATIONS: 5"));
    assert!(commitment.data.contains("DURATION_SECS: 120"));
    assert!(commitment
        .data
        .contains("TOOLS_USED: memory_search, file_read"));
    assert!(commitment.data.contains("TOTAL_SATS: 15000"));
    assert!(commitment.data.contains("RESULT_HASH: sha256:deadbeef"));
    assert!(commitment.data.contains("PROOF_CHAIN_HEAD: prev1234"));
    assert!(commitment.data.contains("PROOF_CHAIN_LENGTH: 3"));
    assert!(commitment.data.contains("AGENT_KEY: 02agentkey"));
    assert!(commitment.data.contains("AGENT_CERT: cert-serial-001"));
    assert!(commitment.verify());
}

// ─────────────────────────────────────────────
// Self customer (None caller_key)
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_self_customer() {
    let commitment = custody_proof(
        None,
        "aabb",
        1,
        10,
        &[],
        500,
        "ccdd",
        None,
        0,
        "02agent",
        None,
        None,
    );

    assert!(commitment.data.contains("CUSTOMER_KEY: self"));
    assert!(commitment.verify());
}

// ─────────────────────────────────────────────
// External customer (Some caller_key)
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_external_customer() {
    let commitment = custody_proof(
        Some("02external_pubkey"),
        "aabb",
        1,
        10,
        &[],
        500,
        "ccdd",
        None,
        0,
        "02agent",
        None,
        None,
    );

    assert!(commitment.data.contains("CUSTOMER_KEY: 02external_pubkey"));
    assert!(!commitment.data.contains("CUSTOMER_KEY: self"));
    assert!(commitment.verify());
}

// ─────────────────────────────────────────────
// Chain linking with prev_hash
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_chain() {
    let c1 = custody_proof(
        None,
        "task1",
        3,
        60,
        &[],
        1000,
        "res1",
        None,
        0,
        "02agent",
        None,
        None,
    );

    let c2 = custody_proof(
        None,
        "task2",
        5,
        120,
        &[],
        2000,
        "res2",
        Some(&c1.hash_hex()),
        1,
        "02agent",
        None,
        Some(&c1.hash_hex()),
    );

    assert!(c2.prev_hash.is_some());
    assert_eq!(c2.prev_hash.as_deref(), Some(c1.hash_hex().as_str()));

    // Same data but without prev_hash should produce different hash
    let c2_no_chain = custody_proof(
        None,
        "task2",
        5,
        120,
        &[],
        2000,
        "res2",
        Some(&c1.hash_hex()),
        1,
        "02agent",
        None,
        None,
    );
    assert_ne!(c2.hash, c2_no_chain.hash);
}

// ─────────────────────────────────────────────
// Certificate serial present
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_with_cert() {
    let commitment = custody_proof(
        None,
        "task1",
        1,
        10,
        &[],
        100,
        "res1",
        None,
        0,
        "02agent",
        Some("cert-serial-XYZ"),
        None,
    );

    assert!(commitment.data.contains("AGENT_CERT: cert-serial-XYZ"));
    assert!(!commitment.data.contains("AGENT_CERT: none"));
    assert!(commitment.verify());
}

// ─────────────────────────────────────────────
// Certificate serial absent
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_no_cert() {
    let commitment = custody_proof(
        None,
        "task1",
        1,
        10,
        &[],
        100,
        "res1",
        None,
        0,
        "02agent",
        None,
        None,
    );

    assert!(commitment.data.contains("AGENT_CERT: none"));
    assert!(commitment.verify());
}

// ─────────────────────────────────────────────
// All expected field names present
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_data_contains_all_fields() {
    let tools = vec!["think".to_string()];
    let commitment = custody_proof(
        Some("02customer"),
        "taskhash123",
        10,
        300,
        &tools,
        50000,
        "resulthash456",
        Some("chainhead789"),
        5,
        "02agentkey",
        Some("cert-001"),
        None,
    );

    let expected_fields = [
        "CUSTODY_PROOF:",
        "CUSTOMER_KEY:",
        "TASK_HASH:",
        "ITERATIONS:",
        "DURATION_SECS:",
        "TOOLS_USED:",
        "TOTAL_SATS:",
        "RESULT_HASH:",
        "PROOF_CHAIN_HEAD:",
        "PROOF_CHAIN_LENGTH:",
        "AGENT_KEY:",
        "AGENT_CERT:",
    ];

    for field in &expected_fields {
        assert!(
            commitment.data.contains(field),
            "Missing field '{}' in proof data:\n{}",
            field,
            commitment.data
        );
    }
}

// ─────────────────────────────────────────────
// Proof is verifiable (hash matches)
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_verify() {
    let commitment = custody_proof(
        Some("02key"),
        "hash1",
        3,
        60,
        &["tool1".to_string()],
        5000,
        "reshash",
        None,
        0,
        "02agent",
        None,
        None,
    );

    assert!(commitment.verify());

    // Tamper with data — should fail verification
    let mut tampered = commitment.clone();
    tampered.data = "tampered data".to_string();
    assert!(!tampered.verify());
}

// ─────────────────────────────────────────────
// Deterministic: same inputs + timestamp → same hash
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_deterministic() {
    let _tools = ["tool_a".to_string()];
    let data = "CUSTODY_PROOF: v1\nCUSTOMER_KEY: self\nTASK_HASH: sha256:abc\n\
         ITERATIONS: 2\nDURATION_SECS: 30\n\
         TOOLS_USED: tool_a\nTOTAL_SATS: 1000\n\
         RESULT_HASH: sha256:def\n\
         PROOF_CHAIN_HEAD: none\nPROOF_CHAIN_LENGTH: 0\n\
         AGENT_KEY: 02agent\nAGENT_CERT: none"
        .to_string();

    let c1 =
        ProofCommitment::with_timestamp(ProofType::Custody, &data, "2026-03-25T12:00:00Z", None);
    let c2 =
        ProofCommitment::with_timestamp(ProofType::Custody, &data, "2026-03-25T12:00:00Z", None);
    assert_eq!(c1.hash, c2.hash);
}

// ─────────────────────────────────────────────
// Empty tools list
// ─────────────────────────────────────────────

#[test]
fn test_custody_proof_empty_tools() {
    let commitment = custody_proof(
        None,
        "task1",
        0,
        0,
        &[],
        0,
        "empty",
        None,
        0,
        "02agent",
        None,
        None,
    );

    assert!(commitment.data.contains("TOOLS_USED: "));
    // Empty join produces empty string after the colon
    assert!(
        commitment.data.contains("TOOLS_USED: \n")
            || commitment.data.contains("TOOLS_USED: \nTOTAL_SATS")
    );
    assert!(commitment.verify());
}
