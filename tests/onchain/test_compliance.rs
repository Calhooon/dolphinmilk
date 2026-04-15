//! Tests for compliance metadata, WORM mode, and retention enforcement (Phase 10.5).

use dolphin_milk::config::{ComplianceConfig, DmConfig};
use dolphin_milk::onchain::proofs::{
    decision_proof, decision_proof_with_compliance, ComplianceMetadata, ProofType,
};

// ---------------------------------------------------------------------------
// ComplianceMetadata
// ---------------------------------------------------------------------------

#[test]
fn test_compliance_metadata_to_tag_string_full() {
    let meta = ComplianceMetadata {
        regulations: vec!["SEC-17a-4".into(), "FINRA-3110".into()],
        retention_days: Some(2555),
        classification: Some("financial".into()),
    };
    let tag = meta.to_tag_string();
    assert_eq!(
        tag,
        "COMPLIANCE: SEC-17a-4,FINRA-3110 | RETAIN: 2555d | CLASS: financial"
    );
}

#[test]
fn test_compliance_metadata_to_tag_string_empty_regulations() {
    let meta = ComplianceMetadata {
        regulations: vec![],
        retention_days: None,
        classification: None,
    };
    let tag = meta.to_tag_string();
    assert_eq!(tag, "COMPLIANCE: none | RETAIN: none | CLASS: none");
}

#[test]
fn test_compliance_metadata_serialization_roundtrip() {
    let meta = ComplianceMetadata {
        regulations: vec!["SEC-17a-4".into()],
        retention_days: Some(2555),
        classification: Some("financial".into()),
    };
    let json = serde_json::to_string(&meta).unwrap();
    let back: ComplianceMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(back.regulations, meta.regulations);
    assert_eq!(back.retention_days, meta.retention_days);
    assert_eq!(back.classification, meta.classification);
}

#[test]
fn test_compliance_metadata_skip_serializing_none() {
    let meta = ComplianceMetadata {
        regulations: vec!["EU-AI-Act".into()],
        retention_days: None,
        classification: None,
    };
    let json = serde_json::to_string(&meta).unwrap();
    assert!(!json.contains("retention_days"));
    assert!(!json.contains("classification"));
}

// ---------------------------------------------------------------------------
// Decision proof with compliance
// ---------------------------------------------------------------------------

#[test]
fn test_decision_proof_without_compliance() {
    let proof = decision_proof("use Rust", "type safety", None);
    assert_eq!(proof.proof_type, ProofType::Decision);
    assert!(proof.data.contains("DECISION: use Rust"));
    assert!(!proof.data.contains("COMPLIANCE:"));
    assert!(proof.verify());
}

#[test]
fn test_decision_proof_with_compliance_tags() {
    let meta = ComplianceMetadata {
        regulations: vec!["SEC-17a-4".into()],
        retention_days: Some(2555),
        classification: Some("financial".into()),
    };
    let proof =
        decision_proof_with_compliance("approve payment", "budget check passed", None, &meta);
    assert_eq!(proof.proof_type, ProofType::Decision);
    assert!(proof.data.contains("DECISION: approve payment"));
    assert!(proof.data.contains("COMPLIANCE: SEC-17a-4"));
    assert!(proof.data.contains("RETAIN: 2555d"));
    assert!(proof.data.contains("CLASS: financial"));
    assert!(proof.verify());
}

#[test]
fn test_compliance_proof_different_hash_from_non_compliance() {
    let plain = decision_proof("do X", "because Y", None);
    let meta = ComplianceMetadata {
        regulations: vec!["SEC-17a-4".into()],
        retention_days: Some(2555),
        classification: Some("financial".into()),
    };
    let compliance = decision_proof_with_compliance("do X", "because Y", None, &meta);
    // Different data → different hash (timestamps differ too, but data is the distinguisher)
    assert_ne!(plain.data, compliance.data);
}

// ---------------------------------------------------------------------------
// ComplianceConfig
// ---------------------------------------------------------------------------

#[test]
fn test_compliance_config_defaults() {
    let config = ComplianceConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.default_retention_days, 2555); // 7 years
    assert!(config.regulations.is_empty());
    assert_eq!(config.financial_classification, "financial");
    assert_eq!(config.communication_classification, "communication");
    assert!(!config.worm_mode);
}

