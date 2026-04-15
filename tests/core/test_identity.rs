//! Tests for Phase 9.1: Identity/Soul System.
//!
//! Tests identity memory entries (Knowledge category, tag "identity"),
//! the find_identity_entry() helper, soul section in system prompt,
//! and the IDENTITY_TAG constant.

use std::thread;
use std::time::Duration;

use dolphin_milk::context::prompt::{build_system_prompt, PromptContext, ToolDesc};
use dolphin_milk::memory::store::{MemoryCategory, MemoryEntry, MemoryStore, IDENTITY_TAG};

// ---------------------------------------------------------------------------
// IDENTITY_TAG constant
// ---------------------------------------------------------------------------

#[test]
fn test_identity_tag_value() {
    assert_eq!(IDENTITY_TAG, "identity");
}

#[test]
fn test_identity_tag_is_static_str() {
    // Ensure it can be used in tag vectors without allocation issues
    let tags: Vec<String> = vec![IDENTITY_TAG.to_string()];
    assert_eq!(tags[0], "identity");
}

// ---------------------------------------------------------------------------
// find_identity_entry()
// ---------------------------------------------------------------------------

#[test]
fn test_find_identity_entry_no_entries() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    assert!(
        store.find_identity_entry().is_none(),
        "empty store should return None"
    );
}

#[test]
fn test_find_identity_entry_no_identity_tag() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Store a knowledge entry without the identity tag
    let entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "BSV uses the original protocol.",
        vec!["bsv".to_string()],
        "test",
    );
    store.store(&entry).unwrap();

    assert!(
        store.find_identity_entry().is_none(),
        "entry without identity tag should not be found"
    );
}

#[test]
fn test_find_identity_entry_with_identity_tag() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    let entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\n## Core\n- **Name**: test-agent",
        vec![IDENTITY_TAG.to_string()],
        "identity-bootstrap",
    );
    let expected_id = entry.id.clone();
    store.store(&entry).unwrap();

    let found = store.find_identity_entry();
    assert!(found.is_some(), "should find the identity entry");
    let found = found.unwrap();
    assert_eq!(found.id, expected_id);
    assert!(found.content.contains("test-agent"));
    assert!(found.tags.contains(&IDENTITY_TAG.to_string()));
}

#[test]
fn test_find_identity_entry_returns_newest() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Store an older identity entry
    let old_entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\n## Purpose\nOld purpose.",
        vec![IDENTITY_TAG.to_string()],
        "identity-bootstrap",
    );
    store.store(&old_entry).unwrap();

    // Small delay to ensure different timestamps
    thread::sleep(Duration::from_millis(20));

    // Store a newer identity entry
    let new_entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\n## Purpose\nNew purpose.",
        vec![IDENTITY_TAG.to_string()],
        "identity-update",
    );
    let expected_id = new_entry.id.clone();
    store.store(&new_entry).unwrap();

    let found = store.find_identity_entry();
    assert!(found.is_some(), "should find identity entry");
    let found = found.unwrap();
    assert_eq!(found.id, expected_id, "should return the newest entry");
    assert!(found.content.contains("New purpose."));
}

#[test]
fn test_find_identity_entry_ignores_other_categories() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Store a session entry with identity tag (should be ignored — wrong category)
    let session_entry = MemoryEntry::new(
        MemoryCategory::Session,
        "session with identity tag",
        vec![IDENTITY_TAG.to_string()],
        "test",
    );
    store.store(&session_entry).unwrap();

    // Store an execution entry with identity tag (should be ignored — wrong category)
    let exec_entry = MemoryEntry::new(
        MemoryCategory::Execution,
        "execution with identity tag",
        vec![IDENTITY_TAG.to_string()],
        "test",
    );
    store.store(&exec_entry).unwrap();

    assert!(
        store.find_identity_entry().is_none(),
        "identity entries must be in Knowledge category"
    );
}

#[test]
fn test_find_identity_entry_mixed_entries() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Store various non-identity entries
    let e1 = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "fact about BSV",
        vec!["bsv".to_string()],
        "test",
    );
    store.store(&e1).unwrap();

    let e2 = MemoryEntry::new(MemoryCategory::Session, "session summary", vec![], "test");
    store.store(&e2).unwrap();

    // Now store the identity entry
    let identity = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\n## Core\n- **Name**: worm-1",
        vec![IDENTITY_TAG.to_string(), "core".to_string()],
        "identity-bootstrap",
    );
    let expected_id = identity.id.clone();
    store.store(&identity).unwrap();

    // Add more non-identity entries
    let e3 = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "another fact",
        vec!["fact".to_string()],
        "test",
    );
    store.store(&e3).unwrap();

    let found = store.find_identity_entry();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, expected_id);
}

// ---------------------------------------------------------------------------
// Identity entry content format
// ---------------------------------------------------------------------------

