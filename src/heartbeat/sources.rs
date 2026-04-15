//! Work sources that feed the scheduler's PriorityInbox.
//!
//! Three independent scan functions on `Scheduler`:
//! - `poll_messagebox()` — polls MessageBox inboxes for new messages
//! - `scan_continuations()` — scans workspace/continuations/ for paused tasks
//! - `scan_schedules()` — scans workspace/schedules/ for due recurring schedules

use super::commission_payments::{handle_commission_payment_claim, handle_commission_payment_sent};
use super::features::{check_sender_trust, should_respond_to_message, TrustTier};
use super::{InboxItem, Scheduler, TaskPriority};
use crate::messagebox::types::{
    BOX_DM_COORDINATION, BOX_RESULTS_INBOX, BOX_STATUS_INBOX, BOX_TASK_INBOX,
};

// ---------------------------------------------------------------------------
// Scheduler scan methods
// ---------------------------------------------------------------------------

impl Scheduler {
    /// Poll MessageBox inboxes and route messages by inbox type.
    ///
    /// Only `task_inbox` messages spawn new LLM tasks. Other inboxes are
    /// handled as metadata:
    /// - `results_inbox`  → delivery receipts, logged but never spawn tasks
    /// - `status_inbox`   → progress updates, logged but never spawn tasks
    /// - `coordination`   → peer metadata, logged but never spawn tasks
    ///
    /// Within `task_inbox`, messages are further filtered by:
    /// - Self-message check (prevent feedback loops)
    /// - Message type guard (`should_respond_to_message`)
    /// - Sender trust check (`check_sender_trust`)
    /// - Parent detection (for priority escalation)
    pub(super) async fn poll_messagebox(&mut self) {
        let parent_key = &self.config.parent.identity_key;
        let trusted_certifiers = &self.config.parent.trusted_certifiers;
        let own_identity_key = &self.app_state.auth.server_identity_key;
        let conv_manager = crate::conversation::ConversationManager::new(&self.workspace);

        match self.messagebox.poll_inboxes().await {
            Ok(messages) => {
                if messages.is_empty() {
                    tracing::debug!("Scheduler: no new messages");
                } else {
                    tracing::info!("Scheduler: {} new message(s)", messages.len());

                    for msg in &messages {
                        let sender_prefix = &msg.sender[..msg.sender.len().min(16)];
                        let inbox = msg.message_box.as_deref().unwrap_or("unknown");

                        // Skip self-sent messages on ALL inboxes — these are delivery
                        // confirmations, not incoming conversations.
                        if !own_identity_key.is_empty() && msg.sender == *own_identity_key {
                            tracing::info!(
                                "Scheduler: skipping self-sent message on {inbox} from {sender_prefix}"
                            );
                            continue;
                        }

                        // Route by inbox type
                        match inbox {
                            // -------------------------------------------------------
                            // task_inbox: actionable work — may spawn an LLM task
                            // -------------------------------------------------------
                            x if x == BOX_TASK_INBOX => {
                                // EPIC #329 Phase 3 §C.3: server-side commission
                                // payment claim. If the body is type
                                // "commission_payment_claim", validate against
                                // the agent's dm-delegation-revocation basket
                                // and pay via wallet.create_action — no LLM,
                                // no task spawn. The message is still ACKed
                                // along with the rest at the bottom of the loop.
                                let body_type =
                                    msg.body.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                if body_type == "commission_payment_claim" {
                                    let result = handle_commission_payment_claim(
                                        self.app_state.wallet.clone(),
                                        self.messagebox.clone(),
                                        &msg.body,
                                        &msg.sender,
                                        Some(self.workspace.as_path()),
                                    )
                                    .await;
                                    if result.success {
                                        tracing::info!(
                                            sender = %sender_prefix,
                                            sats = result.sats_paid,
                                            txid = ?result.txid,
                                            "commission_payment_claim handled and paid"
                                        );
                                    } else {
                                        tracing::warn!(
                                            sender = %sender_prefix,
                                            "commission_payment_claim REJECTED: {}",
                                            result.reason
                                        );
                                    }
                                    continue;
                                }

                                // Message type guard: skip task_result, status_update,
                                // coordination, and turn-limited agent_messages
                                if !should_respond_to_message(&msg.body) {
                                    tracing::info!(
                                        "Scheduler: skipping {inbox} message from {sender_prefix} — filtered by message type/turn-taking"
                                    );
                                    continue;
                                }

                                // Trust check
                                let trust =
                                    check_sender_trust(&msg.sender, parent_key, trusted_certifiers);
                                match trust {
                                    TrustTier::Family | TrustTier::Vouched => {
                                        tracing::info!(
                                            "Scheduler: trusted sender ({trust:?}) on {inbox} from {sender_prefix}"
                                        );
                                    }
                                    TrustTier::Unknown => {
                                        // Phase 1: allow unknown senders for backwards compat
                                        tracing::info!(
                                            "Scheduler: unknown trust for {sender_prefix} — allowing (Phase 1 compat)"
                                        );
                                    }
                                    TrustTier::Discovered | TrustTier::Stranger => {
                                        // Phase 2: will enforce fees here
                                        tracing::info!(
                                            "Scheduler: low-trust sender ({trust:?}) from {sender_prefix} — allowing (Phase 2 will enforce fees)"
                                        );
                                    }
                                }

                                // Detect inbound delegation envelopes. When the body's
                                // `type` is `task_delegation`, the envelope is a
                                // commission from another agent carrying a signed
                                // delegation cert + task description. We thread the
                                // full (untruncated) envelope through `InboxItem`
                                // so `WormLoop::setup_task()` can verify the cert
                                // and apply its caveats. The task description we
                                // queue is the envelope's `task` field, NOT the
                                // generic "Process inbox message ..." preview —
                                // the cert's purpose_hash is bound to that exact
                                // task string.
                                let is_delegation = msg.body.get("type").and_then(|v| v.as_str())
                                    == Some("task_delegation");

                                let (task_description, delegation_envelope) = if is_delegation {
                                    let task_text = msg
                                        .body
                                        .get("task")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if task_text.is_empty() {
                                        tracing::warn!(
                                            "Scheduler: task_delegation from {sender_prefix} \
                                             is missing `task` field — falling back to generic inbox wrapper"
                                        );
                                        let body_preview = serde_json::to_string(&msg.body)
                                            .unwrap_or_default()
                                            .chars()
                                            .take(200)
                                            .collect::<String>();
                                        let desc = format!(
                                            "Process inbox message from {} (reply to this identity key: {}): {}",
                                            sender_prefix, msg.sender, body_preview,
                                        );
                                        (desc, None)
                                    } else {
                                        tracing::info!(
                                            "Scheduler: inbound task_delegation from {sender_prefix} — \
                                             threading envelope through to runner (task={} chars)",
                                            task_text.len()
                                        );
                                        (task_text, Some(msg.body.clone()))
                                    }
                                } else {
                                    let body_preview = serde_json::to_string(&msg.body)
                                        .unwrap_or_default()
                                        .chars()
                                        .take(200)
                                        .collect::<String>();
                                    let desc = format!(
                                        "Process inbox message from {} (reply to this identity key: {}): {}",
                                        sender_prefix, msg.sender, body_preview,
                                    );
                                    (desc, None)
                                };

                                // Parent messages get highest priority
                                let priority =
                                    if !parent_key.is_empty() && msg.sender == *parent_key {
                                        tracing::info!(
                                        "Scheduler: parent message detected from {sender_prefix}"
                                    );
                                        TaskPriority::Highest
                                    } else {
                                        TaskPriority::Normal
                                    };

                                // Session routing:
                                // - task_delegation envelopes get a FRESH commission-scoped
                                //   conversation keyed by commission_id. These are one-shot
                                //   contracts, not chats, and must NEVER share prior history
                                //   with the sender's previous messages. See EPIC #329 Phase 3.
                                // - Plain messages fall back to find_by_participant for
                                //   multi-turn continuity with the sender.
                                let session_id = if delegation_envelope.is_some() {
                                    let commission_id = msg
                                        .body
                                        .get("commission_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    let sid = format!("commission-{commission_id}");
                                    tracing::info!(
                                        "Scheduler: routing task_delegation from {sender_prefix} \
                                         to fresh commission conversation {sid}"
                                    );
                                    Some(sid)
                                } else {
                                    let sid = conv_manager.find_by_participant(&msg.sender);
                                    if let Some(ref sid) = sid {
                                        tracing::info!(
                                            "Scheduler: routing task_inbox message from {sender_prefix} to conversation {sid}"
                                        );
                                    }
                                    sid
                                };

                                self.inbox.push(InboxItem {
                                    message: task_description,
                                    priority,
                                    session_id,
                                    model_override: None,
                                    submitted_at: chrono::Utc::now(),
                                    reply_to: Some(msg.sender.clone()),
                                    max_iterations: None,
                                    origin: "message".to_string(),
                                    delegation_envelope,
                                });
                            }

                            // -------------------------------------------------------
                            // results_inbox: delivery receipts — NEVER spawn tasks
                            // -------------------------------------------------------
                            x if x == BOX_RESULTS_INBOX => {
                                let task_id = msg
                                    .body
                                    .get("task_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let status = msg
                                    .body
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                tracing::info!(
                                    "Scheduler: received task_result from {sender_prefix} — task={task_id} status={status} (logged, not spawning task)"
                                );
                                // Phase 2: match to originating exchange via exchange_id
                                // and append result to the originating conversation.
                            }

                            // -------------------------------------------------------
                            // status_inbox: progress updates — NEVER spawn tasks
                            // -------------------------------------------------------
                            x if x == BOX_STATUS_INBOX => {
                                // EPIC #329 Phase 3 §C.3 Coral side: a
                                // commission_payment_sent reply from the
                                // commission issuer carrying the BEEF for
                                // the payment. Internalize via the wallet
                                // funding flow so the agent's balance
                                // actually increases.
                                let body_type =
                                    msg.body.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                if body_type == "commission_payment_sent" {
                                    let result = handle_commission_payment_sent(
                                        self.app_state.wallet.clone(),
                                        &msg.body,
                                        Some(self.workspace.as_path()),
                                    )
                                    .await;
                                    if result.success {
                                        tracing::info!(
                                            sender = %sender_prefix,
                                            sats = result.sats_paid,
                                            txid = ?result.txid,
                                            "commission_payment_sent internalized"
                                        );
                                    } else {
                                        tracing::warn!(
                                            sender = %sender_prefix,
                                            "commission_payment_sent FAILED to internalize: {}",
                                            result.reason
                                        );
                                    }
                                    continue;
                                }

                                let task_id = msg
                                    .body
                                    .get("task_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                let status = msg
                                    .body
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                tracing::info!(
                                    "Scheduler: received status_update from {sender_prefix} — task={task_id} status={status} (logged, not spawning task)"
                                );
                            }

                            // -------------------------------------------------------
                            // coordination: peer metadata — NEVER spawn tasks
                            // -------------------------------------------------------
                            x if x == BOX_DM_COORDINATION => {
                                let signal = msg
                                    .body
                                    .get("signal")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");
                                tracing::info!(
                                    "Scheduler: received coordination signal={signal} from {sender_prefix} (logged, not spawning task)"
                                );
                                // Phase 2: update peer registry with capabilities/load info
                            }

                            // -------------------------------------------------------
                            // Unknown inbox — log and skip
                            // -------------------------------------------------------
                            _ => {
                                tracing::warn!(
                                    "Scheduler: message from {sender_prefix} on unknown inbox '{inbox}' — ignoring"
                                );
                            }
                        }
                    }

                    // Acknowledge ALL processed messages (including non-task ones).
                    // If ack fails, messages will be re-delivered next tick.
                    let ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
                    if let Err(e) = self.messagebox.acknowledge_message(&ids).await {
                        tracing::warn!(
                            "Scheduler: failed to acknowledge {} message(s): {e} — \
                             these messages may be re-delivered on next tick (potential duplicates)",
                            ids.len()
                        );
                    }
                }
            }
            Err(e) => {
                tracing::debug!("Scheduler poll failed (non-fatal): {e}");
            }
        }
    }

    /// Scan workspace/continuations/ for ready continuation files and push them into the inbox.
    pub(super) fn scan_continuations(&mut self) {
        let cont_dir = self.workspace.join("continuations");

        let entries = match std::fs::read_dir(&cont_dir) {
            Ok(e) => e,
            Err(_) => return, // No continuations dir = nothing to do
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read continuation {}: {e}", path.display());
                    continue;
                }
            };

            let state: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Failed to parse continuation {}: {e}", path.display());
                    continue;
                }
            };

            // Check wake_at time
            if let Some(wake_at) = state.get("wake_at").and_then(|v| v.as_str()) {
                if let Ok(wake_time) = chrono::DateTime::parse_from_rfc3339(wake_at) {
                    if wake_time > chrono::Utc::now() {
                        continue; // Not time yet
                    }
                }
            }
            // If wake_at is null, resume immediately (next heartbeat)

            let cont_id = state
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let task = state
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("Resume task");

            let task_desc = format!("CONTINUATION:{cont_id}\nResume task: {task}");

            self.inbox.push(InboxItem {
                message: task_desc,
                priority: TaskPriority::Low,
                session_id: None,
                model_override: None,
                submitted_at: chrono::Utc::now(),
                reply_to: None,
                max_iterations: None,
                origin: "continuation".to_string(),
                delegation_envelope: None,
            });

            tracing::info!("Queued continuation {cont_id} for resumption");

            // Move to completed
            let done_dir = cont_dir.join("completed");
            let _ = std::fs::create_dir_all(&done_dir);
            let _ = std::fs::rename(&path, done_dir.join(entry.file_name()));
        }
    }

    /// Scan workspace/schedules/ for due schedules and push them into the inbox.
    pub(super) fn scan_schedules(&mut self) {
        let schedules_dir = self.workspace.join("schedules");

        let entries = match std::fs::read_dir(&schedules_dir) {
            Ok(e) => e,
            Err(_) => return, // No schedules dir = nothing to do
        };

        let now = chrono::Utc::now();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read schedule {}: {e}", path.display());
                    continue;
                }
            };

            let mut schedule: crate::tools::schedule_tools::Schedule =
                match serde_json::from_str(&content) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to parse schedule {}: {e}", path.display());
                        continue;
                    }
                };

            if !schedule.enabled {
                continue;
            }

            // Check if the schedule is due
            let next_run = match chrono::DateTime::parse_from_rfc3339(&schedule.next_run) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("Failed to parse next_run for schedule {}: {e}", schedule.id);
                    continue;
                }
            };

            if next_run > now {
                continue; // Not due yet
            }

            tracing::info!(
                "Schedule {} is due: '{}'",
                schedule.id,
                schedule.description
            );

            self.inbox.push(InboxItem {
                message: schedule.description.clone(),
                priority: TaskPriority::Normal,
                session_id: schedule.conversation_id.clone(),
                model_override: None,
                submitted_at: chrono::Utc::now(),
                reply_to: None,
                max_iterations: None,
                origin: "schedule".to_string(),
                delegation_envelope: None,
            });

            // Update schedule for next run (depends on schedule type)
            schedule.last_run = Some(now.to_rfc3339());
            schedule.run_count += 1;

            // Record this run in history
            {
                use crate::tools::schedule_tools::ScheduleRun;
                let run_ref = format!("{}-run-{}", schedule.id, schedule.run_count);
                schedule.run_history.push(ScheduleRun {
                    task_id: run_ref,
                    started_at: now.to_rfc3339(),
                    completed_at: None,
                    status: "triggered".to_string(),
                    sats_spent: 0,
                    iterations: 0,
                });
                // Cap history at 50 entries
                if schedule.run_history.len() > 50 {
                    let excess = schedule.run_history.len() - 50;
                    schedule.run_history.drain(0..excess);
                }
            }

            use crate::tools::schedule_tools::ScheduleType;
            if schedule.one_shot || schedule.schedule_type == ScheduleType::Once {
                // One-shot: disable after first run
                schedule.enabled = false;
                tracing::info!("One-shot schedule {} disabled after run", schedule.id);
            } else if schedule.schedule_type == ScheduleType::Cron {
                // Cron: compute next from expression
                if let Some(ref expr) = schedule.cron_expression {
                    match crate::tools::schedule_tools::compute_next_cron_run(expr) {
                        Ok(next) => schedule.next_run = next.to_rfc3339(),
                        Err(e) => {
                            tracing::warn!(
                                "Cron error for schedule {}: {e} — disabling",
                                schedule.id
                            );
                            schedule.enabled = false;
                        }
                    }
                } else {
                    tracing::warn!(
                        "Cron schedule {} has no expression — disabling",
                        schedule.id
                    );
                    schedule.enabled = false;
                }
            } else {
                // Interval (default): next_run = now + interval_secs
                schedule.next_run =
                    (now + chrono::Duration::seconds(schedule.interval_secs as i64)).to_rfc3339();
            }

            // Write back updated schedule (atomic: write .tmp then rename)
            match serde_json::to_string_pretty(&schedule) {
                Ok(updated) => {
                    let tmp_path = path.with_extension("json.tmp");
                    if let Err(e) = std::fs::write(&tmp_path, &updated) {
                        tracing::warn!("Failed to write schedule tmp {}: {e}", schedule.id);
                    } else if let Err(e) = std::fs::rename(&tmp_path, &path) {
                        tracing::warn!("Failed to rename schedule tmp {}: {e}", schedule.id);
                        // Clean up orphaned .tmp file
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to serialize schedule {}: {e}", schedule.id);
                }
            }
        }
    }
}
