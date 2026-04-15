//! Staleness-aware overlay re-registration.
//!
//! This module provides an idempotent re-registration path for the agent's
//! overlay UTXOs. It solves a specific staleness bug: the overlay registration
//! is published once at boot from whatever certificate the agent happened to
//! hold at that moment (often a self-signed default-capabilities cert). Any
//! later `POST /certificates/issue` call that installs a role-specific cert
//! would leave the overlay index pointing at the OLD capabilities — making
//! `overlay_lookup(findByCapability: ...)` return empty for the new role.
//!
//! The approach (user's words: "option A sounds god like — sense staleness and
//! de register (spend) and then register again"):
//!
//! 1. Look up the agent's existing registration on the overlay (by identity
//!    key) via [`check_registered`].
//! 2. Compare the existing record's `name`, `capabilities`, and `certifier_key`
//!    against the values the caller wants published. If everything matches,
//!    the registration is fresh — return early with
//!    `skipped_because_already_fresh: true`.
//! 3. Otherwise the registration is stale. Spend every UTXO in the
//!    `dm-agent-registration` basket via [`deregister_from_overlay`], which
//!    paginates the basket, spends each UTXO back to the wallet, and submits
//!    each spending tx to the overlay's `/submit` endpoint so the overlay
//!    prunes the old record.
//! 4. Finally, publish a fresh registration via [`register_on_overlay`] with
//!    the caller's current name/capabilities/certifier.
//!
//! The function is best-effort on errors: spend failures on individual UTXOs
//! are logged but do not abort the re-registration. Caller handles a
//! [`DmError`] return as "log and continue" — the certificate was still
//! issued, only the overlay index update failed.

use serde::{Deserialize, Serialize};

use crate::error::DmError;
use crate::wallet::WalletBackend;

use super::registration::{check_registered, deregister_from_overlay, register_on_overlay};

/// Outcome of an overlay re-registration attempt.
///
/// Returned by [`reregister_on_overlay`] so callers (and the HTTP endpoint)
/// can see exactly what happened without re-querying the overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReregistrationResult {
    /// Did we find an existing registration on the overlay? (0 or 1 —
    /// `check_registered` returns at most one match by identity key.)
    pub stale_found: usize,
    /// How many stale UTXOs were successfully spent from the
    /// `dm-agent-registration` basket.
    pub stale_spent: usize,
    /// Txid of the new registration, if one was published. `None` when
    /// [`skipped_because_already_fresh`] is `true` or registration failed.
    pub new_registration_txid: Option<String>,
    /// Capabilities string (comma-separated) that the new registration
    /// advertises. Echoed back for easy client-side verification.
    pub new_capabilities: String,
    /// Agent name that the new registration advertises.
    pub new_name: String,
    /// True when the existing registration already matched the target name,
    /// capabilities, and certifier — no spending or re-publishing happened.
    pub skipped_because_already_fresh: bool,
}

/// Decide whether an existing overlay registration matches the target state.
///
/// Pure function split out for unit testing — compares the raw fields without
/// touching the wallet or overlay HTTP.
///
/// Returns `true` when the registration can be left alone.
pub(crate) fn registration_matches(
    existing_name: &str,
    existing_capabilities: &[String],
    existing_certifier: &str,
    target_name: &str,
    target_capabilities: &str,
    target_certifier: &str,
) -> bool {
    if existing_name != target_name {
        return false;
    }
    if existing_certifier != target_certifier {
        return false;
    }
    // Compare capabilities as CSV — `check_registered` returns them as a
    // `Vec<String>` split on `,` whereas the registration path stores them
    // as a single comma-separated string.
    let existing_csv = existing_capabilities.join(",");
    existing_csv == target_capabilities
}

