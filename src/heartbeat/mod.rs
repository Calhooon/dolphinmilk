//! Scheduler — priority-aware inbox, concurrency-limited task spawner, and event-driven wake.
//!
//! Replaces the simple heartbeat poller with a proper scheduler. The scheduler:
//! - Polls MessageBox inboxes for new messages (priority: Normal)
//! - Scans continuations directory for paused tasks to resume (priority: Low)
//! - Maintains a priority queue sorted by priority then submission time
//! - Limits concurrent tasks via `active_task_count` on AppState
//! - Uses `tokio::select!` with 250ms coalescing window for event-driven wake
//!
//! The `HeartbeatDaemon` type alias and constructor are preserved for backward compatibility
//! with existing tests.

pub mod commission_payments;
pub mod features;
pub mod sources;
pub mod wake;

// Re-export all public items so `crate::heartbeat::Foo` continues to work.
pub use features::{
    check_sender_trust, estimate_proofs_per_day, is_within_active_hours, is_within_active_hours_at,
    read_heartbeat_checklist, run_memory_maintenance, should_respond_to_message,
    MaintenanceSummary, TrustTier,
};
pub use wake::{SystemEvent, WakeReason, WakeRequest, HEARTBEAT_CONVERSATION_ID};

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::time::{interval, Duration};

use crate::auth::AuthriteClient;
use crate::config::DmConfig;
use crate::messagebox::client::MessageBoxClient;

// ---------------------------------------------------------------------------
// Task priority and inbox item
// ---------------------------------------------------------------------------

/// Priority levels for queued tasks. Higher priority items are dequeued first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Unknown sender — lowest priority.
    Lowest = 0,
    /// Self-generated (continuation, follow-up).
    Low = 1,
    /// Known agent, scheduled task, MessageBox message.
    Normal = 2,
    /// Parent (human owner) — highest priority.
    Highest = 3,
}

/// An item in the scheduler's priority inbox.
#[derive(Debug, Clone)]
pub struct InboxItem {
    pub message: String,
    pub priority: TaskPriority,
    pub session_id: Option<String>,
    pub model_override: Option<String>,
    pub submitted_at: chrono::DateTime<chrono::Utc>,
    /// Sender identity key for reply routing via MessageBox.
    /// When set, the task result is sent back to this key after completion.
    pub reply_to: Option<String>,
    /// Override the default max iterations (50) for this task.
    /// Used by reflection tasks to cap iterations at a low number.
    pub max_iterations: Option<u32>,
    /// How this task was spawned (e.g. "message", "schedule", "continuation", "reflection", "checklist", "trigger").
    pub origin: String,
    /// Full delegation envelope from an inbound `task_delegation` MessageBox body,
    /// untruncated. Populated by `poll_messagebox()` when `msg.body.type ==
    /// "task_delegation"`. `None` for all other inbox origins. Threaded through
    /// `spawn_task()` → `WormLoop::set_pending_delegation()` so the runner can
    /// parse the cert + chain and verify it at task setup.
    pub delegation_envelope: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Priority inbox
// ---------------------------------------------------------------------------

/// A simple priority queue backed by a sorted Vec.
///
/// Items are sorted by priority (highest first), then by submission time
/// (oldest first within the same priority). The queue is typically small
/// (<10 items), so a linear insert is efficient enough.
#[derive(Debug, Default)]
pub struct PriorityInbox {
    items: Vec<InboxItem>,
}

impl PriorityInbox {
    /// Create a new empty inbox.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Insert an item in priority order.
    ///
    /// Items with higher priority come first. Within the same priority,
    /// items are ordered by submission time (oldest first — FIFO).
    pub fn push(&mut self, item: InboxItem) {
        // Find the insertion point: after all items with higher or same-priority-and-older-time.
        let pos = self.items.partition_point(|existing| {
            if existing.priority != item.priority {
                // Higher priority comes first (greater enum value = higher priority)
                existing.priority > item.priority
            } else {
                // Same priority: older items first
                existing.submitted_at <= item.submitted_at
            }
        });
        self.items.insert(pos, item);
    }

