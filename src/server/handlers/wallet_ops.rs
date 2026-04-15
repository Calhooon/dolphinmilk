//! Wallet-related route handlers (output lookup, decrypt).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::wallet::WalletBackend;

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

// -- Local types --

/// GET /output/{basket}/{txid} -- fetch a UTXO's locking script from wallet.
///
/// Returns the raw script hex and parsed PushDrop data fields if applicable.
#[derive(Debug, Serialize, Deserialize)]
pub struct OutputResponse {
    pub txid: String,
    pub basket: String,
    pub locking_script: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_fields: Option<Vec<String>>, // hex-encoded PushDrop fields
    pub satoshis: u64,
}

/// POST /decrypt -- decrypt ciphertext via the wallet's BRC-42 decrypt endpoint.
#[derive(Debug, Deserialize)]
pub struct DecryptRequest {
    pub ciphertext_hex: String,
    pub protocol_id: serde_json::Value,
    pub key_id: String,
    #[serde(default = "default_counterparty")]
    pub counterparty: String,
}

fn default_counterparty() -> String {
    "self".to_string()
}

#[derive(Debug, Serialize)]
pub struct DecryptResponse {
    pub plaintext: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed_json: Option<serde_json::Value>,
}

// -- Handlers --

pub(crate) async fn get_output(
    State(state): State<Arc<AppState>>,
    Path((basket, txid)): Path<(String, String)>,
) -> Result<axum::Json<OutputResponse>, (StatusCode, String)> {
    match find_output_in_basket(state.wallet.as_ref(), &basket, &txid).await {
        Some(resp) => Ok(axum::Json(resp)),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("txid {txid} not found in basket \"{basket}\""),
        )),
    }
}

/// Search listOutputs for a specific txid in a basket.
async fn find_output_in_basket(
    wallet: &dyn WalletBackend,
    basket: &str,
    txid: &str,
) -> Option<OutputResponse> {
    let mut offset = 0u64;
    let limit = 100u64;
    loop {
        let result = match wallet
            .list_outputs(basket, "locking scripts", limit, offset)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("find_output_in_basket: listOutputs failed for {basket}: {e}");
                return None;
            }
        };
        if result.get("error").is_some() {
            return None;
        }
        let arr = result.get("outputs").and_then(|v| v.as_array())?;
        if arr.is_empty() {
            return None;
        }
        for output in arr {
            // Wallet returns "outpoint" as "txid.vout", not a separate "txid" field
            let out_txid = output
                .get("outpoint")
                .and_then(|v| v.as_str())
                .and_then(|op| op.split('.').next())
                .or_else(|| output.get("txid").and_then(|v| v.as_str()))
                .unwrap_or_default();
            if out_txid == txid {
                return Some(build_output_response(output, txid, basket));
            }
        }
        if arr.len() < limit as usize {
            break;
        }
        offset += limit;
    }
    None
}

/// Build an OutputResponse from a wallet output JSON value.
fn build_output_response(output: &serde_json::Value, txid: &str, basket: &str) -> OutputResponse {
    let script_hex = output
        .get("lockingScript")
        .and_then(|v| v.as_str())
        .or_else(|| output.get("locking_script").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();
    let satoshis = output.get("satoshis").and_then(|v| v.as_u64()).unwrap_or(0);
    let data_fields = parse_push_drop_fields(&script_hex);
    OutputResponse {
        txid: txid.to_string(),
        basket: basket.to_string(),
        locking_script: script_hex,
        data_fields,
        satoshis,
    }
}

/// Parse PushDrop data fields from a locking script hex string.
/// Delegates to `crate::onchain::state::parse_push_drop_fields`.
pub(crate) fn parse_push_drop_fields(script_hex: &str) -> Option<Vec<String>> {
    crate::onchain::state::parse_push_drop_fields(script_hex)
}

/// GET /wallet/address — returns a BSV funding address for the wallet.
pub(crate) async fn get_funding_address(
    State(state): State<Arc<AppState>>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, String)> {
    match state.wallet.receive_address("1").await {
        Ok((pubkey, script, suffix)) => {
            // Compute BSV address from P2PKH script: OP_DUP OP_HASH160 <20-bytes> OP_EQUALVERIFY OP_CHECKSIG
            let script_bytes = hex::decode(&script).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("script decode: {e}"),
                )
            })?;
            if script_bytes.len() < 23 {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invalid P2PKH script".into(),
                ));
            }
            let pkh = &script_bytes[3..23];

            use sha2::{Digest, Sha256};
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(pkh);
            let checksum = Sha256::digest(Sha256::digest(&payload));
            payload.extend_from_slice(&checksum[..4]);

            let address = base58_encode(&payload);

            Ok(axum::Json(serde_json::json!({
                "address": address,
                "pubkey": pubkey,
                "suffix": suffix,
            })))
        }
        Err(e) => Err((StatusCode::SERVICE_UNAVAILABLE, format!("wallet: {e}"))),
    }
}

