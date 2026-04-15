//! Integration tests for the analytics module: efficiency trends, cost replay,
//! ROI reporting, provider benchmarks, server routes, and cost_analysis tool.

use std::collections::HashMap;
use std::fs;
use std::io::Write;

use chrono::{TimeZone, Utc};
use dolphin_milk::analytics::{
    aggregate_benchmarks, compute_cost_analysis, compute_efficiency_at, compute_roi,
    compute_roi_from_summaries, cost_replay, cost_replay_from_events, infer_provider,
    load_benchmarks, load_benchmarks_from_path, record_benchmark, BenchmarkEntry,
};
use dolphin_milk::transcript::TranscriptEvent;
use tempfile::tempdir;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Create a synthetic transcript JSONL file for a task.
///
/// `iterations` is a list of (model, sats_effective, prompt_tokens, completion_tokens) per think step.
/// `start_ts` is the Unix epoch timestamp for the session_start event.
fn write_transcript(
    workspace: &std::path::Path,
    task_id: &str,
    start_ts: f64,
    iterations: &[(&str, u64, u64, u64)],
) {
    let task_dir = workspace.join("tasks").join(task_id);
    fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("session.jsonl");
    let mut f = fs::File::create(&path).unwrap();

    // session_start event
    let start_event = serde_json::json!({
        "ts": start_ts,
        "type": "session_start",
        "id": "a1b2c3d4",
        "task": task_id,
        "model": iterations.first().map(|i| i.0).unwrap_or("unknown"),
    });
    writeln!(f, "{}", serde_json::to_string(&start_event).unwrap()).unwrap();

    // think_response events
    for (i, (model, sats_effective, prompt_tokens, completion_tokens)) in
        iterations.iter().enumerate()
    {
        let think_event = serde_json::json!({
            "ts": start_ts + (i as f64 + 1.0) * 10.0,
            "type": "think_response",
            "id": format!("tr{:06}", i),
            "role": "assistant",
            "content": format!("Response iteration {i}"),
            "model": model,
            "sats_paid": sats_effective + 50,
            "sats_effective": sats_effective,
            "sats_refunded": 50,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "finish_reason": "stop",
            "duration_ms": 1500,
        });
        writeln!(f, "{}", serde_json::to_string(&think_event).unwrap()).unwrap();
    }

    // session_end event
    let end_event = serde_json::json!({
        "ts": start_ts + (iterations.len() as f64 + 1.0) * 10.0,
        "type": "session_end",
        "id": "z9y8x7w6",
        "iterations": iterations.len(),
        "sats_spent": iterations.iter().map(|i| i.1).sum::<u64>(),
    });
    writeln!(f, "{}", serde_json::to_string(&end_event).unwrap()).unwrap();
}

/// Get a Unix timestamp for a given date (midnight UTC).
fn date_ts(year: i32, month: u32, day: u32) -> f64 {
    Utc.with_ymd_and_hms(year, month, day, 12, 0, 0)
        .unwrap()
        .timestamp() as f64
}

/// Reference "now" for tests — 2026-03-22 noon UTC.
fn test_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 22, 12, 0, 0).unwrap()
}

// ── Tests ───────────────────────────────────────────────────────────────

#[test]
fn test_empty_workspace_returns_zeros() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    fs::create_dir_all(workspace.join("tasks")).unwrap();

    let report = compute_efficiency_at(workspace, test_now());

    assert_eq!(report.cost_per_task_sats, 0.0);
    assert_eq!(report.cost_per_token_sats, 0.0);
    assert_eq!(report.tokens_per_iteration, 0.0);
    assert_eq!(report.daily_trend.len(), 30);
    assert!(report.by_model.is_empty());
    assert_eq!(report.period_comparison.current.tasks, 0);
    assert_eq!(report.period_comparison.previous.tasks, 0);
}

#[test]
fn test_missing_tasks_dir_returns_zeros() {
    let dir = tempdir().unwrap();
    // Don't create tasks/ directory at all
    let report = compute_efficiency_at(dir.path(), test_now());
    assert_eq!(report.cost_per_task_sats, 0.0);
    assert!(report.by_model.is_empty());
}

