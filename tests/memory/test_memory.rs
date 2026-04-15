//! Integration tests for the memory system (Phase 2d).
//!
//! Tests the full stack: store → index → search → tools → session summarization.

use dolphin_milk::memory::search::MemoryIndex;
use dolphin_milk::memory::session::SessionSummarizer;
use dolphin_milk::memory::store::{MemoryCategory, MemoryEntry, MemoryStore};

use chrono::Utc;
use serde_json::{json, Value};

// ─────────────────────────────────────────────
// MemoryStore tests
// ─────────────────────────────────────────────

#[test]
fn test_store_init_creates_directories() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("memory");
    let _store = MemoryStore::new(base.clone());

    assert!(base.join("knowledge").is_dir());
    assert!(base.join("sessions").is_dir());
    assert!(base.join("execution").is_dir());
}

#[test]
fn test_store_roundtrip_knowledge() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    let entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "The x402 payment protocol uses BEEF transactions for payment verification.",
        vec![
            "x402".to_string(),
            "beef".to_string(),
            "payment".to_string(),
        ],
        "test-session",
    );
    let id = entry.id.clone();
    store.store(&entry).unwrap();

    let loaded = store.read(&id).unwrap();
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.category, MemoryCategory::Knowledge);
    assert!(loaded.content.contains("x402 payment protocol"));
    assert_eq!(loaded.tags, vec!["x402", "beef", "payment"]);
    assert_eq!(loaded.source, "test-session");
}

#[test]
fn test_store_roundtrip_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    let entry = MemoryEntry::new(
        MemoryCategory::Session,
        "Session summary: explored BSV blockchain architecture",
        vec!["bsv".to_string(), "architecture".to_string()],
        "session-2026-02-23",
    );
    let id = entry.id.clone();
    store.store(&entry).unwrap();

    let loaded = store.read(&id).unwrap();
    assert_eq!(loaded.category, MemoryCategory::Session);
}

#[test]
fn test_store_list_all_categories() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    store
        .store(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "fact 1",
            vec![],
            "test",
        ))
        .unwrap();
    store
        .store(&MemoryEntry::new(
            MemoryCategory::Session,
            "summary 1",
            vec![],
            "test",
        ))
        .unwrap();
    store
        .store(&MemoryEntry::new(
            MemoryCategory::Execution,
            "log 1",
            vec![],
            "test",
        ))
        .unwrap();
    store
        .store(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "fact 2",
            vec![],
            "test",
        ))
        .unwrap();

    let all = store.list(None).unwrap();
    assert_eq!(all.len(), 4);

    let knowledge = store.list(Some(MemoryCategory::Knowledge)).unwrap();
    assert_eq!(knowledge.len(), 2);

    let sessions = store.list(Some(MemoryCategory::Session)).unwrap();
    assert_eq!(sessions.len(), 1);

    let execution = store.list(Some(MemoryCategory::Execution)).unwrap();
    assert_eq!(execution.len(), 1);
}

#[test]
fn test_store_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    let entry = MemoryEntry::new(MemoryCategory::Knowledge, "temporary fact", vec![], "test");
    let id = entry.id.clone();
    store.store(&entry).unwrap();

    assert!(store.read(&id).is_ok());
    store.delete(&id).unwrap();
    assert!(store.read(&id).is_err());
}

#[test]
fn test_store_list_sorted_by_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Store entries with slight time gaps
    let e1 = MemoryEntry::new(MemoryCategory::Knowledge, "first", vec![], "test");
    store.store(&e1).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));

    let e2 = MemoryEntry::new(MemoryCategory::Knowledge, "second", vec![], "test");
    store.store(&e2).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));

    let e3 = MemoryEntry::new(MemoryCategory::Knowledge, "third", vec![], "test");
    store.store(&e3).unwrap();

    let all = store.list(Some(MemoryCategory::Knowledge)).unwrap();
    assert_eq!(all.len(), 3);
    // Sorted newest first
    assert_eq!(all[0].content, "third");
    assert_eq!(all[1].content, "second");
    assert_eq!(all[2].content, "first");
}

#[test]
fn test_store_special_characters_in_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    let content = "Special chars: ---\n```rust\nfn main() {}\n```\n## Heading\n- bullet\n> quote";
    let entry = MemoryEntry::new(MemoryCategory::Knowledge, content, vec![], "test");
    let id = entry.id.clone();
    store.store(&entry).unwrap();

    let loaded = store.read(&id).unwrap();
    assert_eq!(loaded.content, content);
}

#[test]
fn test_category_parse_variants() {
    assert_eq!(
        "knowledge".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Knowledge
    );
    assert_eq!(
        "Knowledge".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Knowledge
    );
    assert_eq!(
        "KNOWLEDGE".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Knowledge
    );
    assert_eq!(
        "session".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Session
    );
    assert_eq!(
        "sessions".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Session
    );
    assert_eq!(
        "execution".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Execution
    );
    assert!("invalid".parse::<MemoryCategory>().is_err());
}