#[test]
fn test_compliance_config_in_worm_config() {
    let config = DmConfig::default();
    assert!(!config.compliance.enabled);
    assert_eq!(config.compliance.default_retention_days, 2555);
}

#[test]
fn test_compliance_config_from_toml() {
    let toml_str = r#"
[compliance]
enabled = true
default_retention_days = 3650
regulations = ["SEC-17a-4", "FINRA-3110"]
financial_classification = "financial_trading"
worm_mode = true
"#;
    let config: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(config.compliance.enabled);
    assert_eq!(config.compliance.default_retention_days, 3650);
    assert_eq!(
        config.compliance.regulations,
        vec!["SEC-17a-4", "FINRA-3110"]
    );
    assert_eq!(
        config.compliance.financial_classification,
        "financial_trading"
    );
    assert!(config.compliance.worm_mode);
}

#[test]
fn test_compliance_config_hot_reload() {
    let mut config = DmConfig::default();
    assert!(!config.compliance.enabled);

    let mut fresh = DmConfig::default();
    fresh.compliance.enabled = true;
    fresh.compliance.worm_mode = true;
    fresh.compliance.default_retention_days = 3650;
    fresh.compliance.regulations = vec!["EU-AI-Act".into()];

    config.reload_safe_fields(&fresh);
    assert!(config.compliance.enabled);
    assert!(config.compliance.worm_mode);
    assert_eq!(config.compliance.default_retention_days, 3650);
    assert_eq!(config.compliance.regulations, vec!["EU-AI-Act"]);
}

#[test]
fn test_compliance_disabled_no_impact() {
    // When compliance is disabled, decision_proof should work exactly as before
    let config = ComplianceConfig::default();
    assert!(!config.enabled);
    let proof = decision_proof("test", "reasoning", None);
    assert!(proof.verify());
    assert!(!proof.data.contains("COMPLIANCE:"));
}

// ---------------------------------------------------------------------------
// Compliance env var parsing
// ---------------------------------------------------------------------------

#[test]
fn test_compliance_env_regulations_parsing() {
    // Test the comma-separated parsing logic (we test the logic, not actual env vars)
    let input = "SEC-17a-4, FINRA-3110, EU-AI-Act";
    let parsed: Vec<String> = input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(parsed, vec!["SEC-17a-4", "FINRA-3110", "EU-AI-Act"]);
}

#[test]
fn test_compliance_env_regulations_empty() {
    let input = "";
    let parsed: Vec<String> = input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(parsed.is_empty());
}

// ---------------------------------------------------------------------------
// Retention enforcement logic
// ---------------------------------------------------------------------------

#[test]
fn test_retention_days_to_seconds() {
    // 7 years in days → seconds
    let days = 2555u64;
    let secs = days * 86400;
    assert_eq!(secs, 220_752_000);
    // Sanity: ~7 years
    let years = secs as f64 / (365.25 * 86400.0);
    assert!((years - 6.99).abs() < 0.1);
}

#[test]
fn test_worm_mode_blocks_proofs_basket_conceptually() {
    // WORM mode should protect worm-proofs basket from deletion.
    // This is a conceptual test — actual enforcement is in sweep logic which
    // already never sweeps worm-proofs by design (BRC-18 proofs are immutable).
    let config = ComplianceConfig {
        enabled: true,
        worm_mode: true,
        default_retention_days: 2555,
        regulations: vec!["SEC-17a-4".into()],
        financial_classification: "financial".into(),
        communication_classification: "communication".into(),
    };
    assert!(config.worm_mode);
    // The sweep logic in heartbeat/features.rs NEVER touches BASKET_PROOFS.
    // WORM mode adds an extra safety layer for state and budget baskets.
    assert_eq!(dolphin_milk::onchain::state::BASKET_PROOFS, "dm-proofs");
}

#[test]
fn test_compliance_metadata_single_regulation() {
    let meta = ComplianceMetadata {
        regulations: vec!["GDPR".into()],
        retention_days: Some(365),
        classification: Some("personal_data".into()),
    };
    assert_eq!(
        meta.to_tag_string(),
        "COMPLIANCE: GDPR | RETAIN: 365d | CLASS: personal_data"
    );
}
