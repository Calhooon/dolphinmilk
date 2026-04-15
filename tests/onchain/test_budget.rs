//! Integration tests for the budget tracker module.

use chrono::Utc;
use dolphin_milk::budget::{BudgetLimits, BudgetTracker, SpendingEntry};
use serde_json::Value;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_budget_tracker_basic_flow() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());

    // Record LLM spending
    tracker.record(
        "llm",
        "think",
        200,
        serde_json::json!({"model": "gpt-5-mini"}),
    );
    tracker.record(
        "llm",
        "think",
        150,
        serde_json::json!({"model": "gpt-5-mini"}),
    );

    // Record proof spending
    tracker.record("proofs", "brc18", 1, serde_json::json!({"txid": "abc123"}));

    assert_eq!(tracker.task_sats(), 351);
    assert_eq!(tracker.hourly_sats(), 351);
    assert_eq!(tracker.daily_sats(), 351);
}

#[test]
fn test_budget_check_limit_prevents_overspend() {
    let limits = BudgetLimits {
        max_per_task: 500,
        max_per_hour: 100_000,
        max_per_day: 1_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    // Spend 400 sats
    tracker.record("llm", "think", 400, Value::Null);

    // Check: 400 + 50 = 450 <= 500 → OK
    assert!(tracker.check_limit(50).is_ok());

    // Check: 400 + 200 = 600 > 500 → ERROR
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Task budget limit reached"));
}

#[test]
fn test_budget_hourly_limit() {
    let limits = BudgetLimits {
        max_per_task: 1_000_000,
        max_per_hour: 1000,
        max_per_day: 10_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    tracker.record("llm", "think", 900, Value::Null);

    // 900 + 200 > 1000 hourly limit
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Hourly budget limit reached"));
}

#[test]
fn test_budget_daily_limit() {
    let limits = BudgetLimits {
        max_per_task: 1_000_000,
        max_per_hour: 1_000_000,
        max_per_day: 1000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    tracker.record("llm", "think", 900, Value::Null);

    // 900 + 200 > 1000 daily limit
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Daily budget limit reached"));
}

#[test]
fn test_budget_jsonl_log_roundtrip() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("budget.jsonl");

    let mut tracker = BudgetTracker::new(Some(log_path.clone()), BudgetLimits::default());

    tracker.record(
        "llm",
        "think",
        200,
        serde_json::json!({"model": "gpt-5-mini", "tokens": 150}),
    );
    tracker.record(
        "proofs",
        "brc18",
        1,
        serde_json::json!({"txid": "deadbeef"}),
    );
    tracker.record("tokens", "brc48", 5, Value::Null);

    // Read back and verify
    let contents = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = contents.trim().lines().collect();
    assert_eq!(lines.len(), 3);

    // Parse each line
    let e1: SpendingEntry = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(e1.service, "llm");
    assert_eq!(e1.operation, "think");
    assert_eq!(e1.sats, 200);

    let e2: SpendingEntry = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(e2.service, "proofs");
    assert_eq!(e2.sats, 1);

    let e3: SpendingEntry = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(e3.service, "tokens");
    assert_eq!(e3.sats, 5);
}

#[test]
fn test_budget_report_structure() {
    let limits = BudgetLimits {
        max_per_task: 10_000,
        max_per_hour: 100_000,
        max_per_day: 1_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    tracker.record("llm", "think", 200, Value::Null);
    tracker.record("llm", "think", 300, Value::Null);
    tracker.record("proofs", "brc18", 1, Value::Null);

    let report = tracker.report();

    assert_eq!(report.task_sats, 501);
    assert_eq!(report.total_operations, 3);
    assert_eq!(report.services.len(), 2);
    assert_eq!(report.services["llm"].total_sats, 500);
    assert_eq!(report.services["llm"].count, 2);
    assert_eq!(report.services["proofs"].total_sats, 1);
    assert_eq!(report.services["proofs"].count, 1);

    // Limits
    assert_eq!(report.limits.max_per_task, 10_000);
    assert_eq!(report.limits.task_remaining, 9_499);
    assert_eq!(report.limits.hourly_remaining, 99_499);
}

#[test]
fn test_budget_report_serializable() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
    tracker.record("llm", "think", 100, Value::Null);

    let report = tracker.report();
    let json = serde_json::to_string(&report).unwrap();

    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["task_sats"], 100);
    assert_eq!(parsed["total_operations"], 1);
}

#[test]
fn test_budget_reset_task_preserves_history() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());

    tracker.record("llm", "think", 500, Value::Null);
    assert_eq!(tracker.task_sats(), 500);

    tracker.reset_task();
    assert_eq!(tracker.task_sats(), 0);

    // But hourly/daily totals still include old entries
    assert_eq!(tracker.hourly_sats(), 500);
    assert_eq!(tracker.daily_sats(), 500);

    // New task spending starts fresh
    tracker.record("llm", "think", 100, Value::Null);
    assert_eq!(tracker.task_sats(), 100);
    assert_eq!(tracker.hourly_sats(), 600);
}

