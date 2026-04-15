//! Audit verification for all 12 BRC-18 proof types (#229).
//!
//! For each proof type, this test:
//! 1. Generates a proof with known data via the convenience constructor
//! 2. Records the data string + timestamp
//! 3. Independently recomputes SHA-256(prev_hash_bytes || data || timestamp)
//! 4. Compares against the commitment's stored hash
//! 5. Verifies tamper detection by mutating data

use dolphin_milk::proofs::{
    capability_proof, chain_break_proof, conversation_integrity_proof, custody_proof,
    decision_proof, decision_proof_with_compliance, escalation_proof, memory_commitment_proof,
    message_receive_proof, message_send_proof, task_completion_proof, ComplianceMetadata,
    ProofCommitment, ProofType,
};
use sha2::{Digest, Sha256};

/// Independently recompute the proof hash the same way proofs.rs does.
fn recompute_hash(prev_hash: Option<&str>, data: &str, timestamp: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let Some(ph) = prev_hash {
        if let Ok(bytes) = hex::decode(ph) {
            hasher.update(&bytes);
        }
    }
    hasher.update(data.as_bytes());
    hasher.update(timestamp.as_bytes());
    hasher.finalize().into()
}

/// Assert that a commitment's hash matches an independent recomputation.
fn assert_hash_matches(commitment: &ProofCommitment) {
    let recomputed = recompute_hash(
        commitment.prev_hash.as_deref(),
        &commitment.data,
        &commitment.timestamp,
    );
    assert_eq!(
        commitment.hash, recomputed,
        "Hash mismatch for {:?} proof: stored hash does not match recomputed SHA-256",
        commitment.proof_type
    );
}

// ─────────────────────────────────────────────────
// 1. Decision proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_decision_proof() {
    let c = decision_proof("use Rust for agent", "type safety and performance", None);
    assert_eq!(c.proof_type, ProofType::Decision);
    assert!(c.data.contains("DECISION: use Rust for agent"));
    assert!(c.data.contains("REASONING: type safety and performance"));
    assert_hash_matches(&c);
    assert!(c.verify());
}

#[test]
fn test_audit_decision_proof_chained() {
    let c1 = decision_proof("step 1", "reason 1", None);
    let c2 = decision_proof("step 2", "reason 2", Some(&c1.hash_hex()));
    assert_hash_matches(&c2);
    assert_eq!(c2.prev_hash.as_deref(), Some(c1.hash_hex().as_str()));
    // Different prev_hash → different hash
    let c2_no_chain = decision_proof("step 2", "reason 2", None);
    assert_ne!(c2.hash, c2_no_chain.hash);
}

// ─────────────────────────────────────────────────
// 2. Decision proof with compliance
// ─────────────────────────────────────────────────

