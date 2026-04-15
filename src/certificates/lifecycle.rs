//! Certificate lifecycle management — acquire, issue, revoke, relinquish,
//! revocation checks, and boot-time authorization validation.

use serde_json::{json, Value};

use super::types::*;
use crate::error::DmError;
use crate::onchain::state::{self, StateToken, TokenType, BASKET_REVOCATION, BASKET_STATE};
use crate::wallet::WalletBackend;

/// Certificate manager for BRC-52 agent identity.
pub struct CertificateManager {
    wallet: std::sync::Arc<dyn WalletBackend>,
}

impl CertificateManager {
    pub fn new(wallet: std::sync::Arc<dyn WalletBackend>) -> Self {
        Self { wallet }
    }

    /// Check if the agent has a valid authorization certificate.
    pub async fn has_authorization(&self) -> Result<bool, DmError> {
        let result = self
            .wallet
            .list_certificates(
                &[],                     // any certifier
                &[CERT_TYPE_AGENT_AUTH], // our type
                10,
                0,
            )
            .await?;

        let certs = result
            .get("certificates")
            .or_else(|| result.get("totalCertificates"))
            .and_then(|v| {
                if v.is_array() {
                    Some(v.as_array().unwrap().len())
                } else {
                    v.as_u64().map(|n| n as usize)
                }
            })
            .unwrap_or(0);

        Ok(certs > 0)
    }

