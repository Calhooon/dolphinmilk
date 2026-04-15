//! Tests for state provenance chain — compute_state_root() (#202).
//!
//! Verifies that the SHA-256 state root hash is deterministic, changes
//! with different inputs, handles missing tokens gracefully, and produces
//! valid hex output.

use dolphin_milk::state::compute_state_root;

// ─────────────────────────────────────────────
// Determinism
// ─────────────────────────────────────────────

#[test]
fn test_compute_state_root_deterministic() {
    let a = compute_state_root(
        Some("tx_commit_1"),
        Some("tx_budget_1"),
        Some("tx_cap_1"),
        Some("tx_checkpoint_1"),
    );
    let b = compute_state_root(
        Some("tx_commit_1"),
        Some("tx_budget_1"),
        Some("tx_cap_1"),
        Some("tx_checkpoint_1"),
    );
    assert_eq!(a, b, "Same inputs must produce the same state root");
}

// ─────────────────────────────────────────────
// Sensitivity to input changes
// ─────────────────────────────────────────────

#[test]
fn test_compute_state_root_changes_with_input() {
    let base = compute_state_root(
        Some("tx_commit_1"),
        Some("tx_budget_1"),
        Some("tx_cap_1"),
        Some("tx_checkpoint_1"),
    );

    // Changing task_commitment txid changes the root
    let changed_commit = compute_state_root(
        Some("tx_commit_2"),
        Some("tx_budget_1"),
        Some("tx_cap_1"),
        Some("tx_checkpoint_1"),
    );
    assert_ne!(
        base, changed_commit,
        "Different task_commitment txid must change root"
    );

    // Changing budget_allocation txid changes the root
    let changed_budget = compute_state_root(
        Some("tx_commit_1"),
        Some("tx_budget_2"),
        Some("tx_cap_1"),
        Some("tx_checkpoint_1"),
    );
    assert_ne!(
        base, changed_budget,
        "Different budget_allocation txid must change root"
    );

    // Changing capability_declaration txid changes the root
    let changed_cap = compute_state_root(
        Some("tx_commit_1"),
        Some("tx_budget_1"),
        Some("tx_cap_2"),
        Some("tx_checkpoint_1"),
    );
    assert_ne!(
        base, changed_cap,
        "Different capability_declaration txid must change root"
    );

    // Changing checkpoint txid changes the root
    let changed_checkpoint = compute_state_root(
        Some("tx_commit_1"),
        Some("tx_budget_1"),
        Some("tx_cap_1"),
        Some("tx_checkpoint_2"),
    );
    assert_ne!(
        base, changed_checkpoint,
        "Different checkpoint txid must change root"
    );
}

// ─────────────────────────────────────────────
// Missing (None) tokens
// ─────────────────────────────────────────────

#[test]
fn test_compute_state_root_missing_tokens() {
    // All present vs one missing should differ
    let all_present = compute_state_root(Some("tx1"), Some("tx2"), Some("tx3"), Some("tx4"));
    let one_missing = compute_state_root(None, Some("tx2"), Some("tx3"), Some("tx4"));
    assert_ne!(
        all_present, one_missing,
        "None vs Some must produce different root"
    );
}

#[test]
fn test_compute_state_root_all_none() {
    let root = compute_state_root(None, None, None, None);
    // Should still produce a valid hash (SHA-256 of four empty strings)
    assert!(!root.is_empty(), "All-None must still produce a hash");
    assert_eq!(root.len(), 64, "SHA-256 hex must be 64 characters");
}

#[test]
fn test_compute_state_root_partial() {
    // Some present, some None — should work and be deterministic
    let a = compute_state_root(Some("tx_commit_abc"), None, None, Some("tx_checkpoint_xyz"));
    let b = compute_state_root(Some("tx_commit_abc"), None, None, Some("tx_checkpoint_xyz"));
    assert_eq!(a, b, "Partial inputs must be deterministic");

    // Different from all-None
    let all_none = compute_state_root(None, None, None, None);
    assert_ne!(a, all_none, "Partial must differ from all-None");
}

// ─────────────────────────────────────────────
// Output format
// ─────────────────────────────────────────────

#[test]
fn test_state_root_is_hex() {
    let root = compute_state_root(Some("abc123"), Some("def456"), None, Some("ghi789"));
    assert!(
        root.chars().all(|c| c.is_ascii_hexdigit()),
        "State root must be valid hex, got: {root}"
    );
}

#[test]
fn test_state_root_length() {
    let root = compute_state_root(
        Some("aabbccdd"),
        Some("eeff0011"),
        Some("22334455"),
        Some("66778899"),
    );
    assert_eq!(
        root.len(),
        64,
        "SHA-256 hex digest must be exactly 64 characters"
    );
}