#[test]
fn test_identity_entry_content_format() {
    // Verify the bootstrap format matches the spec
    let name = "test-agent";
    let identity_key = "02abc123";
    let capabilities = "llm,tools,messaging,x402";
    let certifier = "self";
    let deployed = "2026-03-03T00:00:00Z";

    let content = format!(
        "# Agent Identity\n\n\
         ## Core (from certificate)\n\
         - **Name**: {name}\n\
         - **Identity Key**: {identity_key}\n\
         - **Capabilities**: {capabilities}\n\
         - **Certifier**: {certifier}\n\
         - **Deployed**: {deployed}\n\n\
         ## Purpose\n\
         Autonomous BSV agent. Purpose evolving through experience.\n\n\
         ## Operational Notes\n\
         No operational observations yet."
    );

    assert!(content.contains("# Agent Identity"));
    assert!(content.contains("## Core (from certificate)"));
    assert!(content.contains("test-agent"));
    assert!(content.contains("02abc123"));
    assert!(content.contains("## Purpose"));
    assert!(content.contains("## Operational Notes"));

    // Verify it round-trips through MemoryEntry
    let entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        content.clone(),
        vec![IDENTITY_TAG.to_string()],
        "identity-bootstrap",
    );
    assert_eq!(entry.content, content);
    assert_eq!(entry.category, MemoryCategory::Knowledge);
}

// ---------------------------------------------------------------------------
// section_soul() formatting via build_system_prompt()
// ---------------------------------------------------------------------------

fn default_ctx() -> PromptContext {
    PromptContext {
        identity_key: String::new(),
        balance_sats: 0,
        model: String::new(),
        tools: Vec::new(),
        memory_summary: String::new(),
        available_files: Vec::new(),
        task: String::new(),
        budget_remaining: 0,
        low_power: false,
        inbox_count: 0,
        skills_section: String::new(),
        workspace_path: String::new(),
        certificate_info: None,
        has_external_messages: false,
        identity_soul: None,
        auto_recall_ids: vec![],
        basket_health: std::collections::HashMap::new(),
        spendable_output_count: 0,
        instructions: String::new(),
        env_snapshot: None,
        working_memory_section: String::new(),
    }
}

#[test]
fn test_soul_section_omitted_when_none() {
    let ctx = default_ctx();
    let prompt = build_system_prompt(&ctx);
    assert!(
        !prompt.contains("# Soul"),
        "Soul section should not appear when identity_soul is None"
    );
    assert!(
        !prompt.contains("memory_store"),
        "Soul instructions should not appear when identity_soul is None"
    );
}

#[test]
fn test_soul_section_present_when_some() {
    let mut ctx = default_ctx();
    ctx.identity_soul =
        Some("# Agent Identity\n\n## Purpose\nI help with BSV development.".to_string());

    let prompt = build_system_prompt(&ctx);
    assert!(
        prompt.contains("# Soul"),
        "Soul section should appear when identity_soul is Some"
    );
    assert!(
        prompt.contains("I help with BSV development."),
        "Soul content should be included"
    );
    assert!(
        prompt.contains("memory_store"),
        "Should include instructions about updating identity"
    );
    assert!(
        prompt.contains("on-chain proofs"),
        "Should mention on-chain proofs for identity updates"
    );
}

#[test]
fn test_soul_section_between_identity_and_external_warning() {
    let mut ctx = default_ctx();
    ctx.identity_soul = Some("My soul content.".to_string());
    ctx.has_external_messages = true;

    let prompt = build_system_prompt(&ctx);

    // Find positions of key sections
    let identity_pos = prompt
        .find("# Identity")
        .expect("Identity section must exist");
    let soul_pos = prompt.find("# Soul").expect("Soul section must exist");
    let warning_pos = prompt
        .find("# External Message Warning")
        .expect("External warning section must exist");

    assert!(
        identity_pos < soul_pos,
        "Identity ({identity_pos}) must come before Soul ({soul_pos})"
    );
    assert!(
        soul_pos < warning_pos,
        "Soul ({soul_pos}) must come before External Warning ({warning_pos})"
    );
}

#[test]
fn test_soul_section_formatting() {
    let mut ctx = default_ctx();
    ctx.identity_soul = Some("Test content here.".to_string());

    let prompt = build_system_prompt(&ctx);

    // Extract the soul section
    let soul_start = prompt.find("# Soul").unwrap();
    let soul_section = &prompt[soul_start..];

    // Verify formatting
    assert!(soul_section.starts_with("# Soul\n\n"));
    assert!(soul_section.contains("Test content here."));
    assert!(soul_section.contains("You may update your identity via memory_store"));
    assert!(soul_section.contains("category \"knowledge\""));
    assert!(soul_section.contains("tag \"identity\""));
}

