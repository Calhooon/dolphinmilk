//! Chat and OpenAI-compatible route handlers.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::events::StepEvent;

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::task_spawner::spawn_task;
use super::super::types::*;
use super::super::{
    prune_chat_command_dedupe, AppState, ChatCommandDedupeEntry, ChatCommandDedupeState,
};

// -- OpenAI-compat local types --

/// OpenAI-format chat request.
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIChatRequest {
    messages: Vec<OpenAIMessage>,
    #[serde(default)]
    stream: bool,
    /// Maps to `session_id` for conversation persistence.
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    model: Option<String>,
    /// Treated as max_iterations (not tokens) to cap agent loop length.
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct OpenAIMessage {
    role: String,
    content: String,
}

/// Non-streaming response in OpenAI format.
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIChatResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAIChoice {
    index: u32,
    message: OpenAIMessage,
    finish_reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAIUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

// -- Handlers --

/// POST /chat -- submit a chat message, returns signed JSON with task_id and session_id.
/// Poll GET /task/{id}/events for transcript-driven streaming.
pub(crate) async fn chat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "POST", "/chat", None, &headers, Some(&body)).await?;

    let req: ChatRequest =
        serde_json::from_slice(&body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let tags = crate::server::types::normalize_tags(req.tags.clone());

    // Validate attachments if present
    if let Some(ref attachments) = req.attachments {
        use crate::server::types::{
            ALLOWED_ATTACHMENT_MIMES, MAX_ATTACHMENTS, MAX_ATTACHMENT_SIZE,
        };
        use base64::Engine;

        if attachments.len() > MAX_ATTACHMENTS {
            let body = serde_json::json!({"error": format!("Too many attachments: {} (max {})", attachments.len(), MAX_ATTACHMENTS)});
            return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
        }
        for att in attachments {
            if !ALLOWED_ATTACHMENT_MIMES.contains(&att.mime_type.as_str()) {
                let body = serde_json::json!({"error": format!("Unsupported attachment type: {}. Allowed: {:?}", att.mime_type, ALLOWED_ATTACHMENT_MIMES)});
                return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body)
                    .await;
            }
            let decoded_size = base64::engine::general_purpose::STANDARD
                .decode(&att.data)
                .map(|d| d.len())
                .unwrap_or(0);
            if decoded_size == 0 {
                let body = serde_json::json!({"error": format!("Invalid base64 data for attachment: {}", att.filename)});
                return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body)
                    .await;
            }
            if decoded_size > MAX_ATTACHMENT_SIZE {
                let body = serde_json::json!({"error": format!("Attachment '{}' is {} bytes (max {} bytes)", att.filename, decoded_size, MAX_ATTACHMENT_SIZE)});
                return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body)
                    .await;
            }
        }
    }

    // Enforce cert-driven tag policy: if tags_required, reject tasks without tags
    if tags.is_empty() {
        let policy = crate::certificates::read_cert_tag_policy(state.wallet.clone()).await;
        if policy.tags_required == Some(true) {
            let body = serde_json::json!({"error": "Tags are required by certificate policy but none were provided"});
            return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
        }
    }

    let requester = auth_ctx
        .as_ref()
        .map(|ctx| ctx.client_identity_key.as_str())
        .unwrap_or("dev");
    let dedupe_key = req
        .client_command_id
        .as_ref()
        .map(|id| format!("{requester}:{id}"));

    let (task_id, session_id) = if let Some(key) = dedupe_key {
        loop {
            let mut should_spawn = false;
            let mut wait_for: Option<Arc<Notify>> = None;
            {
                let mut dedupe = state.task_mgr.chat_command_dedupe.lock().await;
                prune_chat_command_dedupe(&mut dedupe);
                match dedupe.get(&key) {
                    Some(entry) => match &entry.state {
                        ChatCommandDedupeState::Complete {
                            task_id,
                            session_id,
                        } => {
                            let response_body = serde_json::json!({
                                "task_id": task_id,
                                "session_id": session_id,
                            });
                            return signed_json_response(
                                &state,
                                auth_ctx,
                                StatusCode::OK,
                                &response_body,
                            )
                            .await;
                        }
                        ChatCommandDedupeState::Pending(notify) => {
                            wait_for = Some(Arc::clone(notify));
                        }
                    },
                    None => {
                        let notify = Arc::new(Notify::new());
                        dedupe.insert(
                            key.clone(),
                            ChatCommandDedupeEntry {
                                created_at: Instant::now(),
                                state: ChatCommandDedupeState::Pending(notify),
                            },
                        );
                        should_spawn = true;
                    }
                }
            }

            if let Some(notify) = wait_for {
                notify.notified().await;
                continue;
            }

            if should_spawn {
                let task_id = uuid::Uuid::new_v4().to_string();
                let spawn_result = spawn_task(
                    &state,
                    task_id,
                    req.message.clone(),
                    req.max_iterations,
                    req.session_id.clone(),
                    req.model.clone(),
                    None,
                    tags.clone(),
                    "chat".to_string(),
                    req.attachments.clone(),
                    None,
                )
                .await;

                let notify = {
                    let mut dedupe = state.task_mgr.chat_command_dedupe.lock().await;
                    match &spawn_result {
                        Ok((task_id, session_id)) => match dedupe.get_mut(&key) {
                            Some(entry) => match &entry.state {
                                ChatCommandDedupeState::Pending(notify) => {
                                    let notify = Arc::clone(notify);
                                    entry.created_at = Instant::now();
                                    entry.state = ChatCommandDedupeState::Complete {
                                        task_id: task_id.clone(),
                                        session_id: session_id.clone(),
                                    };
                                    Some(notify)
                                }
                                ChatCommandDedupeState::Complete { .. } => None,
                            },
                            None => None,
                        },
                        Err(_) => match dedupe.remove(&key) {
                            Some(entry) => match entry.state {
                                ChatCommandDedupeState::Pending(notify) => Some(notify),
                                ChatCommandDedupeState::Complete { .. } => None,
                            },
                            None => None,
                        },
                    }
                };

                if let Some(notify) = notify {
                    notify.notify_waiters();
                }

                match spawn_result {
                    Ok(ids) => break ids,
                    Err(StatusCode::FORBIDDEN) => {
                        let body = serde_json::json!({"error": "Agent certificate is revoked or missing — cannot accept tasks until parent issues a new certificate via POST /certificates/issue"});
                        return signed_json_response(
                            &state,
                            auth_ctx,
                            StatusCode::FORBIDDEN,
                            &body,
                        )
                        .await;
                    }
                    Err(status) => return Err(status),
                }
            }
        }
    } else {
        let task_id = uuid::Uuid::new_v4().to_string();
        match spawn_task(
            &state,
            task_id,
            req.message,
            req.max_iterations,
            req.session_id,
            req.model,
            None,
            tags,
            "chat".to_string(),
            req.attachments,
            None,
        )
        .await
        {
            Ok(ids) => ids,
            Err(StatusCode::FORBIDDEN) => {
                let body = serde_json::json!({"error": "Agent certificate is revoked or missing — cannot accept tasks until parent issues a new certificate via POST /certificates/issue"});
                return signed_json_response(&state, auth_ctx, StatusCode::FORBIDDEN, &body).await;
            }
            Err(status) => return Err(status),
        }
    };

    let response_body = serde_json::json!({
        "task_id": task_id,
        "session_id": session_id,
    });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &response_body).await
}

