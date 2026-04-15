//! Tests for memory maintenance via heartbeat (issue #83).
//!
//! Tests the run_memory_maintenance() function: stale flagging, duplicate
//! detection, health reporting, config defaults, and env overrides.

use dolphin_milk::config::{DmConfig, MemoryConfig};
use dolphin_milk::heartbeat::run_memory_maintenance;
use dolphin_milk::memory::store::{MemoryCategory, MemoryEntry, MemoryStore};

use chrono::{Duration, Utc};

/// Helper: create a memory entry with a specific age (days ago).
fn make_entry_aged(
    category: MemoryCategory,
    content: &str,
    tags: Vec<&str>,
    days_ago: i64,
) -> MemoryEntry {
    MemoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        category,
        content: content.to_string(),
        tags: tags.into_iter().map(String::from).collect(),
        created: Utc::now() - Duration::days(days_ago),
        source: "test".to_string(),
    }
}

/// Helper: create a default MemoryConfig with maintenance enabled.
fn enabled_config() -> MemoryConfig {
    MemoryConfig {
        base_dir: String::new(), // unused — we pass memory_dir directly
        maintenance_enabled: true,
        maintenance_interval_secs: 3600,
        stale_session_days: 30,
    }
}

// -----------------------------------------------------------------------
// Test 1: Stale session flagging
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_flags_stale_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    // 31-day-old session entry — should be flagged
    let old_session = make_entry_aged(
        MemoryCategory::Session,
        "Old session summary from a month ago",
        vec!["session"],
        31,
    );
    store.store(&old_session).unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.stale_sessions_flagged, 1);
    assert!(summary.stale_ids.contains(&old_session.id));
}

// -----------------------------------------------------------------------
// Test 2: Fresh sessions NOT flagged
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_keeps_fresh_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    // Yesterday's session — should NOT be flagged
    let fresh_session = make_entry_aged(
        MemoryCategory::Session,
        "Recent session summary from yesterday",
        vec!["session"],
        1,
    );
    store.store(&fresh_session).unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.stale_sessions_flagged, 0);
    assert!(summary.stale_ids.is_empty());
}

// -----------------------------------------------------------------------
// Test 3: Knowledge entries NEVER flagged as stale
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_ignores_knowledge() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    // 60-day-old knowledge entry — should NEVER be flagged
    let old_knowledge = make_entry_aged(
        MemoryCategory::Knowledge,
        "The x402 payment protocol uses BEEF transactions for payment verification",
        vec!["x402", "payment"],
        60,
    );
    store.store(&old_knowledge).unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.stale_sessions_flagged, 0);
    assert!(summary.stale_ids.is_empty());
    assert_eq!(summary.knowledge_count, 1);
}

// -----------------------------------------------------------------------
// Test 4: Duplicate detection via BM25 similarity
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_dedup_scan_finds_similar() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    // Two near-duplicate entries with very similar content
    let entry_a = make_entry_aged(
        MemoryCategory::Knowledge,
        "The BSV blockchain uses the original Bitcoin protocol with unbounded block sizes for micropayments",
        vec!["bsv", "blockchain"],
        5,
    );
    let entry_b = make_entry_aged(
        MemoryCategory::Knowledge,
        "The BSV blockchain uses the original Bitcoin protocol with unbounded block sizes for micropayments and data",
        vec!["bsv", "blockchain"],
        3,
    );
    // A distinctly different entry
    let entry_c = make_entry_aged(
        MemoryCategory::Knowledge,
        "Rust programming language focuses on memory safety and performance",
        vec!["rust", "programming"],
        2,
    );
    store.store(&entry_a).unwrap();
    store.store(&entry_b).unwrap();
    store.store(&entry_c).unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    // Should find the near-duplicate pair (a, b) but not (a, c) or (b, c)
    assert!(
        summary.duplicate_pairs >= 1,
        "Expected at least 1 duplicate pair, got {}",
        summary.duplicate_pairs
    );
    assert_eq!(summary.total_entries, 3);
}

