//! Tests for escalation BRC-18 proofs (Issue #204).
//!
//! Verifies:
//!   - ProofType::Escalation display and serde
//!   - escalation_proof() convenience function data format
//!   - EscalationReason::trigger_type_str() for all variants
//!   - EscalationDetector::build_proof_data() from events
//!   - Proof chain linking with prev_hash

use dolphin_milk::proofs::{escalation_proof, ProofCommitment, ProofType};
use dolphin_milk::runner::escalation::{EscalationDetector, EscalationProofData, EscalationReason};

// ─────────────────────────────────────────────
// ProofType::Escalation — Display
// ─────────────────────────────────────────────

#[test]
fn test_escalation_proof_type_display() {
    assert_eq!(ProofType::Escalation.to_string(), "escalation");
}

// ─────────────────────────────────────────────
// ProofType::Escalation — Serde roundtrip
// ─────────────────────────────────────────────

#[test]
fn test_escalation_proof_type_serde() {
    let json = serde_json::to_string(&ProofType::Escalation).unwrap();
    assert_eq!(json, r#""Escalation""#);
    let back: ProofType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProofType::Escalation);
}

// ─────────────────────────────────────────────
// escalation_proof() — data format
// ─────────────────────────────────────────────

#[test]
fn test_escalation_proof_creation() {
    let tools = vec!["memory_search".to_string(), "file_read".to_string()];
    let commitment = escalation_proof(
        "Tool loop detected — memory_search called 5 times",
        "tool_loop",
        7,
        150_000,
        250_000,
        2,
        "Search for relevant documents",
        &tools,
        None,
    );

    assert_eq!(commitment.proof_type, ProofType::Escalation);
    assert!(commitment.verify());
    assert!(commitment.prev_hash.is_none());

    // Verify all fields are present in the data
    assert!(commitment.data.contains("ESCALATION: Tool loop detected"));
    assert!(commitment.data.contains("TRIGGER: tool_loop"));
    assert!(commitment.data.contains("ITERATION: 7"));
    assert!(commitment.data.contains("BUDGET_SPENT: 150000 / 250000"));
    assert!(commitment.data.contains("ERROR_COUNT: 2"));
    assert!(commitment
        .data
        .contains("LAST_DECISION: Search for relevant documents"));
    assert!(commitment
        .data
        .contains("TOOLS_ATTEMPTED: memory_search, file_read"));
}

#[test]
fn test_escalation_proof_empty_tools() {
    let commitment = escalation_proof(
        "Budget threshold exceeded",
        "budget_threshold",
        3,
        200_000,
        250_000,
        0,
        "Check balance",
        &[],
        None,
    );

    assert_eq!(commitment.proof_type, ProofType::Escalation);
    assert!(commitment.verify());
    assert!(commitment.data.contains("TOOLS_ATTEMPTED: "));
}

// ─────────────────────────────────────────────
// trigger_type_str() — all variants
// ─────────────────────────────────────────────

#[test]
fn test_trigger_type_str_tool_loop() {
    let reason = EscalationReason::ToolLoop {
        tool_name: "memory_search".to_string(),
        call_count: 5,
    };
    assert_eq!(reason.trigger_type_str(), "tool_loop");
}

#[test]
fn test_trigger_type_str_budget_threshold() {
    let reason = EscalationReason::BudgetThreshold {
        spent: 80_000,
        limit: 100_000,
        percent: 80.0,
    };
    assert_eq!(reason.trigger_type_str(), "budget_threshold");
}

#[test]
fn test_trigger_type_str_error_threshold() {
    let reason = EscalationReason::ErrorThreshold { error_count: 4 };
    assert_eq!(reason.trigger_type_str(), "error_threshold");
}

#[test]
fn test_trigger_type_str_explicit_uncertainty() {
    let reason = EscalationReason::ExplicitUncertainty {
        trigger_phrase: "i'm stuck".to_string(),
    };
    assert_eq!(reason.trigger_type_str(), "explicit_uncertainty");
}

// ─────────────────────────────────────────────
// build_proof_data() — from EscalationEvent
// ─────────────────────────────────────────────

#[test]
fn test_build_proof_data() {
    let reason = EscalationReason::ToolLoop {
        tool_name: "memory_search".to_string(),
        call_count: 3,
    };
    let event = EscalationDetector::build_event(reason, 5);
    let proof_data = EscalationDetector::build_proof_data(&event);

    assert_eq!(proof_data.iteration, 5);
    assert_eq!(proof_data.trigger_type, "tool_loop");
    assert!(proof_data.reason.contains("memory_search"));
    assert!(proof_data.reason.contains("3 times"));
}

#[test]
fn test_build_proof_data_budget_threshold() {
    let reason = EscalationReason::BudgetThreshold {
        spent: 200_000,
        limit: 250_000,
        percent: 80.0,
    };
    let event = EscalationDetector::build_event(reason, 10);
    let proof_data = EscalationDetector::build_proof_data(&event);

    assert_eq!(proof_data.iteration, 10);
    assert_eq!(proof_data.trigger_type, "budget_threshold");
    assert!(proof_data.reason.contains("80.0%"));
}

