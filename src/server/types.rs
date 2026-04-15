//! Request and response types for the HTTP API.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Normalize raw tags: trim whitespace, lowercase, deduplicate, filter empty strings.
pub fn normalize_tags(raw: Option<Vec<String>>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    raw.unwrap_or_default()
        .into_iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

/// Info about a submitted task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub task: String,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub iterations: u32,
    pub sats_spent: u64,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proof_txids: Vec<String>,
    /// Business-outcome tags for ROI tracking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// How this task was spawned (e.g. "chat", "task", "message", "fork", "schedule", "continuation", "reflection", "checklist").
    #[serde(default)]
    pub origin: String,
    /// Conversation this task belongs to (None for orphan tasks like heartbeat/scheduled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Queued,
    Running,
    Complete,
    Error,
    Cancelled,
    /// Server crashed or was killed while this task was running.
    /// The transcript is on disk and the task can be resumed via POST /task/{id}/resume.
    Interrupted,
}

// -- Request / Response types --

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskRequest {
    pub task: String,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Optional business-outcome tags for ROI tracking.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional model override for this task.
    #[serde(default)]
    pub model: Option<String>,
}

pub(crate) fn default_max_iterations() -> u32 {
    50
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskResponse {
    pub id: String,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageRequest {
    pub sender: String,
    pub body: serde_json::Value,
    #[serde(default)]
    pub message_box: Option<String>,
    /// Optional conversation ID to join an existing conversation.
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    pub accepted: bool,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatTriggerRequest {
    /// Optional task description. Defaults to a generic manual trigger message.
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_secs: u64,
    /// Seconds since the scheduler last ticked. `None` if scheduler hasn't ticked yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_last_tick_secs_ago: Option<u64>,
    /// Whether the wallet service is reachable. Checked with a 2s timeout.
    #[serde(default)]
    pub wallet_connected: bool,
    /// Wallet URL for UI authentication (from config).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub tasks: Vec<TaskInfo>,
    pub active_count: usize,
    pub total_sats: u64,
}

/// A file attachment sent with a chat message (base64-encoded).
///
/// Supported MIME types: image/png, image/jpeg, image/gif, image/webp.
/// Max size per attachment: 5 MB (before base64 encoding).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachment {
    /// Base64-encoded file content.
    pub data: String,
    /// MIME type (e.g., "image/png").
    pub mime_type: String,
    /// Original filename.
    pub filename: String,
}

/// Maximum size per attachment in bytes (5 MB).
pub const MAX_ATTACHMENT_SIZE: usize = 5 * 1024 * 1024;
/// Maximum number of attachments per message.
pub const MAX_ATTACHMENTS: usize = 10;
/// Allowed MIME types for attachments.
pub const ALLOWED_ATTACHMENT_MIMES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Optional conversation ID to continue a prior conversation.
    /// If None, a new conversation is created.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional model override (e.g., "claude-sonnet-4-6", "gpt-5-mini").
    /// If None, uses server's configured default.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional client-provided idempotency key.
    /// Reusing the same key returns the original task/session instead of spawning again.
    #[serde(default)]
    pub client_command_id: Option<String>,
    /// Optional business-outcome tags for ROI tracking.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional file attachments (images) sent with the message.
    /// Each attachment is base64-encoded with a MIME type.
    #[serde(default)]
    pub attachments: Option<Vec<ChatAttachment>>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryParams {
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

// -- Phase 3 API response types --

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskSummary>,
    pub total: usize,
}

// -- Phase 4: Memory API response types --

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub entries: Vec<MemoryListItem>,
    pub total: usize,
    pub categories: HashMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct MemoryListItem {
    pub id: String,
    pub category: String,
    pub tags: Vec<String>,
    pub created: String,
    pub source: String,
    pub content_preview: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryDetailResponse {
    pub id: String,
    pub category: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created: String,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    pub query: String,
    pub results: Vec<MemorySearchResultItem>,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResultItem {
    pub id: String,
    pub category: String,
    pub content: String,
    pub score: f32,
    pub tags: Vec<String>,
    pub created: String,
}

#[derive(Debug, Deserialize)]
pub struct MemoryListParams {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default = "default_memory_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_memory_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchParams {
    pub q: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    5
}

// -- Phase 5: Budget detail response types --

#[derive(Debug, Serialize)]
pub struct BudgetDetailResponse {
    pub entries: Vec<SpendingEntryDto>,
    pub by_service: HashMap<String, ServiceBreakdown>,
    pub total_sats: u64,
}

#[derive(Debug, Serialize)]
pub struct SpendingEntryDto {
    pub service: String,
    pub operation: String,
    pub sats: u64,
    pub timestamp: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ServiceBreakdown {
    pub total_sats: u64,
    pub operations: HashMap<String, OperationStats>,
}

#[derive(Debug, Serialize)]
pub struct OperationStats {
    pub count: u64,
    pub total_sats: u64,
    pub avg_sats: u64,
}

#[derive(Debug, Deserialize)]
pub struct BudgetDetailParams {
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: String,
    pub task: String,
    pub status: String,
    pub iterations: u32,
    pub sats_spent: u64,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proof_txids: Vec<String>,
    #[serde(default)]
    pub tokens: u64,
    /// Business-outcome tags for ROI tracking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Estimated human-equivalent minutes saved by this task.
    #[serde(default)]
    pub time_saved_minutes: u32,
    /// How this task was spawned (e.g. "chat", "task", "message", "fork", "schedule", "continuation", "reflection", "checklist").
    #[serde(default)]
    pub origin: String,
    /// Conversation this task belongs to (None for orphan tasks like heartbeat/scheduled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditResponse {
    pub task_id: String,
    pub events: Vec<AuditEvent>,
    pub summary: AuditSummary,
    /// Conversation this task belongs to (None for orphan tasks).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: f64,
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditSummary {
    pub total_events: usize,
    pub iterations: u32,
    pub sats_spent: u64,
    pub proof_count: usize,
    pub tool_calls: usize,
    pub duration_secs: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProofsResponse {
    pub task_id: String,
    /// BRC-18 OP_RETURN proof chain — hash-linked, immutable, auditable.
    pub proofs: Vec<ProofDetail>,
    /// BRC-48 state tokens — spendable UTXOs carrying agent state (TaskCommitment, BudgetAllocation, etc.).
    pub checkpoints: Vec<ProofDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofDetail {
    pub txid: String,
    pub proof_type: String,
    pub hash: String,
    pub timestamp: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sats_cost: Option<u64>,
    /// Basket name for on-chain lookup (e.g. "dm-state" for checkpoints, "dm-proofs" for OP_RETURN proofs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basket: Option<String>,
    /// Which iteration this proof covers (present for decision, task_completion, budget_snapshot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    /// Previous proof hash in the chain (hex-encoded). None for the first proof.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationResponse {
    pub task_id: String,
    pub messages: Vec<serde_json::Value>,
}

/// Information about an available LLM model.
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub max_input_tokens: usize,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentResponse {
    pub identity_key: String,
    pub balance: u64,
    pub version: String,
    pub uptime_secs: u64,
    pub total_tasks: usize,
    pub total_sats_spent: u64,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_status: Option<String>,
    /// Whether the agent's certificate has been revoked (best-effort check).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_revoked: Option<bool>,
    /// The server's configured default model.
    #[serde(default)]
    pub default_model: String,
    /// Available LLM models with their capabilities.
    #[serde(default)]
    pub available_models: Vec<ModelInfo>,
}
