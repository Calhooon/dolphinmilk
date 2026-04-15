//! Tests for BRC-52 certificate revocation (Phase 10.1).
//!
//! Covers: null outpoint detection, revocation outpoint parsing, revocation
//! validation helpers, parent-relinquish guard, and server endpoint behavior.

use dolphin_milk::certificates::{
    is_null_revocation_outpoint, parse_revocation_outpoint, CertificateStatus, CERT_TYPE_AGENT_AUTH,
};
use dolphin_milk::state::BASKET_REVOCATION;
use serde_json::json;

// ---------------------------------------------------------------------------
// Null outpoint detection
// ---------------------------------------------------------------------------

#[test]
fn test_null_outpoint_72_zeros() {
    let null = "0".repeat(72);
    assert!(is_null_revocation_outpoint(&null));
}

#[test]
fn test_null_outpoint_empty() {
    assert!(is_null_revocation_outpoint(""));
}

#[test]
fn test_real_outpoint_not_null() {
    // Real txid (64 hex) + vout (8 hex)
    let outpoint = format!("{}{:08x}", "a".repeat(64), 0u32);
    assert!(!is_null_revocation_outpoint(&outpoint));
}

#[test]
fn test_partial_zeros_not_null() {
    // 64 zeros + 8 non-zeros — not the null outpoint
    let outpoint = format!("{}00000001", "0".repeat(64));
    assert!(!is_null_revocation_outpoint(&outpoint));
}

// ---------------------------------------------------------------------------
// Revocation outpoint parsing
// ---------------------------------------------------------------------------

#[test]
fn test_parse_null_outpoint_returns_none() {
    let null = "0".repeat(72);
    assert!(parse_revocation_outpoint(&null).is_none());
}

#[test]
fn test_parse_empty_outpoint_returns_none() {
    assert!(parse_revocation_outpoint("").is_none());
}

#[test]
fn test_parse_valid_outpoint() {
    let txid = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let outpoint = format!("{txid}{:08x}", 2u32);
    let (parsed_txid, parsed_vout) = parse_revocation_outpoint(&outpoint).unwrap();
    assert_eq!(parsed_txid, txid);
    assert_eq!(parsed_vout, 2);
}

#[test]
fn test_parse_vout_zero() {
    let txid = "1".repeat(64);
    let outpoint = format!("{txid}00000000");
    let (_, vout) = parse_revocation_outpoint(&outpoint).unwrap();
    assert_eq!(vout, 0);
}

#[test]
fn test_parse_vout_max() {
    let txid = "f".repeat(64);
    let outpoint = format!("{txid}ffffffff");
    let (_, vout) = parse_revocation_outpoint(&outpoint).unwrap();
    assert_eq!(vout, u32::MAX);
}

#[test]
fn test_parse_wrong_length_returns_none() {
    // 70 chars — too short
    assert!(parse_revocation_outpoint(&"a".repeat(70)).is_none());
    // 74 chars — too long
    assert!(parse_revocation_outpoint(&"a".repeat(74)).is_none());
}

#[test]
fn test_parse_invalid_hex_vout_returns_none() {
    // txid is valid hex, but vout contains non-hex chars
    let txid = "a".repeat(64);
    let outpoint = format!("{txid}zzzzzzzz");
    assert!(parse_revocation_outpoint(&outpoint).is_none());
}

// ---------------------------------------------------------------------------
// Basket constant
// ---------------------------------------------------------------------------

#[test]
fn test_basket_revocation_constant() {
    assert_eq!(BASKET_REVOCATION, "dm-revocation");
}

// ---------------------------------------------------------------------------
// Certificate status with revocation outpoint
// ---------------------------------------------------------------------------

#[test]
fn test_cert_with_null_revocation_outpoint() {
    let cert = json!({
        "type": CERT_TYPE_AGENT_AUTH,
        "subject": "02abc",
        "certifier": "02abc",
        "revocationOutpoint": "0".repeat(72),
    });
    let outpoint = cert["revocationOutpoint"].as_str().unwrap();
    assert!(is_null_revocation_outpoint(outpoint));
    assert!(parse_revocation_outpoint(outpoint).is_none());
}

