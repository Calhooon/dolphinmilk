//! Tests for scheduled tasks — schedule creation, interval parsing, scan_schedules,
//! cancellation, and listing.

use std::path::PathBuf;

use dolphin_milk::tools::schedule_tools::{
    all_schedule_tools, compute_next_cron_run, parse_interval, Schedule, ScheduleType,
};

// ---------------------------------------------------------------------------
// Interval parsing tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_interval_seconds() {
    assert_eq!(parse_interval("30s").unwrap(), 30);
    assert_eq!(parse_interval("1s").unwrap(), 1);
    assert_eq!(parse_interval("120s").unwrap(), 120);
}

#[test]
fn test_parse_interval_minutes() {
    assert_eq!(parse_interval("30m").unwrap(), 1800);
    assert_eq!(parse_interval("1m").unwrap(), 60);
    assert_eq!(parse_interval("5m").unwrap(), 300);
}

#[test]
fn test_parse_interval_hours() {
    assert_eq!(parse_interval("1h").unwrap(), 3600);
    assert_eq!(parse_interval("24h").unwrap(), 86400);
    assert_eq!(parse_interval("12h").unwrap(), 43200);
}

#[test]
fn test_parse_interval_days() {
    assert_eq!(parse_interval("1d").unwrap(), 86400);
    assert_eq!(parse_interval("7d").unwrap(), 604800);
    assert_eq!(parse_interval("30d").unwrap(), 2592000);
}

#[test]
fn test_parse_interval_whitespace_trimmed() {
    assert_eq!(parse_interval("  1h  ").unwrap(), 3600);
    assert_eq!(parse_interval(" 30m ").unwrap(), 1800);
}

#[test]
fn test_parse_interval_invalid_suffix() {
    assert!(parse_interval("10x").is_err());
    assert!(parse_interval("10w").is_err());
    assert!(parse_interval("10").is_err());
}

#[test]
fn test_parse_interval_invalid_number() {
    assert!(parse_interval("abch").is_err());
    assert!(parse_interval("-5h").is_err());
}

#[test]
fn test_parse_interval_empty() {
    assert!(parse_interval("").is_err());
    assert!(parse_interval("   ").is_err());
}

#[test]
fn test_parse_interval_zero() {
    assert!(parse_interval("0h").is_err());
    assert!(parse_interval("0m").is_err());
    assert!(parse_interval("0s").is_err());
    assert!(parse_interval("0d").is_err());
}

// ---------------------------------------------------------------------------
// Schedule struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_schedule_serde_roundtrip() {
    let now = chrono::Utc::now();
    let schedule = Schedule {
        id: "sched-test-123".to_string(),
        description: "Check balance daily".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 86400,
        cron_expression: None,
        next_run: now.to_rfc3339(),
        conversation_id: Some("conv-abc".to_string()),
        created_by: "agent".to_string(),
        enabled: true,
        created_at: now.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    let json_str = serde_json::to_string_pretty(&schedule).unwrap();
    let parsed: Schedule = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.id, "sched-test-123");
    assert_eq!(parsed.description, "Check balance daily");
    assert_eq!(parsed.schedule_type, ScheduleType::Interval);
    assert_eq!(parsed.interval_secs, 86400);
    assert!(parsed.enabled);
    assert_eq!(parsed.run_count, 0);
    assert!(parsed.last_run.is_none());
    assert_eq!(parsed.conversation_id, Some("conv-abc".to_string()));
    assert_eq!(parsed.created_by, "agent");
    assert!(!parsed.one_shot);
}

