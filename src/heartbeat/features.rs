//! Feature-like capabilities layered on the scheduler.
//!
//! - Active hours enforcement (`is_within_active_hours`)
//! - Heartbeat checklist parsing (`read_heartbeat_checklist`)
//! - Turn-taking protocol (`should_respond_to_message`)
//! - Checklist check, reflection scan, stale task reaping, and memory maintenance (methods on `Scheduler`)

use std::path::Path;
use std::sync::atomic::Ordering;

use crate::config::{ActiveHours, MemoryConfig};
use crate::error::DmError;
use crate::memory::search::MemoryIndex;
use crate::memory::store::{MemoryCategory, MemoryStore};

use super::wake::HEARTBEAT_CONVERSATION_ID;
use super::{InboxItem, Scheduler, TaskPriority};

// ---------------------------------------------------------------------------
// Active hours check
// ---------------------------------------------------------------------------

/// Check if the current time is within the configured active hours window.
/// Returns `true` if no active hours are configured (always active).
/// Gracefully degrades on invalid input (returns `true`).
pub fn is_within_active_hours(active_hours: &ActiveHours) -> bool {
    is_within_active_hours_at(active_hours, chrono::Utc::now())
}

/// Testable variant: check if `now` falls within the active hours window.
pub fn is_within_active_hours_at(
    active_hours: &ActiveHours,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let (start_str, end_str) = match (&active_hours.start, &active_hours.end) {
        (Some(s), Some(e)) => (s.as_str(), e.as_str()),
        _ => return true, // Not configured = always active
    };

    let start = match chrono::NaiveTime::parse_from_str(start_str, "%H:%M") {
        Ok(t) => t,
        Err(_) => {
            tracing::warn!("Invalid active_hours start: {start_str}");
            return true;
        }
    };
    let end = match chrono::NaiveTime::parse_from_str(end_str, "%H:%M") {
        Ok(t) => t,
        Err(_) => {
            tracing::warn!("Invalid active_hours end: {end_str}");
            return true;
        }
    };

    let now_time = if let Some(tz_str) = &active_hours.timezone {
        match tz_str.parse::<chrono_tz::Tz>() {
            Ok(tz) => now.with_timezone(&tz).time(),
            Err(_) => {
                tracing::warn!("Unknown timezone: {tz_str}");
                return true;
            }
        }
    } else {
        now.time() // Default to UTC
    };

    if start <= end {
        // Normal range: e.g., 09:00..22:00
        now_time >= start && now_time < end
    } else {
        // Overnight range: e.g., 22:00..06:00
        now_time >= start || now_time < end
    }
}

// ---------------------------------------------------------------------------
// Heartbeat checklist
// ---------------------------------------------------------------------------

/// Read the heartbeat checklist file and return a task description if it has meaningful content.
/// Returns `None` if disabled, missing, empty, or only whitespace/headers.
pub fn read_heartbeat_checklist(
    checklist_file: &Option<String>,
    workspace: &std::path::Path,
) -> Option<String> {
    let filename = match checklist_file {
        Some(f) if !f.is_empty() && f != "none" => f,
        _ => return None, // Disabled
    };

    let checklist_path = workspace.join(filename);
    let content = std::fs::read_to_string(&checklist_path).ok()?;

    // Empty file or only whitespace/headers = skip
    let meaningful = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .count();
    if meaningful == 0 {
        return None;
    }

    Some(format!(
        "HEARTBEAT_CHECK\n\
         Run the following proactive checks. If nothing needs attention, respond with HEARTBEAT_OK.\n\n\
         {content}"
    ))
}

// ---------------------------------------------------------------------------
// Turn-taking protocol for MessageBox exchanges
// ---------------------------------------------------------------------------