#[test]
fn test_budget_from_config() {
    let config = dolphin_milk::config::BudgetConfig {
        max_per_task: 1000,
        max_per_hour: 2000,
        max_per_day: 3000,
        low_power_threshold: 500,
        staging_threshold: 500_000,
        ..Default::default()
    };
    let dir = tempdir().unwrap();
    let tracker = BudgetTracker::from_config(&config, dir.path());

    // Limits match config
    assert!(tracker.check_limit(999).is_ok());
    assert!(tracker.check_limit(1001).is_err());
}

#[test]
fn test_budget_zero_sats_allowed() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
    tracker.record("llm", "think", 0, Value::Null);
    assert_eq!(tracker.task_sats(), 0);
    assert_eq!(tracker.report().total_operations, 1);
}

#[test]
fn test_budget_replay_log_restores_entries_and_services() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("budget.jsonl");

    // Write known spending entries to the JSONL file
    let entries = vec![
        SpendingEntry {
            timestamp: Utc::now(),
            service: "llm".into(),
            operation: "think".into(),
            sats: 200,
            details: serde_json::json!({"model": "gpt-5-mini"}),
        },
        SpendingEntry {
            timestamp: Utc::now(),
            service: "llm".into(),
            operation: "think".into(),
            sats: 300,
            details: Value::Null,
        },
        SpendingEntry {
            timestamp: Utc::now(),
            service: "proofs".into(),
            operation: "brc18".into(),
            sats: 50,
            details: serde_json::json!({"txid": "abc123"}),
        },
    ];

    {
        let mut f = std::fs::File::create(&log_path).unwrap();
        for entry in &entries {
            let line = serde_json::to_string(entry).unwrap();
            writeln!(f, "{}", line).unwrap();
        }
    }

    // Create tracker and replay
    let mut tracker = BudgetTracker::new(Some(log_path), BudgetLimits::default());
    tracker.replay_log();

    // Entries restored
    assert_eq!(tracker.entries().len(), 3);

    // Per-service totals restored
    let report = tracker.report();
    assert_eq!(report.services["llm"].total_sats, 500);
    assert_eq!(report.services["llm"].count, 2);
    assert_eq!(report.services["proofs"].total_sats, 50);
    assert_eq!(report.services["proofs"].count, 1);

    // Hourly and daily sats include replayed entries
    assert_eq!(tracker.hourly_sats(), 550);
    assert_eq!(tracker.daily_sats(), 550);

    // task_sats stays at zero (per-task counter)
    assert_eq!(tracker.task_sats(), 0);
}

