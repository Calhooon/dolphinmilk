//! Tests for H.3: Memory tamper detection.
//!
//! Covers: compute_memory_hash, verify_memory_hash, proof sidecar creation/reading,
//! tamper detection during recall, and backward compatibility for entries without sidecars.

use dolphin_milk::memory::search::{
    format_tamper_warnings, verify_recalled_memories, MemoryIndex, MemorySnippet, TamperWarning,
};
use dolphin_milk::memory::store::{MemoryCategory, MemoryEntry, MemoryStore};
use dolphin_milk::proofs::{compute_memory_hash, verify_memory_hash};

use chrono::Utc;

// ---------------------------------------------------------------------------
// Helper: create a MemoryEntry with current timestamp
// ---------------------------------------------------------------------------

fn make_entry(category: MemoryCategory, content: &str, tags: Vec<&str>) -> MemoryEntry {
    MemoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        category,
        content: content.to_string(),
        tags: tags.into_iter().map(String::from).collect(),
        created: Utc::now(),
        source: "test".to_string(),
    }
}

// ---------------------------------------------------------------------------
// compute_memory_hash tests
// ---------------------------------------------------------------------------

#[test]
fn test_compute_memory_hash_deterministic() {
    let hash1 = compute_memory_hash("hello world", "2026-03-25T12:00:00Z", "knowledge");
    let hash2 = compute_memory_hash("hello world", "2026-03-25T12:00:00Z", "knowledge");
    assert_eq!(hash1, hash2, "Same inputs must produce same hash");
    assert_eq!(hash1.len(), 64, "SHA-256 hex hash should be 64 chars");
}

#[test]
fn test_compute_memory_hash_different_content() {
    let hash1 = compute_memory_hash("content A", "2026-03-25T12:00:00Z", "knowledge");
    let hash2 = compute_memory_hash("content B", "2026-03-25T12:00:00Z", "knowledge");
    assert_ne!(
        hash1, hash2,
        "Different content should produce different hashes"
    );
}

#[test]
fn test_compute_memory_hash_different_timestamp() {
    let hash1 = compute_memory_hash("same content", "2026-03-25T12:00:00Z", "knowledge");
    let hash2 = compute_memory_hash("same content", "2026-03-25T13:00:00Z", "knowledge");
    assert_ne!(
        hash1, hash2,
        "Different timestamps should produce different hashes"
    );
}

#[test]
fn test_compute_memory_hash_different_category() {
    let hash1 = compute_memory_hash("same content", "2026-03-25T12:00:00Z", "knowledge");
    let hash2 = compute_memory_hash("same content", "2026-03-25T12:00:00Z", "session");
    assert_ne!(
        hash1, hash2,
        "Different categories should produce different hashes"
    );
}

// ---------------------------------------------------------------------------
// verify_memory_hash tests
// ---------------------------------------------------------------------------

#[test]
fn test_verify_memory_hash_match() {
    let content = "BSV uses the original Bitcoin protocol.";
    let created = "2026-03-25T14:00:00Z";
    let category = "knowledge";

    let hash = compute_memory_hash(content, created, category);
    assert!(
        verify_memory_hash(content, created, category, &hash),
        "Matching hash should return true"
    );
}

#[test]
fn test_verify_memory_hash_mismatch() {
    let content = "BSV uses the original Bitcoin protocol.";
    let created = "2026-03-25T14:00:00Z";
    let category = "knowledge";

    let hash = compute_memory_hash(content, created, category);

    // Tampered content
    assert!(
        !verify_memory_hash("TAMPERED content", created, category, &hash),
        "Tampered content should return false"
    );

    // Tampered timestamp
    assert!(
        !verify_memory_hash(content, "1999-01-01T00:00:00Z", category, &hash),
        "Tampered timestamp should return false"
    );

    // Tampered category
    assert!(
        !verify_memory_hash(content, created, "execution", &hash),
        "Tampered category should return false"
    );

    // Completely wrong hash
    assert!(
        !verify_memory_hash(
            content,
            created,
            category,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        "Wrong hash should return false"
    );
}

// ---------------------------------------------------------------------------
// Proof sidecar creation and reading tests
// ---------------------------------------------------------------------------

#[test]
fn test_proof_sidecar_creation() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    let entry = make_entry(
        MemoryCategory::Knowledge,
        "The x402 payment protocol enables micropayments.",
        vec!["x402"],
    );

    // Store the entry
    store.store(&entry).unwrap();

    // Compute hash and write sidecar
    let content_hash = compute_memory_hash(
        &entry.content,
        &entry.created.to_rfc3339(),
        &entry.category.to_string(),
    );
    let txid = "abc123def456789012345678901234567890123456789012345678901234abcd";

    let sidecar_path = store
        .write_proof_sidecar(&entry, txid, &content_hash)
        .unwrap();
    assert!(sidecar_path.exists(), "Sidecar file should exist on disk");
    assert!(
        sidecar_path.to_string_lossy().ends_with(".proof"),
        "Sidecar file should have .proof extension"
    );

    // Read sidecar back
    let (read_txid, read_hash) = store
        .read_proof_sidecar(&entry.id, &entry.category)
        .expect("Sidecar should be readable");

    assert_eq!(read_txid, txid);
    assert_eq!(read_hash, content_hash);
}

