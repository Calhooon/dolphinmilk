//! Auto-escalation to human — detects when the agent is stuck and pauses.
//!
//! The `EscalationDetector` monitors agent behavior for signs of being stuck:
//!   - Same tool called 3+ times with identical params (loop)
//!   - Budget usage > 80% with low completion confidence
//!   - Error count > 3 in a single task
//!   - Agent explicitly says it needs help or is uncertain
//!
//! When triggered, the detector emits an `EscalationEvent` describing
//! the reason, which the runner can use to pause the task and request
//! human input.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Data needed to create an on-chain escalation proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationProofData {
    /// Human-readable reason for escalation.
    pub reason: String,
    /// Trigger type identifier (e.g. "tool_loop", "budget_threshold").
    pub trigger_type: String,
    /// Iteration at which escalation was triggered.
    pub iteration: u32,
}

/// Reason why escalation was triggered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EscalationReason {
    /// Same tool called 3+ times with identical parameters.
    ToolLoop {
        tool_name: String,
        call_count: usize,
    },
    /// Budget usage exceeds 80% of task allocation.
    BudgetThreshold {
        spent: u64,
        limit: u64,
        percent: f64,
    },
    /// More than 3 errors encountered in this task.
    ErrorThreshold { error_count: usize },
    /// Agent explicitly expressed uncertainty or requested help.
    ExplicitUncertainty { trigger_phrase: String },
}

impl EscalationReason {
    /// Return a short string identifying the trigger type.
    pub fn trigger_type_str(&self) -> String {
        match self {
            Self::ToolLoop { .. } => "tool_loop".to_string(),
            Self::BudgetThreshold { .. } => "budget_threshold".to_string(),
            Self::ErrorThreshold { .. } => "error_threshold".to_string(),
            Self::ExplicitUncertainty { .. } => "explicit_uncertainty".to_string(),
        }
    }
}

/// An escalation event emitted when the detector triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationEvent {
    /// Why escalation was triggered.
    pub reason: EscalationReason,
    /// Current iteration when escalation occurred.
    pub iteration: u32,
    /// Human-readable description of the escalation.
    pub message: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Whether the task should be paused (default true).
    pub should_pause: bool,
}

/// Resolution provided by a human to resume a paused task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationResolution {
    /// Human-provided guidance or instructions.
    pub guidance: String,
    /// Who resolved it (identity key or name).
    pub resolved_by: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
}

/// Current escalation status for a task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EscalationStatus {
    /// Whether the task is currently escalated (paused).
    pub escalated: bool,
    /// The escalation event, if any.
    pub event: Option<EscalationEvent>,
    /// The resolution, if the escalation has been resolved.
    pub resolution: Option<EscalationResolution>,
}

/// Threshold constants for escalation detection.
const TOOL_LOOP_THRESHOLD: usize = 3;
const ERROR_THRESHOLD: usize = 3;
const BUDGET_THRESHOLD_PERCENT: f64 = 80.0;

/// Phrases that indicate the agent is uncertain or needs help.
const UNCERTAINTY_PHRASES: &[&str] = &[
    "i need help",
    "i'm stuck",
    "i am stuck",
    "i'm not sure how to proceed",
    "i am not sure how to proceed",
    "i need human input",
    "i need human guidance",
    "i cannot figure out",
    "i'm unable to",
    "i am unable to",
    "this is beyond my capabilities",
    "please help",
    "i don't know how to",
    "i do not know how to",
    "escalate to human",
    "requesting human assistance",
];

/// Detects when the agent should escalate to a human.
///
/// Tracks tool call history, error counts, and budget usage to identify
/// patterns that suggest the agent is stuck.
#[derive(Debug)]
pub struct EscalationDetector {
    /// Tool calls keyed by (tool_name, args_hash) -> count.
    tool_call_counts: HashMap<String, usize>,
    /// Number of errors in this task.
    error_count: usize,
    /// Whether escalation has already been triggered (prevent duplicates).
    triggered: bool,
}

impl Default for EscalationDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl EscalationDetector {
    pub fn new() -> Self {
        Self {
            tool_call_counts: HashMap::new(),
            error_count: 0,
            triggered: false,
        }
    }

