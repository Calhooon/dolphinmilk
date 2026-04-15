//! BRC-31 Authrite client — handshake + authenticated requests.
//!
//! Orchestrates the full BRC-31 flow:
//! 1. Handshake with server (once per session, 1-hour TTL)
//! 2. Serialize request payload in BRC-104 binary format
//! 3. Sign with wallet via BRC-42 key derivation
//! 4. Attach auth headers and send

use std::error::Error as StdError;
use std::path::PathBuf;

use axum::http::Response as HttpResponse;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use reqwest::Client;
use serde_json::{json, Value};

use crate::error::DmError;
use crate::wallet::WalletBackend;
use crate::x402::WalletBackendAdapter;

use super::serialization::{build_auth_headers, filter_signable_headers, serialize_request};
use super::session;
use super::session::{
    clear_session_from, load_session_from, save_session_to, AuthSession, DEFAULT_TTL,
};

/// BRC-31 auth version.
const AUTH_VERSION: &str = "0.1";

/// Handshake endpoint.
const HANDSHAKE_PATH: &str = "/.well-known/auth";

/// BRC-31 Authrite client.
///
/// Handles handshake, session caching, and authenticated request signing.
pub struct AuthriteClient {
    /// Shared wallet backend — used for signing, identity, and payment operations.
    wallet_adapter: WalletBackendAdapter,
    http: Client,
    pub session_dir: PathBuf,
}

impl AuthriteClient {
    /// Build a reqwest Client that disables connection pooling.
    /// Cloudflare Workers may drop keep-alive connections between requests,
    /// causing subsequent requests on pooled connections to hang until timeout.
    fn build_http_client() -> Client {
        Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Create a new AuthriteClient from a shared wallet backend.
    ///
    /// `wallet_url` is used to derive a per-wallet session directory (keyed by URL hash)
    /// to prevent session collisions when multiple agents share the same filesystem.
    pub fn new(wallet: std::sync::Arc<dyn WalletBackend + Send + Sync>, wallet_url: &str) -> Self {
        use sha2::{Digest, Sha256};
        let wallet_hash = hex::encode(&Sha256::digest(wallet_url.as_bytes())[..8]);
        let session_dir = std::env::var("HOME")
            .map(|h| {
                PathBuf::from(h)
                    .join(".local/share/brc31-sessions")
                    .join(&wallet_hash)
            })
            .unwrap_or_else(|_| PathBuf::from("/tmp/brc31-sessions").join(&wallet_hash));
        Self {
            wallet_adapter: WalletBackendAdapter(wallet),
            http: Self::build_http_client(),
            session_dir,
        }
    }

    /// Create with a custom session directory (for testing).
    pub fn with_session_dir(
        wallet: std::sync::Arc<dyn WalletBackend + Send + Sync>,
        session_dir: PathBuf,
    ) -> Self {
        Self {
            wallet_adapter: WalletBackendAdapter(wallet),
            http: Self::build_http_client(),
            session_dir,
        }
    }

    /// Access the underlying wallet backend.
    pub fn wallet(&self) -> &dyn WalletBackend {
        &*self.wallet_adapter.0
    }

    /// Access the wallet as a `WalletApi` reference (for x402 payment flow).
    pub fn wallet_api(&self) -> &WalletBackendAdapter {
        &self.wallet_adapter
    }

    /// Get or create a session for the given server URL.
    pub async fn get_or_create_session(&self, server_url: &str) -> Result<AuthSession, DmError> {
        let server_url = server_url.trim_end_matches('/');

        // Try loading cached session
        if let Some(session) = load_session_from(server_url, DEFAULT_TTL, &self.session_dir) {
            tracing::debug!("Reusing existing session for {server_url}");
            return Ok(session);
        }

        // Perform handshake
        self.do_handshake(server_url).await
    }

    /// Perform a BRC-31 handshake with the server.
    pub async fn do_handshake(&self, server_url: &str) -> Result<AuthSession, DmError> {
        let server_url = server_url.trim_end_matches('/');

        // 1. Generate 32-byte client nonce
        let mut client_nonce_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut client_nonce_bytes);
        let client_nonce_b64 = BASE64.encode(client_nonce_bytes);

        // 2. Get our identity key
        let identity_key = self.wallet().get_identity_key().await?;

        // 3. Build and send initialRequest
        let body = json!({
            "version": AUTH_VERSION,
            "messageType": "initialRequest",
            "identityKey": identity_key,
            "initialNonce": client_nonce_b64,
        });

        let url = format!("{server_url}{HANDSHAKE_PATH}");
        tracing::debug!("BRC-31 handshake: POST {url}");

        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Origin", "http://localhost")
            .timeout(std::time::Duration::from_secs(30))
            .json(&body)
            .send()
            .await
            .map_err(|e| DmError::auth(format!("Cannot connect to {url}: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(DmError::auth(format!(
                "Handshake failed: HTTP {status}. Body: {}",
                &body_text[..body_text.len().min(500)]
            )));
        }

        // 4. Parse initialResponse
        let data: Value = resp
            .json()
            .await
            .map_err(|e| DmError::auth(format!("Invalid JSON in handshake response: {e}")))?;