/// Check whether we should respond to an incoming MessageBox message.
///
/// Message types and their handling:
/// - `task_result`    → delivery receipt, NOT actionable (return false)
/// - `status_update`  → informational progress, NOT actionable (return false)
/// - `coordination`   → peer metadata, NOT actionable (return false)
/// - `agent_message`  → apply turn-taking protocol (turn/max_turns/done)
/// - unstructured     → backwards-compatible, allow (return true)
///
/// Only `agent_message` and unstructured messages should spawn tasks.
/// Task results, status updates, and coordination signals are metadata
/// about existing exchanges — they should be logged, not turned into work.
pub fn should_respond_to_message(body: &serde_json::Value) -> bool {
    let msg_type = body.get("type").and_then(|v| v.as_str());

    match msg_type {
        // Task results are delivery receipts — the result of work we requested.
        // They should be matched to the originating exchange, never spawn new tasks.
        Some("task_result") => {
            tracing::info!("MessageBox: skipping task_result — delivery receipt, not actionable");
            false
        }

        // Status updates are progress reports on in-flight work.
        Some("status_update") => {
            tracing::info!("MessageBox: skipping status_update — informational, not actionable");
            false
        }

        // Coordination signals are peer discovery/heartbeat metadata.
        Some("coordination") => {
            tracing::info!("MessageBox: skipping coordination signal — metadata, not actionable");
            false
        }

        // Agent messages: apply turn-taking protocol
        Some("agent_message") => {
            // Check done signal
            if body.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                tracing::info!("MessageBox: ignoring agent_message — sender indicated done");
                return false;
            }

            // Check turn limit
            let turn = body.get("turn").and_then(|v| v.as_u64()).unwrap_or(0);
            let max_turns = body.get("max_turns").and_then(|v| v.as_u64()).unwrap_or(5);
            if turn >= max_turns {
                tracing::info!(
                    "MessageBox: ignoring agent_message — max turns reached ({turn}/{max_turns})"
                );
                return false;
            }

            true
        }

        // Unstructured or unknown message types: allow for backwards compatibility.
        // This covers plain text messages and any new types we haven't seen yet.
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Trust tiers for agent-to-agent messaging
// ---------------------------------------------------------------------------

/// Trust level for an incoming message sender.
///
/// Determines fee requirements and tool access restrictions.
/// Phase 1: only Family and Unknown are used.
/// Phase 2+: Vouched, Discovered, and Stranger tiers with full cert verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustTier {
    /// Same certifier (e.g., both agents issued certs by the same parent).
    /// Full trust, no fee, full tool access.
    Family,
    /// Cert signed by a certifier in our `trusted_certifiers` list.
    /// Low fee, full tool access. (Phase 2)
    Vouched,
    /// Valid cert from an unknown certifier. Found via overlay discovery.
    /// Standard fee (100 sats), restricted tool access. (Phase 2)
    Discovered,
    /// No cert, self-signed, or expired/revoked cert.
    /// High fee (1000+ sats) or block. (Phase 2)
    Stranger,
    /// Cannot determine trust level (no cert info available).
    /// Treated as Stranger for fee purposes, but allowed in Phase 1
    /// for backwards compatibility.
    Unknown,
}

/// Check the trust tier for a message sender.
///
/// Phase 1 implementation: checks if the sender's certifier matches our own
/// parent (Family) or is in the trusted_certifiers list (Vouched). Everything
/// else is Unknown.
///
/// Phase 2 will add cert verification via prove_authorization/verify flow.
pub fn check_sender_trust(
    sender_key: &str,
    parent_key: &str,
    trusted_certifiers: &[String],
) -> TrustTier {
    if sender_key.is_empty() {
        return TrustTier::Unknown;
    }

    // Family: sender IS our parent (operator sending commands)
    if !parent_key.is_empty() && sender_key == parent_key {
        return TrustTier::Family;
    }

    // Vouched: sender is in our trusted_certifiers list
    // (Phase 1: direct key match. Phase 2: check sender's cert certifier field)
    if trusted_certifiers.iter().any(|k| k == sender_key) {
        return TrustTier::Vouched;
    }

    // Phase 2 will add:
    // - Check sender's cert proof (if attached to message) for certifier field
    // - If certifier == our parent → Family
    // - If certifier in trusted_certifiers → Vouched
    // - If valid cert from unknown certifier → Discovered
    // - If no cert or invalid → Stranger

    TrustTier::Unknown
}

// ---------------------------------------------------------------------------
// Scheduler feature methods
// ---------------------------------------------------------------------------

