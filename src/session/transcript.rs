//! Append-only JSONL session transcript.
//!
//! Records every event in the agent loop: think requests/responses,
//! tool calls/results, budget checks, system messages. Supports
//! replay for session resume and crash recovery.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Valid event types for the transcript.
pub const EVENT_TYPES: &[&str] = &[
    "system",
    "user",
    "think_request",
    "think_response",
    "tool_call",
    "tool_result",
    "budget_check",
    "loop_warning",
    "error",
    "session_start",
    "session_end",
    "continuation_save",
    "continuation_resume",
    "proof_created",
    "receipt_stored",
    "checkpoint_created",
    "memory_stored",
    "skill_activated",
    "state_checkpoint",
];

/// A single event in the transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEvent {
    pub ts: f64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub id: String,
    #[serde(flatten)]
    pub data: HashMap<String, Value>,
}

impl TranscriptEvent {
    pub fn new(event_type: &str, data: HashMap<String, Value>) -> Self {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        Self {
            ts,
            event_type: event_type.to_string(),
            id,
            data,
        }
    }
}

/// Append-only JSONL transcript for an agent session.
pub struct Transcript {
    pub path: PathBuf,
    events: Vec<TranscriptEvent>,
}

impl Transcript {
    pub fn new(path: PathBuf) -> Self {
        let mut t = Self {
            path,
            events: Vec::new(),
        };
        if t.path.exists() {
            t.load();
        }
        t
    }

