//! Replay and fork route handlers.
//!
//! - GET  /task/{id}/replay  — structured replay data with cost/tool timelines
//! - POST /task/{id}/fork    — fork execution from a given event index

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::task_spawner::spawn_task;
use super::super::AppState;

use crate::replay::{ForkExecutor, ForkParams, ReplayViewer};

/// Response for GET /task/{id}/replay.
#[derive(Serialize)]
pub(crate) struct ReplayResponse {
    task_id: String,
    total_events: usize,
    total_iterations: u32,
    total_sats: u64,
    duration_secs: f64,
    events: Vec<crate::replay::ReplayEvent>,
    cost_timeline: Vec<crate::replay::CostTimelinePoint>,
    tool_usage: Vec<crate::replay::ToolUsageEntry>,
}

/// Request body for POST /task/{id}/fork.
#[derive(Deserialize)]
pub(crate) struct ForkRequest {
    /// Zero-based event index to fork from.
    pub event_index: usize,
    /// Optional new task message for the forked run.
    #[serde(default)]
    pub message: Option<String>,
    /// Optional model override.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional max iterations override.
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

/// Response for POST /task/{id}/fork.
#[derive(Serialize)]
pub(crate) struct ForkResponse {
    /// New task ID for the forked execution.
    task_id: String,
    /// Session ID for the forked task.
    session_id: String,
    /// Original task description.
    original_task: String,
    /// Number of events from the original transcript consumed.
    events_consumed: usize,
    /// Iteration number at the fork point.
    fork_iteration: u32,
    /// Sats spent in the original run up to the fork point.
    original_sats_at_fork: u64,
}

/// GET /task/{id}/replay — returns structured replay data.
pub(crate) async fn get_replay(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/replay"),
        None,
        &headers,
        None,
    )
    .await?;

    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let viewer = ReplayViewer::new(&transcript_path);
    let timeline = viewer.build_timeline();

    let body = ReplayResponse {
        task_id: id,
        total_events: timeline.total_events,
        total_iterations: timeline.total_iterations,
        total_sats: timeline.total_sats,
        duration_secs: timeline.duration_secs,
        events: timeline.events,
        cost_timeline: timeline.cost_timeline,
        tool_usage: timeline.tool_usage,
    };

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// POST /task/{id}/fork — fork execution from a given event index.
pub(crate) async fn fork_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<ForkRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/task/{id}/fork"),
        None,
        &headers,
        None,
    )
    .await?;

    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        let body = serde_json::json!({"error": "Task transcript not found"});
        return signed_json_response(&state, auth_ctx, StatusCode::NOT_FOUND, &body).await;
    }

    // Prepare the fork: reconstruct conversation state up to the event index
    let fork_params = ForkParams {
        event_index: req.event_index,
        message: req.message.clone(),
        model: req.model.clone(),
        max_iterations: req.max_iterations,
    };

    let fork_result = match ForkExecutor::prepare_fork(&transcript_path, &fork_params) {
        Ok(result) => result,
        Err(e) => {
            let body = serde_json::json!({"error": e});
            return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
        }
    };

    // Build the task message for the forked run
    let task_message = req.message.unwrap_or_else(|| {
        format!(
            "Fork of task {} at event {}: {}",
            id, req.event_index, fork_result.original_task
        )
    });

    let max_iter = req
        .max_iterations
        .unwrap_or(super::super::types::default_max_iterations());

    // Spawn the forked task
    let new_task_id = uuid::Uuid::new_v4().to_string();
    let (task_id, session_id) = spawn_task(
        &state,
        new_task_id,
        task_message,
        max_iter,
        None,      // new conversation
        req.model, // model override
        None,      // no reply routing
        vec!["fork".to_string(), format!("fork-of:{id}")],
        "fork".to_string(),
        None, // no attachments
        None, // no delegation envelope
    )
    .await?;

    // Store the fork metadata so the spawned task can pick up prior messages.
    // We write the prior_messages to a fork context file that task_spawner
    // will detect if needed. For now, the fork metadata is informational.
    let fork_meta_path = state
        .workspace
        .join(format!("tasks/{task_id}/fork_context.json"));
    if let Some(parent) = fork_meta_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let fork_meta = serde_json::json!({
        "source_task_id": id,
        "event_index": req.event_index,
        "fork_iteration": fork_result.fork_iteration,
        "original_sats_at_fork": fork_result.original_sats_at_fork,
        "events_consumed": fork_result.events_consumed,
        "prior_messages": fork_result.prior_messages,
    });
    let _ = std::fs::write(
        &fork_meta_path,
        serde_json::to_string_pretty(&fork_meta).unwrap_or_default(),
    );

    let body = ForkResponse {
        task_id,
        session_id,
        original_task: fork_result.original_task,
        events_consumed: fork_result.events_consumed,
        fork_iteration: fork_result.fork_iteration,
        original_sats_at_fork: fork_result.original_sats_at_fork,
    };

    signed_json_response(&state, auth_ctx, StatusCode::ACCEPTED, &body).await
}
