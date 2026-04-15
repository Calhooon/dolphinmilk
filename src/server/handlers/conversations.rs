//! Conversation route handlers.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::conversation::ConversationManager;
use crate::transcript::Transcript;

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

// -- Local types --

/// Query parameters for conversation detail endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct ConversationDetailParams {
    verify: Option<bool>,
}

/// Extended conversation detail response (includes optional verification).
#[derive(Debug, Serialize)]
pub(crate) struct ConversationDetailWithVerification {
    conversation: crate::conversation::Conversation,
    messages: Vec<crate::conversation::ConversationMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<crate::conversation::ChainVerification>,
    summary: ConversationSummaryStats,
}

/// Aggregated statistics for a conversation detail response.
#[derive(Debug, Serialize)]
pub(crate) struct ConversationSummaryStats {
    pub total_iterations: u32,
    pub total_proofs: usize,
    pub total_artifacts: usize,
    pub duration_secs: f64,
    pub tasks: Vec<ConversationTaskDetail>,
    pub cost_by_service: std::collections::HashMap<String, u64>,
    pub models_used: Vec<String>,
}

/// Per-task detail within a conversation summary.
#[derive(Debug, Serialize)]
pub(crate) struct ConversationTaskDetail {
    pub id: String,
    pub status: String,
    pub task: String,
    pub iterations: u32,
    pub sats_spent: u64,
    pub proof_count: usize,
    pub artifact_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

// -- Handlers --

/// GET /conversations -- list all conversations, newest first.
pub(crate) async fn list_conversations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/conversations", None, &headers, None).await?;
    let mgr = ConversationManager::new(&state.workspace);
    let body = mgr.list().unwrap_or_default();
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /conversations/{id} -- conversation metadata + all messages.
///
/// Self-healing: if the conversation has task_ids but no assistant messages
/// (e.g., because `append_from_transcript` failed or the task panicked),
/// reconstruct messages from the task transcripts on the fly.
pub(crate) async fn get_conversation_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<ConversationDetailParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/conversations/{id}"),
        None,
        &headers,
        None,
    )
    .await?;
    let mgr = ConversationManager::new(&state.workspace);
    let conv = mgr
        .load(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let conv = conv.ok_or(StatusCode::NOT_FOUND)?;
    let mut messages = mgr.load_messages(&id).unwrap_or_default();

    // Self-healing: if tasks ran but no assistant messages were assembled,
    // reconstruct from the task transcripts (the source of truth).
    let has_assistant = messages.iter().any(|m| m.role == "assistant");
    if !has_assistant && !conv.task_ids.is_empty() {
        for task_id in &conv.task_ids {
            let transcript_path = state
                .workspace
                .join(format!("tasks/{task_id}/session.jsonl"));
            if transcript_path.exists() {
                let transcript = Transcript::new(transcript_path);
                if let Err(e) = mgr.append_from_transcript(&id, &transcript, task_id) {
                    tracing::warn!("Self-heal conversation {id} from task {task_id}: {e}");
                }
            }
        }
        // Reload after reconstruction
        messages = mgr.load_messages(&id).unwrap_or_default();
        if messages.iter().any(|m| m.role == "assistant") {
            tracing::info!("Self-healed conversation {id}: recovered messages from transcripts");
        }
    }

    // Reload metadata after potential self-heal (append_from_transcript updates meta)
    let conv = mgr
        .load(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let verification = if params.verify.unwrap_or(false) {
        Some(
            mgr.verify_chain(&id)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )
    } else {
        None
    };

    // Build summary stats from task data
    let summary = build_conversation_summary(&state, &conv).await;

    let body = ConversationDetailWithVerification {
        conversation: conv,
        messages,
        verification,
        summary,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// Build aggregated summary stats for a conversation from its task data.
async fn build_conversation_summary(
    state: &Arc<AppState>,
    conv: &crate::conversation::Conversation,
) -> ConversationSummaryStats {
    let mut total_iterations: u32 = 0;
    let mut total_proofs: usize = 0;
    let mut total_artifacts: usize = 0;
    let mut first_started: Option<f64> = None;
    let mut last_completed: Option<f64> = None;
    let mut cost_by_service: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    let mut models_used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut task_details: Vec<ConversationTaskDetail> = Vec::new();

    let system_files: std::collections::HashSet<&str> = [
        "session.jsonl",
        "budget.jsonl",
        "artifacts.json",
        "fork_context.json",
    ]
    .into_iter()
    .collect();

    for task_id in &conv.task_ids {
        let transcript_path = state
            .workspace
            .join(format!("tasks/{task_id}/session.jsonl"));

        let mut task_iterations: u32 = 0;
        let mut task_sats: u64 = 0;
        let mut task_proofs: usize = 0;
        let mut task_desc = String::new();
        let mut task_status = "unknown".to_string();
        let mut task_started: Option<String> = None;
        let mut task_completed: Option<String> = None;

        if transcript_path.exists() {
            let transcript = Transcript::new(transcript_path);
            task_sats = transcript.total_sats_spent();
            let events = transcript.replay();

            for e in events {
                match e.event_type.as_str() {
                    "user" | "session_start" => {
                        if task_desc.is_empty() {
                            if let Some(content) = e
                                .data
                                .get("content")
                                .and_then(|v: &serde_json::Value| v.as_str())
                            {
                                task_desc = content.chars().take(200).collect::<String>();
                            }
                        }
                        if first_started.is_none() || e.ts < first_started.unwrap_or(f64::MAX) {
                            first_started = Some(e.ts);
                        }
                        if task_started.is_none() {
                            task_started = chrono::DateTime::from_timestamp(e.ts as i64, 0)
                                .map(|dt| dt.to_rfc3339());
                        }
                    }
                    "think_response" => {
                        task_iterations += 1;
                        if let Some(model) = e
                            .data
                            .get("model")
                            .and_then(|v: &serde_json::Value| v.as_str())
                        {
                            models_used.insert(model.to_string());
                        }
                        // Track cost by service
                        if let Some(sats) = e
                            .data
                            .get("sats_effective")
                            .and_then(|v: &serde_json::Value| v.as_u64())
                            .or_else(|| {
                                e.data
                                    .get("sats_paid")
                                    .and_then(|v: &serde_json::Value| v.as_u64())
                            })
                        {
                            let service = e
                                .data
                                .get("model")
                                .and_then(|v: &serde_json::Value| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            *cost_by_service.entry(service).or_insert(0) += sats;
                        }
                    }
                    "session_end" => {
                        if let Some(s) = e
                            .data
                            .get("status")
                            .and_then(|v: &serde_json::Value| v.as_str())
                        {
                            task_status = s.to_string();
                        } else {
                            task_status = "complete".to_string();
                        }
                        if e.ts > last_completed.unwrap_or(0.0) {
                            last_completed = Some(e.ts);
                        }
                        task_completed = chrono::DateTime::from_timestamp(e.ts as i64, 0)
                            .map(|dt| dt.to_rfc3339());
                    }
                    "proof_created" | "checkpoint_created" => {
                        task_proofs += 1;
                    }
                    _ => {}
                }
            }
        }

        // Count artifacts for this task
        let task_dir = state.workspace.join("tasks").join(task_id);
        let mut artifact_count: usize = 0;
        if task_dir.exists() {
            let manifest_path = task_dir.join("artifacts.json");
            if manifest_path.exists() {
                if let Ok(content) = tokio::fs::read_to_string(&manifest_path).await {
                    if let Ok(arts) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                        artifact_count = arts.len();
                    }
                }
            }
            // Also count non-system files in the task dir
            if let Ok(mut entries) = tokio::fs::read_dir(&task_dir).await {
                let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
                // Re-read manifest names to avoid double-counting
                if let Ok(content) =
                    tokio::fs::read_to_string(task_dir.join("artifacts.json")).await
                {
                    if let Ok(arts) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                        for a in &arts {
                            if let Some(name) = a.get("name").and_then(|n| n.as_str()) {
                                known.insert(name.to_string());
                            }
                        }
                    }
                }
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if known.contains(&name)
                        || system_files.contains(name.as_str())
                        || name.starts_with('.')
                        || name.starts_with("tool_output_")
                    {
                        continue;
                    }
                    if let Ok(ft) = entry.file_type().await {
                        if ft.is_dir() {
                            continue;
                        }
                    }
                    artifact_count += 1;
                }
            }
        }

        total_iterations += task_iterations;
        total_proofs += task_proofs;
        total_artifacts += artifact_count;

        task_details.push(ConversationTaskDetail {
            id: task_id.clone(),
            status: task_status,
            task: task_desc,
            iterations: task_iterations,
            sats_spent: task_sats,
            proof_count: task_proofs,
            artifact_count,
            started_at: task_started,
            completed_at: task_completed,
        });
    }

    let duration_secs = match (first_started, last_completed) {
        (Some(start), Some(end)) => (end - start).max(0.0),
        _ => 0.0,
    };

    ConversationSummaryStats {
        total_iterations,
        total_proofs,
        total_artifacts,
        duration_secs,
        tasks: task_details,
        cost_by_service,
        models_used: models_used.into_iter().collect(),
    }
}

/// GET /conversations/{id}/verify -- hash chain integrity check.
pub(crate) async fn verify_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/conversations/{id}/verify"),
        None,
        &headers,
        None,
    )
    .await?;
    let mgr = ConversationManager::new(&state.workspace);
    let result = mgr
        .verify_chain(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    signed_json_response(&state, auth_ctx, StatusCode::OK, &result).await
}