    /// Record a tool call for loop detection.
    ///
    /// Returns `Some(EscalationReason)` if the same tool+params have been
    /// called `TOOL_LOOP_THRESHOLD` or more times.
    pub fn record_tool_call(
        &mut self,
        tool_name: &str,
        arguments: &str,
    ) -> Option<EscalationReason> {
        if self.triggered {
            return None;
        }

        let key = format!("{}:{}", tool_name, arguments);
        let count = self.tool_call_counts.entry(key).or_insert(0);
        *count += 1;

        if *count >= TOOL_LOOP_THRESHOLD {
            Some(EscalationReason::ToolLoop {
                tool_name: tool_name.to_string(),
                call_count: *count,
            })
        } else {
            None
        }
    }

    /// Record an error occurrence.
    ///
    /// Returns `Some(EscalationReason)` if the error count exceeds the threshold.
    pub fn record_error(&mut self) -> Option<EscalationReason> {
        if self.triggered {
            return None;
        }

        self.error_count += 1;

        if self.error_count > ERROR_THRESHOLD {
            Some(EscalationReason::ErrorThreshold {
                error_count: self.error_count,
            })
        } else {
            None
        }
    }

    /// Check if budget usage warrants escalation.
    ///
    /// Returns `Some(EscalationReason)` if spent exceeds 80% of the limit.
    pub fn check_budget(&self, spent: u64, limit: u64) -> Option<EscalationReason> {
        if self.triggered || limit == 0 {
            return None;
        }

        let percent = (spent as f64 / limit as f64) * 100.0;
        if percent >= BUDGET_THRESHOLD_PERCENT {
            Some(EscalationReason::BudgetThreshold {
                spent,
                limit,
                percent,
            })
        } else {
            None
        }
    }

    /// Check if the agent's text response indicates explicit uncertainty.
    ///
    /// Returns `Some(EscalationReason)` if an uncertainty phrase is detected.
    pub fn check_text(&self, response: &str) -> Option<EscalationReason> {
        if self.triggered {
            return None;
        }

        let lower = response.to_lowercase();
        for phrase in UNCERTAINTY_PHRASES {
            if lower.contains(phrase) {
                return Some(EscalationReason::ExplicitUncertainty {
                    trigger_phrase: phrase.to_string(),
                });
            }
        }
        None
    }

    /// Mark the detector as triggered (prevents duplicate escalations).
    pub fn mark_triggered(&mut self) {
        self.triggered = true;
    }

