//! Integration tests for the BRC-18 proof trail (Phase 2d.7).
//!
//! Tests proof commitment construction, verification, OP_RETURN script
//! building, serialization roundtrips, tamper detection, and chain linking.

use dolphin_milk::proofs::{
    build_op_return_script, conversation_integrity_proof, decision_proof, memory_commitment_proof,
    task_completion_proof, ProofCommitment, ProofResult, ProofType,
};

// ─────────────────────────────────────────────
// ProofCommitment construction + verification
// ─────────────────────────────────────────────

#[test]
fn test_commitment_new_and_verify() {
    let c = ProofCommitment::new(ProofType::Decision, "use tantivy for search", None);
    assert!(c.verify());
    assert_eq!(c.proof_type, ProofType::Decision);
    assert!(!c.timestamp.is_empty());
    assert!(c.prev_hash.is_none());
}

#[test]
fn test_commitment_with_timestamp_deterministic() {
    let c1 = ProofCommitment::with_timestamp(
        ProofType::TaskCompletion,
        "data",
        "2026-02-23T12:00:00Z",
        None,
    );
    let c2 = ProofCommitment::with_timestamp(
        ProofType::TaskCompletion,
        "data",
        "2026-02-23T12:00:00Z",
        None,
    );
    assert_eq!(c1.hash, c2.hash);
    assert!(c1.verify());
    assert!(c2.verify());
}

#[test]
fn test_commitment_tamper_detection_data() {
    let mut c = ProofCommitment::new(ProofType::Decision, "original decision", None);
    assert!(c.verify());
    c.data = "altered decision".to_string();
    assert!(!c.verify(), "Tampered data should fail verification");
}

#[test]
fn test_commitment_tamper_detection_timestamp() {
    let mut c = ProofCommitment::new(ProofType::BudgetSnapshot, "balance: 50000", None);
    assert!(c.verify());
    c.timestamp = "2020-01-01T00:00:00Z".to_string();
    assert!(!c.verify(), "Tampered timestamp should fail verification");
}

#[test]
fn test_commitment_tamper_detection_hash() {
    let mut c = ProofCommitment::new(ProofType::CapabilityProof, "tool output", None);
    assert!(c.verify());
    c.hash[0] ^= 0xFF; // flip bits
    assert!(!c.verify(), "Tampered hash should fail verification");
}

// ─────────────────────────────────────────────
// OP_RETURN script building
// ─────────────────────────────────────────────

#[test]
fn test_op_return_script_structure() {
    let c = ProofCommitment::new(ProofType::Decision, "test", None);
    let script = build_op_return_script(&c.hash);
    let bytes = hex::decode(&script).expect("valid hex");

    assert_eq!(bytes[0], 0x00, "OP_FALSE");
    assert_eq!(bytes[1], 0x6a, "OP_RETURN");
    assert_eq!(bytes[2], 0x20, "PUSH 32 bytes");
    assert_eq!(&bytes[3..], &c.hash, "hash payload");
    assert_eq!(bytes.len(), 35);
}

#[test]
fn test_op_return_script_different_hashes() {
    let c1 = ProofCommitment::with_timestamp(ProofType::Decision, "A", "t1", None);
    let c2 = ProofCommitment::with_timestamp(ProofType::Decision, "B", "t2", None);
    let s1 = build_op_return_script(&c1.hash);
    let s2 = build_op_return_script(&c2.hash);
    assert_ne!(s1, s2, "Different proofs should produce different scripts");
    // But same prefix
    assert_eq!(&s1[..6], &s2[..6]);
}

// ─────────────────────────────────────────────
// Serialization roundtrips
// ─────────────────────────────────────────────

#[test]
fn test_commitment_json_roundtrip() {
    let c = ProofCommitment::with_timestamp(
        ProofType::MemoryCommitment,
        "5 entries, hash: abc123",
        "2026-02-23T14:00:00Z",
        None,
    );
    let json = serde_json::to_string_pretty(&c).unwrap();
    let back: ProofCommitment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.proof_type, c.proof_type);
    assert_eq!(back.data, c.data);
    assert_eq!(back.timestamp, c.timestamp);
    assert_eq!(back.hash, c.hash);
    assert!(back.verify());
}

#[test]
fn test_proof_result_json_roundtrip() {
    let c = ProofCommitment::with_timestamp(
        ProofType::TaskCompletion,
        "built proof system",
        "2026-02-23T15:00:00Z",
        None,
    );
    let result = ProofResult {
        txid: "deadbeef1234567890abcdef".to_string(),
        commitment: c,
    };
    let json = serde_json::to_string(&result).unwrap();
    let back: ProofResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.txid, "deadbeef1234567890abcdef");
    assert_eq!(back.commitment.proof_type, ProofType::TaskCompletion);
    assert!(back.commitment.verify());
}