#[test]
fn test_single_task_metrics() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Task with 2 iterations: 200 sats each, 100+50 tokens each
    let ts = date_ts(2026, 3, 22);
    write_transcript(
        workspace,
        "task-001",
        ts,
        &[("gpt-5", 200, 100, 50), ("gpt-5", 200, 100, 50)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // 1 task, 400 sats total
    assert_eq!(report.cost_per_task_sats, 400.0);
    // 400 sats / 300 tokens
    assert!((report.cost_per_token_sats - 400.0 / 300.0).abs() < 0.001);
    // 300 tokens / 2 iterations = 150
    assert!((report.tokens_per_iteration - 150.0).abs() < 0.001);
}

#[test]
fn test_multiple_tasks_daily_aggregation() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Two tasks on 2026-03-20, one task on 2026-03-21
    write_transcript(
        workspace,
        "task-a",
        date_ts(2026, 3, 20),
        &[("gpt-5", 100, 50, 25)],
    );
    write_transcript(
        workspace,
        "task-b",
        date_ts(2026, 3, 20),
        &[("gpt-5", 300, 150, 75)],
    );
    write_transcript(
        workspace,
        "task-c",
        date_ts(2026, 3, 21),
        &[("claude-sonnet-4-20250514", 500, 200, 100)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // 3 tasks, 900 sats total → avg 300/task
    assert!((report.cost_per_task_sats - 300.0).abs() < 0.001);

    // Find the day metrics for March 20
    let march_20 = report
        .daily_trend
        .iter()
        .find(|d| d.date == "2026-03-20")
        .unwrap();
    assert_eq!(march_20.tasks, 2);
    assert_eq!(march_20.total_sats, 400); // 100 + 300
    assert!((march_20.avg_cost_per_task - 200.0).abs() < 0.001);

    // Find the day metrics for March 21
    let march_21 = report
        .daily_trend
        .iter()
        .find(|d| d.date == "2026-03-21")
        .unwrap();
    assert_eq!(march_21.tasks, 1);
    assert_eq!(march_21.total_sats, 500);
}

#[test]
fn test_model_breakdown() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-gpt1",
        date_ts(2026, 3, 20),
        &[("gpt-5", 200, 100, 50)],
    );
    write_transcript(
        workspace,
        "task-gpt2",
        date_ts(2026, 3, 20),
        &[("gpt-5", 400, 200, 100)],
    );
    write_transcript(
        workspace,
        "task-claude",
        date_ts(2026, 3, 21),
        &[("claude-sonnet-4-20250514", 600, 300, 150)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    assert_eq!(report.by_model.len(), 2);

    let gpt = &report.by_model["gpt-5"];
    assert_eq!(gpt.tasks, 2);
    assert_eq!(gpt.total_sats, 600); // 200 + 400
    assert_eq!(gpt.total_tokens, 450); // (100+50) + (200+100)
    assert!((gpt.avg_cost_per_task - 300.0).abs() < 0.001);
    assert!((gpt.avg_tokens_per_task - 225.0).abs() < 0.001);

    let claude = &report.by_model["claude-sonnet-4-20250514"];
    assert_eq!(claude.tasks, 1);
    assert_eq!(claude.total_sats, 600);
}

#[test]
fn test_period_comparison_week_over_week() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    let now = test_now(); // 2026-03-22

    // Current week (Mar 16-22): 2 tasks, 500 sats each
    write_transcript(
        workspace,
        "task-cur1",
        date_ts(2026, 3, 17),
        &[("gpt-5", 500, 200, 100)],
    );
    write_transcript(
        workspace,
        "task-cur2",
        date_ts(2026, 3, 19),
        &[("gpt-5", 500, 200, 100)],
    );

    // Previous week (Mar 9-15): 2 tasks, 1000 sats each (more expensive)
    write_transcript(
        workspace,
        "task-prev1",
        date_ts(2026, 3, 10),
        &[("gpt-5", 1000, 200, 100)],
    );
    write_transcript(
        workspace,
        "task-prev2",
        date_ts(2026, 3, 12),
        &[("gpt-5", 1000, 200, 100)],
    );

    let report = compute_efficiency_at(workspace, now);

    assert_eq!(report.period_comparison.current.tasks, 2);
    assert_eq!(report.period_comparison.current.total_sats, 1000);
    assert!((report.period_comparison.current.avg_cost_per_task - 500.0).abs() < 0.001);

    assert_eq!(report.period_comparison.previous.tasks, 2);
    assert_eq!(report.period_comparison.previous.total_sats, 2000);
    assert!((report.period_comparison.previous.avg_cost_per_task - 1000.0).abs() < 0.001);

    // Cost decreased by 50%
    assert!((report.period_comparison.cost_change_pct - (-50.0)).abs() < 0.1);

    // Efficiency improved (more tokens per sat)
    // Current: 600 tokens / 1000 sats = 0.6 tokens/sat
    // Previous: 600 tokens / 2000 sats = 0.3 tokens/sat
    // Change: (0.6 - 0.3) / 0.3 = 100%
    assert!((report.period_comparison.efficiency_change_pct - 100.0).abs() < 0.1);
}

#[test]
fn test_cost_per_token_calculation() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // 1 task: 1000 sats, 500 tokens total
    write_transcript(
        workspace,
        "task-tok",
        date_ts(2026, 3, 22),
        &[("gpt-5", 1000, 300, 200)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // 1000 sats / 500 tokens = 2.0 sats/token
    assert!((report.cost_per_token_sats - 2.0).abs() < 0.001);
}

#[test]
fn test_tokens_per_iteration_calculation() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // 1 task with 3 iterations: total tokens = 3 * (100+50) = 450
    write_transcript(
        workspace,
        "task-iter",
        date_ts(2026, 3, 22),
        &[
            ("gpt-5", 100, 100, 50),
            ("gpt-5", 100, 100, 50),
            ("gpt-5", 100, 100, 50),
        ],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // 450 tokens / 3 iterations = 150
    assert!((report.tokens_per_iteration - 150.0).abs() < 0.001);
}

#[test]
fn test_malformed_transcript_skipped() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Create a good task
    write_transcript(
        workspace,
        "task-good",
        date_ts(2026, 3, 22),
        &[("gpt-5", 200, 100, 50)],
    );

    // Create a malformed transcript (invalid JSON)
    let bad_dir = workspace.join("tasks").join("task-bad");
    fs::create_dir_all(&bad_dir).unwrap();
    let bad_path = bad_dir.join("session.jsonl");
    fs::write(&bad_path, "this is not json\n{also bad}\n").unwrap();

    let report = compute_efficiency_at(workspace, test_now());

    // Should only count the good task
    assert!((report.cost_per_task_sats - 200.0).abs() < 0.001);
}

#[test]
fn test_date_range_filtering() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Task from 60 days ago — outside 30-day trend window
    write_transcript(
        workspace,
        "task-old",
        date_ts(2026, 1, 20),
        &[("gpt-5", 9999, 5000, 5000)],
    );

    // Task from today — inside 30-day trend window
    write_transcript(
        workspace,
        "task-recent",
        date_ts(2026, 3, 22),
        &[("gpt-5", 100, 50, 25)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // Both tasks contribute to global averages
    assert_eq!(report.by_model["gpt-5"].tasks, 2);

    // Only today's task appears in daily trend
    let today_metrics = report
        .daily_trend
        .iter()
        .find(|d| d.date == "2026-03-22")
        .unwrap();
    assert_eq!(today_metrics.tasks, 1);
    assert_eq!(today_metrics.total_sats, 100);

    // Old task shouldn't appear in any daily trend entry (all in Feb or before)
    let jan_20 = report.daily_trend.iter().find(|d| d.date == "2026-01-20");
    assert!(jan_20.is_none()); // Not in the 30-day window
}

#[test]
fn test_day_metrics_sorted_chronologically() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    fs::create_dir_all(workspace.join("tasks")).unwrap();

    let report = compute_efficiency_at(workspace, test_now());

    // 30 days of entries
    assert_eq!(report.daily_trend.len(), 30);

    // First entry should be 29 days ago, last should be today
    assert_eq!(report.daily_trend.first().unwrap().date, "2026-02-21");
    assert_eq!(report.daily_trend.last().unwrap().date, "2026-03-22");

    // Verify chronological order
    for i in 1..report.daily_trend.len() {
        assert!(report.daily_trend[i].date > report.daily_trend[i - 1].date);
    }
}

#[test]
fn test_no_think_responses_zeros() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Create a task with only session_start/end (no think_response)
    let task_dir = workspace.join("tasks").join("task-empty");
    fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("session.jsonl");
    let mut f = fs::File::create(&path).unwrap();

    let start_event = serde_json::json!({
        "ts": date_ts(2026, 3, 22),
        "type": "session_start",
        "id": "a1b2c3d4",
        "task": "task-empty",
    });
    writeln!(f, "{}", serde_json::to_string(&start_event).unwrap()).unwrap();

    let end_event = serde_json::json!({
        "ts": date_ts(2026, 3, 22) + 10.0,
        "type": "session_end",
        "id": "z9y8x7w6",
        "iterations": 0,
    });
    writeln!(f, "{}", serde_json::to_string(&end_event).unwrap()).unwrap();

    let report = compute_efficiency_at(workspace, test_now());

    // Task with no think_responses should be excluded
    assert_eq!(report.cost_per_task_sats, 0.0);
    assert!(report.by_model.is_empty());
}

