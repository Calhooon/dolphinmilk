//! Tests for heartbeat daemon and scheduler — config, construction, priority inbox, and poll behavior.

use std::path::PathBuf;
use std::time::Duration;

use dolphin_milk::config::{ActiveHours, DmConfig, HeartbeatConfig};
use dolphin_milk::heartbeat::{
    check_sender_trust, estimate_proofs_per_day, is_within_active_hours_at,
    read_heartbeat_checklist, should_respond_to_message, HeartbeatDaemon, InboxItem, PriorityInbox,
    SystemEvent, TaskPriority, TrustTier, WakeReason, WakeRequest, HEARTBEAT_CONVERSATION_ID,
};

// -----------------------------------------------------------------------
// Config tests
// -----------------------------------------------------------------------

#[test]
fn test_heartbeat_config_defaults() {
    let hb = HeartbeatConfig::default();
    assert!(hb.enabled);
    assert_eq!(hb.inbox_poll_secs, 60);
    assert_eq!(hb.max_concurrent_tasks, 3);
}

#[test]
fn test_heartbeat_config_from_toml() {
    let toml_str = r#"
[heartbeat]
enabled = false
inbox_poll_secs = 120
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 120);
}

#[test]
fn test_heartbeat_config_partial_toml() {
    // Only set enabled, inbox_poll_secs should default
    let toml_str = r#"
[heartbeat]
enabled = false
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 60);
}

#[test]
fn test_heartbeat_config_missing_section() {
    // No heartbeat section at all — should use defaults
    let toml_str = r#"
[wallet]
url = "http://localhost:3322"
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 60);
}

#[test]
fn test_worm_config_default_has_heartbeat() {
    let cfg = DmConfig::default();
    assert!(cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 60);
}

#[test]
fn test_heartbeat_config_max_concurrent_default() {
    let hb = HeartbeatConfig::default();
    assert_eq!(hb.max_concurrent_tasks, 3);
}

#[test]
fn test_heartbeat_config_max_concurrent_from_toml() {
    let toml_str = r#"
[heartbeat]
max_concurrent_tasks = 10
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.heartbeat.max_concurrent_tasks, 10);
}

#[test]
fn test_heartbeat_config_max_concurrent_with_other_fields() {
    let toml_str = r#"
[heartbeat]
enabled = true
inbox_poll_secs = 15
max_concurrent_tasks = 7
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 15);
    assert_eq!(cfg.heartbeat.max_concurrent_tasks, 7);
}

// -----------------------------------------------------------------------
// Daemon construction tests (backward compat)
// -----------------------------------------------------------------------

#[test]
fn test_heartbeat_daemon_creation() {
    let config = DmConfig::default();
    let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
    assert!(daemon.is_enabled());
    assert_eq!(daemon.poll_interval(), Duration::from_secs(60));
}

#[test]
fn test_poll_interval_matches_config() {
    let mut config = DmConfig::default();
    config.heartbeat.inbox_poll_secs = 30;
    let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
    assert_eq!(daemon.poll_interval(), Duration::from_secs(30));
}

#[test]
fn test_heartbeat_disabled_config() {
    let mut config = DmConfig::default();
    config.heartbeat.enabled = false;
    let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
    assert!(!daemon.is_enabled());
}

#[test]
fn test_heartbeat_custom_poll_interval() {
    let mut config = DmConfig::default();
    config.heartbeat.inbox_poll_secs = 5;
    let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
    assert_eq!(daemon.poll_interval(), Duration::from_secs(5));
}

#[test]
fn test_heartbeat_large_poll_interval() {
    let mut config = DmConfig::default();
    config.heartbeat.inbox_poll_secs = 3600; // hourly
    let daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
    assert_eq!(daemon.poll_interval(), Duration::from_secs(3600));
}

// -----------------------------------------------------------------------
// Channel / async tests (backward compat)
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_heartbeat_poll_once_no_wallet() {
    // When no wallet is running, poll_once should handle the error gracefully
    // (no panic, just logs the failure).
    let mut config = DmConfig::default();
    // Point at a port where nothing is listening
    config.wallet.url = "http://127.0.0.1:19999".to_string();
    config.wallet.timeout = 2;

    let mut daemon = HeartbeatDaemon::new(config, PathBuf::from("test-workspace"));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);

    // poll_once should not panic when the wallet/messagebox is unreachable
    daemon.poll_once(&tx).await;

    // No messages should have been sent through the channel
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_heartbeat_channel_capacity() {
    // Verify that the channel is created and can send/receive
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);

    tx.send("test task 1".into()).await.unwrap();
    tx.send("test task 2".into()).await.unwrap();

    assert_eq!(rx.recv().await.unwrap(), "test task 1");
    assert_eq!(rx.recv().await.unwrap(), "test task 2");
}

