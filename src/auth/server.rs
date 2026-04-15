//! BRC-31 server-side authentication.
//!
//! Provides handshake handling, session management, and request verification
//! for BRC-31 Authrite on the server side. Reuses `serialization.rs` for
//! BRC-104 binary format and `wallet.rs` for signature verification.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::serialization::{
    filter_response_signable_headers, filter_signable_headers, serialize_request,
    serialize_response,
};
use crate::error::DmResult;

use bsv::wallet::substrates::HttpWalletJson;
use bsv::wallet::{Counterparty, CreateSignatureArgs, Protocol, SecurityLevel};

/// BRC-31 protocol version.
pub const AUTH_VERSION: &str = "0.1";

/// Session TTL in seconds (1 hour).
const SESSION_TTL_SECS: u64 = 3600;

/// A server-side BRC-31 session, created during handshake.
#[derive(Debug, Clone)]
pub struct Brc31ServerSession {
    pub client_identity_key: String,
    pub client_nonce_b64: String,
    pub server_nonce_b64: String,
    pub created_at: Instant,
}

impl Brc31ServerSession {
    /// Check if this session has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > SESSION_TTL_SECS
    }
}

/// In-memory store for BRC-31 server sessions, keyed by server nonce.
#[derive(Debug)]
pub struct Brc31SessionStore {
    sessions: HashMap<String, Brc31ServerSession>,
}