#[test]
fn test_budget_replay_log_skips_malformed_lines() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("budget.jsonl");

    // Write a mix of valid and invalid lines
    let valid_entry = SpendingEntry {
        timestamp: Utc::now(),
        service: "llm".into(),
        operation: "think".into(),
        sats: 100,
        details: Value::Null,
    };

    {
        let mut f = std::fs::File::create(&log_path).unwrap();
        writeln!(f, "{}", serde_json::to_string(&valid_entry).unwrap()).unwrap();
        writeln!(f, "this is not valid json").unwrap();
        writeln!(f, "{{\"incomplete\": true").unwrap();
        writeln!(f, "{}", serde_json::to_string(&valid_entry).unwrap()).unwrap();
    }

    let mut tracker = BudgetTracker::new(Some(log_path), BudgetLimits::default());
    tracker.replay_log();

    // Only the 2 valid entries should be replayed
    assert_eq!(tracker.entries().len(), 2);
    assert_eq!(tracker.hourly_sats(), 200);

    let report = tracker.report();
    assert_eq!(report.services["llm"].total_sats, 200);
    assert_eq!(report.services["llm"].count, 2);
}

#[test]
fn test_budget_from_config_replays_existing_log() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("budget.jsonl");

    // Pre-populate the log file
    let entry = SpendingEntry {
        timestamp: Utc::now(),
        service: "tokens".into(),
        operation: "brc48".into(),
        sats: 75,
        details: Value::Null,
    };

    {
        let mut f = std::fs::File::create(&log_path).unwrap();
        writeln!(f, "{}", serde_json::to_string(&entry).unwrap()).unwrap();
    }

    // from_config should replay automatically
    let config = dolphin_milk::config::BudgetConfig {
        max_per_task: 10_000,
        max_per_hour: 100_000,
        max_per_day: 1_000_000,
        low_power_threshold: 500,
        staging_threshold: 500_000,
        ..Default::default()
    };
    let tracker = BudgetTracker::from_config(&config, dir.path());

    assert_eq!(tracker.entries().len(), 1);
    assert_eq!(tracker.hourly_sats(), 75);
    assert_eq!(tracker.daily_sats(), 75);
    assert_eq!(tracker.task_sats(), 0);

    let report = tracker.report();
    assert_eq!(report.services["tokens"].total_sats, 75);
    assert_eq!(report.services["tokens"].count, 1);
}

#[test]
fn test_budget_replay_log_no_file() {
    // When the log file does not exist, replay_log is a no-op
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("nonexistent.jsonl");

    let mut tracker = BudgetTracker::new(Some(log_path), BudgetLimits::default());
    tracker.replay_log();

    assert_eq!(tracker.entries().len(), 0);
    assert_eq!(tracker.hourly_sats(), 0);
}

// ---------------------------------------------------------------------------
// Task 2.1: BudgetReport includes balance field, report_with_balance works
// ---------------------------------------------------------------------------

#[test]
fn test_budget_report_includes_balance_field() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
    tracker.record("llm", "think", 100, Value::Null);

    // Default report() should have balance = 0
    let report = tracker.report();
    assert_eq!(report.balance, 0);

    // report_with_balance() should set the balance field
    let report = tracker.report_with_balance(50_000);
    assert_eq!(report.balance, 50_000);
    // Other fields are unchanged
    assert_eq!(report.task_sats, 100);
    assert_eq!(report.total_operations, 1);
}

#[test]
fn test_budget_report_balance_serializes_correctly() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
    tracker.record("llm", "think", 200, Value::Null);

    let report = tracker.report_with_balance(12345);
    let json = serde_json::to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // balance field must be present in serialized JSON
    assert_eq!(parsed["balance"], 12345);
    assert_eq!(parsed["task_sats"], 200);
}

#[test]
fn test_budget_report_balance_zero_still_present() {
    let tracker = BudgetTracker::new(None, BudgetLimits::default());
    let report = tracker.report_with_balance(0);
    let json = serde_json::to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // balance: 0 should still be in the JSON (default, not skip_serializing_if)
    assert!(parsed.get("balance").is_some());
    assert_eq!(parsed["balance"], 0);
}

