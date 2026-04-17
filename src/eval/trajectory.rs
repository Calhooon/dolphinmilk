//! Trajectory recording and replay from JSONL session transcripts.
//!
//! A trajectory is a structured representation of a single agent session,
//! parsed from the JSONL transcript format used by `session::transcript`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Classification of each step in a trajectory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    /// User or system message initiating a turn.
    UserMessage,
    /// LLM inference response.
    LlmResponse,
    /// Tool invocation by the agent.
    ToolCall,
    /// Result returned from a tool execution.
    ToolResult,
    /// Budget snapshot.
    BudgetCheck,
    /// Error event.
    Error,
    /// Session lifecycle event (start, end, continuation).
    SessionEvent,
    /// Any other event type.
    Other,
}

impl StepType {
    /// Map a transcript event type string to a StepType.
    pub fn from_event_type(event_type: &str) -> Self {
        match event_type {
            "user" | "system" => StepType::UserMessage,
            "think_request" => StepType::UserMessage,
            "think_response" => StepType::LlmResponse,
            "tool_call" => StepType::ToolCall,
            "tool_result" => StepType::ToolResult,
            "budget_check" => StepType::BudgetCheck,
            "error" => StepType::Error,
            "session_start" | "session_end" | "continuation_save" | "continuation_resume" => {
                StepType::SessionEvent
            }
            _ => StepType::Other,
        }
    }
}

/// A single step in an agent trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// Timestamp of the event.
    pub timestamp: DateTime<Utc>,
    /// Classification of this step.
    pub step_type: StepType,
    /// Raw event data from the transcript.
    pub content: Value,
    /// Tokens consumed by this step (for LLM responses).
    pub tokens_used: Option<u64>,
    /// Satoshis spent on this step.
    pub cost_sats: Option<u64>,
    /// Original event type string from the transcript.
    pub event_type: String,
}

/// Metadata about a trajectory session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryMetadata {
    /// Total number of steps.
    pub step_count: usize,
    /// Total tokens used (prompt + completion).
    pub total_tokens: u64,
    /// Total satoshis spent.
    pub total_cost_sats: u64,
    /// Number of LLM round-trips.
    pub llm_rounds: usize,
    /// Number of tool invocations.
    pub tool_calls: usize,
    /// Number of errors encountered.
    pub error_count: usize,
    /// Duration from first to last event in seconds.
    pub duration_secs: f64,
    /// Set of unique tool names used.
    pub tools_used: Vec<String>,
}

/// A complete agent trajectory parsed from a JSONL transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Session identifier.
    pub session_id: String,
    /// Ordered sequence of steps.
    pub steps: Vec<TrajectoryStep>,
    /// Computed metadata about this trajectory.
    pub metadata: TrajectoryMetadata,
}

