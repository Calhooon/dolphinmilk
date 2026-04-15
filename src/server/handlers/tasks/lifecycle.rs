//! Task lifecycle handlers: submit, status, listing, cancel, conversation, file serving.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use serde::Deserialize;

use crate::server::auth::{check_brc31_auth, signed_json_response};
use crate::server::task_spawner::spawn_task;
use crate::server::types::*;
use crate::server::AppState;

// -- Local types used only by task handlers --

/// Query params for GET /tasks.
#[derive(Debug, Deserialize)]
pub(crate) struct TaskListParams {
    /// Filter by tag (AND logic — task must have ALL specified tags).
    /// Single tag: `?tag=X`. Multiple tags: `?tag=X,Y,Z` (comma-separated).
    #[serde(default, deserialize_with = "deserialize_comma_separated")]
    pub tag: Vec<String>,
}

/// Deserialize a comma-separated string into a Vec<String>.
/// `?tag=X` → `["X"]`, `?tag=X,Y,Z` → `["X","Y","Z"]`, absent → `[]`.
fn deserialize_comma_separated<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    Ok(s.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect())
}

// -- Handlers --

pub(crate) async fn submit_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<TaskRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "POST", "/task", None, &headers, None).await?;

    let tags = crate::server::types::normalize_tags(req.tags);

    // Enforce cert-driven tag policy: if tags_required, reject tasks without tags
    if tags.is_empty() {
        let policy = crate::certificates::read_cert_tag_policy(state.wallet.clone()).await;
        if policy.tags_required == Some(true) {
            let body = serde_json::json!({"error": "Tags are required by certificate policy but none were provided"});
            return crate::server::auth::signed_json_response(
                &state,
                auth_ctx,
                StatusCode::BAD_REQUEST,
                &body,
            )
            .await;
        }
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let (task_id, session_id) = spawn_task(
        &state,
        task_id,
        req.task,
        req.max_iterations,
        None,
        req.model,
        None,
        tags,
        "task".to_string(),
        None,
        None,
    )
    .await?;

    let body = TaskResponse {
        id: task_id,
        status: TaskStatus::Running,
        session_id: Some(session_id),
    };
    signed_json_response(&state, auth_ctx, StatusCode::ACCEPTED, &body).await
}

pub(crate) async fn get_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/status", None, &headers, None).await?;
    let tasks = state.task_mgr.tasks.lock().await;
    let task_list: Vec<TaskInfo> = tasks.values().cloned().collect();
    let active = task_list
        .iter()
        .filter(|t| t.status == TaskStatus::Running)
        .count();
    let total_sats: u64 = task_list.iter().map(|t| t.sats_spent).sum();

    let body = StatusResponse {
        tasks: task_list,
        active_count: active,
        total_sats,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

pub(crate) async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", &format!("/task/{id}"), None, &headers, None).await?;
    let tasks = state.task_mgr.tasks.lock().await;
    let info = tasks.get(&id).cloned().ok_or(StatusCode::NOT_FOUND)?;
    signed_json_response(&state, auth_ctx, StatusCode::OK, &info).await
}

