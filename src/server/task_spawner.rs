//! Task spawning — the single entry point for all task creation (chat, HTTP, heartbeat, WebSocket).
//!
//! Uses per-conversation semaphores (1 permit each) so tasks targeting the same
//! conversation queue in FIFO order instead of being rejected with 409.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::http::StatusCode;
use tokio::sync::Semaphore;

use crate::conversation::ConversationManager;
use crate::events::StepEvent;
use crate::runner;

use super::{AppState, TaskInfo, TaskStatus};

/// Spawn a new worm task as a background tokio task with conversation tracking.
///
/// Returns `(task_id, session_id)` immediately. Creates or resumes a conversation.
/// If the conversation already has a running task, this task is queued (status `Queued`)
/// and will start automatically when the previous task completes.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn spawn_task(
    state: &Arc<AppState>,
    task_id: String,
    message: String,
    max_iterations: u32,
    session_id: Option<String>,
    model_override: Option<String>,
    reply_to: Option<String>,
    tags: Vec<String>,
    origin: String,
    attachments: Option<Vec<super::types::ChatAttachment>>,
    delegation_envelope: Option<serde_json::Value>,
) -> Result<(String, String), StatusCode> {
    // Block tasks when certificate is revoked or missing
    if state
        .certificate_revoked
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        tracing::warn!("Task rejected: agent certificate is revoked or missing");
        return Err(StatusCode::FORBIDDEN);
    }

    // Detect /command invocations — expand the command but preserve the original
    // message for the conversation UI. The user sees "/greet Captain Ahab" in their
    // chat history; the LLM sees the expanded command body.
    let original_message = message.clone();
    let message = if message.starts_with('/') {
        let parts: Vec<&str> = message.splitn(2, char::is_whitespace).collect();
        let cmd_name = parts[0].trim_start_matches('/');
        let cmd_args = parts.get(1).map(|s| s.trim());

        let commands = crate::skills::commands::load_commands(&state.workspace);
        if let Some(cmd) = crate::skills::commands::find_command(&commands, cmd_name) {
            let rendered = crate::skills::commands::render_command(cmd, cmd_args);
            tracing::info!("Expanded command /{cmd_name} into task message");
            rendered
        } else {
            message // No matching command — use original message
        }
    } else {
        message
    };

    // Detect continuation tasks
    let (actual_message, continuation_context, continuation_session_id) =
        if message.starts_with("CONTINUATION:") {
            let first_line = message.lines().next().unwrap_or("");
            let cont_id = first_line
                .strip_prefix("CONTINUATION:")
                .unwrap_or("")
                .trim();

            let cont_path = state
                .workspace
                .join(format!("continuations/completed/{cont_id}.json"));
            let context = std::fs::read_to_string(&cont_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());

            let original_task = context
                .as_ref()
                .and_then(|c| c.get("task"))
                .and_then(|t| t.as_str())
                .unwrap_or(&message)
                .to_string();

            let cont_session = context
                .as_ref()
                .and_then(|c| c.get("conversation_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            (original_task, context, cont_session)
        } else {
            (message.clone(), None, None)
        };

    // The message shown in the conversation UI — preserves the original /command
    // syntax so the user sees what they typed, not the expanded command body.
    let display_message = if original_message.starts_with('/') && original_message != message {
        original_message.clone()
    } else {
        actual_message.clone()
    };

    // Use continuation's conversation_id if available
    let effective_session_id = session_id.or(continuation_session_id);

    // Build wakeup message for continuation tasks
    let wakeup_message = if let Some(ref ctx) = continuation_context {
        let iteration = ctx.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0);
        let reason = ctx
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unspecified");
        format!(
            "You are resuming a task after a pause.\n\
             Original task: {actual_message}\n\
             Paused at iteration: {iteration}\n\
             Reason for pause: {reason}\n\n\
             Continue from where you left off."
        )
    } else {
        actual_message.clone()
    };

    let conv_manager = ConversationManager::new(&state.workspace);

    // Resolve or create conversation — append user message immediately (visible in hash chain
    // even while queued). Prior messages are loaded later, after semaphore acquisition, so they
    // include results from the previous task.
    // Build attachment metadata for conversation storage (filenames + mime types, no base64)
    let attachment_meta: Option<Vec<serde_json::Value>> = attachments.as_ref().and_then(|atts| {
        if atts.is_empty() {
            None
        } else {
            Some(
                atts.iter()
                    .map(|a| {
                        serde_json::json!({
                            "filename": a.filename,
                            "mime_type": a.mime_type,
                        })
                    })
                    .collect(),
            )
        }
    });

    let (conv_id, resumed) = if let Some(ref sid) = effective_session_id {
        // Auto-create the session conversation if it doesn't exist yet.
        //
        // EPIC #329 Phase 3: commission-scoped conversations (session_id starts
        // with "commission-") use a distinctive participant_key so that
        // `find_by_participant(sender_identity_key)` NEVER matches them.
        // Commissions are one-shot contracts, not chat, and must never share
        // prior-message history with the sender's plain-chat conversations.
        if conv_manager.load(sid).ok().flatten().is_none() {
            let (participant_key, title) = if sid.starts_with("commission-") {
                let issuer = reply_to.as_deref().unwrap_or("unknown");
                (format!("commission:{issuer}"), format!("Commission: {sid}"))
            } else {
                ("system".to_string(), format!("System: {sid}"))
            };
            let _ = conv_manager.create_with_id(sid, &participant_key, &title);
        }
        // Append user message to hash chain — show original /command, not expanded body
        let _ = conv_manager.append_user_message_with_attachments(
            sid,
            &display_message,
            attachment_meta.clone(),
            Some(&task_id),
        );
        (sid.clone(), true)
    } else {
        // Create new conversation — participant key from wallet if available
        let participant_key = { state.wallet.get_identity_key().await.unwrap_or_default() };
        // If we have attachments, create an empty conversation first, then append the
        // user message with attachment metadata. Otherwise use the simple create() path.
        let conv = if attachment_meta.is_some() {
            let id = format!("conv-{}", uuid::Uuid::new_v4());
            let _ = conv_manager.create_with_id(
                &id,
                &participant_key,
                &crate::conversation::generate_title(&display_message),
            );
            let _ = conv_manager.append_user_message_with_attachments(
                &id,
                &display_message,
                attachment_meta.clone(),
                Some(&task_id),
            );
            conv_manager.load(&id).ok().flatten().unwrap_or_else(|| {
                crate::conversation::Conversation {
                    id: id.clone(),
                    participant_key: participant_key.clone(),
                    title: display_message.chars().take(60).collect(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    message_count: 1,
                    total_sats: 0,
                    task_ids: Vec::new(),
                    head_hash: String::new(),
                    compaction_summary: None,
                    compaction_seq: None,
                }
            })
        } else {
            conv_manager
                .create(&participant_key, &display_message)
                .unwrap_or_else(|_| crate::conversation::Conversation {
                    id: format!("conv-{}", uuid::Uuid::new_v4()),
                    participant_key,
                    title: display_message.chars().take(60).collect(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    message_count: 1,
                    total_sats: 0,
                    task_ids: Vec::new(),
                    head_hash: String::new(),
                    compaction_summary: None,
                    compaction_seq: None,
                })
        };
        (conv.id.clone(), false)
    };

    // Get or create per-conversation semaphore (1 permit = 1 task at a time)
    let semaphore = {
        let mut sems = state.task_mgr.conversation_semaphores.lock().await;
        sems.entry(conv_id.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    };

    // Determine initial status: Running if permit is immediately available, Queued otherwise
    let initial_status = if semaphore.available_permits() > 0 {
        TaskStatus::Running
    } else {
        TaskStatus::Queued
    };

    let info = TaskInfo {
        id: task_id.clone(),
        task: actual_message.clone(),
        status: initial_status,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags,
        origin,
        conversation_id: Some(conv_id.clone()),
    };

    let cancel_flag = Arc::new(AtomicBool::new(false));

    {
        let mut tasks = state.task_mgr.tasks.lock().await;
        tasks.insert(task_id.clone(), info);
    }
    {
        let mut flags = state.task_mgr.cancel_flags.lock().await;
        flags.insert(task_id.clone(), Arc::clone(&cancel_flag));
    }
    {
        let mut sessions = state.task_mgr.task_sessions.lock().await;
        sessions.insert(task_id.clone(), conv_id.clone());
    }

    let mut config = state.config.clone();
    // Apply per-request model override — resolve_endpoint() handles provider auto-detection
    if let Some(ref model) = model_override {
        config.llm.default_model = model.clone();
    }
    let workspace = state.workspace.join(format!("tasks/{task_id}"));
    let workspace_for_uploads = workspace.clone();
    let state_ref = Arc::clone(state);
    let events_tx = state.events_tx.clone();
    let task_id_clone = task_id.clone();
    let conv_id_clone = conv_id.clone();
    let active_task_count = Arc::clone(&state.task_mgr.active_task_count);

    let handle = tokio::spawn(async move {
        // Acquire conversation permit — waits if another task is running on this conversation.
        // Tokio semaphore guarantees FIFO ordering among waiters.
        let _permit = semaphore
            .acquire()
            .await
            .expect("conversation semaphore closed");

        // Check if cancelled while queued
        if cancel_flag.load(Ordering::Relaxed) {
            let mut tasks = state_ref.task_mgr.tasks.lock().await;
            if let Some(info) = tasks.get_mut(&task_id_clone) {
                info.status = TaskStatus::Cancelled;
                info.error = Some("Cancelled while queued".to_string());
                info.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
            drop(tasks);
            let mut flags = state_ref.task_mgr.cancel_flags.lock().await;
            flags.remove(&task_id_clone);
            // _permit drops here, next queued task proceeds
            return;
        }

        // Transition to Running and increment active task count
        {
            let mut tasks = state_ref.task_mgr.tasks.lock().await;
            if let Some(info) = tasks.get_mut(&task_id_clone) {
                info.status = TaskStatus::Running;
            }
        }
        active_task_count.fetch_add(1, Ordering::Relaxed);

        // Emit SessionStarted now that we have the permit
        let _ = events_tx.send((
            task_id_clone.clone(),
            StepEvent::SessionStarted {
                session_id: conv_id_clone.clone(),
                task_id: task_id_clone.clone(),
                resumed,
            },
        ));

        // Load fresh prior messages AFTER acquiring permit — includes previous task's results
        let conv_mgr = ConversationManager::new(&state_ref.workspace);
        let prior_messages = conv_mgr
            .to_openai_messages(&conv_id_clone)
            .unwrap_or_default();

        let global_memory_dir = state_ref.workspace.join("memory");
        let global_workspace = state_ref.workspace.clone();
        let mut worm = runner::create_loop_with_rate_limiter(
            config,
            workspace,
            Some(global_memory_dir),
            Some(global_workspace),
            state_ref.wallet.clone(),
            Some(state_ref.rate_limiter.clone()),
            state_ref.circuit_breakers.clone(),
            Some(state_ref.messagebox.clone()),
        );

        // Register MCP wallet tools if available
        if let Some(ref mcp_client) = state_ref.wallet_mcp {
            let defs = crate::mcp::client::mcp_tools_to_tooldefs(
                mcp_client.clone(),
                // Re-fetch tools from stored summaries
                state_ref
                    .wallet_mcp_tools
                    .iter()
                    .map(|(name, desc, params)| rmcp::model::Tool {
                        name: std::borrow::Cow::Owned(name.clone()),
                        title: None,
                        description: Some(std::borrow::Cow::Owned(desc.clone())),
                        input_schema: std::sync::Arc::new(
                            params.as_object().cloned().unwrap_or_default(),
                        ),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    })
                    .collect(),
                &["wallet_balance"],
            );
            worm.register_mcp_tools(defs).await;
        }

        // Wire up staged-transactions map for tool approval workflows
        worm.set_approval_tx(state_ref.staged_transactions.clone());

        // Bind the runner to its conversation so lifecycle events can
        // surface activity (e.g. commission payment claims) directly into
        // the user-facing conversation timeline. EPIC #329 Phase 3 follow-up.
        worm.set_current_conversation_id(conv_id_clone.clone());

        // Wire up Prometheus metrics registry
        worm.metrics = Some(state_ref.metrics.clone());

        // Apply discovered model capabilities (if any) to override hardcoded context limits.
        // Resolution chain: discovered → hardcoded → config cap.
        {
            let caps = state_ref.model_capabilities.read().await;
            if !caps.is_empty() {
                worm.apply_model_capabilities(&caps);
            }
        }

        // Record continuation resume event if applicable
        if let Some(ref ctx) = continuation_context {
            let cont_id = ctx.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
            let paused_iter = ctx.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            worm.transcript
                .record_continuation_resume(cont_id, &actual_message, paused_iter);
            // Restore proof chain linkage from paused session
            if let Some(hash) = ctx.get("last_proof_hash").and_then(|v| v.as_str()) {
                worm.state.onchain.last_proof_hash = Some(hash.to_string());
            }
        }

        // R.3: Verify BRC-60 hash chain at conversation-bound task start.
        // If prior messages exist, the conversation was resumed — verify its integrity.
        if !prior_messages.is_empty() {
            match conv_mgr.verify_chain(&conv_id_clone) {
                Ok(verification) => {
                    if verification.valid {
                        worm.set_conversation_chain_status(true, None);
                        tracing::debug!(
                            "BRC-60 chain verified for conversation {}: {} messages",
                            conv_id_clone,
                            verification.message_count
                        );
                    } else {
                        // Extract details about the first break for the proof
                        let first_break = verification.messages.iter().find(|m| !m.valid);
                        let chain_break = first_break.map(|fb| runner::ConversationChainBreak {
                            conversation_id: conv_id_clone.clone(),
                            first_break_seq: fb.seq,
                            expected_hash: fb.expected_hash.clone(),
                            actual_hash: fb.hash.clone(),
                            total_breaks: verification.breaks.len(),
                        });
                        worm.set_conversation_chain_status(false, chain_break);
                        tracing::warn!(
                            "BRC-60 chain break in conversation {}: {} break(s) across {} messages",
                            conv_id_clone,
                            verification.breaks.len(),
                            verification.message_count
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to verify BRC-60 chain for conversation {}: {e}",
                        conv_id_clone
                    );
                    // Don't block the task — verification failure is non-fatal
                }
            }

            worm.set_prior_messages(prior_messages);
        }

        // Set user attachments for multimodal content (images sent with the message).
        // Convert ChatAttachment → OpenAI-format content blocks for LLM injection.
        if let Some(ref atts) = attachments {
            if !atts.is_empty() {
                let blocks: Vec<serde_json::Value> = atts
                    .iter()
                    .map(|att| {
                        serde_json::json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", att.mime_type, att.data)
                            }
                        })
                    })
                    .collect();
                worm.set_user_attachments(blocks);

                // Save attachment files to workspace for audit trail + write artifacts.json
                let uploads_dir = workspace_for_uploads.join("uploads");
                let _ = std::fs::create_dir_all(&uploads_dir);
                let mut artifact_entries: Vec<serde_json::Value> = Vec::new();
                for att in atts {
                    if let Ok(bytes) = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &att.data,
                    ) {
                        let _ = std::fs::write(uploads_dir.join(&att.filename), &bytes);
                        let ext = std::path::Path::new(&att.filename)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        let atype = match ext.as_str() {
                            "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" => "image",
                            "pdf" => "document",
                            _ => "file",
                        };
                        artifact_entries.push(serde_json::json!({
                            "name": att.filename,
                            "type": atype,
                            "size_bytes": bytes.len(),
                            "created_by": "user_upload",
                            "created_at": std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64(),
                        }));
                    }
                }
                if !artifact_entries.is_empty() {
                    let manifest_path = workspace_for_uploads.join("artifacts.json");
                    let _ = std::fs::write(
                        &manifest_path,
                        serde_json::to_string_pretty(&artifact_entries).unwrap_or_default(),
                    );
                }
            }
        }

        // Layer 3: Mark tasks from external messages (POST /message) as external-origin.
        // set_external_origin() checks if the sender is the parent and skips restriction if so.
        if let Some(ref sender_key) = reply_to {
            worm.set_external_origin(sender_key);
        }

        // Phase 3 of EPIC #329: inbound task_delegation envelopes carry a
        // signed BRC-52 delegation cert. Stash it on the runner so
        // `setup_task()` can verify it and apply caveats before the loop runs.
        if let Some(envelope) = delegation_envelope {
            worm.set_pending_delegation(envelope);
        }

        worm.run(
            &wakeup_message,
            max_iterations,
            Some((&events_tx, &task_id_clone)),
            cancel_flag.clone(),
        )
        .await;

        // Append transcript messages to conversation
        let conv_mgr = ConversationManager::new(&state_ref.workspace);
        if let Err(e) =
            conv_mgr.append_from_transcript(&conv_id_clone, &worm.transcript, &task_id_clone)
        {
            tracing::error!(
                "Failed to assemble conversation {} from task {}: {e}",
                conv_id_clone,
                task_id_clone
            );
        }
        if let Err(e) = conv_mgr.update_meta(
            &conv_id_clone,
            worm.budget_tracker.task_sats(),
            &task_id_clone,
        ) {
            tracing::error!(
                "Failed to update conversation {} metadata: {e}",
                conv_id_clone
            );
        }

        // -- BRC-18 Conversation integrity proof (best-effort) --
        if let Ok(Some(conv)) = conv_mgr.load(&conv_id_clone) {
            let head_hash = &conv.head_hash;
            let msg_count = conv.message_count as usize;
            let commitment = crate::proofs::conversation_integrity_proof(
                &conv_id_clone,
                head_hash,
                msg_count,
                worm.state.onchain.last_proof_hash.as_deref(),
            );
            match crate::proofs::create_proof(state_ref.wallet.as_ref(), commitment).await {
                Ok(proof_result) => {
                    let hash_hex = proof_result.commitment.hash_hex();
                    tracing::info!(
                        "BRC-18 conversation integrity proof: conv={}, txid={}",
                        conv_id_clone,
                        proof_result.txid
                    );
                    worm.transcript.record_proof_created(
                        &proof_result.txid,
                        "conversation_integrity",
                        &hash_hex,
                        Some(&proof_result.commitment.data),
                        Some(&proof_result.commitment.timestamp),
                        Some(200),
                        Some(worm.state.exec.iteration),
                        Some("dm-proofs"),
                        proof_result.commitment.prev_hash.as_deref(),
                    );
                    worm.state.onchain.last_proof_hash = Some(hash_hex);
                    worm.budget_tracker.record(
                        "proofs",
                        "brc18_conversation_integrity",
                        200,
                        serde_json::json!({"txid": proof_result.txid, "conversation": conv_id_clone}),
                    );
                }
                Err(e) => tracing::warn!("Failed to create conversation integrity proof: {e}"),
            }

            // Auto-sync conversation to wallet (best-effort)
            match conv_mgr
                .sync_to_wallet(state_ref.wallet.as_ref(), &conv_id_clone)
                .await
            {
                Ok(r) => tracing::info!(
                    "Auto-synced conversation {}: txid={}",
                    conv_id_clone,
                    r.txid
                ),
                Err(e) => tracing::warn!("Failed to auto-sync conversation {}: {e}", conv_id_clone),
            }
        }

        // Merge the runner's per-task budget entries into the server's global
        // BudgetTracker so that GET /budget and GET /budget/detail reflect
        // tool, x402, proof, and LLM costs immediately (not just after restart).
        {
            let runner_entries = worm.budget_tracker.entries().to_vec();
            let task_sats = worm.budget_tracker.task_sats();
            let mut global_budget = state_ref.budget.lock().await;
            global_budget.merge_from(&runner_entries);
            global_budget.set_last_task_sats(task_sats);
        }

        // Merge the runner's skill telemetry into the global accumulator.
        {
            let runner_telemetry = worm.skills.telemetry();
            let mut global_telemetry = state_ref.skill_telemetry.lock().await;
            for (name, stats) in &runner_telemetry.activations {
                let global_stats = global_telemetry
                    .activations
                    .entry(name.clone())
                    .or_default();
                global_stats.count += stats.count;
                if stats.last_activated > global_stats.last_activated {
                    global_stats
                        .last_activated
                        .clone_from(&stats.last_activated);
                }
                for ctx in &stats.contexts {
                    global_stats.contexts.push(ctx.clone());
                    if global_stats.contexts.len() > 10 {
                        global_stats.contexts.remove(0);
                    }
                }
            }
        }

        // Use budget_tracker.task_sats() as the authoritative sats count
        // (includes LLM, proofs, tokens — everything recorded in the budget tracker).
        let final_sats_spent = worm.budget_tracker.task_sats();
        let mut tasks = state_ref.task_mgr.tasks.lock().await;
        if let Some(info) = tasks.get_mut(&task_id_clone) {
            info.iterations = worm.state.exec.iteration;
            info.sats_spent = final_sats_spent;
            info.completed_at = Some(chrono::Utc::now().to_rfc3339());
            info.proof_txids = worm.proof_txids();
            if cancel_flag.load(Ordering::Relaxed) {
                info.status = TaskStatus::Cancelled;
                info.error = Some("Cancelled by user".to_string());
            } else if worm.state.exec.error.is_empty() {
                info.status = TaskStatus::Complete;
                info.result = Some(worm.state.exec.result.clone());
            } else {
                info.status = TaskStatus::Error;
                info.error = Some(worm.state.exec.error.clone());
            }
        }
        drop(tasks);

        // Update lifetime counters
        state_ref
            .stats
            .sats_spent
            .fetch_add(final_sats_spent, Ordering::Relaxed);
        state_ref.stats.task_count.fetch_add(1, Ordering::Relaxed);

        // Record Prometheus task completion metrics
        {
            let status_label = if cancel_flag.load(Ordering::Relaxed) {
                "cancelled"
            } else if worm.state.exec.error.is_empty() {
                "complete"
            } else {
                "error"
            };
            state_ref
                .metrics
                .tasks_total
                .with_label_values(&[status_label])
                .inc();
        }

        {
            let mut flags = state_ref.task_mgr.cancel_flags.lock().await;
            flags.remove(&task_id_clone);
        }

        // Reply routing: if this task originated from a MessageBox message,
        // send the result back to the sender. Best-effort — errors are logged
        // but do not fail the task. Skip self-replies to prevent feedback loops.
        if let Some(ref reply_key) = reply_to {
            if reply_key == &state_ref.auth.server_identity_key {
                tracing::info!("Skipping reply routing to self (would create feedback loop)");
            } else {
                let result_text = if worm.state.exec.error.is_empty() {
                    worm.state.exec.result.clone()
                } else {
                    format!("Task failed: {}", worm.state.exec.error)
                };

                let body = serde_json::json!({
                    "type": "task_result",
                    "task_id": task_id_clone,
                    "status": if worm.state.exec.error.is_empty() { "completed" } else { "failed" },
                    "result": result_text,
                });

                match state_ref
                    .messagebox
                    .send_message(
                        reply_key,
                        crate::messagebox::types::BOX_RESULTS_INBOX,
                        &body,
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Reply sent to {}... via MessageBox",
                            &reply_key[..reply_key.len().min(16)]
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to send reply to {}...: {e} — queuing for retry",
                            &reply_key[..reply_key.len().min(16)]
                        );
                        // Enqueue for retry with exponential backoff
                        let queue = crate::delivery::DeliveryQueue::new(&state_ref.workspace);
                        if let Err(qe) = queue.enqueue(
                            reply_key,
                            crate::messagebox::types::BOX_RESULTS_INBOX,
                            body,
                        ) {
                            tracing::error!("Failed to enqueue delivery for retry: {qe}");
                        }
                    }
                }
            }
        }

        // Decrement active task count now that this task is done
        active_task_count.fetch_sub(1, Ordering::Relaxed);

        // Emit system event so the scheduler wakes immediately (e.g. to pick up queued work)
        if let Some(ref tx) = state_ref.scheduler.system_event_tx {
            let status = if cancel_flag.load(Ordering::Relaxed) {
                "cancelled"
            } else if worm.state.exec.error.is_empty() {
                "completed"
            } else {
                "failed"
            };
            let _ = tx.try_send(crate::heartbeat::SystemEvent::TaskCompleted {
                task_id: task_id_clone.clone(),
                status: status.to_string(),
                sats_spent: final_sats_spent,
            });
        }

        // _permit drops here — next queued task on this conversation wakes automatically
    });

    // Supervisor: if the spawned task panics, catch the JoinError and update task status
    // so it doesn't stay "Running" forever. The semaphore permit is dropped on panic
    // (Rust drop semantics), so the next queued task proceeds automatically.
    {
        let state_panic = Arc::clone(state);
        let task_id_panic = task_id.clone();
        let active_panic = Arc::clone(&state.task_mgr.active_task_count);
        tokio::spawn(async move {
            if let Err(join_err) = handle.await {
                tracing::error!("Task {} panicked: {}", task_id_panic, join_err);

                // Update task status to Error
                {
                    let mut tasks = state_panic.task_mgr.tasks.lock().await;
                    if let Some(info) = tasks.get_mut(&task_id_panic) {
                        if matches!(info.status, TaskStatus::Running | TaskStatus::Queued) {
                            info.status = TaskStatus::Error;
                            info.error = Some(format!("Task panicked: {}", join_err));
                            info.completed_at = Some(chrono::Utc::now().to_rfc3339());
                        }
                    }
                }

                // Best-effort: assemble conversation from whatever transcript exists.
                // The task may have produced useful output before panicking.
                {
                    let session_id = state_panic
                        .task_mgr
                        .task_sessions
                        .lock()
                        .await
                        .get(&task_id_panic)
                        .cloned();
                    if let Some(sid) = session_id {
                        let tp = state_panic
                            .workspace
                            .join(format!("tasks/{}/session.jsonl", task_id_panic));
                        if tp.exists() {
                            let conv_mgr = ConversationManager::new(&state_panic.workspace);
                            let transcript = crate::transcript::Transcript::new(tp);
                            if let Err(e) =
                                conv_mgr.append_from_transcript(&sid, &transcript, &task_id_panic)
                            {
                                tracing::warn!(
                                    "Panic recovery: conversation assembly failed for {sid}: {e}"
                                );
                            }
                        }
                    }
                }

                // Decrement active task count (only if it was Running, not Queued)
                // Conservative: always decrement since we can't easily tell.
                // At worst, count briefly goes to 0 when it should be 0 anyway.
                active_panic.fetch_sub(1, Ordering::Relaxed);
            }
            // If Ok(_), the task completed normally and already handled cleanup
        });
    }

    Ok((task_id, conv_id))
}
