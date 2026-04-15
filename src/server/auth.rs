//! BRC-31 Authrite helpers for the HTTP server.
//!
//! Contains the handshake endpoint, request verification, and response signing.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, StatusCode};
use axum::Json;
use serde::Serialize;

use crate::auth::server::{
    extract_auth_params, handle_handshake, sign_handshake_response, sign_response_payload,
    verify_request, Brc31AuthContext, HandshakeRequest, HandshakeResponse,
};

use super::AppState;

/// Convert axum HeaderMap to the `[(String, String)]` format used by auth/serialization.
pub(crate) fn headermap_to_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_string(), val.to_string()))
        })
        .collect()
}

/// Check if a request is BRC-31 authenticated.
///
/// If no wallet identity key is available (wallet unreachable at startup),
/// returns `Ok(None)` (auth disabled). Otherwise verifies the `x-bsv-auth-*`
/// headers against a stored BRC-31 session and returns
/// `Ok(Some(Brc31AuthContext))` with the context needed for response signing.
///
/// For "optional" auth routes (most endpoints), missing auth headers return
/// `Ok(None)` — the request proceeds without signing. For "required" auth
/// routes (`/chat`, `/budget`, `/heartbeat/trigger`), missing headers return 401.
pub(crate) async fn check_brc31_auth(
    state: &AppState,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
) -> Result<Option<Brc31AuthContext>, StatusCode> {
    // No identity key = wallet was unreachable at startup, skip auth
    if state.auth.server_identity_key.is_empty() {
        return Ok(None);
    }

    // Extract auth params from headers
    let header_pairs = headermap_to_pairs(headers);
    let params = match extract_auth_params(&header_pairs) {
        Some(p) => p,
        None => {
            tracing::warn!("BRC-31 auth required for {method} {path}: missing auth headers");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // Look up session by your_nonce (the server nonce stored during handshake)
    let sessions = state.auth.brc31_sessions.lock().await;
    let session = match sessions.get(&params.your_nonce) {
        Some(s) => s,
        None => {
            tracing::warn!(
                "BRC-31 auth failed for {method} {path}: no session for your_nonce={}. \
                 Active sessions: {}",
                &params.your_nonce[..params.your_nonce.len().min(12)],
                sessions.len()
            );
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // Verify client identity matches the parent key OR the agent's own identity key.
    // Parent key (auto-set from wallet at startup) is the primary authorized identity.
    // The agent's own key is also accepted so the embedded UI (which authenticates via
    // the agent's wallet) can access auth-gated endpoints even when a separate parent
    // key is configured (e.g. multi-agent test setups with a shared MetaNet Client).
    let is_parent = params.identity_key == state.config.parent.identity_key;
    let is_self = !state.auth.server_identity_key.is_empty()
        && params.identity_key == state.auth.server_identity_key;
    if !is_parent && !is_self {
        tracing::warn!(
            "BRC-31 auth failed for {method} {path}: identity key mismatch. \
             Got {}, expected {} (or self={})",
            &params.identity_key[..params.identity_key.len().min(16)],
            &state.config.parent.identity_key[..16.min(state.config.parent.identity_key.len())],
            &state.auth.server_identity_key[..16.min(state.auth.server_identity_key.len())]
        );
        return Err(StatusCode::FORBIDDEN);
    }

    // Build auth context before dropping session borrow
    let auth_ctx = Brc31AuthContext {
        client_identity_key: params.identity_key.clone(),
        client_session_nonce: session.client_nonce_b64.clone(),
        server_nonce: session.server_nonce_b64.clone(),
        request_id_b64: params.request_id.clone(),
    };

    // Verify the request signature via wallet service.
    // SDK's AuthFetch signs `parsedUrl.search` which includes the `?` prefix
    // (e.g., "?limit=100&offset=0"), so we must match that format.
    let query_with_prefix = query.map(|q| {
        if q.starts_with('?') {
            q.to_string()
        } else {
            format!("?{q}")
        }
    });
    let valid = verify_request(
        state.wallet.as_ref(),
        &params,
        session,
        method,
        path,
        query_with_prefix.as_deref(),
        &header_pairs,
        body,
    )
    .await;

    if valid {
        tracing::debug!("BRC-31 auth OK for {method} {path}");
        Ok(Some(auth_ctx))
    } else {
        tracing::warn!("BRC-31 auth failed for {method} {path}: signature verification failed");
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Build a signed JSON response with BRC-31 mutual auth headers.
///
/// If `auth_ctx` is `Some` and the server has an identity key, this signs the
/// response body and attaches `x-bsv-auth-*` headers so SDK clients can verify.
/// In dev mode (`auth_ctx` is `None`), returns a plain JSON response.
pub(crate) async fn signed_json_response<T: Serialize>(
    state: &AppState,
    auth_ctx: Option<Brc31AuthContext>,
    status: StatusCode,
    body: &T,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;

    let body_bytes = serde_json::to_vec(body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // If no auth context or no server identity key, return plain JSON
    let auth_ctx = match auth_ctx {
        Some(ctx) if !state.auth.server_identity_key.is_empty() => ctx,
        _ => {
            let mut resp = (status, body_bytes).into_response();
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
            return Ok(resp);
        }
    };

    // Collect response headers for signing (initially just content-type)
    let response_headers: Vec<(String, String)> =
        vec![("content-type".into(), "application/json".into())];

    // Sign the response via wallet service
    let (nonce_b64, signature_hex) = sign_response_payload(
        &state.auth.wallet,
        &auth_ctx,
        status.as_u16(),
        &response_headers,
        &body_bytes,
    )
    .await
    .map_err(|e| {
        tracing::warn!("Failed to sign response: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Build the response with auth headers
    let mut resp = (status, body_bytes).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    resp.headers_mut().insert(
        "x-bsv-auth-version".parse::<HeaderName>().unwrap(),
        "0.1".parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-identity-key".parse::<HeaderName>().unwrap(),
        state.auth.server_identity_key.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-nonce".parse::<HeaderName>().unwrap(),
        nonce_b64.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-your-nonce".parse::<HeaderName>().unwrap(),
        auth_ctx.client_session_nonce.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-signature".parse::<HeaderName>().unwrap(),
        signature_hex.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-request-id".parse::<HeaderName>().unwrap(),
        auth_ctx.request_id_b64.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-message-type".parse::<HeaderName>().unwrap(),
        "general".parse().unwrap(),
    );

    Ok(resp)
}

/// Build a signed raw (non-JSON) response with BRC-31 mutual auth headers.
///
/// Like `signed_json_response`, but accepts pre-built body bytes and a custom
/// content type. Used for file downloads (CSV, JSON exports) that need auth
/// but aren't standard JSON API responses.
pub(crate) async fn signed_raw_response(
    state: &AppState,
    auth_ctx: Option<Brc31AuthContext>,
    status: StatusCode,
    content_type: &str,
    body_bytes: Vec<u8>,
    extra_headers: Vec<(String, String)>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;

    // If no auth context or no server identity key, return plain response
    let auth_ctx = match auth_ctx {
        Some(ctx) if !state.auth.server_identity_key.is_empty() => ctx,
        _ => {
            let mut resp = (status, body_bytes).into_response();
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
            for (k, v) in &extra_headers {
                if let (Ok(name), Ok(val)) = (k.parse::<HeaderName>(), v.parse()) {
                    resp.headers_mut().insert(name, val);
                }
            }
            return Ok(resp);
        }
    };

    // Collect response headers for signing
    let mut response_headers: Vec<(String, String)> =
        vec![("content-type".into(), content_type.into())];
    for (k, v) in &extra_headers {
        response_headers.push((k.clone(), v.clone()));
    }

    // Sign the response via wallet service
    let (nonce_b64, signature_hex) = sign_response_payload(
        &state.auth.wallet,
        &auth_ctx,
        status.as_u16(),
        &response_headers,
        &body_bytes,
    )
    .await
    .map_err(|e| {
        tracing::warn!("Failed to sign response: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Build the response with auth headers
    let mut resp = (status, body_bytes).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    for (k, v) in &extra_headers {
        if let (Ok(name), Ok(val)) = (k.parse::<HeaderName>(), v.parse()) {
            resp.headers_mut().insert(name, val);
        }
    }
    resp.headers_mut().insert(
        "x-bsv-auth-version".parse::<HeaderName>().unwrap(),
        "0.1".parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-identity-key".parse::<HeaderName>().unwrap(),
        state.auth.server_identity_key.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-nonce".parse::<HeaderName>().unwrap(),
        nonce_b64.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-your-nonce".parse::<HeaderName>().unwrap(),
        auth_ctx.client_session_nonce.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-signature".parse::<HeaderName>().unwrap(),
        signature_hex.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-request-id".parse::<HeaderName>().unwrap(),
        auth_ctx.request_id_b64.parse().unwrap(),
    );
    resp.headers_mut().insert(
        "x-bsv-auth-message-type".parse::<HeaderName>().unwrap(),
        "general".parse().unwrap(),
    );

    Ok(resp)
}

/// POST /.well-known/auth -- BRC-31 handshake endpoint.
pub(crate) async fn brc31_handshake(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HandshakeRequest>,
) -> Result<Json<HandshakeResponse>, StatusCode> {
    // Reject handshake if a parent identity key is configured and the client
    // doesn't match the parent OR the agent's own identity. The agent's own key
    // is accepted so the embedded UI (which authenticates via the agent's wallet)
    // can establish sessions even when a separate parent key is configured.
    let parent_key = &state.config.parent.identity_key;
    if !parent_key.is_empty() {
        let is_parent = req.identity_key == *parent_key;
        let is_self = !state.auth.server_identity_key.is_empty()
            && req.identity_key == state.auth.server_identity_key;
        if !is_parent && !is_self {
            tracing::warn!(
                "BRC-31 handshake rejected: identity key mismatch. Got {}, expected {} (or self={})",
                &req.identity_key[..req.identity_key.len().min(16)],
                &parent_key[..parent_key.len().min(16)],
                &state.auth.server_identity_key[..state.auth.server_identity_key.len().min(16)]
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let (mut response, session) = handle_handshake(&req, &state.auth.server_identity_key);

    // Sign the handshake response if server has an identity key
    if !state.auth.server_identity_key.is_empty() {
        match sign_handshake_response(
            &state.auth.wallet,
            &req.initial_nonce,
            &response.initial_nonce,
            &req.identity_key,
        )
        .await
        {
            Ok(sig_bytes) => {
                response.signature = Some(sig_bytes);
            }
            Err(e) => {
                tracing::warn!("Failed to sign handshake response: {e}");
                // Graceful degradation -- continue without signature
            }
        }
    }

    // Store session
    {
        let mut sessions = state.auth.brc31_sessions.lock().await;
        sessions.cleanup(); // Clean expired while we're here
        tracing::info!(
            "BRC-31 handshake: storing session for client={}..., server_nonce={}..., signature={}",
            &req.identity_key[..req.identity_key.len().min(16)],
            &session.server_nonce_b64[..session.server_nonce_b64.len().min(12)],
            if response.signature.is_some() {
                "present"
            } else {
                "none"
            }
        );
        sessions.insert(session);
    }

    Ok(Json(response))
}
