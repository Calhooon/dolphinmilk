//! Deterministic JSON serialization for delegation cert signing/verification.
//!
//! See `docs/DELEGATION-DESIGN.md` §4.1 for the canonicalization spec:
//!
//! 1. JSON with keys sorted alphabetically at every level.
//! 2. No whitespace (compact JSON).
//! 3. The `signature` field excluded entirely from the signed body.
//! 4. UTF-8 encoding, LF terminator.
//!
//! This must be bit-exact deterministic: issuer and verifier MUST produce
//! identical bytes for the same logical cert, regardless of source field ordering
//! or whitespace.

use super::types::DelegationError;
use serde_json::{Map, Value};

/// Produce the canonical byte string used as the signing/verification input
/// for a delegation certificate.
///
/// The `signature` field is always excluded from the output — the signature
/// is computed OVER the canonical body, so it cannot be included in its own input.
///
/// Returns raw bytes (sorted-keys compact JSON + LF terminator).
pub fn canonicalize_cert(cert: &Value) -> Result<Vec<u8>, DelegationError> {
    let stripped = strip_signature(cert);
    let sorted = sort_json_keys(&stripped);
    let mut bytes = serde_json::to_vec(&sorted)
        .map_err(|e| DelegationError::Other(format!("canonicalize serialize: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Compute the SHA-256 hash of a cert's canonical body.
/// Used for chain linking via `delegation_parent_cert_hash`.
///
/// Returns `"sha256:<64hex>"`.
pub fn compute_cert_hash(cert: &Value) -> Result<String, DelegationError> {
    use sha2::{Digest, Sha256};
    let canonical = canonicalize_cert(cert)?;
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    let digest = hasher.finalize();
    Ok(format!("sha256:{}", hex::encode(digest)))
}

/// Compute a purpose hash from a task description.
///
/// Applies the same normalization on both issuer and verifier sides so the
/// hash is bit-exact across agents. Normalization: trim leading/trailing
/// whitespace, then SHA-256 of UTF-8 bytes.
pub fn compute_purpose_hash(task_description: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = task_description.trim();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    format!("sha256:{}", hex::encode(digest))
}

// ── Internals ──────────────────────────────────────────────────────

fn strip_signature(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, val) in m {
                if k == "signature" {
                    continue;
                }
                out.insert(k.clone(), strip_signature(val));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(strip_signature).collect()),
        other => other.clone(),
    }
}

fn sort_json_keys(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            // BTreeMap gives us sorted-key iteration.
            let sorted: std::collections::BTreeMap<String, Value> = m
                .iter()
                .map(|(k, v)| (k.clone(), sort_json_keys(v)))
                .collect();
            // Convert BTreeMap back to serde_json::Map (which preserves insertion order).
            let mut out = Map::new();
            for (k, v) in sorted {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sort_json_keys).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_form_is_deterministic() {
        let a = json!({
            "b": 1,
            "a": 2,
            "nested": { "z": 9, "y": 8 }
        });
        let b = json!({
            "nested": { "y": 8, "z": 9 },
            "a": 2,
            "b": 1
        });
        assert_eq!(
            canonicalize_cert(&a).unwrap(),
            canonicalize_cert(&b).unwrap()
        );
    }

    #[test]
    fn canonical_form_strips_signature() {
        let signed = json!({
            "subject": "abc",
            "signature": "deadbeef",
            "fields": { "x": 1 }
        });
        let unsigned = json!({
            "subject": "abc",
            "fields": { "x": 1 }
        });
        assert_eq!(
            canonicalize_cert(&signed).unwrap(),
            canonicalize_cert(&unsigned).unwrap(),
            "signature field must not affect canonical form"
        );
    }

    #[test]
    fn canonical_form_strips_nested_signature() {
        // Paranoia check: a `signature` nested inside an object should also be stripped.
        // This isn't expected in real certs, but the recursive strip should handle it.
        let a = json!({
            "outer": { "signature": "nope", "x": 1 }
        });
        let b = json!({
            "outer": { "x": 1 }
        });
        assert_eq!(
            canonicalize_cert(&a).unwrap(),
            canonicalize_cert(&b).unwrap()
        );
    }

    #[test]
    fn canonical_form_ends_with_lf() {
        let v = json!({"x": 1});
        let bytes = canonicalize_cert(&v).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
    }

    #[test]
    fn canonical_form_is_compact_no_whitespace() {
        let v = json!({"a": 1, "b": "hello"});
        let bytes = canonicalize_cert(&v).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        // No spaces around colons or commas in compact JSON
        assert!(!s.contains(": "));
        assert!(!s.contains(", "));
    }

    #[test]
    fn canonical_form_sorts_all_levels() {
        let v = json!({
            "c": { "y": 2, "x": 1 },
            "a": [1, 2, 3],
            "b": "hello"
        });
        let bytes = canonicalize_cert(&v).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        // Top-level keys sorted
        let a_pos = s.find("\"a\"").unwrap();
        let b_pos = s.find("\"b\"").unwrap();
        let c_pos = s.find("\"c\"").unwrap();
        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
        // Nested keys sorted
        let x_pos = s.find("\"x\"").unwrap();
        let y_pos = s.find("\"y\"").unwrap();
        assert!(x_pos < y_pos);
    }

    #[test]
    fn canonical_form_preserves_array_order() {
        let a = json!({ "list": [3, 1, 2] });
        let b = json!({ "list": [3, 1, 2] });
        assert_eq!(
            canonicalize_cert(&a).unwrap(),
            canonicalize_cert(&b).unwrap()
        );
        let different = json!({ "list": [1, 2, 3] });
        assert_ne!(
            canonicalize_cert(&a).unwrap(),
            canonicalize_cert(&different).unwrap(),
            "array element order is meaningful"
        );
    }

    #[test]
    fn cert_hash_is_deterministic() {
        let a = json!({
            "b": 1,
            "a": 2,
            "signature": "ignore"
        });
        let b = json!({
            "a": 2,
            "b": 1,
            "signature": "different"
        });
        let ha = compute_cert_hash(&a).unwrap();
        let hb = compute_cert_hash(&b).unwrap();
        assert_eq!(ha, hb);
        assert!(ha.starts_with("sha256:"));
        assert_eq!(ha.len(), "sha256:".len() + 64);
    }

    #[test]
    fn purpose_hash_trims_whitespace() {
        let a = compute_purpose_hash("fetch reddit top 10");
        let b = compute_purpose_hash("   fetch reddit top 10\n\n");
        assert_eq!(a, b);
    }

    #[test]
    fn purpose_hash_is_sensitive_to_content() {
        let a = compute_purpose_hash("fetch reddit top 10");
        let b = compute_purpose_hash("fetch reddit top 11");
        assert_ne!(a, b);
    }

    #[test]
    fn purpose_hash_format() {
        let h = compute_purpose_hash("hello");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
        // Reference SHA-256 of "hello" (already trimmed)
        assert_eq!(
            h,
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