/// POST /wallet/check-funding — check WhatsOnChain for incoming UTXOs and auto-internalize.
pub(crate) async fn check_funding(
    State(state): State<Arc<AppState>>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, String)> {
    // Get the funding address
    let (_, script, _) = state
        .wallet
        .receive_address("1")
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("wallet: {e}")))?;

    let script_bytes = hex::decode(&script).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("script decode: {e}"),
        )
    })?;
    if script_bytes.len() < 23 {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "invalid script".into()));
    }
    let pkh = &script_bytes[3..23];

    use sha2::{Digest, Sha256};
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(pkh);
    let checksum = Sha256::digest(Sha256::digest(&payload));
    payload.extend_from_slice(&checksum[..4]);
    let address = base58_encode(&payload);

    // Check WhatsOnChain for UTXOs at this address
    let client = reqwest::Client::new();
    let woc_url = format!("https://api.whatsonchain.com/v1/bsv/main/address/{address}/unspent");
    let utxos: Vec<serde_json::Value> = match client.get(&woc_url).send().await {
        Ok(resp) if resp.status().is_success() => resp.json().await.unwrap_or_default(),
        _ => vec![],
    };

    if utxos.is_empty() {
        // Check balance anyway — might have been funded via a different method
        let balance = state.wallet.get_balance().await.unwrap_or(0);
        return Ok(axum::Json(serde_json::json!({
            "found": false,
            "message": "No incoming transactions found yet. It may take a few minutes to appear on-chain.",
            "balance": balance,
        })));
    }

    // Try to internalize each UTXO
    let mut internalized = 0;
    for utxo in &utxos {
        let txid = utxo
            .get("tx_hash")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let vout = utxo.get("tx_pos").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if txid.is_empty() {
            continue;
        }
        match state
            .wallet
            .fund_from_woc(txid, Some(vout), Some("1"))
            .await
        {
            Ok(result) => {
                let accepted = result
                    .get("accepted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if accepted {
                    internalized += 1;
                }
            }
            Err(e) => {
                tracing::debug!("check_funding: failed to internalize {txid}: {e}");
            }
        }
    }

    let balance = state.wallet.get_balance().await.unwrap_or(0);

    Ok(axum::Json(serde_json::json!({
        "found": internalized > 0,
        "internalized": internalized,
        "balance": balance,
    })))
}

/// Bitcoin Base58 encoding.
fn base58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let leading_zeros = data.iter().take_while(|&&b| b == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for &byte in data {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) * 256;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut result = String::with_capacity(leading_zeros + digits.len());
    for _ in 0..leading_zeros {
        result.push('1');
    }
    for &d in digits.iter().rev() {
        result.push(ALPHABET[d as usize] as char);
    }
    result
}

// -- UTXO listing types --

#[derive(Debug, Serialize)]
pub struct WalletUtxosResponse {
    pub balance: u64,
    pub utxo_count: u64,
    pub utxos: Vec<UtxoInfo>,
    pub tiers: TierSummary,
}

#[derive(Debug, Serialize)]
pub struct UtxoInfo {
    pub outpoint: String,
    pub satoshis: u64,
    pub spendable: bool,
}

#[derive(Debug, Serialize)]
pub struct TierSummary {
    pub dust: TierInfo,
    pub small: TierInfo,
    pub medium: TierInfo,
    pub large: TierInfo,
}

#[derive(Debug, Serialize)]
pub struct TierInfo {
    pub count: u64,
    pub total_sats: u64,
}

