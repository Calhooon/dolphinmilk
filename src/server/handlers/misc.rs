//! Miscellaneous ingress route handlers (message forwarding, heartbeat trigger).

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::task_spawner::spawn_task;
use super::super::types::*;
use super::super::AppState;

// -- Handlers --

pub(crate) async fn receive_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MessageRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "POST", "/message", None, &headers, None).await?;
    tracing::info!(
        "HTTP message from {}...: box={:?}",
        &req.sender[..req.sender.len().min(16)],
        req.message_box,
    );

    let task_id = uuid::Uuid::new_v4().to_string();
    let body_str = match req.body {
        serde_json::Value::String(ref s) => s.clone(),
        ref v => v.to_string(),
    };
    let task_desc = format!(
        "Process message from {}: {}",
        &req.sender[..req.sender.len().min(16)],
        body_str
    );
    let (task_id, session_id) = spawn_task(
        &state,
        task_id,
        task_desc,
        50,
        req.conversation_id,
        None,
        Some(req.sender.clone()),
        Vec::new(),
        "message".to_string(),
        None,
        None,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let body = serde_json::json!({
        "accepted": true,
        "task_id": task_id,
        "session_id": session_id,
    });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// POST /heartbeat/trigger -- manually inject a task into the scheduler's inbox.
///
/// Requires BRC-31 auth (when parent key is configured). Returns 503 if heartbeat is disabled.
pub(crate) async fn trigger_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Option<Json<HeartbeatTriggerRequest>>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "POST", "/heartbeat/trigger", None, &headers, None).await?;

    let tx = state
        .scheduler
        .heartbeat_tx
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let task_desc = body
        .and_then(|Json(r)| r.message)
        .unwrap_or_else(|| "HEARTBEAT_TRIGGER\nManual heartbeat trigger".to_string());

    tx.send(task_desc)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let resp = serde_json::json!({ "triggered": true });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &resp).await
}