#[tokio::test]
async fn test_heartbeat_sender_dropped_closes_receiver() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);

    tx.send("task".into()).await.unwrap();
    drop(tx);

    // Should receive the last message then None
    assert_eq!(rx.recv().await.unwrap(), "task");
    assert!(rx.recv().await.is_none());
}

// -----------------------------------------------------------------------
// Priority inbox tests
// -----------------------------------------------------------------------

fn make_item(msg: &str, priority: TaskPriority, offset_millis: i64) -> InboxItem {
    InboxItem {
        message: msg.to_string(),
        priority,
        session_id: None,
        model_override: None,
        submitted_at: chrono::Utc::now() + chrono::Duration::milliseconds(offset_millis),
        reply_to: None,
        max_iterations: None,
        origin: "test".to_string(),
        delegation_envelope: None,
    }
}

#[test]
fn test_priority_inbox_empty() {
    let inbox = PriorityInbox::new();
    assert!(inbox.is_empty());
    assert_eq!(inbox.len(), 0);
}

#[test]
fn test_priority_inbox_push_pop_single() {
    let mut inbox = PriorityInbox::new();
    inbox.push(make_item("task-a", TaskPriority::Normal, 0));
    assert_eq!(inbox.len(), 1);
    assert!(!inbox.is_empty());
    let item = inbox.pop().unwrap();
    assert_eq!(item.message, "task-a");
    assert!(inbox.is_empty());
}

#[test]
fn test_priority_inbox_higher_priority_first() {
    let mut inbox = PriorityInbox::new();
    inbox.push(make_item("low", TaskPriority::Low, 0));
    inbox.push(make_item("highest", TaskPriority::Highest, 10));
    inbox.push(make_item("normal", TaskPriority::Normal, 5));

    assert_eq!(inbox.pop().unwrap().message, "highest");
    assert_eq!(inbox.pop().unwrap().message, "normal");
    assert_eq!(inbox.pop().unwrap().message, "low");
    assert!(inbox.is_empty());
}

#[test]
fn test_priority_inbox_fifo_within_same_priority() {
    let mut inbox = PriorityInbox::new();
    inbox.push(make_item("first", TaskPriority::Normal, 0));
    inbox.push(make_item("second", TaskPriority::Normal, 10));
    inbox.push(make_item("third", TaskPriority::Normal, 20));

    assert_eq!(inbox.pop().unwrap().message, "first");
    assert_eq!(inbox.pop().unwrap().message, "second");
    assert_eq!(inbox.pop().unwrap().message, "third");
}

#[test]
fn test_priority_inbox_all_four_levels() {
    let mut inbox = PriorityInbox::new();
    inbox.push(make_item("lowest", TaskPriority::Lowest, 0));
    inbox.push(make_item("low", TaskPriority::Low, 0));
    inbox.push(make_item("normal", TaskPriority::Normal, 0));
    inbox.push(make_item("highest", TaskPriority::Highest, 0));

    assert_eq!(inbox.pop().unwrap().message, "highest");
    assert_eq!(inbox.pop().unwrap().message, "normal");
    assert_eq!(inbox.pop().unwrap().message, "low");
    assert_eq!(inbox.pop().unwrap().message, "lowest");
}

#[test]
fn test_priority_inbox_pop_empty_returns_none() {
    let mut inbox = PriorityInbox::new();
    assert!(inbox.pop().is_none());
}

#[test]
fn test_priority_inbox_interleaved_priorities() {
    let mut inbox = PriorityInbox::new();
    // Two normal, one low, one highest — interleaved
    inbox.push(make_item("normal-1", TaskPriority::Normal, 0));
    inbox.push(make_item("low-1", TaskPriority::Low, 5));
    inbox.push(make_item("highest-1", TaskPriority::Highest, 10));
    inbox.push(make_item("normal-2", TaskPriority::Normal, 15));

    assert_eq!(inbox.pop().unwrap().message, "highest-1");
    assert_eq!(inbox.pop().unwrap().message, "normal-1");
    assert_eq!(inbox.pop().unwrap().message, "normal-2");
    assert_eq!(inbox.pop().unwrap().message, "low-1");
}

// -----------------------------------------------------------------------
// TaskPriority ordering tests
// -----------------------------------------------------------------------

#[test]
fn test_task_priority_ordering() {
    assert!(TaskPriority::Highest > TaskPriority::Normal);
    assert!(TaskPriority::Normal > TaskPriority::Low);
    assert!(TaskPriority::Low > TaskPriority::Lowest);
}

#[test]
fn test_task_priority_equality() {
    assert_eq!(TaskPriority::Normal, TaskPriority::Normal);
    assert_ne!(TaskPriority::Highest, TaskPriority::Lowest);
}

// -----------------------------------------------------------------------
// Phase 6: MessageBox integration tests
// -----------------------------------------------------------------------

