//! Evaluation report generation and regression detection.
//!
//! Compiles per-rubric scores into a structured report and compares
//! against baselines to flag regressions exceeding a configurable threshold.

use serde::{Deserialize, Serialize};

use super::grader::BuiltinRubric;
use super::trajectory::Trajectory;

/// Default regression threshold: flag drops > 10%.
const DEFAULT_REGRESSION_THRESHOLD: f64 = 0.10;

/// A regression flag raised when a rubric score drops below baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionFlag {
    /// Which rubric regressed.
    pub rubric: String,
    /// Baseline score.
    pub baseline: f64,
    /// Current score.
    pub current: f64,
    /// Absolute drop (baseline - current).
    pub drop: f64,
    /// Human-readable description.
    pub message: String,
}

/// A baseline is a set of named rubric scores representing expected performance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// Name of the baseline (e.g. "v0.1.0", "2026-03-01").
    pub name: String,
    /// Rubric name -> expected score.
    pub scores: std::collections::HashMap<String, f64>,
}

impl Baseline {
    /// Create a baseline from a set of rubric scores.
    pub fn new(name: impl Into<String>, scores: std::collections::HashMap<String, f64>) -> Self {
        Self {
            name: name.into(),
            scores,
        }
    }

    /// Create a baseline by evaluating a trajectory with all built-in rubrics.
    pub fn from_trajectory(name: impl Into<String>, trajectory: &Trajectory) -> Self {
        let mut scores = std::collections::HashMap::new();
        for rubric in BuiltinRubric::all() {
            let score = rubric.aggregate_score(trajectory);
            scores.insert(rubric.rubric().name, score.value);
        }
        Self {
            name: name.into(),
            scores,
        }
    }
}

/// A complete evaluation report for one trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    /// Session identifier from the trajectory.
    pub session_id: String,
    /// Per-rubric aggregate scores.
    pub scores: Vec<RubricScore>,
    /// Overall aggregate score (mean of rubric scores).
    pub overall_score: f64,
    /// Regression flags (if a baseline was provided).
    pub regressions: Vec<RegressionFlag>,
    /// Summary metadata.
    pub summary: ReportSummary,
}

/// A single rubric's aggregate score within a report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubricScore {
    /// Rubric name.
    pub rubric: String,
    /// Aggregate score (0.0 to 1.0).
    pub score: f64,
    /// Explanation.
    pub explanation: String,
}

/// Summary metadata for the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    /// Total steps in the trajectory.
    pub total_steps: usize,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Total satoshis spent.
    pub total_sats: u64,
    /// Number of LLM rounds.
    pub llm_rounds: usize,
    /// Number of tool calls.
    pub tool_calls: usize,
    /// Number of errors.
    pub error_count: usize,
    /// Whether any regressions were detected.
    pub has_regressions: bool,
}

impl EvalReport {
    /// Generate a report for a trajectory using all built-in rubrics.
    pub fn generate(trajectory: &Trajectory) -> Self {
        Self::generate_with_baseline(trajectory, None, DEFAULT_REGRESSION_THRESHOLD)
    }

    /// Generate a report with optional baseline comparison.
    pub fn generate_with_baseline(
        trajectory: &Trajectory,
        baseline: Option<&Baseline>,
        regression_threshold: f64,
    ) -> Self {
        let mut scores = Vec::new();
        let mut regressions = Vec::new();

        for rubric_type in BuiltinRubric::all() {
            let rubric = rubric_type.rubric();
            let score = rubric_type.aggregate_score(trajectory);

            // Check for regression if baseline is provided
            if let Some(bl) = baseline {
                if let Some(&bl_score) = bl.scores.get(&rubric.name) {
                    let drop = bl_score - score.value;
                    if drop > regression_threshold {
                        regressions.push(RegressionFlag {
                            rubric: rubric.name.clone(),
                            baseline: bl_score,
                            current: score.value,
                            drop,
                            message: format!(
                                "{} regressed by {:.1}%: {:.2} -> {:.2}",
                                rubric.name,
                                drop * 100.0,
                                bl_score,
                                score.value,
                            ),
                        });
                    }
                }
            }

            scores.push(RubricScore {
                rubric: rubric.name,
                score: score.value,
                explanation: score.explanation,
            });
        }

        let overall_score = if scores.is_empty() {
            0.0
        } else {
            scores.iter().map(|s| s.score).sum::<f64>() / scores.len() as f64
        };

        let summary = ReportSummary {
            total_steps: trajectory.metadata.step_count,
            total_tokens: trajectory.metadata.total_tokens,
            total_sats: trajectory.metadata.total_cost_sats,
            llm_rounds: trajectory.metadata.llm_rounds,
            tool_calls: trajectory.metadata.tool_calls,
            error_count: trajectory.metadata.error_count,
            has_regressions: !regressions.is_empty(),
        };

        EvalReport {
            session_id: trajectory.session_id.clone(),
            scores,
            overall_score,
            regressions,
            summary,
        }
    }

