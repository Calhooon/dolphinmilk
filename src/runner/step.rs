//! Per-iteration phases — OBSERVE → BUILD → THINK → ACT → RECORD.
//!
//! Each phase is a method on `DmLoop`. The `step()` method orchestrates
//! them into a single iteration of the agent loop.
//!
//! Tool execution logic is in `execute.rs`, and the tool approval gate
//! is in `approval.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::context::prompt::build_system_prompt;
use crate::error::DmError;
use crate::events::StepEvent;

use crate::proofs;
use crate::sanitize;
use crate::state;
use crate::think;

use super::{content_hash, emit_event, DmLoop, ReplyObligation, MAX_NUDGES};

/// Stable context passed through all step methods to avoid repeating
/// `tx: Option<(&Sender, &str)>` and `cancel: &AtomicBool` parameters.
pub(crate) struct StepContext {
    /// Broadcast channel for SSE events (server mode). `None` in CLI mode.
    pub events: Option<broadcast::Sender<(String, StepEvent)>>,
    /// Task ID string for event routing.
    pub task_id: String,
    /// Cancellation flag shared with the server's cancel mechanism.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl DmLoop {
    // =============================================================================
    // OBSERVE — poll inbox, partition messages, build obligations
    // =============================================================================

    /// OBSERVE phase: poll inbox, partition messages, build obligations.
    /// Returns (has_external_messages).
    async fn observe(&mut self) -> bool {
        let identity_key = self.get_identity_key().await;
        let (balance, spendable_count) =
            self.wallet.get_balance_and_count().await.unwrap_or((0, 0));
        self.state.budget.cached_balance = balance;
        self.state.budget.cached_spendable_count = spendable_count;

        // Poll MessageBox inboxes (best-effort, non-blocking)
        let all_polled_messages = match self.messagebox.poll_inboxes().await {
            Ok(msgs) => {
                if !msgs.is_empty() {
                    tracing::info!("Inbox: {} message(s) received", msgs.len());
                }
                msgs
            }
            Err(e) => {
                tracing::debug!("Inbox poll skipped: {e}");
                Vec::new()
            }
        };

        // Partition polled messages into:
        //   - task_inbox messages (actionable — will be processed + injected if external)
        //   - non-task_inbox metadata messages (status/results/coordination — ACK only,
        //     NEVER inject into an active task loop. Heartbeat handles cross-task routing.)
        //
        // EPIC #329 Phase 3: without this split, stale status_inbox replies from prior
        // test runs or unrelated peer status updates were being injected into the
        // current task's LLM context, derailing one-shot commissions. Matches
        // heartbeat/sources.rs::poll_messagebox() routing semantics.
        use crate::messagebox::types::BOX_TASK_INBOX;
        let (task_inbox_messages, metadata_messages): (Vec<_>, Vec<_>) = all_polled_messages
            .into_iter()
            .partition(|m| m.message_box.as_deref() == Some(BOX_TASK_INBOX));

        if !metadata_messages.is_empty() {
            let boxes: Vec<&str> = metadata_messages
                .iter()
                .filter_map(|m| m.message_box.as_deref())
                .collect();
            tracing::info!(
                "Inbox: skipping {} metadata message(s) from non-task inboxes {:?} \
                 (logged + acked, NOT injected into task context)",
                metadata_messages.len(),
                boxes
            );
        }

        // EPIC #329 Phase 3 §C.3: server-side commission payment processing.
        //
        // Both `commission_payment_claim` (Captain side) and
        // `commission_payment_sent` (Coral side) are pure server-side flows
        // and must NEVER be injected into the active task's LLM context —
        // otherwise the LLM derails into replying to them. Process them
        // inline here BEFORE the external/self partition; the heartbeat
        // handler also intercepts them via heartbeat/sources.rs::poll_messagebox
        // for cases when the agent is idle (no active task running).
        let (commission_msgs, task_inbox_messages): (Vec<_>, Vec<_>) =
            task_inbox_messages.into_iter().partition(|m| {
                let body_type = m.body.get("type").and_then(|v| v.as_str()).unwrap_or("");
                body_type == "commission_payment_claim" || body_type == "commission_payment_sent"
            });
        for m in &commission_msgs {
            let body_type = m.body.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match body_type {
                "commission_payment_claim" => {
                    let result =
                        crate::heartbeat::commission_payments::handle_commission_payment_claim(
                            self.wallet.clone(),
                            self.messagebox.clone(),
                            &m.body,
                            &m.sender,
                            Some(self.global_workspace.as_path()),
                        )
                        .await;
                    if result.success {
                        tracing::info!(
                            sender = %&m.sender[..16.min(m.sender.len())],
                            sats = result.sats_paid,
                            txid = ?result.txid,
                            "observe(): commission_payment_claim handled and paid"
                        );
                    } else {
                        tracing::warn!(
                            sender = %&m.sender[..16.min(m.sender.len())],
                            "observe(): commission_payment_claim REJECTED: {}",
                            result.reason
                        );
                    }
                }
                "commission_payment_sent" => {
                    let result =
                        crate::heartbeat::commission_payments::handle_commission_payment_sent(
                            self.wallet.clone(),
                            &m.body,
                            Some(self.global_workspace.as_path()),
                        )
                        .await;
                    if result.success {
                        tracing::info!(
                            sender = %&m.sender[..16.min(m.sender.len())],
                            sats = result.sats_paid,
                            txid = ?result.txid,
                            "observe(): commission_payment_sent internalized"
                        );
                    } else {
                        tracing::warn!(
                            sender = %&m.sender[..16.min(m.sender.len())],
                            "observe(): commission_payment_sent FAILED to internalize: {}",
                            result.reason
                        );
                    }
                }
                _ => {}
            }
        }

        // Partition task_inbox messages into external vs self-sent.
        let (inbox_messages, self_messages): (Vec<_>, Vec<_>) = task_inbox_messages
            .into_iter()
            .partition(|m| m.sender != identity_key);

        // Acknowledge ALL messages (external task_inbox + self task_inbox + metadata
        // from other inboxes + commission payment messages handled inline above)
        // so they don't pile up on the MessageBox server. Non-task inboxes are NOT
        // injected but MUST be ACKed to prevent stale state from re-polluting future
        // runs.
        let all_message_ids: Vec<String> = inbox_messages
            .iter()
            .chain(self_messages.iter())
            .chain(metadata_messages.iter())
            .chain(commission_msgs.iter())
            .map(|m| m.message_id.clone())
            .collect();
        if !all_message_ids.is_empty() {
            match self.messagebox.acknowledge_message(&all_message_ids).await {
                Ok(()) => tracing::info!("Acknowledged {} inbox messages", all_message_ids.len()),
                Err(e) => tracing::warn!("Failed to acknowledge messages: {e}"),
            }
        }

        // Log self-delivered messages as confirmations (informational, not actionable)
        if !self_messages.is_empty() {
            tracing::info!(
                "Inbox: {} self-delivered message(s) acknowledged (not injected as incoming)",
                self_messages.len()
            );
            self.transcript.record_system(&format!(
                "DELIVERY CONFIRMED: {} message(s) you sent to yourself arrived and were acknowledged.",
                self_messages.len()
            ));
        }

        // Inject external inbox messages into context — with auto-decryption,
        // signature verification, sanitization, boundary markers, and moderation.
        // (#266) Messages are processed via process_received_message() to auto-decrypt
        // BRC-78 encrypted bodies and auto-verify BRC-77 signatures before injection.
        if !inbox_messages.is_empty() {
            let parent_key = self.config.parent.identity_key.clone();
            let mut inbox_summary: Vec<String> = Vec::new();

            for m in &inbox_messages {
                // Auto-decrypt BRC-78 and auto-verify BRC-77 signatures
                let (body_value, crypto_meta) = match self
                    .messagebox
                    .process_received_message(m)
                    .await
                {
                    Ok(processed) => {
                        let mut meta_parts = Vec::new();
                        if processed.was_encrypted {
                            meta_parts.push("decrypted".to_string());
                        }
                        if processed.was_signed {
                            let valid = processed.signature_valid.unwrap_or(false);
                            meta_parts.push(format!("signature_valid:{valid}"));
                        }
                        let meta = if meta_parts.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", meta_parts.join(", "))
                        };
                        (processed.body, meta)
                    }
                    Err(e) => {
                        tracing::warn!(
                            sender = %m.sender,
                            box_name = ?m.message_box,
                            body_keys = ?m.body.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                            "Observer auto-decrypt failed (using raw body): {e}"
                        );
                        (m.body.clone(), String::new())
                    }
                };

                let raw_body = serde_json::to_string(&body_value).unwrap_or_default();
                let is_parent = sanitize::is_parent_sender(&m.sender, &parent_key);
                let body_display = if is_parent {
                    raw_body.chars().take(500).collect::<String>()
                } else {
                    let sanitized = sanitize::sanitize_external_content(&raw_body);
                    sanitize::wrap_with_boundary(&sanitized)
                };

                // Moderate user message content
                use super::moderation::ModerationOutcome;
                let desc = format!("inbox message from {}", &m.sender);
                match self.moderate_content(&body_display, "user_message", None, &desc) {
                    ModerationOutcome::Blocked => continue,
                    ModerationOutcome::Flagged | ModerationOutcome::Pass => {
                        inbox_summary.push(format!(
                            "[{}] from {}{}:\n{}",
                            m.message_box.as_deref().unwrap_or("unknown"),
                            &m.sender,
                            crypto_meta,
                            body_display
                        ));
                    }
                }
            }

            if !inbox_summary.is_empty() {
                self.transcript.record_user(&format!(
                    "INBOX ({} messages):\n{}",
                    inbox_summary.len(),
                    inbox_summary.join("\n")
                ));
            }
        }