#[test]
fn test_inbox_item_reply_to_field() {
    let item = InboxItem {
        message: "Process inbox message".to_string(),
        priority: TaskPriority::Normal,
        session_id: None,
        model_override: None,
        submitted_at: chrono::Utc::now(),
        reply_to: Some(
            "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".to_string(),
        ),
        max_iterations: None,
        origin: "message".to_string(),
        delegation_envelope: None,
    };
    assert_eq!(
        item.reply_to.as_deref(),
        Some("034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce")
    );
}

#[test]
fn test_inbox_item_reply_to_none() {
    let item = make_item("continuation task", TaskPriority::Low, 0);
    assert!(item.reply_to.is_none());
}

#[test]
fn test_inbox_item_with_session_and_reply() {
    let item = InboxItem {
        message: "Parent message".to_string(),
        priority: TaskPriority::Highest,
        session_id: Some("conv-12345".to_string()),
        model_override: None,
        submitted_at: chrono::Utc::now(),
        reply_to: Some("parent-key-hex".to_string()),
        max_iterations: None,
        origin: "message".to_string(),
        delegation_envelope: None,
    };
    assert_eq!(item.priority, TaskPriority::Highest);
    assert_eq!(item.session_id.as_deref(), Some("conv-12345"));
    assert_eq!(item.reply_to.as_deref(), Some("parent-key-hex"));
}

#[test]
fn test_parent_message_gets_highest_priority() {
    // This tests the expected behavior: parent messages should get Highest priority.
    // The actual detection happens in poll_messagebox(), but we can verify the
    // priority works correctly in the inbox.
    let mut inbox = PriorityInbox::new();

    // Normal message from unknown sender
    let normal = InboxItem {
        message: "Normal agent message".to_string(),
        priority: TaskPriority::Normal,
        session_id: None,
        model_override: None,
        submitted_at: chrono::Utc::now(),
        reply_to: Some("unknown-agent-key".to_string()),
        max_iterations: None,
        origin: "message".to_string(),
        delegation_envelope: None,
    };

    // Parent message (Highest priority)
    let parent = InboxItem {
        message: "Parent message".to_string(),
        priority: TaskPriority::Highest,
        session_id: None,
        model_override: None,
        submitted_at: chrono::Utc::now() + chrono::Duration::milliseconds(100),
        reply_to: Some("parent-identity-key".to_string()),
        max_iterations: None,
        origin: "message".to_string(),
        delegation_envelope: None,
    };

    // Push normal first, then parent
    inbox.push(normal);
    inbox.push(parent);

    // Parent should be dequeued first despite being submitted later
    let first = inbox.pop().unwrap();
    assert_eq!(first.priority, TaskPriority::Highest);
    assert_eq!(first.reply_to.as_deref(), Some("parent-identity-key"));

    let second = inbox.pop().unwrap();
    assert_eq!(second.priority, TaskPriority::Normal);
    assert_eq!(second.reply_to.as_deref(), Some("unknown-agent-key"));
}

// -----------------------------------------------------------------------
// Active hours tests
// -----------------------------------------------------------------------

fn make_utc(hour: u32, minute: u32) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(2026, 2, 27, hour, minute, 0)
        .unwrap()
}

#[test]
fn test_active_hours_within_window() {
    let ah = ActiveHours {
        start: Some("09:00".to_string()),
        end: Some("22:00".to_string()),
        timezone: None, // UTC
    };
    // 14:00 UTC is within 09:00-22:00
    assert!(is_within_active_hours_at(&ah, make_utc(14, 0)));
    // 09:00 exactly is within (inclusive start)
    assert!(is_within_active_hours_at(&ah, make_utc(9, 0)));
}

#[test]
fn test_active_hours_outside_window() {
    let ah = ActiveHours {
        start: Some("09:00".to_string()),
        end: Some("22:00".to_string()),
        timezone: None,
    };
    // 03:00 UTC is outside 09:00-22:00
    assert!(!is_within_active_hours_at(&ah, make_utc(3, 0)));
    // 22:00 exactly is outside (exclusive end)
    assert!(!is_within_active_hours_at(&ah, make_utc(22, 0)));
    // 23:59 is outside
    assert!(!is_within_active_hours_at(&ah, make_utc(23, 59)));
}

#[test]
fn test_active_hours_overnight_range() {
    let ah = ActiveHours {
        start: Some("22:00".to_string()),
        end: Some("06:00".to_string()),
        timezone: None,
    };
    // 23:00 is within (after start)
    assert!(is_within_active_hours_at(&ah, make_utc(23, 0)));
    // 02:00 is within (before end)
    assert!(is_within_active_hours_at(&ah, make_utc(2, 0)));
    // 12:00 is outside
    assert!(!is_within_active_hours_at(&ah, make_utc(12, 0)));
    // 06:00 exactly is outside (exclusive end)
    assert!(!is_within_active_hours_at(&ah, make_utc(6, 0)));
}