// ─────────────────────────────────────────────
// MemoryIndex tests
// ─────────────────────────────────────────────

#[test]
fn test_index_create_and_count() {
    let dir = tempfile::tempdir().unwrap();
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    assert_eq!(index.entry_count().unwrap(), 0);

    let entry = MemoryEntry::new(MemoryCategory::Knowledge, "test content", vec![], "test");
    index.add_entry(&entry).unwrap();
    assert_eq!(index.entry_count().unwrap(), 1);
}

#[test]
fn test_index_search_finds_content() {
    let dir = tempfile::tempdir().unwrap();
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    index
        .add_entry(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "The BSV blockchain uses the original Bitcoin protocol with unbounded block sizes",
            vec!["bsv".to_string(), "blockchain".to_string()],
            "test",
        ))
        .unwrap();

    index
        .add_entry(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "Rust is a systems programming language focused on safety",
            vec!["rust".to_string(), "programming".to_string()],
            "test",
        ))
        .unwrap();

    let results = index.search("BSV blockchain protocol", 5).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("BSV"));
}

#[test]
fn test_index_search_ranking() {
    let dir = tempfile::tempdir().unwrap();
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    // Entry highly relevant to "x402 payment"
    index.add_entry(&MemoryEntry::new(
        MemoryCategory::Knowledge,
        "The x402 payment protocol handles micropayments via BEEF transactions. Payment verification uses BRC-29 key derivation.",
        vec!["x402".to_string(), "payment".to_string()],
        "test",
    )).unwrap();

    // Entry somewhat relevant
    index
        .add_entry(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "The wallet client connects to port 3322 for payment operations",
            vec!["wallet".to_string()],
            "test",
        ))
        .unwrap();

    // Irrelevant entry
    index
        .add_entry(&MemoryEntry::new(
            MemoryCategory::Execution,
            "File search completed with 50 results found in the codebase",
            vec!["search".to_string()],
            "test",
        ))
        .unwrap();

    let results = index.search("x402 payment protocol", 5).unwrap();
    assert!(!results.is_empty());
    // Most relevant should be first (highest score)
    assert!(results[0].content.contains("x402 payment protocol"));
    assert!(results[0].score >= results.last().unwrap().score);
}

#[test]
fn test_index_delete_entry() {
    let dir = tempfile::tempdir().unwrap();
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    let entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "deletable content",
        vec![],
        "test",
    );
    let id = entry.id.clone();
    index.add_entry(&entry).unwrap();
    assert_eq!(index.entry_count().unwrap(), 1);

    index.delete_entry(&id).unwrap();
    assert_eq!(index.entry_count().unwrap(), 0);
}

#[test]
fn test_index_rebuild_from_entries() {
    let dir = tempfile::tempdir().unwrap();
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    // Add some entries
    let entries = vec![
        MemoryEntry::new(MemoryCategory::Knowledge, "fact one", vec![], "test"),
        MemoryEntry::new(MemoryCategory::Knowledge, "fact two", vec![], "test"),
        MemoryEntry::new(MemoryCategory::Session, "session summary", vec![], "test"),
    ];

    // Rebuild from entries
    index.rebuild(&entries).unwrap();
    assert_eq!(index.entry_count().unwrap(), 3);

    // Rebuild again (should clear and re-add)
    let fewer = vec![MemoryEntry::new(
        MemoryCategory::Knowledge,
        "only fact",
        vec![],
        "test",
    )];
    index.rebuild(&fewer).unwrap();
    assert_eq!(index.entry_count().unwrap(), 1);
}

#[test]
fn test_index_reopen_existing() {
    let dir = tempfile::tempdir().unwrap();
    let index_dir = dir.path().join("index");

    // Create and populate
    {
        let index = MemoryIndex::new(index_dir.clone()).unwrap();
        index
            .add_entry(&MemoryEntry::new(
                MemoryCategory::Knowledge,
                "persistent fact",
                vec!["persistent".to_string()],
                "test",
            ))
            .unwrap();
    }

    // Reopen and verify
    let index = MemoryIndex::new(index_dir).unwrap();
    let results = index.search("persistent", 5).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("persistent"));
}

// ─────────────────────────────────────────────
// Store + Index integration (round-trip)
// ─────────────────────────────────────────────

#[test]
fn test_store_then_index_then_search() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().join("memory"));
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    // Store entries
    let entries = vec![
        MemoryEntry::new(
            MemoryCategory::Knowledge,
            "Loop detection uses four algorithms: generic repeat, ping-pong, no-progress, and circuit breaker",
            vec!["loop".to_string(), "detection".to_string()],
            "session-001",
        ),
        MemoryEntry::new(
            MemoryCategory::Knowledge,
            "BEEF transaction format starts with magic bytes 0x01 0x00 0xBE 0xEF",
            vec!["beef".to_string(), "transaction".to_string()],
            "session-001",
        ),
        MemoryEntry::new(
            MemoryCategory::Session,
            "Session completed: implemented context manager with offloading",
            vec!["context".to_string()],
            "session-001",
        ),
    ];

    for entry in &entries {
        store.store(entry).unwrap();
        index.add_entry(entry).unwrap();
    }

    // Verify store has entries
    assert_eq!(store.list(None).unwrap().len(), 3);

    // Search for loop detection
    let results = index.search("loop detection algorithms", 5).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("loop") || results[0].content.contains("Loop"));

    // Search for BEEF
    let results = index.search("BEEF transaction magic bytes", 5).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("BEEF") || results[0].content.contains("0xBE"));
}

