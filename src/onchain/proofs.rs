//! BRC-18 proof trail — immutable proof hashes on-chain via OP_FALSE OP_RETURN.
//!
//! Every significant action produces a cryptographic receipt stored permanently
//! on the BSV blockchain. ~200 sats per proof, one-time cost, no ongoing fees.
//!
//! Script format: `OP_FALSE OP_RETURN <32-byte SHA-256 hash>`
//! Verification: reconstruct hash from original data + timestamp, compare.

use bsv::script::{op, Script};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DmError;
use crate::wallet::WalletBackend;

/// Types of proof commitments the worm can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofType {
    /// Major decision + reasoning.
    Decision,
    /// Task description + result hash.
    TaskCompletion,
    /// Balance + allocations snapshot.
    BudgetSnapshot,
    /// Memory index + content hashes.
    MemoryCommitment,
    /// Tool output + verification data.
    CapabilityProof,
    /// Conversation integrity (head hash + message count).
    ConversationIntegrity,
    /// Certificate revocation record.
    CertificateRevocation,
    /// Agent escalation to human (reason + context).
    Escalation,
    /// Conversation hash chain integrity break detected.
    ConversationBreak,
    /// Cross-agent message sent via BRC-33.
    MessageSend,
    /// Cross-agent message received via BRC-33.
    MessageReceive,
    /// Cryptographic proof of work done for billing/custody.
    Custody,
}

impl std::fmt::Display for ProofType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decision => write!(f, "decision"),
            Self::TaskCompletion => write!(f, "task_completion"),
            Self::BudgetSnapshot => write!(f, "budget_snapshot"),
            Self::MemoryCommitment => write!(f, "memory_commitment"),
            Self::CapabilityProof => write!(f, "capability_proof"),
            Self::ConversationIntegrity => write!(f, "conversation_integrity"),
            Self::CertificateRevocation => write!(f, "certificate_revocation"),
            Self::Escalation => write!(f, "escalation"),
            Self::ConversationBreak => write!(f, "conversation_break"),
            Self::MessageSend => write!(f, "message_send"),
            Self::MessageReceive => write!(f, "message_receive"),
            Self::Custody => write!(f, "custody"),
        }
    }
}

/// A proof commitment ready to be stored on-chain.
///
/// The hash is `SHA-256(prev_hash_bytes || data || timestamp)`. The data and timestamp are
/// kept alongside so the proof can be verified later by recomputing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofCommitment {
    pub proof_type: ProofType,
    /// Human-readable proof data (the thing being proven).
    pub data: String,
    /// RFC 3339 timestamp when the commitment was created.
    pub timestamp: String,
    /// SHA-256(prev_hash_bytes || data || timestamp) — the 32-byte hash stored on-chain.
    pub hash: [u8; 32],
    /// Hex of previous proof hash in the chain (None for first proof).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
}

/// Result of successfully creating a proof on-chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofResult {
    /// Transaction ID containing the OP_RETURN proof.
    pub txid: String,
    /// The commitment that was proven.
    pub commitment: ProofCommitment,
}

impl ProofCommitment {
    /// Create a new proof commitment. Hashes `prev_hash_bytes || data || timestamp` with SHA-256.
    pub fn new(proof_type: ProofType, data: &str, prev_hash: Option<&str>) -> Self {
        let timestamp = Utc::now().to_rfc3339();
        let hash = compute_proof_hash(prev_hash, data, &timestamp);
        Self {
            proof_type,
            data: data.to_string(),
            timestamp,
            hash,
            prev_hash: prev_hash.map(|s| s.to_string()),
        }
    }

    /// Create a commitment with a specific timestamp (for testing/reconstruction).
    pub fn with_timestamp(
        proof_type: ProofType,
        data: &str,
        timestamp: &str,
        prev_hash: Option<&str>,
    ) -> Self {
        let hash = compute_proof_hash(prev_hash, data, timestamp);
        Self {
            proof_type,
            data: data.to_string(),
            timestamp: timestamp.to_string(),
            hash,
            prev_hash: prev_hash.map(|s| s.to_string()),
        }
    }

    /// Verify that the hash matches `SHA-256(prev_hash_bytes || data || timestamp)`.
    pub fn verify(&self) -> bool {
        let computed = compute_proof_hash(self.prev_hash.as_deref(), &self.data, &self.timestamp);
        computed == self.hash
    }

