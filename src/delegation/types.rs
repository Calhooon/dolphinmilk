//! Core types for scoped cross-agent delegation certificates.
//!
//! See `docs/DELEGATION-DESIGN.md` §3 for the full schema.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Well-known certificate type for delegation certs.
pub const CERT_TYPE_DELEGATION: &str = "agent-delegation";

/// Current schema version. Verifiers reject certs with a different version.
pub const DELEGATION_VERSION_V1: &str = "1";

/// Maximum cert chain depth for multi-hop re-delegation.
/// Depth 1 = single hop (root delegation).
/// Depth 5 = four re-delegations.
pub const MAX_CHAIN_DEPTH: u8 = 5;

/// Clock skew tolerance (seconds) for `issued_at` checks.
pub const CLOCK_SKEW_TOLERANCE_SECS: i64 = 60;

/// BRC-77 protocol ID used when signing/verifying delegation certs.
/// Deliberately distinct from the existing `[2, "certificate signing"]`
/// used for parent→agent cert issuance, to prevent verify-path collisions.
pub fn delegation_protocol_id() -> Value {
    json!([2, "agent delegation v1"])
}

/// A parsed delegation certificate. Strongly typed view of the JSON wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationCert {
    // ── BRC-52 base fields ────────────────────────────────────────
    /// 66-hex compressed pubkey of the recipient (subject of the delegation).
    pub subject: String,
    /// 66-hex compressed pubkey of the issuer (signer of this cert).
    pub certifier: String,
    /// Unique serial, format: `delegation-<issuer_name>-<task_prefix>-<unix_ms>`.
    pub serial_number: String,
    /// Revocation UTXO outpoint (`<txid>.<vout>` or 72-hex format).
    pub revocation_outpoint: String,
    /// Hex-encoded BRC-77 signature over the canonical body (see canonicalize.rs).
    pub signature: String,

    // ── Delegation-specific fields (stored in BRC-52 `fields` map) ──
    /// Schema version. Must equal `DELEGATION_VERSION_V1`.
    pub version: String,
    /// Tool names the subject may invoke while running the task.
    pub capabilities: Vec<String>,
    /// Optional per-tool argument scoping. Map from tool name to allowed arg prefixes.
    /// Stored on-wire as `"tool:arg1,arg2;tool2:arg1"` flat string.
    pub capability_args: Option<BTreeMap<String, Vec<String>>>,
    /// Hard cap on sats the subject may spend on this task.
    pub budget_cap_sats: u64,
    /// RFC3339 UTC timestamp past which the cert is invalid.
    pub expires_at: DateTime<Utc>,
    /// Purpose binding — `"sha256:<64hex>"` of the task description.
    pub purpose_hash: String,
    /// Optional payment terms (informational — no on-chain enforcement from the cert alone).
    pub payment: Option<PaymentTerms>,
    /// Hash of the parent cert in the chain, if this is a re-delegation.
    /// Format: `"sha256:<64hex>"`. Absent or empty = root delegation.
    pub parent_cert_hash: Option<String>,
    /// 66-hex identity key of the trust root (the certifier at the top of the chain).
    /// Must be in the subject's `trust.certifiers` config set for the cert to be accepted.
    pub root_certifier: String,
    /// RFC3339 UTC timestamp when the cert was signed.
    pub issued_at: DateTime<Utc>,
}

/// Optional payment terms embedded in a delegation cert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentTerms {
    pub amount_per_unit: u64,
    pub unit: String,
    pub max_total: u64,
    pub derivation_invoice: String,
}

/// Result of a successful delegation verification.
#[derive(Debug, Clone)]
pub struct VerifiedDelegation {
    /// The leaf (most-specific) cert in the chain — the one that directly applies to this task.
    pub cert: DelegationCert,
    /// The effective capability set after intersecting with parent chain caveats.
    pub effective_capabilities: Vec<String>,
    /// The effective capability-arg scoping after intersecting with parent chain caveats.
    pub effective_capability_args: Option<BTreeMap<String, Vec<String>>>,
    /// The effective budget cap after taking the minimum of all chain entries.
    pub effective_budget_cap_sats: u64,
    /// The effective expiry after taking the earliest of all chain entries.
    pub effective_expires_at: DateTime<Utc>,
    /// Chain depth (1 = single hop, 2 = one re-delegation, etc.).
    pub chain_depth: u8,
    /// The root certifier (terminal entry in the chain).
    pub root_certifier: String,
}