#[test]
fn test_active_hours_not_configured() {
    // No start/end = always active
    let ah = ActiveHours::default();
    assert!(is_within_active_hours_at(&ah, make_utc(3, 0)));
    assert!(is_within_active_hours_at(&ah, make_utc(14, 0)));

    // Only start, no end = always active
    let ah2 = ActiveHours {
        start: Some("09:00".to_string()),
        end: None,
        timezone: None,
    };
    assert!(is_within_active_hours_at(&ah2, make_utc(3, 0)));
}

#[test]
fn test_active_hours_invalid_timezone() {
    let ah = ActiveHours {
        start: Some("09:00".to_string()),
        end: Some("22:00".to_string()),
        timezone: Some("Not/A/Timezone".to_string()),
    };
    // Invalid timezone = graceful degradation (returns true)
    assert!(is_within_active_hours_at(&ah, make_utc(3, 0)));
}

#[test]
fn test_active_hours_invalid_time_format() {
    // Invalid start time
    let ah = ActiveHours {
        start: Some("9am".to_string()),
        end: Some("22:00".to_string()),
        timezone: None,
    };
    assert!(is_within_active_hours_at(&ah, make_utc(3, 0)));

    // Invalid end time
    let ah2 = ActiveHours {
        start: Some("09:00".to_string()),
        end: Some("10pm".to_string()),
        timezone: None,
    };
    assert!(is_within_active_hours_at(&ah2, make_utc(3, 0)));
}

#[test]
fn test_active_hours_config_from_toml() {
    let toml_str = r#"
[heartbeat]
enabled = true

[heartbeat.active_hours]
start = "09:00"
end = "22:00"
timezone = "America/New_York"
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.heartbeat.active_hours.start.as_deref(), Some("09:00"));
    assert_eq!(cfg.heartbeat.active_hours.end.as_deref(), Some("22:00"));
    assert_eq!(
        cfg.heartbeat.active_hours.timezone.as_deref(),
        Some("America/New_York")
    );
}

// -----------------------------------------------------------------------
// Heartbeat checklist tests
// -----------------------------------------------------------------------

#[test]
fn test_heartbeat_checklist_empty_file_skips() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("HEARTBEAT.md"), "").unwrap();
    let cfg = Some("HEARTBEAT.md".to_string());
    assert!(read_heartbeat_checklist(&cfg, dir.path()).is_none());
}

#[test]
fn test_heartbeat_checklist_headers_only_skips() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("HEARTBEAT.md"),
        "# Heartbeat Checklist\n\n## Section\n\n",
    )
    .unwrap();
    let cfg = Some("HEARTBEAT.md".to_string());
    assert!(read_heartbeat_checklist(&cfg, dir.path()).is_none());
}

#[test]
fn test_heartbeat_checklist_with_content_sends_task() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("HEARTBEAT.md"),
        "# Checks\n\n1. Check wallet balance\n2. Review continuations\n",
    )
    .unwrap();
    let cfg = Some("HEARTBEAT.md".to_string());
    let result = read_heartbeat_checklist(&cfg, dir.path());
    assert!(result.is_some());
    let task = result.unwrap();
    assert!(task.starts_with("HEARTBEAT_CHECK"));
    assert!(task.contains("Check wallet balance"));
    assert!(task.contains("Review continuations"));
}

#[test]
fn test_heartbeat_checklist_missing_file_skips() {
    let dir = tempfile::tempdir().unwrap();
    // No HEARTBEAT.md file
    let cfg = Some("HEARTBEAT.md".to_string());
    assert!(read_heartbeat_checklist(&cfg, dir.path()).is_none());
}

#[test]
fn test_heartbeat_checklist_disabled_config() {
    let dir = tempfile::tempdir().unwrap();
    // Write content but config is None
    std::fs::write(dir.path().join("HEARTBEAT.md"), "1. Check something\n").unwrap();
    assert!(read_heartbeat_checklist(&None, dir.path()).is_none());

    // Config set to "none"
    let cfg = Some("none".to_string());
    assert!(read_heartbeat_checklist(&cfg, dir.path()).is_none());

    // Config set to empty string
    let cfg2 = Some(String::new());
    assert!(read_heartbeat_checklist(&cfg2, dir.path()).is_none());
}

#[test]
fn test_heartbeat_checklist_config_default() {
    let hb = HeartbeatConfig::default();
    assert_eq!(hb.checklist_file.as_deref(), Some("HEARTBEAT.md"));
}

#[test]
fn test_heartbeat_checklist_config_from_toml() {
    let toml_str = r#"
[heartbeat]
checklist_file = "MY_CHECKS.md"
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(
        cfg.heartbeat.checklist_file.as_deref(),
        Some("MY_CHECKS.md")
    );
}

// -----------------------------------------------------------------------
// Skip-if-busy tests (Phase 4C)
// -----------------------------------------------------------------------

