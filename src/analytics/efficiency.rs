//! Efficiency analytics — pure computation from transcript data.
//!
//! Scans `workspace/tasks/*/session.jsonl` transcripts to compute:
//! - Cost per task, cost per token, tokens per iteration
//! - Daily time series (last 30 days)
//! - Model comparison breakdown
//! - Period-over-period comparison (this week vs last week)

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::transcript::{Transcript, TranscriptEvent};

// ── Output structs ──────────────────────────────────────────────────────

/// Top-level efficiency report returned by the API.
#[derive(Debug, Clone, Serialize)]
pub struct EfficiencyReport {
    /// Average sats per task (across all tasks).
    pub cost_per_task_sats: f64,
    /// Average sats per token (prompt + completion).
    pub cost_per_token_sats: f64,
    /// Average tokens (prompt + completion) per iteration.
    pub tokens_per_iteration: f64,
    /// Daily metrics for the last 30 days (sorted chronologically).
    pub daily_trend: Vec<DayMetrics>,
    /// Per-model breakdown.
    pub by_model: HashMap<String, ModelMetrics>,
    /// This week vs last week comparison.
    pub period_comparison: PeriodComparison,
}

/// Aggregated metrics for a single calendar day.
#[derive(Debug, Clone, Serialize)]
pub struct DayMetrics {
    /// ISO 8601 date string (YYYY-MM-DD).
    pub date: String,
    /// Number of tasks that started on this day.
    pub tasks: u32,
    /// Total satoshis spent on this day.
    pub total_sats: u64,
    /// Total tokens (prompt + completion) on this day.
    pub total_tokens: u64,
    /// Average sats per task on this day.
    pub avg_cost_per_task: f64,
}

/// Per-model aggregated metrics.
#[derive(Debug, Clone, Serialize)]
pub struct ModelMetrics {
    /// Number of tasks using this model (primary model).
    pub tasks: u32,
    /// Total satoshis spent on this model.
    pub total_sats: u64,
    /// Total tokens consumed by this model.
    pub total_tokens: u64,
    /// Average sats per task for this model.
    pub avg_cost_per_task: f64,
    /// Average tokens per task for this model.
    pub avg_tokens_per_task: f64,
}

/// Summary for a single time period.
#[derive(Debug, Clone, Serialize)]
pub struct PeriodSummary {
    /// Label for the period (e.g. "2026-03-16 to 2026-03-22").
    pub label: String,
    pub tasks: u32,
    pub total_sats: u64,
    pub total_tokens: u64,
    pub avg_cost_per_task: f64,
}

/// Comparison between two adjacent periods.
#[derive(Debug, Clone, Serialize)]
pub struct PeriodComparison {
    pub current: PeriodSummary,
    pub previous: PeriodSummary,
    /// Percentage change in cost per task (positive = more expensive).
    pub cost_change_pct: f64,
    /// Percentage change in tokens per sat (positive = more efficient).
    pub efficiency_change_pct: f64,
}

// ── Internal intermediate data ──────────────────────────────────────────

/// Extracted per-task summary from a single transcript.
#[derive(Debug, Clone)]
struct TaskSummary {
    /// Calendar date the task started (from session_start or first event ts).
    date: NaiveDate,
    /// Primary model used (from first think_response).
    model: String,
    /// Total sats spent (LLM inference + tool spending).
    total_sats: u64,
    /// Total tokens (prompt + completion).
    total_tokens: u64,
    /// Number of think iterations (think_response events).
    iterations: u32,
}

// ── Core computation ────────────────────────────────────────────────────

/// Compute an efficiency report from all task transcripts in the workspace.
///
/// This is a pure function that reads transcript files but never modifies them.
pub fn compute_efficiency(workspace: &Path) -> EfficiencyReport {
    compute_efficiency_at(workspace, Utc::now())
}

/// Compute an efficiency report using a specific reference time.
///
/// Exposed for testing so we can control "now".
pub fn compute_efficiency_at(workspace: &Path, now: DateTime<Utc>) -> EfficiencyReport {
    let summaries = scan_transcripts(workspace);
    build_report(&summaries, now)
}

/// Scan all task transcripts and extract per-task summaries.
fn scan_transcripts(workspace: &Path) -> Vec<TaskSummary> {
    let tasks_dir = workspace.join("tasks");
    let mut summaries = Vec::new();

    let entries = match std::fs::read_dir(&tasks_dir) {
        Ok(e) => e,
        Err(_) => return summaries,
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

        if let Some(summary) = extract_task_summary(&transcript_path) {
            summaries.push(summary);
        }
    }

    summaries
}

