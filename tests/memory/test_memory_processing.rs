//! Integration tests for post-store memory processing (CI.17 / issue #99).
//!
//! Tests: duplicate detection, tag extraction, quality validation, and non-fatal behavior.

use dolphin_milk::memory::processing::{
    check_duplicates, extract_tags, post_process, validate_quality,
};
use dolphin_milk::memory::search::MemoryIndex;
use dolphin_milk::memory::store::{MemoryCategory, MemoryEntry};

use chrono::Utc;

fn make_entry(id: &str, content: &str) -> MemoryEntry {
    MemoryEntry {
        id: id.to_string(),
        category: MemoryCategory::Knowledge,
        content: content.to_string(),
        tags: vec![],
        created: Utc::now(),
        source: "test".to_string(),
    }
}

/// Populate the index with diverse filler entries to produce realistic
/// BM25 IDF scores. Without a large enough corpus, BM25 IDF is too low
/// and even strong matches score near the dedup threshold, causing
/// flaky tests. 20 diverse documents produce stable IDF values.
fn add_filler_entries(idx: &MemoryIndex) {
    let fillers = [
        "Rust programming language features include ownership, borrowing, and lifetimes",
        "Docker containers provide process isolation and dependency management",
        "Machine learning models require training data and validation sets",
        "GraphQL provides a query language for APIs with type safety",
        "Kubernetes orchestrates container workloads across clusters",
        "Redis is an in-memory data structure store used for caching",
        "PostgreSQL supports JSONB columns for document storage",
        "React components use JSX syntax for declarative UI rendering",
        "Terraform manages infrastructure as code across cloud providers",
        "Nginx serves as a reverse proxy and load balancer for web applications",
        "Git version control tracks changes across distributed repositories",
        "WebAssembly enables near-native performance in browser environments",
        "OAuth2 authorization framework delegates access tokens securely",
        "Prometheus collects time series metrics from instrumented applications",
        "Elasticsearch indexes documents for full text retrieval at scale",
        "RabbitMQ message broker routes asynchronous tasks between services",
        "Ansible automates server provisioning through declarative playbooks",
        "SQLite embedded database requires zero configuration for local storage",
        "gRPC framework enables efficient binary serialization between microservices",
        "Vault secrets manager rotates credentials and encrypts sensitive configuration",
    ];
    for (i, text) in fillers.iter().enumerate() {
        idx.add_entry(&make_entry(&format!("filler-{i:03}"), text))
            .unwrap();
    }
}

// ─────────────────────────────────────────────
// Duplicate detection tests
// ─────────────────────────────────────────────

#[test]
fn test_dedup_detects_exact_match() {
    let dir = tempfile::tempdir().unwrap();
    let idx = MemoryIndex::new(dir.path().join("index")).unwrap();
    add_filler_entries(&idx);

    let content = "The x402 payment protocol uses BEEF transactions for payment verification";
    let existing = make_entry("existing-001", content);
    idx.add_entry(&existing).unwrap();

    // Check duplicate — searching with same content, different ID
    let result = check_duplicates(&idx, content, "new-001");
    assert!(result.is_duplicate, "Exact duplicate should be detected");
    assert_eq!(result.matching_id.as_deref(), Some("existing-001"));
    assert!(result.score.unwrap() >= 20.0);
}

#[test]
fn test_dedup_detects_near_match() {
    let dir = tempfile::tempdir().unwrap();
    let idx = MemoryIndex::new(dir.path().join("index")).unwrap();
    add_filler_entries(&idx);

    let existing = make_entry(
        "existing-001",
        "The x402 payment protocol uses BEEF transactions for payment verification and settlement on chain",
    );
    idx.add_entry(&existing).unwrap();

    // Near-duplicate: same core content, minor rewording at the end
    let near_dup =
        "The x402 payment protocol uses BEEF transactions for payment verification and settlement";
    let result = check_duplicates(&idx, near_dup, "new-001");
    assert!(
        result.is_duplicate,
        "Near duplicate should be detected (score={:.2})",
        result.score.unwrap_or(0.0)
    );
    assert_eq!(result.matching_id.as_deref(), Some("existing-001"));
}