    /// Return the hash as a hex string.
    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash)
    }
}

/// Compute `SHA-256(prev_hash_bytes || data || timestamp)`.
fn compute_proof_hash(prev_hash: Option<&str>, data: &str, timestamp: &str) -> [u8; 32] {
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

/// Build an OP_FALSE OP_RETURN locking script for a 32-byte hash.
///
/// Format: `OP_FALSE (0x00) | OP_RETURN (0x6a) | OP_PUSH32 (0x20) | <32 bytes>`
///
/// Uses the BSV SDK's `Script` type for proper script construction.
/// Returns the script as a hex string (suitable for `createAction` lockingScript).
pub fn build_op_return_script(hash: &[u8; 32]) -> String {
    let mut script = Script::new();
    script
        .write_opcode(op::OP_FALSE)
        .write_opcode(op::OP_RETURN)
        .write_bin(hash);
    script.to_hex()
}

/// Create an on-chain BRC-18 proof via the wallet.
///
/// Broadcasts a transaction with an OP_FALSE OP_RETURN output containing
/// the commitment hash. Cost: ~200 sats (one-time, permanent).
pub async fn create_proof(
    wallet: &dyn WalletBackend,
    commitment: ProofCommitment,
) -> Result<ProofResult, DmError> {
    let script = build_op_return_script(&commitment.hash);

    let output = serde_json::json!({
        "lockingScript": script,
        "satoshis": 0,
        "outputDescription": "dolphin milk proof",
        "basket": "dm-proofs",
    });

    let result = wallet
        .create_action(
            &[output],
            "dolphin milk proof",
            false, // don't accept delayed broadcast — proofs should confirm
            false, // don't randomize outputs
        )
        .await?;

    Ok(ProofResult {
        txid: result.txid,
        commitment,
    })
}

/// Compliance metadata optionally attached to proofs when compliance mode is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceMetadata {
    /// Regulatory frameworks this proof satisfies (e.g., "SEC-17a-4", "FINRA-3110").
    pub regulations: Vec<String>,
    /// Retention requirement in days (e.g., 2555 = 7 years for SEC 17a-4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u64>,
    /// Classification level (e.g., "financial", "operational", "communication").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
}

impl ComplianceMetadata {
    /// Format compliance metadata as a tag string for proof data.
    pub fn to_tag_string(&self) -> String {
        let regs = if self.regulations.is_empty() {
            "none".to_string()
        } else {
            self.regulations.join(",")
        };
        let retain = self
            .retention_days
            .map(|d| format!("{d}d"))
            .unwrap_or_else(|| "none".to_string());
        let class = self.classification.as_deref().unwrap_or("none");
        format!("COMPLIANCE: {regs} | RETAIN: {retain} | CLASS: {class}")
    }
}

/// Convenience: create a decision proof.
pub fn decision_proof(decision: &str, reasoning: &str, prev_hash: Option<&str>) -> ProofCommitment {
    let data = format!("DECISION: {}\nREASONING: {}", decision, reasoning);
    ProofCommitment::new(ProofType::Decision, &data, prev_hash)
}

/// Convenience: create a decision proof with compliance metadata.
pub fn decision_proof_with_compliance(
    decision: &str,
    reasoning: &str,
    prev_hash: Option<&str>,
    compliance: &ComplianceMetadata,
) -> ProofCommitment {
    let data = format!(
        "DECISION: {}\nREASONING: {}\n{}",
        decision,
        reasoning,
        compliance.to_tag_string()
    );
    ProofCommitment::new(ProofType::Decision, &data, prev_hash)
}

/// Convenience: create a task completion proof.
pub fn task_completion_proof(
    task: &str,
    result_summary: &str,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!("TASK: {}\nRESULT: {}", task, result_summary);
    ProofCommitment::new(ProofType::TaskCompletion, &data, prev_hash)
}

/// Convenience: create a memory commitment proof.
pub fn memory_commitment_proof(
    entry_count: usize,
    content_hashes: &[String],
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let hashes_str = content_hashes.join(",");
    let data = format!("ENTRIES: {}\nHASHES: {}", entry_count, hashes_str);
    ProofCommitment::new(ProofType::MemoryCommitment, &data, prev_hash)
}