        let server_identity_key = data
            .get("identityKey")
            .or_else(|| data.get("identity_key"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| DmError::auth("Server initialResponse missing identityKey"))?
            .to_string();

        let server_nonce_b64 = data
            .get("initialNonce")
            .or_else(|| data.get("initial_nonce"))
            .or_else(|| data.get("nonce"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| DmError::auth("Server initialResponse missing initialNonce"))?
            .to_string();

        // Validate yourNonce (anti-replay)
        if let Some(your_nonce) = data
            .get("yourNonce")
            .or_else(|| data.get("your_nonce"))
            .and_then(|v| v.as_str())
        {
            if your_nonce != client_nonce_b64 {
                return Err(DmError::auth(format!(
                    "Server yourNonce mismatch: expected {client_nonce_b64}, got {your_nonce}"
                )));
            }
        }

        if server_nonce_b64.len() < 4 {
            return Err(DmError::auth(format!(
                "Server nonce suspiciously short: {server_nonce_b64}"
            )));
        }

        // 5. Build, persist, and return session
        let session = AuthSession::new(
            server_url.to_string(),
            server_identity_key,
            server_nonce_b64,
            client_nonce_b64,
        );

        if let Err(e) = save_session_to(&session, &self.session_dir) {
            tracing::warn!("Failed to persist BRC-31 session: {e}");
        }

        tracing::info!("BRC-31 session established with {server_url}");
        Ok(session)
    }

    /// Make an authenticated BRC-31 request.
    ///
    /// This is the main entry point for all authenticated HTTP calls.
    /// Handles session management, payload serialization, signing, and sending.
    /// On 401 "Session not found", clears the stale session and retries once.
    pub async fn authenticated_request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::Response, DmError> {
        let response = self.send_authenticated(method, url, headers, body).await?;

        // Retry once on stale session (401 "Session not found")
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Read the body to check for stale session indicator
            let body_text = response.text().await.unwrap_or_default();
            if body_text.contains("Session not found") {
                let server_base = session::base_url_from(url);
                tracing::warn!("Stale BRC-31 session for {server_base} — clearing and retrying");
                clear_session_from(&server_base, &self.session_dir);
                return self.send_authenticated(method, url, headers, body).await;
            }
            // Not a stale session — reconstruct the response for the caller.
            // We consumed the body, so build an http::Response manually.
            let resp = reqwest::Response::from(
                HttpResponse::builder()
                    .status(reqwest::StatusCode::UNAUTHORIZED)
                    .body(body_text)
                    .unwrap(),
            );
            return Ok(resp);
        }

        Ok(response)
    }

    /// Internal: build and send a single authenticated request.
    async fn send_authenticated(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::Response, DmError> {
        // 1. Parse URL
        let parsed =
            url::Url::parse(url).map_err(|e| DmError::auth(format!("Invalid URL: {e}")))?;
        let server_base = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
        let server_base = if let Some(port) = parsed.port() {
            format!("{server_base}:{port}")
        } else {
            server_base
        };
        let path = parsed.path();
        let query = if parsed.query().is_some_and(|q| !q.is_empty()) {
            Some(format!("?{}", parsed.query().unwrap()))
        } else {
            None
        };

        // 2. Get or create session
        let session = self.get_or_create_session(&server_base).await?;

        // 3. Normalize JSON body (compact form so signed bytes match server)
        let body_bytes = match body {
            Some(b) => {
                // Try to normalize as JSON
                if let Ok(parsed_json) = serde_json::from_slice::<Value>(b) {
                    Some(serde_json::to_vec(&parsed_json).unwrap_or_else(|_| b.to_vec()))
                } else {
                    Some(b.to_vec())
                }
            }
            None => None,
        };

        // 4. Ensure content-type if body present
        let mut all_headers: Vec<(String, String)> = headers.to_vec();
        if body_bytes.is_some()
            && !all_headers
                .iter()
                .any(|(k, _)| k.to_lowercase() == "content-type")
        {
            all_headers.push(("content-type".into(), "application/json".into()));
        }

        // 5. Generate per-message nonce and request ID
        let mut msg_nonce_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut msg_nonce_bytes);
        let msg_nonce_b64 = BASE64.encode(msg_nonce_bytes);

        let mut request_id_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut request_id_bytes);
        let request_id_b64 = BASE64.encode(request_id_bytes);

        // 6. Filter and sort signable headers
        let signable_headers = filter_signable_headers(&all_headers);

        // 7. Serialize request for signing
        let serialized = serialize_request(
            &request_id_bytes,
            &method.to_uppercase(),
            Some(path),
            query.as_deref(),
            &signable_headers,
            body_bytes.as_deref(),
        );

        // 8. Sign: key_id = "{msg_nonce_b64} {server_nonce_b64}"
        let key_id = format!("{} {}", msg_nonce_b64, session.server_nonce_b64);
        let protocol_id = json!([2, "auth message signature"]);