/// GET /tasks -- list all tasks across sessions (in-memory + disk).
pub(crate) async fn list_tasks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<TaskListParams>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/tasks", None, &headers, None).await?;
    let filter_tags: Vec<String> = params
        .tag
        .into_iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let mut summaries: HashMap<String, TaskSummary> = HashMap::new();

    // 1. Collect in-memory tasks (current/recent)
    // For sats_spent, prefer transcript data (consistent with /task/{id}/audit)
    // since worm.state.sats_spent includes proof costs that the transcript
    // total_sats_spent() doesn't count.
    {
        let tasks = state.task_mgr.tasks.lock().await;
        for (id, info) in tasks.iter() {
            // Try to compute sats, tokens, and tool_calls from transcript for consistency
            let (sats_spent, tokens, tool_call_count) = {
                let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
                if transcript_path.exists() {
                    let transcript = crate::transcript::Transcript::new(transcript_path);
                    let (pt, ct) = transcript.total_tokens();
                    let tc = transcript
                        .replay()
                        .iter()
                        .filter(|e| e.event_type == "tool_call")
                        .count() as u32;
                    (transcript.total_sats_spent(), pt + ct, tc)
                } else {
                    (info.sats_spent, 0, 0)
                }
            };
            let time_saved_minutes = crate::time_estimate::estimate_time_saved(
                &info.tags,
                info.iterations,
                tool_call_count,
            );
            summaries.insert(
                id.clone(),
                TaskSummary {
                    id: info.id.clone(),
                    task: info.task.clone(),
                    status: serde_json::to_value(&info.status)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "unknown".into()),
                    iterations: info.iterations,
                    sats_spent,
                    started_at: info.started_at.clone(),
                    completed_at: info.completed_at.clone(),
                    proof_txids: info.proof_txids.clone(),
                    tokens,
                    tags: info.tags.clone(),
                    time_saved_minutes,
                    origin: info.origin.clone(),
                    conversation_id: info.conversation_id.clone(),
                },
            );
        }
    }

    // 2. Scan disk for historical tasks not in memory
    let tasks_dir = state.workspace.join("tasks");
    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let task_id = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            // Skip if already in memory
            if summaries.contains_key(&task_id) {
                continue;
            }
            let transcript_path = path.join("session.jsonl");
            if !transcript_path.exists() {
                continue;
            }
            // Load minimal info from transcript
            let transcript = crate::transcript::Transcript::new(transcript_path);
            let events = transcript.replay();

            let mut task_desc = String::new();
            let mut started_at = String::new();
            let mut completed_at: Option<String> = None;
            let mut status = "unknown".to_string();
            let mut iterations: u32 = 0;
            let mut proof_txids: Vec<String> = Vec::new();

            // Extract from session_start event
            for e in events {
                match e.event_type.as_str() {
                    "session_start" | "user" => {
                        if task_desc.is_empty() {
                            if let Some(content) = e.data.get("content").and_then(|v| v.as_str()) {
                                task_desc = content.to_string();
                            }
                        }
                        if started_at.is_empty() {
                            // Convert unix timestamp to RFC3339
                            started_at = chrono::DateTime::from_timestamp(e.ts as i64, 0)
                                .map(|dt| dt.to_rfc3339())
                                .unwrap_or_default();
                        }
                    }
                    "session_end" => {
                        completed_at = Some(
                            chrono::DateTime::from_timestamp(e.ts as i64, 0)
                                .map(|dt| dt.to_rfc3339())
                                .unwrap_or_default(),
                        );
                        if let Some(s) = e.data.get("status").and_then(|v| v.as_str()) {
                            status = s.to_string();
                        } else {
                            status = "complete".to_string();
                        }
                        if let Some(iter) = e.data.get("iterations").and_then(|v| v.as_u64()) {
                            iterations = iter as u32;
                        }
                    }
                    "proof_created" | "checkpoint_created" => {
                        if let Some(txid) = e.data.get("txid").and_then(|v| v.as_str()) {
                            proof_txids.push(txid.to_string());
                        }
                    }
                    _ => {}
                }
            }

            // If no session_end, mark as incomplete (not complete — the task was
            // likely interrupted before finishing).
            if status == "unknown" && !events.is_empty() {
                status = "incomplete".to_string();
            }

            let sats_spent = transcript.total_sats_spent();
            let (pt, ct) = transcript.total_tokens();
            let tokens = pt + ct;
            let tool_call_count = events
                .iter()
                .filter(|e| e.event_type == "tool_call")
                .count() as u32;
            // Historical tasks from disk don't have tags, so only default estimate
            let time_saved_minutes =
                crate::time_estimate::default_estimate(iterations, tool_call_count);

            // Look up conversation_id from task_sessions
            let conv_id = {
                let sessions = state.task_mgr.task_sessions.lock().await;
                sessions.get(&task_id).cloned()
            };

            summaries.insert(
                task_id.clone(),
                TaskSummary {
                    id: task_id,
                    task: task_desc,
                    status,
                    iterations,
                    sats_spent,
                    started_at,
                    completed_at,
                    proof_txids,
                    tokens,
                    tags: Vec::new(), // Historical tasks from disk don't have tags
                    time_saved_minutes,
                    origin: String::new(), // Historical tasks from disk don't have origin
                    conversation_id: conv_id,
                },
            );
        }
    }

    // 3. Sort by started_at descending
    let mut tasks: Vec<TaskSummary> = summaries.into_values().collect();
    tasks.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    // 4. Apply tag filter (AND logic — task must have ALL requested tags)
    if !filter_tags.is_empty() {
        tasks.retain(|t| filter_tags.iter().all(|ft| t.tags.contains(ft)));
    }

    let total = tasks.len();
    let body = TaskListResponse { tasks, total };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /analytics/time-saved -- aggregate human-equivalent time saved.