#[test]
fn test_soul_section_with_multiline_content() {
    let mut ctx = default_ctx();
    ctx.identity_soul = Some(
        "# Agent Identity\n\n\
         ## Core (from certificate)\n\
         - **Name**: prod-agent\n\
         - **Identity Key**: 02abc\n\n\
         ## Purpose\n\
         I am a production BSV agent.\n\n\
         ## Operational Notes\n\
         I prefer concise responses."
            .to_string(),
    );

    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("prod-agent"));
    assert!(prompt.contains("production BSV agent"));
    assert!(prompt.contains("concise responses"));
}

// ---------------------------------------------------------------------------
// Identity bootstrap integration (format validation)
// ---------------------------------------------------------------------------

#[test]
fn test_identity_bootstrap_creates_valid_entry() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Simulate what runner.rs does during bootstrap
    let name = "dolphin-milk-agent";
    let identity_key = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let capabilities = "llm,tools,messaging,x402";
    let certifier = "self";
    let deployed = chrono::Utc::now().to_rfc3339();

    let content = format!(
        "# Agent Identity\n\n\
         ## Core (from certificate)\n\
         - **Name**: {name}\n\
         - **Identity Key**: {identity_key}\n\
         - **Capabilities**: {capabilities}\n\
         - **Certifier**: {certifier}\n\
         - **Deployed**: {deployed}\n\n\
         ## Purpose\n\
         Autonomous BSV agent. Purpose evolving through experience.\n\n\
         ## Operational Notes\n\
         No operational observations yet."
    );

    let entry = MemoryEntry::new(
        MemoryCategory::Knowledge,
        content,
        vec![IDENTITY_TAG.to_string()],
        "identity-bootstrap",
    );
    store.store(&entry).unwrap();

    // Verify it can be found
    let found = store.find_identity_entry().unwrap();
    assert_eq!(found.category, MemoryCategory::Knowledge);
    assert!(found.tags.contains(&IDENTITY_TAG.to_string()));
    assert!(found.content.contains("dolphin-milk-agent"));
    assert!(found.content.contains(identity_key));
    assert_eq!(found.source, "identity-bootstrap");
}

#[test]
fn test_identity_bootstrap_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // First bootstrap
    let entry1 = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\nFirst version.",
        vec![IDENTITY_TAG.to_string()],
        "identity-bootstrap",
    );
    store.store(&entry1).unwrap();

    // Simulate runner.rs check — identity exists, so skip bootstrap
    let existing = store.find_identity_entry();
    assert!(existing.is_some(), "first entry should be found");

    // Should not create another — runner checks find_identity_entry() first
    let all_knowledge = store.list(Some(MemoryCategory::Knowledge)).unwrap();
    let identity_entries: Vec<_> = all_knowledge
        .iter()
        .filter(|e| e.tags.contains(&IDENTITY_TAG.to_string()))
        .collect();
    assert_eq!(
        identity_entries.len(),
        1,
        "only one identity entry should exist"
    );
}

#[test]
fn test_identity_update_by_agent() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // Bootstrap creates initial identity
    let initial = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\n## Purpose\nAutonomous BSV agent.",
        vec![IDENTITY_TAG.to_string()],
        "identity-bootstrap",
    );
    store.store(&initial).unwrap();

    thread::sleep(Duration::from_millis(20));

    // Agent updates its identity via memory_store tool (creates a new entry)
    let updated = MemoryEntry::new(
        MemoryCategory::Knowledge,
        "# Agent Identity\n\n## Purpose\nI specialize in x402 payment integration.",
        vec![IDENTITY_TAG.to_string()],
        "agent-self-update",
    );
    store.store(&updated).unwrap();

    // find_identity_entry returns the newest
    let found = store.find_identity_entry().unwrap();
    assert!(found.content.contains("x402 payment integration"));
    assert_eq!(found.source, "agent-self-update");
}

// ---------------------------------------------------------------------------
// PromptContext with identity_soul renders correctly
// ---------------------------------------------------------------------------

#[test]
fn test_prompt_context_all_sections_with_soul() {
    let mut ctx = default_ctx();
    ctx.identity_key = "02abc".to_string();
    ctx.identity_soul = Some("I am a test agent.".to_string());
    ctx.memory_summary = "Some memory.".to_string();
    ctx.tools = vec![ToolDesc {
        name: "execute_bash".to_string(),
        description: "Run commands".to_string(),
        category: "sandbox".to_string(),
        deferred: false,
        hint: None,
    }];

    let prompt = build_system_prompt(&ctx);

    // All expected sections should be present
    assert!(prompt.contains("# Identity"));
    assert!(prompt.contains("# Soul"));
    assert!(prompt.contains("# Environment"));
    assert!(prompt.contains("# Wallet"));
    assert!(prompt.contains("# Available Tools"));
    assert!(prompt.contains("# Memory"));
    assert!(prompt.contains("# Working Principles"));

    // Soul content should be included
    assert!(prompt.contains("I am a test agent."));
}