    /// Check if the agent holds a parent-signed authorization certificate.
    ///
    /// Returns true if any cert has `certifier != subject`.
    pub async fn has_parent_authorization(&self) -> Result<bool, DmError> {
        let certs = self.list_auth_certs().await?;
        for cert in &certs {
            let c = cert.get("certificate").unwrap_or(cert);
            let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            if !subject.is_empty() && !certifier.is_empty() && certifier != subject {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Get the current certificate status.
    ///
    /// **Revoked-cert filter**: when scanning for parent-signed certs, this
    /// function calls `is_revoked()` on each candidate and skips any whose
    /// revocation UTXO has been spent. This is necessary because the wallet's
    /// `relinquish_certificate` call is "best-effort" for parent-issued certs
    /// — some wallet backends refuse to remove them from the cert store even
    /// after revocation. Without this filter, a revoked cert can haunt the
    /// cert_status path indefinitely, showing up in the system prompt and
    /// tripping LLM safety responses. Fix: iterate and skip revoked.
    ///
    /// Returns the first NON-REVOKED parent-signed cert. Falls through to
    /// self-signed only if no valid parent-signed cert exists.
    pub async fn certificate_status(&self) -> Result<CertificateStatus, DmError> {
        let identity_key = self.wallet.get_identity_key().await.ok();
        let certs = self.list_auth_certs().await?;

        if certs.is_empty() {
            return Ok(CertificateStatus {
                status: "none".into(),
                certificate: None,
                identity_key,
            });
        }

        // Check for non-revoked parent-signed cert (certifier != subject).
        for cert in &certs {
            let c = cert.get("certificate").unwrap_or(cert);
            let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            if !subject.is_empty() && !certifier.is_empty() && certifier != subject {
                // Skip if this parent-signed cert is revoked (its revocation
                // UTXO has been spent). Stale revoked certs can remain in the
                // wallet's cert store even after revoke_and_relinquish because
                // relinquish_certificate is best-effort for parent-issued certs.
                let revoked = self.is_revoked(c).await.unwrap_or(false);
                if revoked {
                    tracing::debug!(
                        serial = ?c.get("serialNumber"),
                        "certificate_status: skipping revoked parent-signed cert"
                    );
                    continue;
                }
                return Ok(CertificateStatus {
                    status: "parent-signed".into(),
                    certificate: Some(c.clone()),
                    identity_key,
                });
            }
        }

        // No valid parent-signed cert found. Check for self-signed (any
        // first cert where certifier == subject or either is empty).
        for cert in &certs {
            let c = cert.get("certificate").unwrap_or(cert);
            let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            let is_self_signed = certifier.is_empty() || subject.is_empty() || certifier == subject;
            if is_self_signed {
                return Ok(CertificateStatus {
                    status: "self-signed".into(),
                    certificate: Some(c.clone()),
                    identity_key,
                });
            }
        }

        // All parent-signed certs are revoked AND no self-signed exists.
        // Report as none — the agent has no usable authorization.
        Ok(CertificateStatus {
            status: "none".into(),
            certificate: None,
            identity_key,
        })
    }

    /// List authorization certificates from the wallet.
    pub async fn list_auth_certs(&self) -> Result<Vec<Value>, DmError> {
        let result = self
            .wallet
            .list_certificates(&[], &[CERT_TYPE_AGENT_AUTH], 10, 0)
            .await?;

        Ok(result
            .get("certificates")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Acquire a parent-signed authorization certificate.
    ///
    /// Flow:
    /// 1. Get agent's identity key (subject)
    /// 2. Get parent's identity key from parent wallet
    /// 3. Parent wallet signs the certificate data
    /// 4. Agent wallet acquires the cert with parent as certifier
    #[allow(clippy::too_many_arguments)]
    pub async fn acquire_parent_authorization(
        &self,
        parent_wallet: &dyn WalletBackend,
        agent_name: &str,
        capabilities: &[&str],
        budget_per_task: Option<u64>,
        budget_per_hour: Option<u64>,
        budget_per_day: Option<u64>,
        budget_per_week: Option<u64>,
        budget_per_month: Option<u64>,
        budget_lifetime: Option<u64>,
    ) -> Result<Value, DmError> {
        let agent_key = self.wallet.get_identity_key().await?;
        let parent_key = parent_wallet.get_identity_key().await?;

        let ts = chrono::Utc::now().timestamp();
        let serial_number = format!("dm-{}-{}-{}", agent_name, &agent_key[..8], ts);

        // Create a revocation UTXO in the agent's wallet.
        // The parent revokes by requesting the agent spend this UTXO via
        // POST /certificates/revoke (BRC-31 auth, parent-only).
        let revocation_data = json!({
            "type": "CertificateRevocation",
            "certificate_type": CERT_TYPE_AGENT_AUTH,
            "subject": agent_key,
            "certifier": parent_key,
            "serial_number": serial_number,
            "issued_at": chrono::Utc::now().to_rfc3339(),
        });
        let revocation_token = StateToken::new(TokenType::Checkpoint, revocation_data);
        let rev_result = state::create_token_in_basket(
            &*self.wallet,
            revocation_token,
            false,
            "self",
            Some(BASKET_REVOCATION),
        )
        .await?;
        let revocation_outpoint = format!(
            "{}{:08x}",
            rev_result.txid,
            rev_result.token.vout.unwrap_or(0)
        );
        tracing::info!(
            "BRC-52: created revocation UTXO {} for cert {}",
            &revocation_outpoint[..16],
            &serial_number
        );

        // Parent signs the certificate data
        let sign_data = format!("{} {} {}", CERT_TYPE_AGENT_AUTH, agent_key, serial_number);
        let signature_bytes = parent_wallet
            .create_signature(
                sign_data.as_bytes(),
                &json!([2, "certificate signing"]),
                "1",
                &agent_key, // counterparty = agent
            )
            .await?;
        let signature_hex = hex::encode(&signature_bytes);

        let mut fields = serde_json::Map::new();
        fields.insert("name".into(), json!(agent_name));
        fields.insert("capabilities".into(), json!(capabilities.join(",")));
        if let Some(v) = budget_per_task.filter(|&v| v > 0) {
            fields.insert("budget_per_task".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_hour.filter(|&v| v > 0) {
            fields.insert("budget_per_hour".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_day.filter(|&v| v > 0) {
            fields.insert("budget_per_day".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_week.filter(|&v| v > 0) {
            fields.insert("budget_per_week".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_month.filter(|&v| v > 0) {
            fields.insert("budget_per_month".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_lifetime.filter(|&v| v > 0) {
            fields.insert("budget_lifetime".into(), json!(v.to_string()));
        }
        fields.insert("deployed_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        fields.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));

        let certificate = json!({
            "certificateType": CERT_TYPE_AGENT_AUTH,
            "type": CERT_TYPE_AGENT_AUTH,
            "subject": agent_key,
            "certifier": parent_key,
            "serialNumber": serial_number,
            "revocationOutpoint": revocation_outpoint,
            "signature": signature_hex,
            "fields": fields,
            "acquisitionProtocol": "direct",
        });

        self.wallet.acquire_certificate(&certificate).await
    }

    /// Acquire a self-signed authorization certificate.
    ///
    /// The agent's own identity key is used as both subject and certifier,
    /// creating a minimal identity chain. A real deployment would use a
    /// parent/principal key as the certifier.
    #[allow(clippy::too_many_arguments)]
    pub async fn acquire_authorization(
        &self,
        agent_name: &str,
        capabilities: &[&str],
        certifier_key: Option<&str>,
        budget_per_task: Option<u64>,
        budget_per_hour: Option<u64>,
        budget_per_day: Option<u64>,
        budget_per_week: Option<u64>,
        budget_per_month: Option<u64>,
        budget_lifetime: Option<u64>,
    ) -> Result<Value, DmError> {
        let identity_key = self.wallet.get_identity_key().await?;
        let certifier = certifier_key.unwrap_or(&identity_key);

        let ts = chrono::Utc::now().timestamp();
        let serial_number = format!("dm-{}-{}-{}", agent_name, &identity_key[..8], ts);

        // Null revocation outpoint (72 hex chars = 36 bytes: 32-byte txid + 4-byte vout)
        let revocation_outpoint = "0".repeat(72);

        // Sign the certificate for direct acquisition
        let sign_data = format!(
            "{} {} {}",
            CERT_TYPE_AGENT_AUTH, identity_key, serial_number
        );
        let signature_bytes = self
            .wallet
            .create_signature(
                sign_data.as_bytes(),
                &serde_json::json!([2, "certificate signing"]),
                "1",
                "self",
            )
            .await?;
        let signature_hex = hex::encode(&signature_bytes);

        let mut fields = serde_json::Map::new();
        fields.insert("name".into(), json!(agent_name));
        fields.insert("capabilities".into(), json!(capabilities.join(",")));
        if let Some(v) = budget_per_task.filter(|&v| v > 0) {
            fields.insert("budget_per_task".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_hour.filter(|&v| v > 0) {
            fields.insert("budget_per_hour".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_day.filter(|&v| v > 0) {
            fields.insert("budget_per_day".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_week.filter(|&v| v > 0) {
            fields.insert("budget_per_week".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_per_month.filter(|&v| v > 0) {
            fields.insert("budget_per_month".into(), json!(v.to_string()));
        }
        if let Some(v) = budget_lifetime.filter(|&v| v > 0) {
            fields.insert("budget_lifetime".into(), json!(v.to_string()));
        }
        fields.insert("deployed_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        fields.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));

        let certificate = json!({
            "certificateType": CERT_TYPE_AGENT_AUTH,
            "type": CERT_TYPE_AGENT_AUTH,
            "subject": identity_key,
            "certifier": certifier,
            "serialNumber": serial_number,
            "revocationOutpoint": revocation_outpoint,
            "signature": signature_hex,
            "fields": fields,
            "acquisitionProtocol": "direct",
        });

        self.wallet.acquire_certificate(&certificate).await
    }

    /// Relinquish any self-signed authorization certificates.
    ///
    /// Called after a parent-signed cert is acquired to maintain the
    /// one-active-auth-cert rule.
    pub async fn relinquish_self_signed(&self) -> Result<usize, DmError> {
        let certs = self.list_auth_certs().await?;
        let mut count = 0;
        for cert in &certs {
            let c = cert.get("certificate").unwrap_or(cert);
            let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            if !subject.is_empty() && certifier == subject {
                if let Err(e) = self.wallet.relinquish_certificate(c).await {
                    tracing::warn!("Failed to relinquish self-signed cert: {e}");
                } else {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Generate a proof of authorization for a verifier.
    ///
    /// Reveals only the requested fields without exposing others.
    pub async fn prove_authorization(&self, fields: &[&str]) -> Result<Value, DmError> {
        let result = self
            .wallet
            .list_certificates(&[], &[CERT_TYPE_AGENT_AUTH], 1, 0)
            .await?;

        let cert_wrapper = result
            .get("certificates")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| DmError::wallet("No authorization certificate found".to_string()))?;

        // The wallet returns each entry as {"certificate": {...}}; unwrap if present
        let cert = cert_wrapper.get("certificate").unwrap_or(cert_wrapper);

        self.wallet.prove_certificate(cert, fields).await
    }

    /// Check whether a parent-signed certificate has been revoked.
    ///
    /// A certificate is revoked when its revocation UTXO has been spent
    /// (relinquished from the `worm-revocation` basket). Null outpoints
    /// (self-signed certs) are treated as "no revocation mechanism" and
    /// always return Ok(false).
    ///
    /// Paginated: fetches up to 1000 UTXOs per page and continues until
    /// the target outpoint is found or all pages are exhausted.
    pub async fn is_revoked(&self, cert: &Value) -> Result<bool, DmError> {
        let (txid, _vout) = match self.find_revocation_outpoint(cert).await {
            Some(parsed) => parsed,
            None => return Ok(false), // No revocation mechanism found
        };

        // Paginate through all revocation basket outputs to find the matching UTXO.
        // If the UTXO is found, the cert is still valid (not revoked).
        // If not found after scanning all pages, the UTXO was spent — cert is revoked.
        let limit: u64 = 1000;
        let mut offset: u64 = 0;
        loop {
            match self
                .wallet
                .list_outputs(BASKET_REVOCATION, "locking scripts", limit, offset)
                .await
            {
                Ok(result) => {
                    let outputs = result
                        .get("outputs")
                        .and_then(|v| v.as_array())
                        .or_else(|| result.as_array())
                        .cloned()
                        .unwrap_or_default();

                    let found = outputs.iter().any(|o| {
                        o.get("outpoint")
                            .and_then(|op| op.as_str())
                            .map(|op| op.starts_with(&txid))
                            .unwrap_or(false)
                    });

                    if found {
                        return Ok(false); // UTXO exists — not revoked
                    }

                    // If fewer results than the limit, we've reached the last page
                    if (outputs.len() as u64) < limit {
                        return Ok(true); // UTXO not found — revoked
                    }

                    offset += limit;
                }
                Err(e) => {
                    tracing::warn!("Failed to check revocation UTXO: {e}");
                    // On error, assume not revoked (fail-open for connectivity issues)
                    return Ok(false);
                }
            }
        }
    }

    /// Find the revocation outpoint for a certificate.
    ///
    /// Tries multiple sources in order:
    /// 1. The certificate's `revocationOutpoint` field (fast path)
    /// 2. Scanning the `worm-revocation` basket for matching serial number
    /// 3. Scanning the `worm-state` basket (legacy: older certs stored there)
    pub async fn find_revocation_outpoint(&self, cert: &Value) -> Option<(String, u32)> {
        // Fast path: try the certificate field
        let outpoint_str = cert
            .get("revocationOutpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(parsed) = parse_revocation_outpoint(outpoint_str) {
            return Some(parsed);
        }

        // Fallback: scan baskets for matching serial number
        let serial = cert
            .get("serialNumber")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if serial.is_empty() {
            return None;
        }

        // Try worm-revocation first (new certs), then worm-state (legacy)
        for basket in &[BASKET_REVOCATION, BASKET_STATE] {
            if let Some(found) = self.scan_basket_for_revocation(basket, serial).await {
                return Some(found);
            }
        }

        None
    }

    /// Scan a basket for a revocation UTXO matching the given serial number.
    async fn scan_basket_for_revocation(
        &self,
        basket: &str,
        serial_number: &str,
    ) -> Option<(String, u32)> {
        let result = self
            .wallet
            .list_outputs(basket, "locking scripts", 100, 0)
            .await
            .ok()?;

        let outputs = result
            .get("outputs")
            .and_then(|v| v.as_array())
            .or_else(|| result.as_array())
            .cloned()
            .unwrap_or_default();

        for output in &outputs {
            let script_hex = output
                .get("lockingScript")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if script_hex.is_empty() {
                continue;
            }

            // Parse PushDrop fields from the locking script
            let fields = match state::parse_push_drop_fields(script_hex) {
                Some(f) if f.len() >= 2 => f,
                _ => continue,
            };

            // Second field is the data (hex-encoded JSON)
            let data_bytes = match hex::decode(&fields[1]) {
                Ok(b) => b,
                Err(_) => continue,
            };

            // Try to parse as JSON and check for matching serial number
            if let Ok(data) = serde_json::from_slice::<Value>(&data_bytes) {
                let is_revocation =
                    data.get("type").and_then(|v| v.as_str()) == Some("CertificateRevocation");
                let matches_serial =
                    data.get("serial_number").and_then(|v| v.as_str()) == Some(serial_number);

                if is_revocation && matches_serial {
                    // Extract outpoint from the output
                    if let Some(outpoint_str) = output.get("outpoint").and_then(|v| v.as_str()) {
                        // outpoint format: "txid.vout"
                        let parts: Vec<&str> = outpoint_str.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            if let Ok(vout) = parts[1].parse::<u32>() {
                                return Some((parts[0].to_string(), vout));
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// List all certificates the agent holds.
    pub async fn list_all(&self) -> Result<Vec<Value>, DmError> {
        let result = self.wallet.list_certificates(&[], &[], 100, 0).await?;

        Ok(result
            .get("certificates")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Release a certificate from the wallet.
    pub async fn relinquish(&self, certificate: &Value) -> Result<(), DmError> {
        self.wallet.relinquish_certificate(certificate).await?;
        Ok(())
    }

    /// Check whether the agent already holds a parent-signed `agent-authorization`
    /// certificate whose `fields.name` equals `agent_name` AND whose `fields.capabilities`
    /// (comma-separated, byte-exact) equals `capabilities_csv` AND is not revoked.
    ///
    /// Returns `true` only when such a cert exists, meaning boot-time re-issuance
    /// can be skipped. Any mismatch (different name, different capabilities, different
    /// byte ordering) or a revoked cert returns `false` and the caller should
    /// re-acquire with the desired state.
    ///
    /// The revocation check is critical: a cert's on-disk record can persist
    /// after its revocation UTXO has been spent/relinquished, leaving the wallet
    /// in an asymmetric state where `certificate_status()` (which filters revoked)
    /// returns "none" but `has_matching_parent_cert()` would otherwise say
    /// "existing cert is fine, skip re-issue." Without the revocation check, the
    /// daemon boots with a structurally-valid-but-effectively-revoked cert and
    /// every subsequent `/certificates` query returns empty. Observed on
    /// worker-bsky-en-11 on 2026-04-15 after a mid-run failure.
    pub async fn has_matching_parent_cert(
        &self,
        agent_name: &str,
        capabilities_csv: &str,
    ) -> Result<bool, DmError> {
        let certs = self.list_auth_certs().await?;
        for entry in &certs {
            let c = entry.get("certificate").unwrap_or(entry);
            let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            // Parent-signed = certifier is present and differs from subject.
            let is_parent_signed =
                !subject.is_empty() && !certifier.is_empty() && certifier != subject;
            if !is_parent_signed {
                continue;
            }
            let cert_name = c
                .get("fields")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let cert_caps = c
                .get("fields")
                .and_then(|f| f.get("capabilities"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cert_name == agent_name && cert_caps == capabilities_csv {
                // Final guard: skip if the cert is revoked (its revocation UTXO
                // is no longer in any revocation basket). Without this, a stale
                // revoked cert causes a boot-time false positive and cluster.js
                // step 5 times out because /certificates returns "none".
                let revoked = self.is_revoked(c).await.unwrap_or(false);
                if revoked {
                    tracing::warn!(
                        name = %agent_name,
                        capabilities = %capabilities_csv,
                        "has_matching_parent_cert: found matching cert but it is revoked — forcing re-issue"
                    );
                    continue;
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Revoke a parent-signed `agent-authorization` certificate and remove it from the
    /// wallet store. Canonical two-step revocation matching `POST /certificates/revoke`:
    ///
    /// 1. Find the revocation UTXO via `find_revocation_outpoint()` and relinquish it
    ///    from `BASKET_REVOCATION` (fallback: `BASKET_STATE` for legacy certs). After
    ///    this, `is_revoked()` will return `true` — the cert is considered revoked on
    ///    the paginated basket scan, even without broadcasting a separate spend tx.
    /// 2. Create a BRC-18 `CertificateRevocation` on-chain proof as the audit record
    ///    (best-effort — failure is logged but does not fail the revocation).
    /// 3. Relinquish the certificate from the wallet store (best-effort).
    ///
    /// Returns `Ok(Some(txid))` if the BRC-18 proof was broadcast, `Ok(None)` if the
    /// cert had no revocation outpoint (legacy) or if the proof step failed.
    pub async fn revoke_and_relinquish(&self, cert: &Value) -> Result<Option<String>, DmError> {
        let serial = cert
            .get("serialNumber")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let subject = cert
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let certifier = cert
            .get("certifier")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let proof_txid = if let Some((txid, vout)) = self.find_revocation_outpoint(cert).await {
            // Step 1: remove the revocation UTXO from its basket — tries dm-revocation
            // first, falls back to dm-state for legacy certs.
            let primary = self
                .wallet
                .relinquish_output(BASKET_REVOCATION, &txid, vout)
                .await;
            if primary.is_err() {
                let _ = self
                    .wallet
                    .relinquish_output(BASKET_STATE, &txid, vout)
                    .await;
            }

            // Step 2: best-effort BRC-18 CertificateRevocation proof.
            let proof_data = format!(
                "REVOCATION: {serial}\nSUBJECT: {subject}\nCERTIFIER: {certifier}\nREVOCATION_OUTPOINT: {txid}:{vout}"
            );
            let commitment = crate::onchain::proofs::ProofCommitment::new(
                crate::onchain::proofs::ProofType::CertificateRevocation,
                &proof_data,
                None,
            );
            match crate::onchain::proofs::create_proof(&*self.wallet, commitment).await {
                Ok(result) => {
                    tracing::info!(
                        serial = %serial,
                        proof_txid = %result.txid,
                        "BRC-52: revoked parent-signed cert (relinquished revocation UTXO + proof)"
                    );
                    Some(result.txid)
                }
                Err(e) => {
                    tracing::warn!(
                        serial = %serial,
                        error = %e,
                        "BRC-52: revocation UTXO relinquished but proof creation failed (non-fatal)"
                    );
                    None
                }
            }
        } else {
            tracing::warn!(
                serial = %serial,
                "BRC-52: no revocation UTXO found for cert — falling back to relinquish-only"
            );
            None
        };

        // Step 3: remove the cert from the wallet store. This is best-effort —
        // some wallets refuse to relinquish parent-issued certs and that's OK
        // because step 1 already made it revoked.
        if let Err(e) = self.wallet.relinquish_certificate(cert).await {
            tracing::debug!(
                serial = %serial,
                error = %e,
                "BRC-52: relinquish_certificate failed (non-fatal — cert is already revoked via UTXO spend)"
            );
        }

        Ok(proof_txid)
    }
}

/// Check authorization certificate status at boot.
///
/// Validates that a parent-signed certificate exists and is not revoked.
/// Does NOT auto-acquire certificates — the parent must issue one via
/// `POST /certificates/issue`.
///
/// Returns:
/// - `Ok(CertCheckResult::Valid)` — valid, non-revoked parent-signed cert
/// - `Ok(CertCheckResult::Revoked)` — cert exists but has been revoked
/// - `Ok(CertCheckResult::None)` — no certificate present
pub async fn check_authorization(wallet: std::sync::Arc<dyn WalletBackend>) -> CertCheckResult {
    let mgr = CertificateManager::new(wallet);

    // Check if we have a parent-signed cert
    match mgr.has_parent_authorization().await {
        Ok(true) => {
            // Validate revocation status
            let status = match mgr.certificate_status().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("BRC-52 certificate status check failed: {e}");
                    return CertCheckResult::Valid; // fail-open on wallet errors
                }
            };
            if let Some(ref cert) = status.certificate {
                match mgr.is_revoked(cert).await {
                    Ok(true) => {
                        tracing::error!(
                            "BRC-52 authorization certificate has been REVOKED by parent"
                        );
                        return CertCheckResult::Revoked;
                    }
                    Ok(false) => {
                        tracing::info!(
                            "BRC-52 authorization certificate: present (parent-signed, valid)"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("BRC-52 revocation check failed (non-fatal): {e}");
                    }
                }
            } else {
                tracing::info!("BRC-52 authorization certificate: present (parent-signed)");
            }
            CertCheckResult::Valid
        }
        Ok(false) => {
            tracing::warn!(
                "No BRC-52 authorization certificate — agent capabilities restricted \
                 until parent issues one via POST /certificates/issue"
            );
            CertCheckResult::None
        }
        Err(e) => {
            tracing::warn!("BRC-52 certificate check failed: {e}");
            CertCheckResult::None
        }
    }
}
