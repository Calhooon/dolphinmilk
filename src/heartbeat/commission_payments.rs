//! EPIC #329 Phase 3 §C.3 — Captain-side and Coral-side handlers for the
//! commission payment flow.
//!
//! Two MessageBox body types are handled here, both server-side (no LLM
//! consultation, no task spawn) so payment is deterministic and atomic.
//!
//! ## Direction
//!
//! ```text
//!  CORAL                        Captain
//!    │                            │
//!    │  commission_payment_claim  │   handle_commission_payment_claim:
//!    ├───────────────────────────►│   - validate against dm-delegation-revocation
//!    │                            │   - wallet.create_action(P2PKH → claimant)
//!    │                            │   - record budget + transcript
//!    │                            │
//!    │   commission_payment_sent  │
//!    │◄───────────────────────────┤
//!    │                            │
//!    │  handle_commission_payment_sent:
//!    │  - parse beef_hex
//!    │  - wallet.fund_from_tx (BRC-29 internalize)
//!    │  - record receipt
//! ```
//!
//! ## Validation rules (Captain side)
//!
//! 1. Look up the cert's revocation UTXO in `dm-delegation-revocation` by
//!    matching `serialNumber`. The wallet IS the source of truth for what
//!    Captain has issued — no separate file storage needed.
//! 2. The claim's `claimant_identity_key` MUST equal the revocation UTXO's
//!    `subject` field. (Only the original recipient can claim.)
//! 3. The claim's `amount_sats` MUST be ≤ the revocation UTXO's
//!    `max_payment_sats` (cert-declared cap).
//! 4. Hard cap: amount must be ≤ MAX_COMMISSION_PAYMENT_SATS to prevent
//!    misconfigured certs from draining a wallet. 1M sats for v1.
//!
//! Phase 3 v1 does NOT track double-payment. A malicious recipient could
//! submit the same claim twice. Hardcoded sanity cap + (future) paid-set in
//! workspace. For the hackathon E2E it's fine because we run one round.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::error::DmError;
use crate::messagebox::client::MessageBoxClient;
use crate::messagebox::types::BOX_STATUS_INBOX;
use crate::wallet::WalletBackend;

/// Hard sanity cap on commission payouts. Prevents a misconfigured cert
/// or a buggy claim from draining the wallet.
pub const MAX_COMMISSION_PAYMENT_SATS: u64 = 1_000_000;

/// Basket name where Captain's delegate_task stores per-commission
/// revocation UTXOs. Mirrors src/tools/delegation_tools.rs.
const BASKET_DELEGATION_REVOCATION: &str = "dm-delegation-revocation";

/// Result of a commission_payment_claim handling pass.
#[derive(Debug, Clone)]
pub struct PaymentResult {
    pub success: bool,
    pub txid: Option<String>,
    pub sats_paid: u64,
    pub reason: String,
}

impl PaymentResult {
    fn ok(txid: String, sats_paid: u64) -> Self {
        Self {
            success: true,
            txid: Some(txid),
            sats_paid,
            reason: format!("paid {sats_paid} sats"),
        }
    }
    fn rejected(reason: impl Into<String>) -> Self {
        Self {
            success: false,
            txid: None,
            sats_paid: 0,
            reason: reason.into(),
        }
    }
}