#[test]
fn test_efficiency_report_serialization() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-ser",
        date_ts(2026, 3, 22),
        &[("gpt-5", 300, 150, 75)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // Should serialize to JSON without error
    let json = serde_json::to_value(&report).unwrap();

    assert!(json["cost_per_task_sats"].is_f64());
    assert!(json["cost_per_token_sats"].is_f64());
    assert!(json["tokens_per_iteration"].is_f64());
    assert!(json["daily_trend"].is_array());
    assert!(json["by_model"].is_object());
    assert!(json["period_comparison"].is_object());
    assert!(json["period_comparison"]["current"]["label"].is_string());
    assert!(json["period_comparison"]["previous"]["label"].is_string());
    assert!(json["period_comparison"]["cost_change_pct"].is_f64());
    assert!(json["period_comparison"]["efficiency_change_pct"].is_f64());
}

#[test]
fn test_tool_sats_included_in_cost() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Create a task with think_response + tool_result that has sats_paid
    let task_dir = workspace.join("tasks").join("task-tool");
    fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("session.jsonl");
    let mut f = fs::File::create(&path).unwrap();

    let ts = date_ts(2026, 3, 22);

    let start = serde_json::json!({
        "ts": ts,
        "type": "session_start",
        "id": "s001",
        "task": "task-tool",
    });
    writeln!(f, "{}", serde_json::to_string(&start).unwrap()).unwrap();

    let think = serde_json::json!({
        "ts": ts + 10.0,
        "type": "think_response",
        "id": "t001",
        "role": "assistant",
        "content": "Let me use a tool",
        "model": "gpt-5",
        "sats_paid": 200,
        "sats_effective": 150,
        "sats_refunded": 50,
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "finish_reason": "tool_calls",
        "duration_ms": 1000,
    });
    writeln!(f, "{}", serde_json::to_string(&think).unwrap()).unwrap();

    let tool_result = serde_json::json!({
        "ts": ts + 20.0,
        "type": "tool_result",
        "id": "r001",
        "role": "tool",
        "call_id": "call-1",
        "name": "x402_call",
        "content": "result",
        "success": true,
        "sats_paid": 500,
    });
    writeln!(f, "{}", serde_json::to_string(&tool_result).unwrap()).unwrap();

    let end = serde_json::json!({
        "ts": ts + 30.0,
        "type": "session_end",
        "id": "e001",
        "iterations": 1,
    });
    writeln!(f, "{}", serde_json::to_string(&end).unwrap()).unwrap();

    let report = compute_efficiency_at(workspace, test_now());

    // Total sats = sats_effective (150) + tool sats_paid (500) = 650
    assert!((report.cost_per_task_sats - 650.0).abs() < 0.001);
}