#[test]
fn test_schedule_serde_no_optional_fields() {
    let now = chrono::Utc::now();
    let schedule = Schedule {
        id: "sched-test-456".to_string(),
        description: "No conversation".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: now.to_rfc3339(),
        conversation_id: None,
        created_by: "parent".to_string(),
        enabled: true,
        created_at: now.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    let json_str = serde_json::to_string(&schedule).unwrap();
    // conversation_id and last_run should be omitted
    assert!(!json_str.contains("conversation_id"));
    assert!(!json_str.contains("last_run"));

    let parsed: Schedule = serde_json::from_str(&json_str).unwrap();
    assert!(parsed.conversation_id.is_none());
    assert!(parsed.last_run.is_none());
}

#[test]
fn test_schedule_with_last_run() {
    let now = chrono::Utc::now();
    let schedule = Schedule {
        id: "sched-ran".to_string(),
        description: "Already ran".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: now.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: now.to_rfc3339(),
        last_run: Some(now.to_rfc3339()),
        run_count: 5,
        run_history: vec![],
        one_shot: false,
    };

    let json_str = serde_json::to_string(&schedule).unwrap();
    assert!(json_str.contains("last_run"));

    let parsed: Schedule = serde_json::from_str(&json_str).unwrap();
    assert!(parsed.last_run.is_some());
    assert_eq!(parsed.run_count, 5);
}

// ---------------------------------------------------------------------------
// Tool registration tests
// ---------------------------------------------------------------------------

#[test]
fn test_all_schedule_tools_count() {
    let tools = all_schedule_tools(PathBuf::from("/tmp/test"));
    assert_eq!(tools.len(), 3);
}

#[test]
fn test_all_schedule_tools_names() {
    let tools = all_schedule_tools(PathBuf::from("/tmp/test"));
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"create_schedule"));
    assert!(names.contains(&"list_schedules"));
    assert!(names.contains(&"cancel_schedule"));
}

#[test]
fn test_all_schedule_tools_category() {
    let tools = all_schedule_tools(PathBuf::from("/tmp/test"));
    for tool in &tools {
        assert_eq!(tool.category, "schedule");
    }
}

#[test]
fn test_schedule_tools_have_descriptions() {
    let tools = all_schedule_tools(PathBuf::from("/tmp/test"));
    for tool in &tools {
        assert!(
            !tool.description.is_empty(),
            "Tool {} has empty description",
            tool.name
        );
    }
}

#[test]
fn test_schedule_tools_have_parameters() {
    let tools = all_schedule_tools(PathBuf::from("/tmp/test"));
    for tool in &tools {
        assert!(
            tool.parameters.is_object(),
            "Tool {} has non-object parameters",
            tool.name
        );
    }
}

// ---------------------------------------------------------------------------
// Tool execution tests (async)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_create_schedule_tool() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());

    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Check wallet balance",
        "interval": "1h"
    });

    let result = (create_tool.execute)(params).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed
        .get("id")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("sched-"));
    assert_eq!(
        parsed.get("description").unwrap().as_str().unwrap(),
        "Check wallet balance"
    );
    assert_eq!(parsed.get("interval_secs").unwrap().as_u64().unwrap(), 3600);
    assert!(parsed.get("next_run").is_some());

    // Verify file was created
    let schedules_dir = workspace.join("schedules");
    let id = parsed.get("id").unwrap().as_str().unwrap();
    let schedule_path = schedules_dir.join(format!("{id}.json"));
    assert!(schedule_path.exists());

    // Verify file content
    let content = std::fs::read_to_string(&schedule_path).unwrap();
    let schedule: Schedule = serde_json::from_str(&content).unwrap();
    assert_eq!(schedule.description, "Check wallet balance");
    assert_eq!(schedule.interval_secs, 3600);
    assert!(schedule.enabled);
    assert_eq!(schedule.run_count, 0);
    assert_eq!(schedule.created_by, "agent");
}

#[tokio::test]
async fn test_create_schedule_with_conversation_id() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());

    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Daily report",
        "interval": "24h",
        "conversation_id": "conv-daily-123"
    });

    let result = (create_tool.execute)(params).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    let id = parsed.get("id").unwrap().as_str().unwrap();
    let schedule_path = workspace.join("schedules").join(format!("{id}.json"));
    let content = std::fs::read_to_string(&schedule_path).unwrap();
    let schedule: Schedule = serde_json::from_str(&content).unwrap();
    assert_eq!(schedule.conversation_id, Some("conv-daily-123".to_string()));
}

#[tokio::test]
async fn test_create_schedule_missing_description() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "interval": "1h"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
}

#[tokio::test]
async fn test_create_schedule_missing_interval() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Some task"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
}