#[test]
fn test_skip_if_busy_at_capacity() {
    // When active_task_count >= max_concurrent_tasks, scheduler should be "at capacity"
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let count = Arc::new(AtomicUsize::new(3));
    let max_concurrent = 3;
    // At capacity (3/3)
    assert!(count.load(Ordering::Relaxed) >= max_concurrent);
    // Over capacity (4/3)
    count.store(4, Ordering::Relaxed);
    assert!(count.load(Ordering::Relaxed) >= max_concurrent);
}

#[test]
fn test_skip_if_busy_has_capacity() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let count = Arc::new(AtomicUsize::new(0));
    let max_concurrent = 3;
    // Empty — has capacity
    assert!(count.load(Ordering::Relaxed) < max_concurrent);
    // 2/3 — has capacity
    count.store(2, Ordering::Relaxed);
    assert!(count.load(Ordering::Relaxed) < max_concurrent);
}

// -----------------------------------------------------------------------
// Phase 7: Reflection config tests
// -----------------------------------------------------------------------

#[test]
fn test_reflection_config_defaults() {
    let cfg = HeartbeatConfig::default();
    assert!(!cfg.reflection_enabled);
    assert_eq!(cfg.reflection_interval_secs, 60);
    assert_eq!(cfg.reflection_model, "claude-haiku-4-5-20251001");
    assert_eq!(cfg.reflection_max_iterations, 3);
}

#[test]
fn test_reflection_config_from_toml() {
    let toml_str = r#"
[heartbeat]
reflection_enabled = true
reflection_interval_secs = 120
reflection_model = "gpt-4-turbo"
reflection_max_iterations = 5
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.heartbeat.reflection_enabled);
    assert_eq!(cfg.heartbeat.reflection_interval_secs, 120);
    assert_eq!(cfg.heartbeat.reflection_model, "gpt-4-turbo");
    assert_eq!(cfg.heartbeat.reflection_max_iterations, 5);
}

#[test]
fn test_inbox_item_max_iterations_field() {
    let item = InboxItem {
        message: "Reflection task".to_string(),
        priority: TaskPriority::Lowest,
        session_id: Some("reflection".to_string()),
        model_override: Some("claude-haiku-4-5-20251001".to_string()),
        submitted_at: chrono::Utc::now(),
        reply_to: None,
        max_iterations: Some(3),
        origin: "reflection".to_string(),
        delegation_envelope: None,
    };
    assert_eq!(item.max_iterations, Some(3));
    assert_eq!(item.priority, TaskPriority::Lowest);
    assert_eq!(item.session_id.as_deref(), Some("reflection"));
}

#[test]
fn test_inbox_item_max_iterations_none_default() {
    let item = make_item("normal task", TaskPriority::Normal, 0);
    assert!(item.max_iterations.is_none());
}

// -----------------------------------------------------------------------
// Phase 8B: Turn-taking protocol tests
// -----------------------------------------------------------------------

#[test]
fn test_should_respond_unstructured_message() {
    // Plain text body — always respond
    let body = serde_json::json!("Hello, can you help me?");
    assert!(should_respond_to_message(&body));
}

#[test]
fn test_should_respond_non_agent_message() {
    // JSON object but not an agent_message — always respond
    let body = serde_json::json!({"task": "do something", "data": 42});
    assert!(should_respond_to_message(&body));
}

#[test]
fn test_should_respond_agent_message_within_limit() {
    let body = serde_json::json!({
        "type": "agent_message",
        "turn": 2,
        "max_turns": 5,
        "done": false,
        "body": {"content": "still chatting"}
    });
    assert!(should_respond_to_message(&body));
}

#[test]
fn test_should_respond_max_turns_reached() {
    let body = serde_json::json!({
        "type": "agent_message",
        "turn": 5,
        "max_turns": 5,
        "done": false,
        "body": {"content": "one more thing"}
    });
    assert!(!should_respond_to_message(&body));
}

#[test]
fn test_should_respond_done_signal() {
    let body = serde_json::json!({
        "type": "agent_message",
        "turn": 1,
        "max_turns": 5,
        "done": true,
        "body": {"content": "goodbye"}
    });
    assert!(!should_respond_to_message(&body));
}

#[test]
fn test_should_respond_default_max_turns() {
    // No explicit max_turns — defaults to 5
    let body = serde_json::json!({
        "type": "agent_message",
        "turn": 4,
        "body": {"content": "turn 4 of default 5"}
    });
    assert!(should_respond_to_message(&body)); // 4 < 5

    let body_at_limit = serde_json::json!({
        "type": "agent_message",
        "turn": 5,
        "body": {"content": "turn 5 of default 5"}
    });
    assert!(!should_respond_to_message(&body_at_limit)); // 5 >= 5
}