/// Errors produced by delegation verification.
#[derive(Debug, thiserror::Error)]
pub enum DelegationError {
    #[error("malformed cert: {0}")]
    Malformed(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("unsupported delegation version: expected {expected}, got {got}")]
    UnsupportedVersion { expected: String, got: String },

    #[error("subject mismatch: cert is for {got}, I am {expected}")]
    WrongSubject { expected: String, got: String },

    #[error("signature verification failed")]
    InvalidSignature,

    #[error("unknown root certifier: {0} not in trust.certifiers")]
    UnknownRootCertifier(String),

    #[error("cert expired at {expired_at}")]
    Expired { expired_at: DateTime<Utc> },

    #[error(
        "cert not yet valid (issued_at={issued_at} is in the future beyond clock skew tolerance)"
    )]
    NotYetValid { issued_at: DateTime<Utc> },

    #[error("purpose hash mismatch: cert={cert}, task={task}")]
    PurposeMismatch { cert: String, task: String },

    #[error("cert revoked: outpoint {outpoint} has been spent")]
    Revoked { outpoint: String },

    #[error("chain too deep: {depth} exceeds max {max}")]
    ChainTooDeep { depth: u8, max: u8 },

    #[error("narrowing violation: {0}")]
    NarrowingViolation(String),

    #[error("parent cert hash mismatch")]
    ParentHashMismatch,

    #[error("parent cert required by parent_cert_hash field but not provided in chain")]
    ParentCertMissing,

    #[error("wallet error during verification: {0}")]
    Wallet(String),

    #[error("verifier error: {0}")]
    Other(String),
}

/// Runtime context attached to a task that was spawned via a delegation.
/// Lives on `LoopState` (added in Phase 3) and drives tool/budget sandboxing.
#[derive(Debug, Clone)]
pub struct DelegationContext {
    pub verified: VerifiedDelegation,
    /// Full chain, oldest-first (root cert at index 0).
    pub chain: Vec<DelegationCert>,
}

// ── Parsing helpers ───────────────────────────────────────────────

impl DelegationCert {
    /// Parse a delegation cert from its JSON wire form.
    ///
    /// Accepts the BRC-52 envelope with `certificateType == "agent-delegation"`
    /// and extracts the delegation-specific fields from the `fields` map.
    pub fn from_value(v: &Value) -> Result<Self, DelegationError> {
        let obj = v
            .as_object()
            .ok_or_else(|| DelegationError::Malformed("cert is not a JSON object".into()))?;

        // Base BRC-52 fields
        let cert_type = obj
            .get("certificateType")
            .or_else(|| obj.get("type"))
            .and_then(|v| v.as_str())
            .ok_or(DelegationError::MissingField("certificateType"))?;
        if cert_type != CERT_TYPE_DELEGATION {
            return Err(DelegationError::Malformed(format!(
                "not a delegation cert: type={cert_type}"
            )));
        }

        let subject = require_str(obj, "subject")?.to_string();
        let certifier = require_str(obj, "certifier")?.to_string();
        let serial_number = require_str(obj, "serialNumber")?.to_string();
        let revocation_outpoint = require_str(obj, "revocationOutpoint")?.to_string();
        let signature = require_str(obj, "signature")?.to_string();

        // Delegation-specific fields from the `fields` map
        let fields = obj
            .get("fields")
            .and_then(|v| v.as_object())
            .ok_or(DelegationError::MissingField("fields"))?;

        let version = require_fields_str(fields, "delegation_version")?.to_string();
        let capabilities = require_fields_str(fields, "delegation_capabilities")?
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let capability_args = fields
            .get("delegation_capability_args")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(parse_capability_args)
            .transpose()?;
        let budget_cap_sats = require_fields_str(fields, "delegation_budget_cap_sats")?
            .parse::<u64>()
            .map_err(|e| DelegationError::Malformed(format!("budget_cap_sats: {e}")))?;
        let expires_at = parse_rfc3339(require_fields_str(fields, "delegation_expires_at")?)
            .map_err(|e| DelegationError::Malformed(format!("expires_at: {e}")))?;
        let purpose_hash = require_fields_str(fields, "delegation_purpose_hash")?.to_string();
        let parent_cert_hash = fields
            .get("delegation_parent_cert_hash")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let root_certifier = require_fields_str(fields, "delegation_root_certifier")?.to_string();
        let issued_at = parse_rfc3339(require_fields_str(fields, "delegation_issued_at")?)
            .map_err(|e| DelegationError::Malformed(format!("issued_at: {e}")))?;

        // Optional payment terms
        let payment = parse_payment_terms(fields)?;

        Ok(Self {
            subject,
            certifier,
            serial_number,
            revocation_outpoint,
            signature,
            version,
            capabilities,
            capability_args,
            budget_cap_sats,
            expires_at,
            purpose_hash,
            payment,
            parent_cert_hash,
            root_certifier,
            issued_at,
        })
    }

