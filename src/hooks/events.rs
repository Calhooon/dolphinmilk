//! Hook event definitions — 20+ events across 6 categories.
//!
//! Each event represents a lifecycle moment where hooks can observe,
//! modify, or block agent behavior.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A lifecycle event that can trigger registered hooks.
///
/// Events are grouped into 6 categories: Tool, Permission, Session,
/// Context, Task, and Config events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookEvent {
    // -------------------------------------------------------------------------
    // Tool events
    // -------------------------------------------------------------------------
    /// Fired before a tool is executed. Blocking hooks can prevent execution.
    PreToolExecution {
        tool_name: String,
        parameters: Value,
    },
    /// Fired after a tool completes successfully.
    PostToolExecution {
        tool_name: String,
        result: String,
        duration_ms: u64,
    },
    /// Fired when a tool execution fails.
    ToolError { tool_name: String, error: String },

    // -------------------------------------------------------------------------
    // Permission events
    // -------------------------------------------------------------------------
    /// Fired when the agent requests a permission.
    PermissionRequest { action: String, resource: String },
    /// Fired when a permission is denied.
    PermissionDenied {
        action: String,
        resource: String,
        reason: String,
    },
    /// Fired when spending crosses a budget threshold.
    BudgetThreshold {
        current_sats: u64,
        limit_sats: u64,
        percentage: f64,
    },

    // -------------------------------------------------------------------------
    // Session events
    // -------------------------------------------------------------------------
    /// Fired when a new session begins.
    SessionStart { session_id: String },
    /// Fired when a session ends.
    SessionEnd { session_id: String, summary: String },
    /// Fired on each conversation turn.
    ConversationTurn {
        conversation_id: String,
        role: String,
        content_preview: String,
    },

    // -------------------------------------------------------------------------
    // Context events
    // -------------------------------------------------------------------------
    /// Fired when context is compacted to fit the window.
    ContextCompaction {
        before_tokens: usize,
        after_tokens: usize,
    },
    /// Fired before context compaction begins. Blocking hooks can prevent compaction.
    PreCompact {
        message_count: usize,
        token_estimate: usize,
    },
    /// Fired after context compaction completes (notification only).
    PostCompact {
        original_message_count: usize,
        compacted_message_count: usize,
        summary: String,
    },
    /// Fired when context exceeds the configured limit.
    ContextOverflow { token_count: usize, limit: usize },
    /// Fired when the system prompt is rebuilt with changed sections.
    SystemPromptUpdate { sections_changed: Vec<String> },

    // -------------------------------------------------------------------------
    // Task events
    // -------------------------------------------------------------------------
    /// Fired when a new task is created.
    TaskCreated {
        task_id: String,
        description: String,
    },
    /// Fired when a task begins executing.
    TaskStarted { task_id: String },
    /// Fired when a task completes successfully.
    TaskCompleted {
        task_id: String,
        result_summary: String,
    },
    /// Fired when a task fails.
    TaskFailed { task_id: String, error: String },
    /// Fired at the start of each iteration.
    IterationStart { task_id: String, iteration: u32 },
    /// Fired at the end of each iteration.
    IterationEnd {
        task_id: String,
        iteration: u32,
        action_taken: String,
    },

    // -------------------------------------------------------------------------
    // Config events
    // -------------------------------------------------------------------------
    /// Fired when configuration is reloaded.
    ConfigReloaded { changed_keys: Vec<String> },
    /// Fired when a configuration error is detected.
    ConfigError { key: String, error: String },

    // -------------------------------------------------------------------------
    // BSV-specific events
    // -------------------------------------------------------------------------
    /// Fired when an on-chain proof is created.
    OnProofCreated {
        proof_type: String,
        txid: String,
        sats_cost: u64,
    },
    /// Fired when a cross-agent message is received.
    OnMessageReceived {
        sender_key: String,
        message_box: String,
    },
    /// Fired when an x402 payment is made.
    OnPaymentMade {
        provider: String,
        sats_paid: u64,
        model: String,
    },
    /// Fired when a certificate action occurs (acquire, revoke, relinquish).
    OnCertificateAction {
        action: String,
        certificate_type: String,
    },
    /// Fired when auto-escalation is triggered.
    OnEscalation { reason: String, iteration: u32 },
    /// Fired when a sub-agent is spawned.
    SubagentStart {
        agent_id: String,
        task_description: String,
    },
    /// Fired when a sub-agent completes.
    SubagentStop {
        agent_id: String,
        result: String,
        sats_spent: u64,
    },
}

/// Discriminant enum matching [`HookEvent`] variants without payloads.
///
/// Used for hook registration — hooks register for an event *type*,
/// then receive the full event with payload when it fires.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventType {
    // Tool
    PreToolExecution,
    PostToolExecution,
    ToolError,
    // Permission
    PermissionRequest,
    PermissionDenied,
    BudgetThreshold,
    // Session
    SessionStart,
    SessionEnd,
    ConversationTurn,
    // Context
    ContextCompaction,
    PreCompact,
    PostCompact,
    ContextOverflow,
    SystemPromptUpdate,
    // Task
    TaskCreated,
    TaskStarted,
    TaskCompleted,
    TaskFailed,
    IterationStart,
    IterationEnd,
    // Config
    ConfigReloaded,
    ConfigError,
    // BSV-specific
    OnProofCreated,
    OnMessageReceived,
    OnPaymentMade,
    OnCertificateAction,
    OnEscalation,
    SubagentStart,
    SubagentStop,
}

impl HookEvent {
    /// Return the discriminant [`HookEventType`] for this event.
    /// Returns the string that matchers should test against.
    /// For tool events, this is the tool_name. For others, returns None.
    pub fn matcher_target(&self) -> Option<String> {
        match self {
            Self::PreToolExecution { tool_name, .. } => Some(tool_name.clone()),
            Self::PostToolExecution { tool_name, .. } => Some(tool_name.clone()),
            Self::ToolError { tool_name, .. } => Some(tool_name.clone()),
            _ => None,
        }
    }