    /// Whether escalation has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.triggered
    }

    /// Reset the detector state (e.g., after resolution).
    pub fn reset(&mut self) {
        self.tool_call_counts.clear();
        self.error_count = 0;
        self.triggered = false;
    }

    /// Current error count.
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Build an `EscalationEvent` from a reason.
    pub fn build_event(reason: EscalationReason, iteration: u32) -> EscalationEvent {
        let message = match &reason {
            EscalationReason::ToolLoop {
                tool_name,
                call_count,
            } => {
                format!(
                    "Tool '{}' called {} times with identical parameters — possible loop detected",
                    tool_name, call_count
                )
            }
            EscalationReason::BudgetThreshold { percent, .. } => {
                format!(
                    "Budget usage at {:.1}% — high spend with uncertain progress",
                    percent
                )
            }
            EscalationReason::ErrorThreshold { error_count } => {
                format!(
                    "{} errors encountered — agent may need human guidance",
                    error_count
                )
            }
            EscalationReason::ExplicitUncertainty { trigger_phrase } => {
                format!("Agent expressed uncertainty: \"{}\"", trigger_phrase)
            }
        };

        EscalationEvent {
            reason,
            iteration,
            message,
            timestamp: chrono::Utc::now().to_rfc3339(),
            should_pause: true,
        }
    }

    /// Build structured escalation proof data from an event.
    ///
    /// Returns the data needed to create a BRC-18 escalation proof.
    pub fn build_proof_data(event: &EscalationEvent) -> EscalationProofData {
        EscalationProofData {
            reason: event.message.clone(),
            trigger_type: event.reason.trigger_type_str(),
            iteration: event.iteration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_loop_detection() {
        let mut det = EscalationDetector::new();
        let args = r#"{"query": "test"}"#;

        // First two calls: no escalation
        assert!(det.record_tool_call("memory_search", args).is_none());
        assert!(det.record_tool_call("memory_search", args).is_none());

        // Third call: triggers
        let reason = det.record_tool_call("memory_search", args);
        assert!(reason.is_some());
        if let Some(EscalationReason::ToolLoop {
            tool_name,
            call_count,
        }) = reason
        {
            assert_eq!(tool_name, "memory_search");
            assert_eq!(call_count, 3);
        } else {
            panic!("Expected ToolLoop");
        }
    }

    #[test]
    fn test_different_args_no_loop() {
        let mut det = EscalationDetector::new();
        assert!(det
            .record_tool_call("memory_search", r#"{"q":"a"}"#)
            .is_none());
        assert!(det
            .record_tool_call("memory_search", r#"{"q":"b"}"#)
            .is_none());
        assert!(det
            .record_tool_call("memory_search", r#"{"q":"c"}"#)
            .is_none());
    }

    #[test]
    fn test_error_threshold() {
        let mut det = EscalationDetector::new();
        assert!(det.record_error().is_none()); // 1
        assert!(det.record_error().is_none()); // 2
        assert!(det.record_error().is_none()); // 3
        let reason = det.record_error(); // 4 > 3
        assert!(reason.is_some());
        if let Some(EscalationReason::ErrorThreshold { error_count }) = reason {
            assert_eq!(error_count, 4);
        }
    }

    #[test]
    fn test_budget_threshold() {
        let det = EscalationDetector::new();

        // 79% — no escalation
        assert!(det.check_budget(790, 1000).is_none());

        // 80% — triggers
        let reason = det.check_budget(800, 1000);
        assert!(reason.is_some());
        if let Some(EscalationReason::BudgetThreshold { percent, .. }) = reason {
            assert!((percent - 80.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn test_budget_zero_limit() {
        let det = EscalationDetector::new();
        assert!(det.check_budget(500, 0).is_none());
    }

    #[test]
    fn test_explicit_uncertainty() {
        let det = EscalationDetector::new();

        // No uncertainty
        assert!(det.check_text("I found the answer").is_none());

        // Uncertainty detected
        let reason = det.check_text("I'm stuck and don't know how to proceed");
        assert!(reason.is_some());
        if let Some(EscalationReason::ExplicitUncertainty { trigger_phrase }) = reason {
            assert_eq!(trigger_phrase, "i'm stuck");
        }
    }

    #[test]
    fn test_triggered_prevents_duplicates() {
        let mut det = EscalationDetector::new();
        det.mark_triggered();

        // All checks return None when triggered
        assert!(det.record_tool_call("x", "y").is_none());
        assert!(det.record_error().is_none());
        assert!(det.check_text("i'm stuck").is_none());
    }

    #[test]
    fn test_reset() {
        let mut det = EscalationDetector::new();
        det.record_error();
        det.record_error();
        det.mark_triggered();

        det.reset();
        assert!(!det.is_triggered());
        assert_eq!(det.error_count(), 0);
    }

    #[test]
    fn test_build_event() {
        let reason = EscalationReason::ErrorThreshold { error_count: 5 };
        let event = EscalationDetector::build_event(reason, 3);

        assert_eq!(event.iteration, 3);
        assert!(event.message.contains("5 errors"));
        assert!(event.should_pause);
    }

    #[test]
    fn test_escalation_status_default() {
        let status = EscalationStatus::default();
        assert!(!status.escalated);
        assert!(status.event.is_none());
        assert!(status.resolution.is_none());
    }

    #[test]
    fn test_escalation_reason_serde() {
        let reason = EscalationReason::ToolLoop {
            tool_name: "test".to_string(),
            call_count: 3,
        };
        let json = serde_json::to_string(&reason).unwrap();
        let parsed: EscalationReason = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reason);
    }

    #[test]
    fn test_escalation_event_serde() {
        let event = EscalationEvent {
            reason: EscalationReason::ErrorThreshold { error_count: 4 },
            iteration: 2,
            message: "test".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            should_pause: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EscalationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.iteration, 2);
        assert!(parsed.should_pause);
    }

    #[test]
    fn test_escalation_resolution_serde() {
        let resolution = EscalationResolution {
            guidance: "Try a different approach".to_string(),
            resolved_by: "operator".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&resolution).unwrap();
        let parsed: EscalationResolution = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.guidance, "Try a different approach");
    }

    #[test]
    fn test_uncertainty_case_insensitive() {
        let det = EscalationDetector::new();
        let reason = det.check_text("I NEED HELP with this task");
        assert!(reason.is_some());
    }
}