    /// Serialize back to the BRC-52 wire form.
    pub fn to_value(&self) -> Value {
        let mut fields = serde_json::Map::new();
        fields.insert("delegation_version".into(), json!(self.version));
        fields.insert(
            "delegation_capabilities".into(),
            json!(self.capabilities.join(",")),
        );
        if let Some(ref args) = self.capability_args {
            fields.insert(
                "delegation_capability_args".into(),
                json!(serialize_capability_args(args)),
            );
        }
        fields.insert(
            "delegation_budget_cap_sats".into(),
            json!(self.budget_cap_sats.to_string()),
        );
        fields.insert(
            "delegation_expires_at".into(),
            json!(self.expires_at.to_rfc3339()),
        );
        fields.insert("delegation_purpose_hash".into(), json!(self.purpose_hash));
        if let Some(ref p) = self.payment {
            fields.insert(
                "delegation_payment_amount_per_unit".into(),
                json!(p.amount_per_unit.to_string()),
            );
            fields.insert("delegation_payment_unit".into(), json!(p.unit));
            fields.insert(
                "delegation_payment_max_total".into(),
                json!(p.max_total.to_string()),
            );
            fields.insert(
                "delegation_payment_derivation_invoice".into(),
                json!(p.derivation_invoice),
            );
        }
        if let Some(ref h) = self.parent_cert_hash {
            fields.insert("delegation_parent_cert_hash".into(), json!(h));
        }
        fields.insert(
            "delegation_root_certifier".into(),
            json!(self.root_certifier),
        );
        fields.insert(
            "delegation_issued_at".into(),
            json!(self.issued_at.to_rfc3339()),
        );

        json!({
            "certificateType": CERT_TYPE_DELEGATION,
            "type": CERT_TYPE_DELEGATION,
            "subject": self.subject,
            "certifier": self.certifier,
            "serialNumber": self.serial_number,
            "revocationOutpoint": self.revocation_outpoint,
            "signature": self.signature,
            "fields": fields,
            "acquisitionProtocol": "direct",
        })
    }
}

// ── Private parsing helpers ───────────────────────────────────────

fn require_str<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, DelegationError> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .ok_or(DelegationError::MissingField(key))
}

fn require_fields_str<'a>(
    fields: &'a serde_json::Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, DelegationError> {
    fields
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or(DelegationError::MissingField(key))
}

fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc))
}

/// Parse the capability_args wire format: `tool1:arg1,arg2;tool2:arg1`
/// into a sorted map `{tool1: [arg1, arg2], tool2: [arg1]}`.
pub(crate) fn parse_capability_args(
    s: &str,
) -> Result<BTreeMap<String, Vec<String>>, DelegationError> {
    let mut out = BTreeMap::new();
    for entry in s.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (tool, args) = entry.split_once(':').ok_or_else(|| {
            DelegationError::Malformed(format!("capability_args entry missing colon: {entry}"))
        })?;
        let tool = tool.trim().to_string();
        if tool.is_empty() {
            return Err(DelegationError::Malformed(
                "capability_args entry has empty tool name".into(),
            ));
        }
        let args: Vec<String> = args
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        out.insert(tool, args);
    }
    Ok(out)
}

/// Serialize the capability_args map back to wire format.
/// Output is sorted by tool name (BTreeMap iteration order) and args are sorted
/// within each tool — makes the canonical form deterministic.
pub(crate) fn serialize_capability_args(map: &BTreeMap<String, Vec<String>>) -> String {
    let mut parts = Vec::new();
    for (tool, args) in map {
        let mut sorted_args = args.clone();
        sorted_args.sort();
        parts.push(format!("{}:{}", tool, sorted_args.join(",")));
    }
    parts.join(";")
}

