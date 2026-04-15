//! ReplayViewer — reads a session JSONL transcript and yields events with metadata.
//!
//! Supports seeking to any event index and provides enriched metadata per event:
//! timestamp, cumulative cost, tool name, iteration number, duration since start.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::transcript::{Transcript, TranscriptEvent};

/// A single replay event with enriched metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayEvent {
    /// Zero-based index in the transcript.
    pub index: usize,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: f64,
    /// Event type (e.g. "think_response", "tool_call").
    pub event_type: String,
    /// Short event ID (8-char UUID prefix).
    pub id: String,
    /// Current iteration number (incremented on each think_request).
    pub iteration: u32,
    /// Sats spent by this specific event (0 for non-spending events).
    pub event_sats: u64,
    /// Cumulative sats spent up to and including this event.
    pub cumulative_sats: u64,
    /// Tool name, if this is a tool_call or tool_result event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Model name, if this is a think_request or think_response event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Seconds elapsed since the first event in the transcript.
    pub elapsed_secs: f64,
    /// The full event data.
    pub data: serde_json::Value,
}

/// A point on the cumulative cost timeline.
#[derive(Debug, Clone, Serialize)]
pub struct CostTimelinePoint {
    /// Seconds since task start.
    pub elapsed_secs: f64,
    /// Cumulative sats spent at this point.
    pub cumulative_sats: u64,
    /// Event type that caused this cost.
    pub event_type: String,
    /// Event index.
    pub index: usize,
}

/// A tool usage entry summarizing one tool invocation.
#[derive(Debug, Clone, Serialize)]
pub struct ToolUsageEntry {
    /// Tool name.
    pub name: String,
    /// Iteration in which the tool was called.
    pub iteration: u32,
    /// Event index of the tool_call event.
    pub call_index: usize,
    /// Event index of the tool_result event (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_index: Option<usize>,
    /// Whether the tool call succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Sats paid by the tool (from tool_result).
    pub sats_paid: u64,
}

/// Complete structured replay timeline for a task.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayTimeline {
    /// Task ID (derived from transcript path).
    pub task_id: String,
    /// All events with metadata.
    pub events: Vec<ReplayEvent>,
    /// Cumulative cost timeline (only events that cost sats).
    pub cost_timeline: Vec<CostTimelinePoint>,
    /// Tool usage timeline.
    pub tool_usage: Vec<ToolUsageEntry>,
    /// Total sats spent.
    pub total_sats: u64,
    /// Total event count.
    pub total_events: usize,
    /// Total iterations (think_request count).
    pub total_iterations: u32,
    /// Total duration in seconds (last event ts - first event ts).
    pub duration_secs: f64,
}

/// Reads a transcript and provides structured replay access.
pub struct ReplayViewer {
    transcript: Transcript,
    path: PathBuf,
}

impl ReplayViewer {
    /// Create a new ReplayViewer from a transcript file path.
    pub fn new(path: &Path) -> Self {
        Self {
            transcript: Transcript::new(path.to_path_buf()),
            path: path.to_path_buf(),
        }
    }

    /// Total number of events in the transcript.
    pub fn event_count(&self) -> usize {
        self.transcript.event_count()
    }