// -----------------------------------------------------------------------
// Heartbeat-in-Conversation (Phase 9)
// -----------------------------------------------------------------------

#[test]
fn test_heartbeat_conversation_constant() {
    // The dedicated system conversation ID must be stable
    assert_eq!(HEARTBEAT_CONVERSATION_ID, "conv-heartbeat");
}

#[test]
fn test_heartbeat_conversation_id_is_valid_conv_id() {
    // Must start with "conv-" to be recognized by ConversationManager
    assert!(HEARTBEAT_CONVERSATION_ID.starts_with("conv-"));
    // Must not contain path separators or whitespace
    assert!(!HEARTBEAT_CONVERSATION_ID.contains('/'));
    assert!(!HEARTBEAT_CONVERSATION_ID.contains('\\'));
    assert!(!HEARTBEAT_CONVERSATION_ID.contains(' '));
}

// -----------------------------------------------------------------------
// Phase 4: Wake coalescing tests
// -----------------------------------------------------------------------

#[test]
fn test_wake_reason_six_distinct_variants() {
    use std::collections::HashSet;
    let reasons = [
        WakeReason::MessageReceived,
        WakeReason::ContinuationReady,
        WakeReason::ScheduleDue,
        WakeReason::HeartbeatFile,
        WakeReason::ExternalTrigger,
        WakeReason::TimerExpired,
    ];
    let unique: HashSet<_> = reasons.iter().collect();
    assert_eq!(unique.len(), 6);
}

#[test]
fn test_wake_reason_clone_and_eq() {
    let r1 = WakeReason::ScheduleDue;
    let r2 = r1.clone();
    assert_eq!(r1, r2);
}

#[test]
fn test_wake_request_construction() {
    let req = WakeRequest {
        reason: WakeReason::MessageReceived,
        priority: 3,
        timestamp: std::time::Instant::now(),
    };
    assert_eq!(req.reason, WakeReason::MessageReceived);
    assert_eq!(req.priority, 3);
}

#[test]
fn test_wake_batch_dedup_preserves_first() {
    use std::collections::HashSet;
    let mut batch = vec![
        WakeRequest {
            reason: WakeReason::MessageReceived,
            priority: 3,
            timestamp: std::time::Instant::now(),
        },
        WakeRequest {
            reason: WakeReason::TimerExpired,
            priority: 1,
            timestamp: std::time::Instant::now(),
        },
        WakeRequest {
            reason: WakeReason::MessageReceived,
            priority: 2, // different priority but same reason
            timestamp: std::time::Instant::now(),
        },
    ];
    let mut seen = HashSet::new();
    batch.retain(|r| seen.insert(r.reason.clone()));

    // Duplicate MessageReceived removed, two remain
    assert_eq!(batch.len(), 2);
    // First occurrence kept (priority 3), second (priority 2) dropped
    assert_eq!(batch[0].priority, 3);
    assert_eq!(batch[1].reason, WakeReason::TimerExpired);
}

#[test]
fn test_wake_batch_sort_by_priority() {
    let mut batch = [
        WakeRequest {
            reason: WakeReason::TimerExpired,
            priority: 1,
            timestamp: std::time::Instant::now(),
        },
        WakeRequest {
            reason: WakeReason::ExternalTrigger,
            priority: 3,
            timestamp: std::time::Instant::now(),
        },
        WakeRequest {
            reason: WakeReason::ContinuationReady,
            priority: 0,
            timestamp: std::time::Instant::now(),
        },
    ];
    batch.sort_by_key(|r| r.priority);

    assert_eq!(batch[0].reason, WakeReason::ContinuationReady);
    assert_eq!(batch[1].reason, WakeReason::TimerExpired);
    assert_eq!(batch[2].reason, WakeReason::ExternalTrigger);
}

#[tokio::test]
async fn test_wake_channel_sends_and_receives() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<WakeRequest>(16);

    tx.send(WakeRequest {
        reason: WakeReason::MessageReceived,
        priority: 3,
        timestamp: std::time::Instant::now(),
    })
    .await
    .unwrap();

    let req = rx.recv().await.unwrap();
    assert_eq!(req.reason, WakeReason::MessageReceived);
}

#[tokio::test]
async fn test_wake_channel_dropped_sender_closes_receiver() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<WakeRequest>(16);
    drop(tx);
    assert!(rx.recv().await.is_none());
}

// -----------------------------------------------------------------------
// System event tests (Task 4.2)
// -----------------------------------------------------------------------

#[test]
fn test_system_event_task_completed_to_wake_request() {
    let evt = SystemEvent::TaskCompleted {
        task_id: "t-123".to_string(),
        status: "completed".to_string(),
        sats_spent: 500,
    };
    let wake = evt.to_wake_request();
    assert_eq!(wake.reason, WakeReason::ContinuationReady);
    assert_eq!(wake.priority, 3); // action priority
}