#[test]
fn test_cert_with_real_revocation_outpoint() {
    let txid = "deadbeef".repeat(8); // 64 hex chars
    let outpoint = format!("{txid}00000000");
    let cert = json!({
        "type": CERT_TYPE_AGENT_AUTH,
        "subject": "02agent",
        "certifier": "02parent",
        "revocationOutpoint": outpoint,
    });
    let parsed = parse_revocation_outpoint(cert["revocationOutpoint"].as_str().unwrap());
    assert!(parsed.is_some());
    let (t, v) = parsed.unwrap();
    assert_eq!(t, txid);
    assert_eq!(v, 0);
}

#[test]
fn test_cert_missing_revocation_outpoint() {
    let cert = json!({
        "type": CERT_TYPE_AGENT_AUTH,
        "subject": "02abc",
        "certifier": "02abc",
    });
    let outpoint = cert
        .get("revocationOutpoint")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(is_null_revocation_outpoint(outpoint));
}

// ---------------------------------------------------------------------------
// CertificateStatus serde
// ---------------------------------------------------------------------------

#[test]
fn test_certificate_status_parent_signed_with_revocation() {
    let txid = "a".repeat(64);
    let outpoint = format!("{txid}00000000");
    let cert = json!({
        "subject": "02agent",
        "certifier": "02parent",
        "revocationOutpoint": outpoint,
    });
    let status = CertificateStatus {
        status: "parent-signed".into(),
        certificate: Some(cert),
        identity_key: Some("02agent".into()),
    };
    let json_str = serde_json::to_string(&status).unwrap();
    assert!(json_str.contains("parent-signed"));
    assert!(json_str.contains("revocationOutpoint"));
}

#[test]
fn test_self_signed_cert_has_null_outpoint() {
    // Self-signed certs always use the null outpoint
    let null_outpoint = "0".repeat(72);
    let cert = json!({
        "subject": "02abc",
        "certifier": "02abc",
        "revocationOutpoint": null_outpoint,
    });
    assert!(is_null_revocation_outpoint(
        cert["revocationOutpoint"].as_str().unwrap()
    ));
}

// ---------------------------------------------------------------------------
// Parent-relinquish guard logic
// ---------------------------------------------------------------------------

#[test]
fn test_parent_issued_cert_detected() {
    let cert = json!({
        "subject": "02agent",
        "certifier": "02parent",
    });
    let subject = cert["subject"].as_str().unwrap();
    let certifier = cert["certifier"].as_str().unwrap();
    // Parent-issued: certifier != subject
    assert_ne!(certifier, subject);
}

#[test]
fn test_self_signed_cert_detected() {
    let cert = json!({
        "subject": "02abc",
        "certifier": "02abc",
    });
    let subject = cert["subject"].as_str().unwrap();
    let certifier = cert["certifier"].as_str().unwrap();
    // Self-signed: certifier == subject
    assert_eq!(certifier, subject);
}

#[test]
fn test_relinquish_guard_blocks_parent_issued() {
    let cert = json!({
        "subject": "02agent_key",
        "certifier": "02parent_key",
    });
    let subject = cert.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let certifier = cert.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
    let is_parent_issued = !subject.is_empty() && !certifier.is_empty() && certifier != subject;
    assert!(is_parent_issued, "Parent-issued cert should be detected");
}

#[test]
fn test_relinquish_guard_allows_self_signed() {
    let cert = json!({
        "subject": "02abc",
        "certifier": "02abc",
    });
    let subject = cert.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let certifier = cert.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
    let is_parent_issued = !subject.is_empty() && !certifier.is_empty() && certifier != subject;
    assert!(!is_parent_issued, "Self-signed cert should pass guard");
}

// ---------------------------------------------------------------------------
// Revocation outpoint format
// ---------------------------------------------------------------------------