/// Extract a TaskSummary from a single transcript file.
fn extract_task_summary(transcript_path: &Path) -> Option<TaskSummary> {
    let transcript = Transcript::new(transcript_path.to_path_buf());
    let events = transcript.replay();
    if events.is_empty() {
        return None;
    }

    // Determine task start date
    let date = extract_task_date(events);

    // Extract primary model from first think_response
    let model = events
        .iter()
        .find(|e| e.event_type == "think_response")
        .and_then(|e| e.data.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Total sats: LLM inference (sats_effective) + tool spending (sats_paid)
    let total_sats = transcript.total_sats_spent();

    // Total tokens
    let (prompt_tokens, completion_tokens) = transcript.total_tokens();
    let total_tokens = prompt_tokens + completion_tokens;

    // Count iterations (think_response events)
    let iterations = events
        .iter()
        .filter(|e| e.event_type == "think_response")
        .count() as u32;

    // Only include tasks that had at least one think_response
    // (otherwise there's no meaningful cost data)
    if iterations == 0 {
        return None;
    }

    Some(TaskSummary {
        date,
        model,
        total_sats,
        total_tokens,
        iterations,
    })
}

/// Extract the task start date from transcript events.
///
/// Prefers session_start timestamp, falls back to first event timestamp.
fn extract_task_date(events: &[TranscriptEvent]) -> NaiveDate {
    let ts = events
        .iter()
        .find(|e| e.event_type == "session_start")
        .map(|e| e.ts)
        .unwrap_or_else(|| events.first().map(|e| e.ts).unwrap_or(0.0));

    timestamp_to_date(ts)
}

/// Convert a Unix timestamp (f64) to a NaiveDate.
fn timestamp_to_date(ts: f64) -> NaiveDate {
    DateTime::from_timestamp(ts as i64, 0)
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap())
        .date_naive()
}

// ── Report building ─────────────────────────────────────────────────────

/// Build the full efficiency report from extracted task summaries.
fn build_report(summaries: &[TaskSummary], now: DateTime<Utc>) -> EfficiencyReport {
    let task_count = summaries.len() as f64;

    // Global averages
    let total_sats: u64 = summaries.iter().map(|s| s.total_sats).sum();
    let total_tokens: u64 = summaries.iter().map(|s| s.total_tokens).sum();
    let total_iterations: u32 = summaries.iter().map(|s| s.iterations).sum();

    let cost_per_task_sats = if task_count > 0.0 {
        total_sats as f64 / task_count
    } else {
        0.0
    };
    let cost_per_token_sats = if total_tokens > 0 {
        total_sats as f64 / total_tokens as f64
    } else {
        0.0
    };
    let tokens_per_iteration = if total_iterations > 0 {
        total_tokens as f64 / total_iterations as f64
    } else {
        0.0
    };

    // Daily trend (last 30 days)
    let daily_trend = build_daily_trend(summaries, now);

    // Model breakdown
    let by_model = build_model_breakdown(summaries);

    // Period comparison (this week vs last week)
    let period_comparison = build_period_comparison(summaries, now);

    EfficiencyReport {
        cost_per_task_sats,
        cost_per_token_sats,
        tokens_per_iteration,
        daily_trend,
        by_model,
        period_comparison,
    }
}

/// Build daily metrics for the last 30 days.
fn build_daily_trend(summaries: &[TaskSummary], now: DateTime<Utc>) -> Vec<DayMetrics> {
    let today = now.date_naive();
    let start_date = today - Duration::days(29); // 30 days including today

    // Group summaries by date
    let mut by_date: HashMap<NaiveDate, Vec<&TaskSummary>> = HashMap::new();
    for s in summaries {
        if s.date >= start_date && s.date <= today {
            by_date.entry(s.date).or_default().push(s);
        }
    }

    // Build metrics for each day in range (include days with zero activity)
    let mut trend = Vec::with_capacity(30);
    let mut date = start_date;
    while date <= today {
        let day_tasks = by_date.get(&date);
        let tasks = day_tasks.map(|t| t.len() as u32).unwrap_or(0);
        let total_sats: u64 = day_tasks
            .map(|t| t.iter().map(|s| s.total_sats).sum())
            .unwrap_or(0);
        let total_tokens: u64 = day_tasks
            .map(|t| t.iter().map(|s| s.total_tokens).sum())
            .unwrap_or(0);
        let avg_cost_per_task = if tasks > 0 {
            total_sats as f64 / tasks as f64
        } else {
            0.0
        };

        trend.push(DayMetrics {
            date: date.format("%Y-%m-%d").to_string(),
            tasks,
            total_sats,
            total_tokens,
            avg_cost_per_task,
        });
        date += Duration::days(1);
    }

    trend
}