        let signature = self
            .wallet()
            .create_signature(
                &serialized,
                &protocol_id,
                &key_id,
                &session.server_identity_key,
            )
            .await?;
        let signature_hex = hex::encode(&signature);

        // 9. Get our identity key
        let identity_key = self.wallet().get_identity_key().await?;

        // 10. Build auth headers
        let auth_headers = build_auth_headers(
            &identity_key,
            &msg_nonce_b64,
            &session.server_nonce_b64,
            &signature_hex,
            &request_id_b64,
        );

        // 11. Build and send request
        let mut builder = match method.to_uppercase().as_str() {
            "GET" => self.http.get(url),
            "POST" => self.http.post(url),
            "PUT" => self.http.put(url),
            "DELETE" => self.http.delete(url),
            "PATCH" => self.http.patch(url),
            other => {
                return Err(DmError::auth(format!("Unsupported HTTP method: {other}")));
            }
        };

        // Add original headers
        for (key, value) in &all_headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        // Add auth headers (override any conflicts)
        for (key, value) in &auth_headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        // Add body
        if let Some(ref b) = body_bytes {
            builder = builder.body(b.clone());
        }

        builder = builder.timeout(std::time::Duration::from_secs(300));

        let response = builder
            .send()
            .await
            .map_err(|e| {
                let is_timeout = e.is_timeout();
                let is_connect = e.is_connect();
                let source = StdError::source(&e).map(|s| s.to_string()).unwrap_or_default();
                DmError::auth(format!(
                    "HTTP request failed: {e} [timeout={is_timeout}, connect={is_connect}, source={source}]"
                ))
            })?;

        Ok(response)
    }

    /// Convenience: authenticated POST with JSON body.
    pub async fn post_json(&self, url: &str, body: &Value) -> Result<reqwest::Response, DmError> {
        let body_bytes = serde_json::to_vec(body)
            .map_err(|e| DmError::auth(format!("Failed to serialize body: {e}")))?;
        let headers = vec![("content-type".into(), "application/json".into())];
        self.authenticated_request("POST", url, &headers, Some(&body_bytes))
            .await
    }

    /// Convenience: authenticated GET.
    pub async fn get(&self, url: &str) -> Result<reqwest::Response, DmError> {
        self.authenticated_request("GET", url, &[], None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::HttpWalletClient;

    #[test]
    fn test_authrite_client_creation() {
        let wallet: std::sync::Arc<dyn WalletBackend + Send + Sync> = std::sync::Arc::new(
            HttpWalletClient::new("http://localhost:3322", "http://localhost", 30),
        );
        let client = AuthriteClient::new(wallet, "http://localhost:3322");
        assert!(client
            .session_dir
            .to_string_lossy()
            .contains("brc31-sessions"));
    }

    #[test]
    fn test_authrite_client_custom_session_dir() {
        let wallet: std::sync::Arc<dyn WalletBackend + Send + Sync> = std::sync::Arc::new(
            HttpWalletClient::new("http://localhost:3322", "http://localhost", 30),
        );
        let dir = PathBuf::from("/tmp/test-auth-sessions");
        let client = AuthriteClient::with_session_dir(wallet, dir.clone());
        assert_eq!(client.session_dir, dir);
    }

    #[tokio::test]
    async fn test_handshake_mock() {
        let mut server = mockito::Server::new_async().await;
        let server_url = server.url();

        let _mock = server
            .mock("POST", "/.well-known/auth")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::to_string(&json!({
                    "version": "0.1",
                    "messageType": "initialResponse",
                    "identityKey": "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1",
                    "initialNonce": "c2VydmVyX25vbmNlX2Jhc2U2NA==",
                }))
                .unwrap(),
            )
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let wallet: std::sync::Arc<dyn WalletBackend + Send + Sync> = std::sync::Arc::new(
            HttpWalletClient::new("http://localhost:3322", "http://localhost", 30),
        );
        let client = AuthriteClient::with_session_dir(wallet, dir.path().to_path_buf());

        let result = client.do_handshake(&server_url).await;

        // If wallet is running, handshake succeeds. If not, it fails with a wallet error.
        match result {
            Ok(session) => {
                assert_eq!(
                    session.server_identity_key,
                    "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1"
                );
                assert_eq!(session.server_nonce_b64, "c2VydmVyX25vbmNlX2Jhc2U2NA==");
                assert!(session.server_url.starts_with("http://"));
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("wallet") || msg.contains("Wallet") || msg.contains("reachable"),
                    "Expected wallet error, got: {msg}"
                );
            }
        }
    }

    #[test]
    fn test_url_parsing() {
        let url =
            url::Url::parse("https://messagebox.babbage.systems/sendMessage?foo=bar").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("messagebox.babbage.systems"));
        assert_eq!(url.path(), "/sendMessage");
        assert_eq!(url.query(), Some("foo=bar"));
        assert!(url.port().is_none());
    }

    #[test]
    fn test_url_parsing_with_port() {
        let url = url::Url::parse("http://localhost:8080/api/v1").unwrap();
        assert_eq!(url.port(), Some(8080));
        let base = format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.port().unwrap()
        );
        assert_eq!(base, "http://localhost:8080");
    }
}