impl Trajectory {
    /// Parse a trajectory from a JSONL transcript file.
    ///
    /// Each line in the file should be a JSON object with at minimum
    /// `ts`, `type`, and `id` fields matching the `TranscriptEvent` format.
    pub fn from_jsonl(path: &Path) -> Result<Self, String> {
        let file =
            fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
        let reader = BufReader::new(file);
        let mut steps = Vec::new();
        let mut session_id = String::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| format!("line {}: {e}", line_num + 1))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let event: Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("line {}: invalid JSON: {e}", line_num + 1))?;

            let event_type = event
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            // Extract session_id from session_start events
            if event_type == "session_start" {
                if let Some(sid) = event.get("session_id").and_then(|v| v.as_str()) {
                    session_id = sid.to_string();
                }
            }

            let ts = event.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let timestamp = DateTime::from_timestamp(ts as i64, ((ts.fract()) * 1e9) as u32)
                .unwrap_or_default();

            let step_type = StepType::from_event_type(&event_type);

            let tokens_used = match event_type.as_str() {
                "think_response" => {
                    let prompt = event
                        .get("prompt_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let completion = event
                        .get("completion_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let total = prompt + completion;
                    if total > 0 {
                        Some(total)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            let cost_sats = match event_type.as_str() {
                "think_response" => event.get("sats_effective").and_then(|v| v.as_u64()),
                "tool_result" => {
                    let paid = event.get("sats_paid").and_then(|v| v.as_u64()).unwrap_or(0);
                    if paid > 0 {
                        Some(paid)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            steps.push(TrajectoryStep {
                timestamp,
                step_type,
                content: event,
                tokens_used,
                cost_sats,
                event_type,
            });
        }

        if session_id.is_empty() {
            // Fall back to filename as session ID
            session_id = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
        }

        let metadata = Self::compute_metadata(&steps);

        Ok(Trajectory {
            session_id,
            steps,
            metadata,
        })
    }

    /// Parse a trajectory from JSONL content provided as a string.
    pub fn from_jsonl_str(content: &str, session_id: &str) -> Result<Self, String> {
        let mut steps = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let event: Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("line {}: invalid JSON: {e}", line_num + 1))?;

            let event_type = event
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let ts = event.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let timestamp = DateTime::from_timestamp(ts as i64, ((ts.fract()) * 1e9) as u32)
                .unwrap_or_default();

            let step_type = StepType::from_event_type(&event_type);

            let tokens_used = if event_type == "think_response" {
                let prompt = event
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let completion = event
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let total = prompt + completion;
                if total > 0 {
                    Some(total)
                } else {
                    None
                }
            } else {
                None
            };

            let cost_sats = match event_type.as_str() {
                "think_response" => event.get("sats_effective").and_then(|v| v.as_u64()),
                "tool_result" => {
                    let paid = event.get("sats_paid").and_then(|v| v.as_u64()).unwrap_or(0);
                    if paid > 0 {
                        Some(paid)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            steps.push(TrajectoryStep {
                timestamp,
                step_type,
                content: event,
                tokens_used,
                cost_sats,
                event_type,
            });
        }

        let metadata = Self::compute_metadata(&steps);

        Ok(Trajectory {
            session_id: session_id.to_string(),
            steps,
            metadata,
        })
    }

    /// Compute aggregate metadata from a list of steps.
    fn compute_metadata(steps: &[TrajectoryStep]) -> TrajectoryMetadata {
        let total_tokens: u64 = steps.iter().filter_map(|s| s.tokens_used).sum();
        let total_cost_sats: u64 = steps.iter().filter_map(|s| s.cost_sats).sum();
        let llm_rounds = steps
            .iter()
            .filter(|s| s.step_type == StepType::LlmResponse)
            .count();
        let tool_calls = steps
            .iter()
            .filter(|s| s.step_type == StepType::ToolCall)
            .count();
        let error_count = steps
            .iter()
            .filter(|s| s.step_type == StepType::Error)
            .count();

        let duration_secs = if steps.len() >= 2 {
            let first = steps.first().unwrap().timestamp;
            let last = steps.last().unwrap().timestamp;
            (last - first).num_milliseconds() as f64 / 1000.0
        } else {
            0.0
        };

        let mut tools_used: Vec<String> = steps
            .iter()
            .filter(|s| s.step_type == StepType::ToolCall)
            .filter_map(|s| s.content.get("name").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        tools_used.sort();

        TrajectoryMetadata {
            step_count: steps.len(),
            total_tokens,
            total_cost_sats,
            llm_rounds,
            tool_calls,
            error_count,
            duration_secs,
            tools_used,
        }
    }

    /// Get all user messages from the trajectory.
    pub fn user_messages(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter(|s| s.event_type == "user")
            .filter_map(|s| s.content.get("content").and_then(|v| v.as_str()))
            .collect()
    }

    /// Get all tool names invoked during this trajectory.
    pub fn tool_names(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter(|s| s.step_type == StepType::ToolCall)
            .filter_map(|s| s.content.get("name").and_then(|v| v.as_str()))
            .collect()
    }

    /// Get all LLM response texts from the trajectory.
    pub fn llm_responses(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter(|s| s.step_type == StepType::LlmResponse)
            .filter_map(|s| s.content.get("content").and_then(|v| v.as_str()))
            .collect()
    }

    /// Extract topic keywords from user messages using simple word frequency.
    ///
    /// Returns the top `limit` keywords by frequency, excluding common stop words.
    pub fn extract_topics(&self, limit: usize) -> Vec<String> {
        let stop_words: std::collections::HashSet<&str> = [
            "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has",
            "had", "do", "does", "did", "will", "would", "could", "should", "may", "might",
            "shall", "can", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as",
            "into", "through", "during", "before", "after", "about", "between", "under", "above",
            "up", "down", "out", "off", "over", "then", "than", "too", "very", "just", "also",
            "no", "not", "only", "own", "same", "so", "that", "this", "what", "which", "who",
            "how", "all", "each", "every", "both", "few", "more", "most", "other", "some", "such",
            "any", "and", "but", "or", "nor", "if", "it", "its", "i", "me", "my", "we", "you",
            "your", "he", "she", "they", "them", "their", "her", "his",
        ]
        .into_iter()
        .collect();

        let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        for msg in self.user_messages() {
            for word in msg.split_whitespace() {
                let clean = word
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                if clean.len() >= 3 && !stop_words.contains(clean.as_str()) {
                    *freq.entry(clean).or_insert(0) += 1;
                }
            }
        }

        let mut sorted: Vec<(String, usize)> = freq.into_iter().collect();
        sorted.sort_by_key(|x| std::cmp::Reverse(x.1));
        sorted.into_iter().take(limit).map(|(w, _)| w).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_jsonl() -> &'static str {
        concat!(
            r#"{"ts":1700000000.0,"type":"session_start","id":"s1","session_id":"sess-001","task":"test task"}"#,
            "\n",
            r#"{"ts":1700000001.0,"type":"user","id":"u1","content":"What is BSV?"}"#,
            "\n",
            r#"{"ts":1700000002.0,"type":"think_response","id":"t1","role":"assistant","content":"BSV is Bitcoin SV.","model":"gpt-5-mini","sats_paid":500,"sats_effective":400,"sats_refunded":0,"prompt_tokens":50,"completion_tokens":20,"finish_reason":"stop","duration_ms":1200}"#,
            "\n",
            r#"{"ts":1700000003.0,"type":"tool_call","id":"tc1","call_id":"call-1","name":"memory_search","arguments":{"query":"BSV"}}"#,
            "\n",
            r#"{"ts":1700000004.0,"type":"tool_result","id":"tr1","call_id":"call-1","name":"memory_search","content":"Found 2 results","success":true,"sats_paid":0}"#,
            "\n",
            r#"{"ts":1700000005.0,"type":"think_response","id":"t2","role":"assistant","content":"Based on my search, BSV stands for Bitcoin Satoshi Vision.","model":"gpt-5-mini","sats_paid":600,"sats_effective":500,"sats_refunded":0,"prompt_tokens":100,"completion_tokens":30,"finish_reason":"stop","duration_ms":800}"#,
            "\n",
        )
    }

    #[test]
    fn test_parse_trajectory_from_str() {
        let traj = Trajectory::from_jsonl_str(sample_jsonl(), "test-session").unwrap();
        assert_eq!(traj.session_id, "test-session");
        assert_eq!(traj.steps.len(), 6);
        assert_eq!(traj.metadata.llm_rounds, 2);
        assert_eq!(traj.metadata.tool_calls, 1);
        assert_eq!(traj.metadata.total_tokens, 200); // 50+20+100+30
        assert_eq!(traj.metadata.total_cost_sats, 900); // 400+500
    }

    #[test]
    fn test_step_type_classification() {
        assert_eq!(StepType::from_event_type("user"), StepType::UserMessage);
        assert_eq!(StepType::from_event_type("system"), StepType::UserMessage);
        assert_eq!(
            StepType::from_event_type("think_response"),
            StepType::LlmResponse
        );
        assert_eq!(StepType::from_event_type("tool_call"), StepType::ToolCall);
        assert_eq!(
            StepType::from_event_type("tool_result"),
            StepType::ToolResult
        );
        assert_eq!(
            StepType::from_event_type("budget_check"),
            StepType::BudgetCheck
        );
        assert_eq!(StepType::from_event_type("error"), StepType::Error);
        assert_eq!(
            StepType::from_event_type("session_start"),
            StepType::SessionEvent
        );
        assert_eq!(StepType::from_event_type("unknown_type"), StepType::Other);
    }

    #[test]
    fn test_user_messages_extraction() {
        let traj = Trajectory::from_jsonl_str(sample_jsonl(), "test").unwrap();
        let msgs = traj.user_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "What is BSV?");
    }

    #[test]
    fn test_tool_names_extraction() {
        let traj = Trajectory::from_jsonl_str(sample_jsonl(), "test").unwrap();
        let tools = traj.tool_names();
        assert_eq!(tools, vec!["memory_search"]);
    }

    #[test]
    fn test_llm_responses_extraction() {
        let traj = Trajectory::from_jsonl_str(sample_jsonl(), "test").unwrap();
        let responses = traj.llm_responses();
        assert_eq!(responses.len(), 2);
        assert!(responses[0].contains("BSV is Bitcoin SV"));
    }

    #[test]
    fn test_extract_topics() {
        let jsonl = concat!(
            r#"{"ts":1.0,"type":"user","id":"u1","content":"Tell me about BSV transaction fees and UTXO management"}"#,
            "\n",
            r#"{"ts":2.0,"type":"user","id":"u2","content":"How do BSV fees compare to other blockchains"}"#,
            "\n",
        );
        let traj = Trajectory::from_jsonl_str(jsonl, "test").unwrap();
        let topics = traj.extract_topics(5);
        assert!(topics.contains(&"bsv".to_string()));
        assert!(topics.contains(&"fees".to_string()));
    }

    #[test]
    fn test_empty_jsonl() {
        let traj = Trajectory::from_jsonl_str("", "empty").unwrap();
        assert_eq!(traj.steps.len(), 0);
        assert_eq!(traj.metadata.step_count, 0);
        assert_eq!(traj.metadata.duration_secs, 0.0);
    }

    #[test]
    fn test_metadata_tools_used_sorted() {
        let jsonl = concat!(
            r#"{"ts":1.0,"type":"tool_call","id":"tc1","call_id":"c1","name":"execute_bash","arguments":{}}"#,
            "\n",
            r#"{"ts":2.0,"type":"tool_call","id":"tc2","call_id":"c2","name":"memory_search","arguments":{}}"#,
            "\n",
            r#"{"ts":3.0,"type":"tool_call","id":"tc3","call_id":"c3","name":"execute_bash","arguments":{}}"#,
            "\n",
        );
        let traj = Trajectory::from_jsonl_str(jsonl, "test").unwrap();
        assert_eq!(
            traj.metadata.tools_used,
            vec!["execute_bash", "memory_search"]
        );
        assert_eq!(traj.metadata.tool_calls, 3);
    }

    #[test]
    fn test_trajectory_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, sample_jsonl()).unwrap();

        let traj = Trajectory::from_jsonl(path.as_path()).unwrap();
        assert_eq!(traj.session_id, "sess-001");
        assert_eq!(traj.steps.len(), 6);
    }

    #[test]
    fn test_error_count() {
        let jsonl = concat!(
            r#"{"ts":1.0,"type":"user","id":"u1","content":"do something"}"#,
            "\n",
            r#"{"ts":2.0,"type":"error","id":"e1","error":"budget exceeded"}"#,
            "\n",
            r#"{"ts":3.0,"type":"error","id":"e2","error":"tool failed"}"#,
            "\n",
        );
        let traj = Trajectory::from_jsonl_str(jsonl, "test").unwrap();
        assert_eq!(traj.metadata.error_count, 2);
    }
}