#[test]
fn test_budget_report_balance_deserialization_default() {
    // When balance is missing from JSON, it should default to 0
    let json_without_balance = r#"{
        "task_sats": 100,
        "total_operations": 1,
        "services": {},
        "hourly_sats": 100,
        "daily_sats": 100,
        "limits": {
            "max_per_task": 250000,
            "max_per_hour": 2500000,
            "max_per_day": 25000000,
            "task_remaining": 249900,
            "hourly_remaining": 2499900,
            "daily_remaining": 24999900
        }
    }"#;

    let report: dolphin_milk::budget::BudgetReport =
        serde_json::from_str(json_without_balance).unwrap();
    assert_eq!(report.balance, 0);
    assert_eq!(report.task_sats, 100);
}

#[test]
fn test_budget_report_delegates_to_report_with_balance() {
    let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
    tracker.record("llm", "think", 300, Value::Null);

    // report() should produce the same output as report_with_balance(0)
    let r1 = tracker.report();
    let r2 = tracker.report_with_balance(0);

    assert_eq!(r1.task_sats, r2.task_sats);
    assert_eq!(r1.total_operations, r2.total_operations);
    assert_eq!(r1.balance, r2.balance);
    assert_eq!(r1.balance, 0);
}

#[test]
fn test_budget_replay_then_record_appends_correctly() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("budget.jsonl");

    // Pre-populate with one entry
    let entry = SpendingEntry {
        timestamp: Utc::now(),
        service: "llm".into(),
        operation: "think".into(),
        sats: 100,
        details: Value::Null,
    };
    {
        let mut f = std::fs::File::create(&log_path).unwrap();
        writeln!(f, "{}", serde_json::to_string(&entry).unwrap()).unwrap();
    }

    // Create tracker, replay, then record a new entry
    let mut tracker = BudgetTracker::new(Some(log_path.clone()), BudgetLimits::default());
    tracker.replay_log();
    tracker.record("proofs", "brc18", 50, Value::Null);

    // In-memory state has both
    assert_eq!(tracker.entries().len(), 2);
    assert_eq!(tracker.hourly_sats(), 150);
    assert_eq!(tracker.task_sats(), 50); // only the new record

    // JSONL file has both (1 original + 1 appended)
    let contents = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = contents.trim().lines().collect();
    assert_eq!(lines.len(), 2);
}

// ===========================================================================
// Issue #6: Weekly/monthly/lifetime budget caps
// ===========================================================================

#[test]
fn test_weekly_limit_prevents_overspend() {
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 1000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    // Add entries spread across the 7-day window (all recent)
    for _ in 0..5 {
        tracker.record("llm", "think", 180, Value::Null);
    }
    assert_eq!(tracker.weekly_sats(), 900);

    // 900 + 200 > 1000 weekly limit
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Weekly budget limit reached"));
}

#[test]
fn test_monthly_limit_prevents_overspend() {
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 2000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    tracker.record("llm", "think", 1800, Value::Null);
    assert_eq!(tracker.monthly_sats(), 1800);

    // 1800 + 300 > 2000 monthly limit
    let err = tracker.check_limit(300).unwrap_err();
    assert!(err.to_string().contains("Monthly budget limit reached"));
}

#[test]
fn test_lifetime_limit_prevents_overspend() {
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 10_000_000,
        max_lifetime: 5000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    tracker.record("llm", "think", 4500, Value::Null);

    // 4500 + 600 > 5000 lifetime limit
    let err = tracker.check_limit(600).unwrap_err();
    assert!(err.to_string().contains("Lifetime budget limit reached"));

    // 4500 + 400 <= 5000 — should pass
    assert!(tracker.check_limit(400).is_ok());
}

#[test]
fn test_weekly_limit_allows_after_window_expires() {
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 1000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    // Manually add an entry older than 7 days (8 days ago)
    tracker.insert_entry(SpendingEntry {
        timestamp: Utc::now() - chrono::Duration::hours(192), // 8 days
        service: "llm".into(),
        operation: "think".into(),
        sats: 999,
        details: Value::Null,
    });

    // Weekly should NOT count the old entry
    assert_eq!(tracker.weekly_sats(), 0);
    // But lifetime does
    assert_eq!(tracker.lifetime_sats(), 999);

    // Should be able to spend up to weekly limit
    assert!(tracker.check_limit(900).is_ok());
}