#[test]
fn test_proof_type_all_variants_serialize() {
    let variants = [
        ProofType::Decision,
        ProofType::TaskCompletion,
        ProofType::BudgetSnapshot,
        ProofType::MemoryCommitment,
        ProofType::CapabilityProof,
        ProofType::ConversationIntegrity,
    ];
    for pt in &variants {
        let json = serde_json::to_string(pt).unwrap();
        let back: ProofType = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, pt);
    }
}

// ─────────────────────────────────────────────
// Convenience helper functions
// ─────────────────────────────────────────────

#[test]
fn test_decision_proof_helper() {
    let c = decision_proof("switch to Rust", "type safety and BSV SDK alignment", None);
    assert_eq!(c.proof_type, ProofType::Decision);
    assert!(c.data.contains("DECISION: switch to Rust"));
    assert!(c.data.contains("REASONING: type safety"));
    assert!(c.verify());
}

#[test]
fn test_task_completion_proof_helper() {
    let c = task_completion_proof("implement BRC-18", "18 tests passing", None);
    assert_eq!(c.proof_type, ProofType::TaskCompletion);
    assert!(c.data.contains("TASK: implement BRC-18"));
    assert!(c.data.contains("RESULT: 18 tests passing"));
    assert!(c.verify());
}

#[test]
fn test_memory_commitment_proof_helper() {
    let hashes = vec!["hash1".into(), "hash2".into(), "hash3".into()];
    let c = memory_commitment_proof(3, &hashes, None);
    assert_eq!(c.proof_type, ProofType::MemoryCommitment);
    assert!(c.data.contains("ENTRIES: 3"));
    assert!(c.data.contains("hash1,hash2,hash3"));
    assert!(c.verify());
}

#[test]
fn test_memory_commitment_empty() {
    let c = memory_commitment_proof(0, &[], None);
    assert!(c.data.contains("ENTRIES: 0"));
    assert!(c.data.contains("HASHES: "));
    assert!(c.verify());
}

// ─────────────────────────────────────────────
// hash_hex utility
// ─────────────────────────────────────────────

#[test]
fn test_hash_hex_length_and_validity() {
    let c = ProofCommitment::new(ProofType::Decision, "test", None);
    let hex_str = c.hash_hex();
    assert_eq!(hex_str.len(), 64, "SHA-256 = 32 bytes = 64 hex chars");
    assert!(hex::decode(&hex_str).is_ok());
}

#[test]
fn test_hash_hex_matches_raw_hash() {
    let c = ProofCommitment::with_timestamp(ProofType::Decision, "x", "t", None);
    assert_eq!(hex::decode(c.hash_hex()).unwrap(), c.hash.to_vec());
}

// ─────────────────────────────────────────────
// Proof chain linking (Phase 1)
// ─────────────────────────────────────────────

#[test]
fn test_proof_chain_linking() {
    // Build a chain of 3 proofs, each referencing the previous
    let c1 = ProofCommitment::with_timestamp(
        ProofType::Decision,
        "step 1",
        "2026-02-27T10:00:00Z",
        None,
    );
    assert!(c1.verify());
    assert!(c1.prev_hash.is_none());
    let h1 = c1.hash_hex();

    let c2 = ProofCommitment::with_timestamp(
        ProofType::Decision,
        "step 2",
        "2026-02-27T10:01:00Z",
        Some(&h1),
    );
    assert!(c2.verify());
    assert_eq!(c2.prev_hash.as_deref(), Some(h1.as_str()));
    let h2 = c2.hash_hex();

    let c3 = ProofCommitment::with_timestamp(
        ProofType::TaskCompletion,
        "step 3",
        "2026-02-27T10:02:00Z",
        Some(&h2),
    );
    assert!(c3.verify());
    assert_eq!(c3.prev_hash.as_deref(), Some(h2.as_str()));

    // Verify chain links are unique — same data+ts but different prev_hash → different hash
    let c2_no_chain = ProofCommitment::with_timestamp(
        ProofType::Decision,
        "step 2",
        "2026-02-27T10:01:00Z",
        None,
    );
    assert_ne!(
        c2.hash, c2_no_chain.hash,
        "prev_hash must affect the proof hash"
    );
}