#[test]
fn test_dedup_allows_distinct_topics() {
    let dir = tempfile::tempdir().unwrap();
    let idx = MemoryIndex::new(dir.path().join("index")).unwrap();
    add_filler_entries(&idx);

    let existing = make_entry(
        "existing-001",
        "The x402 payment protocol uses BEEF transactions for payment verification",
    );
    idx.add_entry(&existing).unwrap();

    // Completely different topic — not in filler set
    let different =
        "Astronomy observations of distant galaxies reveal dark matter distribution patterns";
    let result = check_duplicates(&idx, different, "new-001");
    assert!(
        !result.is_duplicate,
        "Distinct topics should not be flagged as duplicates"
    );
}

#[test]
fn test_dedup_empty_index_no_panic() {
    let dir = tempfile::tempdir().unwrap();
    let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

    let result = check_duplicates(&idx, "Some new content to check", "new-001");
    assert!(!result.is_duplicate);
    assert!(result.matching_id.is_none());
}

// ─────────────────────────────────────────────
// Tag extraction tests
// ─────────────────────────────────────────────

#[test]
fn test_tag_extraction_hashtags() {
    let tags = extract_tags("#bitcoin and #wallet are important");
    assert!(
        tags.contains(&"bitcoin".to_string()),
        "Expected 'bitcoin' in tags: {:?}",
        tags
    );
    assert!(
        tags.contains(&"wallet".to_string()),
        "Expected 'wallet' in tags: {:?}",
        tags
    );
}

#[test]
fn test_tag_extraction_caps_terms() {
    let tags = extract_tags("Uses BRC-31 AUTH for BSV transactions");
    assert!(
        tags.contains(&"auth".to_string()),
        "Expected 'auth' in tags: {:?}",
        tags
    );
    assert!(
        tags.contains(&"brc-31".to_string()),
        "Expected 'brc-31' in tags: {:?}",
        tags
    );
}

#[test]
fn test_tag_extraction_max_five() {
    let content = "#one #two #three #four #five #six #seven should stop at five";
    let tags = extract_tags(content);
    assert!(
        tags.len() <= 5,
        "Should not exceed 5 tags, got {}: {:?}",
        tags.len(),
        tags
    );
}

#[test]
fn test_tag_extraction_empty_content() {
    assert!(extract_tags("").is_empty());
    assert!(extract_tags("   ").is_empty());
}

// ─────────────────────────────────────────────
// Quality validation tests
// ─────────────────────────────────────────────

#[test]
fn test_quality_too_short() {
    let warnings = validate_quality("short");
    assert!(
        warnings.iter().any(|w| w.contains("very short")),
        "Expected short content warning, got: {:?}",
        warnings
    );
}

#[test]
fn test_quality_all_whitespace() {
    let warnings = validate_quality("   \n\t  ");
    assert!(
        warnings.iter().any(|w| w.contains("all whitespace")),
        "Expected whitespace warning, got: {:?}",
        warnings
    );
}

#[test]
fn test_quality_valid_content() {
    let warnings =
        validate_quality("The x402 payment protocol uses BEEF transactions for verification");
    assert!(
        warnings.is_empty(),
        "Valid content should have no warnings, got: {:?}",
        warnings
    );
}

// ─────────────────────────────────────────────
// post_process integration test
// ─────────────────────────────────────────────

#[test]
fn test_post_process_non_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

    // Empty content: should produce warnings but not panic
    let result = post_process(&idx, "", "test-001");
    assert!(
        !result.warnings.is_empty(),
        "Empty content should produce warnings"
    );
    assert!(!result.dedup.is_duplicate);

    // Valid content with tags: should succeed cleanly
    let result = post_process(
        &idx,
        "Valid content with #tags and AUTH tokens for BSV",
        "test-002",
    );
    // Always returns — the function is non-fatal regardless of input
    assert!(!result.dedup.is_duplicate);

    // Extremely long content should not panic
    let long = "word ".repeat(10000);
    let result = post_process(&idx, &long, "test-003");
    assert!(!result.dedup.is_duplicate);
}