    pub fn event_type(&self) -> HookEventType {
        match self {
            Self::PreToolExecution { .. } => HookEventType::PreToolExecution,
            Self::PostToolExecution { .. } => HookEventType::PostToolExecution,
            Self::ToolError { .. } => HookEventType::ToolError,
            Self::PermissionRequest { .. } => HookEventType::PermissionRequest,
            Self::PermissionDenied { .. } => HookEventType::PermissionDenied,
            Self::BudgetThreshold { .. } => HookEventType::BudgetThreshold,
            Self::SessionStart { .. } => HookEventType::SessionStart,
            Self::SessionEnd { .. } => HookEventType::SessionEnd,
            Self::ConversationTurn { .. } => HookEventType::ConversationTurn,
            Self::ContextCompaction { .. } => HookEventType::ContextCompaction,
            Self::PreCompact { .. } => HookEventType::PreCompact,
            Self::PostCompact { .. } => HookEventType::PostCompact,
            Self::ContextOverflow { .. } => HookEventType::ContextOverflow,
            Self::SystemPromptUpdate { .. } => HookEventType::SystemPromptUpdate,
            Self::TaskCreated { .. } => HookEventType::TaskCreated,
            Self::TaskStarted { .. } => HookEventType::TaskStarted,
            Self::TaskCompleted { .. } => HookEventType::TaskCompleted,
            Self::TaskFailed { .. } => HookEventType::TaskFailed,
            Self::IterationStart { .. } => HookEventType::IterationStart,
            Self::IterationEnd { .. } => HookEventType::IterationEnd,
            Self::ConfigReloaded { .. } => HookEventType::ConfigReloaded,
            Self::ConfigError { .. } => HookEventType::ConfigError,
            Self::OnProofCreated { .. } => HookEventType::OnProofCreated,
            Self::OnMessageReceived { .. } => HookEventType::OnMessageReceived,
            Self::OnPaymentMade { .. } => HookEventType::OnPaymentMade,
            Self::OnCertificateAction { .. } => HookEventType::OnCertificateAction,
            Self::OnEscalation { .. } => HookEventType::OnEscalation,
            Self::SubagentStart { .. } => HookEventType::SubagentStart,
            Self::SubagentStop { .. } => HookEventType::SubagentStop,
        }
    }
}

impl std::fmt::Display for HookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::PreToolExecution => "pre_tool_execution",
            Self::PostToolExecution => "post_tool_execution",
            Self::ToolError => "tool_error",
            Self::PermissionRequest => "permission_request",
            Self::PermissionDenied => "permission_denied",
            Self::BudgetThreshold => "budget_threshold",
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
            Self::ConversationTurn => "conversation_turn",
            Self::ContextCompaction => "context_compaction",
            Self::PreCompact => "pre_compact",
            Self::PostCompact => "post_compact",
            Self::ContextOverflow => "context_overflow",
            Self::SystemPromptUpdate => "system_prompt_update",
            Self::TaskCreated => "task_created",
            Self::TaskStarted => "task_started",
            Self::TaskCompleted => "task_completed",
            Self::TaskFailed => "task_failed",
            Self::IterationStart => "iteration_start",
            Self::IterationEnd => "iteration_end",
            Self::ConfigReloaded => "config_reloaded",
            Self::ConfigError => "config_error",
            Self::OnProofCreated => "on_proof_created",
            Self::OnMessageReceived => "on_message_received",
            Self::OnPaymentMade => "on_payment_made",
            Self::OnCertificateAction => "on_certificate_action",
            Self::OnEscalation => "on_escalation",
            Self::SubagentStart => "subagent_start",
            Self::SubagentStop => "subagent_stop",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for HookEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pre_tool_execution" => Ok(Self::PreToolExecution),
            "post_tool_execution" => Ok(Self::PostToolExecution),
            "tool_error" => Ok(Self::ToolError),
            "permission_request" => Ok(Self::PermissionRequest),
            "permission_denied" => Ok(Self::PermissionDenied),
            "budget_threshold" => Ok(Self::BudgetThreshold),
            "session_start" => Ok(Self::SessionStart),
            "session_end" => Ok(Self::SessionEnd),
            "conversation_turn" => Ok(Self::ConversationTurn),
            "context_compaction" => Ok(Self::ContextCompaction),
            "pre_compact" => Ok(Self::PreCompact),
            "post_compact" => Ok(Self::PostCompact),
            "context_overflow" => Ok(Self::ContextOverflow),
            "system_prompt_update" => Ok(Self::SystemPromptUpdate),
            "task_created" => Ok(Self::TaskCreated),
            "task_started" => Ok(Self::TaskStarted),
            "task_completed" => Ok(Self::TaskCompleted),
            "task_failed" => Ok(Self::TaskFailed),
            "iteration_start" => Ok(Self::IterationStart),
            "iteration_end" => Ok(Self::IterationEnd),
            "config_reloaded" => Ok(Self::ConfigReloaded),
            "config_error" => Ok(Self::ConfigError),
            "on_proof_created" => Ok(Self::OnProofCreated),
            "on_message_received" => Ok(Self::OnMessageReceived),
            "on_payment_made" => Ok(Self::OnPaymentMade),
            "on_certificate_action" => Ok(Self::OnCertificateAction),
            "on_escalation" => Ok(Self::OnEscalation),
            "subagent_start" => Ok(Self::SubagentStart),
            "subagent_stop" => Ok(Self::SubagentStop),
            _ => Err(format!("unknown hook event type: {s}")),
        }
    }
}