#[tokio::test]
async fn test_create_schedule_invalid_interval() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Some task",
        "interval": "invalid"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
}

#[tokio::test]
async fn test_list_schedules_empty() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let list_tool = tools.iter().find(|t| t.name == "list_schedules").unwrap();

    let result = (list_tool.execute)(serde_json::json!({})).await;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
    assert!(parsed.is_empty());
}

#[tokio::test]
async fn test_list_schedules_returns_all() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());

    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();
    let list_tool = tools.iter().find(|t| t.name == "list_schedules").unwrap();

    // Create two schedules
    (create_tool.execute)(serde_json::json!({
        "description": "Task A",
        "interval": "1h"
    }))
    .await;

    (create_tool.execute)(serde_json::json!({
        "description": "Task B",
        "interval": "24h"
    }))
    .await;

    let result = (list_tool.execute)(serde_json::json!({})).await;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed.len(), 2);

    // Verify both are present
    let descriptions: Vec<&str> = parsed
        .iter()
        .map(|s| s.get("description").unwrap().as_str().unwrap())
        .collect();
    assert!(descriptions.contains(&"Task A"));
    assert!(descriptions.contains(&"Task B"));
}

#[tokio::test]
async fn test_cancel_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());

    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();
    let cancel_tool = tools.iter().find(|t| t.name == "cancel_schedule").unwrap();

    // Create a schedule
    let result = (create_tool.execute)(serde_json::json!({
        "description": "Task to cancel",
        "interval": "1h"
    }))
    .await;
    let created: serde_json::Value = serde_json::from_str(&result).unwrap();
    let id = created.get("id").unwrap().as_str().unwrap();

    // Cancel it
    let cancel_result = (cancel_tool.execute)(serde_json::json!({
        "id": id
    }))
    .await;
    let cancel_parsed: serde_json::Value = serde_json::from_str(&cancel_result).unwrap();
    assert_eq!(cancel_parsed.get("id").unwrap().as_str().unwrap(), id);
    assert!(cancel_parsed.get("cancelled").unwrap().as_bool().unwrap());

    // Verify it's disabled on disk
    let schedule_path = workspace.join("schedules").join(format!("{id}.json"));
    let content = std::fs::read_to_string(&schedule_path).unwrap();
    let schedule: Schedule = serde_json::from_str(&content).unwrap();
    assert!(!schedule.enabled);
}

#[tokio::test]
async fn test_cancel_schedule_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let cancel_tool = tools.iter().find(|t| t.name == "cancel_schedule").unwrap();

    let result = (cancel_tool.execute)(serde_json::json!({
        "id": "sched-nonexistent"
    }))
    .await;
    assert!(result.starts_with("Error:"));
}

#[tokio::test]
async fn test_cancel_schedule_missing_id() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let cancel_tool = tools.iter().find(|t| t.name == "cancel_schedule").unwrap();

    let result = (cancel_tool.execute)(serde_json::json!({})).await;
    assert!(result.starts_with("Error:"));
}

// ---------------------------------------------------------------------------
// Scan schedules tests (simulating the Scheduler's scan_schedules behavior)
// ---------------------------------------------------------------------------