/// Staleness-aware re-registration entry point.
///
/// See the module docs for the full flow. `capabilities` is a comma-separated
/// string (e.g. `"web_fetch,execute_bash,delegate_task"`). `certifier_key` is
/// optional and defaults to the agent's own identity key when `None`.
///
/// On success returns a [`ReregistrationResult`] describing what happened.
///
/// This function is intentionally idempotent: calling it twice in a row will
/// do the full work on the first call and return
/// `skipped_because_already_fresh: true` on the second.
pub async fn reregister_on_overlay(
    wallet: &dyn WalletBackend,
    overlay_url: &str,
    name: &str,
    capabilities: &str,
    certifier_key: Option<&str>,
) -> Result<ReregistrationResult, DmError> {
    // Resolve the effective certifier — fall back to identity key (self-loop)
    // when the caller didn't supply one. This mirrors `register_on_overlay`'s
    // fallback so the staleness comparison matches what would be published.
    let identity_key = wallet.get_identity_key().await?;
    let effective_certifier = certifier_key.unwrap_or(&identity_key).to_string();

    tracing::info!(
        name,
        capabilities,
        certifier = %effective_certifier,
        "Overlay re-registration: checking current state"
    );

    // Phase 1: look up what (if anything) is currently published for this
    // identity key. Errors here are non-fatal — we assume "not registered"
    // and fall through to registration.
    let existing = match check_registered(wallet, overlay_url).await {
        Ok(record) => record,
        Err(e) => {
            tracing::warn!(
                "Overlay lookup failed during re-registration: {e} — treating as not-registered"
            );
            None
        }
    };

    let stale_found = existing.as_ref().map(|_| 1).unwrap_or(0);

    // Phase 2: if the current registration already matches the target, we're
    // done. This is the idempotent fast path.
    if let Some(ref record) = existing {
        if registration_matches(
            &record.name,
            &record.capabilities,
            &record.certifier_key,
            name,
            capabilities,
            &effective_certifier,
        ) {
            tracing::info!(
                name = %record.name,
                capabilities = %record.capabilities.join(","),
                "Overlay re-registration: existing record is fresh, no action needed"
            );
            return Ok(ReregistrationResult {
                stale_found,
                stale_spent: 0,
                new_registration_txid: None,
                new_capabilities: capabilities.to_string(),
                new_name: name.to_string(),
                skipped_because_already_fresh: true,
            });
        }

        tracing::info!(
            old_name = %record.name,
            new_name = %name,
            old_capabilities = %record.capabilities.join(","),
            new_capabilities = %capabilities,
            old_certifier = %record.certifier_key,
            new_certifier = %effective_certifier,
            "Overlay re-registration: STALE — spending old UTXOs and re-publishing"
        );
    } else {
        tracing::info!(
            name,
            capabilities,
            "Overlay re-registration: no existing record, publishing fresh"
        );
    }

    // Phase 3: spend every UTXO in the agent-registration basket. This also
    // submits each spending tx to the overlay's `/submit` endpoint so the
    // overlay prunes the old record. Partial failures are swallowed inside
    // `deregister_from_overlay` — it logs and keeps going.
    let stale_spent = match deregister_from_overlay(wallet, overlay_url).await {
        Ok(n) => n,
        Err(e) => {
            // Log loud — we'll still try the fresh registration below so the
            // agent isn't completely absent from the overlay, but the old
            // records may linger until the next attempt.
            tracing::warn!(
                "deregister_from_overlay failed during re-registration: {e} — \
                 continuing with fresh registration anyway"
            );
            0
        }
    };

    // Phase 4: publish the fresh registration with the caller's target state.
    let new_txid =
        register_on_overlay(wallet, overlay_url, name, capabilities, certifier_key).await?;

    tracing::info!(
        txid = %new_txid,
        stale_found,
        stale_spent,
        "Overlay re-registration complete"
    );

    Ok(ReregistrationResult {
        stale_found,
        stale_spent,
        new_registration_txid: Some(new_txid),
        new_capabilities: capabilities.to_string(),
        new_name: name.to_string(),
        skipped_because_already_fresh: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- registration_matches (pure, no wallet) ----

    #[test]
    fn matches_exact() {
        assert!(registration_matches(
            "scraper",
            &["web_fetch".to_string(), "execute_bash".to_string()],
            "02abcd",
            "scraper",
            "web_fetch,execute_bash",
            "02abcd",
        ));
    }

    #[test]
    fn mismatch_on_name() {
        assert!(!registration_matches(
            "old-name",
            &["web_fetch".to_string()],
            "02abcd",
            "new-name",
            "web_fetch",
            "02abcd",
        ));
    }

    #[test]
    fn mismatch_on_capabilities_added() {
        assert!(!registration_matches(
            "scraper",
            &["web_fetch".to_string()],
            "02abcd",
            "scraper",
            "web_fetch,execute_bash",
            "02abcd",
        ));
    }

    #[test]
    fn mismatch_on_capabilities_removed() {
        assert!(!registration_matches(
            "scraper",
            &["web_fetch".to_string(), "execute_bash".to_string()],
            "02abcd",
            "scraper",
            "web_fetch",
            "02abcd",
        ));
    }

    #[test]
    fn mismatch_on_capability_order() {
        // Order-sensitive by design — a cert reissuance that reorders
        // capabilities should trigger re-registration so the overlay's
        // CSV is exactly byte-identical to what the cert advertises.
        assert!(!registration_matches(
            "scraper",
            &["execute_bash".to_string(), "web_fetch".to_string()],
            "02abcd",
            "scraper",
            "web_fetch,execute_bash",
            "02abcd",
        ));
    }

    #[test]
    fn mismatch_on_certifier() {
        assert!(!registration_matches(
            "scraper",
            &["web_fetch".to_string()],
            "02deadbeef",
            "scraper",
            "web_fetch",
            "02cafebabe",
        ));
    }

    #[test]
    fn matches_default_self_loop_certifier() {
        // When a self-signed agent publishes, the certifier == identity key.
        // A subsequent re-registration with the same identity should match.
        let id = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        assert!(registration_matches(
            "dolphin-milk-agent",
            &["llm".to_string(), "tools".to_string()],
            id,
            "dolphin-milk-agent",
            "llm,tools",
            id,
        ));
    }

    // ---- ReregistrationResult serde ----

    #[test]
    fn result_serializes_to_expected_json_shape() {
        let r = ReregistrationResult {
            stale_found: 1,
            stale_spent: 1,
            new_registration_txid: Some("abc123".to_string()),
            new_capabilities: "web_fetch,execute_bash".to_string(),
            new_name: "scraper".to_string(),
            skipped_because_already_fresh: false,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["stale_found"], 1);
        assert_eq!(json["stale_spent"], 1);
        assert_eq!(json["new_registration_txid"], "abc123");
        assert_eq!(json["new_capabilities"], "web_fetch,execute_bash");
        assert_eq!(json["new_name"], "scraper");
        assert_eq!(json["skipped_because_already_fresh"], false);
    }

    #[test]
    fn result_round_trips_through_serde() {
        let r = ReregistrationResult {
            stale_found: 0,
            stale_spent: 0,
            new_registration_txid: None,
            new_capabilities: "llm".to_string(),
            new_name: "agent".to_string(),
            skipped_because_already_fresh: true,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ReregistrationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.stale_found, 0);
        assert_eq!(parsed.stale_spent, 0);
        assert!(parsed.new_registration_txid.is_none());
        assert!(parsed.skipped_because_already_fresh);
    }
}