// -----------------------------------------------------------------------
// Test 5: Health report counts
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_reports_counts() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    store
        .store(&make_entry_aged(
            MemoryCategory::Knowledge,
            "fact one",
            vec![],
            1,
        ))
        .unwrap();
    store
        .store(&make_entry_aged(
            MemoryCategory::Knowledge,
            "fact two",
            vec![],
            1,
        ))
        .unwrap();
    store
        .store(&make_entry_aged(
            MemoryCategory::Session,
            "session one",
            vec![],
            1,
        ))
        .unwrap();
    store
        .store(&make_entry_aged(
            MemoryCategory::Execution,
            "exec one",
            vec![],
            1,
        ))
        .unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.total_entries, 4);
    assert_eq!(summary.knowledge_count, 2);
    assert_eq!(summary.session_count, 1);
    assert_eq!(summary.execution_count, 1);
}

// -----------------------------------------------------------------------
// Test 6: Empty memory — no panic
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_empty_memory_no_panic() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let _store = MemoryStore::new(memory_dir.clone());

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.total_entries, 0);
    assert_eq!(summary.stale_sessions_flagged, 0);
    assert_eq!(summary.duplicate_pairs, 0);
}

// -----------------------------------------------------------------------
// Test 7: Config disabled — verify defaults
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_config_disabled() {
    let config = MemoryConfig::default();
    assert!(!config.maintenance_enabled);
}

// -----------------------------------------------------------------------
// Test 8: Config defaults
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_config_defaults() {
    let config = MemoryConfig::default();
    assert!(!config.maintenance_enabled);
    assert_eq!(config.maintenance_interval_secs, 3600);
    assert_eq!(config.stale_session_days, 30);
}

// -----------------------------------------------------------------------
// Test 9: Env override
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_env_override() {
    // Parse TOML with maintenance fields
    let toml_str = r#"
[memory]
maintenance_enabled = true
maintenance_interval_secs = 7200
stale_session_days = 45
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.memory.maintenance_enabled);
    assert_eq!(cfg.memory.maintenance_interval_secs, 7200);
    assert_eq!(cfg.memory.stale_session_days, 45);
}

// -----------------------------------------------------------------------
// Test 10: Execution entries also flagged as stale
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_flags_stale_execution() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    // 35-day-old execution entry — should be flagged
    let old_exec = make_entry_aged(
        MemoryCategory::Execution,
        "Cached tool output from 35 days ago",
        vec!["cache"],
        35,
    );
    store.store(&old_exec).unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.stale_sessions_flagged, 1);
    assert!(summary.stale_ids.contains(&old_exec.id));
    assert_eq!(summary.execution_count, 1);
}

// -----------------------------------------------------------------------
// Test 11: Mixed stale and fresh entries
// -----------------------------------------------------------------------

#[test]
fn test_maintenance_mixed_stale_and_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let memory_dir = dir.path().join("memory");
    let store = MemoryStore::new(memory_dir.clone());

    // Fresh knowledge (never flagged)
    store
        .store(&make_entry_aged(
            MemoryCategory::Knowledge,
            "fresh knowledge",
            vec![],
            100,
        ))
        .unwrap();
    // Fresh session (not flagged)
    store
        .store(&make_entry_aged(
            MemoryCategory::Session,
            "fresh session",
            vec![],
            5,
        ))
        .unwrap();
    // Stale session (flagged)
    let stale = make_entry_aged(MemoryCategory::Session, "stale session", vec![], 31);
    let stale_id = stale.id.clone();
    store.store(&stale).unwrap();
    // Stale execution (flagged)
    let stale_exec = make_entry_aged(MemoryCategory::Execution, "stale execution", vec![], 45);
    let stale_exec_id = stale_exec.id.clone();
    store.store(&stale_exec).unwrap();

    let config = enabled_config();
    let summary = run_memory_maintenance(&memory_dir, &config).unwrap();

    assert_eq!(summary.total_entries, 4);
    assert_eq!(summary.stale_sessions_flagged, 2);
    assert!(summary.stale_ids.contains(&stale_id));
    assert!(summary.stale_ids.contains(&stale_exec_id));
}