fn parse_payment_terms(
    fields: &serde_json::Map<String, Value>,
) -> Result<Option<PaymentTerms>, DelegationError> {
    let has_any = fields.contains_key("delegation_payment_amount_per_unit")
        || fields.contains_key("delegation_payment_unit")
        || fields.contains_key("delegation_payment_max_total")
        || fields.contains_key("delegation_payment_derivation_invoice");
    if !has_any {
        return Ok(None);
    }
    let amount_per_unit = require_fields_str(fields, "delegation_payment_amount_per_unit")?
        .parse::<u64>()
        .map_err(|e| DelegationError::Malformed(format!("payment.amount_per_unit: {e}")))?;
    let unit = require_fields_str(fields, "delegation_payment_unit")?.to_string();
    let max_total = require_fields_str(fields, "delegation_payment_max_total")?
        .parse::<u64>()
        .map_err(|e| DelegationError::Malformed(format!("payment.max_total: {e}")))?;
    let derivation_invoice =
        require_fields_str(fields, "delegation_payment_derivation_invoice")?.to_string();
    Ok(Some(PaymentTerms {
        amount_per_unit,
        unit,
        max_total,
        derivation_invoice,
    }))
}

// ── Unit tests for parsing helpers ────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_id_is_stable() {
        let pid = delegation_protocol_id();
        assert_eq!(pid, json!([2, "agent delegation v1"]));
    }

    #[test]
    fn parse_capability_args_basic() {
        let parsed =
            parse_capability_args("web_fetch:reddit.com,hn.com;x402_call:api.bsv").unwrap();
        assert_eq!(
            parsed.get("web_fetch").unwrap(),
            &vec!["reddit.com", "hn.com"]
        );
        assert_eq!(parsed.get("x402_call").unwrap(), &vec!["api.bsv"]);
    }

    #[test]
    fn parse_capability_args_empty() {
        let parsed = parse_capability_args("").unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_capability_args_missing_colon() {
        let err = parse_capability_args("web_fetch").unwrap_err();
        assert!(matches!(err, DelegationError::Malformed(_)));
    }

    #[test]
    fn serialize_capability_args_sorted() {
        let mut map = BTreeMap::new();
        map.insert(
            "web_fetch".to_string(),
            vec!["hn.com".to_string(), "reddit.com".to_string()],
        );
        map.insert("x402_call".to_string(), vec!["api.bsv".to_string()]);
        let s = serialize_capability_args(&map);
        // BTreeMap iterates in sorted order, and args are sorted inside
        assert_eq!(s, "web_fetch:hn.com,reddit.com;x402_call:api.bsv");
    }

    #[test]
    fn cert_round_trip() {
        let cert = make_test_cert();
        let v = cert.to_value();
        let parsed = DelegationCert::from_value(&v).unwrap();
        assert_eq!(cert, parsed);
    }

    #[test]
    fn rejects_wrong_cert_type() {
        let mut v = make_test_cert().to_value();
        v.as_object_mut()
            .unwrap()
            .insert("certificateType".into(), json!("agent-authorization"));
        let err = DelegationCert::from_value(&v).unwrap_err();
        assert!(matches!(err, DelegationError::Malformed(_)));
    }

    #[test]
    fn rejects_missing_subject() {
        let mut v = make_test_cert().to_value();
        v.as_object_mut().unwrap().remove("subject");
        let err = DelegationCert::from_value(&v).unwrap_err();
        assert!(matches!(err, DelegationError::MissingField("subject")));
    }

    #[test]
    fn rejects_missing_capabilities() {
        let mut v = make_test_cert().to_value();
        v.as_object_mut()
            .unwrap()
            .get_mut("fields")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("delegation_capabilities");
        let err = DelegationCert::from_value(&v).unwrap_err();
        assert!(matches!(
            err,
            DelegationError::MissingField("delegation_capabilities")
        ));
    }

    fn make_test_cert() -> DelegationCert {
        DelegationCert {
            subject: "02".repeat(33),
            certifier: "03".repeat(33),
            serial_number: "delegation-test-aa-1".into(),
            revocation_outpoint: "aa".repeat(36),
            signature: "ab".repeat(32),
            version: DELEGATION_VERSION_V1.into(),
            capabilities: vec!["web_fetch".into(), "memory_store".into()],
            capability_args: None,
            budget_cap_sats: 50_000,
            expires_at: Utc::now() + chrono::Duration::minutes(30),
            purpose_hash: format!("sha256:{}", "cd".repeat(32)),
            payment: None,
            parent_cert_hash: None,
            root_certifier: "03".repeat(33),
            issued_at: Utc::now(),
        }
    }
}