#[test]
fn test_proof_sidecar_missing_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().to_path_buf());

    // No sidecar written — read_proof_sidecar should return None
    let result = store.read_proof_sidecar("nonexistent-id", &MemoryCategory::Knowledge);
    assert!(result.is_none(), "Missing sidecar should return None");
}

// ---------------------------------------------------------------------------
// Tamper detection on recall tests
// ---------------------------------------------------------------------------

#[test]
fn test_tamper_detection_on_recall() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().join("memory"));
    let index = MemoryIndex::new(dir.path().join("index")).unwrap();

    // Store an entry and create its proof sidecar
    let entry = make_entry(
        MemoryCategory::Knowledge,
        "Original content that should not be tampered with.",
        vec!["important"],
    );
    let entry_id = entry.id.clone();
    let created_str = entry.created.to_rfc3339();
    let category_str = entry.category.to_string();

    store.store(&entry).unwrap();
    index.add_entry(&entry).unwrap();

    let content_hash = compute_memory_hash(&entry.content, &created_str, &category_str);
    store
        .write_proof_sidecar(&entry, "faketxid123", &content_hash)
        .unwrap();

    // Now tamper with the file on disk
    let tampered_content = "TAMPERED content that was modified after storage.";
    let tampered_entry = MemoryEntry {
        id: entry_id.clone(),
        category: MemoryCategory::Knowledge,
        content: tampered_content.to_string(),
        tags: vec!["important".to_string()],
        created: entry.created,
        source: "test".to_string(),
    };
    // Overwrite the .md file with tampered content
    store.store(&tampered_entry).unwrap();
    // Re-index so search returns tampered content
    index.add_entry(&tampered_entry).unwrap();

    // Simulate a recall — create a snippet with the tampered content
    let snippet = MemorySnippet {
        id: entry_id.clone(),
        category: category_str.clone(),
        content: tampered_content.to_string(),
        score: 5.0,
        tags: vec!["important".to_string()],
        created: created_str,
    };

    // Verify should detect the tamper
    let warnings = verify_recalled_memories(&[snippet], &store);
    assert_eq!(
        warnings.len(),
        1,
        "Should detect exactly one tampered entry"
    );
    assert_eq!(warnings[0].entry_id, entry_id);
    assert_ne!(warnings[0].expected_hash, warnings[0].actual_hash);
}

#[test]
fn test_no_sidecar_graceful() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().join("memory"));

    // Store an entry WITHOUT creating a proof sidecar
    let entry = make_entry(
        MemoryCategory::Knowledge,
        "Entry without proof sidecar — backward compatible.",
        vec![],
    );
    store.store(&entry).unwrap();

    // Create a snippet mimicking a recall result
    let snippet = MemorySnippet {
        id: entry.id.clone(),
        category: entry.category.to_string(),
        content: entry.content.clone(),
        score: 3.0,
        tags: vec![],
        created: entry.created.to_rfc3339(),
    };

    // Verify should produce no warnings — no sidecar means skip
    let warnings = verify_recalled_memories(&[snippet], &store);
    assert!(
        warnings.is_empty(),
        "Entries without sidecars should not produce warnings"
    );
}