#[test]
fn test_multiple_models_per_task_uses_first() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Task that starts with gpt-5 then switches to claude
    let task_dir = workspace.join("tasks").join("task-mixed");
    fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("session.jsonl");
    let mut f = fs::File::create(&path).unwrap();

    let ts = date_ts(2026, 3, 22);

    let start = serde_json::json!({
        "ts": ts, "type": "session_start", "id": "s1", "task": "task-mixed"
    });
    writeln!(f, "{}", serde_json::to_string(&start).unwrap()).unwrap();

    // First think with gpt-5
    let think1 = serde_json::json!({
        "ts": ts + 10.0, "type": "think_response", "id": "t1",
        "role": "assistant", "content": "resp1", "model": "gpt-5",
        "sats_paid": 100, "sats_effective": 100, "sats_refunded": 0,
        "prompt_tokens": 50, "completion_tokens": 25,
        "finish_reason": "stop", "duration_ms": 500,
    });
    writeln!(f, "{}", serde_json::to_string(&think1).unwrap()).unwrap();

    // Second think with claude
    let think2 = serde_json::json!({
        "ts": ts + 20.0, "type": "think_response", "id": "t2",
        "role": "assistant", "content": "resp2", "model": "claude-sonnet-4-20250514",
        "sats_paid": 200, "sats_effective": 200, "sats_refunded": 0,
        "prompt_tokens": 80, "completion_tokens": 40,
        "finish_reason": "stop", "duration_ms": 700,
    });
    writeln!(f, "{}", serde_json::to_string(&think2).unwrap()).unwrap();

    let end = serde_json::json!({
        "ts": ts + 30.0, "type": "session_end", "id": "e1", "iterations": 2,
    });
    writeln!(f, "{}", serde_json::to_string(&end).unwrap()).unwrap();

    let report = compute_efficiency_at(workspace, test_now());

    // Primary model should be gpt-5 (first think_response)
    assert!(report.by_model.contains_key("gpt-5"));
    assert_eq!(report.by_model["gpt-5"].tasks, 1);
    // All 300 sats attributed to primary model
    assert_eq!(report.by_model["gpt-5"].total_sats, 300);
}

#[test]
fn test_period_comparison_no_previous_data() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Only current week data
    write_transcript(
        workspace,
        "task-now",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    assert_eq!(report.period_comparison.current.tasks, 1);
    assert_eq!(report.period_comparison.previous.tasks, 0);

    // With no previous data, change percentages should be 0
    assert_eq!(report.period_comparison.cost_change_pct, 0.0);
    assert_eq!(report.period_comparison.efficiency_change_pct, 0.0);
}

#[test]
fn test_zero_days_have_zero_metrics() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Single task on March 22
    write_transcript(
        workspace,
        "task-one",
        date_ts(2026, 3, 22),
        &[("gpt-5", 100, 50, 25)],
    );

    let report = compute_efficiency_at(workspace, test_now());

    // March 21 should be zero
    let march_21 = report
        .daily_trend
        .iter()
        .find(|d| d.date == "2026-03-21")
        .unwrap();
    assert_eq!(march_21.tasks, 0);
    assert_eq!(march_21.total_sats, 0);
    assert_eq!(march_21.total_tokens, 0);
    assert_eq!(march_21.avg_cost_per_task, 0.0);
}

// ── Cost Replay Tests (#62) ──────────────────────────────────────────

#[test]
fn test_cost_replay_from_transcript() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Create a task with gpt-5: 400 sats for 300 tokens (150 each iter)
    write_transcript(
        workspace,
        "task-replay",
        date_ts(2026, 3, 22),
        &[("gpt-5", 200, 100, 50), ("gpt-5", 200, 100, 50)],
    );

    let mut pricing = HashMap::new();
    pricing.insert("gpt-5".to_string(), 150.0); // 150 sats / 1K tokens
    pricing.insert("gpt-5-mini".to_string(), 15.0); // 15 sats / 1K tokens (10x cheaper)

    let comparison = cost_replay(workspace, "task-replay", "gpt-5-mini", &pricing).unwrap();

    assert_eq!(comparison.task_id, "task-replay");
    assert_eq!(comparison.original_model, "gpt-5");
    assert_eq!(comparison.original_cost, 400);
    assert_eq!(comparison.original_tokens, 300);
    assert_eq!(comparison.alternative_model, "gpt-5-mini");
    // 300 tokens / 1000 * 15 = 4.5 → 4 sats
    assert_eq!(comparison.alternative_cost, 4);
    assert_eq!(comparison.savings_sats, 396);
    assert!(comparison.savings_pct > 98.0);
}