    /// Remove and return the highest-priority (and oldest within that priority) item.
    pub fn pop(&mut self) -> Option<InboxItem> {
        if self.items.is_empty() {
            None
        } else {
            Some(self.items.remove(0))
        }
    }

    /// Number of items in the queue.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Scheduler (replaces HeartbeatDaemon)
// ---------------------------------------------------------------------------

/// Background scheduler that polls MessageBox inboxes, manages a priority
/// inbox, and spawns tasks with concurrency limiting and event-driven wake.
pub struct Scheduler {
    pub(crate) config: DmConfig,
    pub(crate) messagebox: std::sync::Arc<MessageBoxClient>,
    pub(crate) workspace: PathBuf,
    pub(crate) inbox: PriorityInbox,
    pub(crate) max_concurrent: usize,
    /// Shared reference to the application state (for spawn_task and adaptive tick).
    pub(crate) app_state: Arc<crate::server::AppState>,
    /// Timestamp of last reflection task submission.
    pub(crate) last_reflection: Option<std::time::Instant>,
    /// SHA-256 hash of the last processed HEARTBEAT.md content.
    pub(crate) last_checklist_hash: Option<String>,
    /// When the last checklist was processed.
    pub(crate) last_checklist_time: Option<std::time::Instant>,
    /// Receiver for wake requests from event sources.
    pub(crate) wake_rx: tokio::sync::mpsc::Receiver<WakeRequest>,
    /// Persistent delivery queue for failed outbound MessageBox messages.
    pub(crate) delivery_queue: crate::delivery::DeliveryQueue,
    /// Config hot-reload receiver (cloned from AppState). `None` when watcher is disabled.
    pub(crate) config_rx: Option<tokio::sync::watch::Receiver<DmConfig>>,
    /// Timestamp of last certificate revocation check (every 10 minutes).
    pub(crate) last_revocation_check: Option<std::time::Instant>,
    /// Timestamp of last UTXO lifecycle sweep.
    pub(crate) last_sweep: Option<std::time::Instant>,
    /// Timestamp of last memory maintenance run.
    pub(crate) last_memory_maintenance: Option<std::time::Instant>,
    /// Timestamp of last basket monitoring log.
    pub(crate) last_basket_log: Option<std::time::Instant>,
}

impl Scheduler {
    /// Create a new scheduler from config with access to shared app state.
    ///
    /// Returns `(Scheduler, Sender)` — distribute the sender to event sources
    /// so they can wake the scheduler immediately.
    pub fn new(
        config: DmConfig,
        workspace: PathBuf,
        app_state: Arc<crate::server::AppState>,
    ) -> (Self, tokio::sync::mpsc::Sender<WakeRequest>) {
        let messagebox = app_state.messagebox.clone();
        let max_concurrent = config.heartbeat.max_concurrent_tasks;
        let (wake_tx, wake_rx) = tokio::sync::mpsc::channel::<WakeRequest>(64);
        let delivery_queue = crate::delivery::DeliveryQueue::new(&workspace);
        let pending = delivery_queue.scan_on_startup();
        if pending > 0 {
            tracing::info!(
                "DeliveryQueue: {} pending delivery/deliveries recovered from previous run",
                pending
            );
        }
        let config_rx = app_state.config_rx.clone();
        (
            Self {
                config,
                messagebox,
                workspace,
                inbox: PriorityInbox::new(),
                max_concurrent,
                app_state,
                last_reflection: None,
                last_checklist_hash: None,
                last_checklist_time: None,
                wake_rx,
                delivery_queue,
                config_rx,
                last_revocation_check: None,
                last_sweep: None,
                last_memory_maintenance: None,
                last_basket_log: None,
            },
            wake_tx,
        )
    }

