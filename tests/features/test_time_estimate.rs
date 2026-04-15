//! Tests for time-saved estimation (E.5) — human-equivalent time saved per task.
//!
//! Covers:
//! - Tag parsing: `time-saved:Xh`, `time-saved:Xm`, decimal hours
//! - Default heuristic estimation from iterations + tool calls
//! - Cap at 480 minutes (8 hours)
//! - Tag overrides default estimate
//! - Aggregate analytics (total, this week, this month)
//! - Serialization of analytics types
//! - Edge cases (zero iterations, missing tags, invalid tags)

use dolphin_milk::server::types::TaskSummary;
use dolphin_milk::time_estimate::*;

// ===========================================================================
// Tag parsing
// ===========================================================================

#[test]
fn test_parse_time_saved_tag_hours() {
    let tags = vec!["time-saved:2h".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), Some(120));
}

#[test]
fn test_parse_time_saved_tag_minutes() {
    let tags = vec!["time-saved:30m".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), Some(30));
}

#[test]
fn test_parse_time_saved_tag_decimal_hours() {
    let tags = vec!["time-saved:1.5h".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), Some(90));
}

#[test]
fn test_parse_time_saved_tag_missing() {
    let tags = vec!["billing".to_string(), "roi".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), None);
}

#[test]
fn test_parse_time_saved_tag_invalid() {
    let tags = vec!["time-saved:abc".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), None);
}

#[test]
fn test_parse_time_saved_tag_empty_tags() {
    let tags: Vec<String> = vec![];
    assert_eq!(parse_time_saved_tag(&tags), None);
}

#[test]
fn test_parse_time_saved_tag_among_other_tags() {
    let tags = vec![
        "billing".to_string(),
        "time-saved:3h".to_string(),
        "roi".to_string(),
    ];
    assert_eq!(parse_time_saved_tag(&tags), Some(180));
}

#[test]
fn test_parse_time_saved_tag_capped_at_480() {
    // 100 hours = 6000 minutes, but capped at 480
    let tags = vec!["time-saved:100h".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), Some(480));
}

#[test]
fn test_parse_time_saved_tag_zero_hours() {
    let tags = vec!["time-saved:0h".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), Some(0));
}

#[test]
fn test_parse_time_saved_tag_zero_minutes() {
    let tags = vec!["time-saved:0m".to_string()];
    assert_eq!(parse_time_saved_tag(&tags), Some(0));
}

// ===========================================================================
// Default estimation
// ===========================================================================

#[test]
fn test_default_estimate_single_iteration() {
    // 1 iteration * 5 min + 0 tool calls * 2 min = 5
    assert_eq!(default_estimate(1, 0), 5);
}

#[test]
fn test_default_estimate_many_iterations() {
    // 10 iterations * 5 = 50
    assert_eq!(default_estimate(10, 0), 50);
}

#[test]
fn test_default_estimate_with_tool_calls() {
    // 3 iterations * 5 + 5 tool calls * 2 = 15 + 10 = 25
    assert_eq!(default_estimate(3, 5), 25);
}

#[test]
fn test_default_estimate_capped_at_480() {
    // 200 iterations * 5 = 1000, capped at 480
    assert_eq!(default_estimate(200, 0), 480);
    // 100 iterations * 5 + 100 tool calls * 2 = 500 + 200 = 700, capped at 480
    assert_eq!(default_estimate(100, 100), 480);
}

#[test]
fn test_zero_iterations_zero_minutes() {
    assert_eq!(default_estimate(0, 0), 0);
}

#[test]
fn test_default_estimate_zero_iterations_with_tool_calls() {
    // 0 iterations, 3 tool calls: 0 + 6 = 6
    assert_eq!(default_estimate(0, 3), 6);
}

// ===========================================================================
// Combined estimate (tag override vs default)
// ===========================================================================

#[test]
fn test_tag_overrides_default_estimate() {
    let tags = vec!["time-saved:2h".to_string()];
    // Tag says 120 min, default would be 5*5 + 2*10 = 45
    assert_eq!(estimate_time_saved(&tags, 5, 10), 120);
}

#[test]
fn test_estimate_falls_back_to_default() {
    let tags = vec!["billing".to_string()];
    // No time-saved tag, so use default: 3*5 + 4*2 = 23
    assert_eq!(estimate_time_saved(&tags, 3, 4), 23);
}

#[test]
fn test_estimate_from_task_info() {
    // Simulate computing time_saved from TaskInfo-like data
    let tags = vec!["time-saved:45m".to_string()];
    let iterations = 10;
    let tool_calls = 5;
    let result = estimate_time_saved(&tags, iterations, tool_calls);
    assert_eq!(result, 45); // Tag override wins
}

// ===========================================================================
// Aggregate analytics
// ===========================================================================

#[test]
fn test_aggregate_total_hours() {
    let tasks = vec![
        TaskTimeSavedInput {
            task_id: "t1".into(),
            description: "task 1".into(),
            minutes_saved: 60,
            source: "estimate".into(),
            started_at_epoch_secs: 1000,
        },
        TaskTimeSavedInput {
            task_id: "t2".into(),
            description: "task 2".into(),
            minutes_saved: 120,
            source: "tag".into(),
            started_at_epoch_secs: 2000,
        },
    ];
    let analytics = aggregate_time_saved(&tasks, 100_000);
    assert!((analytics.total_hours_saved - 3.0).abs() < f64::EPSILON);
}

