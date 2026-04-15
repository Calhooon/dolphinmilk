//! Revocation checking via on-chain UTXO status.
//!
//! See `DELEGATION-DESIGN.md` §4.2 and §6 for the full revocation semantics.
//!
//! **Key departure from existing BRC-52 code:** `src/certificates/lifecycle.rs` has
//! an `is_revoked()` that paginates through the agent's OWN `worm-revocation` basket.
//! That only works for certs issued to self. For peer delegation certs, we cannot
//! scan the peer's basket — the revocation UTXO lives in whatever basket the
//! issuer keeps, and we don't have access.
//!
//! Instead, we check the outpoint's spent-status directly on-chain. The UTXO is
//! public: if it's still unspent, the cert is valid; if it's been spent, revoked.
//!
//! **Phase 1 scope:** this module defines the trait and a simple implementation
//! that delegates to a provided async closure. The real wallet-backed implementation
//! is Phase 3 and depends on open question #1 (which `WalletBackend` method to use).
//! For now, Phase 1 ships with:
//!
//! - The [`RevocationChecker`](super::verify::RevocationChecker) trait (defined in `verify.rs`).
//! - [`NullRevocationChecker`] — always reports unspent. Used in tests that don't
//!   care about revocation behavior.
//! - [`StaticRevocationChecker`] — a map of outpoint → spent-bool. Used in tests
//!   that need deterministic revocation results.
//!
//! Phase 3 adds `WalletRevocationChecker` that wraps a `WalletBackend`.

use super::types::DelegationError;
use super::verify::RevocationChecker;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A revocation checker that reports every outpoint as unspent.
///
/// Useful for tests and for deployments where revocation checking is
/// intentionally disabled (not recommended in production).
pub struct NullRevocationChecker;

#[async_trait]
impl RevocationChecker for NullRevocationChecker {
    async fn is_outpoint_spent(&self, _outpoint: &str) -> Result<bool, DelegationError> {
        Ok(false)
    }
}

/// A revocation checker backed by a static in-memory map.
/// The same as [`NullRevocationChecker`] by default; outpoints added via
/// [`Self::mark_spent`] are reported as spent.
///
/// Used by unit tests that want deterministic revocation answers.
#[derive(Default, Clone)]
pub struct StaticRevocationChecker {
    spent: Arc<RwLock<HashMap<String, bool>>>,
}

impl StaticRevocationChecker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark an outpoint as spent. Future `is_outpoint_spent(outpoint)` returns `Ok(true)`.
    pub fn mark_spent(&self, outpoint: impl Into<String>) {
        if let Ok(mut map) = self.spent.write() {
            map.insert(outpoint.into(), true);
        }
    }

    /// Mark an outpoint as unspent (default state). Clears a previous `mark_spent`.
    pub fn mark_unspent(&self, outpoint: impl Into<String>) {
        if let Ok(mut map) = self.spent.write() {
            map.insert(outpoint.into(), false);
        }
    }
}

#[async_trait]
impl RevocationChecker for StaticRevocationChecker {
    async fn is_outpoint_spent(&self, outpoint: &str) -> Result<bool, DelegationError> {
        let map = self
            .spent
            .read()
            .map_err(|e| DelegationError::Wallet(format!("lock poisoned: {e}")))?;
        Ok(*map.get(outpoint).unwrap_or(&false))
    }
}

/// Revocation checker backed by the rust-overlay `ls_dm_delegation` lookup
/// service.
///
/// Phase 3 of EPIC #329 — wires the verifier into the deployed overlay
/// (https://rust-overlay.dev-a3e.workers.dev) where Phase 2's `delegate_task`
/// publishes its revocation UTXOs under `tm_dm_delegation`. The query is:
///
/// ```text
/// POST /lookup
/// { "service": "ls_dm_delegation",
///   "query":   { "findByOutpoint": "<txid>.<vout>" } }
///
/// Response: { "type": "output-list", "outputs": [...] }
/// ```
///
/// Semantics:
/// - **Outputs non-empty** → the UTXO is in the unspent set → **not revoked**.
/// - **Outputs empty** → the UTXO has been spent (or was never indexed) →
///   **revoked**. Both cases are treated identically by Phase 3 v1: the cert
///   is no longer trustable. The "never indexed" subcase happens when the
///   issuer's `delegate_task` failed to submit to the overlay; the issuer can
///   re-publish manually if they want the cert to be queryable.
/// - **Network / wallet error** → `Err(DelegationError::Wallet)`. The
///   per-call enforcement loop in `runner/execute.rs` (Phase 3F) applies the
///   fail-closed-for-paid / fail-open-for-lightweight policy from open
///   question Q-A.
///
/// This struct deliberately does NOT short-circuit Phase 1's `is_revoked()`
/// in `certificates/lifecycle.rs` (which paginates own baskets and is the
/// right answer for own BRC-52 agent-authorization certs). Both checkers
/// coexist; the runner picks `OverlayRevocationChecker` for inbound peer
/// delegation certs.
pub struct OverlayRevocationChecker {
    overlay_url: String,
    http: reqwest::Client,
}

