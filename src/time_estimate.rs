//! Human-equivalent time-saved estimation for ROI justification.
//!
//! Provides two estimation modes:
//! 1. **Tag-based override**: Parse `time-saved:Xh` or `time-saved:Xm` from task tags.
//! 2. **Default heuristic**: `min(iterations * 5 + tool_calls * 2, 480)` minutes.
//!
//! The cap of 480 minutes (8 hours) prevents unrealistic estimates.

use serde::{Deserialize, Serialize};

/// Maximum minutes a single task can claim (8 hours).
pub const MAX_MINUTES_PER_TASK: u32 = 480;

/// Minutes of human-equivalent work per agent iteration.
const MINUTES_PER_ITERATION: u32 = 5;

/// Minutes of human-equivalent work per tool call.
const MINUTES_PER_TOOL_CALL: u32 = 2;

/// Parse a `time-saved:Xh` or `time-saved:Xm` tag value.
///
/// Returns `Some(minutes)` if a valid tag is found, `None` otherwise.
/// Accepts decimal hours (e.g., `time-saved:1.5h` → 90 minutes).
pub fn parse_time_saved_tag(tags: &[String]) -> Option<u32> {
    for tag in tags {
        if let Some(value) = tag.strip_prefix("time-saved:") {
            let value = value.trim();
            if let Some(hours_str) = value.strip_suffix('h') {
                if let Ok(hours) = hours_str.parse::<f64>() {
                    if hours >= 0.0 {
                        let minutes = (hours * 60.0).round() as u32;
                        return Some(minutes.min(MAX_MINUTES_PER_TASK));
                    }
                }
            } else if let Some(mins_str) = value.strip_suffix('m') {
                if let Ok(mins) = mins_str.parse::<u32>() {
                    return Some(mins.min(MAX_MINUTES_PER_TASK));
                }
            }
        }
    }
    None
}

/// Compute a default time-saved estimate from iteration and tool call counts.
///
/// Formula: `min(iterations * 5 + tool_calls * 2, 480)` minutes.
pub fn default_estimate(iterations: u32, tool_calls: u32) -> u32 {
    let raw = iterations
        .saturating_mul(MINUTES_PER_ITERATION)
        .saturating_add(tool_calls.saturating_mul(MINUTES_PER_TOOL_CALL));
    raw.min(MAX_MINUTES_PER_TASK)
}

/// Compute time-saved for a task, preferring tag override over default heuristic.
pub fn estimate_time_saved(tags: &[String], iterations: u32, tool_calls: u32) -> u32 {
    parse_time_saved_tag(tags).unwrap_or_else(|| default_estimate(iterations, tool_calls))
}

/// Per-task time-saved detail for the analytics endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTimeSaved {
    pub task_id: String,
    pub description: String,
    pub minutes_saved: u32,
    pub source: String, // "tag" or "estimate"
}

/// Aggregate time-saved analytics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSavedAnalytics {
    pub total_hours_saved: f64,
    pub this_week_hours: f64,
    pub this_month_hours: f64,
    pub per_task: Vec<TaskTimeSaved>,
}

/// Compute aggregate time-saved analytics from a list of per-task data.
///
/// `now` is the current UTC timestamp in seconds since epoch.
pub fn aggregate_time_saved(
    tasks: &[TaskTimeSavedInput],
    now_epoch_secs: i64,
) -> TimeSavedAnalytics {
    let one_week_secs: i64 = 7 * 24 * 3600;
    let one_month_secs: i64 = 30 * 24 * 3600;

    let mut total_minutes: u64 = 0;
    let mut week_minutes: u64 = 0;
    let mut month_minutes: u64 = 0;
    let mut per_task = Vec::with_capacity(tasks.len());

    for t in tasks {
        let minutes = t.minutes_saved as u64;
        total_minutes += minutes;

        let age_secs = now_epoch_secs.saturating_sub(t.started_at_epoch_secs);
        if age_secs <= one_week_secs {
            week_minutes += minutes;
        }
        if age_secs <= one_month_secs {
            month_minutes += minutes;
        }

        per_task.push(TaskTimeSaved {
            task_id: t.task_id.clone(),
            description: t.description.clone(),
            minutes_saved: t.minutes_saved,
            source: t.source.clone(),
        });
    }

    TimeSavedAnalytics {
        total_hours_saved: total_minutes as f64 / 60.0,
        this_week_hours: week_minutes as f64 / 60.0,
        this_month_hours: month_minutes as f64 / 60.0,
        per_task,
    }
}

/// Input for aggregate computation — carries the timestamp for windowing.
#[derive(Debug, Clone)]
pub struct TaskTimeSavedInput {
    pub task_id: String,
    pub description: String,
    pub minutes_saved: u32,
    pub source: String,
    pub started_at_epoch_secs: i64,
}