/// Convenience: create a capability proof for a spending tool call.
pub fn capability_proof(
    tool_name: &str,
    args_hash: &str,
    result_hash: &str,
    sats_paid: u64,
    payment_txid: Option<&str>,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!(
        "TOOL: {}\nARGS_HASH: sha256:{}\nRESULT_HASH: sha256:{}\nSATS: {}\nTXID: {}",
        tool_name,
        args_hash,
        result_hash,
        sats_paid,
        payment_txid.unwrap_or("none"),
    );
    ProofCommitment::new(ProofType::CapabilityProof, &data, prev_hash)
}

/// Convenience: create a conversation integrity proof.
pub fn conversation_integrity_proof(
    conv_id: &str,
    head_hash: &str,
    message_count: usize,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!(
        "CONVERSATION: {}\nHEAD_HASH: {}\nMESSAGES: {}",
        conv_id, head_hash, message_count
    );
    ProofCommitment::new(ProofType::ConversationIntegrity, &data, prev_hash)
}

/// Convenience: create an escalation proof with structured data.
#[allow(clippy::too_many_arguments)]
pub fn escalation_proof(
    reason: &str,
    trigger_type: &str,
    iteration: u32,
    budget_spent: u64,
    budget_cap: u64,
    error_count: usize,
    last_decision: &str,
    tools_attempted: &[String],
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!(
        "ESCALATION: {reason}\nTRIGGER: {trigger_type}\nITERATION: {iteration}\n\
         BUDGET_SPENT: {budget_spent} / {budget_cap}\nERROR_COUNT: {error_count}\n\
         LAST_DECISION: {last_decision}\nTOOLS_ATTEMPTED: {}",
        tools_attempted.join(", ")
    );
    ProofCommitment::new(ProofType::Escalation, &data, prev_hash)
}

/// Convenience: create a proof recording a conversation hash chain break.
pub fn chain_break_proof(
    conv_id: &str,
    expected_hash: &str,
    actual_hash: &str,
    break_point: usize,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!(
        "CHAIN_BREAK: conversation {conv_id}\n\
         EXPECTED_HASH: {expected_hash}\n\
         ACTUAL_HASH: {actual_hash}\n\
         BREAK_POINT: message {break_point}"
    );
    ProofCommitment::new(ProofType::ConversationBreak, &data, prev_hash)
}

/// Convenience: create a proof recording a cross-agent message send via BRC-33.
#[allow(clippy::too_many_arguments)]
pub fn message_send_proof(
    message_hash: &str,
    recipient: &str,
    box_name: &str,
    sats: u64,
    signed: bool,
    encrypted: bool,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!(
        "MESSAGE_SEND: {message_hash}\nRECIPIENT: {recipient}\nBOX: {box_name}\n\
         DELIVERY_COST: {sats}\nSIGNED: {signed}\nENCRYPTED: {encrypted}"
    );
    ProofCommitment::new(ProofType::MessageSend, &data, prev_hash)
}

/// Convenience: create a proof recording a cross-agent message receive via BRC-33.
pub fn message_receive_proof(
    message_hash: &str,
    sender: &str,
    box_name: &str,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let data = format!("MESSAGE_RECEIVE: {message_hash}\nSENDER: {sender}\nBOX: {box_name}");
    ProofCommitment::new(ProofType::MessageReceive, &data, prev_hash)
}

/// Convenience: create a custody proof for billing verification.
///
/// Captures work summary, customer identity, and proof chain linkage
/// so customers can independently verify that work was performed.
#[allow(clippy::too_many_arguments)]
pub fn custody_proof(
    caller_key: Option<&str>,
    task_hash: &str,
    iterations: u32,
    duration_secs: u64,
    tools_used: &[String],
    total_sats: u64,
    result_hash: &str,
    proof_chain_head: Option<&str>,
    proof_chain_length: usize,
    agent_key: &str,
    cert_serial: Option<&str>,
    prev_hash: Option<&str>,
) -> ProofCommitment {
    let customer = caller_key.unwrap_or("self");
    let tools = tools_used.join(", ");
    let chain_head = proof_chain_head.unwrap_or("none");
    let cert = cert_serial.unwrap_or("none");
    let data = format!(
        "CUSTODY_PROOF: v1\nCUSTOMER_KEY: {customer}\nTASK_HASH: sha256:{task_hash}\n\
         ITERATIONS: {iterations}\nDURATION_SECS: {duration_secs}\n\
         TOOLS_USED: {tools}\nTOTAL_SATS: {total_sats}\n\
         RESULT_HASH: sha256:{result_hash}\n\
         PROOF_CHAIN_HEAD: {chain_head}\nPROOF_CHAIN_LENGTH: {proof_chain_length}\n\
         AGENT_KEY: {agent_key}\nAGENT_CERT: {cert}"
    );
    ProofCommitment::new(ProofType::Custody, &data, prev_hash)
}