#[test]
fn test_cost_replay_missing_task() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    fs::create_dir_all(workspace.join("tasks")).unwrap();

    let pricing = HashMap::new();
    let result = cost_replay(workspace, "nonexistent", "gpt-5-mini", &pricing);
    assert!(result.is_none());
}

#[test]
fn test_cost_replay_from_events() {
    // Build fixture transcript events
    let events = vec![
        make_think_event(10.0, "gpt-5", 500, 200, 100, 1500),
        make_think_event(20.0, "gpt-5", 300, 150, 75, 1200),
    ];

    let mut pricing = HashMap::new();
    pricing.insert("claude-haiku-4-5-20251001".to_string(), 30.0);

    let comparison =
        cost_replay_from_events("test-task", &events, "claude-haiku-4-5-20251001", &pricing)
            .unwrap();

    assert_eq!(comparison.original_model, "gpt-5");
    assert_eq!(comparison.original_cost, 800); // 500 + 300
    assert_eq!(comparison.original_tokens, 525); // (200+100) + (150+75)
    assert_eq!(comparison.alternative_model, "claude-haiku-4-5-20251001");
    // 525 / 1000 * 30 = 15.75 → 15
    assert_eq!(comparison.alternative_cost, 15);
    assert!(comparison.savings_sats > 0);
}

#[test]
fn test_cost_replay_no_think_events() {
    let events: Vec<TranscriptEvent> = vec![];
    let pricing = HashMap::new();
    let result = cost_replay_from_events("empty-task", &events, "gpt-5", &pricing);
    assert!(result.is_none());
}

#[test]
fn test_cost_replay_unknown_alt_model_uses_fallback() {
    let events = vec![make_think_event(10.0, "gpt-5", 1000, 400, 100, 1500)];

    // Pricing table does NOT contain "new-model"
    let pricing = HashMap::new();

    let comparison = cost_replay_from_events("test-task", &events, "new-model", &pricing).unwrap();

    // Should derive rate from original data: 1000 sats / 500 tokens * 1000 = 2000 sats/1K tokens
    // Then apply: 500 / 1000 * 2000 = 1000 (same as original)
    assert_eq!(comparison.alternative_cost, comparison.original_cost);
    assert_eq!(comparison.savings_sats, 0);
}

// ── ROI Report Tests (#51) ───────────────────────────────────────────

#[test]
fn test_roi_empty_workspace() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    fs::create_dir_all(workspace.join("tasks")).unwrap();

    let report = compute_roi(workspace);

    assert_eq!(report.total_spent_sats, 0);
    assert_eq!(report.tasks_completed, 0);
    assert_eq!(report.tasks_failed, 0);
    assert_eq!(report.avg_cost_per_task, 0.0);
    assert_eq!(report.efficiency_score, 0.0);
}

#[test]
fn test_roi_from_summaries_all_completed() {
    // (sats, tokens, completed, quality_score)
    let summaries = vec![
        (500_u64, 300_u64, true, Some(0.85)),
        (300, 200, true, Some(0.90)),
        (700, 400, true, Some(0.75)),
    ];

    let report = compute_roi_from_summaries(&summaries);

    assert_eq!(report.total_spent_sats, 1500);
    assert_eq!(report.tasks_completed, 3);
    assert_eq!(report.tasks_failed, 0);
    assert!((report.avg_cost_per_task - 500.0).abs() < 0.001);
    assert_eq!(report.total_tokens, 900);
    // efficiency: 3 / 1500 * 1_000_000 = 2000
    assert!((report.efficiency_score - 2000.0).abs() < 0.001);
    // avg quality: (0.85 + 0.90 + 0.75) / 3 = 0.8333...
    assert!((report.avg_quality_score - 0.8333).abs() < 0.01);
}

#[test]
fn test_roi_from_summaries_mixed_outcomes() {
    let summaries = vec![
        (1000_u64, 500_u64, true, None),
        (800, 400, false, None),
        (600, 300, true, None),
        (1200, 600, false, None),
    ];

    let report = compute_roi_from_summaries(&summaries);

    assert_eq!(report.total_spent_sats, 3600);
    assert_eq!(report.tasks_completed, 2);
    assert_eq!(report.tasks_failed, 2);
    // avg cost per task: 3600 / 4 = 900
    assert!((report.avg_cost_per_task - 900.0).abs() < 0.001);
    // cost per completed: (1000 + 600) / 2 = 800
    assert!((report.cost_per_completed_task - 800.0).abs() < 0.001);
    // cost per failed: (800 + 1200) / 2 = 1000
    assert!((report.cost_per_failed_task - 1000.0).abs() < 0.001);
    // No quality scores available
    assert_eq!(report.avg_quality_score, 0.0);
}

#[test]
fn test_roi_from_transcripts() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Completed task (has session_end without error)
    write_transcript(
        workspace,
        "task-ok",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    // Failed task (session_end with error)
    write_transcript_with_error(
        workspace,
        "task-fail",
        date_ts(2026, 3, 22),
        "Budget exceeded",
    );

    let report = compute_roi(workspace);

    assert_eq!(report.tasks_completed, 1);
    assert_eq!(report.tasks_failed, 1);
    assert!(report.total_spent_sats > 0);
}