/// GET /wallet/utxos — returns the spendable UTXO set from the default basket with tier summaries.
pub(crate) async fn get_wallet_utxos(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/wallet/utxos", None, &headers, None).await?;

    let mut utxos = Vec::new();
    let mut offset = 0u64;
    let limit = 100u64;
    loop {
        let result = state
            .wallet
            .list_outputs("default", "locking scripts", limit, offset)
            .await
            .map_err(|e| {
                tracing::warn!("get_wallet_utxos: listOutputs failed: {e}");
                StatusCode::SERVICE_UNAVAILABLE
            })?;
        if result.get("error").is_some() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let arr = result
            .get("outputs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if arr.is_empty() {
            break;
        }
        for output in &arr {
            let spendable = output
                .get("spendable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !spendable {
                continue;
            }
            let outpoint = output
                .get("outpoint")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let satoshis = output.get("satoshis").and_then(|v| v.as_u64()).unwrap_or(0);
            utxos.push(UtxoInfo {
                outpoint,
                satoshis,
                spendable,
            });
        }
        if (arr.len() as u64) < limit {
            break;
        }
        offset += limit;
    }

    // Compute tier summary
    let mut dust = TierInfo {
        count: 0,
        total_sats: 0,
    };
    let mut small = TierInfo {
        count: 0,
        total_sats: 0,
    };
    let mut medium = TierInfo {
        count: 0,
        total_sats: 0,
    };
    let mut large = TierInfo {
        count: 0,
        total_sats: 0,
    };
    let mut balance = 0u64;

    for utxo in &utxos {
        balance += utxo.satoshis;
        let tier = if utxo.satoshis < 1_000 {
            &mut dust
        } else if utxo.satoshis <= 100_000 {
            &mut small
        } else if utxo.satoshis <= 1_000_000 {
            &mut medium
        } else {
            &mut large
        };
        tier.count += 1;
        tier.total_sats += utxo.satoshis;
    }

    let resp = WalletUtxosResponse {
        balance,
        utxo_count: utxos.len() as u64,
        utxos,
        tiers: TierSummary {
            dust,
            small,
            medium,
            large,
        },
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &resp).await
}

pub(crate) async fn decrypt_data(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "POST", "/decrypt", None, &headers, Some(&body)).await?;

    let req: DecryptRequest = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    let ciphertext = hex::decode(&req.ciphertext_hex).map_err(|_| StatusCode::BAD_REQUEST)?;

    let plaintext_bytes = state
        .wallet
        .decrypt(
            &ciphertext,
            &req.protocol_id,
            &req.key_id,
            &req.counterparty,
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let plaintext = String::from_utf8_lossy(&plaintext_bytes).to_string();
    let parsed_json = serde_json::from_str::<serde_json::Value>(&plaintext).ok();

    let resp = DecryptResponse {
        plaintext,
        parsed_json,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &resp).await
}

// ── POST /wallet/split ─────────────────────────────────────────────
//
// Atomically split the wallet's spendable balance into N equal-sized
// UTXOs. Thin wrapper over `WalletBackend::split_utxos(count)` which
// dispatches to the embedded wallet's `split_utxos` method (mirroring
// bsv-wallet-cli's split.rs logic). HTTP-only wallets return an error.

#[derive(Debug, Deserialize)]
pub struct SplitRequest {
    pub count: u32,
}

#[derive(Debug, Serialize)]
pub struct SplitResponse {
    pub txid: String,
    pub per_output_sats: u64,
    pub count: u32,
    pub view_url: String,
}

pub(crate) async fn split_wallet(
    State(state): State<Arc<AppState>>,
    axum::Json(req): axum::Json<SplitRequest>,
) -> Result<axum::Json<SplitResponse>, (StatusCode, String)> {
    if req.count < 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            "count must be >= 2".to_string(),
        ));
    }

    let (txid, per_output_sats, count) = state
        .wallet
        .split_utxos(req.count)
        .await
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, format!("{e}")))?;

    let view_url = format!("https://whatsonchain.com/tx/{txid}");
    Ok(axum::Json(SplitResponse {
        txid,
        per_output_sats,
        count,
        view_url,
    }))
}