#[test]
fn test_audit_decision_proof_with_compliance() {
    let compliance = ComplianceMetadata {
        regulations: vec!["SEC-17a-4".into(), "FINRA-3110".into()],
        retention_days: Some(2555),
        classification: Some("financial".into()),
    };
    let c = decision_proof_with_compliance("comply", "regulation", None, &compliance);
    assert_eq!(c.proof_type, ProofType::Decision);
    assert!(c.data.contains("COMPLIANCE: SEC-17a-4,FINRA-3110"));
    assert!(c.data.contains("RETAIN: 2555d"));
    assert!(c.data.contains("CLASS: financial"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 3. TaskCompletion proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_task_completion_proof() {
    let c = task_completion_proof("build memory system", "156 tests passing", None);
    assert_eq!(c.proof_type, ProofType::TaskCompletion);
    assert!(c.data.contains("TASK: build memory system"));
    assert!(c.data.contains("RESULT: 156 tests passing"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 4. MemoryCommitment proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_memory_commitment_proof() {
    let hashes = vec![
        "aaa111".to_string(),
        "bbb222".to_string(),
        "ccc333".to_string(),
    ];
    let c = memory_commitment_proof(3, &hashes, None);
    assert_eq!(c.proof_type, ProofType::MemoryCommitment);
    assert!(c.data.contains("ENTRIES: 3"));
    assert!(c.data.contains("HASHES: aaa111,bbb222,ccc333"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 5. CapabilityProof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_capability_proof() {
    let c = capability_proof(
        "x402_call",
        "deadbeef01234567",
        "cafebabe89abcdef",
        15000,
        Some("tx_payment_abc"),
        None,
    );
    assert_eq!(c.proof_type, ProofType::CapabilityProof);
    assert!(c.data.contains("TOOL: x402_call"));
    assert!(c.data.contains("ARGS_HASH: sha256:deadbeef01234567"));
    assert!(c.data.contains("RESULT_HASH: sha256:cafebabe89abcdef"));
    assert!(c.data.contains("SATS: 15000"));
    assert!(c.data.contains("TXID: tx_payment_abc"));
    assert_hash_matches(&c);
}

#[test]
fn test_audit_capability_proof_no_txid() {
    let c = capability_proof("search_tools", "abc", "def", 0, None, None);
    assert!(c.data.contains("TXID: none"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 6. ConversationIntegrity proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_conversation_integrity_proof() {
    let c = conversation_integrity_proof("conv-abc123", "deadbeef", 42, None);
    assert_eq!(c.proof_type, ProofType::ConversationIntegrity);
    assert!(c.data.contains("CONVERSATION: conv-abc123"));
    assert!(c.data.contains("HEAD_HASH: deadbeef"));
    assert!(c.data.contains("MESSAGES: 42"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 7. Escalation proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_escalation_proof() {
    let tools = vec!["search_tools".to_string(), "file_read".to_string()];
    let c = escalation_proof(
        "Agent stuck in tool loop",
        "tool_loop",
        12,
        45000,
        250000,
        3,
        "Try different approach",
        &tools,
        None,
    );
    assert_eq!(c.proof_type, ProofType::Escalation);
    assert!(c.data.contains("ESCALATION: Agent stuck in tool loop"));
    assert!(c.data.contains("TRIGGER: tool_loop"));
    assert!(c.data.contains("ITERATION: 12"));
    assert!(c.data.contains("BUDGET_SPENT: 45000 / 250000"));
    assert!(c.data.contains("ERROR_COUNT: 3"));
    assert!(c.data.contains("LAST_DECISION: Try different approach"));
    assert!(c.data.contains("TOOLS_ATTEMPTED: search_tools, file_read"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 8. ConversationBreak proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_chain_break_proof() {
    let c = chain_break_proof("conv-xyz", "expected_aabb", "actual_ccdd", 7, None);
    assert_eq!(c.proof_type, ProofType::ConversationBreak);
    assert!(c.data.contains("CHAIN_BREAK: conversation conv-xyz"));
    assert!(c.data.contains("EXPECTED_HASH: expected_aabb"));
    assert!(c.data.contains("ACTUAL_HASH: actual_ccdd"));
    assert!(c.data.contains("BREAK_POINT: message 7"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 9. MessageSend proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_message_send_proof() {
    let c = message_send_proof(
        "hash_abc123",
        "02recipient_key",
        "status_inbox",
        500,
        true,
        true,
        None,
    );
    assert_eq!(c.proof_type, ProofType::MessageSend);
    assert!(c.data.contains("MESSAGE_SEND: hash_abc123"));
    assert!(c.data.contains("RECIPIENT: 02recipient_key"));
    assert!(c.data.contains("BOX: status_inbox"));
    assert!(c.data.contains("DELIVERY_COST: 500"));
    assert!(c.data.contains("SIGNED: true"));
    assert!(c.data.contains("ENCRYPTED: true"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 10. MessageReceive proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_message_receive_proof() {
    let c = message_receive_proof("hash_def456", "02sender_key", "general_inbox", None);
    assert_eq!(c.proof_type, ProofType::MessageReceive);
    assert!(c.data.contains("MESSAGE_RECEIVE: hash_def456"));
    assert!(c.data.contains("SENDER: 02sender_key"));
    assert!(c.data.contains("BOX: general_inbox"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 11. Custody proof
// ─────────────────────────────────────────────────

#[test]
fn test_audit_custody_proof() {
    let tools = vec!["memory_search".to_string(), "file_read".to_string()];
    let c = custody_proof(
        Some("02customer_key"),
        "task_hash_abc",
        5,
        120,
        &tools,
        15000,
        "result_hash_xyz",
        Some("prev_chain_head"),
        3,
        "02agent_key",
        Some("cert-serial-001"),
        None,
    );
    assert_eq!(c.proof_type, ProofType::Custody);
    assert!(c.data.contains("CUSTODY_PROOF: v1"));
    assert!(c.data.contains("CUSTOMER_KEY: 02customer_key"));
    assert!(c.data.contains("TASK_HASH: sha256:task_hash_abc"));
    assert!(c.data.contains("ITERATIONS: 5"));
    assert!(c.data.contains("DURATION_SECS: 120"));
    assert!(c.data.contains("TOOLS_USED: memory_search, file_read"));
    assert!(c.data.contains("TOTAL_SATS: 15000"));
    assert!(c.data.contains("RESULT_HASH: sha256:result_hash_xyz"));
    assert!(c.data.contains("PROOF_CHAIN_HEAD: prev_chain_head"));
    assert!(c.data.contains("PROOF_CHAIN_LENGTH: 3"));
    assert!(c.data.contains("AGENT_KEY: 02agent_key"));
    assert!(c.data.contains("AGENT_CERT: cert-serial-001"));
    assert_hash_matches(&c);
}

#[test]
fn test_audit_custody_proof_self_customer() {
    let c = custody_proof(
        None,
        "hash",
        1,
        10,
        &[],
        100,
        "res",
        None,
        0,
        "02key",
        None,
        None,
    );
    assert!(c.data.contains("CUSTOMER_KEY: self"));
    assert!(c.data.contains("PROOF_CHAIN_HEAD: none"));
    assert!(c.data.contains("AGENT_CERT: none"));
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 12. BudgetSnapshot proof (uses ProofCommitment::new directly)
// ─────────────────────────────────────────────────

#[test]
fn test_audit_budget_snapshot_proof() {
    let data = "BALANCE: 500000\nTASK_SPENT: 15000\nITERATIONS: 3\nSERVICES: llm:12000, state:3000";
    let c = ProofCommitment::new(ProofType::BudgetSnapshot, data, None);
    assert_eq!(c.proof_type, ProofType::BudgetSnapshot);
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// 13. CertificateRevocation proof (uses ProofCommitment::new directly)
// ─────────────────────────────────────────────────

#[test]
fn test_audit_certificate_revocation_proof() {
    let data = "REVOKED: cert-serial-abc\nREASON: parent operator revoked authorization";
    let c = ProofCommitment::new(ProofType::CertificateRevocation, data, None);
    assert_eq!(c.proof_type, ProofType::CertificateRevocation);
    assert_hash_matches(&c);
}

// ─────────────────────────────────────────────────
// Cross-cutting: tamper detection across all types
// ─────────────────────────────────────────────────

#[test]
fn test_audit_tamper_detection_data() {
    let mut c = decision_proof("original decision", "original reasoning", None);
    assert!(c.verify());
    c.data = "DECISION: tampered\nREASONING: tampered".to_string();
    assert!(!c.verify(), "Tampered data should fail verification");
}

#[test]
fn test_audit_tamper_detection_timestamp() {
    let mut c = task_completion_proof("task", "result", None);
    assert!(c.verify());
    c.timestamp = "1999-01-01T00:00:00Z".to_string();
    assert!(!c.verify(), "Tampered timestamp should fail verification");
}

#[test]
fn test_audit_tamper_detection_prev_hash() {
    let c1 = decision_proof("step 1", "reason 1", None);
    let mut c2 = decision_proof("step 2", "reason 2", Some(&c1.hash_hex()));
    assert!(c2.verify());
    c2.prev_hash =
        Some("0000000000000000000000000000000000000000000000000000000000000000".to_string());
    assert!(!c2.verify(), "Tampered prev_hash should fail verification");
}

#[test]
fn test_audit_tamper_detection_hash() {
    let mut c = memory_commitment_proof(1, &["hash1".to_string()], None);
    assert!(c.verify());
    c.hash = [0xFFu8; 32];
    assert!(!c.verify(), "Tampered hash should fail verification");
}

// ─────────────────────────────────────────────────
// Cross-cutting: deterministic hashing
// ─────────────────────────────────────────────────

#[test]
fn test_audit_deterministic_hashing() {
    let ts = "2026-03-25T12:00:00Z";
    let c1 = ProofCommitment::with_timestamp(ProofType::Decision, "same data", ts, None);
    let c2 = ProofCommitment::with_timestamp(ProofType::Decision, "same data", ts, None);
    assert_eq!(
        c1.hash, c2.hash,
        "Same inputs must produce identical hashes"
    );
    assert_hash_matches(&c1);
    assert_hash_matches(&c2);
}

#[test]
fn test_audit_different_types_same_data_same_hash() {
    // ProofType is NOT included in the hash — only data, timestamp, and prev_hash
    let ts = "2026-03-25T12:00:00Z";
    let c1 = ProofCommitment::with_timestamp(ProofType::Decision, "data", ts, None);
    let c2 = ProofCommitment::with_timestamp(ProofType::TaskCompletion, "data", ts, None);
    assert_eq!(
        c1.hash, c2.hash,
        "Hash is type-agnostic — only data+timestamp+prev_hash matter"
    );
}

// ─────────────────────────────────────────────────
// Full chain: all 12 types linked
// ─────────────────────────────────────────────────

#[test]
fn test_audit_full_12_type_chain() {
    let tools = vec!["tool1".to_string()];

    let c1 = decision_proof("d1", "r1", None);
    assert_hash_matches(&c1);

    let c2 = task_completion_proof("t1", "done", Some(&c1.hash_hex()));
    assert_hash_matches(&c2);

    let c3 = ProofCommitment::new(
        ProofType::BudgetSnapshot,
        "BALANCE: 100",
        Some(&c2.hash_hex()),
    );
    assert_hash_matches(&c3);

    let c4 = memory_commitment_proof(1, &["h1".to_string()], Some(&c3.hash_hex()));
    assert_hash_matches(&c4);

    let c5 = capability_proof("tool", "ah", "rh", 100, Some("tx1"), Some(&c4.hash_hex()));
    assert_hash_matches(&c5);

    let c6 = conversation_integrity_proof("conv-1", "head", 5, Some(&c5.hash_hex()));
    assert_hash_matches(&c6);

    let c7 = ProofCommitment::new(
        ProofType::CertificateRevocation,
        "REVOKED: cert1",
        Some(&c6.hash_hex()),
    );
    assert_hash_matches(&c7);

    let c8 = escalation_proof(
        "stuck",
        "tool_loop",
        3,
        1000,
        5000,
        2,
        "last",
        &tools,
        Some(&c7.hash_hex()),
    );
    assert_hash_matches(&c8);

    let c9 = chain_break_proof("conv-2", "exp", "act", 3, Some(&c8.hash_hex()));
    assert_hash_matches(&c9);

    let c10 = message_send_proof("mh", "rec", "box", 10, true, false, Some(&c9.hash_hex()));
    assert_hash_matches(&c10);

    let c11 = message_receive_proof("mh2", "snd", "inbox", Some(&c10.hash_hex()));
    assert_hash_matches(&c11);

    let c12 = custody_proof(
        Some("02cust"),
        "th",
        1,
        1,
        &tools,
        100,
        "rh",
        Some(&c11.hash_hex()),
        11,
        "02ag",
        None,
        Some(&c11.hash_hex()),
    );
    assert_hash_matches(&c12);

    // Verify the entire chain
    assert!(c1.verify());
    assert!(c2.verify());
    assert!(c3.verify());
    assert!(c4.verify());
    assert!(c5.verify());
    assert!(c6.verify());
    assert!(c7.verify());
    assert!(c8.verify());
    assert!(c9.verify());
    assert!(c10.verify());
    assert!(c11.verify());
    assert!(c12.verify());
}
