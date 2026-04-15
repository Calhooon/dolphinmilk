//! BRC-100 wallet interface — trait definition and HTTP implementation.

mod http;
mod types;

#[cfg(feature = "embedded-wallet")]
mod embedded;

pub use http::HttpWalletClient;
pub use types::{CreateActionResult, ANYONE_KEY};

#[cfg(feature = "embedded-wallet")]
pub use embedded::EmbeddedWalletClient;

// Default wallet backend — HTTP for now. Server handlers create wallet clients
// on demand via from_config(). The embedded wallet is used by `dolphin-milk init`
// to create the wallet DB in-process, and `dolphin-milk start` auto-spawns
// `bsv-wallet daemon` as a child process (HTTP API + background Monitor for proof syncing).
pub type WalletClient = HttpWalletClient;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::DmError;

/// Abstract wallet backend implementing BRC-100 operations.
///
/// Every method maps to a BRC-100 wallet endpoint. The default implementation
/// is [`HttpWalletClient`], which talks to `bsv-wallet-cli` over HTTP.
#[async_trait]
pub trait WalletBackend: Send + Sync {
    // ── Key Management ───────────────────────────────────────────────

    /// Get the wallet's identity public key (66-char hex compressed).
    async fn get_identity_key(&self) -> Result<String, DmError>;

    /// Derive a public key via BRC-42 key derivation.
    async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, DmError>;

    /// Generic wallet endpoint call. Routes to POST (with body) or GET (no body).
    async fn raw_call(&self, method: &str, params: Option<Value>) -> Result<Value, DmError>;

    // ── Transaction Creation ─────────────────────────────────────────

