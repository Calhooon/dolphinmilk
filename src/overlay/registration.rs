//! AGENT PushDrop registration — build, sign, submit to overlay.
//!
//! Flow:
//! 1. Get identity key from wallet
//! 2. Build 6 unsigned fields: AGENT, identity, certifier, endpoint, capabilities
//! 3. Sign concatenated fields[0..5] via BRC-42 derived key
//! 4. Get BRC-42 derived locking key (forSelf = true)
//! 5. Build PushDrop with locking key + 6 fields
//! 6. Create transaction via wallet → get BEEF
//! 7. POST BEEF to overlay /submit with x-topics: ["tm_agent"]

use serde_json::json;

use bsv::primitives::ec::PublicKey;
use bsv::script::templates::PushDrop;

use crate::error::DmError;
use crate::wallet::WalletBackend;

use super::{
    agent_registry_protocol_id, AGENT_REGISTRY_COUNTERPARTY, AGENT_REGISTRY_KEY_ID,
    BASKET_AGENT_REGISTRATION,
};

/// Check if this agent is already registered on the overlay (by identity key).
/// Returns the existing registration's name and capabilities if found.
pub async fn check_registered(
    wallet: &dyn WalletBackend,
    overlay_url: &str,
) -> Result<Option<super::lookup::AgentRecord>, DmError> {
    let identity_key = wallet.get_identity_key().await?;

    let query = json!({
        "service": "ls_agent",
        "query": {"findByIdentityKey": identity_key}
    });

    let resp = reqwest::Client::new()
        .post(format!("{overlay_url}/lookup"))
        .header("Content-Type", "application/json")
        .json(&query)
        .send()
        .await
        .map_err(|e| DmError::wallet(format!("Overlay lookup failed: {e}")))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| DmError::wallet(format!("Overlay response parse error: {e}")))?;

    let records = super::lookup::parse_agent_records(&body);
    Ok(records.into_iter().find(|r| r.identity_key == identity_key))
}

/// Deregister from the overlay by spending **every** wallet-tracked
/// registration UTXO and submitting each spending tx to the overlay so the
/// engine prunes the corresponding agent records.
///
/// EPIC #329 Phase 3: previously this only spent the first 10 outputs from a
/// single `list_outputs` call. Wallets that had accumulated more than 10
/// stale registrations across many test runs would leave records on the
/// overlay forever. Now we paginate-and-spend in a loop until the basket
/// is empty (or we make zero progress in a pass — guard against infinite
/// loops on a partial-failure scenario).
///
/// Returns the number of UTXOs successfully spent. Errors from individual
/// spends are logged and skipped, never bubbled up — the caller should
/// continue with re-registration even if some old UTXOs couldn't be spent.
pub async fn deregister_from_overlay(
    wallet: &dyn WalletBackend,
    overlay_url: &str,
) -> Result<usize, DmError> {
    const PAGE_SIZE: u64 = 100;
    const MAX_PASSES: usize = 50;

    let mut total_spent: usize = 0;

    for pass in 1..=MAX_PASSES {
        let outputs = wallet
            .list_outputs(BASKET_AGENT_REGISTRATION, "locking scripts", PAGE_SIZE, 0)
            .await?;

        let outputs_arr = outputs
            .get("outputs")
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();

        if outputs_arr.is_empty() {
            if total_spent > 0 {
                tracing::info!(
                    spent = total_spent,
                    passes = pass - 1,
                    "Overlay deregistration complete — basket empty"
                );
            }
            return Ok(total_spent);
        }

        tracing::info!(
            pass,
            in_basket = outputs_arr.len(),
            "Spending stale agent-registration UTXOs (pass {pass})"
        );

        let mut spent_this_pass: usize = 0;

        for output in &outputs_arr {
            let txid = output.get("txid").and_then(|v| v.as_str());
            let vout = output.get("vout").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let Some(txid) = txid else {
                tracing::warn!("Output in dm-agent-registration basket missing txid: {output}");
                continue;
            };

            // Step 1: spend the UTXO via the official wallet path
            let result = match wallet
                .spend_output(
                    BASKET_AGENT_REGISTRATION,
                    txid,
                    vout,
                    "Overlay deregistration — replacing with updated registration",
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        txid = %txid,
                        vout,
                        "Failed to spend stale registration UTXO: {e}"
                    );
                    continue;
                }
            };

            // Step 2: submit the spending tx to the overlay so it prunes
            // the corresponding agent record. Best-effort — if the submit
            // fails, the spend is still on-chain and the overlay will catch
            // it on the next sync.
            let client = reqwest::Client::new();
            match client
                .post(format!("{overlay_url}/submit"))
                .header("Content-Type", "application/octet-stream")
                .header("x-topics", r#"["tm_agent"]"#)
                .body(result.tx)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        tracing::info!(
                            old_txid = %txid,
                            spend_txid = %result.txid,
                            "Stale registration spent and submitted to overlay"
                        );
                        spent_this_pass += 1;
                        total_spent += 1;
                    } else {
                        tracing::warn!(
                            old_txid = %txid,
                            "Overlay submit failed (HTTP {status}): {body} — spend is still on-chain"
                        );
                        spent_this_pass += 1;
                        total_spent += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        old_txid = %txid,
                        "Overlay submit network error: {e} — spend is still on-chain"
                    );
                    spent_this_pass += 1;
                    total_spent += 1;
                }
            }
        }

        if spent_this_pass == 0 {
            // Made zero progress this pass — break to avoid infinite loop on a
            // wallet that's persistently failing to spend.
            tracing::warn!(
                pass,
                in_basket = outputs_arr.len(),
                "Deregistration loop made no progress this pass — giving up"
            );
            break;
        }
    }

    Ok(total_spent)
}