pub(crate) async fn get_time_saved(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/analytics/time-saved", None, &headers, None).await?;

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut inputs: Vec<crate::time_estimate::TaskTimeSavedInput> = Vec::new();

    // Collect from in-memory tasks
    {
        let tasks = state.task_mgr.tasks.lock().await;
        for (id, info) in tasks.iter() {
            let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
            let tool_call_count = if transcript_path.exists() {
                let transcript = crate::transcript::Transcript::new(transcript_path);
                transcript
                    .replay()
                    .iter()
                    .filter(|e| e.event_type == "tool_call")
                    .count() as u32
            } else {
                0
            };
            let tag_override = crate::time_estimate::parse_time_saved_tag(&info.tags);
            let minutes = crate::time_estimate::estimate_time_saved(
                &info.tags,
                info.iterations,
                tool_call_count,
            );
            let started_epoch = chrono::DateTime::parse_from_rfc3339(&info.started_at)
                .map(|dt| dt.timestamp())
                .unwrap_or(0);
            inputs.push(crate::time_estimate::TaskTimeSavedInput {
                task_id: info.id.clone(),
                description: info.task.clone(),
                minutes_saved: minutes,
                source: if tag_override.is_some() {
                    "tag".into()
                } else {
                    "estimate".into()
                },
                started_at_epoch_secs: started_epoch,
            });
        }
    }

    // Scan disk for historical tasks not in memory
    let seen: std::collections::HashSet<String> =
        inputs.iter().map(|i| i.task_id.clone()).collect();
    let tasks_dir = state.workspace.join("tasks");
    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let task_id = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if seen.contains(&task_id) {
                continue;
            }
            let transcript_path = path.join("session.jsonl");
            if !transcript_path.exists() {
                continue;
            }
            let transcript = crate::transcript::Transcript::new(transcript_path);
            let events = transcript.replay();

            let mut task_desc = String::new();
            let mut started_epoch: i64 = 0;
            let mut iterations: u32 = 0;

            for e in events {
                match e.event_type.as_str() {
                    "session_start" | "user" => {
                        if task_desc.is_empty() {
                            if let Some(content) = e.data.get("content").and_then(|v| v.as_str()) {
                                task_desc = content.to_string();
                            }
                        }
                        if started_epoch == 0 {
                            started_epoch = e.ts as i64;
                        }
                    }
                    "session_end" => {
                        if let Some(iter) = e.data.get("iterations").and_then(|v| v.as_u64()) {
                            iterations = iter as u32;
                        }
                    }
                    _ => {}
                }
            }

            let tool_call_count = events
                .iter()
                .filter(|e| e.event_type == "tool_call")
                .count() as u32;
            let minutes = crate::time_estimate::default_estimate(iterations, tool_call_count);

            inputs.push(crate::time_estimate::TaskTimeSavedInput {
                task_id,
                description: task_desc,
                minutes_saved: minutes,
                source: "estimate".into(),
                started_at_epoch_secs: started_epoch,
            });
        }
    }

    let analytics = crate::time_estimate::aggregate_time_saved(&inputs, now_epoch);
    signed_json_response(&state, auth_ctx, StatusCode::OK, &analytics).await
}