    /// Run the scheduler loop. Designed to be `tokio::spawn`'d.
    ///
    /// Uses `tokio::select!` to wake on either:
    /// - A wake request from an event source (task completed, message received, etc.)
    /// - A timer fallback using the configured `inbox_poll_secs`
    ///
    /// When a wake request arrives, a 250ms coalescing window batches additional
    /// concurrent events into one processing pass — deduplicating by `WakeReason`.
    pub async fn run(mut self) {
        let base_interval = Duration::from_secs(self.config.heartbeat.inbox_poll_secs);
        tracing::info!(
            "Scheduler started: base interval {}s, max {} concurrent tasks",
            base_interval.as_secs(),
            self.max_concurrent,
        );

        loop {
            let mut pending: Vec<WakeRequest> = Vec::new();

            // Drain any buffered system events first (non-blocking)
            self.drain_system_events(&mut pending).await;

            if pending.is_empty() {
                // Nothing buffered — block on either wake_rx or timer
                tokio::select! {
                    // A wake request arrived — collect it and start the coalescing window
                    Some(req) = self.wake_rx.recv() => {
                        pending.push(req);
                    }
                    // Timer fallback: fire a TimerExpired wake if no events arrive
                    _ = tokio::time::sleep(base_interval) => {
                        pending.push(WakeRequest {
                            reason: WakeReason::TimerExpired,
                            priority: 1,
                            timestamp: std::time::Instant::now(),
                        });
                    }
                }
            }

            // Coalescing window: drain additional requests that arrive within 250ms
            let coalesce_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
            loop {
                tokio::select! {
                    Some(r) = self.wake_rx.recv() => { pending.push(r); }
                    _ = tokio::time::sleep_until(coalesce_deadline) => { break; }
                }
            }

            // Drain system events that arrived during the coalescing window
            self.drain_system_events(&mut pending).await;

            // Deduplicate by WakeReason — process each reason at most once per batch
            let mut seen = HashSet::new();
            pending.retain(|r| seen.insert(r.reason.clone()));

            // Sort by priority (lower number = higher urgency)
            pending.sort_by_key(|r| r.priority);

            // Process the batch
            self.process_wake_batch(&pending).await;
        }
    }

    /// Retry any due outbound deliveries from the persistent delivery queue.
    async fn retry_due_deliveries(&self) {
        let due = self.delivery_queue.due_items();
        if due.is_empty() {
            return;
        }
        tracing::info!("DeliveryQueue: {} item(s) due for retry", due.len());

        for delivery in due {
            match self
                .messagebox
                .send_message(&delivery.recipient, &delivery.message_box, &delivery.body)
                .await
            {
                Ok(_) => {
                    tracing::info!("DeliveryQueue: sent {} successfully", delivery.id);
                    self.delivery_queue.mark_success(&delivery.id);
                }
                Err(e) => {
                    self.delivery_queue
                        .mark_failed(&delivery.id, &e.to_string());
                }
            }
        }
    }

    /// Drain buffered system events from `AppState.scheduler.system_event_rx` and convert
    /// each to a `WakeRequest`. Non-blocking — returns immediately if no events
    /// are pending or if the channel doesn't exist.
    async fn drain_system_events(&self, pending: &mut Vec<WakeRequest>) {
        if let Some(rx_arc) = &self.app_state.scheduler.system_event_rx {
            let mut rx_guard = rx_arc.lock().await;
            while let Ok(evt) = rx_guard.try_recv() {
                tracing::debug!("System event: {:?}", evt);
                pending.push(evt.to_wake_request());
            }
        }
    }

