//! Session summarization — extract structured summaries from transcript events.
//!
//! Produces human-readable session summaries from the raw JSONL transcript,
//! suitable for long-term memory storage and context injection.

use chrono::Utc;
use serde_json::Value;

use crate::memory::store::{MemoryCategory, MemoryEntry};

/// Stateless utility for summarizing agent sessions from transcript events.
#[derive(Debug, Clone)]
pub struct SessionSummarizer;

impl SessionSummarizer {
    /// Summarize transcript events into a structured markdown summary.
    ///
    /// Extracts: task description, tools used, key results, errors,
    /// sats spent, and iteration count.
    pub fn summarize_events(events: &[Value]) -> String {
        let mut task = String::from("(unknown task)");
        let mut tool_actions: Vec<String> = Vec::new();
        let mut findings: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut total_sats: u64 = 0;
        let mut iterations: u64 = 0;
        let mut outcome = String::new();
        let mut timestamp = Utc::now().to_rfc3339();

        for event in events {
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match event_type {
                "session_start" => {
                    if let Some(ts) = event.get("ts").and_then(|v| v.as_f64()) {
                        let secs = ts as i64;
                        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
                            timestamp = dt.to_rfc3339();
                        }
                    }
                }
                "user"
                    // First user message is typically the task
                    if task == "(unknown task)" => {
                        if let Some(content) = event.get("content").and_then(|v| v.as_str()) {
                            task = truncate_str(content, 200);
                        }
                    }
                "think_response" => {
                    iterations += 1;

                    if let Some(sats) = event.get("sats_effective").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                    }

                    // Extract key findings from assistant content
                    if let Some(content) = event.get("content").and_then(|v| v.as_str()) {
                        if !content.is_empty() {
                            // Take the first sentence as a finding if it's substantive
                            let first_sentence = extract_first_sentence(content);
                            if first_sentence.len() > 20 && findings.len() < 5 {
                                findings.push(first_sentence);
                            }
                        }

                        // Last think_response content is the outcome
                        outcome = truncate_str(content, 300);
                    }
                }
                "tool_call" => {
                    let name = event
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");

                    // Find matching result
                    let result_summary = find_tool_result(events, call_id);
                    tool_actions.push(format!("{name}: {result_summary}"));
                }
                "error" => {
                    if let Some(err) = event.get("error").and_then(|v| v.as_str()) {
                        errors.push(truncate_str(err, 150));
                    }
                }
                _ => {}
            }
        }

        // Build summary
        let mut summary = format!("## Session Summary: {task}\n");
        summary.push_str(&format!("**Date**: {timestamp}\n"));
        summary.push_str(&format!("**Iterations**: {iterations}\n"));
        summary.push_str(&format!("**Cost**: {total_sats} sats\n"));

        if !tool_actions.is_empty() {
            summary.push_str("\n### Actions Taken\n");
            for action in &tool_actions {
                summary.push_str(&format!("- {action}\n"));
            }
        }

        if !findings.is_empty() {
            summary.push_str("\n### Key Findings\n");
            for finding in &findings {
                summary.push_str(&format!("- {finding}\n"));
            }
        }

        if !errors.is_empty() {
            summary.push_str("\n### Errors\n");
            for err in &errors {
                summary.push_str(&format!("- {err}\n"));
            }
        }

        summary.push_str("\n### Outcome\n");
        if outcome.is_empty() {
            summary.push_str("No final response recorded.\n");
        } else {
            summary.push_str(&format!("{outcome}\n"));
        }

        summary
    }

    /// Create a MemoryEntry of category Session from transcript events.
    pub fn create_session_entry(events: &[Value], session_id: &str) -> MemoryEntry {
        let summary = Self::summarize_events(events);

        // Extract tags from tool names used
        let mut tags: Vec<String> = vec!["session".to_string()];
        for event in events {
            if event.get("type").and_then(|v| v.as_str()) == Some("tool_call") {
                if let Some(name) = event.get("name").and_then(|v| v.as_str()) {
                    let tag = name.to_string();
                    if !tags.contains(&tag) {
                        tags.push(tag);
                    }
                }
            }
        }

        MemoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            category: MemoryCategory::Session,
            content: summary,
            tags,
            created: Utc::now(),
            source: session_id.to_string(),
        }
    }
}

/// Find the result for a tool call by its call_id.
fn find_tool_result(events: &[Value], call_id: &str) -> String {
    if call_id.is_empty() {
        return "(no result)".to_string();
    }

    for event in events {
        let is_result = event.get("type").and_then(|v| v.as_str()) == Some("tool_result");
        let matches_id = event.get("call_id").and_then(|v| v.as_str()) == Some(call_id);

        if is_result && matches_id {
            let success = event
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let content = event.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let prefix = if success { "OK" } else { "FAILED" };
            return format!("{prefix} — {}", truncate_str(content, 100));
        }
    }

    "(no result)".to_string()
}