impl OverlayRevocationChecker {
    /// Construct from an overlay base URL (e.g.
    /// `https://rust-overlay.dev-a3e.workers.dev`). Trailing slashes are
    /// trimmed so callers can pass whatever the config holds.
    pub fn new(overlay_url: impl Into<String>) -> Self {
        let url = overlay_url.into().trim_end_matches('/').to_string();
        Self {
            overlay_url: url,
            http: reqwest::Client::new(),
        }
    }

    /// Override the underlying HTTP client (for tests with a mock or short
    /// timeouts).
    #[cfg(test)]
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http = client;
        self
    }
}

#[async_trait]
impl RevocationChecker for OverlayRevocationChecker {
    async fn is_outpoint_spent(&self, outpoint: &str) -> Result<bool, DelegationError> {
        // Parse first so callers can tell parse errors from network errors.
        let (txid, vout) = parse_outpoint(outpoint)?;
        let canonical = format!("{txid}.{vout}");

        let body = serde_json::json!({
            "service": "ls_dm_delegation",
            "query": { "findByOutpoint": canonical },
        });

        let resp = self
            .http
            .post(format!("{}/lookup", self.overlay_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                DelegationError::Wallet(format!(
                    "overlay /lookup network error for {canonical}: {e}"
                ))
            })?;

        let status = resp.status();
        let response_body: serde_json::Value = resp.json().await.map_err(|e| {
            DelegationError::Wallet(format!(
                "overlay /lookup response parse error for {canonical}: {e}"
            ))
        })?;

        if !status.is_success() {
            let msg = response_body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown overlay error");
            return Err(DelegationError::Wallet(format!(
                "overlay /lookup HTTP {status} for {canonical}: {msg}"
            )));
        }

        // Successful response is `{type: "output-list", outputs: [...]}`. If
        // the outputs array is non-empty, the UTXO is still unspent and the
        // cert is valid. Empty array means spent (or never indexed) — both
        // are treated as revoked in v1.
        let outputs = response_body
            .get("outputs")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        Ok(outputs == 0)
    }
}