        // Build reply obligations from external inbox messages.
        self.state.comms.pending_replies.clear();
        for msg in &inbox_messages {
            if !self
                .state
                .comms
                .pending_replies
                .iter()
                .any(|r| r.sender_key == msg.sender)
            {
                self.state.comms.pending_replies.push(ReplyObligation {
                    sender_key: msg.sender.clone(),
                    message_box: "status_inbox".to_string(),
                });
            }
        }

        // BRC-18 MessageReceive proofs — best-effort, never fails the iteration.
        // Create a proof for each external inbox message received this iteration.
        for msg in &inbox_messages {
            let body_str = serde_json::to_string(&msg.body).unwrap_or_default();
            let msg_hash = content_hash(&body_str);
            let box_name = msg.message_box.as_deref().unwrap_or("unknown");
            let commitment = proofs::message_receive_proof(
                &msg_hash,
                &msg.sender,
                box_name,
                self.state.onchain.last_proof_hash.as_deref(),
            );
            self.record_proof(commitment, "message_receive", None).await;

            // Fire OnMessageReceived hook (notification-only)
            let _ = self
                .hook_registry
                .fire(crate::hooks::HookEvent::OnMessageReceived {
                    sender_key: msg.sender.clone(),
                    message_box: box_name.to_string(),
                })
                .await;
        }

