//! ForkExecutor — reconstruct conversation state from a transcript up to a given
//! event index, then produce the parameters needed to start a new agent run
//! from that point.
//!
//! Uses the existing `Transcript::to_messages()` logic but truncates at the
//! specified event index. The resulting message list can be injected as
//! `prior_messages` into a new `DmLoop`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use crate::transcript::Transcript;

/// Parameters for a fork operation.
#[derive(Debug, Clone, Deserialize)]
pub struct ForkParams {
    /// Zero-based event index to fork from. The new run will include
    /// all conversation context up to (but not including) this event.
    pub event_index: usize,
    /// Optional new task description. If not provided, the original
    /// task description is reused with a "Fork" prefix.
    #[serde(default)]
    pub message: Option<String>,
    /// Optional model override for the forked run.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional max iterations override.
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

/// Result of preparing a fork — the reconstructed state.
#[derive(Debug, Clone, Serialize)]
pub struct ForkResult {
    /// The original task description extracted from the transcript.
    pub original_task: String,
    /// The message list reconstructed up to the fork point (OpenAI format).
    pub prior_messages: Vec<Value>,
    /// Number of events consumed to build the prior messages.
    pub events_consumed: usize,
    /// Iteration number at the fork point.
    pub fork_iteration: u32,
    /// Sats spent up to the fork point in the original run.
    pub original_sats_at_fork: u64,
}

/// Prepares a fork from a transcript at a given event index.
pub struct ForkExecutor;

impl ForkExecutor {
    /// Reconstruct conversation state from a transcript up to `event_index`.
    ///
    /// Returns the prior messages in OpenAI format, suitable for injection
    /// into a new `DmLoop` via `set_prior_messages()`.
    pub fn prepare_fork(transcript_path: &Path, params: &ForkParams) -> Result<ForkResult, String> {
        if !transcript_path.exists() {
            return Err(format!(
                "Transcript not found: {}",
                transcript_path.display()
            ));
        }

        let transcript = Transcript::new(transcript_path.to_path_buf());
        let all_events = transcript.replay();

        if params.event_index > all_events.len() {
            return Err(format!(
                "Event index {} exceeds transcript length {}",
                params.event_index,
                all_events.len()
            ));
        }

        // Extract original task description from the first user event
        let original_task = all_events
            .iter()
            .find(|e| e.event_type == "user")
            .and_then(|e| e.data.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown task")
            .to_string();

        // Build messages from events up to the fork point
        let prior_messages = build_messages_up_to(all_events, params.event_index);

        // Calculate iteration and cost at fork point
        let mut iteration: u32 = 0;
        let mut cumulative_sats: u64 = 0;
        for (i, event) in all_events.iter().enumerate() {
            if i >= params.event_index {
                break;
            }
            if event.event_type == "think_request" {
                iteration += 1;
            }
            cumulative_sats += match event.event_type.as_str() {
                "think_response" => event
                    .data
                    .get("sats_effective")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                "tool_result" => event
                    .data
                    .get("sats_paid")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                _ => 0,
            };
        }

        Ok(ForkResult {
            original_task,
            prior_messages,
            events_consumed: params.event_index,
            fork_iteration: iteration,
            original_sats_at_fork: cumulative_sats,
        })
    }
}

/// Build OpenAI-format messages from transcript events up to (not including) `up_to_index`.
///
/// This is similar to `Transcript::to_messages()` but stops at a specific event index.
fn build_messages_up_to(
    events: &[crate::transcript::TranscriptEvent],
    up_to_index: usize,
) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    let mut pending_tool_calls: HashMap<String, Value> = HashMap::new();

    for (i, event) in events.iter().enumerate() {
        if i >= up_to_index {
            break;
        }

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