/// Helper to write a schedule file directly for scan testing.
fn write_schedule(workspace: &std::path::Path, schedule: &Schedule) {
    let schedules_dir = workspace.join("schedules");
    std::fs::create_dir_all(&schedules_dir).unwrap();
    let path = schedules_dir.join(format!("{}.json", schedule.id));
    let content = serde_json::to_string_pretty(schedule).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Helper to read a schedule file back.
fn read_schedule(workspace: &std::path::Path, id: &str) -> Schedule {
    let path = workspace.join("schedules").join(format!("{id}.json"));
    let content = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}

#[test]
fn test_scan_picks_up_due_schedule() {
    // Simulate what scan_schedules does: read due schedules, update next_run
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let schedule = Schedule {
        id: "sched-due-1".to_string(),
        description: "Due task".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: past.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: past.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    // Read and check it's due
    let loaded = read_schedule(&workspace, "sched-due-1");
    let next_run = chrono::DateTime::parse_from_rfc3339(&loaded.next_run).unwrap();
    assert!(next_run <= chrono::Utc::now(), "Schedule should be due");
    assert!(loaded.enabled);

    // Simulate updating after scan
    let now = chrono::Utc::now();
    let mut updated = loaded;
    updated.next_run = (now + chrono::Duration::seconds(updated.interval_secs as i64)).to_rfc3339();
    updated.last_run = Some(now.to_rfc3339());
    updated.run_count += 1;
    write_schedule(&workspace, &updated);

    // Verify updates
    let final_schedule = read_schedule(&workspace, "sched-due-1");
    assert_eq!(final_schedule.run_count, 1);
    assert!(final_schedule.last_run.is_some());
    let new_next = chrono::DateTime::parse_from_rfc3339(&final_schedule.next_run).unwrap();
    assert!(
        new_next > chrono::Utc::now(),
        "next_run should be in the future"
    );
}

#[test]
fn test_scan_skips_future_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let future = chrono::Utc::now() + chrono::Duration::seconds(3600);
    let schedule = Schedule {
        id: "sched-future-1".to_string(),
        description: "Future task".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: future.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: chrono::Utc::now().to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    let loaded = read_schedule(&workspace, "sched-future-1");
    let next_run = chrono::DateTime::parse_from_rfc3339(&loaded.next_run).unwrap();
    assert!(
        next_run > chrono::Utc::now(),
        "Schedule should NOT be due yet"
    );
    assert_eq!(loaded.run_count, 0);
}

#[test]
fn test_scan_skips_disabled_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let schedule = Schedule {
        id: "sched-disabled-1".to_string(),
        description: "Disabled task".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: past.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: false, // disabled
        created_at: past.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    let loaded = read_schedule(&workspace, "sched-disabled-1");
    assert!(!loaded.enabled, "Schedule should be disabled");
    // Even though next_run is in the past, it should not be picked up
}

#[test]
fn test_scan_schedules_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    // No schedules dir at all — should not panic
    let schedules_dir = workspace.join("schedules");
    assert!(!schedules_dir.exists());
}

#[test]
fn test_schedule_conversation_id_passed_through() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let schedule = Schedule {
        id: "sched-conv-1".to_string(),
        description: "Recurring conversation task".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: past.to_rfc3339(),
        conversation_id: Some("conv-recurring-abc".to_string()),
        created_by: "agent".to_string(),
        enabled: true,
        created_at: past.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    let loaded = read_schedule(&workspace, "sched-conv-1");
    assert_eq!(
        loaded.conversation_id,
        Some("conv-recurring-abc".to_string())
    );
}

#[test]
fn test_schedule_run_count_increments() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let now = chrono::Utc::now();
    let schedule = Schedule {
        id: "sched-counter".to_string(),
        description: "Counter test".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 60,
        cron_expression: None,
        next_run: (now - chrono::Duration::seconds(10)).to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: now.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    // Simulate 3 scan cycles
    for expected_count in 1..=3 {
        let mut s = read_schedule(&workspace, "sched-counter");
        s.next_run =
            (chrono::Utc::now() + chrono::Duration::seconds(s.interval_secs as i64)).to_rfc3339();
        s.last_run = Some(chrono::Utc::now().to_rfc3339());
        s.run_count += 1;
        write_schedule(&workspace, &s);

        let updated = read_schedule(&workspace, "sched-counter");
        assert_eq!(updated.run_count, expected_count);
    }
}

#[test]
fn test_atomic_schedule_write_no_orphaned_tmp() {
    // Simulate the atomic write pattern: write .tmp, rename to .json.
    // After a successful cycle, no .tmp file should remain.
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let schedules_dir = workspace.join("schedules");
    std::fs::create_dir_all(&schedules_dir).unwrap();

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let schedule = Schedule {
        id: "sched-atomic-1".to_string(),
        description: "Atomic write test".to_string(),
        schedule_type: ScheduleType::Interval,
        interval_secs: 3600,
        cron_expression: None,
        next_run: past.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: past.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    let path = schedules_dir.join("sched-atomic-1.json");
    let content = serde_json::to_string_pretty(&schedule).unwrap();
    std::fs::write(&path, &content).unwrap();

    // Simulate the atomic update: write to .tmp then rename
    let mut updated: Schedule =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    updated.run_count += 1;
    updated.last_run = Some(chrono::Utc::now().to_rfc3339());
    updated.next_run = (chrono::Utc::now() + chrono::Duration::seconds(3600)).to_rfc3339();

    let tmp_path = path.with_extension("json.tmp");
    let updated_content = serde_json::to_string_pretty(&updated).unwrap();
    std::fs::write(&tmp_path, &updated_content).unwrap();
    assert!(tmp_path.exists(), ".tmp file should exist before rename");
    std::fs::rename(&tmp_path, &path).unwrap();

    // After rename: .tmp should be gone, .json should have updated content
    assert!(
        !tmp_path.exists(),
        ".tmp file should not exist after rename"
    );
    let final_schedule = read_schedule(&workspace, "sched-atomic-1");
    assert_eq!(final_schedule.run_count, 1);
    assert!(final_schedule.last_run.is_some());
}

// ---------------------------------------------------------------------------
// Cron and Once schedule type tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_create_cron_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Monday morning standup",
        "schedule_type": "cron",
        "cron_expression": "0 9 * * MON"
    });

    let result = (create_tool.execute)(params).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed
        .get("id")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("sched-"));
    assert_eq!(
        parsed.get("schedule_type").unwrap().as_str().unwrap(),
        "cron"
    );
    assert_eq!(
        parsed.get("cron_expression").unwrap().as_str().unwrap(),
        "0 9 * * MON"
    );
    assert!(parsed.get("next_run").is_some());
    // interval_secs should be absent for cron
    assert!(parsed.get("interval_secs").is_none());

    // Verify file content
    let id = parsed.get("id").unwrap().as_str().unwrap();
    let schedule_path = workspace.join("schedules").join(format!("{id}.json"));
    let content = std::fs::read_to_string(&schedule_path).unwrap();
    let schedule: Schedule = serde_json::from_str(&content).unwrap();
    assert_eq!(schedule.schedule_type, ScheduleType::Cron);
    assert_eq!(schedule.cron_expression, Some("0 9 * * MON".to_string()));
    assert_eq!(schedule.interval_secs, 0);
    assert!(!schedule.one_shot);
}