#[test]
fn test_monthly_limit_allows_after_window_expires() {
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 1000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    // Add an entry older than 30 days (31 days ago = 744 hours)
    tracker.insert_entry(SpendingEntry {
        timestamp: Utc::now() - chrono::Duration::hours(744),
        service: "llm".into(),
        operation: "think".into(),
        sats: 999,
        details: Value::Null,
    });

    // Monthly should NOT count the old entry
    assert_eq!(tracker.monthly_sats(), 0);

    // Should be able to spend up to monthly limit
    assert!(tracker.check_limit(900).is_ok());
}

#[test]
fn test_lifetime_is_cumulative_across_all_time() {
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 10_000_000,
        max_lifetime: 5000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);

    // Add entries across different time periods — all count toward lifetime
    tracker.insert_entry(SpendingEntry {
        timestamp: Utc::now() - chrono::Duration::hours(1000), // ~41 days ago
        service: "llm".into(),
        operation: "think".into(),
        sats: 2000,
        details: Value::Null,
    });
    tracker.insert_entry(SpendingEntry {
        timestamp: Utc::now() - chrono::Duration::hours(500), // ~20 days ago
        service: "llm".into(),
        operation: "think".into(),
        sats: 2000,
        details: Value::Null,
    });

    assert_eq!(tracker.lifetime_sats(), 4000);

    // 4000 + 1500 > 5000 — should fail
    let err = tracker.check_limit(1500).unwrap_err();
    assert!(err.to_string().contains("Lifetime"));
}

#[test]
fn test_budget_report_includes_new_windows() {
    let limits = BudgetLimits {
        max_per_task: 10_000,
        max_per_hour: 100_000,
        max_per_day: 1_000_000,
        max_per_week: 5_000_000,
        max_per_month: 20_000_000,
        max_lifetime: 100_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 500, Value::Null);

    let report = tracker.report();

    // New fields are present and correct
    assert_eq!(report.weekly_sats, 500);
    assert_eq!(report.monthly_sats, 500);
    assert_eq!(report.lifetime_sats, 500);
    assert_eq!(report.limits.max_per_week, 5_000_000);
    assert_eq!(report.limits.max_per_month, 20_000_000);
    assert_eq!(report.limits.max_lifetime, 100_000_000);
    assert_eq!(report.limits.weekly_remaining, 4_999_500);
    assert_eq!(report.limits.monthly_remaining, 19_999_500);
    assert_eq!(report.limits.lifetime_remaining, 99_999_500);
}

#[test]
fn test_default_limits_are_sane() {
    let limits = BudgetLimits::default();
    assert!(limits.max_per_task > 0);
    assert!(limits.max_per_hour > limits.max_per_task);
    assert!(limits.max_per_day > limits.max_per_hour);
    assert!(limits.max_per_week > 0);
    assert!(limits.max_per_month > limits.max_per_week);
    assert_eq!(limits.max_lifetime, 0); // unlimited by default
    assert_eq!(limits.enforcement, "strict");
}

#[test]
fn test_all_limits_checked_in_order() {
    // Set each limit tight so we can verify which fires first.
    // Task = 1000, all others very large.
    let limits = BudgetLimits {
        max_per_task: 1000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 10_000_000,
        max_lifetime: 10_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 900, Value::Null);
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Task budget limit"));

    // Hourly = 1000, task = large
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 1000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 10_000_000,
        max_lifetime: 10_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 900, Value::Null);
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Hourly budget limit"));

    // Daily = 1000, task/hourly = large
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 1000,
        max_per_week: 10_000_000,
        max_per_month: 10_000_000,
        max_lifetime: 10_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 900, Value::Null);
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Daily budget limit"));

    // Weekly = 1000, task/hourly/daily = large
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 1000,
        max_per_month: 10_000_000,
        max_lifetime: 10_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 900, Value::Null);
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Weekly budget limit"));

    // Monthly = 1000
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 1000,
        max_lifetime: 10_000_000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 900, Value::Null);
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Monthly budget limit"));

    // Lifetime = 1000
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 10_000_000,
        max_per_month: 10_000_000,
        max_lifetime: 1000,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 900, Value::Null);
    let err = tracker.check_limit(200).unwrap_err();
    assert!(err.to_string().contains("Lifetime budget limit"));
}