#[test]
fn test_roi_report_serialization() {
    let summaries = vec![(500_u64, 300_u64, true, Some(0.9))];
    let report = compute_roi_from_summaries(&summaries);
    let json = serde_json::to_value(&report).unwrap();

    assert!(json["total_spent_sats"].is_u64());
    assert!(json["tasks_completed"].is_u64());
    assert!(json["tasks_failed"].is_u64());
    assert!(json["avg_cost_per_task"].is_f64());
    assert!(json["efficiency_score"].is_f64());
    assert!(json["avg_quality_score"].is_f64());
}

// ── Provider Benchmark Tests (#64) ───────────────────────────────────

#[test]
fn test_benchmark_record_and_load() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    let entry = BenchmarkEntry {
        timestamp: "2026-03-22T12:00:00Z".to_string(),
        provider: "openai-agent".to_string(),
        model: "gpt-5".to_string(),
        latency_ms: 1500,
        cost_sats: 500,
        tokens: 300,
        success: true,
    };

    record_benchmark(workspace, &entry);
    record_benchmark(workspace, &entry);

    let loaded = load_benchmarks(workspace);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].model, "gpt-5");
    assert_eq!(loaded[0].latency_ms, 1500);
}

#[test]
fn test_benchmark_aggregate() {
    let entries = vec![
        BenchmarkEntry {
            timestamp: "2026-03-22T12:00:00Z".to_string(),
            provider: "openai-agent".to_string(),
            model: "gpt-5".to_string(),
            latency_ms: 1000,
            cost_sats: 400,
            tokens: 200,
            success: true,
        },
        BenchmarkEntry {
            timestamp: "2026-03-22T12:01:00Z".to_string(),
            provider: "openai-agent".to_string(),
            model: "gpt-5".to_string(),
            latency_ms: 2000,
            cost_sats: 600,
            tokens: 300,
            success: true,
        },
        BenchmarkEntry {
            timestamp: "2026-03-22T12:02:00Z".to_string(),
            provider: "openai-agent".to_string(),
            model: "gpt-5".to_string(),
            latency_ms: 1500,
            cost_sats: 500,
            tokens: 250,
            success: false,
        },
        BenchmarkEntry {
            timestamp: "2026-03-22T12:03:00Z".to_string(),
            provider: "claude-chat".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            latency_ms: 3000,
            cost_sats: 800,
            tokens: 400,
            success: true,
        },
    ];

    let benchmarks = aggregate_benchmarks(&entries);

    assert_eq!(benchmarks.len(), 2);

    // Find the claude benchmark
    let claude = benchmarks
        .iter()
        .find(|b| b.provider == "claude-chat")
        .unwrap();
    assert_eq!(claude.sample_count, 1);
    assert!((claude.avg_latency_ms - 3000.0).abs() < 0.001);
    assert!((claude.avg_cost_sats - 800.0).abs() < 0.001);
    assert!((claude.success_rate - 1.0).abs() < 0.001);

    // Find the openai benchmark
    let openai = benchmarks
        .iter()
        .find(|b| b.provider == "openai-agent")
        .unwrap();
    assert_eq!(openai.sample_count, 3);
    // avg latency: (1000 + 2000 + 1500) / 3 = 1500
    assert!((openai.avg_latency_ms - 1500.0).abs() < 0.001);
    // avg cost: (400 + 600 + 500) / 3 = 500
    assert!((openai.avg_cost_sats - 500.0).abs() < 0.001);
    // success rate: 2/3
    assert!((openai.success_rate - 2.0 / 3.0).abs() < 0.001);
    assert_eq!(openai.total_tokens, 750);
}

#[test]
fn test_benchmark_empty_aggregation() {
    let benchmarks = aggregate_benchmarks(&[]);
    assert!(benchmarks.is_empty());
}

#[test]
fn test_infer_provider() {
    assert_eq!(infer_provider("gpt-5"), "openai-agent");
    assert_eq!(infer_provider("gpt-5-mini"), "openai-agent");
    assert_eq!(infer_provider("gpt-4.1"), "openai-agent");
    assert_eq!(infer_provider("o4-mini"), "openai-agent");
    assert_eq!(infer_provider("o1-preview"), "openai-agent");
    assert_eq!(infer_provider("o3-mini"), "openai-agent");
    assert_eq!(infer_provider("claude-sonnet-4-20250514"), "claude-chat");
    assert_eq!(infer_provider("claude-haiku-4-5-20251001"), "claude-chat");
    assert_eq!(infer_provider("llama-3"), "unknown");
}

#[test]
fn test_benchmark_from_file_path() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test-benchmarks.jsonl");

    // Write entries manually
    let mut f = fs::File::create(&path).unwrap();
    let entry = serde_json::json!({
        "timestamp": "2026-03-22T12:00:00Z",
        "provider": "openai-agent",
        "model": "gpt-5",
        "latency_ms": 1500,
        "cost_sats": 500,
        "tokens": 300,
        "success": true,
    });
    writeln!(f, "{}", serde_json::to_string(&entry).unwrap()).unwrap();

    let loaded = load_benchmarks_from_path(&path);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].provider, "openai-agent");
}