#[test]
fn test_proof_chain_tamper_prev_hash() {
    let c1 =
        ProofCommitment::with_timestamp(ProofType::Decision, "first", "2026-02-27T10:00:00Z", None);
    let h1 = c1.hash_hex();

    let mut c2 = ProofCommitment::with_timestamp(
        ProofType::Decision,
        "second",
        "2026-02-27T10:01:00Z",
        Some(&h1),
    );
    assert!(c2.verify());

    // Corrupt the prev_hash
    c2.prev_hash =
        Some("0000000000000000000000000000000000000000000000000000000000000000".to_string());
    assert!(!c2.verify(), "Tampered prev_hash should fail verification");

    // Remove the prev_hash entirely
    c2.prev_hash = None;
    assert!(!c2.verify(), "Removed prev_hash should fail verification");
}

#[test]
fn test_content_hash_in_proof() {
    // Verify that SHA-256 hex content hashes round-trip correctly as proof data
    use sha2::{Digest, Sha256};
    let content = "This is a long response from the LLM that would normally be truncated...";
    let hash = hex::encode(Sha256::digest(content.as_bytes()));
    let proof_data = format!("sha256:{}", hash);

    let c = ProofCommitment::with_timestamp(
        ProofType::Decision,
        &format!(
            "DECISION: Iteration 1: response generated\nREASONING: {}",
            proof_data
        ),
        "2026-02-27T10:00:00Z",
        None,
    );
    assert!(c.verify());
    assert!(c.data.contains(&format!("sha256:{}", hash)));

    // Verify the content hash is deterministic
    let hash2 = hex::encode(Sha256::digest(content.as_bytes()));
    assert_eq!(hash, hash2);
}

#[test]
fn test_proof_chain_with_helpers() {
    // Verify that helper functions properly pass prev_hash
    let c1 = decision_proof("decide A", "reason A", None);
    assert!(c1.verify());
    assert!(c1.prev_hash.is_none());
    let h1 = c1.hash_hex();

    let c2 = task_completion_proof("task B", "result B", Some(&h1));
    assert!(c2.verify());
    assert_eq!(c2.prev_hash.as_deref(), Some(h1.as_str()));
    let h2 = c2.hash_hex();

    let c3 = memory_commitment_proof(2, &["x".into(), "y".into()], Some(&h2));
    assert!(c3.verify());
    assert_eq!(c3.prev_hash.as_deref(), Some(h2.as_str()));
}

#[test]
fn test_proof_chain_serialization_roundtrip() {
    // Ensure prev_hash survives JSON serialization
    let c1 = ProofCommitment::with_timestamp(
        ProofType::Decision,
        "chain test",
        "2026-02-27T10:00:00Z",
        None,
    );
    let h1 = c1.hash_hex();

    let c2 = ProofCommitment::with_timestamp(
        ProofType::BudgetSnapshot,
        "TASK_SATS: 1000",
        "2026-02-27T10:01:00Z",
        Some(&h1),
    );
    let json = serde_json::to_string(&c2).unwrap();
    let back: ProofCommitment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.prev_hash.as_deref(), Some(h1.as_str()));
    assert_eq!(back.hash, c2.hash);
    assert!(back.verify());

    // None prev_hash should not appear in JSON
    let json_no_chain = serde_json::to_string(&c1).unwrap();
    assert!(
        !json_no_chain.contains("prev_hash"),
        "None prev_hash should be omitted from JSON"
    );
}

// ─────────────────────────────────────────────
// Conversation integrity proof (Phase 2)
// ─────────────────────────────────────────────

#[test]
fn test_conversation_integrity_proof_helper() {
    let c = conversation_integrity_proof("conv-abc123", "deadbeef1234", 42, None);
    assert_eq!(c.proof_type, ProofType::ConversationIntegrity);
    assert!(c.data.contains("CONVERSATION: conv-abc123"));
    assert!(c.data.contains("HEAD_HASH: deadbeef1234"));
    assert!(c.data.contains("MESSAGES: 42"));
    assert!(c.verify());
    assert!(c.prev_hash.is_none());
}

#[test]
fn test_conversation_integrity_proof_with_chain() {
    let c1 = decision_proof("test", "reason", None);
    let h1 = c1.hash_hex();

    let c2 = conversation_integrity_proof("conv-xyz", "aabbccdd", 10, Some(&h1));
    assert_eq!(c2.proof_type, ProofType::ConversationIntegrity);
    assert_eq!(c2.prev_hash.as_deref(), Some(h1.as_str()));
    assert!(c2.verify());

    // Different prev_hash → different proof hash
    let c2_no_chain = conversation_integrity_proof("conv-xyz", "aabbccdd", 10, None);
    assert_ne!(c2.hash, c2_no_chain.hash);
}