    /// Process a coalesced batch of wake requests.
    async fn process_wake_batch(&mut self, _wake_reasons: &[WakeRequest]) {
        // Check for config hot-reload (sub-second detection via notify watcher)
        if let Some(ref mut config_rx) = self.config_rx {
            if config_rx.has_changed().unwrap_or(false) {
                let fresh = config_rx.borrow_and_update().clone();
                self.config.reload_safe_fields(&fresh);
                self.max_concurrent = self.config.heartbeat.max_concurrent_tasks;
                if let Some(ref tx) = self.app_state.scheduler.system_event_tx {
                    let _ = tx.try_send(SystemEvent::ConfigChanged);
                }
                tracing::info!(
                    "Config reloaded: inbox_poll_secs={}, max_concurrent={}, model={}",
                    self.config.heartbeat.inbox_poll_secs,
                    self.config.heartbeat.max_concurrent_tasks,
                    self.config.llm.default_model,
                );
            }
        }

        // Guard 1: Active hours — skip work if outside window
        if !is_within_active_hours(&self.config.heartbeat.active_hours) {
            tracing::debug!("Scheduler: skipping batch — outside active hours");
            return;
        }

        // Guard 2: Skip-if-busy — skip polling/scanning when at capacity
        if !self.has_capacity() {
            tracing::debug!(
                "Scheduler: skipping batch — at capacity ({}/{})",
                self.app_state
                    .task_mgr
                    .active_task_count
                    .load(Ordering::Relaxed),
                self.max_concurrent,
            );
            self.record_liveness_tick();
            return;
        }

        // Run all scan phases (they are idempotent)
        self.poll_messagebox().await;
        self.scan_continuations();
        self.scan_schedules();
        self.check_heartbeat_file();
        self.scan_reflection();

        // Retry any failed outbound deliveries that are due
        self.retry_due_deliveries().await;

        // Drain externally-triggered tasks (from /heartbeat/trigger endpoint)
        if let Some(rx) = &self.app_state.scheduler.heartbeat_rx {
            let mut rx_guard = rx.lock().await;
            while let Ok(msg) = rx_guard.try_recv() {
                self.inbox.push(InboxItem {
                    message: msg,
                    priority: TaskPriority::Highest,
                    session_id: Some(HEARTBEAT_CONVERSATION_ID.to_string()),
                    model_override: None,
                    submitted_at: chrono::Utc::now(),
                    reply_to: None,
                    max_iterations: None,
                    origin: "trigger".to_string(),
                    delegation_envelope: None,
                });
            }
        }

        // Drain inbox while we have capacity
        self.drain_inbox().await;

        // Reap stale tasks: in-memory tasks stuck as "running" with no progress
        self.reap_stale_tasks().await;

        // Periodic UTXO lifecycle sweep
        self.maybe_sweep_tokens().await;

        // Periodic certificate revocation check (every 10 minutes)
        const REVOCATION_CHECK_INTERVAL_SECS: u64 = 600;
        let should_check = match self.last_revocation_check {
            None => true,
            Some(last) => last.elapsed().as_secs() >= REVOCATION_CHECK_INTERVAL_SECS,
        };
        if should_check {
            self.check_certificate_revocation().await;
            self.last_revocation_check = Some(std::time::Instant::now());
        }

        // Periodic memory maintenance (interval from config, default OFF)
        self.maybe_run_memory_maintenance().await;

        // Periodic basket monitoring (capacity planning)
        self.maybe_log_basket_status().await;

        // Record liveness
        self.record_liveness_tick();
    }

    /// Record a liveness tick (epoch seconds) for /health monitoring.
    fn record_liveness_tick(&self) {
        let epoch_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.app_state
            .scheduler
            .last_scheduler_tick
            .store(epoch_secs, Ordering::Relaxed);
    }

    /// Returns whether there is capacity to spawn another task.
    fn has_capacity(&self) -> bool {
        let active = self
            .app_state
            .task_mgr
            .active_task_count
            .load(Ordering::Relaxed);
        active < self.max_concurrent
    }