#[test]
fn test_benchmark_malformed_lines_skipped() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test-benchmarks.jsonl");

    let mut f = fs::File::create(&path).unwrap();
    writeln!(f, "not json").unwrap();
    let entry = serde_json::json!({
        "timestamp": "2026-03-22T12:00:00Z",
        "provider": "openai-agent",
        "model": "gpt-5",
        "latency_ms": 1500,
        "cost_sats": 500,
        "tokens": 300,
        "success": true,
    });
    writeln!(f, "{}", serde_json::to_string(&entry).unwrap()).unwrap();
    writeln!(f, "{{broken json").unwrap();

    let loaded = load_benchmarks_from_path(&path);
    assert_eq!(loaded.len(), 1); // Only the valid entry
}

#[test]
fn test_benchmark_backfill_from_transcripts() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-bf1",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100), ("gpt-5", 300, 150, 75)],
    );
    write_transcript(
        workspace,
        "task-bf2",
        date_ts(2026, 3, 22),
        &[("claude-sonnet-4-20250514", 800, 400, 200)],
    );

    let entries = dolphin_milk::analytics::backfill_benchmarks_from_transcripts(workspace);

    // 3 think_response events total
    assert_eq!(entries.len(), 3);

    let gpt_entries: Vec<_> = entries.iter().filter(|e| e.model == "gpt-5").collect();
    assert_eq!(gpt_entries.len(), 2);
    assert_eq!(gpt_entries[0].provider, "openai-agent");

    let claude_entries: Vec<_> = entries
        .iter()
        .filter(|e| e.model == "claude-sonnet-4-20250514")
        .collect();
    assert_eq!(claude_entries.len(), 1);
    assert_eq!(claude_entries[0].provider, "claude-chat");
}

// ── Combined Cost Analysis ───────────────────────────────────────────

#[test]
fn test_combined_cost_analysis() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-ca1",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    let analysis = compute_cost_analysis(workspace);

    // Efficiency report populated
    assert!((analysis.efficiency.cost_per_task_sats - 500.0).abs() < 0.001);
    // ROI report populated
    assert_eq!(analysis.roi.tasks_completed, 1);
    assert_eq!(analysis.roi.total_spent_sats, 500);
    // Benchmarks auto-backfilled
    assert!(!analysis.benchmarks.is_empty());
}

// ── Server Route Tests (Axum oneshot) ────────────────────────────────

#[tokio::test]
async fn test_analytics_cost_comparison_route() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    // Create a transcript to compare BEFORE building router (build_router scans workspace)
    write_transcript(
        &workspace,
        "task-route-cmp",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    std::mem::forget(dir);
    let router = dolphin_milk::server::build_router(
        {
            let mut c = dolphin_milk::config::DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/analytics/cost-comparison?task_id=task-route-cmp&alt_model=gpt-5-mini")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["original_model"], "gpt-5");
    assert_eq!(json["alternative_model"], "gpt-5-mini");
    assert!(json["savings_sats"].is_i64());
    assert!(json["savings_pct"].is_f64());
}

#[tokio::test]
async fn test_analytics_cost_comparison_missing_task() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    fs::create_dir_all(workspace.join("tasks")).unwrap();
    std::mem::forget(dir);

    let router = dolphin_milk::server::build_router(
        {
            let mut c = dolphin_milk::config::DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/analytics/cost-comparison?task_id=nonexistent&alt_model=gpt-5-mini")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_analytics_roi_route() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    write_transcript(
        &workspace,
        "task-roi-1",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    std::mem::forget(dir);
    let router = dolphin_milk::server::build_router(
        {
            let mut c = dolphin_milk::config::DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/analytics/roi")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json["total_spent_sats"].is_u64());
    assert!(json["tasks_completed"].is_u64());
    assert!(json["efficiency_score"].is_f64());
}

#[tokio::test]
async fn test_analytics_benchmarks_route() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    write_transcript(
        &workspace,
        "task-bench-1",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    std::mem::forget(dir);
    let router = dolphin_milk::server::build_router(
        {
            let mut c = dolphin_milk::config::DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        workspace,
    )
    .await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/analytics/benchmarks")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json.is_array());
}

// ── Cost Analysis Tool Test ──────────────────────────────────────────

#[test]
fn test_cost_analysis_tool_registration() {
    let dir = tempdir().unwrap();
    let tools = dolphin_milk::tools::analytics_tools::all_analytics_tools(dir.path().to_path_buf());
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "cost_analysis");
    assert_eq!(tools[0].category, "analytics");
}

#[tokio::test]
async fn test_cost_analysis_tool_summary_action() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-tool-sum",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    let tools = dolphin_milk::tools::analytics_tools::all_analytics_tools(workspace.to_path_buf());
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({"action": "summary"})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(json["efficiency"].is_object());
    assert!(json["roi"].is_object());
    assert!(json["benchmarks"].is_array());
}

#[tokio::test]
async fn test_cost_analysis_tool_roi_action() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-tool-roi",
        date_ts(2026, 3, 22),
        &[("gpt-5", 300, 150, 75)],
    );

    let tools = dolphin_milk::tools::analytics_tools::all_analytics_tools(workspace.to_path_buf());
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({"action": "roi"})).await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["tasks_completed"], 1);
    assert_eq!(json["total_spent_sats"], 300);
}