impl Scheduler {
    /// Read the heartbeat checklist file and push a proactive check task if it has content.
    /// Dedup: skip if the same content was already processed within the cooldown period.
    pub(super) fn check_heartbeat_file(&mut self) {
        if let Some(task_desc) =
            read_heartbeat_checklist(&self.config.heartbeat.checklist_file, &self.workspace)
        {
            // Compute hash of content for dedup
            use sha2::{Digest, Sha256};
            let content_hash = hex::encode(Sha256::digest(task_desc.as_bytes()));

            // Skip if same content within cooldown
            let cooldown =
                std::time::Duration::from_secs(self.config.heartbeat.checklist_cooldown_secs);
            if let (Some(ref last_hash), Some(last_time)) =
                (&self.last_checklist_hash, self.last_checklist_time)
            {
                if *last_hash == content_hash && last_time.elapsed() < cooldown {
                    tracing::debug!(
                        "Scheduler: heartbeat checklist unchanged within cooldown, skipping"
                    );
                    return;
                }
            }

            tracing::info!("Scheduler: heartbeat checklist has content, queueing proactive check");
            self.last_checklist_hash = Some(content_hash);
            self.last_checklist_time = Some(std::time::Instant::now());
            self.inbox.push(InboxItem {
                message: task_desc,
                priority: TaskPriority::Low,
                session_id: Some(HEARTBEAT_CONVERSATION_ID.to_string()),
                model_override: None,
                submitted_at: chrono::Utc::now(),
                reply_to: None,
                max_iterations: None,
                origin: "checklist".to_string(),
                delegation_envelope: None,
            });
        }
    }

    /// Check if an autonomous reflection task should be submitted.
    ///
    /// Guards:
    /// 1. Feature must be enabled (`reflection_enabled`)
    /// 2. Interval must have elapsed since last reflection
    /// 3. No reflection task currently running (session_id "reflection" not active)
    pub(super) fn scan_reflection(&mut self) {
        if !self.config.heartbeat.reflection_enabled {
            return;
        }

        // Check interval
        if let Some(last) = self.last_reflection {
            if last.elapsed().as_secs() < self.config.heartbeat.reflection_interval_secs {
                return;
            }
        }

        // Check if a reflection task is already running via semaphore permit availability
        if let Ok(sems) = self.app_state.task_mgr.conversation_semaphores.try_lock() {
            if let Some(sem) = sems.get("reflection") {
                if sem.available_permits() == 0 {
                    tracing::debug!("Scheduler: skipping reflection — already running");
                    return;
                }
            }
        }

        let active = self
            .app_state
            .task_mgr
            .active_task_count
            .load(Ordering::Relaxed);
        let queued = self.inbox.len();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let prompt = format!(
            "REFLECTION\n\
             Review current agent state and reflect on recent activity.\n\
             Active tasks: {active}, queued: {queued}. Time: {now}\n\
             If nothing needs attention, respond with REFLECTION_OK."
        );

        tracing::info!("Scheduler: submitting autonomous reflection task");

        self.inbox.push(InboxItem {
            message: prompt,
            priority: TaskPriority::Lowest,
            session_id: Some("reflection".to_string()),
            model_override: Some(self.config.heartbeat.reflection_model.clone()),
            submitted_at: chrono::Utc::now(),
            reply_to: None,
            max_iterations: Some(self.config.heartbeat.reflection_max_iterations),
            origin: "reflection".to_string(),
            delegation_envelope: None,
        });

        self.last_reflection = Some(std::time::Instant::now());
    }

    /// Check if the agent's parent-signed certificate has been revoked.
    ///
    /// Called periodically (every 10 minutes). If the revocation UTXO has been
    /// spent, sets the `certificate_revoked` flag on AppState to block new tasks.
    /// The operator can re-issue a certificate via POST /certificates/issue.
    pub(super) async fn check_certificate_revocation(&self) {
        let wallet = self.app_state.wallet.clone();
        let mgr = crate::certificates::CertificateManager::new(wallet);

        // Only check revocation if we have a parent-signed cert.
        // Self-signed certs are the default — never block on those.
        let status = match mgr.certificate_status().await {
            Ok(s) if s.status == "parent-signed" => s,
            _ => return, // self-signed or no cert — nothing to revoke
        };

        if let Some(ref cert) = status.certificate {
            match mgr.is_revoked(cert).await {
                Ok(true) => {
                    self.app_state
                        .certificate_revoked
                        .store(true, Ordering::SeqCst);
                    tracing::error!(
                        "CERTIFICATE REVOKED: parent has revoked agent authorization. \
                         Tasks blocked until re-authorized via POST /certificates/issue."
                    );
                }
                Ok(false) => {
                    // Cert valid — ensure flag is cleared (covers re-issue scenario)
                    self.app_state
                        .certificate_revoked
                        .store(false, Ordering::SeqCst);
                    tracing::debug!("Certificate revocation check: valid");
                }
                Err(e) => {
                    tracing::warn!("Certificate revocation check failed: {e}");
                }
            }
        }
    }