/// Parse a cert revocation outpoint string into `(txid, vout)`.
///
/// Phase 2's `delegate_task` tool stores outpoints in the canonical
/// `"<txid>.<vout>"` form on each [`super::types::DelegationCert`]. This helper
/// is used by Phase 3 revocation checkers (overlay-backed and otherwise) to
/// turn that wire form into structured fields before querying.
///
/// Three formats are accepted, in priority order:
/// - Canonical `"<64-hex txid>.<vout>"` — the wire form Phase 2 produces.
/// - Bare 64-hex `"<txid>"` — older format, vout assumed `0`.
/// - 72-hex compact `"<txid_64_hex><vout_8_hex_le>"` — legacy BRC-52 form.
///
/// The well-known 72-zero-char null placeholder (used by self-signed certs
/// that have no revocation mechanism) is rejected with an error so callers
/// can short-circuit before contacting the overlay or wallet.
///
/// Returns [`DelegationError::Wallet`] for any malformed input. The error
/// variant is reused so callers can handle parse failures uniformly with
/// real wallet/network errors during a revocation check.
pub fn parse_outpoint(outpoint: &str) -> Result<(String, u32), DelegationError> {
    let outpoint = outpoint.trim();
    if outpoint.is_empty() {
        return Err(DelegationError::Wallet(
            "empty revocation outpoint".to_string(),
        ));
    }
    // Reject the well-known null placeholder (72 zero hex chars).
    if outpoint.len() == 72 && outpoint.bytes().all(|b| b == b'0') {
        return Err(DelegationError::Wallet(
            "null revocation outpoint placeholder is not checkable".to_string(),
        ));
    }

    // Format 1: "txid.vout"
    if let Some((txid, vout_str)) = outpoint.split_once('.') {
        let vout: u32 = vout_str.parse().map_err(|e| {
            DelegationError::Wallet(format!("invalid vout in outpoint '{outpoint}': {e}"))
        })?;
        if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DelegationError::Wallet(format!(
                "invalid txid in outpoint '{outpoint}': must be 64 hex chars"
            )));
        }
        return Ok((txid.to_string(), vout));
    }

    // Format 2: bare 64-hex txid (vout assumed 0)
    if outpoint.len() == 64 && outpoint.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok((outpoint.to_string(), 0));
    }

    // Format 3: 72-hex compact form (txid + 8-hex little-endian vout)
    if outpoint.len() == 72 && outpoint.chars().all(|c| c.is_ascii_hexdigit()) {
        let txid = &outpoint[..64];
        let vout_hex = &outpoint[64..];
        let mut vout_le = [0u8; 4];
        for (i, byte) in vout_le.iter_mut().enumerate() {
            let off = i * 2;
            *byte = u8::from_str_radix(&vout_hex[off..off + 2], 16).map_err(|e| {
                DelegationError::Wallet(format!("invalid 72-hex outpoint '{outpoint}': {e}"))
            })?;
        }
        let vout = u32::from_le_bytes(vout_le);
        return Ok((txid.to_string(), vout));
    }

    Err(DelegationError::Wallet(format!(
        "unrecognized revocation outpoint format: '{outpoint}'"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_checker_reports_unspent() {
        let c = NullRevocationChecker;
        assert!(!c.is_outpoint_spent("anything").await.unwrap());
        assert!(!c.is_outpoint_spent("").await.unwrap());
        assert!(!c.is_outpoint_spent(&"a".repeat(72)).await.unwrap());
    }

    #[tokio::test]
    async fn static_checker_default_reports_unspent() {
        let c = StaticRevocationChecker::new();
        assert!(!c.is_outpoint_spent("abc.0").await.unwrap());
    }

    #[tokio::test]
    async fn static_checker_mark_spent() {
        let c = StaticRevocationChecker::new();
        c.mark_spent("abc.0");
        assert!(c.is_outpoint_spent("abc.0").await.unwrap());
        assert!(!c.is_outpoint_spent("xyz.1").await.unwrap());
    }

    #[tokio::test]
    async fn static_checker_mark_unspent_clears() {
        let c = StaticRevocationChecker::new();
        c.mark_spent("abc.0");
        assert!(c.is_outpoint_spent("abc.0").await.unwrap());
        c.mark_unspent("abc.0");
        assert!(!c.is_outpoint_spent("abc.0").await.unwrap());
    }

    // ── parse_outpoint helper ────────────────────────────────────────

    #[test]
    fn parse_outpoint_canonical_form() {
        let txid = "a".repeat(64);
        let outpoint = format!("{txid}.5");
        let (parsed_txid, parsed_vout) = parse_outpoint(&outpoint).unwrap();
        assert_eq!(parsed_txid, txid);
        assert_eq!(parsed_vout, 5);
    }

    #[test]
    fn parse_outpoint_canonical_zero_vout() {
        let txid = "b".repeat(64);
        let outpoint = format!("{txid}.0");
        let (parsed_txid, parsed_vout) = parse_outpoint(&outpoint).unwrap();
        assert_eq!(parsed_txid, txid);
        assert_eq!(parsed_vout, 0);
    }

    #[test]
    fn parse_outpoint_bare_txid_defaults_vout_zero() {
        let txid = "c".repeat(64);
        let (parsed_txid, parsed_vout) = parse_outpoint(&txid).unwrap();
        assert_eq!(parsed_txid, txid);
        assert_eq!(parsed_vout, 0);
    }

    #[test]
    fn parse_outpoint_72_hex_legacy_form() {
        // 64-hex txid + 8-hex le-encoded vout=3 (03 00 00 00).
        let txid = "d".repeat(64);
        let outpoint = format!("{txid}03000000");
        assert_eq!(outpoint.len(), 72);
        let (parsed_txid, parsed_vout) = parse_outpoint(&outpoint).unwrap();
        assert_eq!(parsed_txid, txid);
        assert_eq!(parsed_vout, 3);
    }

    #[test]
    fn parse_outpoint_rejects_empty() {
        let err = parse_outpoint("").unwrap_err();
        assert!(matches!(err, DelegationError::Wallet(_)));
    }

    #[test]
    fn parse_outpoint_rejects_null_72_hex_placeholder() {
        let null = "0".repeat(72);
        let err = parse_outpoint(&null).unwrap_err();
        match err {
            DelegationError::Wallet(msg) => assert!(msg.contains("null")),
            _ => panic!("expected Wallet error for null placeholder"),
        }
    }

    #[test]
    fn parse_outpoint_rejects_short_txid() {
        let err = parse_outpoint("abcdef.0").unwrap_err();
        assert!(matches!(err, DelegationError::Wallet(_)));
    }

    #[test]
    fn parse_outpoint_rejects_non_hex_txid() {
        let outpoint = format!("{}.0", "z".repeat(64));
        let err = parse_outpoint(&outpoint).unwrap_err();
        assert!(matches!(err, DelegationError::Wallet(_)));
    }

    #[test]
    fn parse_outpoint_rejects_unparseable_vout() {
        let outpoint = format!("{}.notanumber", "a".repeat(64));
        let err = parse_outpoint(&outpoint).unwrap_err();
        assert!(matches!(err, DelegationError::Wallet(_)));
    }

    #[test]
    fn parse_outpoint_trims_whitespace() {
        let txid = "e".repeat(64);
        let outpoint = format!("  {txid}.7  ");
        let (parsed_txid, parsed_vout) = parse_outpoint(&outpoint).unwrap();
        assert_eq!(parsed_txid, txid);
        assert_eq!(parsed_vout, 7);
    }

    // ── OverlayRevocationChecker ─────────────────────────────────────
    //
    // Uses mockito to stand up a fake overlay /lookup endpoint and verify
    // the three semantic outcomes (unspent / spent / network error). The
    // canonical txid format is enforced by parse_outpoint, so we use a real
    // 64-hex placeholder for the test outpoint.

    fn test_outpoint() -> String {
        format!("{}.0", "a".repeat(64))
    }

    #[tokio::test]
    async fn overlay_checker_unspent_when_outputs_present() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/lookup")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"type":"output-list","outputs":[{"beef":[1,2,3],"outputIndex":0}]}"#)
            .create_async()
            .await;

        let checker = OverlayRevocationChecker::new(server.url());
        let revoked = checker.is_outpoint_spent(&test_outpoint()).await.unwrap();
        assert!(!revoked, "non-empty outputs → not revoked");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn overlay_checker_spent_when_outputs_empty() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/lookup")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"type":"output-list","outputs":[]}"#)
            .create_async()
            .await;

        let checker = OverlayRevocationChecker::new(server.url());
        let revoked = checker.is_outpoint_spent(&test_outpoint()).await.unwrap();
        assert!(revoked, "empty outputs → revoked");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn overlay_checker_http_error_propagates() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/lookup")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(r#"{"message":"engine exploded"}"#)
            .create_async()
            .await;

        let checker = OverlayRevocationChecker::new(server.url());
        let err = checker
            .is_outpoint_spent(&test_outpoint())
            .await
            .unwrap_err();
        match err {
            DelegationError::Wallet(msg) => {
                assert!(
                    msg.contains("HTTP"),
                    "error should mention HTTP status: {msg}"
                );
            }
            _ => panic!("expected Wallet error variant"),
        }
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn overlay_checker_parse_error_short_circuits_before_network() {
        // Use an obviously broken URL — if the test reaches the network the
        // call will fail, but parse_outpoint should reject the input first
        // and we should never reach the HTTP layer.
        let checker = OverlayRevocationChecker::new("http://127.0.0.1:1");
        let err = checker
            .is_outpoint_spent("not-an-outpoint")
            .await
            .unwrap_err();
        match err {
            DelegationError::Wallet(msg) => {
                // Should be the parse error, not a network error.
                assert!(
                    msg.contains("unrecognized") || msg.contains("invalid"),
                    "expected parse error, got: {msg}"
                );
            }
            _ => panic!("expected Wallet error"),
        }
    }

    #[tokio::test]
    async fn overlay_checker_trims_trailing_slash() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/lookup")
            .with_status(200)
            .with_body(r#"{"type":"output-list","outputs":[{"beef":[],"outputIndex":0}]}"#)
            .create_async()
            .await;
        // Pass URL with trailing slash — the checker should normalize it.
        let url = format!("{}/", server.url());
        let checker = OverlayRevocationChecker::new(url);
        assert!(!checker.is_outpoint_spent(&test_outpoint()).await.unwrap());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn overlay_checker_canonicalizes_outpoint_before_query() {
        // Bare 64-hex txid → checker should canonicalize to "txid.0" before
        // sending to overlay. We assert by matching the request body.
        let mut server = mockito::Server::new_async().await;
        let txid = "f".repeat(64);
        let expected_query = format!("{txid}.0");
        let mock = server
            .mock("POST", "/lookup")
            .match_body(mockito::Matcher::PartialJsonString(format!(
                r#"{{"query":{{"findByOutpoint":"{expected_query}"}}}}"#
            )))
            .with_status(200)
            .with_body(r#"{"type":"output-list","outputs":[]}"#)
            .create_async()
            .await;

        let checker = OverlayRevocationChecker::new(server.url());
        let _ = checker.is_outpoint_spent(&txid).await.unwrap();
        mock.assert_async().await;
    }
}