#[tokio::test]
async fn test_cost_analysis_tool_compare_action() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    write_transcript(
        workspace,
        "task-tool-cmp",
        date_ts(2026, 3, 22),
        &[("gpt-5", 500, 200, 100)],
    );

    let tools = dolphin_milk::tools::analytics_tools::all_analytics_tools(workspace.to_path_buf());
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "action": "compare",
        "task_id": "task-tool-cmp",
        "alt_model": "gpt-5-mini"
    }))
    .await;

    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(json["original_model"], "gpt-5");
    assert_eq!(json["alternative_model"], "gpt-5-mini");
    assert!(json["savings_sats"].is_i64());
}

#[tokio::test]
async fn test_cost_analysis_tool_compare_missing_params() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    fs::create_dir_all(workspace.join("tasks")).unwrap();

    let tools = dolphin_milk::tools::analytics_tools::all_analytics_tools(workspace.to_path_buf());
    let tool = &tools[0];

    // Missing task_id
    let result =
        (tool.execute)(serde_json::json!({"action": "compare", "alt_model": "gpt-5-mini"})).await;
    assert!(result.contains("Error"));

    // Missing alt_model
    let result = (tool.execute)(serde_json::json!({"action": "compare", "task_id": "test"})).await;
    assert!(result.contains("Error"));
}

#[tokio::test]
async fn test_cost_analysis_tool_unknown_action() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    fs::create_dir_all(workspace.join("tasks")).unwrap();

    let tools = dolphin_milk::tools::analytics_tools::all_analytics_tools(workspace.to_path_buf());
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({"action": "unknown"})).await;
    assert!(result.contains("Error"));
}

// ── Scenarios JSON loading ───────────────────────────────────────────

#[test]
fn test_scenarios_json_loadable() {
    // Verify scenarios.json can be loaded and parsed as sample data
    let data = include_str!("../integration/scenarios.json");
    let json: serde_json::Value = serde_json::from_str(data).unwrap();

    let scenarios = json["scenarios"].as_array().unwrap();
    assert!(!scenarios.is_empty());

    // Verify we can extract cost data from scenario baselines
    let costs: Vec<u64> = scenarios
        .iter()
        .filter_map(|s| s["baseline_sats"].as_u64())
        .collect();
    assert!(!costs.is_empty());

    // The total baseline cost should be a meaningful number
    let total: u64 = costs.iter().sum();
    assert!(total > 0);
}

#[test]
fn test_scenarios_p_json_valid() {
    let data = include_str!("../integration/scenarios_P.json");
    let scenarios: Vec<serde_json::Value> = serde_json::from_str(data).unwrap();

    assert_eq!(scenarios.len(), 2);
    assert_eq!(scenarios[0]["id"], 26);
    assert_eq!(scenarios[0]["name"], "cost_aggregation");
    assert_eq!(scenarios[1]["id"], 27);
    assert_eq!(scenarios[1]["name"], "cost_comparison");
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Create a think_response TranscriptEvent for testing.
fn make_think_event(
    ts: f64,
    model: &str,
    sats_effective: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    duration_ms: u64,
) -> TranscriptEvent {
    let mut data = HashMap::new();
    data.insert("role".into(), serde_json::Value::String("assistant".into()));
    data.insert(
        "content".into(),
        serde_json::Value::String("Response".into()),
    );
    data.insert("model".into(), serde_json::Value::String(model.into()));
    data.insert("sats_effective".into(), serde_json::json!(sats_effective));
    data.insert("prompt_tokens".into(), serde_json::json!(prompt_tokens));
    data.insert(
        "completion_tokens".into(),
        serde_json::json!(completion_tokens),
    );
    data.insert("duration_ms".into(), serde_json::json!(duration_ms));
    data.insert(
        "finish_reason".into(),
        serde_json::Value::String("stop".into()),
    );

    TranscriptEvent {
        ts,
        event_type: "think_response".to_string(),
        id: format!("te{}", ts as u64),
        data,
    }
}

/// Write a transcript that ends with an error.
fn write_transcript_with_error(
    workspace: &std::path::Path,
    task_id: &str,
    start_ts: f64,
    error_msg: &str,
) {
    let task_dir = workspace.join("tasks").join(task_id);
    fs::create_dir_all(&task_dir).unwrap();
    let path = task_dir.join("session.jsonl");
    let mut f = fs::File::create(&path).unwrap();

    let start_event = serde_json::json!({
        "ts": start_ts,
        "type": "session_start",
        "id": "s001",
        "task": task_id,
    });
    writeln!(f, "{}", serde_json::to_string(&start_event).unwrap()).unwrap();

    let think_event = serde_json::json!({
        "ts": start_ts + 10.0,
        "type": "think_response",
        "id": "t001",
        "role": "assistant",
        "content": "Starting...",
        "model": "gpt-5",
        "sats_paid": 200,
        "sats_effective": 150,
        "sats_refunded": 50,
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "finish_reason": "stop",
        "duration_ms": 1000,
    });
    writeln!(f, "{}", serde_json::to_string(&think_event).unwrap()).unwrap();

    let end_event = serde_json::json!({
        "ts": start_ts + 20.0,
        "type": "session_end",
        "id": "e001",
        "iterations": 1,
        "error": error_msg,
    });
    writeln!(f, "{}", serde_json::to_string(&end_event).unwrap()).unwrap();
}
