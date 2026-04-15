//! Schedule tools — create, list, and cancel recurring scheduled tasks.
//!
//! Schedules are JSON files in `workspace/schedules/`. The Scheduler
//! (heartbeat.rs) scans them on each tick and enqueues due tasks.
//!
//! Schedule types:
//! - **Interval** (default): fires every N seconds. Backward-compatible with existing schedules.
//! - **Cron**: fires at times matching a 5-field cron expression (via `croner` crate).
//! - **Once**: fires once, then auto-disables.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::str::FromStr;

use crate::tools::registry::ToolDef;

/// Schedule type — controls how `next_run` is computed after each execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleType {
    /// Fixed interval: next_run = last_run + interval_secs. Default for backward compat.
    #[default]
    Interval,
    /// Cron expression: next_run computed from `cron_expression` field.
    Cron,
    /// One-shot: disabled automatically after first successful run.
    Once,
}

/// A single execution record for a schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRun {
    pub task_id: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub status: String, // "triggered", "completed", "failed"
    pub sats_spent: u64,
    pub iterations: u32,
}

/// A recurring schedule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub description: String,
    /// Schedule type. Default: "interval" for backward compatibility.
    #[serde(default)]
    pub schedule_type: ScheduleType,
    /// Seconds between runs (for type=interval). Required when type=interval.
    #[serde(default)]
    pub interval_secs: u64,
    /// Cron expression (for type=cron, e.g. "0 9 * * MON"). Required when type=cron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    pub next_run: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub created_by: String,
    pub enabled: bool,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    pub run_count: u32,
    /// Recent execution history (capped at 50 entries).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_history: Vec<ScheduleRun>,
    /// If true, the schedule is disabled after first run (type=once semantics).
    #[serde(default)]
    pub one_shot: bool,
}

/// Parse a human-friendly interval string to seconds.
///
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours), `d` (days).
/// Examples: "30s" -> 30, "5m" -> 300, "1h" -> 3600, "7d" -> 604800.
pub fn parse_interval(input: &str) -> Result<u64, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty interval string".to_string());
    }

    let (num_str, suffix) = if let Some(s) = input.strip_suffix('d') {
        (s, "d")
    } else if let Some(s) = input.strip_suffix('h') {
        (s, "h")
    } else if let Some(s) = input.strip_suffix('m') {
        (s, "m")
    } else if let Some(s) = input.strip_suffix('s') {
        (s, "s")
    } else {
        return Err(format!(
            "invalid interval '{input}': must end with s, m, h, or d"
        ));
    };

    let num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in interval '{input}'"))?;

    if num == 0 {
        return Err("interval must be greater than zero".to_string());
    }

    let secs = match suffix {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => unreachable!(),
    };

    Ok(secs)
}

/// Validate a cron expression and compute the next occurrence after now.
///
/// Uses the `croner` crate for 5-field cron parsing (min hour dom month dow).
pub fn compute_next_cron_run(expr: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let cron = croner::Cron::from_str(expr)
        .map_err(|e| format!("invalid cron expression '{expr}': {e}"))?;
    let now = chrono::Utc::now();
    cron.find_next_occurrence(&now, false)
        .map_err(|e| format!("no upcoming cron occurrence for '{expr}': {e}"))
}

/// Return all schedule tools.
pub fn all_schedule_tools(workspace: PathBuf) -> Vec<ToolDef> {
    vec![
        create_schedule_tool(workspace.clone()),
        list_schedules_tool(workspace.clone()),
        cancel_schedule_tool(workspace),
    ]
}