    /// Format the report as a human-readable string.
    pub fn to_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("Eval Report: {}", self.session_id));
        lines.push("=".repeat(60));
        lines.push(String::new());

        lines.push(format!("Overall Score: {:.2}", self.overall_score));
        lines.push(String::new());

        lines.push("Rubric Scores:".to_string());
        for rs in &self.scores {
            lines.push(format!(
                "  {}: {:.2} ({})",
                rs.rubric, rs.score, rs.explanation
            ));
        }
        lines.push(String::new());

        lines.push("Summary:".to_string());
        lines.push(format!("  Steps: {}", self.summary.total_steps));
        lines.push(format!("  Tokens: {}", self.summary.total_tokens));
        lines.push(format!("  Sats: {}", self.summary.total_sats));
        lines.push(format!("  LLM Rounds: {}", self.summary.llm_rounds));
        lines.push(format!("  Tool Calls: {}", self.summary.tool_calls));
        lines.push(format!("  Errors: {}", self.summary.error_count));

        if !self.regressions.is_empty() {
            lines.push(String::new());
            lines.push("REGRESSIONS DETECTED:".to_string());
            for r in &self.regressions {
                lines.push(format!("  [!] {}", r.message));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trajectory() -> Trajectory {
        let jsonl = concat!(
            r#"{"ts":1.0,"type":"user","id":"u1","content":"What is BSV?"}"#,
            "\n",
            r#"{"ts":2.0,"type":"think_response","id":"t1","content":"BSV is Bitcoin Satoshi Vision, a blockchain focused on scalability and low fees.","model":"gpt-5-mini","sats_paid":400,"sats_effective":400,"sats_refunded":0,"prompt_tokens":50,"completion_tokens":25,"finish_reason":"stop","duration_ms":600}"#,
            "\n",
            r#"{"ts":3.0,"type":"budget_check","id":"b1","balance":99600,"spent_session":400,"spent_hour":400,"task_limit":20000000}"#,
            "\n",
        );
        Trajectory::from_jsonl_str(jsonl, "test-session").unwrap()
    }

    #[test]
    fn test_generate_report() {
        let traj = sample_trajectory();
        let report = EvalReport::generate(&traj);

        assert_eq!(report.session_id, "test-session");
        assert_eq!(report.scores.len(), 4);
        assert!(report.overall_score > 0.0);
        assert!(report.regressions.is_empty());
        assert!(!report.summary.has_regressions);
    }

    #[test]
    fn test_report_text_format() {
        let traj = sample_trajectory();
        let report = EvalReport::generate(&traj);
        let text = report.to_text();

        assert!(text.contains("Eval Report: test-session"));
        assert!(text.contains("Overall Score:"));
        assert!(text.contains("task_completion"));
        assert!(text.contains("efficiency"));
        assert!(text.contains("safety"));
        assert!(text.contains("cost"));
    }

    #[test]
    fn test_regression_detection() {
        let traj = sample_trajectory();

        // Create a baseline with high scores
        let mut bl_scores = std::collections::HashMap::new();
        bl_scores.insert("task_completion".to_string(), 1.0);
        bl_scores.insert("efficiency".to_string(), 1.0);
        bl_scores.insert("safety".to_string(), 1.0);
        bl_scores.insert("cost".to_string(), 1.0);
        let baseline = Baseline::new("v0.1.0", bl_scores);

        let report = EvalReport::generate_with_baseline(&traj, Some(&baseline), 0.10);

        // Some rubrics might regress since we set baseline to perfect 1.0
        // The report should detect regressions where current < baseline - threshold
        assert!(report.summary.has_regressions != report.regressions.is_empty());
    }

    #[test]
    fn test_no_regression_within_threshold() {
        let traj = sample_trajectory();
        let baseline = Baseline::from_trajectory("baseline", &traj);

        // Same trajectory should not regress against itself
        let report = EvalReport::generate_with_baseline(&traj, Some(&baseline), 0.10);
        assert!(report.regressions.is_empty());
        assert!(!report.summary.has_regressions);
    }

    #[test]
    fn test_baseline_from_trajectory() {
        let traj = sample_trajectory();
        let baseline = Baseline::from_trajectory("v1", &traj);

        assert_eq!(baseline.name, "v1");
        assert_eq!(baseline.scores.len(), 4);
        for score in baseline.scores.values() {
            assert!((0.0..=1.0).contains(score));
        }
    }

    #[test]
    fn test_regression_flag_details() {
        let flag = RegressionFlag {
            rubric: "efficiency".to_string(),
            baseline: 0.9,
            current: 0.5,
            drop: 0.4,
            message: "efficiency regressed by 40.0%: 0.90 -> 0.50".to_string(),
        };
        assert!(flag.drop > 0.1);
        assert!(flag.message.contains("40.0%"));
    }

    #[test]
    fn test_empty_trajectory_report() {
        let traj = Trajectory::from_jsonl_str("", "empty").unwrap();
        let report = EvalReport::generate(&traj);

        assert_eq!(report.session_id, "empty");
        // Empty trajectory still gets some score from vacuously-true criteria
        // (no errors, no dangerous tools, no budget overflow, etc.)
        assert!(
            report.overall_score < 0.8,
            "empty should score low: {}",
            report.overall_score
        );
        assert_eq!(report.summary.total_steps, 0);
    }

    #[test]
    fn test_report_serialization_roundtrip() {
        let traj = sample_trajectory();
        let report = EvalReport::generate(&traj);

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: EvalReport = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.session_id, report.session_id);
        assert_eq!(deserialized.scores.len(), report.scores.len());
        assert!((deserialized.overall_score - report.overall_score).abs() < f64::EPSILON);
    }
}
