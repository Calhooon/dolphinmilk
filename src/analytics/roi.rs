//! ROI reporting (#51).
//!
//! Correlates spending with task outcomes to produce return-on-investment metrics.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::transcript::Transcript;

// ── Output structs ──────────────────────────────────────────────────────

/// ROI report correlating spending with task outcomes.
#[derive(Debug, Clone, Serialize)]
pub struct RoiReport {
    /// Total satoshis spent across all tasks.
    pub total_spent_sats: u64,
    /// Number of tasks that completed successfully.
    pub tasks_completed: u32,
    /// Number of tasks that failed.
    pub tasks_failed: u32,
    /// Average cost per completed task.
    pub avg_cost_per_task: f64,
    /// Efficiency score: tasks_completed / total_spent_sats * 1_000_000 (tasks per M sats).
    pub efficiency_score: f64,
    /// Average cost per completed task vs failed task ratio.
    pub cost_per_completed_task: f64,
    /// Average cost per failed task.
    pub cost_per_failed_task: f64,
    /// Total tokens consumed across all tasks.
    pub total_tokens: u64,
    /// Average quality score from eval framework (0.0 if unavailable).
    pub avg_quality_score: f64,
}

// ── Internal intermediate data ──────────────────────────────────────────

/// Internal summary for ROI computation.
#[derive(Debug, Clone)]
struct RoiTaskSummary {
    sats: u64,
    tokens: u64,
    completed: bool,
    quality_score: Option<f64>,
}

// ── Core computation ────────────────────────────────────────────────────

/// Compute an ROI report from all task transcripts in the workspace.
pub fn compute_roi(workspace: &Path) -> RoiReport {
    let tasks_dir = workspace.join("tasks");
    let mut summaries = Vec::new();

    let entries = match fs::read_dir(&tasks_dir) {
        Ok(e) => e,
        Err(_) => return empty_roi(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let transcript_path = path.join("session.jsonl");
        if !transcript_path.exists() {
            continue;
        }
        if let Some(summary) = extract_roi_summary(&transcript_path) {
            summaries.push(summary);
        }
    }

    build_roi_report(&summaries)
}

/// Compute ROI report from pre-built summaries (for testing).
pub fn compute_roi_from_summaries(task_summaries: &[(u64, u64, bool, Option<f64>)]) -> RoiReport {
    let summaries: Vec<RoiTaskSummary> = task_summaries
        .iter()
        .map(|(sats, tokens, completed, quality)| RoiTaskSummary {
            sats: *sats,
            tokens: *tokens,
            completed: *completed,
            quality_score: *quality,
        })
        .collect();
    build_roi_report(&summaries)
}

fn extract_roi_summary(transcript_path: &Path) -> Option<RoiTaskSummary> {
    let transcript = Transcript::new(transcript_path.to_path_buf());
    let events = transcript.replay();

    if events.is_empty() {
        return None;
    }

    // Check for at least one think_response
    let has_think = events.iter().any(|e| e.event_type == "think_response");
    if !has_think {
        return None;
    }

    let sats = transcript.total_sats_spent();
    let (prompt, completion) = transcript.total_tokens();
    let tokens = prompt + completion;

    // Determine completion status from session_end event
    let completed = events
        .iter()
        .find(|e| e.event_type == "session_end")
        .map(|e| {
            // A task is "completed" if it has a session_end without an error
            let has_error = e
                .data
                .get("error")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
            !has_error
        })
        .unwrap_or(false); // No session_end means incomplete/failed

    // Try to load eval quality score if available
    let quality_score = load_eval_score(transcript_path);

    Some(RoiTaskSummary {
        sats,
        tokens,
        completed,
        quality_score,
    })
}

/// Try to load an eval score from a sibling eval.json file.
fn load_eval_score(transcript_path: &Path) -> Option<f64> {
    let eval_path = transcript_path.with_file_name("eval.json");
    if !eval_path.exists() {
        return None;
    }
    let data = fs::read_to_string(&eval_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;
    json.get("aggregate_score")
        .and_then(|v| v.as_f64())
        .or_else(|| json.get("score").and_then(|v| v.as_f64()))
}

fn build_roi_report(summaries: &[RoiTaskSummary]) -> RoiReport {
    if summaries.is_empty() {
        return empty_roi();
    }

    let total_spent_sats: u64 = summaries.iter().map(|s| s.sats).sum();
    let total_tokens: u64 = summaries.iter().map(|s| s.tokens).sum();
    let tasks_completed = summaries.iter().filter(|s| s.completed).count() as u32;
    let tasks_failed = summaries.iter().filter(|s| !s.completed).count() as u32;

    let total_tasks = summaries.len() as f64;
    let avg_cost_per_task = if total_tasks > 0.0 {
        total_spent_sats as f64 / total_tasks
    } else {
        0.0
    };

    let completed_sats: u64 = summaries
        .iter()
        .filter(|s| s.completed)
        .map(|s| s.sats)
        .sum();
    let failed_sats: u64 = summaries
        .iter()
        .filter(|s| !s.completed)
        .map(|s| s.sats)
        .sum();

    let cost_per_completed_task = if tasks_completed > 0 {
        completed_sats as f64 / tasks_completed as f64
    } else {
        0.0
    };

    let cost_per_failed_task = if tasks_failed > 0 {
        failed_sats as f64 / tasks_failed as f64
    } else {
        0.0
    };

    let efficiency_score = if total_spent_sats > 0 {
        (tasks_completed as f64 / total_spent_sats as f64) * 1_000_000.0
    } else {
        0.0
    };

    let quality_scores: Vec<f64> = summaries.iter().filter_map(|s| s.quality_score).collect();
    let avg_quality_score = if quality_scores.is_empty() {
        0.0
    } else {
        quality_scores.iter().sum::<f64>() / quality_scores.len() as f64
    };

    RoiReport {
        total_spent_sats,
        tasks_completed,
        tasks_failed,
        avg_cost_per_task,
        efficiency_score,
        cost_per_completed_task,
        cost_per_failed_task,
        total_tokens,
        avg_quality_score,
    }
}

fn empty_roi() -> RoiReport {
    RoiReport {
        total_spent_sats: 0,
        tasks_completed: 0,
        tasks_failed: 0,
        avg_cost_per_task: 0.0,
        efficiency_score: 0.0,
        cost_per_completed_task: 0.0,
        cost_per_failed_task: 0.0,
        total_tokens: 0,
        avg_quality_score: 0.0,
    }
}
