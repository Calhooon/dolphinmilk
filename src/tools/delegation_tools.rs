//! Delegation tool — `delegate_task`.
//!
//! This is Phase 2 of the delegation macaroons epic (EPIC #329). The tool
//! lets a Captain (or any issuer) commission work to a Coral/Worker by:
//!
//! 1. Constructing a BRC-52 "agent-delegation" certificate with scoped caveats
//!    (capabilities, capability_args, budget_cap, expiry, purpose_hash, payment).
//! 2. Creating an on-chain revocation UTXO in the `dm-delegation-revocation`
//!    basket — spending this UTXO revokes the cert.
//! 3. Signing the canonical body with BRC-77 (protocol `[2, "agent delegation v1"]`).
//! 4. Wrapping the cert in a `task_delegation` envelope with a `commission_id`.
//! 5. Dispatching the commission to the recipient's `task_inbox` via MessageBox.
//!
//! End-to-end. Atomic from the LLM's perspective. On failure, any partial
//! side effects (specifically: a signed but undelivered cert with its revocation
//! UTXO still on-chain) are logged as warnings but do not require cleanup —
//! the UTXO is harmless and the issuer can revoke it later by spending it.
//!
//! Phase 2 is single-hop only (no re-delegation). `parent_cert_hash` is always
//! `None`. Multi-hop / re-delegation lands in Phase 5.
//!
//! See `docs/DELEGATION-DESIGN.md` §3 (schema), §4 (verification), §9.1 (sender).

use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::delegation::{
    canonicalize_cert, compute_purpose_hash, delegation_protocol_id, DelegationCert, PaymentTerms,
    DELEGATION_VERSION_V1,
};
use crate::messagebox::client::MessageBoxClient;
use crate::messagebox::types::BOX_TASK_INBOX;
use crate::tools::registry::ToolDef;
use crate::wallet::WalletBackend;

/// New BRC-46 basket for delegation revocation UTXOs, kept separate from the
/// existing `dm-revocation` basket used by BRC-52 agent-authorization certs.
///
/// Keeping delegation revocations in their own basket means:
///   - Agent-authorization revocation scans are not polluted by delegation UTXOs.
///   - Delegation lifecycle management (sweeping, analytics) can operate on just
///     this basket without filtering.
///   - Peer verifiers (Phase 3) will scan this basket name when checking
///     revocation status on received delegation certs.
const BASKET_DELEGATION_REVOCATION: &str = "dm-delegation-revocation";

/// Satoshi amount locked in the revocation UTXO. Tiny (1 sat) — the UTXO's
/// value is its existence, not its amount. Matches MIN_TOKEN_SATS in onchain::state.
const REVOCATION_UTXO_SATS: u64 = 1;

/// Minimum and maximum expiry window for a delegation cert, in seconds.
/// Matches the range declared in the tool's JSON schema.
const MIN_EXPIRES_IN_SECS: i64 = 60; // 1 minute
const MAX_EXPIRES_IN_SECS: i64 = 24 * 60 * 60; // 24 hours

// ── Core impl ──────────────────────────────────────────────────────────────

/// Result of a successful `delegate_task` call — surfaced to the LLM.
#[derive(Debug, Clone)]
struct DelegateResult {
    commission_id: String,
    /// Cert serial number — included so the runner can write a
    /// serial→conv_id index for the issuer's commission_payment_claim
    /// handler to find the original conversation when surfacing payments.
    /// EPIC #329 Phase 3.
    serial_number: String,
    delegation_cert_hash: String,
    revocation_txid: String,
    sent_message_id: String,
    recipient: String,
    amount_sats: u64,
}

impl DelegateResult {
    fn to_json(&self) -> Value {
        json!({
            "commission_id": self.commission_id,
            "serial_number": self.serial_number,
            "delegation_cert_hash": self.delegation_cert_hash,
            "revocation_txid": self.revocation_txid,
            "sent_message_id": self.sent_message_id,
            "recipient": self.recipient,
            "amount_sats": self.amount_sats,
        })
    }
}

/// Parse optional per-tool argument scoping from the tool's JSON params into
/// the BRC-52 wire format: `{ "tool": ["arg1", "arg2"], ... }` →
/// `BTreeMap<String, Vec<String>>`.
fn parse_capability_args_input(
    value: Option<&Value>,
) -> Result<Option<std::collections::BTreeMap<String, Vec<String>>>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| "capability_args must be a JSON object".to_string())?;
    if obj.is_empty() {
        return Ok(None);
    }
    let mut out = std::collections::BTreeMap::new();
    for (tool, args_val) in obj {
        let args = args_val
            .as_array()
            .ok_or_else(|| format!("capability_args.{tool} must be an array of strings"))?;
        let mut arg_strs = Vec::with_capacity(args.len());
        for a in args {
            let s = a
                .as_str()
                .ok_or_else(|| format!("capability_args.{tool} entries must be strings"))?;
            arg_strs.push(s.to_string());
        }
        out.insert(tool.clone(), arg_strs);
    }
    Ok(Some(out))
}