    /// Create a transaction via the wallet.
    async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        randomize_outputs: bool,
    ) -> Result<CreateActionResult, DmError>;

    /// Spend a specific output on-chain via createAction with inputs.
    async fn spend_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
        description: &str,
    ) -> Result<CreateActionResult, DmError>;

    /// Internalize an incoming transaction into the wallet.
    async fn internalize_action(
        &self,
        tx_bytes: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, DmError>;

    /// Atomically split the wallet's spendable balance into `count` equal-sized
    /// UTXOs. Returns `(txid, sats_per_output, count)`.
    ///
    /// Primary use case: parallel-agent funding — spawn N sub-agents, each
    /// claims one split output from a single initial deposit.
    ///
    /// Default impl returns "not supported" — only `EmbeddedWalletClient`
    /// overrides this (the split sequence uses toolbox primitives that aren't
    /// exposed through the raw HTTP wallet surface). HTTP-backed callers
    /// should use `dolphin-milk split` CLI instead.
    async fn split_utxos(&self, _count: u32) -> Result<(String, u64, u32), DmError> {
        Err(DmError::wallet(
            "split not supported by this wallet backend — \
             use `dolphin-milk split <count>` CLI, \
             or rebuild the server with the `embedded-wallet` feature",
        ))
    }

    // ── Output Management ────────────────────────────────────────────

    /// List spendable outputs and return total balance in satoshis.
    async fn get_balance(&self) -> Result<u64, DmError>;

    /// List spendable outputs and return (total_balance_sats, spendable_output_count).
    /// Default impl calls get_balance() and returns count=0; override for real counts.
    async fn get_balance_and_count(&self) -> Result<(u64, u64), DmError> {
        let balance = self.get_balance().await?;
        Ok((balance, 0))
    }

    /// List outputs (UTXOs) from the wallet.
    async fn list_outputs(
        &self,
        basket: &str,
        include: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError>;

    /// Release a specific output back to the wallet.
    async fn relinquish_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
    ) -> Result<Value, DmError>;

    // ── Cryptographic Operations ─────────────────────────────────────

    /// Sign data with a derived key via BRC-42 + ECDSA.
    async fn create_signature(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError>;

    /// Verify a signature created with create_signature.
    async fn verify_signature(
        &self,
        data: &[u8],
        signature: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError>;

    /// Encrypt data using BRC-42 derived key.
    async fn encrypt(
        &self,
        plaintext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError>;

    /// Decrypt data using BRC-42 derived key.
    async fn decrypt(
        &self,
        ciphertext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError>;

    /// Create an HMAC using BRC-42 derived key.
    async fn create_hmac(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError>;

    /// Verify an HMAC created with create_hmac.
    async fn verify_hmac(
        &self,
        data: &[u8],
        hmac: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError>;

    // ── Certificate Operations ───────────────────────────────────────

    /// Acquire a certificate from a certifier.
    async fn acquire_certificate(&self, certificate: &Value) -> Result<Value, DmError>;

    /// List certificates, filtered by certifiers and/or types.
    async fn list_certificates(
        &self,
        certifiers: &[&str],
        types: &[&str],
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError>;

    /// Prove ownership of a certificate, revealing specified fields.
    async fn prove_certificate(
        &self,
        certificate: &Value,
        fields_to_reveal: &[&str],
    ) -> Result<Value, DmError>;

    /// Release a certificate from the wallet.
    async fn relinquish_certificate(&self, certificate: &Value) -> Result<Value, DmError>;

    // ── Discovery Operations ─────────────────────────────────────────

    /// Discover certificates by identity key.
    async fn discover_by_identity_key(
        &self,
        identity_key: &str,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError>;

    /// Discover certificates by attributes.
    async fn discover_by_attributes(
        &self,
        attributes: &Value,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError>;

    // ── Key Linkage Operations ───────────────────────────────────────

    /// Reveal counterparty key linkage to a verifier.
    async fn reveal_counterparty_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        privileged: bool,
    ) -> Result<Value, DmError>;

    /// Reveal specific key linkage for a protocol+key to a verifier.
    async fn reveal_specific_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        protocol_id: &Value,
        key_id: &str,
        privileged: bool,
    ) -> Result<Value, DmError>;

    // ── Status / Metadata ────────────────────────────────────────────

    /// Check if wallet is reachable and authenticated.
    async fn is_authenticated(&self) -> Result<Value, DmError>;

    /// Get the current block height.
    async fn get_height(&self) -> Result<u64, DmError>;

    /// Get the wallet's network (e.g. "mainnet").
    async fn get_network(&self) -> Result<String, DmError>;

    /// Get the wallet version string.
    async fn get_version(&self) -> Result<String, DmError>;

    /// Block until the wallet is authenticated. Useful at startup.
    async fn wait_for_authentication(&self) -> Result<Value, DmError>;

    // ── Chain Operations ─────────────────────────────────────────────

    /// Get a block header by height (hex-encoded).
    async fn get_header_for_height(&self, height: u64) -> Result<String, DmError>;

    // ── Action Management ────────────────────────────────────────────

    /// Sign a previously created action (when signAndProcess was false).
    async fn sign_action(&self, reference: &str) -> Result<Value, DmError>;

    /// Abort a previously created action that hasn't been signed yet.
    async fn abort_action(&self, reference: &str) -> Result<Value, DmError>;

    // ── Transaction History ──────────────────────────────────────────

    /// List actions (transactions) from the wallet.
    async fn list_actions(
        &self,
        labels: &[&str],
        include_labels: bool,
        include_inputs: bool,
        include_outputs: bool,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError>;

    // ── Funding Operations ───────────────────────────────────────────

    /// Generate a receiving address that the wallet can later spend.
    async fn receive_address(&self, suffix: &str) -> Result<(String, String, String), DmError>;

    /// Internalize an external funding transaction using BEEF from WhatsOnChain.
    /// If `vout` is None, auto-detects the matching output.
    async fn fund_from_woc(
        &self,
        txid: &str,
        vout: Option<u32>,
        derivation_suffix: Option<&str>,
    ) -> Result<Value, DmError>;
}

// ── Blanket impl: Arc<dyn WalletBackend> delegates to the inner trait object ──
//
// This allows code holding `Arc<dyn WalletBackend>` to pass `&self` where
// `&dyn WalletBackend` is expected, without manual dereferencing.
#[async_trait]
impl WalletBackend for std::sync::Arc<dyn WalletBackend> {
    async fn get_identity_key(&self) -> Result<String, DmError> {
        (**self).get_identity_key().await
    }
    async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, DmError> {
        (**self)
            .get_public_key(protocol_id, key_id, counterparty, for_self)
            .await
    }
    async fn raw_call(&self, method: &str, params: Option<Value>) -> Result<Value, DmError> {
        (**self).raw_call(method, params).await
    }
    async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        randomize_outputs: bool,
    ) -> Result<CreateActionResult, DmError> {
        (**self)
            .create_action(
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
        (**self).spend_output(basket, txid, vout, description).await
    }
    async fn internalize_action(
        &self,
        tx_bytes: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, DmError> {
        (**self)
            .internalize_action(tx_bytes, outputs, description)
            .await
    }
    async fn get_balance(&self) -> Result<u64, DmError> {
        (**self).get_balance().await
    }
    async fn get_balance_and_count(&self) -> Result<(u64, u64), DmError> {
        (**self).get_balance_and_count().await
    }
    async fn list_outputs(
        &self,
        basket: &str,
        include: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        (**self).list_outputs(basket, include, limit, offset).await
    }
    async fn relinquish_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
    ) -> Result<Value, DmError> {
        (**self).relinquish_output(basket, txid, vout).await
    }
    async fn create_signature(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        (**self)
            .create_signature(data, protocol_id, key_id, counterparty)
            .await
    }
    async fn verify_signature(
        &self,
        data: &[u8],
        signature: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        (**self)
            .verify_signature(data, signature, protocol_id, key_id, counterparty)
            .await
    }
    async fn encrypt(
        &self,
        plaintext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        (**self)
            .encrypt(plaintext, protocol_id, key_id, counterparty)
            .await
    }
    async fn decrypt(
        &self,
        ciphertext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        (**self)
            .decrypt(ciphertext, protocol_id, key_id, counterparty)
            .await
    }
    async fn create_hmac(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        (**self)
            .create_hmac(data, protocol_id, key_id, counterparty)
            .await
    }
    async fn verify_hmac(
        &self,
        data: &[u8],
        hmac: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        (**self)
            .verify_hmac(data, hmac, protocol_id, key_id, counterparty)
            .await
    }
    async fn acquire_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        (**self).acquire_certificate(certificate).await
    }
    async fn list_certificates(
        &self,
        certifiers: &[&str],
        types: &[&str],
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        (**self)
            .list_certificates(certifiers, types, limit, offset)
            .await
    }
    async fn prove_certificate(
        &self,
        certificate: &Value,
        fields_to_reveal: &[&str],
    ) -> Result<Value, DmError> {
        (**self)
            .prove_certificate(certificate, fields_to_reveal)
            .await
    }
    async fn relinquish_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        (**self).relinquish_certificate(certificate).await
    }
    async fn discover_by_identity_key(
        &self,
        identity_key: &str,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        (**self)
            .discover_by_identity_key(identity_key, cert_type, limit)
            .await
    }
    async fn discover_by_attributes(
        &self,
        attributes: &Value,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        (**self)
            .discover_by_attributes(attributes, cert_type, limit)
            .await
    }
    async fn reveal_counterparty_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        (**self)
            .reveal_counterparty_key_linkage(counterparty, verifier, privileged)
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
        (**self)
            .reveal_specific_key_linkage(counterparty, verifier, protocol_id, key_id, privileged)
            .await
    }
    async fn is_authenticated(&self) -> Result<Value, DmError> {
        (**self).is_authenticated().await
    }
    async fn get_height(&self) -> Result<u64, DmError> {
        (**self).get_height().await
    }
    async fn get_network(&self) -> Result<String, DmError> {
        (**self).get_network().await
    }
    async fn get_version(&self) -> Result<String, DmError> {
        (**self).get_version().await
    }
    async fn wait_for_authentication(&self) -> Result<Value, DmError> {
        (**self).wait_for_authentication().await
    }
    async fn get_header_for_height(&self, height: u64) -> Result<String, DmError> {
        (**self).get_header_for_height(height).await
    }
    async fn sign_action(&self, reference: &str) -> Result<Value, DmError> {
        (**self).sign_action(reference).await
    }
    async fn abort_action(&self, reference: &str) -> Result<Value, DmError> {
        (**self).abort_action(reference).await
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
        (**self)
            .list_actions(
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
        (**self).receive_address(suffix).await
    }
    async fn fund_from_woc(
        &self,
        txid: &str,
        vout: Option<u32>,
        derivation_suffix: Option<&str>,
    ) -> Result<Value, DmError> {
        (**self).fund_from_woc(txid, vout, derivation_suffix).await
    }
}
