//! HTTP client for bsv-wallet-cli at :3322.
//!
//! Provides a Rust interface to the BSV wallet application running locally.
//! The wallet implements the BRC-100 interface and exposes it via HTTP.
//!
//! All public methods are defined as inherent methods on [`HttpWalletClient`]
//! so that callers do not need to import the [`WalletBackend`] trait.
//! The [`WalletBackend`] trait impl delegates to these inherent methods.

use reqwest::Client;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

use bsv::primitives::PublicKey;
use bsv::script::templates::P2PKH;
use bsv::wallet::KeyDeriver;

use crate::error::{DmError, ErrorContext};

use super::types::{CreateActionResult, ANYONE_KEY};
use super::WalletBackend;

/// Serializes all spending operations (createAction, spend_output) across
/// all wallet client instances. Prevents concurrent UTXO contention when
/// multiple tools fire in parallel.
static SPEND_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));

/// HTTP client for a BRC-100 wallet.
#[derive(Debug, Clone)]
pub struct HttpWalletClient {
    pub url: String,
    pub origin: String,
    pub timeout: Duration,
    client: Client,
}

impl HttpWalletClient {
    pub fn new(url: &str, origin: &str, timeout: u64) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            origin: origin.to_string(),
            timeout: Duration::from_secs(timeout),
            client: Client::new(),
        }
    }

    pub fn from_config(cfg: &crate::config::WalletConfig) -> Self {
        Self::new(&cfg.url, &cfg.origin, cfg.timeout)
    }

    // ── Private HTTP helpers ─────────────────────────────────────────

    /// POST to a wallet endpoint with retry on transient errors (database locked, etc.).
    /// Used by state-modifying operations (createAction, relinquishOutput) where
    /// SQLite contention can cause sporadic failures.
    async fn call_with_retry(&self, method: &str, params: Value) -> Result<Value, DmError> {
        const RETRY_DELAYS_MS: &[u64] = &[100, 250, 500];

        let start = Instant::now();
        let mut last_err = None;
        for (attempt, &delay) in RETRY_DELAYS_MS.iter().enumerate() {
            match self.call(method, params.clone()).await {
                Ok(val) => {
                    let elapsed = start.elapsed();
                    if attempt > 0 {
                        tracing::debug!(
                            wallet_method = method,
                            wallet_latency_ms = elapsed.as_millis() as u64,
                            retries = attempt,
                            "Wallet /{method} completed in {}ms after {} retries",
                            elapsed.as_millis(),
                            attempt
                        );
                    }
                    return Ok(val);
                }
                Err(e) => {
                    let msg = e.to_string();
                    let is_transient = msg.contains("database is locked")
                        || msg.contains("SQLITE_BUSY")
                        || msg.contains("HTTP 500")
                        || msg.contains("HTTP 503");
                    if is_transient && attempt + 1 < RETRY_DELAYS_MS.len() {
                        tracing::warn!(
                            "Wallet /{method} transient error (attempt {}/{}), retrying in {delay}ms: {msg}",
                            attempt + 1,
                            RETRY_DELAYS_MS.len()
                        );
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap())
    }

    /// POST to a wallet endpoint.
    async fn call(&self, method: &str, params: Value) -> Result<Value, DmError> {
        let start = Instant::now();
        let endpoint = format!("{}/{}", self.url, method);
        let resp = self
            .client
            .post(&endpoint)
            .header("Origin", &self.origin)
            .header("Content-Type", "application/json")
            .timeout(self.timeout)
            .json(&params)
            .send()
            .await
            .map_err(|e| {
                let mut ctx = ErrorContext::new();
                ctx.insert("url".into(), json!(self.url));
                ctx.insert("method".into(), json!(method));
                DmError::wallet_with(format!("Wallet not reachable at {}: {e}", self.url), ctx)
            })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            DmError::wallet(format!("Wallet /{method} failed to read response: {e}"))
        })?;
        // Some wallet endpoints return empty body on success (e.g. relinquishCertificate)
        let body: Value = if text.is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    // Non-JSON response — e.g. axum 422 plain-text rejection.
                    // Include the raw text in the error for diagnosis.
                    if !status.is_success() {
                        let preview = if text.len() > 200 {
                            &text[..200]
                        } else {
                            &text
                        };
                        return Err(DmError::wallet(format!(
                            "Wallet /{method} returned HTTP {}: {preview}",
                            status.as_u16()
                        )));
                    }
                    return Err(DmError::wallet(format!(
                        "Wallet /{method} returned invalid JSON: {e}"
                    )));
                }
            }
        };

        if !status.is_success() {
            let error_msg = body
                .get("message")
                .or(body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(DmError::wallet(format!(
                "Wallet /{method} returned HTTP {}: {error_msg}",
                status.as_u16()
            )));
        }

        // Some wallet errors come back as 200 with an error field
        if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
            if !err.is_empty() {
                return Err(DmError::wallet(format!("Wallet /{method} error: {err}")));
            }
        }

        let elapsed = start.elapsed();
        tracing::debug!(
            wallet_method = method,
            wallet_latency_ms = elapsed.as_millis() as u64,
            "Wallet /{method} completed in {}ms",
            elapsed.as_millis()
        );

        Ok(body)
    }

    /// GET a wallet endpoint (for status endpoints that take no body).
    async fn call_get(&self, method: &str) -> Result<Value, DmError> {
        let endpoint = format!("{}/{}", self.url, method);
        let resp = self
            .client
            .get(&endpoint)
            .header("Origin", &self.origin)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| DmError::wallet(format!("Wallet not reachable: {e}")))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| DmError::wallet(format!("Wallet /{method} returned invalid JSON: {e}")))?;

        if !status.is_success() {
            let error_msg = body
                .get("message")
                .or(body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(DmError::wallet(format!(
                "Wallet /{method} returned HTTP {}: {error_msg}",
                status.as_u16()
            )));
        }

        Ok(body)
    }

    /// Funding derivation protocol — used for both `receive_address` and `fund_from_woc`.
    /// Protocol: `[2, "3241645161d8"]` (same as BRC-29 payment protocol).
    fn funding_protocol() -> Value {
        json!([2, "3241645161d8"])
    }

    /// Default derivation prefix for receiving funds.
    const FUNDING_PREFIX: &'static str = "dolphin-milk-fund";

    // ── Public inherent methods (all 40 BRC-100 operations) ──────────
    // These are the methods that call sites use directly. Keeping them
    // as inherent methods means callers do NOT need to import WalletBackend.

    /// Get the wallet's identity public key (66-char hex compressed).
    pub async fn get_identity_key(&self) -> Result<String, DmError> {
        let result = self
            .call("getPublicKey", json!({"identityKey": true}))
            .await?;
        result
            .get("publicKey")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DmError::wallet("Missing 'publicKey' in response"))
    }

    /// Derive a public key via BRC-42 key derivation.
    pub async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, DmError> {
        let result = self
            .call(
                "getPublicKey",
                json!({
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                    "forSelf": for_self,
                }),
            )
            .await?;
        result
            .get("publicKey")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DmError::wallet("Missing 'publicKey' in response"))
    }

    /// Sign data with a derived key via BRC-42 + ECDSA.
    pub async fn create_signature(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let data_array: Vec<u8> = data.to_vec();
        let result = self
            .call(
                "createSignature",
                json!({
                    "data": data_array,
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                }),
            )
            .await?;

        let sig_array = result
            .get("signature")
            .and_then(|v| v.as_array())
            .ok_or_else(|| DmError::wallet("Missing 'signature' array in response"))?;

        sig_array
            .iter()
            .map(|v| {
                v.as_u64()
                    .map(|n| n as u8)
                    .ok_or_else(|| DmError::wallet("Invalid byte in signature array"))
            })
            .collect()
    }

    /// Create a transaction via the wallet.
    ///
    /// Serialized via a global semaphore to prevent concurrent UTXO contention
    /// when multiple tools fire in parallel.
    pub async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        randomize_outputs: bool,
    ) -> Result<CreateActionResult, DmError> {
        let _permit = SPEND_SEMAPHORE.acquire().await.unwrap();

        let result = self
            .call_with_retry(
                "createAction",
                json!({
                    "description": description,
                    "outputs": outputs,
                    "options": {
                        "acceptDelayedBroadcast": accept_delayed_broadcast,
                        "randomizeOutputs": randomize_outputs,
                    },
                }),
            )
            .await?;

        let txid = result
            .get("txid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DmError::wallet("Missing 'txid' in createAction response"))?
            .to_string();

        let tx = if let Some(arr) = result.get("tx").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u8))
                .collect()
        } else {
            Vec::new()
        };

        Ok(CreateActionResult {
            txid,
            tx,
            raw: result,
        })
    }

    /// Spend a specific output on-chain via createAction with inputs.
    ///
    /// Unlike `relinquish_output` (which only removes from wallet tracking),
    /// this creates an actual on-chain transaction that spends the UTXO.
    /// Uses flat inputs array with unlockingScriptLength (73 bytes for
    /// PushDrop `<sig> OP_CHECKSIG` pattern). The wallet resolves the
    /// locking script and signing key from its stored output data.
    pub async fn spend_output(
        &self,
        _basket: &str,
        txid: &str,
        vout: u32,
        description: &str,
    ) -> Result<CreateActionResult, DmError> {
        let _permit = SPEND_SEMAPHORE.acquire().await.unwrap();

        let outpoint = format!("{}.{}", txid, vout);
        let result = self
            .call_with_retry(
                "createAction",
                json!({
                    "description": description,
                    "inputs": [{
                        "outpoint": outpoint,
                        "inputDescription": description,
                        "unlockingScriptLength": 73,
                    }],
                    "outputs": [],
                    "options": {
                        "acceptDelayedBroadcast": false,
                        "randomizeOutputs": false,
                        "trustSelf": "known",
                    },
                }),
            )
            .await?;

        let spend_txid = result
            .get("txid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DmError::wallet("Missing 'txid' in createAction response"))?
            .to_string();

        let tx = if let Some(arr) = result.get("tx").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u8))
                .collect()
        } else {
            Vec::new()
        };

        Ok(CreateActionResult {
            txid: spend_txid,
            tx,
            raw: result,
        })
    }

    /// Internalize an incoming transaction into the wallet.
    pub async fn internalize_action(
        &self,
        tx_bytes: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, DmError> {
        let tx_array: Vec<u8> = tx_bytes.to_vec();
        self.call(
            "internalizeAction",
            json!({
                "tx": tx_array,
                "outputs": outputs,
                "description": description,
            }),
        )
        .await
    }

    /// List spendable outputs and return total balance in satoshis.
    pub async fn get_balance(&self) -> Result<u64, DmError> {
        let (balance, _count) = self.get_balance_and_count().await?;
        Ok(balance)
    }

    /// List spendable outputs and return (total_balance_sats, spendable_output_count).
    pub async fn get_balance_and_count(&self) -> Result<(u64, u64), DmError> {
        let mut total: u64 = 0;
        let mut count: u64 = 0;
        let mut offset: u64 = 0;
        let limit: u64 = 100;
        loop {
            let result = self
                .call(
                    "listOutputs",
                    json!({
                        "basket": "default",
                        "include": "locking scripts",
                        "limit": limit,
                        "offset": offset,
                    }),
                )
                .await?;
            let outputs = result
                .get("outputs")
                .and_then(|v| v.as_array())
                .unwrap_or(&Vec::new())
                .clone();
            if outputs.is_empty() {
                break;
            }
            for output in &outputs {
                if output
                    .get("spendable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    total += output.get("satoshis").and_then(|v| v.as_u64()).unwrap_or(0);
                    count += 1;
                }
            }
            if (outputs.len() as u64) < limit {
                break;
            }
            offset += limit;
        }
        Ok((total, count))
    }

    /// List outputs (UTXOs) from the wallet.
    pub async fn list_outputs(
        &self,
        basket: &str,
        include: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        self.call(
            "listOutputs",
            json!({
                "basket": basket,
                "include": include,
                "limit": limit,
                "offset": offset,
            }),
        )
        .await
    }

    /// List actions (transactions) from the wallet.
    pub async fn list_actions(
        &self,
        labels: &[&str],
        include_labels: bool,
        include_inputs: bool,
        include_outputs: bool,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        self.call(
            "listActions",
            json!({
                "labels": labels,
                "labelQueryMode": "any",
                "includeLabels": include_labels,
                "includeInputs": include_inputs,
                "includeOutputs": include_outputs,
                "limit": limit,
                "offset": offset,
            }),
        )
        .await
    }

    /// Generic wallet endpoint call. Routes to POST (with body) or GET (no body).
    /// This is the public bridge for the `wallet_call` tool — lets the agent
    /// call any BRC-100 endpoint by name without a dedicated wrapper method.
    pub async fn raw_call(&self, method: &str, params: Option<Value>) -> Result<Value, DmError> {
        match params {
            Some(p) => self.call(method, p).await,
            None => self.call_get(method).await,
        }
    }

    // ── Status endpoints (GET) ───────────────────────────────────────

    /// Check if wallet is reachable and authenticated.
    pub async fn is_authenticated(&self) -> Result<Value, DmError> {
        self.call_get("isAuthenticated").await
    }

    /// Get the current block height.
    pub async fn get_height(&self) -> Result<u64, DmError> {
        let result = self.call_get("getHeight").await?;
        result
            .get("height")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| DmError::wallet("Missing 'height' in response"))
    }

    /// Get the wallet's network (e.g. "mainnet").
    pub async fn get_network(&self) -> Result<String, DmError> {
        let result = self.call_get("getNetwork").await?;
        result
            .get("network")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DmError::wallet("Missing 'network' in response"))
    }

    /// Get the wallet version string.
    pub async fn get_version(&self) -> Result<String, DmError> {
        let result = self.call_get("getVersion").await?;
        result
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DmError::wallet("Missing 'version' in response"))
    }

    /// Block until the wallet is authenticated. Useful at startup.
    pub async fn wait_for_authentication(&self) -> Result<Value, DmError> {
        self.call_get("waitForAuthentication").await
    }

    // ── Cryptography endpoints ───────────────────────────────────────

    /// Verify a signature created with create_signature.
    pub async fn verify_signature(
        &self,
        data: &[u8],
        signature: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        let result = self
            .call(
                "verifySignature",
                json!({
                    "data": data.to_vec(),
                    "signature": signature.to_vec(),
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                }),
            )
            .await?;
        Ok(result
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// Encrypt data using BRC-42 derived key.
    pub async fn encrypt(
        &self,
        plaintext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let result = self
            .call(
                "encrypt",
                json!({
                    "plaintext": plaintext.to_vec(),
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                }),
            )
            .await?;
        parse_byte_array(&result, "ciphertext")
    }

    /// Decrypt data using BRC-42 derived key.
    pub async fn decrypt(
        &self,
        ciphertext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let result = self
            .call(
                "decrypt",
                json!({
                    "ciphertext": ciphertext.to_vec(),
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                }),
            )
            .await?;
        parse_byte_array(&result, "plaintext")
    }

    /// Create an HMAC using BRC-42 derived key.
    pub async fn create_hmac(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let result = self
            .call(
                "createHmac",
                json!({
                    "data": data.to_vec(),
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                }),
            )
            .await?;
        parse_byte_array(&result, "hmac")
    }

    /// Verify an HMAC created with create_hmac.
    pub async fn verify_hmac(
        &self,
        data: &[u8],
        hmac: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        let result = self
            .call(
                "verifyHmac",
                json!({
                    "data": data.to_vec(),
                    "hmac": hmac.to_vec(),
                    "protocolID": protocol_id,
                    "keyID": key_id,
                    "counterparty": counterparty,
                }),
            )
            .await?;
        Ok(result
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    // ── Chain endpoints ──────────────────────────────────────────────

    /// Get a block header by height (hex-encoded).
    pub async fn get_header_for_height(&self, height: u64) -> Result<String, DmError> {
        let result = self
            .call("getHeaderForHeight", json!({"height": height}))
            .await?;
        result
            .get("header")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DmError::wallet("Missing 'header' in response"))
    }

    // ── Transaction endpoints ────────────────────────────────────────

    /// Sign a previously created action (when signAndProcess was false).
    pub async fn sign_action(&self, reference: &str) -> Result<Value, DmError> {
        self.call("signAction", json!({"reference": reference}))
            .await
    }

    /// Abort a previously created action that hasn't been signed yet.
    pub async fn abort_action(&self, reference: &str) -> Result<Value, DmError> {
        self.call("abortAction", json!({"reference": reference}))
            .await
    }

    // ── Output endpoints ─────────────────────────────────────────────

    /// Release a specific output back to the wallet.
    pub async fn relinquish_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
    ) -> Result<Value, DmError> {
        self.call_with_retry(
            "relinquishOutput",
            json!({
                "basket": basket,
                "output": {"txid": txid, "vout": vout},
            }),
        )
        .await
    }

    // ── Certificate endpoints ────────────────────────────────────────

    /// Acquire a certificate from a certifier.
    ///
    /// The `certificate` value should contain all required fields at the top level:
    /// `certificateType`, `subject`, `certifier`, `serialNumber`, `revocationOutpoint`,
    /// `signature`, `fields`, and `acquisitionProtocol`.
    pub async fn acquire_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        self.call("acquireCertificate", certificate.clone()).await
    }

    /// List certificates, filtered by certifiers and/or types.
    pub async fn list_certificates(
        &self,
        certifiers: &[&str],
        types: &[&str],
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        self.call(
            "listCertificates",
            json!({
                "certifiers": certifiers,
                "types": types,
                "limit": limit,
                "offset": offset,
            }),
        )
        .await
    }

    /// Prove ownership of a certificate, revealing specified fields.
    pub async fn prove_certificate(
        &self,
        certificate: &Value,
        fields_to_reveal: &[&str],
    ) -> Result<Value, DmError> {
        self.call(
            "proveCertificate",
            json!({
                "certificate": certificate,
                "fieldsToReveal": fields_to_reveal,
            }),
        )
        .await
    }

    /// Release a certificate from the wallet.
    ///
    /// The wallet expects top-level fields: certificateType, serialNumber, certifier.
    pub async fn relinquish_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        let cert_type = certificate
            .get("type")
            .or_else(|| certificate.get("certificateType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let serial = certificate
            .get("serialNumber")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let certifier = certificate
            .get("certifier")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        self.call(
            "relinquishCertificate",
            json!({
                "certificateType": cert_type,
                "serialNumber": serial,
                "certifier": certifier,
            }),
        )
        .await
    }

    // ── Discovery endpoints ──────────────────────────────────────────

    /// Discover certificates by identity key.
    pub async fn discover_by_identity_key(
        &self,
        identity_key: &str,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        let mut params = json!({"identityKey": identity_key, "limit": limit});
        if let Some(t) = cert_type {
            params["type"] = json!(t);
        }
        self.call("discoverByIdentityKey", params).await
    }

    /// Discover certificates by attributes.
    pub async fn discover_by_attributes(
        &self,
        attributes: &Value,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        let mut params = json!({"attributes": attributes, "limit": limit});
        if let Some(t) = cert_type {
            params["type"] = json!(t);
        }
        self.call("discoverByAttributes", params).await
    }

    // ── Key linkage endpoints ────────────────────────────────────────

    /// Reveal counterparty key linkage to a verifier.
    pub async fn reveal_counterparty_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        self.call(
            "revealCounterpartyKeyLinkage",
            json!({
                "counterparty": counterparty,
                "verifier": verifier,
                "privileged": privileged,
            }),
        )
        .await
    }

    /// Reveal specific key linkage for a protocol+key to a verifier.
    pub async fn reveal_specific_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        protocol_id: &Value,
        key_id: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        self.call(
            "revealSpecificKeyLinkage",
            json!({
                "counterparty": counterparty,
                "verifier": verifier,
                "protocolID": protocol_id,
                "keyID": key_id,
                "privileged": privileged,
            }),
        )
        .await
    }

    /// Generate a receiving address that the wallet can later spend.
    ///
    /// Returns `(pubkey_hex, p2pkh_script_hex, derivation_suffix)`.
    /// The caller should display the address and save the suffix for later internalization.
    pub async fn receive_address(&self, suffix: &str) -> Result<(String, String, String), DmError> {
        let protocol_id = Self::funding_protocol();
        let key_id = format!("{} {suffix}", Self::FUNDING_PREFIX);

        let pubkey = self
            .get_public_key(&protocol_id, &key_id, ANYONE_KEY, true)
            .await?;

        let script = crate::x402::payment::build_p2pkh_script(&pubkey)?;

        Ok((pubkey, script, suffix.to_string()))
    }

    /// Internalize an external funding transaction using BEEF from WhatsOnChain.
    ///
    /// The `derivation_suffix` must match the suffix used when generating the
    /// receiving address via `receive_address()`. If not provided, a default
    /// suffix of `"1"` is used (matching the default `dolphin-milk receive` output).
    pub async fn fund_from_woc(
        &self,
        txid: &str,
        vout: Option<u32>,
        derivation_suffix: Option<&str>,
    ) -> Result<Value, DmError> {
        let suffix = derivation_suffix.unwrap_or("1");

        // Validate txid
        if txid.len() != 64 {
            return Err(DmError::wallet(format!(
                "Invalid txid: expected 64 hex chars, got {}",
                txid.len()
            )));
        }
        hex::decode(txid).map_err(|_| DmError::wallet(format!("Invalid hex in txid: {txid}")))?;

        // Step 1: Derive the expected payment key and script
        let protocol_id = Self::funding_protocol();
        let key_id = format!("{} {suffix}", Self::FUNDING_PREFIX);

        let derived_pubkey = self
            .get_public_key(&protocol_id, &key_id, ANYONE_KEY, true)
            .await?;

        let expected_script = crate::x402::payment::build_p2pkh_script(&derived_pubkey)?;

        tracing::info!(
            "Expected P2PKH script for suffix '{}': {}...{}",
            suffix,
            &expected_script[..10],
            &expected_script[expected_script.len() - 4..],
        );

        // Step 2: Fetch the transaction from WOC
        let tx_url = format!("https://api.whatsonchain.com/v1/bsv/main/tx/{txid}");
        let tx_resp = self
            .client
            .get(&tx_url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to fetch tx from WOC: {e}")))?;

        if !tx_resp.status().is_success() {
            return Err(DmError::wallet(format!(
                "WOC returned HTTP {} for tx {txid}",
                tx_resp.status()
            )));
        }

        let tx_json: Value = tx_resp
            .json()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to parse tx JSON: {e}")))?;

        let vout_array = tx_json
            .get("vout")
            .and_then(|v| v.as_array())
            .ok_or_else(|| DmError::wallet("Missing 'vout' in tx JSON"))?;

        // Step 2b: Auto-detect vout if not specified
        let vout = match vout {
            Some(v) => v,
            None => {
                let matches: Vec<(u32, u64)> = vout_array
                    .iter()
                    .enumerate()
                    .filter_map(|(i, output)| {
                        let script = output
                            .get("scriptPubKey")
                            .and_then(|sp| sp.get("hex"))
                            .and_then(|h| h.as_str())
                            .unwrap_or("");
                        if script.to_lowercase() == expected_script.to_lowercase() {
                            let sats = output
                                .get("value")
                                .and_then(|v| v.as_f64())
                                .map(|v| (v * 1e8) as u64)
                                .unwrap_or(0);
                            Some((i as u32, sats))
                        } else {
                            None
                        }
                    })
                    .collect();

                match matches.len() {
                    0 => {
                        return Err(DmError::wallet(format!(
                            "No outputs in tx {txid} match the expected script for suffix '{suffix}'.\n\
                             Expected: {expected_script}\n\
                             The tx has {} outputs — none match a wallet-derived address.\n\
                             Use 'dolphin-milk receive --suffix {suffix}' to check your address.",
                            vout_array.len(),
                        )));
                    }
                    1 => {
                        let (idx, sats) = matches[0];
                        tracing::info!("Auto-detected matching output at vout {idx} ({sats} sats)");
                        idx
                    }
                    _ => {
                        let listing: Vec<String> = matches
                            .iter()
                            .map(|(idx, sats)| format!("  vout {idx}: {sats} sats"))
                            .collect();
                        return Err(DmError::wallet(format!(
                            "Multiple outputs in tx {txid} match the expected script:\n{}\n\
                             Specify which one with --vout <N>",
                            listing.join("\n"),
                        )));
                    }
                }
            }
        };

        let output = vout_array.get(vout as usize).ok_or_else(|| {
            DmError::wallet(format!(
                "Output index {} out of range (tx has {} outputs)",
                vout,
                vout_array.len()
            ))
        })?;

        let actual_script = output
            .get("scriptPubKey")
            .and_then(|sp| sp.get("hex"))
            .and_then(|h| h.as_str())
            .unwrap_or("");

        if actual_script.to_lowercase() != expected_script.to_lowercase() {
            let actual_sats = output
                .get("value")
                .and_then(|v| v.as_f64())
                .map(|v| (v * 1e8) as u64)
                .unwrap_or(0);

            return Err(DmError::wallet(format!(
                "Script mismatch for tx {txid} vout {vout} ({actual_sats} sats).\n\
                 Expected: {expected_script}\n\
                 Actual:   {actual_script}\n\
                 This output was NOT sent to a wallet-derived address.\n\
                 Use 'dolphin-milk receive' to generate a proper receiving address first,\n\
                 then send BSV to that address."
            )));
        }

        let actual_sats = output
            .get("value")
            .and_then(|v| v.as_f64())
            .map(|v| (v * 1e8) as u64)
            .unwrap_or(0);

        tracing::info!(
            "Script verified: output {vout} ({actual_sats} sats) matches derived key with suffix '{suffix}'"
        );

        // Step 3: Fetch BEEF from WhatsOnChain
        let woc_url = format!("https://api.whatsonchain.com/v1/bsv/main/tx/{txid}/beef");
        tracing::info!("Fetching BEEF from WOC: {}", woc_url);

        let resp = self
            .client
            .get(&woc_url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to fetch BEEF from WOC: {e}")))?;

        if !resp.status().is_success() {
            return Err(DmError::wallet(format!(
                "WOC returned HTTP {} for BEEF of tx {txid}",
                resp.status()
            )));
        }

        let raw = resp
            .bytes()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to read WOC response: {e}")))?;

        // WOC may return hex string or raw bytes
        let beef_bytes = parse_woc_beef(&raw)?;

        if beef_bytes.len() < 4 {
            return Err(DmError::wallet(format!(
                "BEEF data too short ({} bytes) for tx {txid}",
                beef_bytes.len()
            )));
        }

        tracing::info!("Fetched BEEF: {} bytes for tx {}", beef_bytes.len(), txid);

        // Step 4: Build AtomicBEEF: [0x01, 0x01, 0x01, 0x01] + reversed_txid(32) + beef_bytes
        let mut atomic_beef = vec![0x01u8, 0x01, 0x01, 0x01];
        let txid_bytes = hex::decode(txid).unwrap();
        let reversed: Vec<u8> = txid_bytes.into_iter().rev().collect();
        atomic_beef.extend_from_slice(&reversed);
        atomic_beef.extend_from_slice(&beef_bytes);

        // Step 5: Internalize with verified derivation parameters
        let outputs = vec![json!({
            "outputIndex": vout,
            "protocol": "wallet payment",
            "paymentRemittance": {
                "derivationPrefix": Self::FUNDING_PREFIX,
                "derivationSuffix": suffix,
                "senderIdentityKey": ANYONE_KEY,
            },
        })];

        tracing::info!(
            "Internalizing tx {} vout {} ({} byte AtomicBEEF, {} sats)",
            txid,
            vout,
            atomic_beef.len(),
            actual_sats,
        );

        let result = self
            .internalize_action(
                &atomic_beef,
                &outputs,
                &format!("Fund from external tx {}", &txid[..16]),
            )
            .await?;

        let accepted = result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !accepted {
            return Err(DmError::wallet(format!(
                "Wallet rejected internalization of tx {txid}: {result}"
            )));
        }

        Ok(result)
    }

    // ── Split ────────────────────────────────────────────────────
    //
    // HTTP-backed UTXO split — mirrors `EmbeddedWalletClient::split_utxos`
    // and `bsv-wallet-cli/src/commands/split.rs` byte-for-byte, but driven
    // over the bsv-wallet daemon's JSON-RPC API instead of the in-process
    // toolbox primitives.
    //
    // Both backends must produce IDENTICAL on-chain UTXO layouts for the
    // same input state. The hardcoded constants, fee math, basket/tag/label
    // semantics, and the createAction→internalizeAction sequence are all
    // copied from the CLI reference implementation. If you change one, you
    // must update the embedded copy AND bsv-wallet-cli's split.rs in lockstep.

    /// Mirrors `bsv-wallet-cli` split constants.
    const SPLIT_DERIVATION_PREFIX: &'static str = "SfKxPIJNgdI=";
    const SPLIT_DERIVATION_SUFFIX: &'static str = "NaGLC6fMH50=";
    const SPLIT_BRC29_PROTOCOL: &'static str = "3241645161d8";

    pub async fn split_utxos(&self, count: u32) -> Result<(String, u64, u32), DmError> {
        if count < 2 {
            return Err(DmError::wallet("Split count must be at least 2"));
        }

        // Step 1: enumerate UTXOs from the default basket.
        let list_result = self
            .call(
                "listOutputs",
                json!({
                    "basket": "default",
                    "limit": 1000,
                }),
            )
            .await?;
        let outputs_arr = list_result
            .get("outputs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let utxo_count = outputs_arr.len();
        let total_sats: u64 = outputs_arr
            .iter()
            .map(|o| o.get("satoshis").and_then(|v| v.as_u64()).unwrap_or(0))
            .sum();

        if utxo_count == 0 || total_sats == 0 {
            return Err(DmError::wallet(format!(
                "No UTXOs to split (balance: {total_sats} sats, {utxo_count} UTXOs)"
            )));
        }

        // Step 2: dynamic fee reserve (matches embedded + bsv-wallet-cli).
        let estimated_tx_bytes: u64 = (utxo_count as u64 * 148) + (count as u64 * 34) + 10;
        let fee_reserve: u64 = estimated_tx_bytes.max(500);
        if total_sats <= fee_reserve {
            return Err(DmError::wallet(format!(
                "Balance too low to split ({total_sats} sats, need > {fee_reserve} for fees)"
            )));
        }
        let available = total_sats - fee_reserve;
        let per_output = available / count as u64;
        if per_output < 1 {
            return Err(DmError::wallet(format!(
                "Cannot create {count} outputs from {available} available sats"
            )));
        }

        // Step 3: derive the wallet's own P2PKH locking script.
        // Same derivation path as bsv-wallet-cli so the outputs become
        // spendable after internalizeAction("wallet payment").
        let protocol_id = json!([2, Self::SPLIT_BRC29_PROTOCOL]);
        let key_id = format!(
            "{} {}",
            Self::SPLIT_DERIVATION_PREFIX,
            Self::SPLIT_DERIVATION_SUFFIX
        );
        let derived_pubkey_hex = self
            .get_public_key(&protocol_id, &key_id, ANYONE_KEY, true)
            .await?;
        let derived_pubkey = PublicKey::from_hex(&derived_pubkey_hex)
            .map_err(|e| DmError::wallet(format!("split decode pubkey: {e}")))?;
        let address = derived_pubkey.to_address();
        let lock = P2PKH::lock_from_address(&address)
            .map_err(|e| DmError::wallet(format!("split build P2PKH: {e}")))?;
        let lock_hex = hex::encode(lock.to_binary());

        // Step 4: build N outputs via createAction JSON. basket:null is
        // CRITICAL — a basketed output won't be promoted by the subsequent
        // internalize_action merge path. See embedded.rs for details.
        let outputs: Vec<Value> = (0..count)
            .map(|_| {
                json!({
                    "lockingScript": lock_hex,
                    "satoshis": per_output,
                    "outputDescription": "split output",
                    "basket": null,
                    "tags": ["relinquish"],
                })
            })
            .collect();

        // Acquire spend semaphore to serialize against other wallet writes.
        let _permit = SPEND_SEMAPHORE.acquire().await.unwrap();

        let create_result = self
            .call_with_retry(
                "createAction",
                json!({
                    "description": format!("Split into {count} UTXOs ({per_output} sats each)"),
                    "outputs": outputs,
                    "labels": ["split"],
                    "options": {
                        "randomizeOutputs": false,
                        "signAndProcess": true,
                        "noSend": false,
                    },
                }),
            )
            .await?;
        drop(_permit);

        let txid = create_result
            .get("txid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DmError::wallet("split: createAction returned no txid"))?
            .to_string();

        // Step 5: self-internalize each output via "wallet payment" protocol
        // so the toolbox promotes them to change=1 and makes them available
        // to the coin selector on subsequent createAction calls.
        //
        // Per CreateActionResult spec in bsv-rs: the `tx` field IS the
        // transaction in AtomicBEEF format (hex-encoded). We can pass it
        // directly to internalizeAction — no manual Beef construction needed.
        let atomic_bytes: Vec<u8> = create_result
            .get("tx")
            .and_then(|v| {
                // Might be serialized as an array of bytes or a hex string.
                if let Some(arr) = v.as_array() {
                    Some(
                        arr.iter()
                            .filter_map(|x| x.as_u64().map(|n| n as u8))
                            .collect(),
                    )
                } else if let Some(s) = v.as_str() {
                    hex::decode(s).ok()
                } else {
                    None
                }
            })
            .ok_or_else(|| DmError::wallet("split: createAction returned no tx field"))?;
        if atomic_bytes.is_empty() {
            return Err(DmError::wallet("split: createAction tx was empty"));
        }

        let (_, anyone_pubkey) = KeyDeriver::anyone_key();
        let sender_identity_key = anyone_pubkey.to_hex();

        let internalize_outputs: Vec<Value> = (0..count)
            .map(|i| {
                json!({
                    "outputIndex": i,
                    "protocol": "wallet payment",
                    "paymentRemittance": {
                        "derivationPrefix": Self::SPLIT_DERIVATION_PREFIX,
                        "derivationSuffix": Self::SPLIT_DERIVATION_SUFFIX,
                        "senderIdentityKey": sender_identity_key,
                    },
                })
            })
            .collect();

        self.call(
            "internalizeAction",
            json!({
                "tx": atomic_bytes,
                "outputs": internalize_outputs,
                "description": "Self-internalize split outputs",
                "labels": ["split"],
            }),
        )
        .await?;

        Ok((txid, per_output, count))
    }
}

// ── WalletBackend trait impl (delegates to inherent methods) ─────────

#[async_trait::async_trait]
impl WalletBackend for HttpWalletClient {
    async fn get_identity_key(&self) -> Result<String, DmError> {
        HttpWalletClient::get_identity_key(self).await
    }

    async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, DmError> {
        HttpWalletClient::get_public_key(self, protocol_id, key_id, counterparty, for_self).await
    }

    async fn raw_call(&self, method: &str, params: Option<Value>) -> Result<Value, DmError> {
        HttpWalletClient::raw_call(self, method, params).await
    }

    async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        randomize_outputs: bool,
    ) -> Result<CreateActionResult, DmError> {
        HttpWalletClient::create_action(
            self,
            outputs,
            description,
            accept_delayed_broadcast,
            randomize_outputs,
        )
        .await
    }

    async fn spend_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
        description: &str,
    ) -> Result<CreateActionResult, DmError> {
        HttpWalletClient::spend_output(self, basket, txid, vout, description).await
    }

    async fn internalize_action(
        &self,
        tx_bytes: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, DmError> {
        HttpWalletClient::internalize_action(self, tx_bytes, outputs, description).await
    }

    async fn split_utxos(&self, count: u32) -> Result<(String, u64, u32), DmError> {
        HttpWalletClient::split_utxos(self, count).await
    }

    async fn get_balance(&self) -> Result<u64, DmError> {
        HttpWalletClient::get_balance(self).await
    }

    async fn get_balance_and_count(&self) -> Result<(u64, u64), DmError> {
        HttpWalletClient::get_balance_and_count(self).await
    }

    async fn list_outputs(
        &self,
        basket: &str,
        include: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        HttpWalletClient::list_outputs(self, basket, include, limit, offset).await
    }

    async fn relinquish_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
    ) -> Result<Value, DmError> {
        HttpWalletClient::relinquish_output(self, basket, txid, vout).await
    }

    async fn create_signature(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        HttpWalletClient::create_signature(self, data, protocol_id, key_id, counterparty).await
    }

    async fn verify_signature(
        &self,
        data: &[u8],
        signature: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        HttpWalletClient::verify_signature(self, data, signature, protocol_id, key_id, counterparty)
            .await
    }

    async fn encrypt(
        &self,
        plaintext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        HttpWalletClient::encrypt(self, plaintext, protocol_id, key_id, counterparty).await
    }

    async fn decrypt(
        &self,
        ciphertext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        HttpWalletClient::decrypt(self, ciphertext, protocol_id, key_id, counterparty).await
    }

    async fn create_hmac(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        HttpWalletClient::create_hmac(self, data, protocol_id, key_id, counterparty).await
    }

    async fn verify_hmac(
        &self,
        data: &[u8],
        hmac: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        HttpWalletClient::verify_hmac(self, data, hmac, protocol_id, key_id, counterparty).await
    }

    async fn acquire_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        HttpWalletClient::acquire_certificate(self, certificate).await
    }

    async fn list_certificates(
        &self,
        certifiers: &[&str],
        types: &[&str],
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        HttpWalletClient::list_certificates(self, certifiers, types, limit, offset).await
    }

    async fn prove_certificate(
        &self,
        certificate: &Value,
        fields_to_reveal: &[&str],
    ) -> Result<Value, DmError> {
        HttpWalletClient::prove_certificate(self, certificate, fields_to_reveal).await
    }

    async fn relinquish_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        HttpWalletClient::relinquish_certificate(self, certificate).await
    }

    async fn discover_by_identity_key(
        &self,
        identity_key: &str,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        HttpWalletClient::discover_by_identity_key(self, identity_key, cert_type, limit).await
    }

    async fn discover_by_attributes(
        &self,
        attributes: &Value,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        HttpWalletClient::discover_by_attributes(self, attributes, cert_type, limit).await
    }

    async fn reveal_counterparty_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        HttpWalletClient::reveal_counterparty_key_linkage(self, counterparty, verifier, privileged)
            .await
    }

    async fn reveal_specific_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        protocol_id: &Value,
        key_id: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        HttpWalletClient::reveal_specific_key_linkage(
            self,
            counterparty,
            verifier,
            protocol_id,
            key_id,
            privileged,
        )
        .await
    }

    async fn is_authenticated(&self) -> Result<Value, DmError> {
        HttpWalletClient::is_authenticated(self).await
    }

    async fn get_height(&self) -> Result<u64, DmError> {
        HttpWalletClient::get_height(self).await
    }

    async fn get_network(&self) -> Result<String, DmError> {
        HttpWalletClient::get_network(self).await
    }

    async fn get_version(&self) -> Result<String, DmError> {
        HttpWalletClient::get_version(self).await
    }

    async fn wait_for_authentication(&self) -> Result<Value, DmError> {
        HttpWalletClient::wait_for_authentication(self).await
    }

    async fn get_header_for_height(&self, height: u64) -> Result<String, DmError> {
        HttpWalletClient::get_header_for_height(self, height).await
    }

    async fn sign_action(&self, reference: &str) -> Result<Value, DmError> {
        HttpWalletClient::sign_action(self, reference).await
    }

    async fn abort_action(&self, reference: &str) -> Result<Value, DmError> {
        HttpWalletClient::abort_action(self, reference).await
    }

    async fn list_actions(
        &self,
        labels: &[&str],
        include_labels: bool,
        include_inputs: bool,
        include_outputs: bool,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        HttpWalletClient::list_actions(
            self,
            labels,
            include_labels,
            include_inputs,
            include_outputs,
            limit,
            offset,
        )
        .await
    }

    async fn receive_address(&self, suffix: &str) -> Result<(String, String, String), DmError> {
        HttpWalletClient::receive_address(self, suffix).await
    }

    async fn fund_from_woc(
        &self,
        txid: &str,
        vout: Option<u32>,
        derivation_suffix: Option<&str>,
    ) -> Result<Value, DmError> {
        HttpWalletClient::fund_from_woc(self, txid, vout, derivation_suffix).await
    }
}

/// Extract a byte array field from a wallet JSON response.
fn parse_byte_array(value: &Value, field: &str) -> Result<Vec<u8>, DmError> {
    let arr = value
        .get(field)
        .and_then(|v| v.as_array())
        .ok_or_else(|| DmError::wallet(format!("Missing '{field}' array in response")))?;
    arr.iter()
        .map(|v| {
            v.as_u64()
                .map(|n| n as u8)
                .ok_or_else(|| DmError::wallet(format!("Invalid byte in '{field}' array")))
        })
        .collect()
}

/// Parse BEEF bytes from WOC response (may be hex string or raw bytes).
fn parse_woc_beef(raw: &[u8]) -> Result<Vec<u8>, DmError> {
    // If it looks like a quoted hex string, strip quotes
    let data = if raw.starts_with(b"\"") && raw.ends_with(b"\"") {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };

    // Try to interpret as hex string
    if let Ok(hex_str) = std::str::from_utf8(data) {
        let trimmed = hex_str.trim();
        if trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return hex::decode(trimmed)
                .map_err(|e| DmError::wallet(format!("Invalid hex in BEEF data: {e}")));
        }
    }

    // Raw binary bytes
    Ok(data.to_vec())
}