/// POST /conversations/{id}/sync -- encrypt and store conversation on-chain via wallet.
pub(crate) async fn sync_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/conversations/{id}/sync"),
        None,
        &headers,
        None,
    )
    .await?;
    let mgr = ConversationManager::new(&state.workspace);
    let result = mgr
        .sync_to_wallet(state.wallet.as_ref(), &id)
        .await
        .map_err(|e| {
            tracing::error!("sync_to_wallet failed for {id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    signed_json_response(&state, auth_ctx, StatusCode::OK, &result).await
}

/// POST /conversations/{id}/compact -- manually trigger compaction for a conversation.
///
/// Generates an LLM summary of older messages that exceed `max_history_turns`.
/// Stores the summary in conversation metadata so subsequent tasks see a
/// compact context instead of the full history.  Returns no-op if the
/// conversation is already within limits.
pub(crate) async fn compact_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/conversations/{id}/compact"),
        None,
        &headers,
        None,
    )
    .await?;

    let conv_mgr = ConversationManager::new(&state.workspace);
    let _conv = conv_mgr
        .load(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let messages = conv_mgr
        .load_messages(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let max_turns = state.config.llm.max_history_turns;
    if messages.len() <= max_turns {
        return signed_json_response(
            &state,
            auth_ctx,
            StatusCode::OK,
            &serde_json::json!({
                "compacted": false,
                "reason": "conversation is within history limits",
                "message_count": messages.len(),
                "max_history_turns": max_turns,
            }),
        )
        .await;
    }

    // Convert to OpenAI format for summarization
    let openai_msgs = conv_mgr
        .to_openai_messages(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let drop_count = openai_msgs.len().saturating_sub(max_turns);
    let to_summarize = &openai_msgs[..drop_count];

    // Generate summary via LLM call
    let auth_client =
        crate::auth::AuthriteClient::new(state.wallet.clone(), &state.config.wallet.url);
    let model = state
        .config
        .llm
        .compaction_model
        .as_deref()
        .unwrap_or(&state.config.llm.default_model);

    let summary = crate::context::manager::generate_compaction_summary(
        &auth_client,
        to_summarize,
        model,
        &state.config,
    )
    .await
    .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    // Store summary in conversation metadata
    conv_mgr
        .set_compaction_summary(&id, summary.clone(), drop_count as u32)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    signed_json_response(
        &state,
        auth_ctx,
        StatusCode::OK,
        &serde_json::json!({
            "compacted": true,
            "messages_summarized": drop_count,
            "summary_length": summary.len(),
        }),
    )
    .await
}

/// Query parameters for conversation artifacts endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct ConversationArtifactsParams {
    /// Filter by artifact type (e.g. `?type=image`).
    #[serde(rename = "type")]
    pub artifact_type: Option<String>,
}

/// GET /conversations/{id}/artifacts -- all artifacts from all tasks in a conversation.
pub(crate) async fn get_conversation_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<ConversationArtifactsParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/conversations/{id}/artifacts"),
        None,
        &headers,
        None,
    )
    .await?;

    let mgr = ConversationManager::new(&state.workspace);
    let conv = mgr
        .load(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let conv = conv.ok_or(StatusCode::NOT_FOUND)?;

    // Load messages for turn derivation
    let messages = mgr.load_messages(&id).unwrap_or_default();

    let mut all_artifacts: Vec<serde_json::Value> = Vec::new();
    let mut by_type: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    let system_files: std::collections::HashSet<&str> = [
        "session.jsonl",
        "budget.jsonl",
        "artifacts.json",
        "fork_context.json",
    ]
    .into_iter()
    .collect();

    for task_id in &conv.task_ids {
        let task_dir = state.workspace.join("tasks").join(task_id);
        if !task_dir.exists() {
            continue;
        }

        // Read artifacts.json manifest
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

        // Scan directory for files the manifest might have missed
        let known: std::collections::HashSet<String> = artifacts
            .iter()
            .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        if let Ok(mut entries) = tokio::fs::read_dir(&task_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if known.contains(&name)
                    || system_files.contains(name.as_str())
                    || name.starts_with('.')
                    || name.starts_with("tool_output_")
                {
                    continue;
                }
                if let Ok(ft) = entry.file_type().await {
                    if ft.is_dir() {
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

        // Derive the turn number and prompt for this task
        let (turn, turn_prompt) = messages
            .iter()
            .filter(|m| m.role == "user")
            .enumerate()
            .find(|(_, m)| m.task_id.as_deref() == Some(task_id))
            .map(|(i, m)| {
                let prompt: String = m.content.chars().take(200).collect();
                (i + 1, prompt)
            })
            .unwrap_or_else(|| {
                // Fallback: derive from task_ids ordering
                let idx = conv.task_ids.iter().position(|t| t == task_id).unwrap_or(0);
                (idx + 1, String::new())
            });

        // Enrich each artifact with provenance metadata
        for artifact in &mut artifacts {
            if let Some(obj) = artifact.as_object_mut() {
                obj.insert("task_id".to_string(), serde_json::json!(task_id));
                obj.insert("turn".to_string(), serde_json::json!(turn));
                obj.insert("turn_prompt".to_string(), serde_json::json!(turn_prompt));
                if let Some(name) = obj.get("name").and_then(|n| n.as_str()) {
                    let url = format!("/files/{}/{}", task_id, name);
                    obj.insert("url".to_string(), serde_json::json!(url));
                }
                // Track by_type counts
                let atype = obj
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("file")
                    .to_string();
                *by_type.entry(atype).or_insert(0) += 1;
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

    // Sort by created_at ascending (chronological within conversation)
    all_artifacts.sort_by(|a, b| {
        let a_ts = a.get("created_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b_ts = b.get("created_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
        a_ts.partial_cmp(&b_ts).unwrap_or(std::cmp::Ordering::Equal)
    });

    let total = all_artifacts.len();
    let body = serde_json::json!({
        "conversation_id": id,
        "artifacts": all_artifacts,
        "total": total,
        "by_type": by_type,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}