// ---------------------------------------------------------------------------
// Read proof chain from blockchain (R.1)
// ---------------------------------------------------------------------------

/// A proof read back from the blockchain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnChainProof {
    /// Transaction ID containing the proof.
    pub txid: String,
    /// SHA-256 hash stored in the OP_RETURN output (hex-encoded).
    pub hash: String,
}

/// Divergence between in-memory state and on-chain proof chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDivergence {
    pub field: String,
    pub in_memory: String,
    pub on_chain: String,
}

/// Result of verifying in-memory state against on-chain proofs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub consistent: bool,
    pub chain_length: usize,
    pub divergences: Vec<StateDivergence>,
    pub last_proof_hash: Option<String>,
}

/// Parse the 32-byte hash from a BRC-18 OP_RETURN locking script hex string.
///
/// Expected format: `OP_FALSE(0x00) OP_RETURN(0x6a) OP_PUSH32(0x20) <32 bytes>`
/// Returns the hash as a hex string, or `None` if the script doesn't match.
pub fn parse_op_return_hash(script_hex: &str) -> Option<String> {
    let bytes = hex::decode(script_hex).ok()?;
    // Must be exactly 35 bytes: 00 6a 20 + 32 hash bytes
    if bytes.len() != 35 {
        return None;
    }
    if bytes[0] != 0x00 || bytes[1] != 0x6a || bytes[2] != 0x20 {
        return None;
    }
    Some(hex::encode(&bytes[3..35]))
}

/// Read the agent's own proof chain from the blockchain via wallet `list_outputs`.
///
/// Returns proofs from the `worm-proofs` basket, newest first (by list order).
/// Each output's locking script is parsed to extract the 32-byte OP_RETURN hash.
pub async fn read_proof_chain(
    wallet: &dyn WalletBackend,
    limit: usize,
) -> Result<Vec<OnChainProof>, DmError> {
    let mut proofs = Vec::new();
    let mut offset = 0u64;
    let page_size = 100u64;

    loop {
        let result = wallet
            .list_outputs("dm-proofs", "locking scripts", page_size, offset)
            .await?;

        let outputs = match result.get("outputs").and_then(|v| v.as_array()) {
            Some(arr) => arr.clone(),
            None => break,
        };
        if outputs.is_empty() {
            break;
        }

        for output in &outputs {
            if proofs.len() >= limit {
                break;
            }

            let script_hex = output
                .get("lockingScript")
                .or_else(|| output.get("locking_script"))
                .and_then(|v| v.as_str());

            let txid = output
                .get("outpoint")
                .and_then(|v| v.as_str())
                .and_then(|op| op.split('.').next())
                .or_else(|| output.get("txid").and_then(|v| v.as_str()));

            if let (Some(script), Some(txid)) = (script_hex, txid) {
                if let Some(hash) = parse_op_return_hash(script) {
                    proofs.push(OnChainProof {
                        txid: txid.to_string(),
                        hash,
                    });
                }
            }
        }

        if proofs.len() >= limit || outputs.len() < page_size as usize {
            break;
        }
        offset += page_size;
    }

    Ok(proofs)
}