fn create_schedule_tool(workspace: PathBuf) -> ToolDef {
    ToolDef {
        name: "create_schedule".to_string(),
        description: "Create a scheduled task. Three types: 'interval' (default, repeats every N \
                       seconds), 'cron' (5-field cron expression like '0 9 * * MON'), 'once' (fires \
                       once after a delay then auto-disables)."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "The task description to run on each scheduled execution"
                },
                "schedule_type": {
                    "type": "string",
                    "enum": ["interval", "cron", "once"],
                    "description": "Schedule type: 'interval' (default), 'cron', or 'once'"
                },
                "interval": {
                    "type": "string",
                    "description": "How often to run, for interval/once types (e.g. '30m', '1h', '24h', '7d')"
                },
                "cron_expression": {
                    "type": "string",
                    "description": "5-field cron expression for cron type (e.g. '0 9 * * MON', '*/5 * * * *')"
                },
                "conversation_id": {
                    "type": "string",
                    "description": "Optional conversation ID to join on each run"
                }
            },
            "required": ["description"]
        }),
        execute: Box::new(move |params| {
            let ws = workspace.clone();
            Box::pin(async move { create_schedule(params, ws).await })
        }),
        category: "schedule".to_string(),
        cleanup: None,
    deferred: true,
    always_load: false,
    search_hint: Some("Create a scheduled recurring task".to_string()),
    }
}

fn list_schedules_tool(workspace: PathBuf) -> ToolDef {
    ToolDef {
        name: "list_schedules".to_string(),
        description: "List all scheduled tasks with their status, next run time, and run count."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {}
        }),
        execute: Box::new(move |_params| {
            let ws = workspace.clone();
            Box::pin(async move { list_schedules(ws).await })
        }),
        category: "schedule".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("List all scheduled tasks".to_string()),
    }
}

fn cancel_schedule_tool(workspace: PathBuf) -> ToolDef {
    ToolDef {
        name: "cancel_schedule".to_string(),
        description: "Cancel (disable) a scheduled task. The schedule is preserved for history \
                       but will no longer trigger."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The schedule ID to cancel (e.g. 'sched-...')"
                }
            },
            "required": ["id"]
        }),
        execute: Box::new(move |params| {
            let ws = workspace.clone();
            Box::pin(async move { cancel_schedule(params, ws).await })
        }),
        category: "schedule".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("Cancel a scheduled task".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn create_schedule(params: Value, workspace: PathBuf) -> String {
    let description = match params.get("description").and_then(|v| v.as_str()) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return "Error: description is required".to_string(),
    };

    // Parse schedule type (default: interval for backward compat)
    let schedule_type = match params.get("schedule_type").and_then(|v| v.as_str()) {
        Some("cron") => ScheduleType::Cron,
        Some("once") => ScheduleType::Once,
        Some("interval") | None => ScheduleType::Interval,
        Some(other) => {
            return format!(
                "Error: invalid schedule_type '{other}': must be interval, cron, or once"
            )
        }
    };

    let conversation_id = params
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let now = chrono::Utc::now();
    let id = format!("sched-{}", uuid::Uuid::new_v4());

    // Compute fields based on schedule type
    let (interval_secs, cron_expression, next_run_str, one_shot) = match schedule_type {
        ScheduleType::Interval => {
            let interval_str = match params.get("interval").and_then(|v| v.as_str()) {
                Some(i) => i,
                None => return "Error: interval is required for type=interval".to_string(),
            };
            let secs = match parse_interval(interval_str) {
                Ok(s) => s,
                Err(e) => return format!("Error: {e}"),
            };
            let next = now + chrono::Duration::seconds(secs as i64);
            (secs, None, next.to_rfc3339(), false)
        }
        ScheduleType::Cron => {
            let expr = match params.get("cron_expression").and_then(|v| v.as_str()) {
                Some(e) if !e.is_empty() => e,
                _ => return "Error: cron_expression is required for type=cron".to_string(),
            };
            let next = match compute_next_cron_run(expr) {
                Ok(t) => t,
                Err(e) => return format!("Error: {e}"),
            };
            (0, Some(expr.to_string()), next.to_rfc3339(), false)
        }
        ScheduleType::Once => {
            let interval_str = match params.get("interval").and_then(|v| v.as_str()) {
                Some(i) => i,
                None => return "Error: interval (delay) is required for type=once".to_string(),
            };
            let secs = match parse_interval(interval_str) {
                Ok(s) => s,
                Err(e) => return format!("Error: {e}"),
            };
            let next = now + chrono::Duration::seconds(secs as i64);
            (secs, None, next.to_rfc3339(), true)
        }
    };

    let schedule = Schedule {
        id: id.clone(),
        description: description.clone(),
        schedule_type: schedule_type.clone(),
        interval_secs,
        cron_expression: cron_expression.clone(),
        next_run: next_run_str.clone(),
        conversation_id,
        created_by: "agent".to_string(),
        enabled: true,
        created_at: now.to_rfc3339(),
        last_run: None,
        run_count: 0,
        run_history: vec![],
        one_shot,
    };

    let schedules_dir = workspace.join("schedules");
    if let Err(e) = std::fs::create_dir_all(&schedules_dir) {
        return format!("Error: failed to create schedules directory: {e}");
    }

    let path = schedules_dir.join(format!("{id}.json"));
    let content = match serde_json::to_string_pretty(&schedule) {
        Ok(c) => c,
        Err(e) => return format!("Error: failed to serialize schedule: {e}"),
    };

    if let Err(e) = std::fs::write(&path, content) {
        return format!("Error: failed to write schedule file: {e}");
    }

    let mut result = json!({
        "id": id,
        "description": description,
        "schedule_type": schedule_type,
        "next_run": next_run_str
    });
    if interval_secs > 0 {
        result["interval_secs"] = json!(interval_secs);
    }
    if let Some(ref expr) = cron_expression {
        result["cron_expression"] = json!(expr);
    }
    result.to_string()
}