    /// Sweep stale UTXO tokens if enough time has elapsed since the last sweep.
    ///
    /// Respects `LifecycleConfig`:
    /// - `auto_sweep_enabled` — guards this method
    /// - `sweep_interval_minutes` — minimum time between sweeps
    /// - `budget_token_max_age_hours` — max age for BudgetAllocation tokens
    /// - `checkpoint_max_age_hours` — max age for Checkpoint tokens
    ///
    /// BRC-18 proofs in `worm-proofs` are NEVER swept (immutable audit trail).
    /// When compliance mode is enabled, retention period is respected.
    /// When WORM mode is enabled, proofs basket is additionally protected.
    pub(super) async fn maybe_sweep_tokens(&mut self) {
        if !self.config.lifecycle.auto_sweep_enabled {
            return;
        }

        let interval_secs = self.config.lifecycle.sweep_interval_minutes * 60;
        let should_sweep = match self.last_sweep {
            None => true,
            Some(last) => last.elapsed().as_secs() >= interval_secs,
        };
        if !should_sweep {
            return;
        }

        let wallet = self.app_state.wallet.clone();
        let budget_max_age_secs = (self.config.lifecycle.budget_token_max_age_hours * 3600) as i64;
        let checkpoint_max_age_secs =
            (self.config.lifecycle.checkpoint_max_age_hours * 3600) as i64;

        // Compliance retention: if enabled, enforce minimum retention period on sweeps
        let compliance_retention_secs = if self.config.compliance.enabled {
            Some((self.config.compliance.default_retention_days * 86400) as i64)
        } else {
            None
        };

        let budget_swept = match crate::onchain::state::sweep_stale_tokens_with_retention(
            &*wallet,
            crate::onchain::state::BASKET_BUDGET,
            budget_max_age_secs,
            Some("budget_allocation"),
            compliance_retention_secs,
        )
        .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("Budget token sweep failed: {e}");
                0
            }
        };

        // Sweep only old checkpoint tokens from worm-state basket.
        // TaskCommitment and CapabilityDeclaration tokens are intentionally
        // preserved as historical on-chain state — they document what the agent
        // committed to and what capabilities it had for each task.
        let checkpoint_swept = match crate::onchain::state::sweep_stale_tokens_with_retention(
            &*wallet,
            crate::onchain::state::BASKET_STATE,
            checkpoint_max_age_secs,
            Some("checkpoint"),
            compliance_retention_secs,
        )
        .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("Checkpoint token sweep failed: {e}");
                0
            }
        };

        if budget_swept > 0 || checkpoint_swept > 0 {
            tracing::info!(
                "Token lifecycle sweep: {budget_swept} budget tokens, {checkpoint_swept} checkpoints"
            );
        }

        self.last_sweep = Some(std::time::Instant::now());
    }

    /// Log basket sizes periodically for capacity planning.
    ///
    /// Fetches UTXO counts for all three baskets and logs at INFO level.
    /// Includes a growth rate estimate for the proofs basket (proofs/day)
    /// based on current count and oldest proof age.
    ///
    /// Controlled by `LifecycleConfig::basket_monitoring_interval_secs`.
    /// 0 = disabled.
    pub(super) async fn maybe_log_basket_status(&mut self) {
        let interval = self.config.lifecycle.basket_monitoring_interval_secs;
        if interval == 0 {
            return;
        }

        let should_log = match self.last_basket_log {
            None => true,
            Some(last) => last.elapsed().as_secs() >= interval,
        };
        if !should_log {
            return;
        }

        let wallet = self.app_state.wallet.clone();

        let (state_count, _) =
            crate::onchain::state::basket_status(&*wallet, crate::onchain::state::BASKET_STATE)
                .await;
        let (budget_count, _) =
            crate::onchain::state::basket_status(&*wallet, crate::onchain::state::BASKET_BUDGET)
                .await;
        let (proofs_count, proofs_oldest) =
            crate::onchain::state::basket_status(&*wallet, crate::onchain::state::BASKET_PROOFS)
                .await;

        let growth_rate = estimate_proofs_per_day(proofs_count, proofs_oldest);
        let growth_str = match growth_rate {
            Some(rate) => format!(", proofs_per_day={rate:.1}"),
            None => String::new(),
        };

        tracing::info!(
            "Basket monitoring: state={state_count}, budget={budget_count}, \
             proofs={proofs_count}{growth_str}"
        );

        self.last_basket_log = Some(std::time::Instant::now());
    }

    /// Detect and clean up tasks that have been "running" for too long without
    /// making any progress. This catches tasks where `run()` hung (e.g., wallet
    /// offline, LLM unreachable) or the spawned future panicked silently.
    pub(super) async fn reap_stale_tasks(&self) {
        const STALE_THRESHOLD_SECS: u64 = 30 * 60; // 30 minutes

        let now = chrono::Utc::now();
        let mut tasks = self.app_state.task_mgr.tasks.lock().await;
        let mut reaped = Vec::new();

        for (id, info) in tasks.iter_mut() {
            if info.status != crate::server::TaskStatus::Running {
                continue;
            }
            // Parse started_at and check staleness
            let started = match chrono::DateTime::parse_from_rfc3339(&info.started_at) {
                Ok(dt) => dt.with_timezone(&chrono::Utc),
                Err(_) => continue,
            };
            let age_secs = (now - started).num_seconds();
            if age_secs < STALE_THRESHOLD_SECS as i64 {
                continue;
            }
            // Task is stale: stuck at 0 iterations for >30 min
            if info.iterations == 0 {
                tracing::warn!(
                    "Reaping stale task {} — running for {}s with 0 iterations",
                    id,
                    age_secs
                );
                info.status = crate::server::TaskStatus::Error;
                info.error = Some(format!(
                    "Task reaped: no progress after {}s (likely hung on wallet/LLM)",
                    age_secs
                ));
                info.completed_at = Some(now.to_rfc3339());
                reaped.push(id.clone());
            }
        }
        drop(tasks);

        // Decrement active task count for reaped tasks and clean up cancel flags
        for id in &reaped {
            self.app_state
                .task_mgr
                .active_task_count
                .fetch_sub(1, Ordering::Relaxed);
            let mut flags = self.app_state.task_mgr.cancel_flags.lock().await;
            flags.remove(id);
        }
    }

    /// Run periodic memory maintenance if enough time has elapsed.
    ///
    /// Guards:
    /// 1. Feature must be enabled (`memory.maintenance_enabled`)
    /// 2. Interval must have elapsed since last maintenance run
    ///
    /// Non-fatal: logs and continues on error.
    pub(super) async fn maybe_run_memory_maintenance(&mut self) {
        if !self.config.memory.maintenance_enabled {
            return;
        }

        let interval_secs = self.config.memory.maintenance_interval_secs;
        let should_run = match self.last_memory_maintenance {
            None => true,
            Some(last) => last.elapsed().as_secs() >= interval_secs,
        };
        if !should_run {
            return;
        }

        let memory_dir = std::path::PathBuf::from(&self.config.memory.base_dir);
        match run_memory_maintenance(&memory_dir, &self.config.memory) {
            Ok(summary) => {
                tracing::info!(
                    "Memory maintenance: {} stale session(s) flagged, {} duplicate pair(s), \
                     {} total entries ({} knowledge, {} session, {} execution)",
                    summary.stale_sessions_flagged,
                    summary.duplicate_pairs,
                    summary.total_entries,
                    summary.knowledge_count,
                    summary.session_count,
                    summary.execution_count,
                );
            }
            Err(e) => {
                tracing::warn!("Memory maintenance failed (non-fatal): {e}");
            }
        }

        self.last_memory_maintenance = Some(std::time::Instant::now());
    }
}