    /// Build the complete replay timeline with enriched metadata.
    pub fn build_timeline(&self) -> ReplayTimeline {
        let raw_events = self.transcript.replay();
        let task_id = extract_task_id(&self.path);
        let start_ts = raw_events.first().map(|e| e.ts).unwrap_or(0.0);

        let mut iteration: u32 = 0;
        let mut cumulative_sats: u64 = 0;
        let mut events = Vec::with_capacity(raw_events.len());
        let mut cost_timeline = Vec::new();
        let mut tool_calls: Vec<ToolUsageEntry> = Vec::new();

        for (index, raw) in raw_events.iter().enumerate() {
            if raw.event_type == "think_request" {
                iteration += 1;
            }

            let event_sats = extract_event_sats(raw);
            cumulative_sats += event_sats;
            let elapsed_secs = raw.ts - start_ts;

            let tool_name = extract_tool_name(raw);
            let model = extract_model(raw);

            events.push(ReplayEvent {
                index,
                timestamp: raw.ts,
                event_type: raw.event_type.clone(),
                id: raw.id.clone(),
                iteration,
                event_sats,
                cumulative_sats,
                tool_name: tool_name.clone(),
                model,
                elapsed_secs,
                data: serde_json::to_value(&raw.data).unwrap_or_default(),
            });

            // Track cost timeline for spending events
            if event_sats > 0 {
                cost_timeline.push(CostTimelinePoint {
                    elapsed_secs,
                    cumulative_sats,
                    event_type: raw.event_type.clone(),
                    index,
                });
            }

            // Track tool usage
            if raw.event_type == "tool_call" {
                if let Some(name) = tool_name {
                    tool_calls.push(ToolUsageEntry {
                        name,
                        iteration,
                        call_index: index,
                        result_index: None,
                        success: None,
                        sats_paid: 0,
                    });
                }
            } else if raw.event_type == "tool_result" {
                // Match result to the most recent unresolved call with the same name
                let result_name = raw.data.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let success = raw.data.get("success").and_then(|v| v.as_bool());
                let sats = raw
                    .data
                    .get("sats_paid")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                if let Some(entry) = tool_calls
                    .iter_mut()
                    .rev()
                    .find(|t| t.result_index.is_none() && t.name == result_name)
                {
                    entry.result_index = Some(index);
                    entry.success = success;
                    entry.sats_paid = sats;
                }
            }
        }

        let duration_secs =
            if let (Some(first), Some(last)) = (raw_events.first(), raw_events.last()) {
                last.ts - first.ts
            } else {
                0.0
            };

        ReplayTimeline {
            task_id,
            events,
            cost_timeline,
            tool_usage: tool_calls,
            total_sats: cumulative_sats,
            total_events: raw_events.len(),
            total_iterations: iteration,
            duration_secs,
        }
    }

    /// Get a single event by index with enriched metadata.
    /// Returns None if the index is out of bounds.
    pub fn get_event(&self, index: usize) -> Option<ReplayEvent> {
        let raw_events = self.transcript.replay();
        if index >= raw_events.len() {
            return None;
        }

        let start_ts = raw_events.first().map(|e| e.ts).unwrap_or(0.0);
        let mut iteration: u32 = 0;
        let mut cumulative_sats: u64 = 0;

        for (i, raw) in raw_events.iter().enumerate() {
            if raw.event_type == "think_request" {
                iteration += 1;
            }
            let event_sats = extract_event_sats(raw);
            cumulative_sats += event_sats;

            if i == index {
                return Some(ReplayEvent {
                    index,
                    timestamp: raw.ts,
                    event_type: raw.event_type.clone(),
                    id: raw.id.clone(),
                    iteration,
                    event_sats,
                    cumulative_sats,
                    tool_name: extract_tool_name(raw),
                    model: extract_model(raw),
                    elapsed_secs: raw.ts - start_ts,
                    data: serde_json::to_value(&raw.data).unwrap_or_default(),
                });
            }
        }

        None
    }

    /// Get events in a range [from, to) with enriched metadata.
    pub fn get_events_range(&self, from: usize, to: usize) -> Vec<ReplayEvent> {
        let raw_events = self.transcript.replay();
        let start_ts = raw_events.first().map(|e| e.ts).unwrap_or(0.0);
        let mut iteration: u32 = 0;
        let mut cumulative_sats: u64 = 0;
        let mut result = Vec::new();

        let actual_to = to.min(raw_events.len());

        for (i, raw) in raw_events.iter().enumerate() {
            if raw.event_type == "think_request" {
                iteration += 1;
            }
            let event_sats = extract_event_sats(raw);
            cumulative_sats += event_sats;

            if i >= from && i < actual_to {
                result.push(ReplayEvent {
                    index: i,
                    timestamp: raw.ts,
                    event_type: raw.event_type.clone(),
                    id: raw.id.clone(),
                    iteration,
                    event_sats,
                    cumulative_sats,
                    tool_name: extract_tool_name(raw),
                    model: extract_model(raw),
                    elapsed_secs: raw.ts - start_ts,
                    data: serde_json::to_value(&raw.data).unwrap_or_default(),
                });
            }

            if i >= actual_to {
                break;
            }
        }

        result
    }
}

/// Extract sats spent by a single event.
fn extract_event_sats(event: &TranscriptEvent) -> u64 {
    match event.event_type.as_str() {
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
    }
}

/// Extract tool name from a tool_call or tool_result event.
fn extract_tool_name(event: &TranscriptEvent) -> Option<String> {
    match event.event_type.as_str() {
        "tool_call" | "tool_result" => event
            .data
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Extract model name from think events.
fn extract_model(event: &TranscriptEvent) -> Option<String> {
    match event.event_type.as_str() {
        "think_request" | "think_response" => event
            .data
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Extract task ID from a transcript file path.
/// Assumes path format: .../tasks/{task_id}/session.jsonl
fn extract_task_id(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}