/// GET /task/{id}/conversation -- reconstructed conversation messages.
pub(crate) async fn get_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/conversation"),
        None,
        &headers,
        None,
    )
    .await?;
    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let messages = transcript.to_messages();

    let body = ConversationResponse {
        task_id: id,
        messages,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// POST /task/{id}/cancel -- cancel a running task.
pub(crate) async fn cancel_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    use std::sync::atomic::Ordering;

    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/task/{id}/cancel"),
        None,
        &headers,
        None,
    )
    .await?;
    // Check task exists and is running or queued
    {
        let tasks = state.task_mgr.tasks.lock().await;
        match tasks.get(&id) {
            Some(info) if matches!(info.status, TaskStatus::Running | TaskStatus::Queued) => {}
            Some(_) => return Err(StatusCode::CONFLICT), // already finished
            None => return Err(StatusCode::NOT_FOUND),
        }
    }

    // Set the cancel flag — the spawned task checks this after acquiring the
    // conversation semaphore (for queued tasks) and between iterations (for running tasks).
    let flags = state.task_mgr.cancel_flags.lock().await;
    if let Some(flag) = flags.get(&id) {
        flag.store(true, Ordering::Relaxed);

        let body = serde_json::json!({"status": "cancelled"});
        signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /task/{id}/resume -- resume an interrupted task.
///
/// Creates a new task that continues the interrupted one's conversation.
/// The prior conversation context is injected so the agent can pick up
/// where it left off. Only tasks with status `Interrupted` can be resumed.
pub(crate) async fn resume_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/task/{id}/resume"),
        None,
        &headers,
        None,
    )
    .await?;

    // Look up the interrupted task
    let (task_desc, conversation_id) = {
        let tasks = state.task_mgr.tasks.lock().await;
        match tasks.get(&id) {
            Some(info) if info.status == TaskStatus::Interrupted => {
                (info.task.clone(), info.conversation_id.clone())
            }
            Some(info) => {
                let body = serde_json::json!({
                    "error": format!("Task is {:?}, not interrupted — cannot resume", info.status)
                });
                return signed_json_response(&state, auth_ctx, StatusCode::CONFLICT, &body).await;
            }
            None => return Err(StatusCode::NOT_FOUND),
        }
    };

    // Reconstruct session state from the interrupted transcript
    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    let session_state = if transcript_path.exists() {
        let transcript = crate::transcript::Transcript::new(transcript_path);
        let state = transcript.reconstruct_state();
        Some(state)
    } else {
        None
    };

    // Build resume message with context
    let resume_msg = format!(
        "RESUME interrupted task. Original task: {}\n\
         Continue from where you left off. Do not recap or apologize — just pick up the work.",
        task_desc
    );

    // Spawn a new task on the same conversation (if one exists)
    let new_task_id = uuid::Uuid::new_v4().to_string();
    let (spawned_task_id, session_id) = spawn_task(
        &state,
        new_task_id,
        resume_msg,
        50, // max_iterations (default)
        conversation_id,
        None, // model override
        None, // reply_to
        vec!["resume".to_string(), format!("resumed-from:{id}")],
        "resume".to_string(),
        None, // attachments
        None, // delegation envelope
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Mark the old task as Complete (it's been superseded by the resume)
    {
        let mut tasks = state.task_mgr.tasks.lock().await;
        if let Some(info) = tasks.get_mut(&id) {
            info.status = TaskStatus::Complete;
            info.result = Some(format!("(resumed as task {})", spawned_task_id));
        }
    }

    let body = serde_json::json!({
        "resumed_task_id": spawned_task_id,
        "session_id": session_id,
        "original_task_id": id,
        "original_task": task_desc,
        "prior_iterations": session_state.as_ref().map(|s| s.iterations).unwrap_or(0),
        "prior_sats_spent": session_state.as_ref().map(|s| s.sats_spent).unwrap_or(0),
    });
    signed_json_response(&state, auth_ctx, StatusCode::ACCEPTED, &body).await
}

/// GET /task/{id}/conversation-id -- lightweight lookup of conversation_id for a task.
pub(crate) async fn get_task_conversation_id(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/conversation-id"),
        None,
        &headers,
        None,
    )
    .await?;

    // Verify task exists
    {
        let tasks = state.task_mgr.tasks.lock().await;
        if !tasks.contains_key(&id) {
            return Err(StatusCode::NOT_FOUND);
        }
    }

    let conversation_id = {
        let sessions = state.task_mgr.task_sessions.lock().await;
        sessions.get(&id).cloned()
    };

    let body = serde_json::json!({ "conversation_id": conversation_id });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /task/{id}/artifacts -- list artifacts (files created by the agent) in task workspace.
pub(crate) async fn get_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/artifacts"),
        None,
        &headers,
        None,
    )
    .await?;

    let task_dir = state.workspace.join("tasks").join(&id);
    if !task_dir.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Read artifacts.json manifest if it exists
    let manifest_path = task_dir.join("artifacts.json");
    let mut artifacts: Vec<serde_json::Value> = if manifest_path.exists() {
        tokio::fs::read_to_string(&manifest_path)
            .await
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Also scan directory for any files the manifest might have missed
    let known: std::collections::HashSet<String> = artifacts
        .iter()
        .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();

    let system_files: std::collections::HashSet<&str> = [
        "session.jsonl",
        "budget.jsonl",
        "artifacts.json",
        "fork_context.json",
    ]
    .into_iter()
    .collect();
    if let Ok(mut entries) = tokio::fs::read_dir(&task_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if known.contains(&name) {
                continue;
            }
            if system_files.contains(name.as_str()) {
                continue;
            }
            if name.starts_with('.') || name.starts_with("tool_output_") {
                continue;
            }

            if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    // Skip all directories (system or otherwise)
                    continue;
                }
            }

            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            let ext = std::path::Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let artifact_type = match ext.as_str() {
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" => "image",
                "mp4" | "webm" | "mov" | "avi" => "video",
                "mp3" | "wav" | "ogg" | "m4a" | "flac" => "audio",
                "pdf" => "document",
                _ => "file",
            };

            artifacts.push(serde_json::json!({
                "name": name,
                "type": artifact_type,
                "size_bytes": size,
                "created_by": "",
                "created_at": 0,
            }));
        }
    }

    // Add download URLs
    for artifact in &mut artifacts {
        if let Some(name) = artifact.get("name").and_then(|n| n.as_str()) {
            let url = format!("/files/{}/{}", id, name);
            if let Some(obj) = artifact.as_object_mut() {
                obj.insert("url".to_string(), serde_json::json!(url));
            }
        }
    }

    let total = artifacts.len();
    let body = serde_json::json!({
        "task_id": id,
        "artifacts": artifacts,
        "total": total,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// Query params for GET /artifacts.
#[derive(Debug, Deserialize)]
pub(crate) struct AllArtifactsParams {
    /// Filter by artifact type (e.g. `?type=image`).
    #[serde(rename = "type")]
    pub artifact_type: Option<String>,
}

/// GET /artifacts -- list artifacts across ALL tasks.
///
/// Scans every task directory under `workspace/tasks/`, reads `artifacts.json`
/// manifests and scans for unlisted files, enriches each artifact with
/// `task_id`, `task_text`, and a download URL, then returns the combined list
/// sorted by `created_at` descending. Supports `?type=image` filtering.
pub(crate) async fn get_all_artifacts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<AllArtifactsParams>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/artifacts", None, &headers, None).await?;

    let tasks_dir = state.workspace.join("tasks");
    if !tasks_dir.exists() {
        let body = serde_json::json!({ "artifacts": [], "total": 0 });
        return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
    }

    let mut all_artifacts: Vec<serde_json::Value> = Vec::new();

    let mut entries = tokio::fs::read_dir(&tasks_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        // Only process directories
        let ft = match entry.file_type().await {
            Ok(ft) if ft.is_dir() => ft,
            _ => continue,
        };
        let _ = ft; // suppress unused warning

        let task_id = entry.file_name().to_string_lossy().to_string();
        let task_dir = entry.path();

        // Read artifacts.json manifest if it exists
        let manifest_path = task_dir.join("artifacts.json");
        let mut artifacts: Vec<serde_json::Value> = if manifest_path.exists() {
            tokio::fs::read_to_string(&manifest_path)
                .await
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Also scan directory for any files the manifest might have missed
        let known: std::collections::HashSet<String> = artifacts
            .iter()
            .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        let system_files: std::collections::HashSet<&str> = [
            "session.jsonl",
            "budget.jsonl",
            "artifacts.json",
            "fork_context.json",
        ]
        .into_iter()
        .collect();

        if let Ok(mut dir_entries) = tokio::fs::read_dir(&task_dir).await {
            while let Ok(Some(dir_entry)) = dir_entries.next_entry().await {
                let name = dir_entry.file_name().to_string_lossy().to_string();
                if known.contains(&name) {
                    continue;
                }
                if system_files.contains(name.as_str()) {
                    continue;
                }
                if name.starts_with('.') || name.starts_with("tool_output_") {
                    continue;
                }

                if let Ok(ft) = dir_entry.file_type().await {
                    if ft.is_dir() {
                        continue;
                    }
                }

                let size = dir_entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                let ext = std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let artifact_type = match ext.as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" => "image",
                    "mp4" | "webm" | "mov" | "avi" => "video",
                    "mp3" | "wav" | "ogg" | "m4a" | "flac" => "audio",
                    "pdf" => "document",
                    _ => "file",
                };

                artifacts.push(serde_json::json!({
                    "name": name,
                    "type": artifact_type,
                    "size_bytes": size,
                    "created_by": "",
                    "created_at": 0,
                }));
            }
        }

        if artifacts.is_empty() {
            continue;
        }

        // Read task_text from the first "user" event in session.jsonl
        let task_text = {
            let session_path = task_dir.join("session.jsonl");
            if session_path.exists() {
                extract_task_text(&session_path).await
            } else {
                String::new()
            }
        };

        // Enrich each artifact with task_id, task_text, and download URL
        for artifact in &mut artifacts {
            if let Some(obj) = artifact.as_object_mut() {
                obj.insert("task_id".to_string(), serde_json::json!(task_id));
                obj.insert("task_text".to_string(), serde_json::json!(task_text));
                if let Some(name) = obj.get("name").and_then(|n| n.as_str()) {
                    let url = format!("/files/{}/{}", task_id, name);
                    obj.insert("url".to_string(), serde_json::json!(url));
                }
            }
        }

        all_artifacts.extend(artifacts);
    }

    // Filter by type if requested
    if let Some(ref filter_type) = params.artifact_type {
        all_artifacts.retain(|a| {
            a.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == filter_type.as_str())
                .unwrap_or(false)
        });
    }

    // Sort by created_at descending (newest first)
    all_artifacts.sort_by(|a, b| {
        let a_ts = a.get("created_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b_ts = b.get("created_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
        b_ts.partial_cmp(&a_ts).unwrap_or(std::cmp::Ordering::Equal)
    });

    let total = all_artifacts.len();
    let body = serde_json::json!({
        "artifacts": all_artifacts,
        "total": total,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// Extract the task text from the first "user" event in a session.jsonl file.
async fn extract_task_text(session_path: &std::path::Path) -> String {
    let content = match tokio::fs::read_to_string(session_path).await {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    for line in content.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let event_type = val
                .get("event_type")
                .or_else(|| val.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if event_type == "user" || event_type == "user_message" {
                if let Some(content) = val.get("content").and_then(|c| c.as_str()) {
                    // Return first 200 chars to keep response size reasonable
                    let truncated: String = content.chars().take(200).collect();
                    return truncated;
                }
                // Also check nested data.content
                if let Some(data) = val.get("data") {
                    if let Some(content) = data.get("content").and_then(|c| c.as_str()) {
                        let truncated: String = content.chars().take(200).collect();
                        return truncated;
                    }
                }
            }
        }
    }
    String::new()
}

/// GET /files/{task_id}/{filename} -- serve files from task workspace.
///
/// Serves files from `working/tasks/{task_id}/`, with path traversal protection.
/// Sets Content-Type based on file extension.
pub(crate) async fn serve_task_file(
    State(state): State<Arc<AppState>>,
    Path((task_id, filename)): Path<(String, String)>,
) -> Result<axum::response::Response, StatusCode> {
    // Validate: no path traversal
    if task_id.contains("..")
        || filename.contains("..")
        || filename.contains('/')
        || filename.contains('\\')
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let file_path = state.workspace.join("tasks").join(&task_id).join(&filename);

    // Also check uploads/ subdirectory for user-supplied attachments
    let file_path = if file_path.exists() {
        file_path
    } else {
        let uploads_path = state
            .workspace
            .join("tasks")
            .join(&task_id)
            .join("uploads")
            .join(&filename);
        if uploads_path.exists() {
            uploads_path
        } else {
            file_path // Let the canonical check below produce 404
        }
    };

    // Ensure the resolved path is still within the task directory
    let canonical = file_path
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let task_dir = state.workspace.join("tasks").join(&task_id);
    if let Ok(canonical_task_dir) = task_dir.canonicalize() {
        if !canonical.starts_with(&canonical_task_dir) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let data = tokio::fs::read(&canonical)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    // Determine Content-Type from extension
    let content_type = match filename
        .rsplit('.')
        .next()
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    };

    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(data))
        .unwrap())
}