    /// Drain the inbox, spawning tasks while there is capacity.
    async fn drain_inbox(&mut self) {
        while self.has_capacity() {
            let item = match self.inbox.pop() {
                Some(item) => item,
                None => break,
            };

            let task_id = uuid::Uuid::new_v4().to_string();
            // Use item-level max_iterations if set, otherwise cap checklist tasks at 5
            let max_iterations: u32 =
                item.max_iterations
                    .unwrap_or(if item.message.starts_with("HEARTBEAT_CHECK") {
                        5
                    } else {
                        50
                    });
            tracing::info!(
                "Scheduler spawning task (priority={:?}, max_iter={max_iterations}): {}",
                item.priority,
                &item.message[..item.message.len().min(80)]
            );

            match crate::server::spawn_task(
                &self.app_state,
                task_id,
                item.message.clone(),
                max_iterations,
                item.session_id,
                item.model_override,
                item.reply_to,
                Vec::new(),
                item.origin,
                None,
                item.delegation_envelope,
            )
            .await
            {
                Ok((id, sid)) => {
                    tracing::info!("Scheduler spawned task {id} in conversation {sid}");
                }
                Err(e) => {
                    tracing::error!("Scheduler task spawn failed: {e:?}");
                }
            }
        }

        if !self.inbox.is_empty() {
            tracing::info!(
                "Scheduler: {} item(s) still queued (at capacity: {}/{})",
                self.inbox.len(),
                self.app_state
                    .task_mgr
                    .active_task_count
                    .load(Ordering::Relaxed),
                self.max_concurrent
            );
        }
    }
}

// ---------------------------------------------------------------------------
// HeartbeatDaemon — backward-compatible alias for tests
// ---------------------------------------------------------------------------

/// Backward-compatible type for tests that construct a heartbeat daemon directly.
///
/// The `HeartbeatDaemon` is the original simple poller. It is preserved so that
/// existing tests (which test config parsing, daemon construction, and poll behavior
/// via an mpsc channel) continue to work without modification.
pub struct HeartbeatDaemon {
    config: DmConfig,
    messagebox: MessageBoxClient,
    poll_interval: Duration,
    workspace: PathBuf,
}

impl HeartbeatDaemon {
    /// Create a new heartbeat daemon from config.
    pub fn new(config: DmConfig, workspace: PathBuf) -> Self {
        let wallet: std::sync::Arc<dyn crate::wallet::WalletBackend + Send + Sync> =
            std::sync::Arc::new(crate::wallet::HttpWalletClient::from_config(&config.wallet));
        let auth = AuthriteClient::new(wallet, &config.wallet.url);
        let messagebox = MessageBoxClient::new(auth);
        let poll_interval = Duration::from_secs(config.heartbeat.inbox_poll_secs);
        Self {
            config,
            messagebox,
            poll_interval,
            workspace,
        }
    }

    /// Create with a custom MessageBoxClient (for testing).
    pub fn with_messagebox(
        config: DmConfig,
        messagebox: MessageBoxClient,
        workspace: PathBuf,
    ) -> Self {
        let poll_interval = Duration::from_secs(config.heartbeat.inbox_poll_secs);
        Self {
            config,
            messagebox,
            poll_interval,
            workspace,
        }
    }

    /// Returns the poll interval.
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// Returns whether the heartbeat is enabled in config.
    pub fn is_enabled(&self) -> bool {
        self.config.heartbeat.enabled
    }

    /// Run the heartbeat loop. Designed to be `tokio::spawn`'d.
    ///
    /// Sends task description strings through `task_sender` for each
    /// new inbox message discovered.
    pub async fn run(mut self, task_sender: tokio::sync::mpsc::Sender<String>) {
        tracing::info!(
            "Heartbeat started: polling every {}s",
            self.poll_interval.as_secs()
        );
        let mut ticker = interval(self.poll_interval);

        loop {
            ticker.tick().await;
            self.poll_once(&task_sender).await;
        }
    }

