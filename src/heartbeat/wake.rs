//! Wake coalescing types and system events for the scheduler.
//!
//! `WakeReason` and `WakeRequest` drive the event-driven wake loop.
//! `SystemEvent` is emitted by the agent loop and API handlers to
//! trigger immediate scheduler wake (after 250ms coalescing).

// ---------------------------------------------------------------------------
// Wake coalescing types
// ---------------------------------------------------------------------------

/// Reason for a scheduler wake. Used for coalescing and deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WakeReason {
    /// A new MessageBox message was received.
    MessageReceived,
    /// A continuation's wake_at has elapsed.
    ContinuationReady,
    /// A scheduled task is due.
    ScheduleDue,
    /// The HEARTBEAT.md checklist file has content.
    HeartbeatFile,
    /// An external trigger was submitted (e.g., POST /message, config change).
    ExternalTrigger,
    /// The tick timer expired with no specific event.
    TimerExpired,
}

/// A wake request submitted to the scheduler.
#[derive(Debug, Clone)]
pub struct WakeRequest {
    pub reason: WakeReason,
    /// Priority ordering: retry(0) < interval(1) < default(2) < action(3).
    pub priority: u8,
    pub timestamp: std::time::Instant,
}

// ---------------------------------------------------------------------------
// System events
// ---------------------------------------------------------------------------

/// System events emitted by the agent loop and API handlers.
/// Each event causes the scheduler to wake immediately (after coalescing).
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// A task completed (success, error, or cancelled).
    TaskCompleted {
        task_id: String,
        status: String,
        sats_spent: u64,
    },
    /// A recurring schedule was fired by the scheduler.
    ScheduleFired { schedule_id: String },
    /// A new MessageBox message was received.
    MessageReceived { sender: String, inbox: String },
    /// The process configuration was reloaded (hot reload — Task 4.4).
    ConfigChanged,
    /// A continuation task is now ready to resume.
    ContinuationReady { task_id: String },
}

impl SystemEvent {
    /// Convert to a WakeRequest for the coalescing system.
    pub fn to_wake_request(&self) -> WakeRequest {
        match self {
            SystemEvent::TaskCompleted { .. } | SystemEvent::ContinuationReady { .. } => {
                WakeRequest {
                    reason: WakeReason::ContinuationReady,
                    priority: 3, // action priority
                    timestamp: std::time::Instant::now(),
                }
            }
            SystemEvent::ScheduleFired { .. } => WakeRequest {
                reason: WakeReason::ScheduleDue,
                priority: 2,
                timestamp: std::time::Instant::now(),
            },
            SystemEvent::MessageReceived { .. } => WakeRequest {
                reason: WakeReason::MessageReceived,
                priority: 3,
                timestamp: std::time::Instant::now(),
            },
            SystemEvent::ConfigChanged => WakeRequest {
                reason: WakeReason::ExternalTrigger,
                priority: 1,
                timestamp: std::time::Instant::now(),
            },
        }
    }
}

/// Dedicated system conversation for heartbeat tasks.
///
/// Checklist probes and manual triggers run within this conversation so each
/// heartbeat check sees prior findings as context.  Compaction (Phase 6) keeps
/// the conversation from growing unbounded.
pub const HEARTBEAT_CONVERSATION_ID: &str = "conv-heartbeat";
