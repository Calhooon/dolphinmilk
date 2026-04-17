//! Rich message type system for type-safe transcript traversal.
//!
//! `Message` is a typed overlay on `TranscriptEvent` — it provides pattern-matching,
//! structured field access, and state reconstruction without changing the underlying
//! JSONL serialization format. Convert via `Message::from_event()` or
//! `Transcript::to_typed_messages()`.
//!
//! # Design
//!
//! - **Non-breaking**: `TranscriptEvent` remains the serialization format.
//!   `Message` is derived from events, not stored separately.
//! - **Lossless round-trip**: Every `TranscriptEvent` maps to exactly one `Message`
//!   variant (or `Unknown` for unrecognized types). Extra `data` fields are preserved.
//! - **State reconstruction**: `SessionState::from_messages()` rebuilds key loop state
//!   (iteration count, sats spent, proof hashes, etc.) from a sequence of messages.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::transcript::TranscriptEvent;

// ─── Message enum ──────────────────────────────────────────────────────

/// A typed message parsed from a `TranscriptEvent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    /// System message (injected by runner).
    System { content: String, ts: f64 },

    /// User/task input.
    User { content: String, ts: f64 },

    /// Outgoing LLM inference request.
    ThinkRequest {
        model: String,
        max_tokens: u32,
        message_count: u32,
        ts: f64,
    },

    /// LLM inference response.
    ThinkResponse {
        content: String,
        model: String,
        sats_paid: u64,
        sats_effective: u64,
        sats_refunded: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
        finish_reason: String,
        duration_ms: u64,
        tool_calls: Vec<Value>,
        ts: f64,
    },

    /// Tool invocation.
    ToolCall {
        call_id: String,
        name: String,
        arguments: Value,
        ts: f64,
    },

    /// Tool execution result.
    ToolResult {
        call_id: String,
        name: String,
        content: String,
        success: bool,
        sats_paid: u64,
        ts: f64,
    },

    /// Budget snapshot after an iteration.
    BudgetCheck {
        balance: u64,
        spent_session: u64,
        spent_hour: u64,
        task_limit: u64,
        ts: f64,
    },

    /// Loop detection warning.
    LoopWarning { message: String, ts: f64 },

    /// Error event.
    Error {
        error: String,
        context: Option<HashMap<String, Value>>,
        ts: f64,
    },

    /// Task session start.
    SessionStart { task: String, ts: f64 },

    /// Task session end.
    SessionEnd {
        iterations: u32,
        sats_spent: u64,
        result: String,
        error: String,
        ts: f64,
    },

    /// Task paused for later resumption.
    ContinuationSave {
        continuation_id: String,
        reason: String,
        wake_at: Option<String>,
        ts: f64,
    },

    /// Task resumed from continuation.
    ContinuationResume {
        continuation_id: String,
        original_task: String,
        paused_iteration: u32,
        ts: f64,
    },

    /// On-chain BRC-18 proof created.
    ProofCreated {
        txid: String,
        proof_type: String,
        hash: String,
        prev_hash: Option<String>,
        basket: Option<String>,
        iteration: Option<u32>,
        sats_cost: Option<u64>,
        ts: f64,
    },

    /// BEEF receipt stored to disk.
    ReceiptStored {
        path: String,
        txid: String,
        sats: u64,
        ts: f64,
    },

    /// BRC-48 state checkpoint created.
    CheckpointCreated {
        txid: String,
        token_type: String,
        basket: String,
        checkpoint_data: Option<Value>,
        ts: f64,
    },

    /// Memory entry created.
    MemoryStored {
        memory_id: String,
        category: String,
        tags: Vec<String>,
        content_preview: String,
        ts: f64,
    },

    /// Skill activated.
    SkillActivated {
        skill_name: String,
        context: String,
        ts: f64,
    },

    /// Periodic state checkpoint for crash recovery.
    StateCheckpoint {
        iteration: u32,
        sats_spent: u64,
        last_proof_hash: Option<String>,
        tools_used: Vec<String>,
        model: String,
        ts: f64,
    },

    /// Unrecognized event type — preserves all data for forward compatibility.
    Unknown {
        event_type: String,
        data: HashMap<String, Value>,
        ts: f64,
    },
}