// ---------------------------------------------------------------------------
// Growth rate estimation
// ---------------------------------------------------------------------------

/// Estimate proofs per day from total count and oldest proof age.
///
/// Returns `None` if there are no proofs or no age data.
/// The estimate is an average: `count * 24.0 / oldest_age_hours`.
pub fn estimate_proofs_per_day(count: u64, oldest_age_hours: Option<f64>) -> Option<f64> {
    if count == 0 {
        return Some(0.0);
    }
    let hours = oldest_age_hours?;
    if hours < 0.01 {
        return None; // Too recent to estimate
    }
    Some(count as f64 * 24.0 / hours)
}

// ---------------------------------------------------------------------------
// Memory maintenance
// ---------------------------------------------------------------------------

/// Summary of a memory maintenance run.
#[derive(Debug, Clone, Default)]
pub struct MaintenanceSummary {
    /// Total memory entries across all categories.
    pub total_entries: usize,
    /// Number of knowledge entries.
    pub knowledge_count: usize,
    /// Number of session entries.
    pub session_count: usize,
    /// Number of execution entries.
    pub execution_count: usize,
    /// Number of stale session/execution entries flagged (older than stale_session_days).
    pub stale_sessions_flagged: usize,
    /// Number of near-duplicate pairs found via BM25 similarity.
    pub duplicate_pairs: usize,
    /// IDs of entries flagged as stale.
    pub stale_ids: Vec<String>,
    /// Pairs of IDs identified as near-duplicates (id_a, id_b, similarity_score).
    pub duplicate_id_pairs: Vec<(String, String, f32)>,
}