/// Parse optional payment fields into `PaymentTerms`. All four are required
/// together if any is present — partial payment specs are rejected.
fn parse_payment_terms_input(params: &Value) -> Result<Option<PaymentTerms>, String> {
    let amount = params.get("payment_amount_per_unit");
    let unit = params.get("payment_unit");
    let max_total = params.get("payment_max_total");
    let derivation = params.get("payment_derivation_invoice");

    let any_present = [amount, unit, max_total, derivation]
        .iter()
        .any(|v| v.is_some() && !v.unwrap().is_null());
    if !any_present {
        return Ok(None);
    }

    let amount_per_unit = amount
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "payment_amount_per_unit must be an integer".to_string())?;
    let unit = unit
        .and_then(|v| v.as_str())
        .ok_or_else(|| "payment_unit must be a string".to_string())?
        .to_string();
    let max_total = max_total
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "payment_max_total must be an integer".to_string())?;
    let derivation_invoice = derivation
        .and_then(|v| v.as_str())
        .ok_or_else(|| "payment_derivation_invoice must be a string".to_string())?
        .to_string();

    Ok(Some(PaymentTerms {
        amount_per_unit,
        unit,
        max_total,
        derivation_invoice,
    }))
}

/// Determine the root certifier to stamp on the delegation cert.
///
/// Resolution order (highest priority first):
///
/// 1. **Operator-declared trust root** (`config.trust.certifiers[0]`): if the
///    operator has declared a trust root in config, that ALWAYS wins. The
///    operator's intent is "this is the trust anchor we operate under" and
///    the cert chain should reflect it. This matches the receiver-side check
///    where Coral's `trust.certifiers` is consulted, so issuer + receiver
///    name the same root.
/// 2. **Parent-signed BRC-52 agent-auth cert**: the agent holds a cert where
///    `certifier != subject`. Use the cert's `certifier` field — that's the
///    party that vouched for the agent. This is what `acquire_parent_authorization`
///    sets up at startup when MetaNet Client is configured.
/// 3. **Any cert's certifier** (even self-signed) as a soft fallback.
/// 4. **Self-identity** if all else fails — peer trust model, the issuer
///    vouches for itself.
///
/// EPIC #329 Phase 3 §8 (DELEGATION-DESIGN.md): the trust root is a config
/// surface, not derived from the cert alone. This function honors that.
async fn resolve_root_certifier(
    wallet: &dyn WalletBackend,
    self_identity_key: &str,
    trust_root_override: Option<&str>,
) -> String {
    // Priority 1: operator-declared trust root.
    if let Some(root) = trust_root_override {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            tracing::debug!(
                trust_root = %trimmed,
                "delegate_task: stamping config.trust.certifiers[0] as delegation root"
            );
            return trimmed.to_string();
        }
    }

    // Priority 2/3: read from agent's BRC-52 cert.
    match wallet
        .list_certificates(&[], &[crate::certificates::CERT_TYPE_AGENT_AUTH], 10, 0)
        .await
    {
        Ok(result) => {
            let certs = result
                .get("certificates")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            // Prefer the first parent-signed cert (certifier != subject). Fall
            // back to any other cert if none is parent-signed.
            let mut fallback: Option<String> = None;
            for cert_wrap in &certs {
                let cert = cert_wrap.get("certificate").unwrap_or(cert_wrap);
                let subject = cert.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                let certifier = cert.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
                if !certifier.is_empty() {
                    if !subject.is_empty() && certifier != subject {
                        return certifier.to_string();
                    }
                    if fallback.is_none() {
                        fallback = Some(certifier.to_string());
                    }
                }
            }
            if let Some(f) = fallback {
                return f;
            }
        }
        Err(e) => {
            tracing::debug!("delegate_task: list_certificates lookup failed: {e}");
        }
    }
    tracing::debug!(
        "delegate_task: no parent cert found, using self identity key as root certifier"
    );
    self_identity_key.to_string()
}

/// Dispatcher trait for sending the commission envelope. Lets tests stub out
/// the real MessageBox network call while production uses the live client.
#[async_trait::async_trait]
trait CommissionDispatcher: Send + Sync {
    async fn send(&self, recipient: &str, message_box: &str, body: &Value)
        -> Result<Value, String>;
}

/// Production dispatcher: forwards to the shared `MessageBoxClient`.
struct MessageBoxDispatcher {
    client: Arc<MessageBoxClient>,
}