#[test]
fn test_aggregate_this_week() {
    let now = 1_000_000i64;
    let one_day_ago = now - 86_400;
    let two_weeks_ago = now - 15 * 86_400;

    let tasks = vec![
        TaskTimeSavedInput {
            task_id: "recent".into(),
            description: "recent task".into(),
            minutes_saved: 60,
            source: "estimate".into(),
            started_at_epoch_secs: one_day_ago,
        },
        TaskTimeSavedInput {
            task_id: "old".into(),
            description: "old task".into(),
            minutes_saved: 120,
            source: "estimate".into(),
            started_at_epoch_secs: two_weeks_ago,
        },
    ];
    let analytics = aggregate_time_saved(&tasks, now);
    // Total: 3 hours
    assert!((analytics.total_hours_saved - 3.0).abs() < f64::EPSILON);
    // This week: only "recent" (1 hour)
    assert!((analytics.this_week_hours - 1.0).abs() < f64::EPSILON);
    // This month: both (3 hours)
    assert!((analytics.this_month_hours - 3.0).abs() < f64::EPSILON);
}

#[test]
fn test_aggregate_empty_tasks() {
    let analytics = aggregate_time_saved(&[], 1_000_000);
    assert!((analytics.total_hours_saved - 0.0).abs() < f64::EPSILON);
    assert!((analytics.this_week_hours - 0.0).abs() < f64::EPSILON);
    assert!((analytics.this_month_hours - 0.0).abs() < f64::EPSILON);
    assert!(analytics.per_task.is_empty());
}

#[test]
fn test_aggregate_per_task_populated() {
    let tasks = vec![TaskTimeSavedInput {
        task_id: "t1".into(),
        description: "build feature".into(),
        minutes_saved: 30,
        source: "tag".into(),
        started_at_epoch_secs: 1000,
    }];
    let analytics = aggregate_time_saved(&tasks, 100_000);
    assert_eq!(analytics.per_task.len(), 1);
    assert_eq!(analytics.per_task[0].task_id, "t1");
    assert_eq!(analytics.per_task[0].description, "build feature");
    assert_eq!(analytics.per_task[0].minutes_saved, 30);
    assert_eq!(analytics.per_task[0].source, "tag");
}

// ===========================================================================
// Serialization
// ===========================================================================

#[test]
fn test_time_saved_serialization() {
    let analytics = TimeSavedAnalytics {
        total_hours_saved: 5.5,
        this_week_hours: 2.0,
        this_month_hours: 4.0,
        per_task: vec![TaskTimeSaved {
            task_id: "t1".into(),
            description: "test task".into(),
            minutes_saved: 60,
            source: "estimate".into(),
        }],
    };
    let json_str = serde_json::to_string(&analytics).unwrap();
    assert!(json_str.contains("\"total_hours_saved\":5.5"));
    assert!(json_str.contains("\"this_week_hours\":2.0"));
    assert!(json_str.contains("\"this_month_hours\":4.0"));
    assert!(json_str.contains("\"minutes_saved\":60"));

    // Roundtrip
    let parsed: TimeSavedAnalytics = serde_json::from_str(&json_str).unwrap();
    assert!((parsed.total_hours_saved - 5.5).abs() < f64::EPSILON);
    assert_eq!(parsed.per_task.len(), 1);
    assert_eq!(parsed.per_task[0].task_id, "t1");
}

#[test]
fn test_task_time_saved_serialization() {
    let ts = TaskTimeSaved {
        task_id: "abc".into(),
        description: "hello".into(),
        minutes_saved: 42,
        source: "tag".into(),
    };
    let json_str = serde_json::to_string(&ts).unwrap();
    assert!(json_str.contains("\"minutes_saved\":42"));
    assert!(json_str.contains("\"source\":\"tag\""));

    let parsed: TaskTimeSaved = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.minutes_saved, 42);
}

// ===========================================================================
// TaskSummary field
// ===========================================================================

#[test]
fn test_task_summary_time_saved_field() {
    let summary = TaskSummary {
        id: "t1".into(),
        task: "test".into(),
        status: "complete".into(),
        iterations: 5,
        sats_spent: 100,
        started_at: "2026-03-22T00:00:00Z".into(),
        completed_at: Some("2026-03-22T00:01:00Z".into()),
        proof_txids: Vec::new(),
        tokens: 0,
        tags: Vec::new(),
        time_saved_minutes: 25,
        origin: String::new(),
        conversation_id: None,
    };
    let json_str = serde_json::to_string(&summary).unwrap();
    assert!(json_str.contains("\"time_saved_minutes\":25"));

    let parsed: TaskSummary = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.time_saved_minutes, 25);
}

#[test]
fn test_task_summary_time_saved_default_zero() {
    // JSON without time_saved_minutes should default to 0
    let json_str = r#"{
        "id": "old",
        "task": "legacy",
        "status": "complete",
        "iterations": 1,
        "sats_spent": 100,
        "started_at": "2026-01-01T00:00:00Z",
        "tokens": 0
    }"#;
    let summary: TaskSummary = serde_json::from_str(json_str).unwrap();
    assert_eq!(summary.time_saved_minutes, 0);
}

// ===========================================================================
// Constants
// ===========================================================================

#[test]
fn test_max_minutes_per_task_constant() {
    assert_eq!(MAX_MINUTES_PER_TASK, 480);
}
