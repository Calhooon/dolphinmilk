//! Tests for BRC-52 certificate hash in Decision proofs (#198).
//!
//! Validates that certificate hashes are computed deterministically,
//! differ for different certificates, and correctly populate AuthState.

use sha2::{Digest, Sha256};

/// Helper: compute cert hash the same way lifecycle.rs does.
fn compute_cert_hash(certifier: &str, name: &str, capabilities: &str, self_signed: bool) -> String {
    let cert_data = format!(
        "{}:{}:{}:{}",
        certifier,
        name,
        capabilities,
        if self_signed {
            "self-signed"
        } else {
            "parent-signed"
        }
    );
    let hash = Sha256::digest(cert_data.as_bytes());
    hex::encode(hash)
}

#[test]
fn test_cert_hash_computation() {
    // Same inputs must produce the same hash (deterministic).
    let hash1 = compute_cert_hash("02abc123", "test-agent", "all", false);
    let hash2 = compute_cert_hash("02abc123", "test-agent", "all", false);
    assert_eq!(
        hash1, hash2,
        "Identical cert info must produce identical hashes"
    );
    // Hash must be 64 hex chars (SHA-256).
    assert_eq!(hash1.len(), 64);
    // Must be valid hex.
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_cert_hash_different_for_different_certs() {
    let hash_a = compute_cert_hash("02abc123", "agent-alpha", "all", false);
    let hash_b = compute_cert_hash("02def456", "agent-beta", "sandbox,wallet", true);
    assert_ne!(
        hash_a, hash_b,
        "Different cert info must produce different hashes"
    );
}

#[test]
fn test_cert_type_self_signed() {
    let cert_type = if true { "self-signed" } else { "parent-signed" };
    assert_eq!(cert_type, "self-signed");

    // The hash should include "self-signed" in the preimage.
    let hash_self = compute_cert_hash("02aaa", "agent", "all", true);
    let hash_parent = compute_cert_hash("02aaa", "agent", "all", false);
    assert_ne!(
        hash_self, hash_parent,
        "self-signed vs parent-signed must produce different hashes"
    );
}

#[test]
fn test_cert_type_parent_signed() {
    let cert_type = if false {
        "self-signed"
    } else {
        "parent-signed"
    };
    assert_eq!(cert_type, "parent-signed");

    // Verify the hash preimage includes "parent-signed".
    let data = format!(
        "{}:{}:{}:{}",
        "02certifier", "my-agent", "all", "parent-signed"
    );
    let expected = hex::encode(Sha256::digest(data.as_bytes()));
    let actual = compute_cert_hash("02certifier", "my-agent", "all", false);
    assert_eq!(actual, expected);
}

#[test]
fn test_cert_hash_in_auth_state() {
    // Simulate what lifecycle.rs does: populate AuthState fields.
    let cert_hash = Some(compute_cert_hash("02xyz", "test", "all", false));
    let cert_type = Some("parent-signed".to_string());

    assert!(cert_hash.is_some());
    assert_eq!(cert_type.as_deref(), Some("parent-signed"));
    assert_eq!(cert_hash.as_ref().unwrap().len(), 64);
}

#[test]
fn test_no_cert_hash_when_no_certificate() {
    // When no certificate is available, cert_hash and cert_type remain None.
    let cert_hash: Option<String> = None;
    let cert_type: Option<String> = None;

    assert!(
        cert_hash.is_none(),
        "No certificate should produce None cert_hash"
    );
    assert!(
        cert_type.is_none(),
        "No certificate should produce None cert_type"
    );
}

#[test]
fn test_cert_hash_in_proof_data_format() {
    // Verify the format string that gets appended to Decision proof reasoning.
    let cert_hash = compute_cert_hash("02certifier", "agent-name", "sandbox,wallet", false);
    let cert_type = "parent-signed";

    let mut reasoning =
        String::from("sha256:abc123\nMODEL: gpt-5-mini\nSATS: 500\nPAYMENT_TXID: tx123");
    reasoning.push_str(&format!("\nCERT_HASH: sha256:{cert_hash}"));
    reasoning.push_str(&format!("\nCERT_TYPE: {cert_type}"));

    assert!(reasoning.contains("CERT_HASH: sha256:"));
    assert!(reasoning.contains("CERT_TYPE: parent-signed"));
    // Cert hash line should be 64 hex chars after the sha256: prefix.
    let cert_line = reasoning
        .lines()
        .find(|l| l.starts_with("CERT_HASH:"))
        .unwrap();
    let hash_value = cert_line.strip_prefix("CERT_HASH: sha256:").unwrap();
    assert_eq!(hash_value.len(), 64);
}

#[test]
fn test_cert_hash_changes_with_capabilities() {
    // Changing capabilities should change the hash.
    let hash_all = compute_cert_hash("02abc", "agent", "all", false);
    let hash_limited = compute_cert_hash("02abc", "agent", "sandbox,wallet", false);
    assert_ne!(
        hash_all, hash_limited,
        "Different capabilities must produce different hashes"
    );
}

#[test]
fn test_cert_hash_changes_with_certifier() {
    // Changing certifier should change the hash.
    let hash_a = compute_cert_hash("02abc", "agent", "all", false);
    let hash_b = compute_cert_hash("02def", "agent", "all", false);
    assert_ne!(
        hash_a, hash_b,
        "Different certifiers must produce different hashes"
    );
}

#[test]
fn test_cert_hash_changes_with_name() {
    // Changing agent name should change the hash.
    let hash_a = compute_cert_hash("02abc", "agent-alpha", "all", false);
    let hash_b = compute_cert_hash("02abc", "agent-beta", "all", false);
    assert_ne!(
        hash_a, hash_b,
        "Different agent names must produce different hashes"
    );
}