/// Captain-side: handle a `commission_payment_claim` MessageBox body.
///
/// Validates against the wallet's `dm-delegation-revocation` basket and,
/// on success, broadcasts a P2PKH payment tx to the claimant's address
/// then sends a `commission_payment_sent` reply with the BEEF so the
/// claimant can internalize.
///
/// `global_workspace`, when provided, also surfaces the payment in the
/// agent's `conv-commission-ledger` conversation as a system message — a
/// running ledger of every commission this agent has paid out, visible in
/// the UI conversation sidebar. `None` skips the surfacing.
///
/// All wallet operations are best-effort with logged failures — never
/// panics, never fails the heartbeat loop.
pub async fn handle_commission_payment_claim(
    wallet: Arc<dyn WalletBackend + Send + Sync>,
    messagebox: Arc<MessageBoxClient>,
    claim: &Value,
    claimant_sender: &str,
    global_workspace: Option<&std::path::Path>,
) -> PaymentResult {
    // Extract claim fields
    let serial = claim
        .get("commission_serial")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let amount_sats = claim
        .get("amount_sats")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let receive_address = claim
        .get("receive_address")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let claimant_identity_key = claim
        .get("claimant_identity_key")
        .and_then(|v| v.as_str())
        .unwrap_or(claimant_sender);

    if serial.is_empty() || amount_sats == 0 || receive_address.is_empty() {
        return PaymentResult::rejected("missing serial / amount_sats / receive_address");
    }
    if amount_sats > MAX_COMMISSION_PAYMENT_SATS {
        return PaymentResult::rejected(format!(
            "amount {amount_sats} exceeds hard cap {MAX_COMMISSION_PAYMENT_SATS}"
        ));
    }

    // Look up the matching revocation UTXO in our basket. The PushDrop's
    // second field carries the JSON revocation_data we wrote in delegate_task.
    let outputs = match wallet
        .list_outputs(BASKET_DELEGATION_REVOCATION, "locking scripts", 200, 0)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return PaymentResult::rejected(format!("list_outputs failed: {e}"));
        }
    };
    let outputs_arr = outputs
        .get("outputs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut matching_max_payment: Option<u64> = None;
    let mut matching_subject: Option<String> = None;
    for output in &outputs_arr {
        let script_hex = output
            .get("lockingScript")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if script_hex.is_empty() {
            continue;
        }
        let fields = match crate::onchain::state::parse_push_drop_fields(script_hex) {
            Some(f) => f,
            None => continue,
        };
        // Field layout from delegate_task: [type, data, created_at]
        if fields.len() < 2 {
            continue;
        }
        let data_hex = &fields[1];
        let data_bytes = match hex::decode(data_hex) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let parsed: Value = match serde_json::from_slice(&data_bytes) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let row_serial = parsed
            .get("serial_number")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if row_serial != serial {
            continue;
        }
        // Match — extract validation fields
        matching_subject = parsed
            .get("subject")
            .and_then(|v| v.as_str())
            .map(String::from);
        matching_max_payment = parsed.get("max_payment_sats").and_then(|v| v.as_u64());
        break;
    }

    let max_payment = match matching_max_payment {
        Some(m) => m,
        None => {
            return PaymentResult::rejected(format!(
                "no commission found in dm-delegation-revocation for serial '{serial}'"
            ));
        }
    };
    let cert_subject = matching_subject.unwrap_or_default();

    if cert_subject != claimant_identity_key {
        return PaymentResult::rejected(format!(
            "claimant_identity_key {} does not match cert subject {}",
            &claimant_identity_key[..16.min(claimant_identity_key.len())],
            &cert_subject[..16.min(cert_subject.len())]
        ));
    }
    if amount_sats > max_payment {
        return PaymentResult::rejected(format!(
            "amount_sats {amount_sats} exceeds cert max_payment_sats {max_payment}"
        ));
    }

    // Build the payment tx: a single P2PKH output to the claimant's
    // receive_address. Decode the address back to the 20-byte pubkey hash
    // and construct the standard P2PKH locking script.
    let locking_script_hex = match address_to_p2pkh_script_hex(receive_address) {
        Ok(s) => s,
        Err(e) => {
            return PaymentResult::rejected(format!(
                "address decode failed for {receive_address}: {e}"
            ));
        }
    };

    let outputs = vec![json!({
        "lockingScript": locking_script_hex,
        "satoshis": amount_sats,
        "outputDescription": format!("commission payment for {serial}"),
    })];
    let result = match wallet
        .create_action(
            &outputs,
            &format!("Commission payment for {serial}"),
            false,
            false,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return PaymentResult::rejected(format!("create_action failed: {e}"));
        }
    };

    let payment_txid = result.txid.clone();
    let payment_beef = result.tx.clone();

    tracing::info!(
        serial = %serial,
        recipient = %&claimant_identity_key[..16.min(claimant_identity_key.len())],
        amount_sats,
        payment_txid = %payment_txid,
        "commission payment broadcast"
    );

    // Surface the outgoing payment in the user-facing conversation.
    //
    // Preferred: the original chat conversation that issued delegate_task
    // (looked up via the commission_conv_index file the runner wrote when
    // delegate_task completed). The user sees the payment appear at the
    // bottom of the same conversation thread where they delegated the
    // work — natural UX, just like Coral's commission conversation.
    //
    // Fallback: a global `conv-commission-ledger` conversation that
    // accumulates every commission payout. Used when the index lookup
    // fails (e.g., delegate_task ran in CLI mode without conversation
    // binding, or the index file was cleaned up).
    //
    // Best-effort. Never fails the payment.
    if let Some(ws) = global_workspace {
        let mgr = crate::session::conversation::ConversationManager::new(ws);
        let recipient_short = &claimant_identity_key[..16.min(claimant_identity_key.len())];
        let txid_short = &payment_txid[..16.min(payment_txid.len())];
        let content = format!(
            "💸 Paid {amount_sats} sats to {recipient_short}... — \
             commission {serial} — txid {txid_short}..."
        );

        // Look up the original chat conversation via the index file.
        let index_path = ws
            .join("commission_conv_index")
            .join(format!("{serial}.txt"));
        let chat_conv_id = std::fs::read_to_string(&index_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let target_conv = match chat_conv_id {
            Some(ref id) if mgr.load(id).ok().flatten().is_some() => {
                tracing::debug!(
                    serial,
                    conv_id = %id,
                    "Surfacing commission payment in original chat conversation"
                );
                id.clone()
            }
            _ => {
                let ledger_id = "conv-commission-ledger";
                if mgr.load(ledger_id).ok().flatten().is_none() {
                    let _ =
                        mgr.create_with_id(ledger_id, "commission:ledger", "Commission Payments");
                }
                tracing::debug!(
                    serial,
                    "No chat conv index found — falling back to commission ledger"
                );
                ledger_id.to_string()
            }
        };

        if let Err(e) = mgr.append_system_message(&target_conv, &content, None) {
            tracing::warn!(
                serial,
                target_conv,
                "Failed to append payment to conversation: {e}"
            );
        } else {
            tracing::info!(
                serial,
                target_conv,
                "Surfaced outgoing commission payment in conversation"
            );
        }
    }

    // Send the payment back to the claimant via MessageBox so they can
    // internalize via fund_from_tx.
    let reply_body = json!({
        "type": "commission_payment_sent",
        "commission_serial": serial,
        "amount_sats": amount_sats,
        "payment_txid": payment_txid,
        "beef_hex": hex::encode(&payment_beef),
        "derivation_invoice": claim
            .get("derivation_invoice")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    });

    if let Err(e) = messagebox
        .send_message(claimant_sender, BOX_STATUS_INBOX, &reply_body)
        .await
    {
        tracing::warn!(
            serial = %serial,
            "commission payment broadcast OK but reply send failed: {e} — \
             claimant won't auto-internalize until next sync"
        );
    }

    PaymentResult::ok(payment_txid, amount_sats)
}