#[tokio::test]
async fn test_create_once_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "One-time reminder",
        "schedule_type": "once",
        "interval": "30m"
    });

    let result = (create_tool.execute)(params).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed
        .get("id")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("sched-"));
    assert_eq!(
        parsed.get("schedule_type").unwrap().as_str().unwrap(),
        "once"
    );
    assert_eq!(parsed.get("interval_secs").unwrap().as_u64().unwrap(), 1800);

    // Verify file content
    let id = parsed.get("id").unwrap().as_str().unwrap();
    let schedule: Schedule = serde_json::from_str(
        &std::fs::read_to_string(workspace.join("schedules").join(format!("{id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(schedule.schedule_type, ScheduleType::Once);
    assert!(schedule.one_shot);
    assert!(schedule.enabled); // still enabled until it fires
}

#[tokio::test]
async fn test_create_cron_schedule_missing_expression() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Missing cron",
        "schedule_type": "cron"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
    assert!(result.contains("cron_expression"));
}

#[tokio::test]
async fn test_create_cron_schedule_invalid_expression() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Bad cron",
        "schedule_type": "cron",
        "cron_expression": "not a valid cron"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
}

#[tokio::test]
async fn test_create_once_schedule_missing_interval() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Missing delay",
        "schedule_type": "once"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
    assert!(result.contains("interval"));
}

#[tokio::test]
async fn test_create_invalid_schedule_type() {
    let dir = tempfile::tempdir().unwrap();
    let tools = all_schedule_tools(dir.path().to_path_buf());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Bad type",
        "schedule_type": "weekly"
    });

    let result = (create_tool.execute)(params).await;
    assert!(result.starts_with("Error:"));
    assert!(result.contains("invalid schedule_type"));
}

