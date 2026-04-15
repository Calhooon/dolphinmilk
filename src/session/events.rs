//! Step events for SSE streaming to the web UI.
//!
//! Each event corresponds to a key moment in the agent loop:
//! thinking started/complete, tool call started/complete, response,
//! done, error, budget update. Events are broadcast via a tokio
//! broadcast channel and streamed to SSE clients filtered by task ID.

use serde::Serialize;

/// Maximum characters for tool output in SSE events.
/// Full output stays in the transcript; SSE gets a preview.
const SSE_OUTPUT_LIMIT: usize = 2_000;

/// Events emitted during agent loop execution, streamed to web UI via SSE.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepEvent {
    /// LLM inference started (waiting for x402 response)
    ThinkingStarted { iteration: u32, model: String },
    /// LLM response received
    ThinkingComplete {
        iteration: u32,
        text: String,
        sats_paid: u64,
        tokens: u64,
        has_tool_calls: bool,
    },
    /// Tool execution starting
    ToolCallStarted {
        iteration: u32,
        call_id: String,
        name: String,
        arguments: String,
    },
    /// Tool execution finished
    ToolCallComplete {
        iteration: u32,
        call_id: String,
        name: String,
        output: String,
        success: bool,
    },
    /// Final response (no more tool calls this iteration)
    Response { iteration: u32, text: String },
    /// Agent loop finished
    Done {
        iterations: u32,
        sats_spent: u64,
        result: String,
    },
    /// Error occurred
    Error { iteration: u32, message: String },
    /// Budget snapshot after each iteration
    BudgetUpdate {
        balance: u64,
        spent: u64,
        remaining: u64,
    },
    /// Budget limit exceeded in advisory mode (warn but don't block)
    BudgetAdvisory {
        iteration: u32,
        message: String,
        limit_type: String,
    },
    /// Multi-turn conversation session started (first SSE event for a conversation)
    SessionStarted {
        session_id: String,
        task_id: String,
        resumed: bool,
    },
    /// Tool call requires manual approval before execution.
    /// Emitted when the tool is in the approval list (config or cert-driven).
    /// Use `POST /staged/{staged_ref}/approve` or `POST /staged/{staged_ref}/abort` to proceed.
    ApprovalRequired {
        iteration: u32,
        call_id: String,
        name: String,
        arguments: String,
        staged_ref: String,
        timeout_secs: u64,
    },
    /// Agent behavior suggests it is stuck and needs human intervention.
    /// Task is paused until resolved via `POST /task/{id}/escalation/resolve`.
    Escalation {
        iteration: u32,
        reason: String,
        message: String,
    },
    /// Working memory key set or updated.
    WorkingMemorySet {
        iteration: u32,
        key: String,
        value_preview: String,
    },
    /// Working memory key cleared.
    WorkingMemoryClear { iteration: u32, key: String },
}

/// Truncate text for SSE streaming. Full output stays in transcript.
pub fn truncate_for_sse(output: &str) -> String {
    truncate_for_sse_with_limit(output, SSE_OUTPUT_LIMIT)
}

fn truncate_for_sse_with_limit(output: &str, limit: usize) -> String {
    if output.len() <= limit {
        return output.to_string();
    }
    // Find a char boundary near the limit
    let end = output
        .char_indices()
        .take_while(|(i, _)| *i < limit)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(limit);
    format!(
        "{}\n\n...truncated ({} chars, showing first {})",
        &output[..end],
        output.len(),
        end
    )
}

/// Format a StepEvent as an SSE data line.
pub fn format_sse_event(event: &StepEvent) -> Option<String> {
    serde_json::to_string(event).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinking_started_serde() {
        let event = StepEvent::ThinkingStarted {
            iteration: 1,
            model: "gpt-5-mini".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"thinking_started\""));
        assert!(json.contains("\"iteration\":1"));
        assert!(json.contains("\"model\":\"gpt-5-mini\""));
    }

    #[test]
    fn test_thinking_complete_serde() {
        let event = StepEvent::ThinkingComplete {
            iteration: 2,
            text: "Hello world".into(),
            sats_paid: 500,
            tokens: 100,
            has_tool_calls: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"thinking_complete\""));
        assert!(json.contains("\"has_tool_calls\":true"));
    }

    #[test]
    fn test_tool_call_started_serde() {
        let event = StepEvent::ToolCallStarted {
            iteration: 1,
            call_id: "call-1".into(),
            name: "execute_bash".into(),
            arguments: r#"{"command":"ls"}"#.into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool_call_started\""));
        assert!(json.contains("\"name\":\"execute_bash\""));
    }

    #[test]
    fn test_tool_call_complete_serde() {
        let event = StepEvent::ToolCallComplete {
            iteration: 1,
            call_id: "call-1".into(),
            name: "execute_bash".into(),
            output: "file1.txt\nfile2.txt".into(),
            success: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool_call_complete\""));
        assert!(json.contains("\"success\":true"));
    }

    #[test]
    fn test_response_serde() {
        let event = StepEvent::Response {
            iteration: 3,
            text: "The answer is 42.".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"response\""));
        assert!(json.contains("\"text\":\"The answer is 42.\""));
    }

    #[test]
    fn test_done_serde() {
        let event = StepEvent::Done {
            iterations: 5,
            sats_spent: 2500,
            result: "Task complete".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"done\""));
        assert!(json.contains("\"iterations\":5"));
        assert!(json.contains("\"sats_spent\":2500"));
    }

    #[test]
    fn test_error_serde() {
        let event = StepEvent::Error {
            iteration: 2,
            message: "Budget exhausted".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("\"message\":\"Budget exhausted\""));
    }

    #[test]
    fn test_budget_update_serde() {
        let event = StepEvent::BudgetUpdate {
            balance: 50000,
            spent: 1000,
            remaining: 19000,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"budget_update\""));
        assert!(json.contains("\"balance\":50000"));
    }

    #[test]
    fn test_truncate_short_output() {
        let short = "Hello world";
        assert_eq!(truncate_for_sse(short), short);
    }

    #[test]
    fn test_truncate_long_output() {
        let long = "x".repeat(3000);
        let truncated = truncate_for_sse(&long);
        assert!(truncated.len() < 3000);
        assert!(truncated.contains("...truncated"));
        assert!(truncated.contains("3000 chars"));
    }

    #[test]
    fn test_truncate_exact_limit() {
        let exact = "x".repeat(2000);
        assert_eq!(truncate_for_sse(&exact), exact);
    }

    #[test]
    fn test_truncate_unicode_safe() {
        // Ensure we don't split a multi-byte char
        let mut s = "a".repeat(1998);
        s.push('\u{1F600}'); // 4-byte emoji
        s.push_str(&"b".repeat(100));
        let truncated = truncate_for_sse(&s);
        // Should truncate at a char boundary, not mid-emoji
        assert!(truncated.contains("...truncated"));
    }

    #[test]
    fn test_format_sse_event() {
        let event = StepEvent::Done {
            iterations: 1,
            sats_spent: 0,
            result: "ok".into(),
        };
        let json = format_sse_event(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "done");
    }
}
