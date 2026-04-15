//! x402 payment flow — re-exports from the bsv-x402-server crate.
//!
//! All x402 types, functions, and modules are defined in the standalone
//! `bsv-x402-server` crate and re-exported here for backward compatibility.

pub use bsv_x402_server::cache;
pub use bsv_x402_server::circuit_breaker;
pub use bsv_x402_server::discovery;
pub use bsv_x402_server::error;
pub use bsv_x402_server::payment;
pub use bsv_x402_server::rate_limit;
pub use bsv_x402_server::refund;
pub use bsv_x402_server::registry;
pub use bsv_x402_server::schema;
pub use bsv_x402_server::traits;

// Re-export the error type for convenience
pub use bsv_x402_server::error::X402Error;

// ---------------------------------------------------------------------------
// Trait implementations — bridge worm types to x402 traits
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use bsv_x402_server::traits::{AuthClient, CreateActionResult, WalletApi};
use serde_json::Value;

use crate::auth::session::{base_url_from, clear_session_from};
use crate::auth::AuthriteClient;
use crate::wallet::WalletBackend;

// ---------------------------------------------------------------------------
// WalletBackendAdapter — bridges Arc<dyn WalletBackend> to the x402 WalletApi trait.
//
// This lets any WalletBackend implementation (HTTP or embedded) be used for
// x402 payments without the x402 crate needing to know about WalletBackend.
// ---------------------------------------------------------------------------

/// Adapter that implements `WalletApi` by delegating to `Arc<dyn WalletBackend>`.
pub struct WalletBackendAdapter(pub std::sync::Arc<dyn WalletBackend + Send + Sync>);

#[async_trait]
impl WalletApi for WalletBackendAdapter {
    async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, X402Error> {
        self.0
            .get_public_key(protocol_id, key_id, counterparty, for_self)
            .await
            .map_err(|e| X402Error::payment(e.to_string()))
    }

    async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        random_outputs: bool,
    ) -> Result<CreateActionResult, X402Error> {
        let result = self
            .0
            .create_action(
                outputs,
                description,
                accept_delayed_broadcast,
                random_outputs,
            )
            .await
            .map_err(|e| X402Error::payment(e.to_string()))?;

        Ok(CreateActionResult {
            tx: result.tx,
            txid: result.txid,
        })
    }

    async fn internalize_action(
        &self,
        tx: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, X402Error> {
        self.0
            .internalize_action(tx, outputs, description)
            .await
            .map_err(|e| X402Error::payment(e.to_string()))
    }
}

#[async_trait]
impl AuthClient for AuthriteClient {
    fn wallet(&self) -> &dyn WalletApi {
        AuthriteClient::wallet_api(self)
    }

    async fn authenticated_request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::Response, X402Error> {
        AuthriteClient::authenticated_request(self, method, url, headers, body)
            .await
            .map_err(|e| X402Error::payment(e.to_string()))
    }

    fn session_dir(&self) -> &std::path::Path {
        &self.session_dir
    }

    fn clear_session(&self, base_url: &str) {
        clear_session_from(base_url, &self.session_dir);
    }

    fn base_url_from(&self, url: &str) -> String {
        base_url_from(url)
    }
}

// ---------------------------------------------------------------------------
// Convenience re-exports for backward compat with `DmError`
// ---------------------------------------------------------------------------

/// Bridge: convert X402Error to DmError for call sites that need DmError.
impl From<X402Error> for crate::error::DmError {
    fn from(e: X402Error) -> Self {
        crate::error::DmError::payment(e.to_string())
    }
}

/// Convert the worm's RateLimitConfig to the x402 crate's RateLimitConfig.
impl From<&crate::config::RateLimitConfig> for bsv_x402_server::rate_limit::RateLimitConfig {
    fn from(cfg: &crate::config::RateLimitConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            default_rpm: cfg.default_rpm,
            per_service: cfg
                .per_service
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        bsv_x402_server::rate_limit::ServiceRateLimit {
                            requests_per_minute: v.requests_per_minute,
                        },
                    )
                })
                .collect(),
        }
    }
}