#[tokio::test]
async fn test_create_interval_without_explicit_type() {
    // Backward compat: omitting schedule_type defaults to interval
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let tools = all_schedule_tools(workspace.clone());
    let create_tool = tools.iter().find(|t| t.name == "create_schedule").unwrap();

    let params = serde_json::json!({
        "description": "Old-style schedule",
        "interval": "1h"
    });

    let result = (create_tool.execute)(params).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed.get("schedule_type").unwrap().as_str().unwrap(),
        "interval"
    );
    assert_eq!(parsed.get("interval_secs").unwrap().as_u64().unwrap(), 3600);
}

#[test]
fn test_scan_one_shot_disables_after_run() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let schedule = Schedule {
        id: "sched-oneshot-1".to_string(),
        description: "One-shot task".to_string(),
        schedule_type: ScheduleType::Once,
        interval_secs: 600,
        cron_expression: None,
        next_run: past.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: past.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: true,
    };

    write_schedule(&workspace, &schedule);

    // Simulate what scan_schedules does for one-shot
    let mut loaded = read_schedule(&workspace, "sched-oneshot-1");
    assert!(loaded.enabled);
    assert!(loaded.one_shot);

    // After firing: disable
    loaded.last_run = Some(chrono::Utc::now().to_rfc3339());
    loaded.run_count += 1;
    loaded.enabled = false; // one_shot → disabled
    write_schedule(&workspace, &loaded);

    let final_schedule = read_schedule(&workspace, "sched-oneshot-1");
    assert!(!final_schedule.enabled);
    assert_eq!(final_schedule.run_count, 1);
}

#[test]
fn test_scan_cron_schedule_computes_next_run() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let past = chrono::Utc::now() - chrono::Duration::seconds(60);
    let schedule = Schedule {
        id: "sched-cron-scan".to_string(),
        description: "Cron scan test".to_string(),
        schedule_type: ScheduleType::Cron,
        interval_secs: 0,
        cron_expression: Some("* * * * *".to_string()), // every minute
        next_run: past.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: past.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    // Simulate scan: compute next cron run
    let loaded = read_schedule(&workspace, "sched-cron-scan");
    let next = compute_next_cron_run(loaded.cron_expression.as_ref().unwrap()).unwrap();
    assert!(next > chrono::Utc::now());
    assert!(next <= chrono::Utc::now() + chrono::Duration::seconds(61));
}

#[test]
fn test_list_includes_schedule_type() {
    // Verify that list output includes the schedule_type field
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let now = chrono::Utc::now();
    let schedule = Schedule {
        id: "sched-list-type".to_string(),
        description: "Type in list".to_string(),
        schedule_type: ScheduleType::Cron,
        interval_secs: 0,
        cron_expression: Some("0 9 * * *".to_string()),
        next_run: now.to_rfc3339(),
        conversation_id: None,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: now.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot: false,
    };

    write_schedule(&workspace, &schedule);

    // Read back and verify JSON has schedule_type
    let loaded = read_schedule(&workspace, "sched-list-type");
    assert_eq!(loaded.schedule_type, ScheduleType::Cron);
    assert_eq!(loaded.cron_expression, Some("0 9 * * *".to_string()));
}

#[test]
fn test_backward_compat_old_schedule_json() {
    // Old schedule files without schedule_type/cron_expression/one_shot should deserialize
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let schedules_dir = workspace.join("schedules");
    std::fs::create_dir_all(&schedules_dir).unwrap();

    let old_json = r#"{
        "id": "sched-legacy",
        "description": "Old-format schedule",
        "interval_secs": 7200,
        "next_run": "2026-03-01T12:00:00Z",
        "created_by": "agent",
        "enabled": true,
        "created_at": "2026-03-01T00:00:00Z",
        "run_count": 10
    }"#;

    let path = schedules_dir.join("sched-legacy.json");
    std::fs::write(&path, old_json).unwrap();

    let loaded = read_schedule(&workspace, "sched-legacy");
    assert_eq!(loaded.schedule_type, ScheduleType::Interval);
    assert_eq!(loaded.interval_secs, 7200);
    assert!(loaded.cron_expression.is_none());
    assert!(!loaded.one_shot);
    assert_eq!(loaded.run_count, 10);
}
