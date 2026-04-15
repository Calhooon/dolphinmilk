//! Event and escalation handlers: task events, escalation status, resolution, escalation proofs.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::server::auth::{check_brc31_auth, signed_json_response};
use crate::server::types::*;
use crate::server::AppState;

// -- Local types --

/// DTO for a single transcript event exposed via the events endpoint.
#[derive(Serialize)]
pub(crate) struct TranscriptEventDto {
    index: usize,
    ts: f64,
    #[serde(rename = "type")]
    event_type: String,
    id: String,
    data: serde_json::Value,
}

/// Response for GET /task/{id}/events.
#[derive(Serialize)]
pub(crate) struct TaskEventsResponse {
    task_id: String,
    offset: usize,
    total: usize,
    events: Vec<TranscriptEventDto>,
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

/// Query params for GET /task/{id}/events.
#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(default)]
    since: usize,
}

/// Request body for resolving an escalation.
#[derive(Deserialize)]
pub(crate) struct ResolveEscalationRequest {
    /// Human-provided guidance to resume the task.
    pub guidance: String,
    /// Who resolved this (identity key or name).
    #[serde(default)]
    pub resolved_by: String,
}

/// Response for GET /task/{id}/escalation/proof.
#[derive(Debug, Serialize)]
pub(crate) struct EscalationProofResponse {
    pub task_id: String,
    pub proofs: Vec<EscalationProofDetail>,
}

/// A single escalation proof detail.
#[derive(Debug, Serialize)]
pub(crate) struct EscalationProofDetail {
    pub txid: String,
    pub proof_type: String,
    pub hash: String,
    pub timestamp: String,
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<serde_json::Value>,
}

// -- Handlers --

/// GET /task/{id}/events -- incremental transcript events for poll-based UI (BRC-31 auth).
pub(crate) async fn get_task_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    uri: axum::extract::OriginalUri,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/events"),
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        let task_registered = {
            let tasks = state.task_mgr.tasks.lock().await;
            tasks.contains_key(&id)
        };
        if !task_registered {
            return Err(StatusCode::NOT_FOUND);
        }
        let session_id = {
            let sessions = state.task_mgr.task_sessions.lock().await;
            sessions.get(&id).cloned()
        };
        let body = TaskEventsResponse {
            task_id: id,
            offset: 0,
            total: 0,
            events: vec![],
            active: true,
            session_id,
        };
        return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let since = query.since;
    let total = transcript.event_count();
    let slice = transcript.events_since(since);

    let events: Vec<TranscriptEventDto> = slice
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let data = serde_json::to_value(&e.data)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            TranscriptEventDto {
                index: since + i,
                ts: e.ts,
                event_type: e.event_type.clone(),
                id: e.id.clone(),
                data,
            }
        })
        .collect();

    let active = {
        let tasks = state.task_mgr.tasks.lock().await;
        tasks
            .get(&id)
            .map(|t| t.status == TaskStatus::Running)
            .unwrap_or(false)
    };

    let session_id = {
        let sessions = state.task_mgr.task_sessions.lock().await;
        sessions.get(&id).cloned()
    };

    let body = TaskEventsResponse {
        task_id: id,
        offset: since,
        total,
        events,
        active,
        session_id,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

// =============================================================================
// Escalation endpoints
// =============================================================================

/// GET /task/{id}/escalation — get escalation status for a task.
pub(crate) async fn get_escalation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/escalation"),
        None,
        &headers,
        None,
    )
    .await?;

    let status = {
        let map = state.escalation_status.lock().await;
        map.get(&id).cloned().unwrap_or_default()
    };

    signed_json_response(&state, auth_ctx, StatusCode::OK, &status).await
}

/// POST /task/{id}/escalation/resolve — resolve an escalation with human input.
pub(crate) async fn resolve_escalation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/task/{id}/escalation/resolve"),
        None,
        &headers,
        Some(body.as_ref()),
    )
    .await?;

    let req: ResolveEscalationRequest =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut map = state.escalation_status.lock().await;
    let status = map.entry(id.clone()).or_default();

    if !status.escalated {
        return Err(StatusCode::CONFLICT); // Not escalated
    }

    status.escalated = false;
    status.resolution = Some(crate::runner::escalation::EscalationResolution {
        guidance: req.guidance.clone(),
        resolved_by: if req.resolved_by.is_empty() {
            "unknown".to_string()
        } else {
            req.resolved_by
        },
        timestamp: chrono::Utc::now().to_rfc3339(),
    });

    let response_body = serde_json::json!({
        "task_id": id,
        "resolved": true,
        "guidance": req.guidance,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &response_body).await
}

/// GET /task/{id}/escalation/proof — returns escalation BRC-18 proofs for a task.
pub(crate) async fn get_escalation_proof(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/escalation/proof"),
        None,
        &headers,
        None,
    )
    .await?;

    // Read the task transcript and find escalation proofs
    let transcript_path = state
        .workspace
        .join("tasks")
        .join(&id)
        .join("session.jsonl");
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let events = transcript.get_events_by_type("proof_created");

    let mut proofs = Vec::new();
    for event in &events {
        // Filter for escalation proof type
        let proof_type = event
            .data
            .get("proof_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if proof_type != "Escalation" && proof_type != "escalation" {
            continue;
        }

        let txid = event
            .data
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let hash = event
            .data
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let timestamp = event
            .data
            .get("proof_timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let data = event
            .data
            .get("proof_data")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let prev_hash = event
            .data
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Check if there is a resolution in the escalation status
        let resolution = {
            let map = state.escalation_status.lock().await;
            map.get(&id)
                .and_then(|s| s.resolution.as_ref())
                .map(|r| serde_json::to_value(r).unwrap_or_default())
        };

        proofs.push(EscalationProofDetail {
            txid,
            proof_type: "escalation".to_string(),
            hash,
            timestamp,
            data,
            prev_hash,
            resolution,
        });
    }

    let response = EscalationProofResponse {
        task_id: id,
        proofs,
    };

    signed_json_response(&state, auth_ctx, StatusCode::OK, &response).await
}