#[test]
fn test_rebuild_index_from_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().join("memory"));
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    // Store entries (but don't index them)
    store
        .store(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "BSV uses proof-of-work consensus",
            vec!["bsv".to_string()],
            "test",
        ))
        .unwrap();
    store
        .store(&MemoryEntry::new(
            MemoryCategory::Knowledge,
            "The worm pays for inference via x402",
            vec!["x402".to_string()],
            "test",
        ))
        .unwrap();

    // Index is empty
    assert_eq!(index.entry_count().unwrap(), 0);

    // Rebuild index from store
    let all_entries = store.list(None).unwrap();
    index.rebuild(&all_entries).unwrap();
    assert_eq!(index.entry_count().unwrap(), 2);

    // Now search works
    let results = index.search("proof-of-work", 5).unwrap();
    assert!(!results.is_empty());
}

// ─────────────────────────────────────────────
// Session Summarization tests
// ─────────────────────────────────────────────

#[test]
fn test_session_summarizer_basic() {
    let events: Vec<Value> = vec![
        json!({"type": "session_start", "task": "Research BSV architecture", "ts": 1708700000.0}),
        json!({"type": "user", "content": "Research BSV architecture"}),
        json!({"type": "think_response", "text": "I'll analyze the BSV blockchain architecture.", "sats_paid": 3200}),
        json!({"type": "tool_call", "tool_call_id": "tc1", "name": "execute_bash", "arguments": {"command": "ls"}}),
        json!({"type": "tool_result", "tool_call_id": "tc1", "result": "file1.rs\nfile2.rs"}),
        json!({"type": "session_end", "iterations": 3, "sats_spent": 5000}),
    ];

    let summary = SessionSummarizer::summarize_events(&events);
    assert!(summary.contains("Research BSV architecture"));
    assert!(summary.contains("execute_bash"));
}

#[test]
fn test_session_summarizer_creates_entry() {
    let events: Vec<Value> = vec![
        json!({"type": "session_start", "task": "Debug x402 flow"}),
        json!({"type": "user", "content": "Debug x402 flow"}),
        json!({"type": "think_response", "text": "Found the issue.", "sats_paid": 1000}),
        json!({"type": "session_end", "iterations": 1, "sats_spent": 1000}),
    ];

    let entry = SessionSummarizer::create_session_entry(&events, "session-test-001");
    assert_eq!(entry.category, MemoryCategory::Session);
    assert!(entry.content.contains("Debug x402 flow"));
    assert_eq!(entry.source, "session-test-001");
}

#[test]
fn test_session_summarizer_empty_events() {
    let events: Vec<Value> = vec![];
    let summary = SessionSummarizer::summarize_events(&events);
    // Should still produce valid output, just sparse
    assert!(summary.contains("Session Summary"));
}

#[test]
fn test_session_summarizer_with_errors() {
    let events: Vec<Value> = vec![
        json!({"type": "user", "content": "Do something risky"}),
        json!({"type": "error", "error": "budget exhausted: spent 50000 sats", "error_type": "budget"}),
        json!({"type": "session_end", "iterations": 5, "sats_spent": 50000, "error": "budget exhausted"}),
    ];

    let summary = SessionSummarizer::summarize_events(&events);
    // Should capture the error in the Errors section
    assert!(summary.contains("budget exhausted"));
}

// ─────────────────────────────────────────────
// Memory category and error tests
// ─────────────────────────────────────────────

#[test]
fn test_memory_error_variant() {
    use dolphin_milk::error::DmError;

    let err = DmError::memory("test memory error");
    assert!(format!("{err}").contains("memory error"));
    assert!(format!("{err}").contains("test memory error"));

    // Verify it has context
    let ctx = err.context();
    assert!(ctx.is_empty()); // Default context is empty
}

#[test]
fn test_memory_entry_new_generates_uuid() {
    let e1 = MemoryEntry::new(MemoryCategory::Knowledge, "a", vec![], "test");
    let e2 = MemoryEntry::new(MemoryCategory::Knowledge, "b", vec![], "test");
    assert_ne!(e1.id, e2.id);
    // UUIDs are 36 chars (8-4-4-4-12 format)
    assert_eq!(e1.id.len(), 36);
}

#[test]
fn test_memory_entry_timestamp_is_recent() {
    let entry = MemoryEntry::new(MemoryCategory::Knowledge, "test", vec![], "test");
    let now = Utc::now();
    let diff = now - entry.created;
    // Should be within 1 second
    assert!(diff.num_seconds() < 1);
}