/// Run memory maintenance: flag stale sessions, detect duplicates, report health.
///
/// This function is conservative — it NEVER deletes anything. It only flags and
/// logs findings. Knowledge entries are NEVER flagged as stale.
///
/// Returns a `MaintenanceSummary` with counts and flagged IDs.
pub fn run_memory_maintenance(
    memory_dir: &Path,
    config: &MemoryConfig,
) -> Result<MaintenanceSummary, DmError> {
    let store = MemoryStore::new(memory_dir.to_path_buf());
    let mut summary = MaintenanceSummary::default();

    // Load all entries
    let all_entries = store.list(None)?;
    summary.total_entries = all_entries.len();

    // Count by category
    for entry in &all_entries {
        match entry.category {
            MemoryCategory::Knowledge => summary.knowledge_count += 1,
            MemoryCategory::Session => summary.session_count += 1,
            MemoryCategory::Execution => summary.execution_count += 1,
        }
    }

    // Operation 1: Flag stale session/execution memories
    let now = chrono::Utc::now();
    let stale_threshold = chrono::Duration::days(config.stale_session_days as i64);

    for entry in &all_entries {
        // Knowledge entries are NEVER flagged as stale
        if entry.category == MemoryCategory::Knowledge {
            continue;
        }

        let age = now.signed_duration_since(entry.created);
        if age > stale_threshold {
            tracing::info!(
                "Memory maintenance: stale {} entry '{}' (age: {} days)",
                entry.category,
                entry.id,
                age.num_days(),
            );
            summary.stale_sessions_flagged += 1;
            summary.stale_ids.push(entry.id.clone());
        }
    }

    // Operation 2: Log duplicates via BM25 similarity
    // Build a temporary index for dedup scanning under the memory base dir
    let index_path = memory_dir.join(".maintenance-index");
    let index = MemoryIndex::new(index_path)?;
    index.rebuild(&all_entries)?;

    // For each entry, search for similar entries and flag pairs with high overlap
    let mut seen_pairs = std::collections::HashSet::new();
    for entry in &all_entries {
        // Use the first 200 chars as the query to avoid BM25 dilution
        let query_text = crate::memory::search::extract_key_terms(
            &entry.content.chars().take(200).collect::<String>(),
        );
        if query_text.trim().is_empty() {
            continue;
        }

        let results = match index.search(&query_text, 5) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for result in &results {
            // Skip self-match
            if result.id == entry.id {
                continue;
            }

            // Canonical pair key (sorted) to avoid double-counting
            let pair_key = if entry.id < result.id {
                (entry.id.clone(), result.id.clone())
            } else {
                (result.id.clone(), entry.id.clone())
            };

            if seen_pairs.contains(&pair_key) {
                continue;
            }

            // Check Jaccard similarity for near-duplicate detection
            let similarity =
                crate::memory::search::jaccard_similarity(&entry.content, &result.content);

            if similarity > 0.6 {
                tracing::info!(
                    "Memory maintenance: near-duplicate pair ({}, {}) — similarity {:.2}",
                    entry.id,
                    result.id,
                    similarity,
                );
                seen_pairs.insert(pair_key);
                summary.duplicate_pairs += 1;
                summary.duplicate_id_pairs.push((
                    entry.id.clone(),
                    result.id.clone(),
                    similarity as f32,
                ));
            }
        }
    }

    Ok(summary)
}