/// GET /chat/history -- return conversation messages from a task's transcript.
pub(crate) async fn chat_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HistoryParams>,
) -> Result<axum::response::Response, StatusCode> {
    // Build query string for BRC-31 verification
    let query_str = params.task_id.as_ref().map(|id| format!("task_id={id}"));
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/chat/history",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    // Find the task transcript path
    let task_id = if let Some(id) = params.task_id {
        id
    } else {
        // Return the most recent task's history
        let tasks = state.task_mgr.tasks.lock().await;
        tasks
            .values()
            .max_by_key(|t| &t.started_at)
            .map(|t| t.id.clone())
            .ok_or(StatusCode::NOT_FOUND)?
    };

    let transcript_path = state
        .workspace
        .join(format!("tasks/{task_id}/session.jsonl"));

    if !transcript_path.exists() {
        let empty: Vec<ChatMessage> = Vec::new();
        return signed_json_response(&state, auth_ctx, StatusCode::OK, &empty).await;
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let messages: Vec<ChatMessage> = transcript
        .replay()
        .iter()
        .filter_map(|event| match event.event_type.as_str() {
            "user" => {
                let content = event.data.get("content")?.as_str()?;
                Some(ChatMessage {
                    role: "user".into(),
                    content: content.to_string(),
                    tool_name: None,
                    call_id: None,
                })
            }
            "think_response" => {
                let content = event.data.get("content")?.as_str()?;
                if content.is_empty() {
                    return None;
                }
                Some(ChatMessage {
                    role: "assistant".into(),
                    content: content.to_string(),
                    tool_name: None,
                    call_id: None,
                })
            }
            "tool_result" => {
                let content = event.data.get("content")?.as_str()?;
                let name = event.data.get("name")?.as_str()?;
                let call_id = event.data.get("call_id")?.as_str()?;
                Some(ChatMessage {
                    role: "tool".into(),
                    content: content.to_string(),
                    tool_name: Some(name.to_string()),
                    call_id: Some(call_id.to_string()),
                })
            }
            "error" => {
                let error = event.data.get("error")?.as_str()?;
                Some(ChatMessage {
                    role: "error".into(),
                    content: error.to_string(),
                    tool_name: None,
                    call_id: None,
                })
            }
            _ => None,
        })
        .collect();

    signed_json_response(&state, auth_ctx, StatusCode::OK, &messages).await
}

/// POST /v1/chat/completions -- OpenAI-compatible chat endpoint.
///
/// Translates between OpenAI API format and the existing `/chat` flow.
/// Supports non-streaming only (streaming returns 501).
/// Uses BRC-31 auth (same as `/chat`).
pub(crate) async fn openai_chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<OpenAIChatRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "POST", "/v1/chat/completions", None, &headers, None).await?;

    if req.stream {
        // Streaming not yet implemented
        return signed_json_response(
            &state,
            auth_ctx,
            StatusCode::NOT_IMPLEMENTED,
            &serde_json::json!({
                "error": {
                    "message": "Streaming is not yet supported. Set stream: false.",
                    "type": "not_implemented",
                }
            }),
        )
        .await;
    }

    // Extract the last user message as the chat input
    let last_user_msg = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .ok_or(StatusCode::BAD_REQUEST)?;

    let session_id = req.user.clone();
    let max_iterations = req.max_tokens.unwrap_or(10).min(50);
    let model_override = req.model.clone();

    let task_id = uuid::Uuid::new_v4().to_string();

    // Subscribe to events BEFORE spawning so we don't miss any
    let mut rx = state.events_tx.subscribe();

    let (spawned_task_id, _session_id) = spawn_task(
        &state,
        task_id.clone(),
        last_user_msg.content.clone(),
        max_iterations,
        session_id,
        model_override.clone(),
        None,
        Vec::new(),
        "chat".to_string(),
        None,
        None,
    )
    .await?;

    // Wait for completion by listening for Done or Error event
    let mut response_text = String::new();
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(300);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok((tid, event))) if tid == spawned_task_id => match event {
                StepEvent::Response { text, .. } => {
                    response_text = text;
                }
                StepEvent::Done { result, .. } => {
                    if !result.is_empty() {
                        response_text = result;
                    }
                    break;
                }
                StepEvent::Error { message, .. } => {
                    response_text = format!("Error: {message}");
                    break;
                }
                _ => {}
            },
            Ok(Ok(_)) => {}      // different task
            Ok(Err(_)) => break, // channel closed
            Err(_) => break,     // timeout
        }
    }

    let model_name = model_override.unwrap_or_else(|| state.config.llm.default_model.clone());
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let resp = OpenAIChatResponse {
        id: format!("chatcmpl-{}", &task_id[..8]),
        object: "chat.completion".to_string(),
        created,
        model: model_name,
        choices: vec![OpenAIChoice {
            index: 0,
            message: OpenAIMessage {
                role: "assistant".to_string(),
                content: response_text,
            },
            finish_reason: "stop".to_string(),
        }],
        usage: OpenAIUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
    };

    signed_json_response(&state, auth_ctx, StatusCode::OK, &resp).await
}