/// Build per-model breakdown.
fn build_model_breakdown(summaries: &[TaskSummary]) -> HashMap<String, ModelMetrics> {
    let mut by_model: HashMap<String, Vec<&TaskSummary>> = HashMap::new();
    for s in summaries {
        by_model.entry(s.model.clone()).or_default().push(s);
    }

    by_model
        .into_iter()
        .map(|(model, tasks)| {
            let task_count = tasks.len() as u32;
            let total_sats: u64 = tasks.iter().map(|s| s.total_sats).sum();
            let total_tokens: u64 = tasks.iter().map(|s| s.total_tokens).sum();
            let avg_cost_per_task = if task_count > 0 {
                total_sats as f64 / task_count as f64
            } else {
                0.0
            };
            let avg_tokens_per_task = if task_count > 0 {
                total_tokens as f64 / task_count as f64
            } else {
                0.0
            };

            (
                model,
                ModelMetrics {
                    tasks: task_count,
                    total_sats,
                    total_tokens,
                    avg_cost_per_task,
                    avg_tokens_per_task,
                },
            )
        })
        .collect()
}

/// Build week-over-week comparison.
///
/// "Current week" = last 7 days ending today.
/// "Previous week" = the 7 days before that.
fn build_period_comparison(summaries: &[TaskSummary], now: DateTime<Utc>) -> PeriodComparison {
    let today = now.date_naive();

    // Current week: today - 6 days .. today (inclusive)
    let current_start = today - Duration::days(6);
    let current_end = today;

    // Previous week: today - 13 days .. today - 7 days (inclusive)
    let previous_start = today - Duration::days(13);
    let previous_end = today - Duration::days(7);

    let current = summarize_period(summaries, current_start, current_end, "current_week");
    let previous = summarize_period(summaries, previous_start, previous_end, "previous_week");

    let cost_change_pct = if previous.avg_cost_per_task > 0.0 {
        ((current.avg_cost_per_task - previous.avg_cost_per_task) / previous.avg_cost_per_task)
            * 100.0
    } else {
        0.0
    };

    // Efficiency = tokens per sat (higher is better)
    let current_efficiency = if current.total_sats > 0 {
        current.total_tokens as f64 / current.total_sats as f64
    } else {
        0.0
    };
    let previous_efficiency = if previous.total_sats > 0 {
        previous.total_tokens as f64 / previous.total_sats as f64
    } else {
        0.0
    };
    let efficiency_change_pct = if previous_efficiency > 0.0 {
        ((current_efficiency - previous_efficiency) / previous_efficiency) * 100.0
    } else {
        0.0
    };

    PeriodComparison {
        current,
        previous,
        cost_change_pct,
        efficiency_change_pct,
    }
}

/// Summarize tasks within a date range.
fn summarize_period(
    summaries: &[TaskSummary],
    start: NaiveDate,
    end: NaiveDate,
    label_hint: &str,
) -> PeriodSummary {
    let filtered: Vec<&TaskSummary> = summaries
        .iter()
        .filter(|s| s.date >= start && s.date <= end)
        .collect();

    let tasks = filtered.len() as u32;
    let total_sats: u64 = filtered.iter().map(|s| s.total_sats).sum();
    let total_tokens: u64 = filtered.iter().map(|s| s.total_tokens).sum();
    let avg_cost_per_task = if tasks > 0 {
        total_sats as f64 / tasks as f64
    } else {
        0.0
    };

    let _ = label_hint; // reserved for future use (e.g. "current_week" vs "previous_week")
    let label = format!("{} to {}", start.format("%Y-%m-%d"), end.format("%Y-%m-%d"));

    PeriodSummary {
        label,
        tasks,
        total_sats,
        total_tokens,
        avg_cost_per_task,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_to_date_zero() {
        let date = timestamp_to_date(0.0);
        assert_eq!(date, NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
    }

    #[test]
    fn test_timestamp_to_date_recent() {
        // 1774137600 = 2026-03-22 (verified)
        let date = timestamp_to_date(1774137600.0);
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 22).unwrap());
    }

    #[test]
    fn test_build_report_empty() {
        let now = Utc::now();
        let report = build_report(&[], now);
        assert_eq!(report.cost_per_task_sats, 0.0);
        assert_eq!(report.cost_per_token_sats, 0.0);
        assert_eq!(report.tokens_per_iteration, 0.0);
        assert_eq!(report.daily_trend.len(), 30);
        assert!(report.by_model.is_empty());
    }
}
