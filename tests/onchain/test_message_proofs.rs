//! Tests for BRC-18 message delivery proofs (Issue #200).
//!
//! Verifies:
//!   - ProofType::MessageSend and MessageReceive display and serde
//!   - message_send_proof() convenience function data format
//!   - message_receive_proof() convenience function data format
//!   - Proof chain linking with prev_hash
//!   - All expected fields present in data strings

use dolphin_milk::proofs::{message_receive_proof, message_send_proof, ProofCommitment, ProofType};

// ─────────────────────────────────────────────
// ProofType::MessageSend — Display
// ─────────────────────────────────────────────

#[test]
fn test_message_send_proof_type_display() {
    assert_eq!(ProofType::MessageSend.to_string(), "message_send");
}

// ─────────────────────────────────────────────
// ProofType::MessageReceive — Display
// ─────────────────────────────────────────────

#[test]
fn test_message_receive_proof_type_display() {
    assert_eq!(ProofType::MessageReceive.to_string(), "message_receive");
}

// ─────────────────────────────────────────────
// ProofType::MessageSend — Serde roundtrip
// ─────────────────────────────────────────────

#[test]
fn test_message_proof_type_serde() {
    // MessageSend
    let json = serde_json::to_string(&ProofType::MessageSend).unwrap();
    assert_eq!(json, r#""MessageSend""#);
    let back: ProofType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProofType::MessageSend);

    // MessageReceive
    let json = serde_json::to_string(&ProofType::MessageReceive).unwrap();
    assert_eq!(json, r#""MessageReceive""#);
    let back: ProofType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProofType::MessageReceive);
}

// ─────────────────────────────────────────────
// message_send_proof() — creation and verification
// ─────────────────────────────────────────────

#[test]
fn test_message_send_proof_creation() {
    let commitment = message_send_proof(
        "abc123def456",
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "status_inbox",
        350,
        true,
        false,
        None,
    );

    assert_eq!(commitment.proof_type, ProofType::MessageSend);
    assert!(commitment.verify());
    assert!(commitment.prev_hash.is_none());
}

// ─────────────────────────────────────────────
// message_receive_proof() — creation and verification
// ─────────────────────────────────────────────

#[test]
fn test_message_receive_proof_creation() {
    let commitment = message_receive_proof(
        "deadbeef01234567",
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "task_inbox",
        None,
    );

    assert_eq!(commitment.proof_type, ProofType::MessageReceive);
    assert!(commitment.verify());
    assert!(commitment.prev_hash.is_none());
}

// ─────────────────────────────────────────────
// message_send_proof() — all fields present
// ─────────────────────────────────────────────

#[test]
fn test_message_send_proof_fields() {
    let commitment = message_send_proof(
        "hash123",
        "recipient_key_abc",
        "results_inbox",
        500,
        false,
        true,
        None,
    );

    assert!(commitment.data.contains("MESSAGE_SEND: hash123"));
    assert!(commitment.data.contains("RECIPIENT: recipient_key_abc"));
    assert!(commitment.data.contains("BOX: results_inbox"));
    assert!(commitment.data.contains("DELIVERY_COST: 500"));
    assert!(commitment.data.contains("SIGNED: false"));
    assert!(commitment.data.contains("ENCRYPTED: true"));
}

// ─────────────────────────────────────────────
// message_receive_proof() — all fields present
// ─────────────────────────────────────────────

#[test]
fn test_message_receive_proof_fields() {
    let commitment = message_receive_proof(
        "recv_hash_456",
        "sender_key_xyz",
        "dolphin_milk_coordination",
        None,
    );

    assert!(commitment.data.contains("MESSAGE_RECEIVE: recv_hash_456"));
    assert!(commitment.data.contains("SENDER: sender_key_xyz"));
    assert!(commitment.data.contains("BOX: dolphin_milk_coordination"));
}

// ─────────────────────────────────────────────
// message_send_proof() — chain linking
// ─────────────────────────────────────────────

#[test]
fn test_message_send_proof_chain() {
    let c1 = message_send_proof(
        "hash_first",
        "recipient_a",
        "status_inbox",
        100,
        false,
        false,
        None,
    );

    let c2 = message_send_proof(
        "hash_second",
        "recipient_b",
        "task_inbox",
        200,
        true,
        true,
        Some(&c1.hash_hex()),
    );

    assert!(c1.verify());
    assert!(c2.verify());
    assert!(c1.prev_hash.is_none());
    assert_eq!(c2.prev_hash.as_deref(), Some(c1.hash_hex().as_str()));

    // Different prev_hash produces different commitment hash
    let c2_no_chain = message_send_proof(
        "hash_second",
        "recipient_b",
        "task_inbox",
        200,
        true,
        true,
        None,
    );
    // Cannot compare hashes directly since timestamps differ, but we can verify
    // that the prev_hash field is correctly set
    assert!(c2_no_chain.prev_hash.is_none());
}

// ─────────────────────────────────────────────
// message_receive_proof() — chain linking
// ─────────────────────────────────────────────

#[test]
fn test_message_receive_proof_chain() {
    let c1 = message_receive_proof("recv_first", "sender_1", "task_inbox", None);

    let c2 = message_receive_proof(
        "recv_second",
        "sender_2",
        "status_inbox",
        Some(&c1.hash_hex()),
    );

    assert!(c1.verify());
    assert!(c2.verify());
    assert!(c1.prev_hash.is_none());
    assert_eq!(c2.prev_hash.as_deref(), Some(c1.hash_hex().as_str()));
}

// ─────────────────────────────────────────────
// ProofCommitment serialization roundtrip with MessageSend/Receive
// ─────────────────────────────────────────────

#[test]
fn test_message_send_proof_commitment_serde() {
    let c = ProofCommitment::with_timestamp(
        ProofType::MessageSend,
        "MESSAGE_SEND: abc\nRECIPIENT: xyz",
        "2026-03-25T12:00:00Z",
        None,
    );
    let json = serde_json::to_string(&c).unwrap();
    let back: ProofCommitment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.proof_type, ProofType::MessageSend);
    assert_eq!(back.data, "MESSAGE_SEND: abc\nRECIPIENT: xyz");
    assert_eq!(back.timestamp, "2026-03-25T12:00:00Z");
    assert_eq!(back.hash, c.hash);
    assert!(back.verify());
}

#[test]
fn test_message_receive_proof_commitment_serde() {
    let c = ProofCommitment::with_timestamp(
        ProofType::MessageReceive,
        "MESSAGE_RECEIVE: def\nSENDER: uvw",
        "2026-03-25T13:00:00Z",
        None,
    );
    let json = serde_json::to_string(&c).unwrap();
    let back: ProofCommitment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.proof_type, ProofType::MessageReceive);
    assert_eq!(back.data, "MESSAGE_RECEIVE: def\nSENDER: uvw");
    assert!(back.verify());
}