// ===========================================================================
// Issue #7: Advisory budget mode
// ===========================================================================

#[test]
fn test_advisory_mode_logs_warning_but_continues() {
    let limits = BudgetLimits {
        max_per_task: 500,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        enforcement: "advisory".into(),
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 400, Value::Null);

    // check_limit still returns Err (the caller decides whether to block)
    let result = tracker.check_limit(200);
    assert!(result.is_err());

    // But is_advisory() tells the caller to continue
    assert!(tracker.is_advisory());
}

#[test]
fn test_strict_mode_blocks_execution() {
    let limits = BudgetLimits {
        max_per_task: 500,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        enforcement: "strict".into(),
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 400, Value::Null);

    let result = tracker.check_limit(200);
    assert!(result.is_err());
    assert!(!tracker.is_advisory());
}

#[test]
fn test_advisory_mode_default_is_strict() {
    let limits = BudgetLimits::default();
    assert_eq!(limits.enforcement, "strict");

    let tracker = BudgetTracker::new(None, limits);
    assert!(!tracker.is_advisory());
}

#[test]
fn test_advisory_mode_config_from_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.toml");
    std::fs::write(
        &path,
        r#"
[budget]
enforcement = "advisory"
"#,
    )
    .unwrap();

    let cfg = dolphin_milk::config::load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.budget.enforcement, "advisory");
}

#[test]
fn test_advisory_mode_env_override() {
    // Verify that DOLPHIN_MILK_BUDGET_ENFORCEMENT env var is recognized by the config system.
    // We can't safely set env vars in parallel tests, so just test that the
    // config struct's enforcement field round-trips correctly.
    let config = dolphin_milk::config::BudgetConfig {
        enforcement: "advisory".into(),
        ..Default::default()
    };
    let limits = BudgetLimits::from(&config);
    assert_eq!(limits.enforcement, "advisory");
    let tracker = BudgetTracker::new(None, limits);
    assert!(tracker.is_advisory());
}

#[test]
fn test_advisory_step_event_emitted() {
    use dolphin_milk::events::StepEvent;

    let event = StepEvent::BudgetAdvisory {
        iteration: 3,
        message: "Weekly budget limit reached".into(),
        limit_type: "pre-check".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"type\":\"budget_advisory\""));
    assert!(json.contains("\"iteration\":3"));
    assert!(json.contains("Weekly budget limit reached"));
    assert!(json.contains("\"limit_type\":\"pre-check\""));
}

#[test]
fn test_cert_weekly_limit_overrides_config() {
    let mut tracker = BudgetTracker::new(
        None,
        BudgetLimits {
            max_per_week: 100_000_000,
            ..Default::default()
        },
    );
    assert_eq!(tracker.limits().max_per_week, 100_000_000);

    // Cert sets a tighter weekly limit
    tracker.apply_cert_limits(None, None, None, Some(50_000), None, None, None);
    assert_eq!(tracker.limits().max_per_week, 50_000);
}

#[test]
fn test_cert_monthly_limit_overrides_config() {
    let mut tracker = BudgetTracker::new(
        None,
        BudgetLimits {
            max_per_month: 500_000_000,
            ..Default::default()
        },
    );

    tracker.apply_cert_limits(None, None, None, None, Some(200_000), None, None);
    assert_eq!(tracker.limits().max_per_month, 200_000);
}