/// Coral-side: handle a `commission_payment_sent` reply from Captain by
/// internalizing the payment tx into the wallet via the BRC-29 funding
/// flow.
///
/// The reply carries the spending tx as `beef_hex` and the
/// `derivation_invoice` (= cert serial_number) that was used as the
/// receive_address suffix at claim time. The wallet must be told the
/// same suffix to derive the matching unlocking key for the P2PKH output.
///
/// `global_workspace`, when provided, enables surfacing the payment
/// receipt as a system message in the original commission conversation
/// (looked up via the serial→conv_id index that
/// `runner::emit_commission_payment_claim()` writes at claim time).
/// `None` skips the surfacing — used by call sites without workspace
/// context (tests, etc.).
pub async fn handle_commission_payment_sent(
    wallet: Arc<dyn WalletBackend + Send + Sync>,
    body: &Value,
    global_workspace: Option<&std::path::Path>,
) -> PaymentResult {
    let serial = body
        .get("commission_serial")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let amount_sats = body
        .get("amount_sats")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let payment_txid = body
        .get("payment_txid")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let beef_hex = body.get("beef_hex").and_then(|v| v.as_str()).unwrap_or("");
    let derivation_invoice = body
        .get("derivation_invoice")
        .and_then(|v| v.as_str())
        .unwrap_or(serial);

    if beef_hex.is_empty() || payment_txid.is_empty() {
        return PaymentResult::rejected("missing beef_hex or payment_txid");
    }

    let beef_bytes = match hex::decode(beef_hex) {
        Ok(b) => b,
        Err(e) => {
            return PaymentResult::rejected(format!("beef_hex decode failed: {e}"));
        }
    };

    // Internalize directly from the BEEF the payer included in the
    // `commission_payment_sent` message. Previously this path called
    // `fund_from_woc(payment_txid, ...)` which round-tripped to
    // WhatsOnChain to re-fetch a transaction we ALREADY HAD in hand —
    // and hit a race: WoC indexing lags broadcast by 10-60s, so the
    // receiver would get HTTP 404 on the freshly-broadcast tx and
    // permanently lose the commission payment (no retry, no re-queue).
    //
    // The sender has already built and broadcast the tx and included
    // the authoritative BEEF in the message body. Trust it. The BEEF
    // carries its own SPV proofs, so the receiver can verify without
    // asking any external service.
    //
    // Fallback to WoC only if the BEEF-direct path fails (e.g. malformed
    // BEEF, wallet rejection) — that preserves the prior behavior as a
    // safety net without taking the WoC hit on the happy path.
    let direct_result =
        try_internalize_beef(&wallet, &beef_bytes, payment_txid, derivation_invoice).await;
    let fund_result = match direct_result {
        Ok(v) => Ok(v),
        Err(direct_err) => {
            tracing::warn!(
                payment_txid = %payment_txid,
                "Direct BEEF internalize failed ({direct_err}); falling back to WoC fetch"
            );
            wallet
                .fund_from_woc(payment_txid, None, Some(derivation_invoice))
                .await
        }
    };
    match fund_result {
        Ok(_) => {
            tracing::info!(
                serial = %serial,
                amount_sats,
                payment_txid = %payment_txid,
                "commission payment internalized"
            );

            // Surface the receipt in the original commission conversation
            // via the serial → conv_id index. Best-effort, never fails.
            if let Some(ws) = global_workspace {
                let index_path = ws
                    .join("commission_conv_index")
                    .join(format!("{serial}.txt"));
                if let Ok(conv_id) = std::fs::read_to_string(&index_path) {
                    let conv_id = conv_id.trim();
                    if !conv_id.is_empty() {
                        let mgr = crate::session::conversation::ConversationManager::new(ws);
                        let content = format!(
                            "✅ Commission payment received — {amount_sats} sats from issuer \
                             (txid {}, commission {serial})",
                            &payment_txid[..16.min(payment_txid.len())]
                        );
                        if let Err(e) = mgr.append_system_message(conv_id, &content, None) {
                            tracing::warn!(
                                conv_id,
                                "Failed to append payment receipt system message: {e}"
                            );
                        } else {
                            tracing::info!(
                                conv_id,
                                "Surfaced commission payment receipt in conversation"
                            );
                        }
                    }
                }
            }

            PaymentResult::ok(payment_txid.to_string(), amount_sats)
        }
        Err(e) => PaymentResult::rejected(format!("fund_from_woc failed: {e}")),
    }
}

