//! Tests for BRC-60 hash chain verification at task start (#197).
//!
//! Validates the `ConversationBreak` proof type and `chain_break_proof()`
//! convenience function added for R.3.

use dolphin_milk::proofs::{self, ProofCommitment, ProofType};

// -------------------------------------------------------------------------
// ProofType::ConversationBreak
// -------------------------------------------------------------------------

#[test]
fn test_chain_break_proof_type_display() {
    assert_eq!(
        ProofType::ConversationBreak.to_string(),
        "conversation_break"
    );
}

#[test]
fn test_conversation_break_serde() {
    let json = serde_json::to_string(&ProofType::ConversationBreak).unwrap();
    assert_eq!(json, r#""ConversationBreak""#);
    let back: ProofType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProofType::ConversationBreak);
}

// -------------------------------------------------------------------------
// chain_break_proof()
// -------------------------------------------------------------------------

#[test]
fn test_chain_break_proof_creation() {
    let commitment =
        proofs::chain_break_proof("conv-abc123", "expected_aabbcc", "actual_112233", 5, None);

    assert_eq!(commitment.proof_type, ProofType::ConversationBreak);
    assert!(commitment
        .data
        .contains("CHAIN_BREAK: conversation conv-abc123"));
    assert!(commitment.data.contains("EXPECTED_HASH: expected_aabbcc"));
    assert!(commitment.data.contains("ACTUAL_HASH: actual_112233"));
    assert!(commitment.data.contains("BREAK_POINT: message 5"));
    assert!(commitment.prev_hash.is_none());
    assert!(commitment.verify());
}

#[test]
fn test_chain_break_proof_chain() {
    // First proof — no previous hash
    let first = proofs::chain_break_proof("conv-first", "expected_1", "actual_1", 0, None);
    assert!(first.prev_hash.is_none());
    assert!(first.verify());

    // Second proof — chains to first
    let first_hash = first.hash_hex();
    let second = proofs::chain_break_proof(
        "conv-second",
        "expected_2",
        "actual_2",
        3,
        Some(&first_hash),
    );
    assert_eq!(second.prev_hash.as_deref(), Some(first_hash.as_str()));
    assert!(second.verify());

    // Hashes differ
    assert_ne!(first.hash, second.hash);
}

#[test]
fn test_chain_break_proof_data_format() {
    let commitment = proofs::chain_break_proof("conv-xyz", "aabbccdd", "11223344", 10, None);

    // Verify the exact format of the data field
    let expected_data = "CHAIN_BREAK: conversation conv-xyz\n\
                         EXPECTED_HASH: aabbccdd\n\
                         ACTUAL_HASH: 11223344\n\
                         BREAK_POINT: message 10";
    assert_eq!(commitment.data, expected_data);
}

#[test]
fn test_chain_break_proof_with_timestamp_deterministic() {
    // Using with_timestamp for deterministic hash verification
    let c1 = ProofCommitment::with_timestamp(
        ProofType::ConversationBreak,
        "CHAIN_BREAK: test",
        "2026-03-25T12:00:00Z",
        None,
    );
    let c2 = ProofCommitment::with_timestamp(
        ProofType::ConversationBreak,
        "CHAIN_BREAK: test",
        "2026-03-25T12:00:00Z",
        None,
    );
    assert_eq!(c1.hash, c2.hash);
    assert!(c1.verify());
    assert!(c2.verify());
}

#[test]
fn test_chain_break_proof_hash_hex() {
    let commitment = proofs::chain_break_proof("conv-test", "expected", "actual", 1, None);
    let hex = commitment.hash_hex();
    assert_eq!(hex.len(), 64); // 32 bytes = 64 hex chars
    assert!(hex::decode(&hex).is_ok());
}