    /// Poll all inboxes once and submit any new messages as tasks.
    pub async fn poll_once(&mut self, task_sender: &tokio::sync::mpsc::Sender<String>) {
        match self.messagebox.poll_inboxes().await {
            Ok(messages) => {
                if messages.is_empty() {
                    tracing::debug!("Heartbeat: no new messages");
                } else {
                    tracing::info!("Heartbeat: {} new message(s)", messages.len());

                    for msg in &messages {
                        let sender_prefix = &msg.sender[..msg.sender.len().min(16)];
                        let body_preview = serde_json::to_string(&msg.body)
                            .unwrap_or_default()
                            .chars()
                            .take(200)
                            .collect::<String>();

                        let task_description = format!(
                            "Process inbox message from {}: {}",
                            sender_prefix, body_preview,
                        );

                        if let Err(e) = task_sender.send(task_description).await {
                            tracing::error!("Heartbeat: failed to submit task: {e}");
                        }
                    }

                    // Acknowledge processed messages
                    let ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
                    if let Err(e) = self.messagebox.acknowledge_message(&ids).await {
                        tracing::warn!("Heartbeat: failed to acknowledge messages: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::debug!("Heartbeat poll failed (non-fatal): {e}");
            }
        }

        // Always scan for ready continuations, regardless of messagebox results
        self.scan_continuations(task_sender).await;
    }

    /// Scan workspace/continuations/ for ready continuation files and submit them as tasks.
    async fn scan_continuations(&self, task_sender: &tokio::sync::mpsc::Sender<String>) {
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

            if let Err(e) = task_sender.send(task_desc).await {
                tracing::error!("Failed to submit continuation {cont_id}: {e}");
                continue;
            }

            tracing::info!("Submitted continuation {cont_id} for resumption");

            // Move to completed
            let done_dir = cont_dir.join("completed");
            let _ = std::fs::create_dir_all(&done_dir);
            let _ = std::fs::rename(&path, done_dir.join(entry.file_name()));
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HeartbeatConfig;

    #[test]
    fn test_heartbeat_config_defaults() {
        let hb = HeartbeatConfig::default();
        assert!(hb.enabled);
        assert_eq!(hb.inbox_poll_secs, 60);
        assert_eq!(hb.max_concurrent_tasks, 3);
    }

    #[test]
    fn test_heartbeat_daemon_creation() {
        let config = DmConfig::default();
        let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
        assert!(daemon.is_enabled());
        assert_eq!(daemon.poll_interval(), Duration::from_secs(60));
    }

    #[test]
    fn test_poll_interval_matches_config() {
        let mut config = DmConfig::default();
        config.heartbeat.inbox_poll_secs = 30;
        let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
        assert_eq!(daemon.poll_interval(), Duration::from_secs(30));
    }

    #[test]
    fn test_heartbeat_disabled() {
        let mut config = DmConfig::default();
        config.heartbeat.enabled = false;
        let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
        assert!(!daemon.is_enabled());
    }

    // -----------------------------------------------------------------------
    // Priority inbox tests
    // -----------------------------------------------------------------------

    fn make_item(msg: &str, priority: TaskPriority, offset_millis: i64) -> InboxItem {
        InboxItem {
            message: msg.to_string(),
            priority,
            session_id: None,
            model_override: None,
            submitted_at: chrono::Utc::now() + chrono::Duration::milliseconds(offset_millis),
            reply_to: None,
            max_iterations: None,
            origin: "test".to_string(),
            delegation_envelope: None,
        }
    }

    #[test]
    fn test_priority_inbox_empty() {
        let inbox = PriorityInbox::new();
        assert!(inbox.is_empty());
        assert_eq!(inbox.len(), 0);
    }

    #[test]
    fn test_priority_inbox_push_pop_single() {
        let mut inbox = PriorityInbox::new();
        inbox.push(make_item("task-a", TaskPriority::Normal, 0));
        assert_eq!(inbox.len(), 1);
        let item = inbox.pop().unwrap();
        assert_eq!(item.message, "task-a");
        assert!(inbox.is_empty());
    }

    #[test]
    fn test_priority_inbox_higher_priority_first() {
        let mut inbox = PriorityInbox::new();
        inbox.push(make_item("low", TaskPriority::Low, 0));
        inbox.push(make_item("highest", TaskPriority::Highest, 10));
        inbox.push(make_item("normal", TaskPriority::Normal, 5));

        assert_eq!(inbox.pop().unwrap().message, "highest");
        assert_eq!(inbox.pop().unwrap().message, "normal");
        assert_eq!(inbox.pop().unwrap().message, "low");
        assert!(inbox.is_empty());
    }

    #[test]
    fn test_priority_inbox_fifo_within_same_priority() {
        let mut inbox = PriorityInbox::new();
        inbox.push(make_item("first", TaskPriority::Normal, 0));
        inbox.push(make_item("second", TaskPriority::Normal, 10));
        inbox.push(make_item("third", TaskPriority::Normal, 20));

        assert_eq!(inbox.pop().unwrap().message, "first");
        assert_eq!(inbox.pop().unwrap().message, "second");
        assert_eq!(inbox.pop().unwrap().message, "third");
    }

    #[test]
    fn test_priority_inbox_mixed_priorities_and_times() {
        let mut inbox = PriorityInbox::new();
        // Two normal, one low, one highest — interleaved submission times
        inbox.push(make_item("normal-1", TaskPriority::Normal, 0));
        inbox.push(make_item("low-1", TaskPriority::Low, 5));
        inbox.push(make_item("highest-1", TaskPriority::Highest, 10));
        inbox.push(make_item("normal-2", TaskPriority::Normal, 15));

        assert_eq!(inbox.pop().unwrap().message, "highest-1");
        assert_eq!(inbox.pop().unwrap().message, "normal-1");
        assert_eq!(inbox.pop().unwrap().message, "normal-2");
        assert_eq!(inbox.pop().unwrap().message, "low-1");
        assert!(inbox.is_empty());
    }

    #[test]
    fn test_priority_inbox_pop_empty_returns_none() {
        let mut inbox = PriorityInbox::new();
        assert!(inbox.pop().is_none());
    }

    #[test]
    fn test_priority_inbox_all_four_levels() {
        let mut inbox = PriorityInbox::new();
        inbox.push(make_item("lowest", TaskPriority::Lowest, 0));
        inbox.push(make_item("low", TaskPriority::Low, 0));
        inbox.push(make_item("normal", TaskPriority::Normal, 0));
        inbox.push(make_item("highest", TaskPriority::Highest, 0));

        assert_eq!(inbox.pop().unwrap().message, "highest");
        assert_eq!(inbox.pop().unwrap().message, "normal");
        assert_eq!(inbox.pop().unwrap().message, "low");
        assert_eq!(inbox.pop().unwrap().message, "lowest");
    }

    // -----------------------------------------------------------------------
    // TaskPriority ordering tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_task_priority_ordering() {
        assert!(TaskPriority::Highest > TaskPriority::Normal);
        assert!(TaskPriority::Normal > TaskPriority::Low);
        assert!(TaskPriority::Low > TaskPriority::Lowest);
    }

    #[test]
    fn test_task_priority_equality() {
        assert_eq!(TaskPriority::Normal, TaskPriority::Normal);
        assert_ne!(TaskPriority::Highest, TaskPriority::Lowest);
    }

    // -----------------------------------------------------------------------
    // Config tests for max_concurrent_tasks
    // -----------------------------------------------------------------------

    #[test]
    fn test_heartbeat_config_max_concurrent_default() {
        let hb = HeartbeatConfig::default();
        assert_eq!(hb.max_concurrent_tasks, 3);
    }

    #[test]
    fn test_heartbeat_config_max_concurrent_from_toml() {
        let toml_str = r#"
[heartbeat]
max_concurrent_tasks = 5
"#;
        let cfg: DmConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.heartbeat.max_concurrent_tasks, 5);
    }

    // -----------------------------------------------------------------------
    // Wake coalescing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_wake_reason_equality() {
        assert_eq!(WakeReason::MessageReceived, WakeReason::MessageReceived);
        assert_ne!(WakeReason::MessageReceived, WakeReason::TimerExpired);
    }

    #[test]
    fn test_wake_reason_all_six_variants() {
        let reasons = [
            WakeReason::MessageReceived,
            WakeReason::ContinuationReady,
            WakeReason::ScheduleDue,
            WakeReason::HeartbeatFile,
            WakeReason::ExternalTrigger,
            WakeReason::TimerExpired,
        ];
        // Verify all six are distinct
        let unique: std::collections::HashSet<_> = reasons.iter().collect();
        assert_eq!(unique.len(), 6);
    }

    #[test]
    fn test_wake_reason_hash_dedup() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();

        // Insert two of the same reason
        assert!(seen.insert(WakeReason::MessageReceived));
        assert!(!seen.insert(WakeReason::MessageReceived)); // duplicate rejected

        // Different reason passes
        assert!(seen.insert(WakeReason::TimerExpired));
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn test_wake_request_priority_sorting() {
        let mut requests = [
            WakeRequest {
                reason: WakeReason::TimerExpired,
                priority: 2,
                timestamp: std::time::Instant::now(),
            },
            WakeRequest {
                reason: WakeReason::ExternalTrigger,
                priority: 0,
                timestamp: std::time::Instant::now(),
            },
            WakeRequest {
                reason: WakeReason::MessageReceived,
                priority: 3,
                timestamp: std::time::Instant::now(),
            },
        ];

        // Sort by priority (lower = higher urgency)
        requests.sort_by_key(|r| r.priority);

        assert_eq!(requests[0].priority, 0);
        assert_eq!(requests[1].priority, 2);
        assert_eq!(requests[2].priority, 3);
    }

    #[test]
    fn test_wake_batch_dedup() {
        use std::collections::HashSet;

        let mut pending = vec![
            WakeRequest {
                reason: WakeReason::MessageReceived,
                priority: 3,
                timestamp: std::time::Instant::now(),
            },
            WakeRequest {
                reason: WakeReason::MessageReceived, // duplicate
                priority: 3,
                timestamp: std::time::Instant::now(),
            },
            WakeRequest {
                reason: WakeReason::ScheduleDue,
                priority: 2,
                timestamp: std::time::Instant::now(),
            },
        ];

        // Deduplicate by reason — same logic as in run()
        let mut seen = HashSet::new();
        pending.retain(|r| seen.insert(r.reason.clone()));

        assert_eq!(pending.len(), 2);
        // First MessageReceived kept, second dropped
        assert_eq!(pending[0].reason, WakeReason::MessageReceived);
        assert_eq!(pending[1].reason, WakeReason::ScheduleDue);
    }

    // -----------------------------------------------------------------------
    // System event tests (Task 4.2)
    // -----------------------------------------------------------------------

    #[test]
    fn test_system_event_clone() {
        let evt = SystemEvent::TaskCompleted {
            task_id: "t".to_string(),
            status: "ok".to_string(),
            sats_spent: 100,
        };
        let evt2 = evt.clone();
        // Both produce the same wake request
        let w1 = evt.to_wake_request();
        let w2 = evt2.to_wake_request();
        assert_eq!(w1.reason, w2.reason);
        assert_eq!(w1.priority, w2.priority);
    }

    #[test]
    fn test_system_event_debug_format() {
        let evt = SystemEvent::ConfigChanged;
        let debug = format!("{:?}", evt);
        assert!(debug.contains("ConfigChanged"));
    }

    #[test]
    fn test_system_event_task_completed_carries_data() {
        let evt = SystemEvent::TaskCompleted {
            task_id: "task-42".to_string(),
            status: "failed".to_string(),
            sats_spent: 9999,
        };
        if let SystemEvent::TaskCompleted {
            task_id,
            status,
            sats_spent,
        } = evt
        {
            assert_eq!(task_id, "task-42");
            assert_eq!(status, "failed");
            assert_eq!(sats_spent, 9999);
        } else {
            panic!("Expected TaskCompleted");
        }
    }
}