/// Internalize a commission payment directly from the BEEF bytes the payer
/// included in the `commission_payment_sent` message. Bypasses the WoC
/// round-trip that races against broadcast indexing.
///
/// Path:
///   1. Wrap the raw BEEF in AtomicBEEF (magic 0x01010101 + reversed txid).
///   2. Build the `paymentRemittance` descriptor with the same
///      derivation prefix (`dolphin-milk-fund`) and sender
///      (`ANYONE_KEY`) that `receive_address` / `fund_from_woc` use.
///   3. Call `wallet.internalize_action()` directly.
///
/// Returns `Ok(Value)` on acceptance or `Err(DmError)` with the wallet's
/// reason on rejection. Malformed BEEF, script mismatch, or already-
/// internalized are all surfaced via Err so the caller can fall back to
/// the WoC path.
async fn try_internalize_beef(
    wallet: &Arc<dyn WalletBackend + Send + Sync>,
    beef_bytes: &[u8],
    payment_txid: &str,
    derivation_suffix: &str,
) -> Result<Value, DmError> {
    if payment_txid.len() != 64 {
        return Err(DmError::wallet(format!(
            "invalid payment_txid length {}",
            payment_txid.len()
        )));
    }
    // Build AtomicBEEF if the payload isn't already wrapped. AtomicBEEF
    // format: `0x01 0x01 0x01 0x01` magic + reversed 32-byte txid + BEEF.
    let is_already_atomic = beef_bytes.len() >= 4
        && beef_bytes[0] == 0x01
        && beef_bytes[1] == 0x01
        && beef_bytes[2] == 0x01
        && beef_bytes[3] == 0x01;
    let atomic_beef: Vec<u8> = if is_already_atomic {
        beef_bytes.to_vec()
    } else {
        let mut txid_bytes = hex::decode(payment_txid)
            .map_err(|e| DmError::wallet(format!("invalid hex in payment_txid: {e}")))?;
        txid_bytes.reverse();
        let mut ab = Vec::with_capacity(4 + 32 + beef_bytes.len());
        ab.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
        ab.extend_from_slice(&txid_bytes);
        ab.extend_from_slice(beef_bytes);
        ab
    };

    // Find the vout whose locking script matches the receiver's BRC-29
    // derived key. We don't know the vout up-front, so let the wallet
    // auto-detect by passing vout 0 with the derivation suffix — if that
    // fails, the wallet's internalize_action will return a clear error
    // and we'll fall through to the WoC path which has its own auto-detect.
    //
    // NOTE: This is a shortcut: in practice commission payment txs from
    // our own `createAction` path pay the recipient at vout 0, so this
    // works. The fallback path handles any edge cases.
    let outputs = vec![json!({
        "outputIndex": 0u32,
        "protocol": "wallet payment",
        "paymentRemittance": {
            "derivationPrefix": "dolphin-milk-fund",
            "derivationSuffix": derivation_suffix,
            "senderIdentityKey": crate::wallet::ANYONE_KEY,
        },
    })];

    let result = wallet
        .internalize_action(&atomic_beef, &outputs, "commission payment (direct BEEF)")
        .await?;

    // internalize_action returns 200 with a body even on semantic error.
    // Check for the `accepted: true` field. If missing, surface as Err so
    // the caller falls back to WoC.
    let accepted = result
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !accepted {
        return Err(DmError::wallet(format!(
            "wallet rejected direct BEEF internalize: {result}"
        )));
    }
    Ok(result)
}

