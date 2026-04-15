//! Trait abstractions for wallet and auth clients.
//!
//! These traits decouple the x402 payment logic from specific wallet/auth
//! implementations. The consuming crate (bsv-worm) provides concrete impls.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::X402Error;

/// Result of a `createAction` wallet call.
#[derive(Debug, Clone)]
pub struct CreateActionResult {
    /// Raw transaction bytes (may be raw tx, BEEF, or AtomicBEEF).
    pub tx: Vec<u8>,
    /// Transaction ID (hex).
    pub txid: String,
}

/// Wallet operations needed by the x402 payment flow.
#[async_trait]
pub trait WalletApi: Send + Sync {
    /// Derive a public key via BRC-42 key derivation.
    async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, X402Error>;

    /// Create a funded transaction with the given outputs.
    async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        random_outputs: bool,
    ) -> Result<CreateActionResult, X402Error>;

    /// Internalize an incoming transaction (e.g., a refund).
    async fn internalize_action(
        &self,
        tx: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, X402Error>;
}

/// Response from an authenticated request.
pub struct AuthResponse {
    /// HTTP status code.
    pub status: reqwest::StatusCode,
    /// Response headers.
    pub headers: reqwest::header::HeaderMap,
    /// Response body as bytes.
    pub body: Vec<u8>,
}

/// Authenticated HTTP client operations needed by the x402 payment flow.
#[async_trait]
pub trait AuthClient: Send + Sync {
    /// Get a reference to the wallet API.
    fn wallet(&self) -> &dyn WalletApi;

    /// Send an authenticated request and return the raw response.
    async fn authenticated_request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::Response, X402Error>;

    /// Get the session directory path (for cache management).
    fn session_dir(&self) -> &std::path::Path;

    /// Clear a cached session for the given base URL.
    fn clear_session(&self, base_url: &str);

    /// Extract the base URL from a full URL (for session keying).
    fn base_url_from(&self, url: &str) -> String;
}