impl Brc31SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Insert a session. The key is the server nonce (base64).
    pub fn insert(&mut self, session: Brc31ServerSession) {
        self.sessions
            .insert(session.server_nonce_b64.clone(), session);
    }

    /// Look up a session by the server nonce that was issued during handshake.
    pub fn get(&self, server_nonce: &str) -> Option<&Brc31ServerSession> {
        self.sessions.get(server_nonce).filter(|s| !s.is_expired())
    }

    /// Remove expired sessions.
    pub fn cleanup(&mut self) {
        self.sessions.retain(|_, s| !s.is_expired());
    }

    /// Number of active (non-expired) sessions.
    pub fn len(&self) -> usize {
        self.sessions.values().filter(|s| !s.is_expired()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Brc31SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

/// JSON body for the initial handshake request (`POST /.well-known/auth`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeRequest {
    pub version: String,
    pub message_type: String,
    pub identity_key: String,
    pub initial_nonce: String,
}

/// JSON body for the handshake response.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResponse {
    pub version: String,
    pub message_type: String,
    pub identity_key: String,
    pub initial_nonce: String,
    pub your_nonce: String,
    /// Signature over `client_nonce || server_nonce` (JSON array of byte values).
    /// Required by SDK's `Peer.processInitialResponse()`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

/// Auth parameters extracted from `x-bsv-auth-*` request headers.
#[derive(Debug, Clone)]
pub struct Brc31AuthParams {
    pub version: String,
    pub identity_key: String,
    pub nonce: String,
    pub your_nonce: String,
    pub signature: String,
    pub request_id: String,
    pub message_type: String,
}

/// Context extracted from a verified BRC-31 request, needed for response signing.
#[derive(Debug, Clone)]
pub struct Brc31AuthContext {
    /// Client's identity public key (hex).
    pub client_identity_key: String,
    /// Client's session nonce from handshake (base64).
    pub client_session_nonce: String,
    /// Server's session nonce (base64).
    pub server_nonce: String,
    /// Request ID from the client (base64), echoed in response.
    pub request_id_b64: String,
}

/// Handle a BRC-31 handshake request.
///
/// Generates a 32-byte server nonce, creates a session, and returns
/// the response + session (caller stores the session).
pub fn handle_handshake(
    req: &HandshakeRequest,
    server_identity_key: &str,
) -> (HandshakeResponse, Brc31ServerSession) {
    // Generate 32-byte server nonce
    let mut nonce_bytes = [0u8; 32];
    rand::Fill::fill(&mut nonce_bytes, &mut rand::rng());
    let server_nonce_b64 = base64_encode(&nonce_bytes);

    let session = Brc31ServerSession {
        client_identity_key: req.identity_key.clone(),
        client_nonce_b64: req.initial_nonce.clone(),
        server_nonce_b64: server_nonce_b64.clone(),
        created_at: Instant::now(),
    };

    let response = HandshakeResponse {
        version: AUTH_VERSION.into(),
        message_type: "initialResponse".into(),
        identity_key: server_identity_key.into(),
        initial_nonce: server_nonce_b64,
        your_nonce: req.initial_nonce.clone(),
        signature: None,
    };

    (response, session)
}

/// Extract BRC-31 auth parameters from HTTP headers.
///
/// Returns `None` if any required header is missing.
pub fn extract_auth_params(headers: &[(String, String)]) -> Option<Brc31AuthParams> {
    let find = |name: &str| -> Option<String> {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };

    Some(Brc31AuthParams {
        version: find("x-bsv-auth-version")?,
        identity_key: find("x-bsv-auth-identity-key")?,
        nonce: find("x-bsv-auth-nonce")?,
        your_nonce: find("x-bsv-auth-your-nonce")?,
        signature: find("x-bsv-auth-signature")?,
        request_id: find("x-bsv-auth-request-id")?,
        // SDK doesn't send x-bsv-auth-message-type on requests — default to "general"
        message_type: find("x-bsv-auth-message-type").unwrap_or_else(|| "general".into()),
    })
}

/// Verify a BRC-31 authenticated request.
///
/// Steps:
/// 1. Decode request_id from base64 → 32 bytes
/// 2. Filter signable headers from the full header set
/// 3. Serialize request into BRC-104 binary format
/// 4. Verify signature via wallet
#[allow(clippy::too_many_arguments)]
pub async fn verify_request(
    wallet: &dyn crate::wallet::WalletBackend,
    params: &Brc31AuthParams,
    session: &Brc31ServerSession,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> bool {
    // 1. Decode request_id from base64
    let request_id_bytes = match base64_decode(&params.request_id) {
        Some(b) if b.len() == 32 => b,
        _ => {
            tracing::warn!("BRC-31: invalid request_id (not 32 bytes)");
            return false;
        }
    };
    let mut request_id = [0u8; 32];
    request_id.copy_from_slice(&request_id_bytes);

    // 2. Filter signable headers
    let signable = filter_signable_headers(headers);

    // 3. Serialize the request using BRC-104
    let serialized = serialize_request(&request_id, method, Some(path), query, &signable, body);

    // 4. Build key derivation params (matches client-side AuthFetch)
    let key_id = format!("{} {}", params.nonce, session.server_nonce_b64);
    let counterparty = &params.identity_key;

    // 5. Decode signature from hex
    let sig_bytes = match hex::decode(&params.signature) {
        Ok(b) => b,
        Err(_) => {
            tracing::warn!("BRC-31: invalid signature hex");
            return false;
        }
    };

    // 6. Build protocol_id in the format our WalletClient expects
    let protocol_id = serde_json::json!([2, "auth message signature"]);

    // 7. Verify via wallet using our WalletClient (HttpWalletJson format causes 400)
    match wallet
        .verify_signature(&serialized, &sig_bytes, &protocol_id, &key_id, counterparty)
        .await
    {
        Ok(valid) => valid,
        Err(e) => {
            tracing::warn!("BRC-31: wallet verify_signature error: {e}");
            false
        }
    }
}

/// Base64-encode bytes (standard, with padding).
pub(crate) fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Base64-decode a string (standard, with padding). Returns None on error.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

/// Sign the handshake response (server proving its identity to the client).
///
/// The SDK's `Peer.processInitialResponse()` verifies this signature over
/// `base64_decode(client_nonce) || base64_decode(server_nonce)` (64 bytes).
pub async fn sign_handshake_response(
    wallet: &HttpWalletJson,
    client_nonce_b64: &str,
    server_nonce_b64: &str,
    client_identity_key: &str,
) -> DmResult<Vec<u8>> {
    let client_nonce = base64_decode(client_nonce_b64)
        .ok_or_else(|| crate::error::DmError::auth("Invalid client nonce base64"))?;
    let server_nonce = base64_decode(server_nonce_b64)
        .ok_or_else(|| crate::error::DmError::auth("Invalid server nonce base64"))?;

    // data = client_nonce_bytes || server_nonce_bytes
    let mut data = Vec::with_capacity(client_nonce.len() + server_nonce.len());
    data.extend_from_slice(&client_nonce);
    data.extend_from_slice(&server_nonce);

    let key_id = format!("{client_nonce_b64} {server_nonce_b64}");
    let counterparty = Counterparty::from_hex(client_identity_key)
        .map_err(|e| crate::error::DmError::auth(format!("Invalid client identity key: {e}")))?;

    let result = wallet
        .create_signature(
            CreateSignatureArgs {
                data: Some(data),
                hash_to_directly_sign: None,
                protocol_id: Protocol::new(SecurityLevel::Counterparty, "auth message signature"),
                key_id,
                counterparty: Some(counterparty),
            },
            "dolphin-milk",
        )
        .await
        .map_err(|e| crate::error::DmError::auth(format!("Wallet sign failed: {e}")))?;

    Ok(result.signature)
}

/// Sign an HTTP response for BRC-31 mutual authentication.
///
/// Returns `(response_nonce_b64, signature_hex)` to be placed in auth headers.
pub async fn sign_response_payload(
    wallet: &HttpWalletJson,
    auth_ctx: &Brc31AuthContext,
    status_code: u16,
    response_headers: &[(String, String)],
    body: &[u8],
) -> DmResult<(String, String)> {
    // Generate 32-byte random response nonce
    let mut nonce_bytes = [0u8; 32];
    rand::Fill::fill(&mut nonce_bytes, &mut rand::rng());
    let nonce_b64 = base64_encode(&nonce_bytes);

    // Decode request_id from auth context
    let request_id_bytes = base64_decode(&auth_ctx.request_id_b64)
        .ok_or_else(|| crate::error::DmError::auth("Invalid request_id base64"))?;
    let mut request_id = [0u8; 32];
    if request_id_bytes.len() != 32 {
        return Err(crate::error::DmError::auth("request_id not 32 bytes"));
    }
    request_id.copy_from_slice(&request_id_bytes);

    // Filter response headers (no content-type for responses)
    let signable = filter_response_signable_headers(response_headers);

    // Serialize the response into BRC-104 binary format
    let serialized = serialize_response(&request_id, status_code, &signable, Some(body));

    // Sign with key derivation matching SDK's SimplifiedFetchTransport
    let key_id = format!("{nonce_b64} {}", auth_ctx.client_session_nonce);
    let counterparty = Counterparty::from_hex(&auth_ctx.client_identity_key)
        .map_err(|e| crate::error::DmError::auth(format!("Invalid client identity key: {e}")))?;

    let result = wallet
        .create_signature(
            CreateSignatureArgs {
                data: Some(serialized),
                hash_to_directly_sign: None,
                protocol_id: Protocol::new(SecurityLevel::Counterparty, "auth message signature"),
                key_id,
                counterparty: Some(counterparty),
            },
            "dolphin-milk",
        )
        .await
        .map_err(|e| crate::error::DmError::auth(format!("Wallet sign failed: {e}")))?;

    Ok((nonce_b64, hex::encode(result.signature)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Session store --

    #[test]
    fn test_session_store_new_is_empty() {
        let store = Brc31SessionStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_session_store_insert_and_get() {
        let mut store = Brc31SessionStore::new();
        let session = Brc31ServerSession {
            client_identity_key: "02abc".into(),
            client_nonce_b64: "Y2xpZW50".into(),
            server_nonce_b64: "c2VydmVy".into(),
            created_at: Instant::now(),
        };
        store.insert(session);
        assert_eq!(store.len(), 1);

        let found = store.get("c2VydmVy");
        assert!(found.is_some());
        assert_eq!(found.unwrap().client_identity_key, "02abc");
    }

    #[test]
    fn test_session_store_get_missing() {
        let store = Brc31SessionStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_session_store_expired_returns_none() {
        let mut store = Brc31SessionStore::new();
        let session = Brc31ServerSession {
            client_identity_key: "02abc".into(),
            client_nonce_b64: "Y2xpZW50".into(),
            server_nonce_b64: "c2VydmVy".into(),
            created_at: Instant::now() - std::time::Duration::from_secs(SESSION_TTL_SECS + 1),
        };
        store.insert(session);
        // Session is expired, get() should return None
        assert!(store.get("c2VydmVy").is_none());
        assert!(store.is_empty()); // len() also filters expired
    }

    #[test]
    fn test_session_store_cleanup() {
        let mut store = Brc31SessionStore::new();

        // Add an expired session
        store.insert(Brc31ServerSession {
            client_identity_key: "old".into(),
            client_nonce_b64: "old".into(),
            server_nonce_b64: "old_nonce".into(),
            created_at: Instant::now() - std::time::Duration::from_secs(SESSION_TTL_SECS + 1),
        });

        // Add a fresh session
        store.insert(Brc31ServerSession {
            client_identity_key: "fresh".into(),
            client_nonce_b64: "fresh".into(),
            server_nonce_b64: "fresh_nonce".into(),
            created_at: Instant::now(),
        });

        assert_eq!(store.sessions.len(), 2); // Raw count includes expired
        store.cleanup();
        assert_eq!(store.sessions.len(), 1); // Only fresh remains
        assert!(store.get("fresh_nonce").is_some());
    }

    // -- Handshake --

    #[test]
    fn test_handle_handshake_returns_valid_response() {
        let req = HandshakeRequest {
            version: "0.1".into(),
            message_type: "initialRequest".into(),
            identity_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .into(),
            initial_nonce: "Y2xpZW50bm9uY2U=".into(),
        };

        let (resp, session) = handle_handshake(&req, "02server_key");

        assert_eq!(resp.version, "0.1");
        assert_eq!(resp.message_type, "initialResponse");
        assert_eq!(resp.identity_key, "02server_key");
        assert_eq!(resp.your_nonce, "Y2xpZW50bm9uY2U=");
        // Server nonce should be base64 of 32 bytes (~44 chars)
        assert!(!resp.initial_nonce.is_empty());
        assert!(resp.initial_nonce.len() >= 40);

        assert_eq!(session.client_identity_key, req.identity_key);
        assert_eq!(session.client_nonce_b64, req.initial_nonce);
        assert_eq!(session.server_nonce_b64, resp.initial_nonce);
        assert!(!session.is_expired());
    }

    #[test]
    fn test_handshake_nonces_are_unique() {
        let req = HandshakeRequest {
            version: "0.1".into(),
            message_type: "initialRequest".into(),
            identity_key: "02abc".into(),
            initial_nonce: "Y2xpZW50".into(),
        };

        let (resp1, _) = handle_handshake(&req, "02server");
        let (resp2, _) = handle_handshake(&req, "02server");
        // Two handshakes should produce different server nonces
        assert_ne!(resp1.initial_nonce, resp2.initial_nonce);
    }

    // -- Auth param extraction --

    #[test]
    fn test_extract_auth_params_all_present() {
        let headers = vec![
            ("x-bsv-auth-version".into(), "0.1".into()),
            ("x-bsv-auth-identity-key".into(), "02abc".into()),
            ("x-bsv-auth-nonce".into(), "msg_nonce".into()),
            ("x-bsv-auth-your-nonce".into(), "srv_nonce".into()),
            ("x-bsv-auth-signature".into(), "deadbeef".into()),
            ("x-bsv-auth-request-id".into(), "req_id_b64".into()),
            ("x-bsv-auth-message-type".into(), "general".into()),
        ];

        let params = extract_auth_params(&headers).unwrap();
        assert_eq!(params.version, "0.1");
        assert_eq!(params.identity_key, "02abc");
        assert_eq!(params.nonce, "msg_nonce");
        assert_eq!(params.your_nonce, "srv_nonce");
        assert_eq!(params.signature, "deadbeef");
        assert_eq!(params.request_id, "req_id_b64");
        assert_eq!(params.message_type, "general");
    }

    #[test]
    fn test_extract_auth_params_missing_header() {
        // Missing x-bsv-auth-signature
        let headers = vec![
            ("x-bsv-auth-version".into(), "0.1".into()),
            ("x-bsv-auth-identity-key".into(), "02abc".into()),
            ("x-bsv-auth-nonce".into(), "n".into()),
            ("x-bsv-auth-your-nonce".into(), "yn".into()),
            // signature missing
            ("x-bsv-auth-request-id".into(), "rid".into()),
            ("x-bsv-auth-message-type".into(), "general".into()),
        ];
        assert!(extract_auth_params(&headers).is_none());
    }

    #[test]
    fn test_extract_auth_params_case_insensitive() {
        let headers = vec![
            ("X-BSV-Auth-Version".into(), "0.1".into()),
            ("X-BSV-AUTH-IDENTITY-KEY".into(), "02abc".into()),
            ("x-bsv-auth-nonce".into(), "n".into()),
            ("X-Bsv-Auth-Your-Nonce".into(), "yn".into()),
            ("x-bsv-auth-signature".into(), "sig".into()),
            ("x-bsv-auth-request-id".into(), "rid".into()),
            ("x-bsv-auth-message-type".into(), "general".into()),
        ];
        let params = extract_auth_params(&headers).unwrap();
        assert_eq!(params.identity_key, "02abc");
    }

    // -- Handshake serde --

    #[test]
    fn test_handshake_request_serde() {
        let json = r#"{"version":"0.1","messageType":"initialRequest","identityKey":"02abc","initialNonce":"dGVzdA=="}"#;
        let req: HandshakeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.version, "0.1");
        assert_eq!(req.message_type, "initialRequest");
        assert_eq!(req.identity_key, "02abc");
        assert_eq!(req.initial_nonce, "dGVzdA==");
    }

    #[test]
    fn test_handshake_response_serde() {
        let resp = HandshakeResponse {
            version: "0.1".into(),
            message_type: "initialResponse".into(),
            identity_key: "02server".into(),
            initial_nonce: "c2VydmVy".into(),
            your_nonce: "Y2xpZW50".into(),
            signature: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("messageType"));
        assert!(json.contains("initialResponse"));
        assert!(json.contains("identityKey"));
        assert!(json.contains("initialNonce"));
        assert!(json.contains("yourNonce"));
    }

    // -- Session expiry --

    #[test]
    fn test_session_not_expired_when_fresh() {
        let session = Brc31ServerSession {
            client_identity_key: "02abc".into(),
            client_nonce_b64: "cn".into(),
            server_nonce_b64: "sn".into(),
            created_at: Instant::now(),
        };
        assert!(!session.is_expired());
    }

    #[test]
    fn test_session_expired_after_ttl() {
        let session = Brc31ServerSession {
            client_identity_key: "02abc".into(),
            client_nonce_b64: "cn".into(),
            server_nonce_b64: "sn".into(),
            created_at: Instant::now() - std::time::Duration::from_secs(SESSION_TTL_SECS + 1),
        };
        assert!(session.is_expired());
    }

    // -- Base64 helpers --

    #[test]
    fn test_base64_roundtrip() {
        let data = [0u8; 32];
        let encoded = base64_encode(&data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_base64_decode_invalid() {
        assert!(base64_decode("not valid base64!!!").is_none());
    }

    // -- HandshakeResponse signature field --

    #[test]
    fn test_handshake_response_signature_field_serde() {
        let resp = HandshakeResponse {
            version: "0.1".into(),
            message_type: "initialResponse".into(),
            identity_key: "02server".into(),
            initial_nonce: "c2VydmVy".into(),
            your_nonce: "Y2xpZW50".into(),
            signature: Some(vec![1, 2, 3, 4, 255]),
        };
        let json = serde_json::to_string(&resp).unwrap();
        // Should serialize as a JSON array of numbers
        assert!(json.contains("\"signature\":[1,2,3,4,255]"));
        // Round-trip
        let parsed: HandshakeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.signature.unwrap(), vec![1, 2, 3, 4, 255]);
    }

    #[test]
    fn test_handshake_response_none_signature_omitted() {
        let resp = HandshakeResponse {
            version: "0.1".into(),
            message_type: "initialResponse".into(),
            identity_key: "02server".into(),
            initial_nonce: "c2VydmVy".into(),
            your_nonce: "Y2xpZW50".into(),
            signature: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        // None should not appear in JSON
        assert!(!json.contains("signature"));
    }

    // -- Brc31AuthContext --

    #[test]
    fn test_brc31_auth_context_basic() {
        let ctx = Brc31AuthContext {
            client_identity_key:
                "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".into(),
            client_session_nonce: "Y2xpZW50".into(),
            server_nonce: "c2VydmVy".into(),
            request_id_b64: "cmVxdWVzdA==".into(),
        };
        assert_eq!(ctx.client_identity_key.len(), 66);
        assert!(!ctx.client_session_nonce.is_empty());
        assert!(!ctx.server_nonce.is_empty());
        assert!(!ctx.request_id_b64.is_empty());
    }
}