/// Register this agent on the overlay by publishing a 6-field AGENT PushDrop.
///
/// Fields:
/// - `[0]` `"AGENT"` — protocol tag
/// - `[1]` identity key — 33-byte compressed pubkey
/// - `[2]` certifier key — 33-byte compressed pubkey (from cert, or self)
/// - `[3]` name — agent display name (UTF-8, any non-empty string)
/// - `[4]` capabilities — comma-separated string (from cert)
/// - `[5]` DER ECDSA signature — over concatenated fields\[0..5\]
///
/// The PushDrop's locking key is a BRC-42 derived key (not the identity key).
/// Discovery is via identity key + MessageBox — no HTTP endpoint needed.
pub async fn register_on_overlay(
    wallet: &dyn WalletBackend,
    overlay_url: &str,
    name: &str,
    capabilities: &str,
    certifier_key: Option<&str>,
) -> Result<String, DmError> {
    let identity_key = wallet.get_identity_key().await?;
    let certifier = certifier_key.unwrap_or(&identity_key);

    // Convert hex pubkeys to raw bytes
    let identity_bytes = hex::decode(&identity_key)
        .map_err(|e| DmError::wallet(format!("Invalid identity key hex: {e}")))?;
    let certifier_bytes = hex::decode(certifier)
        .map_err(|e| DmError::wallet(format!("Invalid certifier key hex: {e}")))?;

    if identity_bytes.len() != 33 {
        return Err(DmError::wallet(format!(
            "Identity key must be 33 bytes, got {}",
            identity_bytes.len()
        )));
    }
    if certifier_bytes.len() != 33 {
        return Err(DmError::wallet(format!(
            "Certifier key must be 33 bytes, got {}",
            certifier_bytes.len()
        )));
    }

    // Build unsigned fields
    let field_0 = b"AGENT".to_vec();
    let field_1 = identity_bytes;
    let field_2 = certifier_bytes;
    let field_3 = name.as_bytes().to_vec();
    let field_4 = capabilities.as_bytes().to_vec();

    // Concatenate fields[0..5] for signing
    let sign_data: Vec<u8> = [
        field_0.as_slice(),
        field_1.as_slice(),
        field_2.as_slice(),
        field_3.as_slice(),
        field_4.as_slice(),
    ]
    .concat();

    // Sign with BRC-42 derived key
    let signature = wallet
        .create_signature(
            &sign_data,
            &agent_registry_protocol_id(),
            AGENT_REGISTRY_KEY_ID,
            AGENT_REGISTRY_COUNTERPARTY,
        )
        .await?;

    // Get the BRC-42 derived locking key (forSelf = true)
    let locking_key = wallet
        .get_public_key(
            &agent_registry_protocol_id(),
            AGENT_REGISTRY_KEY_ID,
            AGENT_REGISTRY_COUNTERPARTY,
            true,
        )
        .await?;

    let sign_data_hash = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(&sign_data))
    };
    tracing::info!(
        identity = %identity_key,
        locking_key = %locking_key,
        sign_data_len = sign_data.len(),
        sign_data_sha256 = %sign_data_hash,
        field0_hex = %hex::encode(&field_0),
        field1_hex = %hex::encode(&field_1),
        field2_hex = %hex::encode(&field_2),
        field3_hex = %hex::encode(field_3.as_slice()),
        field4_hex = %hex::encode(field_4.as_slice()),
        sig_len = signature.len(),
        sig_hex = %hex::encode(&signature),
        "Overlay registration: signing details (compare SHA-256 with CF Worker logs)"
    );

    // Build PushDrop locking script with 6 fields.
    // IMPORTANT: Use LockPosition::Before (the default) — the overlay's topic manager
    // decodes PushDrop with the default position. Our internal state tokens use After,
    // but overlay registration must match the overlay's decoder.
    let pubkey = PublicKey::from_hex(&locking_key)
        .map_err(|e| DmError::wallet(format!("Invalid locking key: {e}")))?;
    let pd_fields: Vec<Vec<u8>> = vec![field_0, field_1, field_2, field_3, field_4, signature];
    let pushdrop = PushDrop::new(pubkey, pd_fields);
    let script_hex = pushdrop.lock().to_hex();

    tracing::info!(
        script_len = script_hex.len() / 2,
        script_prefix = %&script_hex[..script_hex.len().min(80)],
        "Overlay registration: PushDrop script"
    );

    // Create transaction via wallet
    let output = json!({
        "lockingScript": script_hex,
        "satoshis": 1,
        "outputDescription": "AGENT overlay registration",
        "basket": BASKET_AGENT_REGISTRATION,
        "tags": ["overlay-agent-registration"],
    });

    let result = wallet
        .create_action(&[output], "Overlay agent registration", false, false)
        .await?;

    // Submit BEEF to overlay
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{overlay_url}/submit"))
        .header("Content-Type", "application/octet-stream")
        .header("x-topics", r#"["tm_agent"]"#)
        .body(result.tx)
        .send()
        .await
        .map_err(|e| DmError::wallet(format!("Overlay submit failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| DmError::wallet(format!("Overlay response read error: {e}")))?;

    if !status.is_success() {
        return Err(DmError::wallet(format!(
            "Overlay submit failed (HTTP {status}): {body}"
        )));
    }

    // Check the steak response — outputs_to_admit must be non-empty.
    // HTTP 200 just means the BEEF was valid, not that outputs were admitted.
    let admitted = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(steak) => {
            let admitted_count = steak
                .get("tm_agent")
                .and_then(|tm| tm.get("outputsToAdmit"))
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            tracing::info!(
                steak = %body,
                admitted = admitted_count,
                "Overlay submit response"
            );
            admitted_count > 0
        }
        Err(_) => {
            tracing::warn!(body = %body, "Overlay submit: could not parse steak response");
            true // assume success if response isn't JSON
        }
    };

    if admitted {
        tracing::info!(
            txid = %result.txid,
            name = %name,
            capabilities = %capabilities,
            "Registered on overlay"
        );
        Ok(result.txid)
    } else {
        tracing::error!(
            txid = %result.txid,
            body = %body,
            "Overlay registration REJECTED — outputs_to_admit is empty. \
             Check signature parameters: protocol=[2,\"agent registry\"], keyID=\"1\", counterparty=\"anyone\""
        );
        Err(DmError::wallet(format!(
            "Overlay registration rejected: no outputs admitted. Steak: {body}"
        )))
    }
}