/// Decode a Base58 BSV address back into a P2PKH locking script hex.
///
/// Inverse of `tools::wallet_tools::script_to_address`.
fn address_to_p2pkh_script_hex(address: &str) -> Result<String, String> {
    let payload = base58_decode(address).map_err(|e| format!("base58 decode: {e}"))?;
    if payload.len() != 25 {
        return Err(format!(
            "decoded payload length {} (expected 25 = version + hash160 + checksum)",
            payload.len()
        ));
    }
    // payload[0] = version byte (0x00 for mainnet P2PKH)
    // payload[1..21] = 20-byte hash160
    // payload[21..25] = checksum
    let hash160 = &payload[1..21];
    let mut script = Vec::with_capacity(25);
    script.push(0x76); // OP_DUP
    script.push(0xa9); // OP_HASH160
    script.push(0x14); // push 20 bytes
    script.extend_from_slice(hash160);
    script.push(0x88); // OP_EQUALVERIFY
    script.push(0xac); // OP_CHECKSIG
    Ok(hex::encode(script))
}

/// Bitcoin Base58 decoder. Mirrors the encoder in tools::wallet_tools.
fn base58_decode(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut result = vec![0u8; input.len()];
    let mut written = 0;
    for c in input.bytes() {
        let mut carry = ALPHABET
            .iter()
            .position(|&b| b == c)
            .ok_or_else(|| format!("invalid base58 char: {c}"))?;
        for byte in result.iter_mut().take(written) {
            carry += (*byte as usize) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            result[written] = (carry & 0xff) as u8;
            written += 1;
            carry >>= 8;
        }
    }
    // Handle leading '1's (zeros)
    let leading_ones = input.bytes().take_while(|&b| b == b'1').count();
    let mut output = vec![0u8; leading_ones];
    output.extend(result[..written].iter().rev());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn base58_round_trip_known_vector() {
        // Empty hash160 → "1111111111111111111114oLvT2"
        // Use a real Bitcoin test vector instead.
        let address = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"; // Satoshi's first address
        let decoded = base58_decode(address).unwrap();
        assert_eq!(decoded.len(), 25);
        assert_eq!(decoded[0], 0x00); // mainnet version
    }

    #[test]
    fn address_to_script_round_trip() {
        // Encode → decode round trip via the wallet_tools helpers we mirror
        let hash = [0x01u8; 20];
        let mut payload = vec![0x00u8];
        payload.extend_from_slice(&hash);
        let checksum = sha2::Sha256::digest(sha2::Sha256::digest(&payload));
        payload.extend_from_slice(&checksum[..4]);
        let address = crate::tools::wallet_tools::base58_encode(&payload);

        let script_hex = address_to_p2pkh_script_hex(&address).unwrap();
        // 76 a9 14 <20 bytes> 88 ac
        assert_eq!(script_hex.len(), 50);
        assert!(script_hex.starts_with("76a914"));
        assert!(script_hex.ends_with("88ac"));
        let body = &script_hex[6..46];
        assert_eq!(body, hex::encode(hash));
    }
}