// ─── Conversion from TranscriptEvent ───────────────────────────────────

/// Helper to extract a string field from event data.
fn str_field(data: &HashMap<String, Value>, key: &str) -> String {
    data.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Helper to extract a u64 field from event data.
fn u64_field(data: &HashMap<String, Value>, key: &str) -> u64 {
    data.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

/// Helper to extract a u32 field from event data.
fn u32_field(data: &HashMap<String, Value>, key: &str) -> u32 {
    data.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u32
}

impl Message {
    /// Convert a `TranscriptEvent` into a typed `Message`.
    ///
    /// Every event type maps to exactly one variant. Unrecognized types
    /// become `Message::Unknown` with all data preserved.
    pub fn from_event(event: &TranscriptEvent) -> Self {
        let d = &event.data;
        let ts = event.ts;

        match event.event_type.as_str() {
            "system" => Message::System {
                content: str_field(d, "content"),
                ts,
            },
            "user" => Message::User {
                content: str_field(d, "content"),
                ts,
            },
            "think_request" => Message::ThinkRequest {
                model: str_field(d, "model"),
                max_tokens: u32_field(d, "max_tokens"),
                message_count: u32_field(d, "message_count"),
                ts,
            },
            "think_response" => Message::ThinkResponse {
                content: str_field(d, "content"),
                model: str_field(d, "model"),
                sats_paid: u64_field(d, "sats_paid"),
                sats_effective: u64_field(d, "sats_effective"),
                sats_refunded: u64_field(d, "sats_refunded"),
                prompt_tokens: u64_field(d, "prompt_tokens"),
                completion_tokens: u64_field(d, "completion_tokens"),
                finish_reason: str_field(d, "finish_reason"),
                duration_ms: u64_field(d, "duration_ms"),
                tool_calls: d
                    .get("tool_calls")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                ts,
            },
            "tool_call" => Message::ToolCall {
                call_id: str_field(d, "call_id"),
                name: str_field(d, "name"),
                arguments: d.get("arguments").cloned().unwrap_or(Value::Null),
                ts,
            },
            "tool_result" => Message::ToolResult {
                call_id: str_field(d, "call_id"),
                name: str_field(d, "name"),
                content: str_field(d, "content"),
                success: d.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
                sats_paid: u64_field(d, "sats_paid"),
                ts,
            },
            "budget_check" => Message::BudgetCheck {
                balance: u64_field(d, "balance"),
                spent_session: u64_field(d, "spent_session"),
                spent_hour: u64_field(d, "spent_hour"),
                task_limit: u64_field(d, "task_limit"),
                ts,
            },
            "loop_warning" => Message::LoopWarning {
                message: str_field(d, "message"),
                ts,
            },
            "error" => Message::Error {
                error: str_field(d, "error"),
                context: d
                    .get("context")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
                ts,
            },
            "session_start" => Message::SessionStart {
                task: str_field(d, "task"),
                ts,
            },
            "session_end" => Message::SessionEnd {
                iterations: u32_field(d, "iterations"),
                sats_spent: u64_field(d, "sats_spent"),
                result: str_field(d, "result"),
                error: str_field(d, "error"),
                ts,
            },
            "continuation_save" => Message::ContinuationSave {
                continuation_id: str_field(d, "continuation_id"),
                reason: str_field(d, "reason"),
                wake_at: d.get("wake_at").and_then(|v| v.as_str()).map(String::from),
                ts,
            },
            "continuation_resume" => Message::ContinuationResume {
                continuation_id: str_field(d, "continuation_id"),
                original_task: str_field(d, "original_task"),
                paused_iteration: u32_field(d, "paused_iteration"),
                ts,
            },
            "proof_created" => Message::ProofCreated {
                txid: str_field(d, "txid"),
                proof_type: str_field(d, "proof_type"),
                hash: str_field(d, "hash"),
                prev_hash: d
                    .get("prev_hash")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                basket: d.get("basket").and_then(|v| v.as_str()).map(String::from),
                iteration: d
                    .get("iteration")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32),
                sats_cost: d.get("sats_cost").and_then(|v| v.as_u64()),
                ts,
            },
            "receipt_stored" => Message::ReceiptStored {
                path: str_field(d, "path"),
                txid: str_field(d, "txid"),
                sats: u64_field(d, "sats"),
                ts,
            },
            "checkpoint_created" => Message::CheckpointCreated {
                txid: str_field(d, "txid"),
                token_type: str_field(d, "token_type"),
                basket: str_field(d, "basket"),
                checkpoint_data: d.get("checkpoint_data").cloned(),
                ts,
            },
            "memory_stored" => Message::MemoryStored {
                memory_id: str_field(d, "memory_id"),
                category: str_field(d, "category"),
                tags: d
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                content_preview: str_field(d, "content_preview"),
                ts,
            },
            "skill_activated" => Message::SkillActivated {
                skill_name: str_field(d, "skill_name"),
                context: str_field(d, "context"),
                ts,
            },
            "state_checkpoint" => Message::StateCheckpoint {
                iteration: u32_field(d, "iteration"),
                sats_spent: u64_field(d, "sats_spent"),
                last_proof_hash: d
                    .get("last_proof_hash")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                tools_used: d
                    .get("tools_used")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                model: str_field(d, "model"),
                ts,
            },
            other => Message::Unknown {
                event_type: other.to_string(),
                data: d.clone(),
                ts,
            },
        }
    }

    /// Get the timestamp of this message.
    pub fn ts(&self) -> f64 {
        match self {
            Message::System { ts, .. }
            | Message::User { ts, .. }
            | Message::ThinkRequest { ts, .. }
            | Message::ThinkResponse { ts, .. }
            | Message::ToolCall { ts, .. }
            | Message::ToolResult { ts, .. }
            | Message::BudgetCheck { ts, .. }
            | Message::LoopWarning { ts, .. }
            | Message::Error { ts, .. }
            | Message::SessionStart { ts, .. }
            | Message::SessionEnd { ts, .. }
            | Message::ContinuationSave { ts, .. }
            | Message::ContinuationResume { ts, .. }
            | Message::ProofCreated { ts, .. }
            | Message::ReceiptStored { ts, .. }
            | Message::CheckpointCreated { ts, .. }
            | Message::MemoryStored { ts, .. }
            | Message::SkillActivated { ts, .. }
            | Message::StateCheckpoint { ts, .. }
            | Message::Unknown { ts, .. } => *ts,
        }
    }

    /// Returns true if this is a terminal message (session end or error).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Message::SessionEnd { .. })
    }

    /// Returns true if this is a think response that was truncated (hit max_tokens).
    pub fn is_truncated(&self) -> bool {
        matches!(self, Message::ThinkResponse { finish_reason, .. }
            if finish_reason == "length" || finish_reason == "max_tokens")
    }

    /// Returns the finish_reason if this is a ThinkResponse.
    pub fn finish_reason(&self) -> Option<&str> {
        match self {
            Message::ThinkResponse { finish_reason, .. } => Some(finish_reason),
            _ => None,
        }
    }
}