/// Verify in-memory state against on-chain proofs.
///
/// Compares the on-chain proof chain length against the expected iteration count,
/// and checks the last proof hash for consistency.
pub fn verify_state_against_proofs(
    proofs: &[OnChainProof],
    iteration: u32,
    _sats_spent: u64,
    last_proof_hash: Option<&str>,
) -> VerificationResult {
    let mut divergences = Vec::new();

    // Compare chain length vs iteration count.
    // Each iteration produces at least one Decision proof, but there are also
    // setup proofs (TaskCompletion, BudgetSnapshot, etc.), so chain_length >= iteration
    // is expected. A chain_length of 0 when iteration > 0 is a divergence.
    if proofs.is_empty() && iteration > 0 {
        divergences.push(StateDivergence {
            field: "chain_length".to_string(),
            in_memory: format!("{}", iteration),
            on_chain: "0".to_string(),
        });
    }

    // Compare last proof hash.
    let chain_last_hash = proofs.first().map(|p| p.hash.as_str());
    if let Some(mem_hash) = last_proof_hash {
        if let Some(chain_hash) = chain_last_hash {
            if mem_hash != chain_hash {
                divergences.push(StateDivergence {
                    field: "last_proof_hash".to_string(),
                    in_memory: mem_hash.to_string(),
                    on_chain: chain_hash.to_string(),
                });
            }
        }
    }

    VerificationResult {
        consistent: divergences.is_empty(),
        chain_length: proofs.len(),
        divergences,
        last_proof_hash: chain_last_hash.map(|s| s.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Memory tamper detection (H.3)
// ---------------------------------------------------------------------------

/// Compute a SHA-256 hash of a memory entry's content, creation timestamp, and category.
///
/// This produces a deterministic hex string that can be stored alongside the entry
/// and later compared to detect tampering or corruption.
pub fn compute_memory_hash(content: &str, created_at: &str, category: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher.update(created_at.as_bytes());
    hasher.update(category.as_bytes());
    hex::encode(hasher.finalize())
}

/// Verify a memory entry's content hash against an expected on-chain hash.
///
/// Recomputes `SHA-256(content || created_at || category)` and compares the hex
/// digest against `expected_hash`. Returns `true` if they match.
pub fn verify_memory_hash(
    content: &str,
    created_at: &str,
    category: &str,
    expected_hash: &str,
) -> bool {
    let computed = compute_memory_hash(content, created_at, category);
    computed == expected_hash
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proof_commitment_new() {
        let c = ProofCommitment::new(ProofType::Decision, "test decision", None);
        assert_eq!(c.data, "test decision");
        assert_eq!(c.proof_type, ProofType::Decision);
        assert!(!c.timestamp.is_empty());
        assert!(c.verify());
        assert!(c.prev_hash.is_none());
    }

    #[test]
    fn test_proof_commitment_with_timestamp() {
        let c = ProofCommitment::with_timestamp(
            ProofType::TaskCompletion,
            "hello",
            "2026-02-23T10:00:00Z",
            None,
        );
        assert_eq!(c.timestamp, "2026-02-23T10:00:00Z");
        assert!(c.verify());

        // Same data + timestamp = same hash (deterministic)
        let c2 = ProofCommitment::with_timestamp(
            ProofType::TaskCompletion,
            "hello",
            "2026-02-23T10:00:00Z",
            None,
        );
        assert_eq!(c.hash, c2.hash);
    }

    #[test]
    fn test_proof_commitment_verify_tampered_data() {
        let mut c = ProofCommitment::new(ProofType::Decision, "original", None);
        c.data = "tampered".to_string();
        assert!(!c.verify());
    }

    #[test]
    fn test_proof_commitment_verify_tampered_timestamp() {
        let mut c = ProofCommitment::new(ProofType::Decision, "data", None);
        c.timestamp = "1999-01-01T00:00:00Z".to_string();
        assert!(!c.verify());
    }

    #[test]
    fn test_different_timestamps_different_hashes() {
        let c1 = ProofCommitment::with_timestamp(
            ProofType::Decision,
            "same",
            "2026-01-01T00:00:00Z",
            None,
        );
        let c2 = ProofCommitment::with_timestamp(
            ProofType::Decision,
            "same",
            "2026-01-02T00:00:00Z",
            None,
        );
        assert_ne!(c1.hash, c2.hash);
    }

    #[test]
    fn test_different_data_different_hashes() {
        let ts = "2026-01-01T00:00:00Z";
        let c1 = ProofCommitment::with_timestamp(ProofType::Decision, "alpha", ts, None);
        let c2 = ProofCommitment::with_timestamp(ProofType::Decision, "beta", ts, None);
        assert_ne!(c1.hash, c2.hash);
    }

    #[test]
    fn test_build_op_return_script_format() {
        let hash = [0xABu8; 32];
        let script = build_op_return_script(&hash);
        // OP_FALSE=00, OP_RETURN=6a, OP_PUSH32=20, then 64 hex chars
        assert_eq!(&script[..6], "006a20");
        assert_eq!(script.len(), 6 + 64);
    }

    #[test]
    fn test_build_op_return_script_known_value() {
        let mut hash = [0u8; 32];
        hash[0] = 0xFF;
        hash[31] = 0x01;
        let script = build_op_return_script(&hash);
        assert!(script.starts_with("006a20ff"));
        assert!(script.ends_with("01"));
    }

    #[test]
    fn test_proof_type_display() {
        assert_eq!(ProofType::Decision.to_string(), "decision");
        assert_eq!(ProofType::TaskCompletion.to_string(), "task_completion");
        assert_eq!(ProofType::BudgetSnapshot.to_string(), "budget_snapshot");
        assert_eq!(ProofType::MemoryCommitment.to_string(), "memory_commitment");
        assert_eq!(ProofType::CapabilityProof.to_string(), "capability_proof");
        assert_eq!(
            ProofType::ConversationIntegrity.to_string(),
            "conversation_integrity"
        );
        assert_eq!(
            ProofType::ConversationBreak.to_string(),
            "conversation_break"
        );
        assert_eq!(ProofType::MessageSend.to_string(), "message_send");
        assert_eq!(ProofType::MessageReceive.to_string(), "message_receive");
    }

    #[test]
    fn test_proof_type_serialize() {
        let json = serde_json::to_string(&ProofType::Decision).unwrap();
        assert_eq!(json, r#""Decision""#);
        let back: ProofType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ProofType::Decision);
    }

    #[test]
    fn test_proof_commitment_serialize_roundtrip() {
        let c = ProofCommitment::with_timestamp(
            ProofType::MemoryCommitment,
            "test data",
            "2026-02-23T12:00:00Z",
            None,
        );
        let json = serde_json::to_string(&c).unwrap();
        let back: ProofCommitment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.proof_type, ProofType::MemoryCommitment);
        assert_eq!(back.data, "test data");
        assert_eq!(back.timestamp, "2026-02-23T12:00:00Z");
        assert_eq!(back.hash, c.hash);
        assert!(back.verify());
    }

    #[test]
    fn test_proof_result_serialize() {
        let c = ProofCommitment::with_timestamp(
            ProofType::TaskCompletion,
            "completed task",
            "2026-02-23T12:00:00Z",
            None,
        );
        let result = ProofResult {
            txid: "abc123def456".to_string(),
            commitment: c,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("abc123def456"));
        assert!(json.contains("TaskCompletion"));
    }

    #[test]
    fn test_hash_hex() {
        let c = ProofCommitment::with_timestamp(
            ProofType::Decision,
            "test",
            "2026-01-01T00:00:00Z",
            None,
        );
        let hex_str = c.hash_hex();
        assert_eq!(hex_str.len(), 64);
        // Verify it matches the raw hash
        assert_eq!(hex::decode(&hex_str).unwrap(), c.hash.to_vec());
    }

    #[test]
    fn test_decision_proof_helper() {
        let c = decision_proof("use Rust", "performance and type safety", None);
        assert_eq!(c.proof_type, ProofType::Decision);
        assert!(c.data.contains("DECISION: use Rust"));
        assert!(c.data.contains("REASONING: performance and type safety"));
        assert!(c.verify());
    }

    #[test]
    fn test_task_completion_proof_helper() {
        let c = task_completion_proof("build memory system", "156 tests passing", None);
        assert_eq!(c.proof_type, ProofType::TaskCompletion);
        assert!(c.data.contains("TASK: build memory system"));
        assert!(c.data.contains("RESULT: 156 tests passing"));
        assert!(c.verify());
    }

    #[test]
    fn test_memory_commitment_proof_helper() {
        let hashes = vec!["aaa".to_string(), "bbb".to_string()];
        let c = memory_commitment_proof(2, &hashes, None);
        assert_eq!(c.proof_type, ProofType::MemoryCommitment);
        assert!(c.data.contains("ENTRIES: 2"));
        assert!(c.data.contains("HASHES: aaa,bbb"));
        assert!(c.verify());
    }

    #[test]
    fn test_compute_proof_hash_deterministic() {
        let h1 = compute_proof_hash(None, "data", "ts");
        let h2 = compute_proof_hash(None, "data", "ts");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_op_return_script_is_valid_hex() {
        let hash = compute_proof_hash(None, "test", "now");
        let script = build_op_return_script(&hash);
        // Should be valid hex
        assert!(hex::decode(&script).is_ok());
        let bytes = hex::decode(&script).unwrap();
        assert_eq!(bytes[0], 0x00); // OP_FALSE
        assert_eq!(bytes[1], 0x6a); // OP_RETURN
        assert_eq!(bytes[2], 0x20); // PUSH 32 bytes
        assert_eq!(bytes.len(), 35); // 3 opcodes + 32 hash bytes
    }
}