#[test]
fn test_system_event_schedule_fired_to_wake_request() {
    let evt = SystemEvent::ScheduleFired {
        schedule_id: "sched-1".to_string(),
    };
    let wake = evt.to_wake_request();
    assert_eq!(wake.reason, WakeReason::ScheduleDue);
    assert_eq!(wake.priority, 2);
}

#[test]
fn test_system_event_message_received_to_wake_request() {
    let evt = SystemEvent::MessageReceived {
        sender: "abc".to_string(),
        inbox: "general_inbox".to_string(),
    };
    let wake = evt.to_wake_request();
    assert_eq!(wake.reason, WakeReason::MessageReceived);
    assert_eq!(wake.priority, 3);
}

#[test]
fn test_system_event_config_changed_to_wake_request() {
    let evt = SystemEvent::ConfigChanged;
    let wake = evt.to_wake_request();
    assert_eq!(wake.reason, WakeReason::ExternalTrigger);
    assert_eq!(wake.priority, 1);
}

#[test]
fn test_system_event_continuation_ready_to_wake_request() {
    let evt = SystemEvent::ContinuationReady {
        task_id: "t-456".to_string(),
    };
    let wake = evt.to_wake_request();
    assert_eq!(wake.reason, WakeReason::ContinuationReady);
    assert_eq!(wake.priority, 3);
}

#[test]
fn test_system_event_all_five_variants_distinct() {
    let events: Vec<SystemEvent> = vec![
        SystemEvent::TaskCompleted {
            task_id: "t".to_string(),
            status: "ok".to_string(),
            sats_spent: 0,
        },
        SystemEvent::ScheduleFired {
            schedule_id: "s".to_string(),
        },
        SystemEvent::MessageReceived {
            sender: "a".to_string(),
            inbox: "b".to_string(),
        },
        SystemEvent::ConfigChanged,
        SystemEvent::ContinuationReady {
            task_id: "c".to_string(),
        },
    ];
    // All should produce valid WakeRequests
    for evt in &events {
        let wake = evt.to_wake_request();
        assert!(wake.priority <= 3);
    }
    assert_eq!(events.len(), 5);
}

#[tokio::test]
async fn test_system_event_channel_send_receive() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<SystemEvent>(16);

    tx.send(SystemEvent::TaskCompleted {
        task_id: "t-1".to_string(),
        status: "completed".to_string(),
        sats_spent: 1000,
    })
    .await
    .unwrap();

    tx.send(SystemEvent::ConfigChanged).await.unwrap();

    let evt1 = rx.recv().await.unwrap();
    assert!(matches!(evt1, SystemEvent::TaskCompleted { .. }));

    let evt2 = rx.recv().await.unwrap();
    assert!(matches!(evt2, SystemEvent::ConfigChanged));
}

#[tokio::test]
async fn test_system_event_try_send_on_full_channel() {
    // Channel capacity of 1
    let (tx, _rx) = tokio::sync::mpsc::channel::<SystemEvent>(1);

    // First send should succeed
    tx.try_send(SystemEvent::ConfigChanged).unwrap();

    // Second send to full channel should return error (not panic)
    let result = tx.try_send(SystemEvent::ConfigChanged);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Basket monitoring config tests
// -----------------------------------------------------------------------

#[test]
fn test_lifecycle_basket_monitoring_default() {
    let cfg = DmConfig::default();
    assert_eq!(cfg.lifecycle.basket_monitoring_interval_secs, 3600);
}

#[test]
fn test_lifecycle_basket_monitoring_from_toml() {
    let toml_str = r#"
[lifecycle]
basket_monitoring_interval_secs = 1800
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.lifecycle.basket_monitoring_interval_secs, 1800);
}

#[test]
fn test_lifecycle_basket_monitoring_disabled() {
    let toml_str = r#"
[lifecycle]
basket_monitoring_interval_secs = 0
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.lifecycle.basket_monitoring_interval_secs, 0);
}

// -----------------------------------------------------------------------
// Growth rate estimation tests
// -----------------------------------------------------------------------

#[test]
fn test_estimate_proofs_per_day_normal() {
    // 240 proofs over 48 hours = 120 proofs/day
    let rate = estimate_proofs_per_day(240, Some(48.0));
    assert!((rate.unwrap() - 120.0).abs() < 0.01);
}

#[test]
fn test_estimate_proofs_per_day_zero_count() {
    // 0 proofs = 0 proofs/day regardless of age
    let rate = estimate_proofs_per_day(0, Some(100.0));
    assert_eq!(rate.unwrap(), 0.0);
}

#[test]
fn test_estimate_proofs_per_day_no_age() {
    // No age data = None (can't estimate)
    let rate = estimate_proofs_per_day(100, None);
    assert!(rate.is_none());
}