// ─── SessionState: reconstructed loop state ────────────────────────────

/// Reconstructed session state from a sequence of typed messages.
///
/// This provides the key fields needed for session recovery without
/// requiring the full `LoopState` struct (which has runtime-only fields).
#[derive(Debug, Clone, Default)]
pub struct SessionState {
    /// Task description.
    pub task: String,
    /// Number of completed iterations (think_request count).
    pub iterations: u32,
    /// Total sats spent (sum of sats_effective from think_responses + sats_paid from tool_results).
    pub sats_spent: u64,
    /// Last proof hash in the chain.
    pub last_proof_hash: Option<String>,
    /// Last known wallet balance.
    pub last_balance: u64,
    /// Whether the session ended normally.
    pub completed: bool,
    /// Error message if the session ended with an error.
    pub error: Option<String>,
    /// Last finish_reason from LLM (for detecting truncation).
    pub last_finish_reason: Option<String>,
    /// Number of think_response events with finish_reason == "length".
    pub truncation_count: u32,
    /// Proof txids created during this session.
    pub proof_txids: Vec<String>,
    /// Checkpoint txids created during this session.
    pub checkpoint_txids: Vec<String>,
    /// Total tool calls executed.
    pub tool_call_count: u32,
    /// Names of tools used.
    pub tools_used: Vec<String>,
    /// Last model used for inference.
    pub last_model: Option<String>,
}