        // Determine if we have external (untrusted) messages.
        self.state.auth.external_origin || {
            let parent_key = &self.config.parent.identity_key;
            inbox_messages
                .iter()
                .any(|m| !sanitize::is_parent_sender(&m.sender, parent_key))
        }
    }

    // =============================================================================
    // BUILD — construct system prompt and LLM messages with compaction
    // =============================================================================

    /// BUILD phase: construct system prompt and LLM messages with compaction.
    async fn build_llm_messages(&mut self, has_external: bool) -> Vec<Value> {
        let identity_key = self.get_identity_key().await;
        let inbox_count = self.state.comms.pending_replies.len();
        let prompt_ctx = self
            .build_prompt_context(&identity_key, inbox_count, has_external)
            .await;
        let system_prompt = build_system_prompt(&prompt_ctx);

        // Record skill telemetry for auto-activated skills injected into the prompt
        if !prompt_ctx.skills_section.is_empty() {
            let active_names: Vec<String> = self
                .skills
                .auto_activated()
                .iter()
                .filter(|s| !(s.name.to_lowercase() == "messaging" && inbox_count == 0))
                .map(|s| s.name.clone())
                .collect();
            for name in &active_names {
                self.skills.record_activation(name, "auto");
                self.transcript.record_skill_activated(name, "auto");
            }
        }

        let mut history = Vec::new();
        if let Some(ref prior) = self.prior_messages {
            history.extend(prior.iter().cloned());
        }
        history.extend(self.transcript.to_messages());

        // Inject user attachment content blocks (multimodal images) into the first user message.
        // Only the CURRENT task's user message gets attachments — prior conversation turns are text-only.
        // This keeps context usage bounded: images are sent once, not replayed on every multi-turn follow-up.
        if let Some(ref attachment_blocks) = self.user_attachment_blocks {
            // Find the LAST user message in the transcript portion (not prior_messages)
            // and replace its string content with a content array including image blocks.
            let prior_len = self.prior_messages.as_ref().map_or(0, |p| p.len());
            if let Some(user_msg) = history[prior_len..]
                .iter_mut()
                .rev()
                .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
            {
                let text = user_msg
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut content_array = vec![serde_json::json!({"type": "text", "text": text})];
                content_array.extend(attachment_blocks.iter().cloned());
                user_msg["content"] = serde_json::json!(content_array);
            }
        }

        // Substitute offloaded tool result previews in history
        if !self.state.storage.offloaded_results.is_empty() {
            for msg in &mut history {
                if msg.get("role").and_then(|v| v.as_str()) == Some("tool") {
                    if let Some(call_id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                        if let Some((_, preview)) =
                            self.state.storage.offloaded_results.get(call_id)
                        {
                            msg["content"] = serde_json::Value::String(preview.clone());
                        }
                    }
                }
            }
        }

        // Microcompact: zero-cost cleanup of old tool results before any LLM compaction.
        // This is Tier 1 — saves tokens (and sats) without an LLM call.
        {
            let keep_recent = crate::context::manager::ContextManager::MICROCOMPACT_KEEP_RECENT;
            // Estimate time gap: use iteration count as a rough proxy
            // (real time tracking would require timestamp on each message).
            let time_gap = if self.state.exec.iteration > 20 {
                Some(90u64) // likely >60 min for long-running tasks
            } else {
                None
            };
            let cleared = self
                .context
                .microcompact(&mut history, keep_recent, time_gap);
            if cleared > 0 {
                self.transcript.record_system(&format!(
                    "Microcompact: cleared {} stale tool results",
                    cleared
                ));
            }
        }

        // Two-phase compaction check with PreCompact/PostCompact hook events.
        if self.config.llm.compaction_enabled
            && self.context.needs_compaction(history.len())
            && self.context.compaction_summary().is_none()
        {
            let drop_count = history.len() - self.context.max_history_turns();
            let original_count = history.len();
            let token_estimate: usize = history
                .iter()
                .take(drop_count)
                .map(crate::context::manager::estimate_message_tokens)
                .sum();

            // Fire PreCompact hook — blocking hooks can prevent compaction
            let pre_results = self
                .hook_registry
                .fire(crate::hooks::HookEvent::PreCompact {
                    message_count: drop_count,
                    token_estimate,
                })
                .await;
            let blocked = crate::hooks::is_blocked(&pre_results);

            if blocked.is_none() {
                let to_drop = &history[..drop_count];
                let compaction_model = self
                    .config
                    .llm
                    .compaction_model
                    .as_deref()
                    .unwrap_or(&self.config.llm.default_model);

                // Budget-aware: skip LLM compaction when budget is critical (>90% spent).
                // Fall back to microcompact-only (already ran above).
                let task_budget_limit = self.budget_tracker.limits().max_per_task;
                let budget_critical = if task_budget_limit > 0 {
                    (self.state.budget.sats_spent as f64 / task_budget_limit as f64) > 0.9
                } else {
                    false
                };

                if budget_critical {
                    tracing::info!(
                        "Skipping LLM compaction: budget critical ({}% spent)",
                        (self.state.budget.sats_spent * 100)
                            .checked_div(task_budget_limit)
                            .unwrap_or(0) as u32
                    );
                } else {
                    let conv_id = self
                        .prior_messages
                        .as_ref()
                        .map(|_| "active-conversation")
                        .unwrap_or("standalone-task");
                    let flushed = crate::context::manager::pre_compaction_memory_flush(
                        &self.auth,
                        to_drop,
                        compaction_model,
                        &self.config,
                        &self.memory_store,
                        conv_id,
                    )
                    .await;
                    if !flushed.is_empty() {
                        tracing::info!(
                            "Pre-compaction memory flush: {} entries stored",
                            flushed.len()
                        );
                    }

                    if let Some(mut summary) = crate::context::manager::generate_compaction_summary(
                        &self.auth,
                        to_drop,
                        compaction_model,
                        &self.config,
                    )
                    .await
                    {
                        // Supplement LLM summary with verified on-chain data (#201)
                        if let Ok(chain) = proofs::read_proof_chain(&self.wallet, 20).await {
                            if !chain.is_empty() {
                                summary.push_str("\n\n[On-chain verified state]\n");
                                summary
                                    .push_str(&format!("- Proof chain length: {}\n", chain.len()));
                                if let Some(last) = chain.first() {
                                    summary.push_str(&format!(
                                        "- Latest proof hash: {}...\n",
                                        &last.hash[..16.min(last.hash.len())]
                                    ));
                                    summary
                                        .push_str(&format!("- Latest proof txid: {}\n", last.txid));
                                }
                                summary.push_str(&format!(
                                    "- Current iteration: {}\n",
                                    self.state.exec.iteration
                                ));
                                summary.push_str(&format!(
                                    "- Sats spent so far: {}\n",
                                    self.state.budget.sats_spent
                                ));
                                if let Some(ref ph) = self.state.onchain.last_proof_hash {
                                    summary.push_str(&format!(
                                        "- In-memory proof hash: {}...\n",
                                        &ph[..16.min(ph.len())]
                                    ));
                                }
                            }
                        }
                        self.context.set_compaction_summary(summary);
                    }
                }

                // Fire PostCompact hook (notification — non-blocking)
                let compacted_count = history.len().saturating_sub(drop_count);
                let summary_text = self
                    .context
                    .compaction_summary()
                    .unwrap_or("[microcompact only]")
                    .to_string();
                let _ = self
                    .hook_registry
                    .fire(crate::hooks::HookEvent::PostCompact {
                        original_message_count: original_count,
                        compacted_message_count: compacted_count,
                        summary: summary_text,
                    })
                    .await;
            } else {
                tracing::info!(
                    "Compaction blocked by hook: {}",
                    blocked.unwrap_or("unknown reason")
                );
            }
        }

        // Token-based auto-compaction: check utilization and compact if needed.
        // This is a lightweight alternative to the LLM-driven two-phase compaction
        // above — it uses heuristic summaries instead of LLM calls, so it's free.
        let task_budget = self.budget_tracker.limits().max_per_task;
        let budget_spent_pct = if task_budget > 0 {
            (self.state.budget.sats_spent as f64 / task_budget as f64) * 100.0
        } else {
            0.0
        };

        let tool_defs_str =
            serde_json::to_string(&self.tools.read().await.to_openai_tools()).unwrap_or_default();

        if let Some(event) = self.context.check_and_compact(
            &system_prompt,
            &mut history,
            &tool_defs_str,
            &prompt_ctx.memory_summary,
            &prompt_ctx.skills_section,
            budget_spent_pct,
        ) {
            self.transcript.record_system(&format!(
                "Auto-compaction: {} → {} tokens ({} messages dropped)",
                event.tokens_before, event.tokens_after, event.messages_dropped,
            ));
        }

        self.context.build_messages(&system_prompt, &history, None)
    }

    // =============================================================================
    // THINK — budget pre-check, call LLM, record response
    // =============================================================================

    /// THINK phase: budget pre-check, call LLM, record response.
    /// Returns (ThinkResult, effective_tool_calls, has_tool_calls).
    async fn think_step(
        &mut self,
        messages: &[Value],
        ctx: &StepContext,
    ) -> Result<(think::ThinkResult, Vec<Value>, bool), DmError> {
        // Budget pre-check (estimate ~500 sats)
        if let Err(e) = self.budget_tracker.check_limit(500) {
            if self.budget_tracker.is_advisory() {
                tracing::warn!("Budget advisory: {} (continuing in advisory mode)", e);
                emit_event(
                    ctx,
                    StepEvent::BudgetAdvisory {
                        iteration: self.state.exec.iteration,
                        message: e.to_string(),
                        limit_type: "pre-check".into(),
                    },
                );
                self.state.budget.over_budget = true;
            } else {
                return Err(e);
            }
        }

        let tool_defs = self.tools.read().await.to_openai_tools();
        let model = &self.config.llm.default_model;
        let max_tokens = self.max_tokens();

        self.transcript
            .record_think_request(messages, model, max_tokens);

        emit_event(
            ctx,
            StepEvent::ThinkingStarted {
                iteration: self.state.exec.iteration,
                model: model.clone(),
            },
        );

        // Acquire rate limit token before LLM call (no-op when None or disabled)
        if let Some(ref rl) = self.rate_limiter {
            let endpoint = think::resolve_endpoint(&self.config, model);
            rl.acquire(endpoint).await?;
        }

        // Stall detection: abort LLM call if no response within timeout.
        // Default 120s; configurable via config.llm.stall_timeout_secs.
        let stall_timeout_secs = self.config.llm.stall_timeout_secs.unwrap_or(120);
        let think_req = think::ThinkRequest {
            auth: &self.auth,
            messages,
            model,
            max_tokens,
            temperature: None,
            tools: Some(&tool_defs),
            config: &self.config,
            thinking_budget: self.config.llm.thinking_budget,
            reasoning_effort: self.config.llm.reasoning_effort.as_deref(),
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(stall_timeout_secs),
            think::think_with_circuit_breaker(&think_req, &self.circuit_breakers),
        )
        .await
        .map_err(|_| {
            tracing::error!(
                "LLM request stalled after {}s — aborting",
                stall_timeout_secs
            );
            self.transcript.record_error(
                &format!(
                    "LLM request timed out after {}s (stall detection)",
                    stall_timeout_secs
                ),
                None,
            );
            DmError::tool(format!("LLM request timed out after {stall_timeout_secs}s"))
        })??;

        // Task 2.4: Sanity check that sats_effective <= sats_paid
        if result.sats_effective > result.sats_paid && result.sats_paid > 0 {
            tracing::warn!(
                "sats_effective ({}) > sats_paid ({}) — accounting anomaly for model {}",
                result.sats_effective,
                result.sats_paid,
                result.model
            );
        }

        self.state.budget.sats_spent += result.sats_effective;
        self.budget_tracker.record(
            "llm",
            "think",
            result.sats_effective,
            serde_json::json!({
                "model": result.model, "tokens": result.total_tokens,
                "paid": result.sats_paid, "refunded": result.sats_refunded,
                "refund_internalized": result.refund_internalized,
                "txid": result.payment_txid, "task_id": self.state.storage.task_id,
            }),
        );

        // Record Prometheus metrics for this LLM call
        if let Some(ref m) = self.metrics {
            m.tokens_total
                .with_label_values(&[&result.model])
                .inc_by(result.total_tokens);
            let provider = if result.model.starts_with("claude") {
                "claude"
            } else {
                "openai"
            };
            m.request_latency_seconds
                .with_label_values(&[provider])
                .observe(result.duration_ms as f64 / 1000.0);
            m.budget_spent_sats
                .with_label_values(&["llm"])
                .inc_by(result.sats_effective as f64);
        }

        // Fire OnPaymentMade hook (notification-only)
        let _ = self
            .hook_registry
            .fire(crate::hooks::HookEvent::OnPaymentMade {
                provider: result.model.clone(),
                sats_paid: result.sats_effective,
                model: self.config.llm.default_model.clone(),
            })
            .await;

        // Moderate LLM response text
        if !result.text.is_empty() {
            use super::moderation::ModerationOutcome;
            if self.moderate_content(&result.text, "llm_response", None, "LLM response")
                == ModerationOutcome::Blocked
            {
                self.state.exec.error = "LLM response blocked by moderation policy".to_string();
                self.state.exec.done = true;
                return Err(DmError::tool(
                    "LLM response blocked by moderation policy".to_string(),
                ));
            }
        }

        // Tool calls (including any text-extracted calls from reasoning model fallback)
        // are already in result.tool_calls — think_with_tools handles extraction (#211).
        let effective_tool_calls = result.tool_calls.clone();
        let has_tool_calls = !effective_tool_calls.is_empty();

        // Record think response
        let tc_ref: Option<&[Value]> = if has_tool_calls {
            Some(&effective_tool_calls)
        } else {
            None
        };
        self.transcript.record_think_response(
            &result.text,
            &result.model,
            result.sats_paid,
            result.sats_effective,
            result.sats_refunded,
            result.prompt_tokens,
            result.completion_tokens,
            tc_ref,
            &result.finish_reason,
            result.duration_ms,
        );

        emit_event(
            ctx,
            StepEvent::ThinkingComplete {
                iteration: self.state.exec.iteration,
                text: result.text.clone(),
                sats_paid: result.sats_effective,
                tokens: result.total_tokens,
                has_tool_calls,
            },
        );

        // Warn if truncated
        if result.finish_reason == "length" {
            tracing::warn!(
                "LLM response truncated (finish_reason=length, {} completion tokens)",
                result.completion_tokens
            );
            self.transcript.record_system(&format!(
                "WARNING: Your previous response was truncated at {} tokens. \
                 Consider breaking your response into smaller parts.",
                result.completion_tokens
            ));
        }

        Ok((result, effective_tool_calls, has_tool_calls))
    }

    // =============================================================================
    // RECORD — decision proof, budget token update, done/nudge logic
    // =============================================================================

    /// RECORD phase: decision proof, budget token update, payment receipt,
    /// budget drain check, done/nudge logic.
    #[allow(clippy::too_many_arguments)]
    async fn record_and_resolve(
        &mut self,
        result: &think::ThinkResult,
        has_tool_calls: bool,
        effective_tool_calls: &[Value],
        called_send_message: bool,
        memory_ids: &[String],
        message_hashes: &[String],
        ctx: &StepContext,
    ) -> Result<(), DmError> {
        // Working memory: expire stale entries
        {
            let mut wm = self.working_memory.write().await;
            let expired = wm.expire(self.state.exec.iteration);
            for key in &expired {
                tracing::info!(key = %key, iteration = self.state.exec.iteration, "working_memory expired");
            }
        }

        // Record budget
        let balance = self.wallet.get_balance().await.unwrap_or(0);
        let task_limit = self.budget_tracker.limits().max_per_task;
        self.transcript.record_budget(
            balance,
            self.state.budget.sats_spent,
            self.state.budget.sats_spent,
            task_limit,
        );

        emit_event(
            ctx,
            StepEvent::BudgetUpdate {
                balance,
                spent: self.state.budget.sats_spent,
                remaining: self
                    .budget_tracker
                    .limits()
                    .max_per_task
                    .saturating_sub(self.state.budget.sats_spent),
            },
        );

        let drain = self
            .detector
            .check_budget_drain(self.state.budget.sats_spent);
        if drain.stuck {
            let mut data = HashMap::new();
            data.insert("message".to_string(), Value::String(drain.message.clone()));
            self.transcript.record("loop_warning", data);
            if drain.level == "critical" {
                return Err(DmError::loop_err(drain.message));
            }
        }

        // BRC-18 Decision proof per iteration
        {
            // Compute pre-state root from current BRC-48 token txids (#202).
            let pre_state_root = state::compute_state_root(
                self.state
                    .onchain
                    .task_commitment
                    .as_ref()
                    .and_then(|t| t.txid.as_deref()),
                self.state
                    .onchain
                    .budget_allocation
                    .as_ref()
                    .and_then(|t| t.txid.as_deref()),
                self.state
                    .onchain
                    .capability_declaration
                    .as_ref()
                    .and_then(|t| t.txid.as_deref()),
                self.state
                    .onchain
                    .last_checkpoint
                    .as_ref()
                    .and_then(|t| t.txid.as_deref()),
            );

            let decision_text = if has_tool_calls {
                let tool_names: Vec<&str> = effective_tool_calls
                    .iter()
                    .filter_map(|tc| {
                        tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                    })
                    .collect();
                let mut parts = format!(
                    "Iteration {}: called tools [{}]",
                    self.state.exec.iteration,
                    tool_names.join(", ")
                );
                if !memory_ids.is_empty() {
                    parts.push_str(&format!(" memory_ids=[{}]", memory_ids.join(",")));
                }
                if !message_hashes.is_empty() {
                    parts.push_str(&format!(" message_hashes=[{}]", message_hashes.join(",")));
                }
                parts
            } else {
                format!(
                    "Iteration {}: response generated",
                    self.state.exec.iteration
                )
            };
            let mut reasoning = format!(
                "sha256:{}\nMODEL: {}\nSATS: {}\nPAYMENT_TXID: {}",
                content_hash(&result.text),
                result.model,
                result.sats_effective,
                result.payment_txid.as_deref().unwrap_or("none"),
            );
            if !message_hashes.is_empty() {
                reasoning.push_str(&format!("\nMESSAGE_HASHES: {}", message_hashes.join(",")));
            }
            // Include BRC-52 certificate hash for authorization auditability (#198).
            if let Some(ref cert_hash) = self.state.auth.cert_hash {
                let cert_type = self.state.auth.cert_type.as_deref().unwrap_or("unknown");
                reasoning.push_str(&format!("\nCERT_HASH: sha256:{cert_hash}"));
                reasoning.push_str(&format!("\nCERT_TYPE: {cert_type}"));
            }
            // State provenance: link this decision to its input BRC-48 state (#202).
            reasoning.push_str(&format!("\nPRE_STATE_ROOT: {pre_state_root}"));
            let commitment = proofs::decision_proof(
                &decision_text,
                &reasoning,
                self.state.onchain.last_proof_hash.as_deref(),
            );
            self.record_proof(
                commitment,
                "decision",
                Some(serde_json::json!({"iteration": self.state.exec.iteration})),
            )
            .await;
        }

        // BRC-48 BudgetAllocation token update
        if self.state.onchain.budget_allocation.is_some() {
            let iter_budget_data = serde_json::json!({
                "budget_cap": self.config.budget.max_per_task,
                "task_spent": self.state.budget.sats_spent,
                "iteration": self.state.exec.iteration,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let old_ba = self.state.onchain.budget_allocation.clone();
            if let Some(token) = self
                .record_token(
                    "budget_allocation",
                    state::BASKET_BUDGET,
                    iter_budget_data,
                    old_ba.as_ref(),
                )
                .await
            {
                self.state.onchain.budget_allocation = Some(token);
            }
        }

        // Store payment receipt
        if result.sats_paid > 0 {
            let beef_dir = self.workspace.join("beef");
            let _ = std::fs::create_dir_all(&beef_dir);
            let receipt = serde_json::json!({
                "iteration": self.state.exec.iteration, "model": result.model,
                "sats_paid": result.sats_paid, "sats_effective": result.sats_effective,
                "sats_refunded": result.sats_refunded, "tokens": result.total_tokens,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let receipt_path = beef_dir.join(format!("{}.json", self.state.exec.iteration));
            if let Ok(json) = serde_json::to_string_pretty(&receipt) {
                let _ = std::fs::write(&receipt_path, json);
                self.transcript.record_receipt_stored(
                    &receipt_path.to_string_lossy(),
                    &format!("think-iter-{}", self.state.exec.iteration),
                    result.sats_effective,
                );
            }
        }

        // Done / nudge logic
        let unfulfilled = self.state.comms.pending_replies.len();

        if !has_tool_calls {
            if unfulfilled > 0 && !called_send_message && self.state.comms.nudge_count < MAX_NUDGES
            {
                self.state.comms.nudge_count += 1;
                tracing::info!(
                    "Nudge {}/{}: {} unfulfilled reply obligation(s) but LLM responded with text",
                    self.state.comms.nudge_count,
                    MAX_NUDGES,
                    unfulfilled
                );
                let sender_list: String = self
                    .state
                    .comms
                    .pending_replies
                    .iter()
                    .map(|r| r.sender_key.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.transcript.record_user(&format!(
                    "You responded with text, but your text responses are internal only — \
                     the sender CANNOT see your text. You MUST use the `send_message` tool \
                     to deliver your reply. Call send_message now with recipient set to \
                     the sender's identity key ({sender_list}) and message_box set to \"status_inbox\"."
                ));
            } else if unfulfilled > 0 && !called_send_message {
                tracing::warn!(
                    "Nudges exhausted ({}). Force-sending reply to {} sender(s).",
                    MAX_NUDGES,
                    unfulfilled
                );
                for obligation in &self.state.comms.pending_replies.clone() {
                    let send_args = serde_json::json!({
                        "recipient": obligation.sender_key,
                        "message_box": obligation.message_box,
                        "body": {"type": "task_result", "result": result.text},
                    });
                    let call_id = format!("forced-{}", self.state.exec.iteration);
                    self.transcript
                        .record_tool_call(&call_id, "send_message", &send_args);
                    match self.execute_tool("send_message", send_args).await {
                        Ok(output) => {
                            tracing::info!(
                                "Forced send_message succeeded: {}",
                                &output[..output.len().min(200)]
                            );
                            self.transcript.record_tool_result(
                                &call_id,
                                "send_message",
                                &output,
                                true,
                                0,
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Forced send_message failed: {e}");
                            self.transcript.record_tool_result(
                                &call_id,
                                "send_message",
                                &e.to_string(),
                                false,
                                0,
                            );
                        }
                    }
                }
                self.state.comms.pending_replies.clear();
                self.state.exec.result = result.text.clone();
                self.state.exec.done = true;
                emit_event(
                    ctx,
                    StepEvent::Response {
                        iteration: self.state.exec.iteration,
                        text: result.text.clone(),
                    },
                );
            } else {
                self.state.exec.result = result.text.clone();
                self.state.exec.done = true;
                emit_event(
                    ctx,
                    StepEvent::Response {
                        iteration: self.state.exec.iteration,
                        text: result.text.clone(),
                    },
                );
            }
        }

        // Tool-call-only done signal
        //
        // BUG-007 fix: drop the `!result.text.is_empty()` requirement. Reasoning
        // models (gpt-5-mini, o-series, gpt-4.1) routinely return empty visible text
        // with their tool calls — the reasoning is hidden. Without this fix, a
        // reasoning-model agent that calls only `send_message` in an iteration will
        // never terminate via this done-signal, even when all reply obligations are
        // already fulfilled. Combined with BUG-008 below (the has_tool_calls nudge
        // gap), this left no termination path for messaging-only reasoning loops —
        // see the Coral refusal-loop forensics in issue #324 for the real-world case
        // that burned 188K sats before the user killed it.
        //
        // When text is empty, synthesize a minimal summary so the final
        // `StepEvent::Response` still carries a readable message.
        if has_tool_calls && self.state.comms.pending_replies.is_empty() && called_send_message {
            let only_messaging = effective_tool_calls.iter().all(|tc| {
                tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    == Some("send_message")
            });
            if only_messaging {
                tracing::info!("All reply obligations fulfilled and only send_message tools called — marking done");
                let final_text = if result.text.is_empty() {
                    format!(
                        "Sent {} message(s) and terminated.",
                        effective_tool_calls.len()
                    )
                } else {
                    result.text.clone()
                };
                self.state.exec.result = final_text.clone();
                self.state.exec.done = true;
                emit_event(
                    ctx,
                    StepEvent::Response {
                        iteration: self.state.exec.iteration,
                        text: final_text,
                    },
                );
            }
        }

        // BUG-008 fix: nudge/force-send when tool calls were made but don't fulfill obligations.
        //
        // The original logic at the top of this section only fired the nudge/force-send
        // fallback when `!has_tool_calls` — i.e., when the LLM returned pure text. If the
        // LLM made tool calls that didn't include a satisfying `send_message`, the runner
        // just continued iterating with no intervention. Combined with the done-signal
        // gap (BUG-007), a reasoning-model agent making repeated tool calls had no code
        // path that could terminate it. Coral's refusal loop in #324 hit exactly this.
        //
        // This branch handles the missing case: tool calls were made, but pending reply
        // obligations are still unfulfilled and no send_message call cleared them.
        // Same nudge-then-force-send ladder as the text-only path above, just triggered
        // from a different precondition.
        if has_tool_calls && unfulfilled > 0 && !called_send_message && !self.state.exec.done {
            if self.state.comms.nudge_count < MAX_NUDGES {
                self.state.comms.nudge_count += 1;
                tracing::info!(
                    "Nudge {}/{}: {} unfulfilled reply obligation(s); LLM made tool calls but not send_message",
                    self.state.comms.nudge_count,
                    MAX_NUDGES,
                    unfulfilled
                );
                let sender_list: String = self
                    .state
                    .comms
                    .pending_replies
                    .iter()
                    .map(|r| r.sender_key.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.transcript.record_user(&format!(
                    "You made tool calls but did not call `send_message` to reply to \
                     {unfulfilled} pending sender(s): {sender_list}. Your tool outputs \
                     are internal — they do not reach the original sender. You MUST \
                     call send_message with recipient set to the sender's identity key \
                     and message_box set to \"task_inbox\"."
                ));
            } else {
                tracing::warn!(
                    "Nudges exhausted ({}). Force-sending reply to {} sender(s) despite tool calls.",
                    MAX_NUDGES,
                    unfulfilled
                );
                let obligations_snapshot = self.state.comms.pending_replies.clone();
                for obligation in &obligations_snapshot {
                    let send_args = serde_json::json!({
                        "recipient": obligation.sender_key,
                        "message_box": obligation.message_box,
                        "body": {"type": "task_result", "result": result.text},
                    });
                    let call_id = format!("forced-{}", self.state.exec.iteration);
                    self.transcript
                        .record_tool_call(&call_id, "send_message", &send_args);
                    match self.execute_tool("send_message", send_args).await {
                        Ok(output) => {
                            tracing::info!(
                                "Forced send_message succeeded: {}",
                                &output[..output.len().min(200)]
                            );
                            self.transcript.record_tool_result(
                                &call_id,
                                "send_message",
                                &output,
                                true,
                                0,
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Forced send_message failed: {e}");
                            self.transcript.record_tool_result(
                                &call_id,
                                "send_message",
                                &e.to_string(),
                                false,
                                0,
                            );
                        }
                    }
                }
                self.state.comms.pending_replies.clear();
                self.state.exec.result = result.text.clone();
                self.state.exec.done = true;
                emit_event(
                    ctx,
                    StepEvent::Response {
                        iteration: self.state.exec.iteration,
                        text: result.text.clone(),
                    },
                );
            }
        }

        // Offload if needed
        if !result.text.is_empty() && self.context.should_offload(&result.text) {
            self.context.offload_to_file(&result.text, "response");
        }

        Ok(())
    }

    // =============================================================================
    // step() — orchestrate one iteration
    // =============================================================================

    /// Execute one iteration of the agent loop.
    ///
    /// Orchestrates OBSERVE → BUILD → THINK → ACT → RECORD phases.
    /// When `ctx.events` is `Some`, emits `StepEvent`s for SSE streaming (server mode).
    pub(crate) async fn step(&mut self, ctx: &StepContext) -> Result<(), DmError> {
        self.state.exec.iteration += 1;
        self.wm_iteration.store(
            self.state.exec.iteration,
            std::sync::atomic::Ordering::Relaxed,
        );
        tracing::info!("Step {}", self.state.exec.iteration);

        // State checkpoint every 5 iterations for crash recovery (#273).
        // Records a snapshot of key loop state so reconstruction doesn't
        // need to replay the entire transcript.
        if self.state.exec.iteration > 1 && self.state.exec.iteration.is_multiple_of(5) {
            let tools: Vec<String> = Vec::new(); // lightweight — full tools_used tracked in SessionState
            self.transcript.record_state_checkpoint(
                self.state.exec.iteration,
                self.state.budget.sats_spent,
                self.state.onchain.last_proof_hash.as_deref(),
                &tools,
                &self.config.llm.default_model,
            );
        }

        // Fire IterationStart hook
        self.hook_registry
            .fire(crate::hooks::HookEvent::IterationStart {
                task_id: ctx.task_id.clone(),
                iteration: self.state.exec.iteration,
            })
            .await;

        // PERIODIC AUTO-VERIFY: compare in-memory state against on-chain proof chain.
        // Runs every `verify_interval` iterations (0 = disabled).
        let verify_interval = self.config.lifecycle.verify_interval;
        if verify_interval > 0
            && self.state.exec.iteration > 1
            && self.state.exec.iteration.is_multiple_of(verify_interval)
        {
            match proofs::read_proof_chain(&self.wallet, 50).await {
                Ok(chain) => {
                    let result = proofs::verify_state_against_proofs(
                        &chain,
                        self.state.exec.iteration,
                        self.state.budget.sats_spent,
                        self.state.onchain.last_proof_hash.as_deref(),
                    );
                    if !result.consistent {
                        tracing::warn!(
                            "State divergence detected at iteration {}: {:?}",
                            self.state.exec.iteration,
                            result.divergences
                        );
                        self.transcript.record_system(&format!(
                            "STATE VERIFICATION WARNING: {} divergence(s) detected between in-memory state and on-chain proofs. Chain length: {}. Divergences: {}",
                            result.divergences.len(),
                            result.chain_length,
                            result.divergences.iter().map(|d| format!("{}: memory={} vs chain={}", d.field, d.in_memory, d.on_chain)).collect::<Vec<_>>().join("; ")
                        ));
                    } else {
                        tracing::debug!(
                            "State verification passed at iteration {} (chain_length={})",
                            self.state.exec.iteration,
                            result.chain_length
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!("Auto-verify skipped: {e}");
                }
            }
        }

        // PERIODIC BRC-48 CONSISTENCY CHECK: compare basket token counts against LoopState.
        // Runs every `consistency_check_interval` iterations (0 = disabled). (#196)
        let consistency_interval = self.config.lifecycle.consistency_check_interval;
        if consistency_interval > 0
            && self.state.exec.iteration > 1
            && self
                .state
                .exec
                .iteration
                .is_multiple_of(consistency_interval)
        {
            // Refresh basket health from on-chain data
            let mut basket_health = std::collections::HashMap::new();
            for basket in &[
                state::BASKET_STATE,
                state::BASKET_BUDGET,
                state::BASKET_PROOFS,
            ] {
                match state::read_active_token_summary(&self.wallet, basket, 200).await {
                    Ok(summary) => {
                        basket_health.insert(basket.to_string(), summary.count);
                    }
                    Err(e) => {
                        tracing::debug!("Consistency check: failed to read basket {basket}: {e}");
                    }
                }
            }

            let result = state::check_consistency(
                self.state.exec.iteration,
                self.state.budget.sats_spent,
                &basket_health,
            );
            if !result.consistent {
                let diverged: Vec<_> = result
                    .checks
                    .iter()
                    .filter(|c| c.status == "diverged")
                    .map(|c| {
                        format!(
                            "{}: memory={} vs chain={}",
                            c.field, c.in_memory, c.on_chain
                        )
                    })
                    .collect();
                tracing::warn!(
                    "BRC-48 token consistency divergence at iteration {}: {}",
                    self.state.exec.iteration,
                    diverged.join("; ")
                );
                self.transcript.record_system(&format!(
                    "TOKEN CONSISTENCY WARNING: {} divergence(s) detected between in-memory state and BRC-48 basket tokens. {}",
                    diverged.len(),
                    diverged.join("; ")
                ));
            } else {
                tracing::debug!(
                    "BRC-48 token consistency check passed at iteration {}",
                    self.state.exec.iteration
                );
            }

            // Update basket_health on LoopState for future reference
            self.state.onchain.basket_health = basket_health;
        }

        // OBSERVE: poll inbox, partition messages, build obligations
        let has_external = self.observe().await;

        // BUILD: construct system prompt and LLM messages
        let messages = self.build_llm_messages(has_external).await;

        // THINK: call LLM with tool definitions
        let (mut result, mut effective_tool_calls, mut has_tool_calls) =
            self.think_step(&messages, ctx).await?;

        // TRUNCATION RECOVERY: if the LLM response was truncated (finish_reason=length)
        // and produced no usable output (empty text, no tool calls), retry with a
        // conciseness instruction. Max 2 retries, then force-stop to prevent runaway loops.
        {
            let mut truncation_retries: u32 = 0;
            const MAX_TRUNCATION_RETRIES: u32 = 2;
            while result.finish_reason == "length"
                && result.text.trim().is_empty()
                && !has_tool_calls
                && truncation_retries < MAX_TRUNCATION_RETRIES
            {
                truncation_retries += 1;
                tracing::warn!(
                    "Truncation recovery {}/{}: empty response at {} completion tokens, retrying with conciseness instruction",
                    truncation_retries, MAX_TRUNCATION_RETRIES, result.completion_tokens
                );
                self.transcript.record_system(
                    "Your previous response was truncated and produced no output. \
                     Respond in 2-3 sentences maximum. Be extremely concise. \
                     If you need to use a tool, just call it — don't explain first.",
                );

                let retry_messages = self.build_llm_messages(has_external).await;
                let (r, tc, htc) = self.think_step(&retry_messages, ctx).await?;
                result = r;
                effective_tool_calls = tc;
                has_tool_calls = htc;
            }

            // If still truncated after retries, force-stop to prevent runaway
            if result.finish_reason == "length"
                && result.text.trim().is_empty()
                && !has_tool_calls
                && truncation_retries >= MAX_TRUNCATION_RETRIES
            {
                tracing::error!(
                    "Truncation recovery exhausted after {} retries — forcing task stop",
                    truncation_retries
                );
                self.state.exec.result =
                    "[Response truncated — output token limit reached. Try a simpler question or use a model with a higher output limit.]".to_string();
                self.state.exec.done = true;
                self.transcript.record_system(
                    "FORCED STOP: Response repeatedly truncated with no usable output. \
                     Task stopped to prevent runaway spending.",
                );
            }
        }

        // TERSE RESUME: if the LLM response was truncated but HAS content,
        // auto-continue with a "no recap" instruction. This saves 200-500 tokens
        // that would otherwise be wasted on "As I was saying..." preamble.
        // Max 3 terse continuations per iteration. Recovery count resets each iteration.
        {
            let mut terse_retries: u32 = 0;
            const MAX_TERSE_RETRIES: u32 = 3;
            while result.finish_reason == "length"
                && !result.text.trim().is_empty()
                && !has_tool_calls
                && terse_retries < MAX_TERSE_RETRIES
            {
                terse_retries += 1;
                tracing::info!(
                    "Terse resume {}/{}: continuing truncated response ({} tokens)",
                    terse_retries,
                    MAX_TERSE_RETRIES,
                    result.completion_tokens
                );
                self.transcript.record_system(
                    "Output token limit hit. Resume directly from where you stopped — \
                     no apology, no recap. Break remaining work into smaller pieces.",
                );

                let resume_messages = self.build_llm_messages(has_external).await;
                let (r, tc, htc) = self.think_step(&resume_messages, ctx).await?;
                result = r;
                effective_tool_calls = tc;
                has_tool_calls = htc;
            }
        }

        // INTENT NUDGE: if LLM described tool actions but didn't call any,
        // inject a system message and re-call (max MAX_INTENT_NUDGES times).
        // This is SEPARATE from the reply obligation nudge system in record_and_resolve().
        {
            let mut intent_nudge_count: u32 = 0;
            while !has_tool_calls
                && intent_nudge_count < super::MAX_INTENT_NUDGES
                && super::signals_tool_intent(&result.text)
            {
                intent_nudge_count += 1;
                tracing::info!(
                    "Intent nudge {}/{}: LLM expressed tool intent without calling a tool",
                    intent_nudge_count,
                    super::MAX_INTENT_NUDGES
                );
                self.transcript.record_system(super::TOOL_INTENT_NUDGE);

                // Re-build messages (transcript now includes the nudge) and re-call LLM
                let nudge_messages = self.build_llm_messages(has_external).await;
                let (r, tc, htc) = self.think_step(&nudge_messages, ctx).await?;
                result = r;
                effective_tool_calls = tc;
                has_tool_calls = htc;
            }
        }

        // ACT: execute tool calls
        let (called_send_message, memory_ids, message_hashes) = if has_tool_calls {
            self.execute_tools(&effective_tool_calls, ctx).await?
        } else {
            (false, Vec::new(), Vec::new())
        };

        // RECORD: proofs, budget updates, done/nudge logic
        let record_result = self
            .record_and_resolve(
                &result,
                has_tool_calls,
                &effective_tool_calls,
                called_send_message,
                &memory_ids,
                &message_hashes,
                ctx,
            )
            .await;

        // Fire IterationEnd hook
        let action = if has_tool_calls {
            effective_tool_calls
                .iter()
                .filter_map(|c| {
                    c.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            "text_response".to_string()
        };
        self.hook_registry
            .fire(crate::hooks::HookEvent::IterationEnd {
                task_id: ctx.task_id.clone(),
                iteration: self.state.exec.iteration,
                action_taken: action,
            })
            .await;

        record_result
    }
}