#[test]
fn test_estimate_proofs_per_day_very_recent() {
    // Oldest proof is practically now — too recent to estimate
    let rate = estimate_proofs_per_day(5, Some(0.001));
    assert!(rate.is_none());
}

#[test]
fn test_estimate_proofs_per_day_one_hour() {
    // 10 proofs in 1 hour = 240 proofs/day
    let rate = estimate_proofs_per_day(10, Some(1.0));
    assert!((rate.unwrap() - 240.0).abs() < 0.01);
}

#[test]
fn test_estimate_proofs_per_day_zero_count_no_age() {
    // 0 proofs with no age = 0 proofs/day
    let rate = estimate_proofs_per_day(0, None);
    assert_eq!(rate.unwrap(), 0.0);
}

// -----------------------------------------------------------------------
// Message type filtering — non-actionable message types
// -----------------------------------------------------------------------

#[test]
fn test_should_respond_task_result_message() {
    // task_result is a delivery receipt — should NOT spawn a task
    let body = serde_json::json!({
        "type": "task_result",
        "task_id": "abc-123",
        "status": "completed",
        "result": "Paris"
    });
    assert!(!should_respond_to_message(&body));
}

#[test]
fn test_should_respond_task_result_failed() {
    // Failed task_result — especially should NOT spawn a task (this was the cascade bug)
    let body = serde_json::json!({
        "type": "task_result",
        "task_id": "abc-123",
        "status": "failed",
        "result": "Task failed: payment error: Insufficient funds"
    });
    assert!(!should_respond_to_message(&body));
}

#[test]
fn test_should_respond_status_update_message() {
    // status_update is an informational progress report — should NOT spawn a task
    let body = serde_json::json!({
        "type": "status_update",
        "task_id": "abc-123",
        "status": "running",
        "progress_pct": 50
    });
    assert!(!should_respond_to_message(&body));
}

#[test]
fn test_should_respond_coordination_message() {
    // coordination is peer metadata — should NOT spawn a task
    let body = serde_json::json!({
        "type": "coordination",
        "signal": "heartbeat",
        "sender_key": "02abcdef1234567890"
    });
    assert!(!should_respond_to_message(&body));
}

#[test]
fn test_should_respond_task_assignment_via_unstructured() {
    // task_assignment doesn't have special handling — falls through to the
    // unstructured/unknown path and returns true (backwards compat)
    let body = serde_json::json!({
        "type": "task_assignment",
        "task": "Research BSV fees",
        "budget_sats": 50000
    });
    assert!(should_respond_to_message(&body));
}

#[test]
fn test_should_respond_plain_json_no_type() {
    // JSON without a "type" field — backwards compatible, should be allowed
    let body = serde_json::json!({"question": "What is the capital of France?"});
    assert!(should_respond_to_message(&body));
}

#[test]
fn test_should_respond_plain_string() {
    // Plain string body — backwards compatible, should be allowed
    let body = serde_json::json!("Hello, can you help me with something?");
    assert!(should_respond_to_message(&body));
}

// -----------------------------------------------------------------------
// Trust tier checks
// -----------------------------------------------------------------------

#[test]
fn test_trust_tier_family_sender_is_parent() {
    // Sender IS our parent operator — Family tier
    let parent_key = "03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0";
    let sender_key = parent_key;
    let trust = check_sender_trust(sender_key, parent_key, &[]);
    assert_eq!(trust, TrustTier::Family);
}

#[test]
fn test_trust_tier_vouched_in_trusted_list() {
    // Sender is in our trusted_certifiers list — Vouched tier
    let parent_key = "03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0";
    let sender_key = "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce";
    let trusted = vec![sender_key.to_string()];
    let trust = check_sender_trust(sender_key, parent_key, &trusted);
    assert_eq!(trust, TrustTier::Vouched);
}

#[test]
fn test_trust_tier_unknown_no_cert_info() {
    // Sender not parent, not in trusted list — Unknown
    let parent_key = "03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0";
    let sender_key = "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce";
    let trust = check_sender_trust(sender_key, parent_key, &[]);
    assert_eq!(trust, TrustTier::Unknown);
}

#[test]
fn test_trust_tier_empty_sender() {
    // Empty sender key — Unknown
    let trust = check_sender_trust("", "03parent", &[]);
    assert_eq!(trust, TrustTier::Unknown);
}

#[test]
fn test_trust_tier_no_parent_configured() {
    // No parent key configured (dev mode) — all senders are Unknown
    let sender_key = "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce";
    let trust = check_sender_trust(sender_key, "", &[]);
    assert_eq!(trust, TrustTier::Unknown);
}

#[test]
fn test_trust_tier_parent_takes_priority_over_trusted() {
    // Sender matches both parent AND trusted list — should be Family (higher tier)
    let key = "03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0";
    let trusted = vec![key.to_string()];
    let trust = check_sender_trust(key, key, &trusted);
    assert_eq!(trust, TrustTier::Family);
}