    fn load(&mut self) {
        let file = match fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to open transcript {}: {e}", self.path.display());
                return;
            }
        };
        let reader = BufReader::new(file);
        let mut loaded = 0u32;
        let mut errors = 0u32;
        for (line_num, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<TranscriptEvent>(trimmed) {
                Ok(event) => {
                    self.events.push(event);
                    loaded += 1;
                }
                Err(e) => {
                    tracing::warn!("Transcript line {} corrupt: {e}", line_num + 1);
                    errors += 1;
                }
            }
        }
        tracing::info!(
            "Loaded {} events from {} ({} errors)",
            loaded,
            self.path.display(),
            errors
        );
    }

    /// Append a timestamped event to the transcript.
    pub fn record(&mut self, event_type: &str, data: HashMap<String, Value>) -> &TranscriptEvent {
        let event = TranscriptEvent::new(event_type, data);
        self.events.push(event);

        // Append to file immediately (crash-safe)
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            if let Ok(json_str) = serde_json::to_string(self.events.last().unwrap()) {
                let _ = writeln!(f, "{json_str}");
            }
        }

        self.events.last().unwrap()
    }

    /// Record a system message.
    pub fn record_system(&mut self, content: &str) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("role".into(), Value::String("system".into()));
        data.insert("content".into(), Value::String(content.into()));
        self.record("system", data)
    }

    /// Record a user/trigger message.
    pub fn record_user(&mut self, content: &str) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("role".into(), Value::String("user".into()));
        data.insert("content".into(), Value::String(content.into()));
        self.record("user", data)
    }

    /// Record an outgoing LLM request.
    pub fn record_think_request(
        &mut self,
        messages: &[Value],
        model: &str,
        max_tokens: u32,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("model".into(), Value::String(model.into()));
        data.insert("max_tokens".into(), serde_json::json!(max_tokens));
        data.insert("message_count".into(), serde_json::json!(messages.len()));
        self.record("think_request", data)
    }

    /// Record an LLM response.
    #[allow(clippy::too_many_arguments)]
    pub fn record_think_response(
        &mut self,
        text: &str,
        model: &str,
        sats_paid: u64,
        sats_effective: u64,
        sats_refunded: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
        tool_calls: Option<&[Value]>,
        finish_reason: &str,
        duration_ms: u64,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("role".into(), Value::String("assistant".into()));
        data.insert("content".into(), Value::String(text.into()));
        data.insert("model".into(), Value::String(model.into()));
        data.insert("sats_paid".into(), serde_json::json!(sats_paid));
        data.insert("sats_effective".into(), serde_json::json!(sats_effective));
        data.insert("sats_refunded".into(), serde_json::json!(sats_refunded));
        data.insert("prompt_tokens".into(), serde_json::json!(prompt_tokens));
        data.insert(
            "completion_tokens".into(),
            serde_json::json!(completion_tokens),
        );
        data.insert("finish_reason".into(), Value::String(finish_reason.into()));
        data.insert("duration_ms".into(), serde_json::json!(duration_ms));
        if let Some(tc) = tool_calls {
            data.insert("tool_calls".into(), serde_json::json!(tc));
        }
        self.record("think_response", data)
    }

    /// Record a tool invocation.
    pub fn record_tool_call(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &Value,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("call_id".into(), Value::String(call_id.into()));
        data.insert("name".into(), Value::String(name.into()));
        data.insert("arguments".into(), arguments.clone());
        self.record("tool_call", data)
    }

    /// Record the result of a tool execution.
    pub fn record_tool_result(
        &mut self,
        call_id: &str,
        name: &str,
        result: &str,
        success: bool,
        sats_paid: u64,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("role".into(), Value::String("tool".into()));
        data.insert("call_id".into(), Value::String(call_id.into()));
        data.insert("name".into(), Value::String(name.into()));
        data.insert("content".into(), Value::String(result.into()));
        data.insert("success".into(), serde_json::json!(success));
        if sats_paid > 0 {
            data.insert("sats_paid".into(), serde_json::json!(sats_paid));
        }
        self.record("tool_result", data)
    }

    /// Record a budget snapshot.
    pub fn record_budget(
        &mut self,
        balance: u64,
        spent_session: u64,
        spent_hour: u64,
        task_limit: u64,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("balance".into(), serde_json::json!(balance));
        data.insert("spent_session".into(), serde_json::json!(spent_session));
        data.insert("spent_hour".into(), serde_json::json!(spent_hour));
        data.insert("task_limit".into(), serde_json::json!(task_limit));
        self.record("budget_check", data)
    }

    /// Record an error event.
    pub fn record_error(
        &mut self,
        error: &str,
        context: Option<HashMap<String, Value>>,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("error".into(), Value::String(error.into()));
        if let Some(ctx) = context {
            data.insert("context".into(), serde_json::json!(ctx));
        }
        self.record("error", data)
    }

    /// Return all events in order.
    pub fn replay(&self) -> &[TranscriptEvent] {
        &self.events
    }

    /// Reconstruct OpenAI-format message list from transcript.
    pub fn to_messages(&self) -> Vec<Value> {
        let mut messages: Vec<Value> = Vec::new();
        let mut pending_tool_calls: HashMap<String, Value> = HashMap::new();

        for event in &self.events {
            match event.event_type.as_str() {
                "system" => {
                    let content = event
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content": content,
                    }));
                }
                "user" => {
                    let content = event
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": content,
                    }));
                }
                "think_response" => {
                    let content = event
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mut msg = serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    });
                    if let Some(tc) = event.data.get("tool_calls") {
                        msg["tool_calls"] = tc.clone();
                        if let Some(arr) = tc.as_array() {
                            for call in arr {
                                if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                                    pending_tool_calls.insert(id.to_string(), call.clone());
                                }
                            }
                        }
                    }
                    messages.push(msg);
                }
                "tool_result" => {
                    let call_id = event
                        .data
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if pending_tool_calls.remove(call_id).is_some() {
                        let content = event
                            .data
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": content,
                        }));
                    } else {
                        tracing::warn!("Orphaned tool result for call_id={call_id}, skipping");
                    }
                }
                "loop_warning" => {
                    let msg = event
                        .data
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Loop detected");
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content": msg,
                    }));
                }
                _ => {}
            }
        }

        messages
    }

    /// Return events starting from the given index.
    /// The array index IS the sequence number — no separate `seq` field needed.
    pub fn events_since(&self, index: usize) -> &[TranscriptEvent] {
        if index >= self.events.len() {
            &[]
        } else {
            &self.events[index..]
        }
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn last_event(&self) -> Option<&TranscriptEvent> {
        self.events.last()
    }

    pub fn get_events_by_type(&self, event_type: &str) -> Vec<&TranscriptEvent> {
        self.events
            .iter()
            .filter(|e| e.event_type == event_type)
            .collect()
    }

    /// Get the full content of a tool_result event by tool_call_id.
    /// Returns None if not found.
    pub fn get_tool_result_content(&self, call_id: &str) -> Option<String> {
        self.events
            .iter()
            .find(|e| {
                e.event_type == "tool_result"
                    && e.data.get("call_id").and_then(|v| v.as_str()) == Some(call_id)
            })
            .and_then(|e| {
                e.data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    }

    /// Get the text content of the last `think_response` event, if any.
    ///
    /// Used by Phase 9.2 auto-recall to extract recent assistant context
    /// for query hint generation.
    /// Read the raw JSONL transcript file content as bytes.
    ///
    /// Used for HMAC computation. Returns the file content or an empty vec
    /// if the file doesn't exist or can't be read.
    pub fn read_bytes(&self) -> Vec<u8> {
        fs::read(&self.path).unwrap_or_default()
    }

    pub fn last_think_response_text(&self) -> Option<String> {
        self.events
            .iter()
            .rev()
            .find(|e| e.event_type == "think_response")
            .and_then(|e| e.data.get("content"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Record a continuation save event.
    pub fn record_continuation_save(
        &mut self,
        continuation_id: &str,
        reason: &str,
        wake_at: Option<&str>,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert(
            "continuation_id".into(),
            Value::String(continuation_id.into()),
        );
        data.insert("reason".into(), Value::String(reason.into()));
        if let Some(wake) = wake_at {
            data.insert("wake_at".into(), Value::String(wake.into()));
        }
        self.record("continuation_save", data)
    }

    /// Record a continuation resume event.
    pub fn record_continuation_resume(
        &mut self,
        continuation_id: &str,
        original_task: &str,
        paused_iteration: u32,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert(
            "continuation_id".into(),
            Value::String(continuation_id.into()),
        );
        data.insert("original_task".into(), Value::String(original_task.into()));
        data.insert(
            "paused_iteration".into(),
            Value::Number(serde_json::Number::from(paused_iteration)),
        );
        self.record("continuation_resume", data)
    }

    /// Record a proof creation event.
    #[allow(clippy::too_many_arguments)]
    pub fn record_proof_created(
        &mut self,
        txid: &str,
        proof_type: &str,
        hash_hex: &str,
        proof_data: Option<&str>,
        proof_timestamp: Option<&str>,
        sats_cost: Option<u64>,
        iteration: Option<u32>,
        basket: Option<&str>,
        prev_hash: Option<&str>,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("txid".into(), Value::String(txid.into()));
        data.insert("proof_type".into(), Value::String(proof_type.into()));
        data.insert("hash".into(), Value::String(hash_hex.into()));
        if let Some(pd) = proof_data {
            data.insert("proof_data".into(), Value::String(pd.into()));
        }
        if let Some(pt) = proof_timestamp {
            data.insert("proof_timestamp".into(), Value::String(pt.into()));
        }
        if let Some(cost) = sats_cost {
            data.insert(
                "sats_cost".into(),
                Value::Number(serde_json::Number::from(cost)),
            );
        }
        if let Some(iter) = iteration {
            data.insert(
                "iteration".into(),
                Value::Number(serde_json::Number::from(iter)),
            );
        }
        if let Some(b) = basket {
            data.insert("basket".into(), Value::String(b.into()));
        }
        if let Some(ph) = prev_hash {
            data.insert("prev_hash".into(), Value::String(ph.into()));
        }
        self.record("proof_created", data)
    }

    /// Record a BEEF receipt storage event.
    pub fn record_receipt_stored(&mut self, path: &str, txid: &str, sats: u64) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("path".into(), Value::String(path.into()));
        data.insert("txid".into(), Value::String(txid.into()));
        data.insert("sats".into(), Value::Number(serde_json::Number::from(sats)));
        self.record("receipt_stored", data)
    }

    /// Record a checkpoint token creation event.
    pub fn record_checkpoint_created(
        &mut self,
        txid: &str,
        token_type: &str,
        basket: &str,
        checkpoint_data: Option<&serde_json::Value>,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("txid".into(), Value::String(txid.into()));
        data.insert("token_type".into(), Value::String(token_type.into()));
        data.insert("basket".into(), Value::String(basket.into()));
        if let Some(cd) = checkpoint_data {
            data.insert("checkpoint_data".into(), cd.clone());
        }
        self.record("checkpoint_created", data)
    }

    /// Record a memory storage event (when the agent stores a new memory).
    pub fn record_memory_stored(
        &mut self,
        memory_id: &str,
        category: &str,
        tags: &[String],
        content_preview: &str,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("memory_id".into(), Value::String(memory_id.into()));
        data.insert("category".into(), Value::String(category.into()));
        data.insert(
            "tags".into(),
            Value::Array(tags.iter().map(|t| Value::String(t.clone())).collect()),
        );
        data.insert(
            "content_preview".into(),
            Value::String(content_preview.into()),
        );
        self.record("memory_stored", data)
    }

    /// Record a skill activation event.
    pub fn record_skill_activated(&mut self, name: &str, context: &str) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert("skill_name".into(), Value::String(name.into()));
        data.insert("context".into(), Value::String(context.into()));
        self.record("skill_activated", data)
    }

    /// Record a state checkpoint for crash recovery.
    ///
    /// Captures a snapshot of key loop state at periodic intervals so that
    /// session recovery can reconstruct state without replaying the entire
    /// transcript. Written every N iterations by the runner.
    pub fn record_state_checkpoint(
        &mut self,
        iteration: u32,
        sats_spent: u64,
        last_proof_hash: Option<&str>,
        tools_used: &[String],
        model: &str,
    ) -> &TranscriptEvent {
        let mut data = HashMap::new();
        data.insert(
            "iteration".into(),
            Value::Number(serde_json::Number::from(iteration)),
        );
        data.insert(
            "sats_spent".into(),
            Value::Number(serde_json::Number::from(sats_spent)),
        );
        if let Some(hash) = last_proof_hash {
            data.insert("last_proof_hash".into(), Value::String(hash.into()));
        }
        data.insert(
            "tools_used".into(),
            Value::Array(
                tools_used
                    .iter()
                    .map(|t| Value::String(t.clone()))
                    .collect(),
            ),
        );
        data.insert("model".into(), Value::String(model.into()));
        data.insert(
            "checkpoint_type".into(),
            Value::String("state_snapshot".into()),
        );
        self.record("state_checkpoint", data)
    }

    /// Sum of all sats spent: LLM inference (think_response.sats_effective) + tool spending (tool_result.sats_paid).
    pub fn total_sats_spent(&self) -> u64 {
        self.events
            .iter()
            .filter_map(|e| match e.event_type.as_str() {
                "think_response" => e.data.get("sats_effective").and_then(|v| v.as_u64()),
                "tool_result" => e.data.get("sats_paid").and_then(|v| v.as_u64()),
                _ => None,
            })
            .sum()
    }

    /// Total (prompt_tokens, completion_tokens) across all think events.
    pub fn total_tokens(&self) -> (u64, u64) {
        let mut prompt = 0u64;
        let mut completion = 0u64;
        for e in &self.events {
            if e.event_type == "think_response" {
                prompt += e
                    .data
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                completion += e
                    .data
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            }
        }
        (prompt, completion)
    }
}
