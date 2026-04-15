//! Certificate-related types, constants, and helper functions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Well-known certificate type for agent authorization.
pub const CERT_TYPE_AGENT_AUTH: &str = "agent-authorization";

/// Null revocation outpoint placeholder (72 hex chars = 36 zero bytes).
pub(crate) const NULL_REVOCATION_OUTPOINT: &str =
    "000000000000000000000000000000000000000000000000000000000000000000000000";

/// Check whether a revocation outpoint is the null placeholder.
pub fn is_null_revocation_outpoint(outpoint: &str) -> bool {
    outpoint.is_empty() || outpoint == NULL_REVOCATION_OUTPOINT
}

/// Parse a revocation outpoint string into (txid, vout).
/// Returns None if the outpoint is null/empty or malformed.
pub fn parse_revocation_outpoint(outpoint: &str) -> Option<(String, u32)> {
    if is_null_revocation_outpoint(outpoint) {
        return None;
    }
    if outpoint.len() != 72 {
        return None;
    }
    let txid = &outpoint[..64];
    let vout = u32::from_str_radix(&outpoint[64..], 16).ok()?;
    Some((txid.to_string(), vout))
}

/// Certificate-derived budget limits and enforcement mode.
pub struct CertBudgetLimits {
    pub per_task: Option<u64>,
    pub per_hour: Option<u64>,
    pub per_day: Option<u64>,
    pub per_week: Option<u64>,
    pub per_month: Option<u64>,
    pub lifetime: Option<u64>,
    /// Certificate-derived enforcement mode. Overrides config when present.
    pub enforcement: Option<String>,
}

impl CertBudgetLimits {
    pub(crate) fn none() -> Self {
        Self {
            per_task: None,
            per_hour: None,
            per_day: None,
            per_week: None,
            per_month: None,
            lifetime: None,
            enforcement: None,
        }
    }
}

/// Certificate-derived moderation policy overrides.
///
/// When present, cert fields take precedence over config — a parent can force-enable
/// moderation or force a stricter mode, and the agent's local config cannot downgrade.
pub struct CertModerationPolicy {
    /// If Some(true), parent forces moderation on; agent config cannot disable.
    pub enabled: Option<bool>,
    /// PII detection mode: "block", "flag", "off". Cert wins over config when stricter.
    pub pii_mode: Option<String>,
    /// Profanity detection mode: "block", "flag", "off". Cert wins over config when stricter.
    pub profanity_mode: Option<String>,
}

impl CertModerationPolicy {
    pub(crate) fn none() -> Self {
        Self {
            enabled: None,
            pii_mode: None,
            profanity_mode: None,
        }
    }
}

/// Certificate-driven tool approval list.
///
/// When a parent-signed certificate includes an `approval_tools` field,
/// those tools require manual approval before execution — regardless of
/// the local `[tool_approval]` config. This lets a parent enforce
/// human-in-the-loop controls that the agent cannot override.
pub struct CertToolApproval {
    /// Comma-separated tool names that require approval (from cert field `approval_tools`).
    /// `None` means the certificate does not specify any approval requirements.
    pub tools: Option<Vec<String>>,
}

impl CertToolApproval {
    pub fn none() -> Self {
        Self { tools: None }
    }
}

/// Certificate-driven tag enforcement policy.
pub struct CertTagPolicy {
    /// When `Some(true)`, all tasks MUST include at least one tag.
    pub tags_required: Option<bool>,
}

impl CertTagPolicy {
    pub(crate) fn none() -> Self {
        Self {
            tags_required: None,
        }
    }
}

/// Certificate status — whether the agent holds a parent-signed, self-signed, or no cert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateStatus {
    /// "parent-signed", "self-signed", or "none"
    pub status: String,
    /// The certificate value, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<Value>,
    /// The agent's identity key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_key: Option<String>,
}

/// Result of the boot-time certificate check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertCheckResult {
    /// Valid, non-revoked parent-signed certificate.
    Valid,
    /// Certificate exists but has been revoked.
    Revoked,
    /// No certificate present.
    None,
}