#[test]
fn test_cert_lifetime_limit_overrides_config() {
    let mut tracker = BudgetTracker::new(
        None,
        BudgetLimits {
            max_lifetime: 0, // unlimited
            ..Default::default()
        },
    );

    tracker.apply_cert_limits(None, None, None, None, None, Some(1_000_000), None);
    assert_eq!(tracker.limits().max_lifetime, 1_000_000);
}

#[test]
fn test_cert_zero_value_means_no_limit() {
    let mut tracker = BudgetTracker::new(
        None,
        BudgetLimits {
            max_per_week: 100_000,
            max_per_month: 200_000,
            max_lifetime: 300_000,
            ..Default::default()
        },
    );

    // Zero values should NOT override (existing pattern: v > 0 guard)
    tracker.apply_cert_limits(None, None, None, Some(0), Some(0), Some(0), None);
    assert_eq!(tracker.limits().max_per_week, 100_000); // unchanged
    assert_eq!(tracker.limits().max_per_month, 200_000); // unchanged
    assert_eq!(tracker.limits().max_lifetime, 300_000); // unchanged
}

#[test]
fn test_unlimited_weekly_allows_any_spend() {
    // max_per_week = 0 means unlimited
    let limits = BudgetLimits {
        max_per_task: 10_000_000,
        max_per_hour: 10_000_000,
        max_per_day: 10_000_000,
        max_per_week: 0,
        max_per_month: 0,
        max_lifetime: 0,
        ..Default::default()
    };
    let mut tracker = BudgetTracker::new(None, limits);
    tracker.record("llm", "think", 9_999_000, Value::Null);

    // Should pass because weekly/monthly/lifetime are all unlimited (0)
    assert!(tracker.check_limit(500).is_ok());
}

#[test]
fn test_budget_report_enforcement_field() {
    let limits = BudgetLimits {
        enforcement: "advisory".into(),
        ..Default::default()
    };
    let tracker = BudgetTracker::new(None, limits);
    let report = tracker.report();
    assert_eq!(report.limits.enforcement, "advisory");

    // Serialize and verify
    let json = serde_json::to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["limits"]["enforcement"], "advisory");
}

#[test]
fn test_budget_report_deserialization_with_new_fields_missing() {
    // Ensure backward compat: old JSON without new fields still deserializes
    let json_old = r#"{
        "task_sats": 100,
        "total_operations": 1,
        "services": {},
        "hourly_sats": 100,
        "daily_sats": 100,
        "limits": {
            "max_per_task": 250000,
            "max_per_hour": 2500000,
            "max_per_day": 25000000,
            "task_remaining": 249900,
            "hourly_remaining": 2499900,
            "daily_remaining": 24999900
        }
    }"#;

    let report: dolphin_milk::budget::BudgetReport = serde_json::from_str(json_old).unwrap();
    assert_eq!(report.weekly_sats, 0);
    assert_eq!(report.monthly_sats, 0);
    assert_eq!(report.lifetime_sats, 0);
    assert_eq!(report.limits.max_per_week, 0);
    assert_eq!(report.limits.max_per_month, 0);
    assert_eq!(report.limits.max_lifetime, 0);
    assert_eq!(report.limits.enforcement, "strict"); // default
}

#[test]
fn test_from_config_with_new_limits() {
    let config = dolphin_milk::config::BudgetConfig {
        max_per_task: 1000,
        max_per_hour: 2000,
        max_per_day: 3000,
        max_per_week: 4000,
        max_per_month: 5000,
        max_lifetime: 6000,
        low_power_threshold: 500,
        staging_threshold: 500_000,
        enforcement: "advisory".into(),
    };
    let dir = tempfile::tempdir().unwrap();
    let tracker = BudgetTracker::from_config(&config, dir.path());
    assert_eq!(tracker.limits().max_per_week, 4000);
    assert_eq!(tracker.limits().max_per_month, 5000);
    assert_eq!(tracker.limits().max_lifetime, 6000);
    assert!(tracker.is_advisory());
}