impl SessionState {
    /// Reconstruct session state from a sequence of typed messages.
    ///
    /// Iterates through messages in order, accumulating state. This is the
    /// foundation for session recovery (#273) — everything needed to resume
    /// a task after a crash is derived from the transcript.
    pub fn from_messages(messages: &[Message]) -> Self {
        let mut state = SessionState::default();

        for msg in messages {
            match msg {
                Message::SessionStart { task, .. } => {
                    state.task = task.clone();
                }
                Message::User { content, .. }
                    // Fall back to first user message if no session_start
                    if state.task.is_empty() => {
                        state.task = content.clone();
                    }
                Message::ThinkRequest { .. } => {
                    state.iterations += 1;
                }
                Message::ThinkResponse {
                    sats_effective,
                    finish_reason,
                    model,
                    ..
                } => {
                    state.sats_spent += sats_effective;
                    state.last_finish_reason = Some(finish_reason.clone());
                    state.last_model = Some(model.clone());
                    if finish_reason == "length" || finish_reason == "max_tokens" {
                        state.truncation_count += 1;
                    }
                }
                Message::ToolCall { name, .. } => {
                    state.tool_call_count += 1;
                    if !state.tools_used.contains(name) {
                        state.tools_used.push(name.clone());
                    }
                }
                Message::ToolResult { sats_paid, .. } => {
                    state.sats_spent += sats_paid;
                }
                Message::BudgetCheck { balance, .. } => {
                    state.last_balance = *balance;
                }
                Message::ProofCreated { txid, hash, .. } => {
                    state.last_proof_hash = Some(hash.clone());
                    state.proof_txids.push(txid.clone());
                }
                Message::CheckpointCreated { txid, .. } => {
                    state.checkpoint_txids.push(txid.clone());
                }
                Message::SessionEnd {
                    result,
                    error,
                    sats_spent,
                    iterations,
                    ..
                } => {
                    state.completed = true;
                    // Prefer session_end totals over accumulated values
                    if *sats_spent > 0 {
                        state.sats_spent = *sats_spent;
                    }
                    if *iterations > 0 {
                        state.iterations = *iterations;
                    }
                    if !error.is_empty() {
                        state.error = Some(error.clone());
                    }
                    let _ = result; // available but not stored in state
                }
                _ => {}
            }
        }

        state
    }

    /// Returns true if the session was interrupted (no session_end event).
    pub fn is_interrupted(&self) -> bool {
        !self.completed
    }

    /// Returns true if the last LLM response was truncated.
    pub fn was_truncated(&self) -> bool {
        self.last_finish_reason.as_deref() == Some("length")
            || self.last_finish_reason.as_deref() == Some("max_tokens")
    }
}

// ─── Transcript extension ──────────────────────────────────────────────

use super::transcript::Transcript;

impl Transcript {
    /// Convert all transcript events to typed messages.
    pub fn to_typed_messages(&self) -> Vec<Message> {
        self.replay().iter().map(Message::from_event).collect()
    }

    /// Reconstruct session state from this transcript.
    pub fn reconstruct_state(&self) -> SessionState {
        let messages = self.to_typed_messages();
        SessionState::from_messages(&messages)
    }
}