#[test]
fn test_verified_entry_no_warning() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().join("memory"));

    // Store entry and write matching sidecar
    let entry = make_entry(
        MemoryCategory::Knowledge,
        "Verified content that has not been tampered with.",
        vec!["verified"],
    );
    let created_str = entry.created.to_rfc3339();
    let category_str = entry.category.to_string();

    store.store(&entry).unwrap();

    let content_hash = compute_memory_hash(&entry.content, &created_str, &category_str);
    store
        .write_proof_sidecar(&entry, "validtxid", &content_hash)
        .unwrap();

    // Create snippet matching the original content
    let snippet = MemorySnippet {
        id: entry.id.clone(),
        category: category_str,
        content: entry.content.clone(),
        score: 4.0,
        tags: vec!["verified".to_string()],
        created: created_str,
    };

    // No warnings expected — content matches
    let warnings = verify_recalled_memories(&[snippet], &store);
    assert!(
        warnings.is_empty(),
        "Verified entry should not produce warnings"
    );
}

#[test]
fn test_mixed_entries_only_tampered_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(dir.path().join("memory"));

    // Entry 1: verified (has sidecar, content matches)
    let entry1 = make_entry(MemoryCategory::Knowledge, "Verified entry one.", vec![]);
    let created1 = entry1.created.to_rfc3339();
    let cat1 = entry1.category.to_string();
    store.store(&entry1).unwrap();
    let hash1 = compute_memory_hash(&entry1.content, &created1, &cat1);
    store.write_proof_sidecar(&entry1, "txid1", &hash1).unwrap();

    // Entry 2: tampered (has sidecar, content modified)
    let entry2 = make_entry(MemoryCategory::Knowledge, "Original entry two.", vec![]);
    let created2 = entry2.created.to_rfc3339();
    let cat2 = entry2.category.to_string();
    store.store(&entry2).unwrap();
    let hash2 = compute_memory_hash(&entry2.content, &created2, &cat2);
    store.write_proof_sidecar(&entry2, "txid2", &hash2).unwrap();

    // Entry 3: no sidecar (backward compat)
    let entry3 = make_entry(MemoryCategory::Session, "Entry without sidecar.", vec![]);
    let created3 = entry3.created.to_rfc3339();
    store.store(&entry3).unwrap();

    let snippets = vec![
        MemorySnippet {
            id: entry1.id.clone(),
            category: cat1,
            content: entry1.content.clone(), // unchanged
            score: 5.0,
            tags: vec![],
            created: created1,
        },
        MemorySnippet {
            id: entry2.id.clone(),
            category: cat2,
            content: "TAMPERED entry two.".to_string(), // tampered!
            score: 4.0,
            tags: vec![],
            created: created2,
        },
        MemorySnippet {
            id: entry3.id.clone(),
            category: "session".to_string(),
            content: entry3.content.clone(),
            score: 3.0,
            tags: vec![],
            created: created3,
        },
    ];

    let warnings = verify_recalled_memories(&snippets, &store);
    assert_eq!(
        warnings.len(),
        1,
        "Only the tampered entry should be flagged"
    );
    assert_eq!(warnings[0].entry_id, entry2.id);
}

// ---------------------------------------------------------------------------
// Format tamper warnings tests
// ---------------------------------------------------------------------------

#[test]
fn test_format_tamper_warnings_empty() {
    let result = format_tamper_warnings(&[]);
    assert!(result.is_empty(), "No warnings should produce empty string");
}

#[test]
fn test_format_tamper_warnings_content() {
    let warnings = vec![TamperWarning {
        entry_id: "abc-123".to_string(),
        expected_hash: "aaaaaaaaaaaaaaaa1111111111111111aaaaaaaaaaaaaaaa1111111111111111"
            .to_string(),
        actual_hash: "bbbbbbbbbbbbbbbb2222222222222222bbbbbbbbbbbbbbbb2222222222222222".to_string(),
    }];
    let result = format_tamper_warnings(&warnings);
    assert!(result.contains("TAMPER WARNINGS"));
    assert!(result.contains("abc-123"));
    assert!(result.contains("aaaaaaaaaaaaaaaa")); // first 16 chars of expected
    assert!(result.contains("bbbbbbbbbbbbbbbb")); // first 16 chars of actual
    assert!(result.contains("DO NOT trust"));
}
