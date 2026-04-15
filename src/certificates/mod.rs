//! BRC-52 certificate management — agent identity and authorization.
//!
//! The `CertificateManager` wraps the four BRC-100 certificate wallet endpoints
//! (acquire, list, prove, relinquish) and provides higher-level operations for
//! agent identity management.
//!
//! On first boot, the agent acquires an "agent-authorization" certificate.
//! If a parent wallet is configured, the parent signs the cert (trust chain).
//! Otherwise, the agent self-signs as a bootstrap placeholder.

pub mod lifecycle;
pub mod policy;
pub mod types;

// Re-export all public types and functions so that `use crate::certificates::*`
// continues to work identically to the old single-file module.

pub use types::{
    is_null_revocation_outpoint, parse_revocation_outpoint, CertBudgetLimits, CertCheckResult,
    CertModerationPolicy, CertTagPolicy, CertToolApproval, CertificateStatus, CERT_TYPE_AGENT_AUTH,
};

pub use lifecycle::{check_authorization, CertificateManager};

pub use policy::{
    read_cert_budget_limits, read_cert_moderation_policy, read_cert_rate_limits,
    read_cert_tag_policy, read_cert_tool_approval,
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_cert_type_constant() {
        assert_eq!(CERT_TYPE_AGENT_AUTH, "agent-authorization");
    }

    #[test]
    fn test_certificate_fields_json() {
        let cert = json!({
            "type": CERT_TYPE_AGENT_AUTH,
            "subject": "02abc",
            "certifier": "02abc",
            "fields": {
                "name": "test-agent",
                "capabilities": "llm,tools",
                "deployed_at": "2026-02-27T00:00:00Z",
                "version": "0.1.0",
            }
        });
        assert_eq!(cert["type"], "agent-authorization");
        assert_eq!(cert["fields"]["name"], "test-agent");
        assert_eq!(cert["fields"]["capabilities"], "llm,tools");
    }

    #[test]
    fn test_certificate_status_none() {
        let status = CertificateStatus {
            status: "none".into(),
            certificate: None,
            identity_key: Some("02abc".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"status\":\"none\""));
        assert!(!json.contains("\"certificate\""));
    }

    #[test]
    fn test_certificate_status_parent_signed() {
        let cert = json!({
            "subject": "02agent",
            "certifier": "02parent",
        });
        let status = CertificateStatus {
            status: "parent-signed".into(),
            certificate: Some(cert),
            identity_key: Some("02agent".into()),
        };
        let json_str = serde_json::to_string(&status).unwrap();
        assert!(json_str.contains("parent-signed"));
        assert!(json_str.contains("02parent"));
    }

    #[test]
    fn test_certificate_status_self_signed() {
        let cert = json!({
            "subject": "02abc",
            "certifier": "02abc",
        });
        let status = CertificateStatus {
            status: "self-signed".into(),
            certificate: Some(cert),
            identity_key: Some("02abc".into()),
        };
        let json_str = serde_json::to_string(&status).unwrap();
        assert!(json_str.contains("self-signed"));
    }

    #[test]
    fn test_certificate_status_serde_roundtrip() {
        let status = CertificateStatus {
            status: "parent-signed".into(),
            certificate: Some(json!({"certifier": "02parent"})),
            identity_key: Some("02agent".into()),
        };
        let json_str = serde_json::to_string(&status).unwrap();
        let parsed: CertificateStatus = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.status, "parent-signed");
        assert!(parsed.certificate.is_some());
        assert_eq!(parsed.identity_key.as_deref(), Some("02agent"));
    }
}