#[test]
fn test_build_proof_data_error_threshold() {
    let reason = EscalationReason::ErrorThreshold { error_count: 4 };
    let event = EscalationDetector::build_event(reason, 2);
    let proof_data = EscalationDetector::build_proof_data(&event);

    assert_eq!(proof_data.iteration, 2);
    assert_eq!(proof_data.trigger_type, "error_threshold");
    assert!(proof_data.reason.contains("4 errors"));
}

#[test]
fn test_build_proof_data_explicit_uncertainty() {
    let reason = EscalationReason::ExplicitUncertainty {
        trigger_phrase: "i need help".to_string(),
    };
    let event = EscalationDetector::build_event(reason, 1);
    let proof_data = EscalationDetector::build_proof_data(&event);

    assert_eq!(proof_data.iteration, 1);
    assert_eq!(proof_data.trigger_type, "explicit_uncertainty");
    assert!(proof_data.reason.contains("i need help"));
}

// ─────────────────────────────────────────────
// EscalationProofData serde roundtrip
// ─────────────────────────────────────────────

#[test]
fn test_escalation_proof_data_serde() {
    let data = EscalationProofData {
        reason: "Tool loop detected".to_string(),
        trigger_type: "tool_loop".to_string(),
        iteration: 7,
    };
    let json = serde_json::to_string(&data).unwrap();
    let back: EscalationProofData = serde_json::from_str(&json).unwrap();
    assert_eq!(back.reason, "Tool loop detected");
    assert_eq!(back.trigger_type, "tool_loop");
    assert_eq!(back.iteration, 7);
}

// ─────────────────────────────────────────────
// Proof chain linking with prev_hash
// ─────────────────────────────────────────────

#[test]
fn test_escalation_proof_chain() {
    let tools = vec!["memory_search".to_string()];

    // First proof — no prev_hash
    let c1 = escalation_proof(
        "First escalation",
        "tool_loop",
        3,
        50_000,
        250_000,
        1,
        "Search attempt",
        &tools,
        None,
    );
    assert!(c1.verify());
    assert!(c1.prev_hash.is_none());

    // Second proof — chains from first
    let c2 = escalation_proof(
        "Second escalation",
        "error_threshold",
        8,
        120_000,
        250_000,
        4,
        "Parse results",
        &tools,
        Some(&c1.hash_hex()),
    );
    assert!(c2.verify());
    assert_eq!(c2.prev_hash.as_deref(), Some(c1.hash_hex().as_str()));

    // Chaining produces different hash than standalone
    let c2_standalone = escalation_proof(
        "Second escalation",
        "error_threshold",
        8,
        120_000,
        250_000,
        4,
        "Parse results",
        &tools,
        None,
    );
    // Different prev_hash means different hash (timestamps may differ, so just check structure)
    // Use with_timestamp for deterministic comparison
    let ts = "2026-03-25T12:00:00Z";
    let c_chained = ProofCommitment::with_timestamp(
        ProofType::Escalation,
        "test data",
        ts,
        Some(&c1.hash_hex()),
    );
    let c_unchained = ProofCommitment::with_timestamp(ProofType::Escalation, "test data", ts, None);
    assert_ne!(
        c_chained.hash, c_unchained.hash,
        "Chained proof must differ from unchained"
    );
    assert!(c_chained.verify());
    assert!(c_unchained.verify());
    // Suppress unused variable warning
    let _ = c2_standalone;
}

// ─────────────────────────────────────────────
// Proof verification — tamper detection
// ─────────────────────────────────────────────

#[test]
fn test_escalation_proof_tamper_detection() {
    let tools = vec!["x402_call".to_string()];
    let mut commitment = escalation_proof(
        "Budget at 85%",
        "budget_threshold",
        5,
        170_000,
        200_000,
        0,
        "API call",
        &tools,
        None,
    );

    assert!(commitment.verify());

    // Tamper with the data
    commitment.data = "ESCALATION: forged reason".to_string();
    assert!(
        !commitment.verify(),
        "Tampered data should fail verification"
    );
}

// ─────────────────────────────────────────────
// Integration: build_proof_data -> escalation_proof
// ─────────────────────────────────────────────

#[test]
fn test_proof_data_to_escalation_proof_integration() {
    // Simulate the full flow: detect -> build event -> build proof data -> create proof
    let reason = EscalationReason::ErrorThreshold { error_count: 5 };
    let event = EscalationDetector::build_event(reason, 12);
    let proof_data = EscalationDetector::build_proof_data(&event);

    let commitment = escalation_proof(
        &proof_data.reason,
        &proof_data.trigger_type,
        proof_data.iteration,
        180_000,
        250_000,
        5,
        "Final decision text",
        &["file_write".to_string(), "execute_bash".to_string()],
        None,
    );

    assert_eq!(commitment.proof_type, ProofType::Escalation);
    assert!(commitment.verify());
    assert!(commitment.data.contains("ESCALATION:"));
    assert!(commitment.data.contains("TRIGGER: error_threshold"));
    assert!(commitment.data.contains("ITERATION: 12"));
    assert!(commitment.data.contains("ERROR_COUNT: 5"));
    assert!(commitment.data.contains("file_write, execute_bash"));
}
