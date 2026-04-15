//! Wallet tools — balance, identity, pay, encrypt/decrypt, list, info.
//!
//! These route to the local wallet at the configured URL.
//! Full BRC-100 coverage so the agent LLM can use the wallet autonomously.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::tools::registry::ToolDef;
use crate::wallet::{HttpWalletClient, WalletBackend};

fn client(wallet_url: &str) -> HttpWalletClient {
    HttpWalletClient::new(wallet_url, "http://localhost", 30)
}

/// Base58 encoding for BSV addresses.
pub(crate) fn base58_encode(data: &[u8]) -> String {
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

/// Convert a P2PKH script hex to a BSV address.
pub(crate) fn script_to_address(script_hex: &str) -> Result<String, String> {
    let script_bytes =
        hex::decode(script_hex).map_err(|e| format!("Failed to decode script: {e}"))?;
    if script_bytes.len() < 25 {
        return Err("Script too short for P2PKH".into());
    }
    // Extract hash160 from P2PKH: 76 a9 14 <20-bytes> 88 ac
    let pkh = &script_bytes[3..23];
    // Base58Check: version(0x00) + hash160 + checksum
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(pkh);
    let checksum = Sha256::digest(Sha256::digest(&payload));
    payload.extend_from_slice(&checksum[..4]);
    Ok(base58_encode(&payload))
}

async fn check_certificates(_params: Value, url: &str) -> String {
    let wallet: std::sync::Arc<dyn WalletBackend> = std::sync::Arc::new(client(url));
    let mgr = crate::certificates::CertificateManager::new(wallet);
    match mgr.certificate_status().await {
        Ok(status) => serde_json::to_string_pretty(&status)
            .unwrap_or_else(|_| "Error: serialization failed".into()),
        Err(e) => format!("Error checking certificates: {e}"),
    }
}

async fn wallet_balance(_params: Value, url: &str) -> String {
    match client(url).get_balance().await {
        Ok(satoshis) => {
            let bsv = satoshis as f64 / 100_000_000.0;
            serde_json::to_string(&json!({
                "satoshis": satoshis,
                "bsv": bsv,
                "status": "connected",
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string())
        }
        Err(e) => format!("Error connecting to wallet: {e}"),
    }
}

async fn wallet_identity(params: Value, url: &str) -> String {
    let protocol_id = params.get("protocol_id");
    let key_id = params.get("key_id").and_then(|v| v.as_str()).unwrap_or("1");
    let counterparty = params
        .get("counterparty")
        .and_then(|v| v.as_str())
        .unwrap_or("self");

    if let Some(pid) = protocol_id {
        match client(url)
            .get_public_key(pid, key_id, counterparty, false)
            .await
        {
            Ok(pubkey) => serde_json::to_string(&json!({"public_key": pubkey, "derived": true}))
                .unwrap_or_else(|_| "Error: serialization failed".to_string()),
            Err(e) => format!("Error: {e}"),
        }
    } else {
        match client(url).get_identity_key().await {
            Ok(identity) => {
                serde_json::to_string(&json!({"identity_key": identity, "derived": false}))
                    .unwrap_or_else(|_| "Error: serialization failed".to_string())
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

async fn wallet_encrypt(params: Value, url: &str) -> String {
    let plaintext = params
        .get("plaintext")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let protocol_id = params
        .get("protocol_id")
        .cloned()
        .unwrap_or(json!([2, "dolphin milk encryption"]));
    let key_id = params.get("key_id").and_then(|v| v.as_str()).unwrap_or("1");
    let counterparty = params
        .get("counterparty")
        .and_then(|v| v.as_str())
        .unwrap_or("self");

    if plaintext.is_empty() {
        return "Error: no plaintext provided".to_string();
    }

    match client(url)
        .encrypt(plaintext.as_bytes(), &protocol_id, key_id, counterparty)
        .await
    {
        Ok(ciphertext) => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&ciphertext);
            serde_json::to_string(&json!({
                "ciphertext_base64": b64,
                "bytes": ciphertext.len(),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string())
        }
        Err(e) => format!("Error: {e}"),
    }
}

async fn wallet_decrypt(params: Value, url: &str) -> String {
    let ciphertext_b64 = params
        .get("ciphertext_base64")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let protocol_id = params
        .get("protocol_id")
        .cloned()
        .unwrap_or(json!([2, "dolphin milk encryption"]));
    let key_id = params.get("key_id").and_then(|v| v.as_str()).unwrap_or("1");
    let counterparty = params
        .get("counterparty")
        .and_then(|v| v.as_str())
        .unwrap_or("self");

    if ciphertext_b64.is_empty() {
        return "Error: no ciphertext_base64 provided".to_string();
    }

    use base64::Engine;
    let ciphertext = match base64::engine::general_purpose::STANDARD.decode(ciphertext_b64) {
        Ok(bytes) => bytes,
        Err(e) => return format!("Error: invalid base64: {e}"),
    };

    match client(url)
        .decrypt(&ciphertext, &protocol_id, key_id, counterparty)
        .await
    {
        Ok(plaintext) => match String::from_utf8(plaintext.clone()) {
            Ok(text) => serde_json::to_string(&json!({"plaintext": text}))
                .unwrap_or_else(|_| "Error: serialization failed".to_string()),
            Err(_) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&plaintext);
                serde_json::to_string(&json!({"plaintext_base64": b64, "binary": true}))
                    .unwrap_or_else(|_| "Error: serialization failed".to_string())
            }
        },
        Err(e) => format!("Error: {e}"),
    }
}

/// Generate a BSV receive address from the wallet's key derivation.
/// Uses BRC-29 protocol with the worm's funding derivation path.
async fn receive_address(params: Value, url: &str) -> String {
    let suffix = params.get("suffix").and_then(|v| v.as_str()).unwrap_or("1");

    match client(url).receive_address(suffix).await {
        Ok((pubkey, script, suffix)) => match script_to_address(&script) {
            Ok(address) => serde_json::to_string(&json!({
                "address": address,
                "public_key": pubkey,
                "suffix": suffix,
                "instructions": format!(
                    "Send BSV to {address}. After the transaction confirms, \
                     use fund_from_tx with the txid to internalize the payment."
                ),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".into()),
            Err(e) => format!("Error: {e}"),
        },
        Err(e) => format!("Error generating receive address: {e}"),
    }
}

/// Internalize an external funding transaction.
/// Either provide `beef_hex` directly (from sender) or `txid` to fetch from WhatsOnChain.
async fn fund_from_tx(params: Value, url: &str) -> String {
    let beef_hex = params.get("beef_hex").and_then(|v| v.as_str());
    let txid = params.get("txid").and_then(|v| v.as_str());
    let vout = params
        .get("vout")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let suffix = params.get("suffix").and_then(|v| v.as_str()).unwrap_or("1");

    if beef_hex.is_none() && txid.is_none() {
        return "Error: provide either 'beef_hex' (direct BEEF from sender) or 'txid' (fetch from WhatsOnChain)".into();
    }

    // Direct BEEF path — sender gave us the raw transaction
    if let Some(beef) = beef_hex {
        let beef_bytes = match hex::decode(beef) {
            Ok(b) => b,
            Err(e) => return format!("Error: invalid hex in beef_hex: {e}"),
        };

        let wallet = client(url);
        let protocol_id = json!([2, "3241645161d8"]);
        let key_id = format!("dolphin-milk-fund {suffix}");

        // Derive the expected pubkey for verification
        let derived_pubkey = match wallet
            .get_public_key(&protocol_id, &key_id, crate::wallet::ANYONE_KEY, true)
            .await
        {
            Ok(pk) => pk,
            Err(e) => return format!("Error deriving key: {e}"),
        };

        // Build AtomicBEEF if not already in that format
        let atomic_beef = if beef_bytes.len() >= 4
            && beef_bytes[0] == 0x01
            && beef_bytes[1] == 0x01
            && beef_bytes[2] == 0x01
            && beef_bytes[3] == 0x01
        {
            // Already AtomicBEEF
            beef_bytes
        } else {
            // Raw BEEF — we need the txid to build AtomicBEEF header.
            // Parse txid from the transaction or require it as a param.
            if let Some(tid) = txid {
                let txid_bytes = match hex::decode(tid) {
                    Ok(b) => b,
                    Err(e) => return format!("Error: invalid txid hex: {e}"),
                };
                let mut ab = vec![0x01u8, 0x01, 0x01, 0x01];
                let reversed: Vec<u8> = txid_bytes.into_iter().rev().collect();
                ab.extend_from_slice(&reversed);
                ab.extend_from_slice(&beef_bytes);
                ab
            } else {
                // Try using raw bytes directly — wallet may accept standard BEEF
                beef_bytes
            }
        };

        let outputs = vec![json!({
            "outputIndex": vout.unwrap_or(0),
            "protocol": "wallet payment",
            "paymentRemittance": {
                "derivationPrefix": "dolphin-milk-fund",
                "derivationSuffix": suffix,
                "senderIdentityKey": crate::wallet::ANYONE_KEY,
            },
        })];

        match wallet
            .internalize_action(&atomic_beef, &outputs, "Internalize direct BEEF funding")
            .await
        {
            Ok(result) => {
                let accepted = result
                    .get("accepted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if accepted {
                    serde_json::to_string(&json!({
                        "accepted": true,
                        "vout": vout,
                        "derived_pubkey": derived_pubkey,
                        "message": "Funding internalized from BEEF. BSV is now spendable in the default basket.",
                    }))
                    .unwrap_or_else(|_| "Error: serialization failed".into())
                } else {
                    format!("Error: wallet rejected internalization: {result}")
                }
            }
            Err(e) => format!("Error internalizing BEEF: {e}"),
        }
    } else {
        // WOC path — fetch BEEF by txid
        let txid = txid.unwrap();
        match client(url).fund_from_woc(txid, vout, Some(suffix)).await {
            Ok(result) => {
                let accepted = result
                    .get("accepted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if accepted {
                    serde_json::to_string(&json!({
                        "accepted": true,
                        "txid": txid,
                        "vout": vout,
                        "message": "Funding internalized from WOC. BSV is now spendable in the default basket.",
                    }))
                    .unwrap_or_else(|_| "Error: serialization failed".into())
                } else {
                    format!("Error: wallet rejected internalization: {result}")
                }
            }
            Err(e) => format!("Error internalizing funding: {e}"),
        }
    }
}

/// Generic BRC-100 wallet endpoint call.
/// Routes the endpoint name + optional params to `WalletClient::raw_call()`.
async fn wallet_call(params: Value, url: &str) -> String {
    let endpoint = match params.get("endpoint").and_then(|v| v.as_str()) {
        Some(e) => e.to_string(),
        None => return "Error: 'endpoint' is required (e.g. \"discoverByIdentityKey\")".into(),
    };

    // Safety blocklist — endpoints that should never be called via wallet_call.
    // After Phase 2 wallet consolidation, all BRC-100 endpoints are accessible
    // via wallet_call (guided by the wallet skill). createAction was previously
    // blocked here when it had a dedicated tool; now wallet_call handles it.
    const BLOCKED: &[&str] = &[];
    if BLOCKED.contains(&endpoint.as_str()) {
        return format!("Error: '{endpoint}' is blocked via wallet_call.");
    }

    let call_params = params.get("params").cloned().filter(|v| !v.is_null());

    match client(url).raw_call(&endpoint, call_params).await {
        Ok(result) => serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| "Error: serialization failed".into()),
        Err(e) => format!("Error calling wallet/{endpoint}: {e}"),
    }
}

/// Create wallet tool definitions registered in the agent's always-on tool set.
///
/// Phase 2 consolidation: 14 → 6 tools. The removed 8 tools (pay, info, sign,
/// verify, list_actions, list_outputs, create_action, header) are accessible via
/// `wallet_call` guided by the wallet skill (skills/wallet/SKILL.md).
/// `check_certificates` is kept because it wraps CertificateManager logic (not a
/// simple raw_call). Their implementation functions are retained for internal use.
pub fn all_wallet_tools(wallet_url: String) -> Vec<ToolDef> {
    let u = wallet_url;
    vec![
        ToolDef {
            name: "wallet_balance".to_string(),
            description: "Check the BSV wallet balance in satoshis and BSV.".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { wallet_balance(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "wallet_identity".to_string(),
            description:
                "Get the wallet's identity key or derive a public key for a protocol."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "protocol_id": {"type": "string", "description": "BRC-42 protocol ID (e.g. [2, \"my-protocol\"]). If omitted, returns identity key."},
                    "key_id": {"type": "string", "description": "Key ID for derivation (default: '1')"},
                    "counterparty": {"type": "string", "description": "Counterparty pubkey, 'self', or 'anyone' (default: 'self')"},
                },
            }),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { wallet_identity(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "wallet_encrypt".to_string(),
            description:
                "Encrypt text using BRC-42 key derivation. Returns base64 ciphertext."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "plaintext": {"type": "string", "description": "Text to encrypt"},
                    "protocol_id": {"description": "BRC-42 protocol ID (default: [2, 'encryption'])"},
                    "key_id": {"type": "string", "description": "Key ID (default: '1')"},
                    "counterparty": {"type": "string", "description": "Recipient pubkey, 'self', or 'anyone' (default: 'self')"},
                },
                "required": ["plaintext"],
            }),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { wallet_encrypt(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "wallet_decrypt".to_string(),
            description:
                "Decrypt base64 ciphertext using BRC-42 key derivation."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ciphertext_base64": {"type": "string", "description": "Base64-encoded ciphertext from wallet_encrypt"},
                    "protocol_id": {"description": "BRC-42 protocol ID (default: [2, 'encryption'])"},
                    "key_id": {"type": "string", "description": "Key ID (default: '1')"},
                    "counterparty": {"type": "string", "description": "Sender pubkey, 'self', or 'anyone' (default: 'self')"},
                },
                "required": ["ciphertext_base64"],
            }),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { wallet_decrypt(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "check_certificates".to_string(),
            description: "Check the agent's BRC-52 authorization certificate status. Returns whether the cert is parent-signed, self-signed, or absent, along with certificate details (certifier, subject, name, capabilities).".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { check_certificates(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "receive_address".to_string(),
            description: "Generate a BSV receive address. Share this address with anyone who wants to send you BSV. After they send, use fund_from_tx to internalize the payment.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "suffix": {
                        "type": "string",
                        "description": "Derivation suffix for the address (default: '1'). Use different suffixes to generate different addresses."
                    },
                },
            }),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { receive_address(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "fund_from_tx".to_string(),
            description: "Internalize an external BSV payment into the wallet. Two modes: (1) provide beef_hex directly if the sender gave you the BEEF, or (2) provide txid to fetch BEEF from WhatsOnChain. The suffix must match the one used in receive_address.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "txid": {
                        "type": "string",
                        "description": "Transaction ID (64-char hex). Required for WOC mode, optional with beef_hex."
                    },
                    "beef_hex": {
                        "type": "string",
                        "description": "Raw BEEF transaction as hex string (from the sender directly). If provided, skips WOC fetch."
                    },
                    "vout": {
                        "type": "integer",
                        "description": "Output index in the transaction (default: 0)"
                    },
                    "suffix": {
                        "type": "string",
                        "description": "Derivation suffix matching the one used in receive_address (default: '1')"
                    },
                },
            }),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { fund_from_tx(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "wallet_call".to_string(),
            description: "Call any BRC-100 wallet endpoint by name. Covers all wallet operations: payments (createAction), signing (createSignature, verifySignature), discovery (discoverByIdentityKey, discoverByAttributes), certificates (listCertificates, proveCertificate), HMAC, key linkage, headers, and more. See the wallet skill for the full endpoint catalog and usage examples.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "endpoint": {
                        "type": "string",
                        "description": "BRC-100 wallet endpoint name (e.g. 'createAction', 'createSignature', 'listActions', 'getHeaderForHeight')"
                    },
                    "params": {
                        "type": "object",
                        "description": "Endpoint parameters as JSON object. Omit for GET endpoints (status checks)."
                    }
                },
                "required": ["endpoint"]
            }),
            execute: { let u = u.clone(); Box::new(move |params| { let u = u.clone(); Box::pin(async move { wallet_call(params, &u).await }) }) },
            category: "wallet".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
    ]
}