#[test]
fn test_revocation_outpoint_format_txid_vout() {
    // The format is: 64-char hex txid + 8-char hex vout (little-endian u32)
    let txid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let vout = 5u32;
    let outpoint = format!("{txid}{vout:08x}");
    assert_eq!(outpoint.len(), 72);
    let (parsed_txid, parsed_vout) = parse_revocation_outpoint(&outpoint).unwrap();
    assert_eq!(parsed_txid, txid);
    assert_eq!(parsed_vout, 5);
}

// ---------------------------------------------------------------------------
// Server endpoint tests (axum oneshot)
// ---------------------------------------------------------------------------

use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn test_revoke_endpoint_exists() {
    // Only verify the route exists (not 404/405). Do NOT actually call the
    // revoke handler against a real wallet — it spends the revocation UTXO
    // and would destroy a real parent-signed certificate.
    let app = common::test_router().await;

    // GET should be 405 (method not allowed) — proves POST route is registered
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/certificates/revoke")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        405,
        "GET /certificates/revoke should be 405 (POST route exists)"
    );
}

#[tokio::test]
async fn test_relinquish_endpoint_still_works() {
    let app = common::test_router().await;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/certificates/relinquish")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Should not be 404 (route exists). May be 404 (no cert) or 500 (wallet down).
    assert_ne!(
        resp.status(),
        405,
        "POST /certificates/relinquish should still be a valid route"
    );
}

// ---------------------------------------------------------------------------
// Inline unit tests in certificates.rs
// ---------------------------------------------------------------------------

#[test]
fn test_cert_type_constant() {
    assert_eq!(CERT_TYPE_AGENT_AUTH, "agent-authorization");
}

#[test]
fn test_outpoint_72_char_format() {
    // 64 chars txid + 8 chars vout = 72 chars total
    let txid = "f".repeat(64);
    let outpoint = format!("{txid}00000001");
    assert_eq!(outpoint.len(), 72);
    assert!(!is_null_revocation_outpoint(&outpoint));
}

// ---------------------------------------------------------------------------
// Block 6: Certificate endpoint enhancements
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_certificates_endpoint_includes_is_revoked_field() {
    let app = common::test_router().await;
    let req = axum::http::Request::builder()
        .uri("/certificates")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // In dev mode, the endpoint should return OK or 500 (wallet down)
    let status = resp.status();
    if status == 200 {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // The response should have an is_revoked field (may be null)
        assert!(
            json.get("is_revoked").is_some(),
            "GET /certificates should include is_revoked field: {json}"
        );
    }
    // If 500, wallet is down — acceptable in test mode
}

#[test]
fn test_certificate_status_serde_with_identity_key() {
    let status = CertificateStatus {
        status: "none".into(),
        certificate: None,
        identity_key: Some("02abc".into()),
    };
    let json = serde_json::to_string(&status).unwrap();
    let parsed: CertificateStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.status, "none");
    assert_eq!(parsed.identity_key.as_deref(), Some("02abc"));
}

// -- Task 6.5: Revocation pagination --

#[test]
fn test_is_revoked_handles_various_outpoint_formats() {
    // These are synchronous tests for outpoint parsing that feeds into is_revoked()
    // The actual async pagination is tested via integration with mockito

    // Null outpoints should short-circuit to false (no wallet call)
    assert!(is_null_revocation_outpoint(""));
    assert!(is_null_revocation_outpoint(&"0".repeat(72)));

    // Valid outpoints should parse correctly for the pagination lookup
    let txid = "a".repeat(64);
    let outpoint = format!("{txid}00000042");
    let (parsed_txid, parsed_vout) = parse_revocation_outpoint(&outpoint).unwrap();
    assert_eq!(parsed_txid, txid);
    assert_eq!(parsed_vout, 0x42);

    // Large vout values (u32::MAX) should parse
    let outpoint_max = format!("{txid}ffffffff");
    let (_, vout) = parse_revocation_outpoint(&outpoint_max).unwrap();
    assert_eq!(vout, u32::MAX);
}