#[async_trait::async_trait]
impl CommissionDispatcher for MessageBoxDispatcher {
    async fn send(
        &self,
        recipient: &str,
        message_box: &str,
        body: &Value,
    ) -> Result<Value, String> {
        self.client
            .send_message(recipient, message_box, body)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Hex validation: exactly 66 lowercase-or-uppercase hex chars.
fn is_66_hex(s: &str) -> bool {
    s.len() == 66 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// POST the revocation BEEF to the overlay's `/submit` endpoint with
/// `x-topics: ["tm_dm_delegation"]` so the lookup service indexes the UTXO.
///
/// Best-effort: returns Err on any HTTP / network / response failure but the
/// caller treats this as a non-fatal warning. Mirrors the same /submit
/// pattern used by `src/overlay/registration.rs` for AGENT registration.
async fn submit_revocation_to_overlay(
    overlay_url: &str,
    beef_tx: &[u8],
    serial_number: &str,
    revocation_txid: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{overlay_url}/submit"))
        .header("Content-Type", "application/octet-stream")
        .header("x-topics", r#"["tm_dm_delegation"]"#)
        .body(beef_tx.to_vec())
        .send()
        .await
        .map_err(|e| format!("overlay /submit network error: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("overlay /submit body read error: {e}"))?;

    if !status.is_success() {
        return Err(format!("overlay /submit returned HTTP {status}: {body}"));
    }

    // Parse the steak response and check whether tm_dm_delegation actually
    // admitted any outputs. HTTP 200 only means the BEEF was valid; if the
    // topic manager rejected every output, the cert won't be queryable.
    if let Ok(steak) = serde_json::from_str::<Value>(&body) {
        let admitted = steak
            .get("tm_dm_delegation")
            .and_then(|tm| tm.get("outputsToAdmit"))
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if admitted == 0 {
            return Err(format!(
                "overlay accepted the BEEF but admitted 0 outputs under tm_dm_delegation \
                 (steak={body})"
            ));
        }
        tracing::info!(
            serial = %serial_number,
            txid = %revocation_txid,
            admitted,
            "delegate_task: revocation tx published to overlay tm_dm_delegation"
        );
    } else {
        // Non-JSON response — log and assume success.
        tracing::info!(
            serial = %serial_number,
            txid = %revocation_txid,
            body = %body,
            "delegate_task: overlay accepted revocation tx (non-JSON response)"
        );
    }

    Ok(())
}

/// Core implementation — validates inputs, builds the cert, creates the
/// revocation UTXO, signs, and dispatches the commission.
///
/// Returns an error string on any failure. The caller (the tool closure)
/// surfaces this directly to the LLM as the tool output.
#[allow(clippy::too_many_arguments)]
async fn delegate_task_impl(
    params: Value,
    wallet: Arc<dyn WalletBackend + Send + Sync>,
    dispatcher: Arc<dyn CommissionDispatcher>,
    overlay_url: String,
    trust_root: Option<String>,
) -> String {
    match delegate_task_inner(params, wallet, dispatcher, overlay_url, trust_root).await {
        Ok(result) => result.to_json().to_string(),
        Err(e) => format!("Error: {e}"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn delegate_task_inner(
    params: Value,
    wallet: Arc<dyn WalletBackend + Send + Sync>,
    dispatcher: Arc<dyn CommissionDispatcher>,
    overlay_url: String,
    trust_root: Option<String>,
) -> Result<DelegateResult, String> {
    // ── 1. Validate inputs ──────────────────────────────────────────────
    let recipient = params
        .get("recipient")
        .and_then(|v| v.as_str())
        .ok_or("recipient is required (66-char hex identity key)")?
        .trim()
        .to_string();
    if !is_66_hex(&recipient) {
        return Err("recipient must be a 66-char hex compressed secp256k1 pubkey".into());
    }

    let task = params
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or("task is required")?
        .trim()
        .to_string();
    if task.is_empty() {
        return Err("task description must be non-empty".into());
    }

    let capabilities_val = params
        .get("capabilities")
        .ok_or("capabilities is required (array of tool names)")?;
    let capabilities_arr = capabilities_val
        .as_array()
        .ok_or("capabilities must be an array of strings")?;
    if capabilities_arr.is_empty() {
        return Err("capabilities must be a non-empty array".into());
    }
    let mut capabilities: Vec<String> = Vec::with_capacity(capabilities_arr.len());
    for c in capabilities_arr {
        let s = c
            .as_str()
            .ok_or("capabilities entries must be strings")?
            .trim();
        if s.is_empty() {
            return Err("capabilities entries must be non-empty strings".into());
        }
        capabilities.push(s.to_string());
    }

    let capability_args = parse_capability_args_input(params.get("capability_args"))?;

    let budget_cap_sats = params
        .get("budget_cap_sats")
        .and_then(|v| v.as_u64())
        .ok_or("budget_cap_sats is required and must be a positive integer")?;
    if budget_cap_sats == 0 {
        return Err("budget_cap_sats must be > 0".into());
    }

    let expires_in_secs = params
        .get("expires_in_secs")
        .and_then(|v| v.as_i64())
        .ok_or("expires_in_secs is required and must be an integer")?;
    if expires_in_secs < MIN_EXPIRES_IN_SECS {
        return Err(format!(
            "expires_in_secs must be >= {MIN_EXPIRES_IN_SECS} (1 minute minimum)"
        ));
    }
    if expires_in_secs > MAX_EXPIRES_IN_SECS {
        return Err(format!(
            "expires_in_secs must be <= {MAX_EXPIRES_IN_SECS} (24 hour maximum)"
        ));
    }

    // EPIC #329 Phase 3 §C.1: payment terms.
    //
    // If the LLM supplies explicit payment_* fields, use them. Otherwise
    // auto-populate a sensible default so EVERY commission cert carries
    // a payment clause Coral can claim against. Default semantics:
    //   - max_total: 50% of the budget_cap (so the recipient earns half
    //     the cap as fee, and Captain still has headroom for x402/proof
    //     spending against the commission's runtime budget)
    //   - amount_per_unit: same as max_total (one-shot payment, not
    //     per-unit metered)
    //   - unit: "commission" (one whole commission = one payment)
    //   - derivation_invoice: filled in below once we have commission_id
    //
    // The derivation_invoice is a per-commission BRC-29 invoice nonce that
    // the recipient uses to derive a unique receive address. We patch it
    // in once commission_id is generated (below in section 9).
    let user_payment = parse_payment_terms_input(&params)?;
    let payment_default_total = (budget_cap_sats / 2).max(1);
    let mut payment = user_payment.or(Some(PaymentTerms {
        amount_per_unit: payment_default_total,
        unit: "commission".to_string(),
        max_total: payment_default_total,
        derivation_invoice: String::new(), // patched after commission_id is generated
    }));

    // ── 2. Fetch self identity + reject self-delegation ────────────────
    let self_identity_key = wallet
        .get_identity_key()
        .await
        .map_err(|e| format!("wallet.get_identity_key failed: {e}"))?;
    if recipient.eq_ignore_ascii_case(&self_identity_key) {
        return Err("cannot delegate to self: recipient must be a different agent".into());
    }

    // ── 3. Compute purpose hash from the task description ─────────────
    let purpose_hash = compute_purpose_hash(&task);
    // Extract the raw hex portion for the serial number prefix. Format is
    // `sha256:<64hex>`; we take the first 8 chars after the prefix.
    let purpose_hash_prefix = purpose_hash
        .strip_prefix("sha256:")
        .map(|s| &s[..s.len().min(8)])
        .unwrap_or("00000000");

    // ── 4. Resolve root certifier (config trust root → parent cert → self) ───────
    let root_certifier =
        resolve_root_certifier(&*wallet, &self_identity_key, trust_root.as_deref()).await;

    // ── 5. Build timestamps + serial number ────────────────────────────
    let now = Utc::now();
    let issued_at = now;
    let expires_at = now + Duration::seconds(expires_in_secs);
    let unix_ms = now.timestamp_millis();
    let self_key_prefix = &self_identity_key[..self_identity_key.len().min(8)];
    let serial_number = format!("delegation-{self_key_prefix}-{purpose_hash_prefix}-{unix_ms}");

    // Patch the default PaymentTerms.derivation_invoice now that we have a
    // unique-per-commission identifier (serial_number). The recipient uses
    // this string as the BRC-29 invoice nonce when deriving the payment
    // receive address. Stable + unique = no derivation collisions across
    // commissions. Only patches if the user didn't supply their own.
    if let Some(ref mut p) = payment {
        if p.derivation_invoice.is_empty() {
            p.derivation_invoice = serial_number.clone();
        }
    }

    // ── 6. Create the revocation UTXO ──────────────────────────────────
    //
    // This is a tiny PushDrop output locked to the issuer's identity key.
    // Spending this UTXO (by the issuer) is the revocation mechanism.
    // We follow the same pattern as certificates/lifecycle.rs::
    // acquire_parent_authorization but use a dedicated delegation basket.
    //
    // The locking script is a simple OP_1 drop + OP_CHECKSIG against the
    // issuer's identity key, encoded as a minimal PushDrop so the wallet
    // can later spend it via listOutputs + spend_output.
    // EPIC #329 Phase 3 §C.3: include max_payment_sats so Captain's
    // commission_payment_claim handler can validate the claim amount
    // against the cert's declared cap by reading this UTXO from the
    // dm-delegation-revocation basket. No separate file storage needed —
    // the wallet IS the source of truth for what Captain has issued.
    let max_payment_sats = payment.as_ref().map(|p| p.max_total).unwrap_or(0);
    let revocation_data = json!({
        "type": "DelegationRevocation",
        "serial_number": serial_number,
        "subject": recipient,
        "certifier": self_identity_key,
        "purpose_hash": purpose_hash,
        "issued_at": issued_at.to_rfc3339(),
        "expires_at": expires_at.to_rfc3339(),
        "max_payment_sats": max_payment_sats,
    });
    let data_bytes = serde_json::to_vec(&revocation_data)
        .map_err(|e| format!("revocation data serialize: {e}"))?;
    let type_bytes = b"delegation_revocation".to_vec();
    let created_at_bytes = now.timestamp().to_string().into_bytes();
    let data_fields: Vec<&[u8]> = vec![&type_bytes, &data_bytes, &created_at_bytes];
    let locking_script =
        crate::onchain::state::build_push_drop_script(&data_fields, &self_identity_key)
            .map_err(|e| format!("build_push_drop_script failed: {e}"))?;

    let output = json!({
        "lockingScript": locking_script,
        "satoshis": REVOCATION_UTXO_SATS,
        "outputDescription": "dolphin milk delegation revocation",
        "basket": BASKET_DELEGATION_REVOCATION,
    });
    let action_result = wallet
        .create_action(
            &[output],
            "dolphin milk delegation revocation",
            false,
            false,
        )
        .await
        .map_err(|e| format!("create_action for revocation UTXO failed: {e}"))?;
    let revocation_txid = action_result.txid.clone();
    let revocation_vout: u32 = 0; // first (and only) output
    let revocation_outpoint = format!("{revocation_txid}.{revocation_vout}");

    // ── 6.5. Publish the revocation tx to the overlay (best-effort) ────
    //
    // The overlay's `tm_dm_delegation` topic indexes this UTXO so the
    // recipient can later query revocation status via `ls_dm_delegation`.
    // While the UTXO is in the unspent set, the cert is valid; once we
    // spend it (revoke), the overlay's spent-output handling removes the
    // record and recipient lookups return empty (== revoked).
    //
    // Best-effort: failures here log a warning but don't fail the tool.
    // The cert is still valid and deliverable; the worst case is that
    // the recipient's revocation checks return Unknown (treated as
    // not-revoked in v1) until the next overlay sync. Operators can
    // re-publish manually via the overlay /submit endpoint.
    if !overlay_url.is_empty() && !action_result.tx.is_empty() {
        if let Err(e) = submit_revocation_to_overlay(
            &overlay_url,
            &action_result.tx,
            &serial_number,
            &revocation_txid,
        )
        .await
        {
            tracing::warn!(
                serial = %serial_number,
                outpoint = %revocation_outpoint,
                error = %e,
                "delegate_task: overlay /submit failed; cert is still deliverable but \
                 recipients may not be able to query revocation status until \
                 the tx is re-published or naturally observed by the overlay"
            );
        }
    } else {
        tracing::debug!(
            serial = %serial_number,
            "delegate_task: skipping overlay submit (overlay_url empty or tx bytes missing)"
        );
    }

    // ── 7. Assemble the cert and sign its canonical body ──────────────
    //
    // We first build the cert with an empty signature, serialize to canonical
    // form, sign those bytes, then stamp the hex signature back on the cert.
    let mut cert = DelegationCert {
        subject: recipient.clone(),
        certifier: self_identity_key.clone(),
        serial_number: serial_number.clone(),
        revocation_outpoint: revocation_outpoint.clone(),
        signature: String::new(), // filled in after signing
        version: DELEGATION_VERSION_V1.to_string(),
        capabilities: capabilities.clone(),
        capability_args: capability_args.clone(),
        budget_cap_sats,
        expires_at,
        purpose_hash: purpose_hash.clone(),
        payment: payment.clone(),
        parent_cert_hash: None, // Phase 2 is single-hop only
        root_certifier: root_certifier.clone(),
        issued_at,
    };

    let canonical_bytes = canonicalize_cert(&cert.to_value())
        .map_err(|e| format!("canonicalize_cert failed: {e}"))?;
    let signature_bytes = wallet
        .create_signature(
            &canonical_bytes,
            &delegation_protocol_id(),
            "delegation",
            "anyone",
        )
        .await
        .map_err(|e| format!("wallet.create_signature failed: {e}"))?;
    cert.signature = hex::encode(&signature_bytes);

    // ── 8. Compute the cert hash (AFTER signing, for audit/return) ────
    // For the hash we use the canonical body (which excludes signature),
    // so that issuer and verifier compute the same hash independently.
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&canonical_bytes);
    let cert_hash = format!("sha256:{}", hex::encode(hasher.finalize()));

    // ── 9. Wrap in the commission envelope ─────────────────────────────
    let commission_id = uuid::Uuid::new_v4().to_string();
    let envelope = json!({
        "type": "task_delegation",
        "task": task,
        "delegation_cert": cert.to_value(),
        "delegation_chain": Value::Array(vec![]), // Phase 2: single-hop, always empty
        "commission_id": commission_id,
    });

    // ── 10. Dispatch to recipient's task_inbox ─────────────────────────
    //
    // sign=false, encrypt=false for Phase 2. The cert IS the trust
    // mechanism — a BRC-77 message-level signature would be redundant
    // with the BRC-77 cert signature. Encryption may land in a future phase.
    //
    // NOTE: If this send fails after the revocation UTXO and signature have
    // already been committed, the cert exists but was never delivered. That
    // UTXO is still unspent on-chain; the issuer can spend it later as a
    // cleanup no-op. We log a warning so operators can detect stranded certs.
    let sent = dispatcher
        .send(&recipient, BOX_TASK_INBOX, &envelope)
        .await
        .map_err(|e| {
            tracing::warn!(
                "delegate_task: send_message failed AFTER signing cert {serial_number}; \
                 revocation UTXO {revocation_outpoint} is stranded and can be spent \
                 by the issuer as cleanup. error: {e}"
            );
            format!("send_message to recipient task_inbox failed: {e}")
        })?;

    let sent_message_id = sent
        .get("sentMessageId")
        .or_else(|| sent.get("messageId"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    tracing::info!(
        "delegate_task: commissioned {} -> {} (commission={}, cert={}, rev_txid={})",
        &self_identity_key[..16.min(self_identity_key.len())],
        &recipient[..16.min(recipient.len())],
        &commission_id[..8.min(commission_id.len())],
        &cert_hash[..24.min(cert_hash.len())],
        &revocation_txid[..16.min(revocation_txid.len())],
    );

    let amount_for_result = payment.as_ref().map(|p| p.max_total).unwrap_or(0);
    Ok(DelegateResult {
        commission_id,
        serial_number: serial_number.clone(),
        delegation_cert_hash: cert_hash,
        revocation_txid,
        sent_message_id,
        recipient: recipient.clone(),
        amount_sats: amount_for_result,
    })
}

// ── Public factory ─────────────────────────────────────────────────────────

/// Build the `delegate_task` tool.
///
/// Wires the tool closure to the real wallet, shared MessageBox client, and
/// the overlay URL. The overlay URL comes from `config.overlay.submit_url` —
/// when non-empty, the tool will publish the revocation tx to the overlay's
/// `tm_dm_delegation` topic so peer recipients can query revocation status.
/// When empty (e.g., overlay disabled), the publish step is silently skipped
/// and the cert is still deliverable but recipients can't query revocation.
///
/// Registered in `runner/mod.rs` alongside the other category tools.
pub fn all_delegation_tools(
    wallet: Arc<dyn WalletBackend + Send + Sync>,
    messagebox: Arc<MessageBoxClient>,
    overlay_url: String,
    trust_root: Option<String>,
) -> Vec<ToolDef> {
    let dispatcher: Arc<dyn CommissionDispatcher> =
        Arc::new(MessageBoxDispatcher { client: messagebox });
    vec![ToolDef {
        name: "delegate_task".to_string(),
        description: "Delegate a task to another agent with a scoped, revocable, purpose-bound \
             capability grant. Constructs a BRC-52 delegation certificate with caveats \
             (capabilities, budget cap, expiry, purpose hash), signs it with BRC-77, \
             creates a revocation UTXO, and dispatches the commission via MessageBox to \
             the recipient's task_inbox. Use after discovering the recipient via \
             overlay_lookup. See the delegation skill for workflow details."
            .to_string(),
        parameters: json!({
            "type": "object",
            "required": [
                "recipient",
                "task",
                "capabilities",
                "budget_cap_sats",
                "expires_in_secs"
            ],
            "properties": {
                "recipient": {
                    "type": "string",
                    "description": "Recipient's 66-char hex identity_key (compressed secp256k1 pubkey). Must NOT be your own key. Discover via overlay_lookup first."
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the recipient. This exact string is hashed to produce the cert's purpose_hash, so the recipient can only use the cert for this task."
                },
                "capabilities": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool names the recipient may invoke while running this task. Example: [\"web_fetch\", \"memory_store\", \"pay_agent\"]. Non-empty."
                },
                "capability_args": {
                    "type": "object",
                    "description": "Optional per-tool argument scoping. Map from tool name to list of allowed arg prefixes. Example: {\"web_fetch\": [\"https://reddit.com/\", \"https://hn.com/\"]}. Unmentioned tools have no arg restrictions beyond the capabilities list."
                },
                "budget_cap_sats": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum sats the recipient may spend on this task."
                },
                "expires_in_secs": {
                    "type": "integer",
                    "minimum": 60,
                    "maximum": 86400,
                    "description": "Seconds from now until the delegation expires. Range: 60 (1 min) to 86400 (24 hours)."
                },
                "payment_amount_per_unit": {
                    "type": "integer",
                    "description": "Optional. Sats per unit of work (e.g., per record). Informational only; actual payment happens separately via pay_agent."
                },
                "payment_unit": {
                    "type": "string",
                    "description": "Optional. What the unit is (e.g., \"record\", \"byte\", \"call\")."
                },
                "payment_max_total": {
                    "type": "integer",
                    "description": "Optional. Max total sats this cert commits to paying."
                },
                "payment_derivation_invoice": {
                    "type": "string",
                    "description": "Optional. BRC-29 derivation invoice number the recipient should pre-compute."
                }
            }
        }),
        execute: {
            let wallet = wallet.clone();
            let dispatcher = dispatcher.clone();
            let overlay_url = overlay_url.clone();
            let trust_root = trust_root.clone();
            Box::new(move |params| {
                let w = wallet.clone();
                let d = dispatcher.clone();
                let url = overlay_url.clone();
                let tr = trust_root.clone();
                Box::pin(delegate_task_impl(params, w, d, url, tr))
            })
        },
        category: "delegation".to_string(),
        cleanup: None,
        deferred: false,
        always_load: true,
        search_hint: None,
    }]
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // ── Mock wallet ────────────────────────────────────────────────────

    struct MockWallet {
        identity_key: String,
        created_action_txid: String,
        signature: Vec<u8>,
    }

    impl MockWallet {
        fn new() -> Self {
            // Valid 66-char hex key (the secp256k1 generator point G)
            Self {
                identity_key: "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                    .to_string(),
                created_action_txid: "a".repeat(64),
                signature: vec![0xDEu8; 64],
            }
        }
    }

    #[async_trait]
    impl WalletBackend for MockWallet {
        async fn get_identity_key(&self) -> Result<String, crate::error::DmError> {
            Ok(self.identity_key.clone())
        }
        async fn get_public_key(
            &self,
            _p: &Value,
            _k: &str,
            _c: &str,
            _f: bool,
        ) -> Result<String, crate::error::DmError> {
            Ok(self.identity_key.clone())
        }
        async fn raw_call(
            &self,
            _m: &str,
            _p: Option<Value>,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn create_action(
            &self,
            _outputs: &[Value],
            _desc: &str,
            _delayed: bool,
            _rand: bool,
        ) -> Result<crate::wallet::CreateActionResult, crate::error::DmError> {
            Ok(crate::wallet::CreateActionResult {
                txid: self.created_action_txid.clone(),
                tx: vec![],
                raw: json!({}),
            })
        }
        async fn spend_output(
            &self,
            _b: &str,
            _t: &str,
            _v: u32,
            _d: &str,
        ) -> Result<crate::wallet::CreateActionResult, crate::error::DmError> {
            Ok(crate::wallet::CreateActionResult {
                txid: self.created_action_txid.clone(),
                tx: vec![],
                raw: json!({}),
            })
        }
        async fn internalize_action(
            &self,
            _tx: &[u8],
            _o: &[Value],
            _d: &str,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn get_balance(&self) -> Result<u64, crate::error::DmError> {
            Ok(0)
        }
        async fn list_outputs(
            &self,
            _b: &str,
            _i: &str,
            _l: u64,
            _o: u64,
        ) -> Result<Value, crate::error::DmError> {
            // Return empty outputs — no existing agent-auth cert found.
            Ok(json!({ "outputs": [] }))
        }
        async fn relinquish_output(
            &self,
            _b: &str,
            _t: &str,
            _v: u32,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn create_signature(
            &self,
            _data: &[u8],
            _protocol_id: &Value,
            _key_id: &str,
            _counterparty: &str,
        ) -> Result<Vec<u8>, crate::error::DmError> {
            Ok(self.signature.clone())
        }
        async fn verify_signature(
            &self,
            _d: &[u8],
            _s: &[u8],
            _p: &Value,
            _k: &str,
            _c: &str,
        ) -> Result<bool, crate::error::DmError> {
            Ok(true)
        }
        async fn encrypt(
            &self,
            p: &[u8],
            _pr: &Value,
            _k: &str,
            _c: &str,
        ) -> Result<Vec<u8>, crate::error::DmError> {
            Ok(p.to_vec())
        }
        async fn decrypt(
            &self,
            c: &[u8],
            _pr: &Value,
            _k: &str,
            _co: &str,
        ) -> Result<Vec<u8>, crate::error::DmError> {
            Ok(c.to_vec())
        }
        async fn create_hmac(
            &self,
            _d: &[u8],
            _p: &Value,
            _k: &str,
            _c: &str,
        ) -> Result<Vec<u8>, crate::error::DmError> {
            Ok(vec![0u8; 32])
        }
        async fn verify_hmac(
            &self,
            _d: &[u8],
            _h: &[u8],
            _p: &Value,
            _k: &str,
            _c: &str,
        ) -> Result<bool, crate::error::DmError> {
            Ok(true)
        }
        async fn acquire_certificate(&self, _c: &Value) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn list_certificates(
            &self,
            _c: &[&str],
            _t: &[&str],
            _l: u64,
            _o: u64,
        ) -> Result<Value, crate::error::DmError> {
            // Return no certs — tool falls back to self as root certifier.
            Ok(json!({ "certificates": [] }))
        }
        async fn prove_certificate(
            &self,
            _c: &Value,
            _f: &[&str],
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn relinquish_certificate(&self, _c: &Value) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn discover_by_identity_key(
            &self,
            _k: &str,
            _t: Option<&str>,
            _l: u64,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn discover_by_attributes(
            &self,
            _a: &Value,
            _t: Option<&str>,
            _l: u64,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn reveal_counterparty_key_linkage(
            &self,
            _c: &str,
            _v: &str,
            _p: bool,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn reveal_specific_key_linkage(
            &self,
            _c: &str,
            _v: &str,
            _p: &Value,
            _k: &str,
            _pr: bool,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn is_authenticated(&self) -> Result<Value, crate::error::DmError> {
            Ok(json!({ "authenticated": true }))
        }
        async fn get_height(&self) -> Result<u64, crate::error::DmError> {
            Ok(0)
        }
        async fn get_network(&self) -> Result<String, crate::error::DmError> {
            Ok("mainnet".to_string())
        }
        async fn get_version(&self) -> Result<String, crate::error::DmError> {
            Ok("mock".to_string())
        }
        async fn wait_for_authentication(&self) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn get_header_for_height(&self, _h: u64) -> Result<String, crate::error::DmError> {
            Ok("".to_string())
        }
        async fn sign_action(&self, _r: &str) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn abort_action(&self, _r: &str) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
        async fn list_actions(
            &self,
            _l: &[&str],
            _la: bool,
            _i: bool,
            _o: bool,
            _li: u64,
            _of: u64,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({ "actions": [] }))
        }
        async fn receive_address(
            &self,
            _s: &str,
        ) -> Result<(String, String, String), crate::error::DmError> {
            Ok(("".into(), "".into(), "".into()))
        }
        async fn fund_from_woc(
            &self,
            _t: &str,
            _v: Option<u32>,
            _s: Option<&str>,
        ) -> Result<Value, crate::error::DmError> {
            Ok(json!({}))
        }
    }

    // ── Recording dispatcher ───────────────────────────────────────────

    #[derive(Default)]
    struct RecordingDispatcher {
        calls: Mutex<Vec<(String, String, Value)>>,
        fail: bool,
    }

    #[async_trait]
    impl CommissionDispatcher for RecordingDispatcher {
        async fn send(
            &self,
            recipient: &str,
            message_box: &str,
            body: &Value,
        ) -> Result<Value, String> {
            self.calls.lock().unwrap().push((
                recipient.to_string(),
                message_box.to_string(),
                body.clone(),
            ));
            if self.fail {
                return Err("simulated dispatch failure".into());
            }
            Ok(json!({ "sentMessageId": "test-msg-id-123" }))
        }
    }

    fn other_key() -> String {
        // Valid 66-hex but different from MockWallet's identity.
        // Format: "03" + 64 hex chars = 66 total.
        "03".to_string() + &"ab".repeat(32)
    }

    fn base_params() -> Value {
        json!({
            "recipient": other_key(),
            "task": "fetch reddit top 10",
            "capabilities": ["web_fetch", "memory_store"],
            "budget_cap_sats": 100_000,
            "expires_in_secs": 600,
        })
    }

    async fn run_tool(params: Value) -> Result<DelegateResult, String> {
        let wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(MockWallet::new());
        let dispatcher: Arc<dyn CommissionDispatcher> = Arc::new(RecordingDispatcher::default());
        // Empty overlay_url disables the overlay /submit step (tests don't run a server).
        delegate_task_inner(params, wallet, dispatcher, String::new(), None).await
    }

    // Run the tool AND return the dispatcher so tests can inspect the
    // commission envelope that was sent.
    async fn run_tool_with_recorder(
        params: Value,
    ) -> (Result<DelegateResult, String>, Arc<RecordingDispatcher>) {
        let wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(MockWallet::new());
        let recorder = Arc::new(RecordingDispatcher::default());
        let dispatcher: Arc<dyn CommissionDispatcher> = recorder.clone();
        let res = delegate_task_inner(params, wallet, dispatcher, String::new(), None).await;
        (res, recorder)
    }

    // ── Happy path ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn happy_path_returns_full_result() {
        let (res, _rec) = run_tool_with_recorder(base_params()).await;
        let result = res.expect("happy path should succeed");
        assert!(!result.commission_id.is_empty());
        assert!(result.delegation_cert_hash.starts_with("sha256:"));
        assert_eq!(result.delegation_cert_hash.len(), "sha256:".len() + 64);
        assert!(!result.revocation_txid.is_empty());
        assert_eq!(result.sent_message_id, "test-msg-id-123");
    }

    // ── Validation failures ────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_self_delegation() {
        let mw = MockWallet::new();
        let mut p = base_params();
        p["recipient"] = json!(mw.identity_key);
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("cannot delegate to self"));
    }

    #[tokio::test]
    async fn rejects_short_recipient() {
        let mut p = base_params();
        p["recipient"] = json!("short");
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("66-char hex"));
    }

    #[tokio::test]
    async fn rejects_non_hex_recipient() {
        let mut p = base_params();
        p["recipient"] = json!("z".repeat(66));
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("66-char hex"));
    }

    #[tokio::test]
    async fn rejects_empty_capabilities() {
        let mut p = base_params();
        p["capabilities"] = json!([]);
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("non-empty"));
    }

    #[tokio::test]
    async fn rejects_zero_budget() {
        let mut p = base_params();
        p["budget_cap_sats"] = json!(0);
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("> 0") || err.contains("positive"));
    }

    #[tokio::test]
    async fn rejects_expires_too_short() {
        let mut p = base_params();
        p["expires_in_secs"] = json!(30);
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains(">=") || err.contains("60"));
    }

    #[tokio::test]
    async fn rejects_expires_too_long() {
        let mut p = base_params();
        p["expires_in_secs"] = json!(86401);
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("<=") || err.contains("86400"));
    }

    #[tokio::test]
    async fn rejects_empty_task() {
        let mut p = base_params();
        p["task"] = json!("   ");
        let err = run_tool(p).await.unwrap_err();
        assert!(err.contains("task"));
    }

    // ── Correctness of the constructed cert ────────────────────────────

    #[tokio::test]
    async fn purpose_hash_matches_compute_purpose_hash() {
        let (_res, rec) = run_tool_with_recorder(base_params()).await;
        let calls = rec.calls.lock().unwrap();
        let envelope = &calls[0].2;
        let cert_val = &envelope["delegation_cert"];
        let ph = cert_val["fields"]["delegation_purpose_hash"]
            .as_str()
            .unwrap();
        assert_eq!(ph, compute_purpose_hash("fetch reddit top 10"));
    }

    #[tokio::test]
    async fn cert_roundtrips_through_from_value() {
        let (_res, rec) = run_tool_with_recorder(base_params()).await;
        let calls = rec.calls.lock().unwrap();
        let envelope = &calls[0].2;
        let cert_val = &envelope["delegation_cert"];
        let parsed = DelegationCert::from_value(cert_val).expect("Phase 1 parser round-trip");
        // Subject/certifier check: recipient is subject, self is certifier
        assert_eq!(parsed.subject, other_key());
        assert_eq!(
            parsed.certifier,
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
        assert!(!parsed.signature.is_empty());
        assert_eq!(parsed.capabilities, vec!["web_fetch", "memory_store"]);
        assert_eq!(parsed.budget_cap_sats, 100_000);
        assert!(parsed.parent_cert_hash.is_none(), "Phase 2 is single-hop");
    }

    #[tokio::test]
    async fn capability_args_flattened_to_wire_format() {
        let mut p = base_params();
        p["capability_args"] = json!({
            "web_fetch": ["https://reddit.com/", "https://hn.com/"],
            "x402_call": ["api.bsv"]
        });
        let (_res, rec) = run_tool_with_recorder(p).await;
        let calls = rec.calls.lock().unwrap();
        let cert_val = &calls[0].2["delegation_cert"];
        let parsed = DelegationCert::from_value(cert_val).unwrap();
        let args = parsed.capability_args.expect("capability_args present");
        // Phase 1's serialize_capability_args sorts args within each tool for
        // canonical determinism, so the round-tripped order is sorted.
        assert_eq!(
            args.get("web_fetch").unwrap(),
            &vec![
                "https://hn.com/".to_string(),
                "https://reddit.com/".to_string(),
            ]
        );
        assert_eq!(args.get("x402_call").unwrap(), &vec!["api.bsv".to_string()]);
    }

    #[tokio::test]
    async fn optional_payment_terms_populate_cert() {
        let mut p = base_params();
        p["payment_amount_per_unit"] = json!(1);
        p["payment_unit"] = json!("record");
        p["payment_max_total"] = json!(500);
        p["payment_derivation_invoice"] = json!("inv-xyz-123");
        let (res, rec) = run_tool_with_recorder(p).await;
        res.expect("happy path with payment");
        let calls = rec.calls.lock().unwrap();
        let cert_val = &calls[0].2["delegation_cert"];
        let parsed = DelegationCert::from_value(cert_val).unwrap();
        let pay = parsed.payment.expect("payment terms present");
        assert_eq!(pay.amount_per_unit, 1);
        assert_eq!(pay.unit, "record");
        assert_eq!(pay.max_total, 500);
        assert_eq!(pay.derivation_invoice, "inv-xyz-123");
    }

    // ── Commission envelope shape ──────────────────────────────────────

    #[tokio::test]
    async fn commission_envelope_has_required_fields() {
        let (res, rec) = run_tool_with_recorder(base_params()).await;
        let result = res.unwrap();
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let envelope = &calls[0].2;
        assert_eq!(envelope["type"], "task_delegation");
        assert_eq!(envelope["task"], "fetch reddit top 10");
        assert!(envelope.get("delegation_cert").is_some());
        assert_eq!(envelope["delegation_chain"], json!([]));
        assert_eq!(envelope["commission_id"], result.commission_id);
    }

    #[tokio::test]
    async fn dispatches_to_task_inbox_not_status_inbox() {
        let (_res, rec) = run_tool_with_recorder(base_params()).await;
        let calls = rec.calls.lock().unwrap();
        assert_eq!(
            calls[0].1, "task_inbox",
            "must dispatch to task_inbox (BUG-006)"
        );
    }

    // ── Tool registration ──────────────────────────────────────────────

    #[tokio::test]
    async fn factory_produces_single_always_on_tool() {
        let wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(MockWallet::new());
        let mb_auth = crate::auth::AuthriteClient::new(wallet.clone(), "http://localhost:3322");
        let messagebox = Arc::new(MessageBoxClient::new(mb_auth));
        let tools = all_delegation_tools(wallet, messagebox, String::new(), None);
        assert_eq!(tools.len(), 1);
        let t = &tools[0];
        assert_eq!(t.name, "delegate_task");
        assert_eq!(t.category, "delegation");
        assert!(t.always_load, "delegate_task must be always-on in prompts");
        assert!(!t.deferred);
    }
}