/// Extract the first sentence from text.
fn extract_first_sentence(text: &str) -> String {
    // Find first sentence boundary
    let trimmed = text.trim();
    for (i, c) in trimmed.char_indices() {
        if (c == '.' || c == '!' || c == '?') && i > 10 {
            return trimmed[..=i].to_string();
        }
        if i > 200 {
            let safe_end = trimmed.floor_char_boundary(200);
            return format!("{}...", &trimmed[..safe_end]);
        }
    }
    truncate_str(trimmed, 200)
}

/// Truncate a string to a maximum length, appending "..." if truncated.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a safe char boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_events() -> Vec<Value> {
        vec![
            json!({
                "type": "session_start",
                "ts": 1740000000.0
            }),
            json!({
                "type": "user",
                "content": "Investigate x402 payment flow and document the handshake"
            }),
            json!({
                "type": "think_response",
                "content": "I'll start by examining the x402 payment module to understand the handshake protocol.",
                "sats_effective": 50,
                "tool_calls": []
            }),
            json!({
                "type": "tool_call",
                "call_id": "tc-001",
                "name": "file_read",
                "arguments": {"path": "src/x402/payment.rs"}
            }),
            json!({
                "type": "tool_result",
                "call_id": "tc-001",
                "name": "file_read",
                "content": "pub fn negotiate_payment(...) -> Result<...>",
                "success": true
            }),
            json!({
                "type": "tool_call",
                "call_id": "tc-002",
                "name": "execute_bash",
                "arguments": {"command": "cargo test"}
            }),
            json!({
                "type": "tool_result",
                "call_id": "tc-002",
                "name": "execute_bash",
                "content": "103 tests passed",
                "success": true
            }),
            json!({
                "type": "think_response",
                "content": "The x402 handshake uses a 3-step process: 1) GET request, 2) 402 response with price, 3) Payment + retry. All tests pass.",
                "sats_effective": 75
            }),
            json!({
                "type": "error",
                "error": "Rate limit hit, retrying after backoff"
            }),
        ]
    }

    #[test]
    fn test_summarize_events_structure() {
        let events = sample_events();
        let summary = SessionSummarizer::summarize_events(&events);

        assert!(summary.contains("## Session Summary:"));
        assert!(summary.contains("x402 payment flow"));
        assert!(summary.contains("**Iterations**: 2"));
        assert!(summary.contains("**Cost**: 125 sats"));
        assert!(summary.contains("### Actions Taken"));
        assert!(summary.contains("file_read:"));
        assert!(summary.contains("execute_bash:"));
        assert!(summary.contains("### Outcome"));
    }

    #[test]
    fn test_summarize_events_captures_errors() {
        let events = sample_events();
        let summary = SessionSummarizer::summarize_events(&events);
        assert!(summary.contains("### Errors"));
        assert!(summary.contains("Rate limit"));
    }

    #[test]
    fn test_summarize_empty_events() {
        let summary = SessionSummarizer::summarize_events(&[]);
        assert!(summary.contains("## Session Summary: (unknown task)"));
        assert!(summary.contains("**Iterations**: 0"));
        assert!(summary.contains("**Cost**: 0 sats"));
    }

    #[test]
    fn test_create_session_entry() {
        let events = sample_events();
        let entry = SessionSummarizer::create_session_entry(&events, "session-2026-02-23-01");

        assert_eq!(entry.category, MemoryCategory::Session);
        assert_eq!(entry.source, "session-2026-02-23-01");
        assert!(entry.tags.contains(&"session".to_string()));
        assert!(entry.tags.contains(&"file_read".to_string()));
        assert!(entry.tags.contains(&"execute_bash".to_string()));
        assert!(entry.content.contains("## Session Summary"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello...");
        assert_eq!(truncate_str("", 5), "");
    }

    #[test]
    fn test_extract_first_sentence() {
        assert_eq!(
            extract_first_sentence("This is a full sentence. And another."),
            "This is a full sentence."
        );
        assert_eq!(extract_first_sentence("Short."), "Short.");
    }

    #[test]
    fn test_find_tool_result_found() {
        let events = sample_events();
        let result = find_tool_result(&events, "tc-001");
        assert!(result.starts_with("OK"));
    }

    #[test]
    fn test_find_tool_result_not_found() {
        let events = sample_events();
        let result = find_tool_result(&events, "nonexistent");
        assert_eq!(result, "(no result)");
    }

    #[test]
    fn test_find_tool_result_empty_id() {
        let events = sample_events();
        let result = find_tool_result(&events, "");
        assert_eq!(result, "(no result)");
    }
}