async fn list_schedules(workspace: PathBuf) -> String {
    let schedules_dir = workspace.join("schedules");

    let entries = match std::fs::read_dir(&schedules_dir) {
        Ok(e) => e,
        Err(_) => return json!([]).to_string(),
    };

    let mut schedules: Vec<Value> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let schedule: Schedule = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let mut entry = json!({
            "id": schedule.id,
            "description": schedule.description,
            "schedule_type": schedule.schedule_type,
            "enabled": schedule.enabled,
            "next_run": schedule.next_run,
            "run_count": schedule.run_count,
            "last_run": schedule.last_run,
            "conversation_id": schedule.conversation_id,
        });
        if schedule.interval_secs > 0 {
            entry["interval_secs"] = json!(schedule.interval_secs);
        }
        if let Some(ref expr) = schedule.cron_expression {
            entry["cron_expression"] = json!(expr);
        }
        schedules.push(entry);
    }

    // Sort by next_run for consistent output
    schedules.sort_by(|a, b| {
        let a_next = a.get("next_run").and_then(|v| v.as_str()).unwrap_or("");
        let b_next = b.get("next_run").and_then(|v| v.as_str()).unwrap_or("");
        a_next.cmp(b_next)
    });

    json!(schedules).to_string()
}

async fn cancel_schedule(params: Value, workspace: PathBuf) -> String {
    let id = match params.get("id").and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() => id,
        _ => return "Error: id is required".to_string(),
    };

    let schedules_dir = workspace.join("schedules");
    let path = schedules_dir.join(format!("{id}.json"));

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return format!("Error: schedule not found: {id}"),
    };

    let mut schedule: Schedule = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => return format!("Error: failed to parse schedule: {e}"),
    };

    schedule.enabled = false;

    let updated = match serde_json::to_string_pretty(&schedule) {
        Ok(c) => c,
        Err(e) => return format!("Error: failed to serialize schedule: {e}"),
    };

    if let Err(e) = std::fs::write(&path, updated) {
        return format!("Error: failed to write schedule: {e}");
    }

    json!({
        "id": id,
        "cancelled": true
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_parse_interval_whitespace() {
        assert_eq!(parse_interval("  1h  ").unwrap(), 3600);
    }

    #[test]
    fn test_parse_interval_invalid_suffix() {
        assert!(parse_interval("10x").is_err());
        assert!(parse_interval("10").is_err());
    }

    #[test]
    fn test_parse_interval_invalid_number() {
        assert!(parse_interval("abch").is_err());
    }

    #[test]
    fn test_parse_interval_empty() {
        assert!(parse_interval("").is_err());
    }

    #[test]
    fn test_parse_interval_zero() {
        assert!(parse_interval("0h").is_err());
        assert!(parse_interval("0m").is_err());
    }

    #[test]
    fn test_schedule_serde_roundtrip() {
        let now = chrono::Utc::now();
        let schedule = Schedule {
            id: "sched-test-123".to_string(),
            description: "Test task".to_string(),
            schedule_type: ScheduleType::Interval,
            interval_secs: 3600,
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

        let json_str = serde_json::to_string(&schedule).unwrap();
        let parsed: Schedule = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.id, "sched-test-123");
        assert_eq!(parsed.description, "Test task");
        assert_eq!(parsed.schedule_type, ScheduleType::Interval);
        assert_eq!(parsed.interval_secs, 3600);
        assert!(parsed.enabled);
        assert_eq!(parsed.run_count, 0);
        assert!(parsed.last_run.is_none());
        assert_eq!(parsed.conversation_id, Some("conv-abc".to_string()));
        assert!(!parsed.one_shot);
    }

    #[test]
    fn test_schedule_serde_no_conversation_id() {
        let now = chrono::Utc::now();
        let schedule = Schedule {
            id: "sched-test-456".to_string(),
            description: "No conversation".to_string(),
            schedule_type: ScheduleType::Interval,
            interval_secs: 86400,
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
        // conversation_id should be omitted (skip_serializing_if)
        assert!(!json_str.contains("conversation_id"));

        let parsed: Schedule = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.conversation_id.is_none());
    }

    #[test]
    fn test_schedule_serde_cron_type() {
        let now = chrono::Utc::now();
        let schedule = Schedule {
            id: "sched-cron-1".to_string(),
            description: "Monday morning report".to_string(),
            schedule_type: ScheduleType::Cron,
            interval_secs: 0,
            cron_expression: Some("0 9 * * MON".to_string()),
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

        let json_str = serde_json::to_string(&schedule).unwrap();
        assert!(json_str.contains("\"cron\""));
        assert!(json_str.contains("0 9 * * MON"));

        let parsed: Schedule = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.schedule_type, ScheduleType::Cron);
        assert_eq!(parsed.cron_expression, Some("0 9 * * MON".to_string()));
    }

    #[test]
    fn test_schedule_serde_once_type() {
        let now = chrono::Utc::now();
        let schedule = Schedule {
            id: "sched-once-1".to_string(),
            description: "One-time reminder".to_string(),
            schedule_type: ScheduleType::Once,
            interval_secs: 3600,
            cron_expression: None,
            next_run: now.to_rfc3339(),
            conversation_id: None,
            created_by: "agent".to_string(),
            enabled: true,
            created_at: now.to_rfc3339(),
            last_run: None,
            run_count: 0,
            run_history: vec![],
            one_shot: true,
        };

        let json_str = serde_json::to_string(&schedule).unwrap();
        assert!(json_str.contains("\"once\""));
        assert!(json_str.contains("\"one_shot\":true"));

        let parsed: Schedule = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.schedule_type, ScheduleType::Once);
        assert!(parsed.one_shot);
    }

    #[test]
    fn test_schedule_backward_compat_no_type_field() {
        // Old schedules without schedule_type/cron_expression/one_shot should deserialize
        let json_str = r#"{
            "id": "sched-old",
            "description": "Legacy schedule",
            "interval_secs": 3600,
            "next_run": "2026-03-01T00:00:00Z",
            "created_by": "agent",
            "enabled": true,
            "created_at": "2026-03-01T00:00:00Z",
            "run_count": 5
        }"#;

        let parsed: Schedule = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.schedule_type, ScheduleType::Interval); // default
        assert!(parsed.cron_expression.is_none());
        assert!(!parsed.one_shot);
        assert_eq!(parsed.interval_secs, 3600);
    }

    #[test]
    fn test_compute_next_cron_run_valid() {
        // "every minute" should have a next occurrence within 60 seconds
        let next = compute_next_cron_run("* * * * *").unwrap();
        let now = chrono::Utc::now();
        assert!(next > now);
        assert!(next <= now + chrono::Duration::seconds(61));
    }

    #[test]
    fn test_compute_next_cron_run_invalid() {
        assert!(compute_next_cron_run("not a cron").is_err());
        assert!(compute_next_cron_run("").is_err());
    }

    #[test]
    fn test_schedule_type_default() {
        assert_eq!(ScheduleType::default(), ScheduleType::Interval);
    }

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
}
