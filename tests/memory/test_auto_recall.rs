//! Tests for Phase 9.2: Automatic BM25 Memory Recall.
//!
//! Covers: extract_query_hints, format_recalled_memories, auto_recall
//! (MMR diversity, recency weighting, exclude_ids), and jaccard_similarity.

use chrono::{Duration, Utc};
use dolphin_milk::memory::search::{
    auto_recall, extract_key_terms, extract_query_hints, format_recalled_memories,
    jaccard_similarity, sanitize_query, MemoryIndex, MemorySnippet,
};
use dolphin_milk::memory::store::{MemoryCategory, MemoryEntry};

// ---------------------------------------------------------------------------
// Helper: create a MemoryEntry with a specific age
// ---------------------------------------------------------------------------

fn make_entry(id: &str, content: &str, category: MemoryCategory, age_hours: i64) -> MemoryEntry {
    MemoryEntry {
        id: id.to_string(),
        category,
        content: content.to_string(),
        tags: vec![],
        created: Utc::now() - Duration::hours(age_hours),
        source: "test".to_string(),
    }
}

fn make_entry_with_tags(id: &str, content: &str, tags: Vec<&str>, age_hours: i64) -> MemoryEntry {
    MemoryEntry {
        id: id.to_string(),
        category: MemoryCategory::Knowledge,
        content: content.to_string(),
        tags: tags.into_iter().map(String::from).collect(),
        created: Utc::now() - Duration::hours(age_hours),
        source: "test".to_string(),
    }
}

fn make_snippet(
    id: &str,
    category: &str,
    content: &str,
    score: f32,
    age_hours: i64,
) -> MemorySnippet {
    let created = Utc::now() - Duration::hours(age_hours);
    MemorySnippet {
        id: id.to_string(),
        category: category.to_string(),
        content: content.to_string(),
        score,
        tags: vec![],
        created: created.to_rfc3339(),
    }
}

// ===========================================================================
// extract_query_hints tests
// ===========================================================================

#[test]
fn test_extract_query_hints_empty_task() {
    let result = extract_query_hints("", None);
    assert_eq!(result, "");
}

#[test]
fn test_extract_query_hints_task_only() {
    let result = extract_query_hints("Generate an image of a sunset", None);
    // Key terms extracted — stop words like "an", "of", "a" filtered out
    assert!(result.contains("Generate"));
    assert!(result.contains("image"));
    assert!(result.contains("sunset"));
    assert!(!result.contains(" an "));
    assert!(!result.contains(" of "));
}

#[test]
fn test_extract_query_hints_with_recent_text() {
    let result = extract_query_hints(
        "Analyze payment flow",
        Some("The x402 payment protocol requires BRC-31 authentication"),
    );
    // Key terms should include domain-specific vocabulary from both sources
    assert!(
        result.contains("payment"),
        "should contain 'payment': '{result}'"
    );
    assert!(result.contains("x402"), "should contain 'x402': '{result}'");
    assert!(
        result.contains("BRC") || result.contains("authentication"),
        "should contain domain terms: '{result}'"
    );
}

#[test]
fn test_extract_query_hints_long_recent_text_truncated() {
    let long_text = "a ".repeat(200); // 400 chars — all stop words
    let result = extract_query_hints("task", Some(&long_text));
    // "a" is a stop word, "task" is also a stop word — should fall back
    // Key terms extraction should produce a short result
    assert!(result.len() <= 500);
}

#[test]
fn test_extract_query_hints_total_truncated_to_500() {
    let long_task = "word ".repeat(120); // 600 chars
    let result = extract_query_hints(&long_task, None);
    // After key term extraction, the result should be much shorter
    // (deduplication means "word" appears once)
    assert!(result.len() <= 500);
}

#[test]
fn test_extract_query_hints_empty_recent_text() {
    let result = extract_query_hints("my task", Some(""));
    assert_eq!(result, "my task");
}

#[test]
fn test_extract_query_hints_whitespace_recent_text() {
    let result = extract_query_hints("my task", Some("   \n  "));
    assert_eq!(result, "my task");
}

// ===========================================================================
// format_recalled_memories tests
// ===========================================================================

#[test]
fn test_format_recalled_memories_empty() {
    let result = format_recalled_memories(&[]);
    assert!(result.is_empty());
}

#[test]
fn test_format_recalled_memories_single() {
    let snippets = vec![make_snippet(
        "s1",
        "knowledge",
        "The x402 payment protocol handles micropayments.",
        0.85,
        1,
    )];
    let result = format_recalled_memories(&snippets);
    assert!(result.contains("## Recalled Memories (auto)"));
    assert!(result.contains("[knowledge]"));
    assert!(result.contains("x402 payment protocol"));
    assert!(result.contains("0.85"));
}

#[test]
fn test_format_recalled_memories_multiple() {
    let snippets = vec![
        make_snippet("s1", "knowledge", "First memory entry.", 0.9, 1),
        make_snippet("s2", "session", "Second memory entry.", 0.7, 2),
        make_snippet("s3", "execution", "Third memory entry.", 0.5, 3),
    ];
    let result = format_recalled_memories(&snippets);
    assert!(result.contains("[knowledge]"));
    assert!(result.contains("[session]"));
    assert!(result.contains("[execution]"));
    assert!(result.contains("First memory entry."));
    assert!(result.contains("Second memory entry."));
    assert!(result.contains("Third memory entry."));
}

#[test]
fn test_format_recalled_memories_content_truncation() {
    let long_content = "word ".repeat(100); // 500 chars
    let snippets = vec![make_snippet("s1", "knowledge", &long_content, 0.8, 1)];
    let result = format_recalled_memories(&snippets);
    // Content should be truncated to ~300 chars with "..."
    assert!(result.contains("..."));
    // Should not contain the full 500-char content
    assert!(result.len() < 600);
}

#[test]
fn test_format_recalled_memories_first_line_heading() {
    let content = "Payment flow overview\nDetailed description of the payment steps.";
    let snippets = vec![make_snippet("s1", "knowledge", content, 0.9, 1)];
    let result = format_recalled_memories(&snippets);
    assert!(result.contains("Payment flow overview"));
}

#[test]
fn test_format_recalled_memories_score_formatting() {
    let snippets = vec![make_snippet("s1", "knowledge", "Test content.", 1.2345, 1)];
    let result = format_recalled_memories(&snippets);
    assert!(result.contains("1.23"));
}

// ===========================================================================
// jaccard_similarity tests
// ===========================================================================

#[test]
fn test_jaccard_identical_strings() {
    let sim = jaccard_similarity("hello world", "hello world");
    assert!((sim - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_completely_different() {
    let sim = jaccard_similarity("hello world", "foo bar baz");
    assert!((sim - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_partial_overlap() {
    let sim = jaccard_similarity("hello world foo", "hello world bar");
    // Intersection: {hello, world} = 2, Union: {hello, world, foo, bar} = 4
    assert!((sim - 0.5).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_case_insensitive() {
    let sim = jaccard_similarity("Hello World", "hello world");
    assert!((sim - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_empty_strings() {
    let sim = jaccard_similarity("", "");
    assert!((sim - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_one_empty() {
    let sim = jaccard_similarity("hello world", "");
    assert!((sim - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_subset() {
    // {hello, world} is subset of {hello, world, foo}
    let sim = jaccard_similarity("hello world", "hello world foo");
    // Intersection: 2, Union: 3
    assert!((sim - 2.0 / 3.0).abs() < 0.001);
}

// ===========================================================================
// auto_recall tests
// ===========================================================================

fn build_test_index(entries: &[MemoryEntry]) -> (tempfile::TempDir, MemoryIndex) {
    let dir = tempfile::tempdir().unwrap();
    let idx = MemoryIndex::new(dir.path().join("index")).unwrap();
    for entry in entries {
        idx.add_entry(entry).unwrap();
    }
    (dir, idx)
}

#[test]
fn test_auto_recall_empty_query() {
    let entries = vec![make_entry(
        "e1",
        "Some content about payments",
        MemoryCategory::Knowledge,
        1,
    )];
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "", 5, &[]);
    assert!(result.is_empty());
}

#[test]
fn test_auto_recall_empty_index() {
    let (_dir, idx) = build_test_index(&[]);
    let result = auto_recall(&idx, "payment", 5, &[]);
    assert!(result.is_empty());
}

#[test]
fn test_auto_recall_basic_results() {
    let entries = vec![
        make_entry(
            "e1",
            "The x402 payment protocol handles micropayments for AI inference",
            MemoryCategory::Knowledge,
            1,
        ),
        make_entry(
            "e2",
            "Tantivy is a search engine library written in Rust",
            MemoryCategory::Knowledge,
            2,
        ),
    ];
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "payment", 5, &[]);
    assert!(!result.is_empty());
    // Payment entry should be recalled
    assert!(result.iter().any(|s| s.content.contains("payment")));
}

#[test]
fn test_auto_recall_exclude_ids_filtering() {
    let entries = vec![
        make_entry(
            "e1",
            "Payment protocol information and details",
            MemoryCategory::Knowledge,
            1,
        ),
        make_entry(
            "e2",
            "Another payment-related memory entry",
            MemoryCategory::Knowledge,
            2,
        ),
    ];
    let (_dir, idx) = build_test_index(&entries);

    // Exclude e1 — should only get e2
    let result = auto_recall(&idx, "payment", 5, &["e1".to_string()]);
    assert!(!result.is_empty());
    assert!(result.iter().all(|s| s.id != "e1"));
}

#[test]
fn test_auto_recall_exclude_all() {
    let entries = vec![make_entry(
        "e1",
        "Payment protocol details for agents",
        MemoryCategory::Knowledge,
        1,
    )];
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "payment", 5, &["e1".to_string()]);
    assert!(result.is_empty());
}

#[test]
fn test_auto_recall_respects_limit() {
    let entries: Vec<MemoryEntry> = (0..10)
        .map(|i| {
            make_entry(
                &format!("e{i}"),
                &format!("Payment protocol variant number {i} with details"),
                MemoryCategory::Knowledge,
                i as i64,
            )
        })
        .collect();
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "payment protocol", 3, &[]);
    assert!(result.len() <= 3);
}

#[test]
fn test_auto_recall_recency_weighting() {
    // Two entries with similar BM25 scores (same word "payment" frequency)
    // but different ages — newer should rank higher
    let entries = vec![
        make_entry(
            "old",
            "payment protocol information for agents",
            MemoryCategory::Knowledge,
            720, // 30 days old
        ),
        make_entry(
            "new",
            "payment protocol information for agents",
            MemoryCategory::Knowledge,
            0, // just created
        ),
    ];
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "payment protocol", 2, &[]);
    assert!(result.len() == 2);
    // Newer entry should come first due to recency weighting
    assert_eq!(result[0].id, "new");
}

#[test]
fn test_auto_recall_mmr_diversity() {
    // Three entries: two very similar (about x402 payment) and one different
    // (about memory search). MMR should prefer diversity.
    let entries = vec![
        make_entry_with_tags(
            "e1",
            "The x402 payment protocol enables micropayments for AI inference services",
            vec!["payment", "x402"],
            1,
        ),
        make_entry_with_tags(
            "e2",
            "The x402 payment protocol enables micropayments for LLM inference calls",
            vec!["payment", "x402"],
            2,
        ),
        make_entry_with_tags(
            "e3",
            "Memory search uses BM25 ranking for relevance scoring of stored entries",
            vec!["memory", "search"],
            1,
        ),
    ];
    let (_dir, idx) = build_test_index(&entries);

    // Query matches all three but e1 and e2 are very similar
    let result = auto_recall(&idx, "x402 payment protocol memory search", 3, &[]);

    // All three should be returned (pool is big enough)
    assert_eq!(result.len(), 3);

    // Due to MMR diversity, both the payment AND memory topics should appear
    let has_payment = result.iter().any(|s| s.content.contains("payment"));
    let has_memory = result.iter().any(|s| s.content.contains("Memory search"));
    assert!(has_payment, "Should include payment-related entries");
    assert!(
        has_memory,
        "Should include memory-related entry due to MMR diversity"
    );
}

#[test]
fn test_auto_recall_zero_limit() {
    let entries = vec![make_entry(
        "e1",
        "Payment protocol",
        MemoryCategory::Knowledge,
        1,
    )];
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "payment", 0, &[]);
    assert!(result.is_empty());
}

#[test]
fn test_auto_recall_whitespace_query() {
    let entries = vec![make_entry(
        "e1",
        "Payment protocol",
        MemoryCategory::Knowledge,
        1,
    )];
    let (_dir, idx) = build_test_index(&entries);
    let result = auto_recall(&idx, "   ", 5, &[]);
    assert!(result.is_empty());
}

// ===========================================================================
// Content truncation edge cases
// ===========================================================================

#[test]
fn test_format_short_content_no_truncation() {
    let snippets = vec![make_snippet("s1", "knowledge", "Short content.", 0.9, 1)];
    let result = format_recalled_memories(&snippets);
    assert!(result.contains("Short content."));
    assert!(!result.contains("..."));
}

#[test]
fn test_format_exactly_300_chars() {
    // Use words to avoid first-line heading truncation triggering "..."
    let content = "abc ".repeat(75); // 300 chars
    let snippets = vec![make_snippet("s1", "knowledge", &content, 0.9, 1)];
    let result = format_recalled_memories(&snippets);
    // Content body should not be truncated (exactly 300 chars)
    // Count "..." occurrences — the heading may include "..." if first line > 80 chars,
    // but the content body should not have truncation
    let content_section = result.split("(relevance:").nth(1).unwrap_or("");
    // After the score, the next line is the content — should not end with "..."
    let after_score = content_section.split('\n').nth(1).unwrap_or("");
    assert!(
        !after_score.ends_with("..."),
        "Content body should not be truncated for exactly 300 chars: {after_score}"
    );
}

#[test]
fn test_format_301_chars_truncated() {
    let content = "x ".repeat(151); // 302 chars with spaces
    let snippets = vec![make_snippet("s1", "knowledge", &content, 0.9, 1)];
    let result = format_recalled_memories(&snippets);
    // Should be truncated
    assert!(result.contains("..."));
}

// ===========================================================================
// Integration: format with auto_recall pipeline
// ===========================================================================

#[test]
fn test_auto_recall_results_format_correctly() {
    let entries = vec![
        make_entry(
            "e1",
            "BSV wallet integration for micropayments",
            MemoryCategory::Knowledge,
            2,
        ),
        make_entry(
            "e2",
            "Agent loop architecture: observe, think, act, record",
            MemoryCategory::Session,
            1,
        ),
    ];
    let (_dir, idx) = build_test_index(&entries);

    let recalled = auto_recall(&idx, "agent wallet", 5, &[]);
    let formatted = format_recalled_memories(&recalled);

    if !recalled.is_empty() {
        assert!(formatted.contains("## Recalled Memories (auto)"));
        assert!(formatted.contains("relevance:"));
    }
}

// ---------------------------------------------------------------------------
// sanitize_query — strip tantivy special chars
// ---------------------------------------------------------------------------

#[test]
fn test_sanitize_query_strips_question_marks() {
    let result = sanitize_query("what are your capabilities?");
    assert!(!result.contains('?'));
    assert!(result.contains("what"));
    assert!(result.contains("capabilities"));
}

#[test]
fn test_sanitize_query_strips_colons_and_parens() {
    let result = sanitize_query("key: 034aa (test)");
    assert!(!result.contains(':'));
    assert!(!result.contains('('));
    assert!(!result.contains(')'));
    assert!(result.contains("key"));
    assert!(result.contains("034aa"));
    assert!(result.contains("test"));
}

#[test]
fn test_sanitize_query_strips_quotes_and_wildcards() {
    let result = sanitize_query("\"exact match\" with wild*");
    assert!(!result.contains('"'));
    assert!(!result.contains('*'));
    assert!(result.contains("exact"));
    assert!(result.contains("wild"));
}

#[test]
fn test_sanitize_query_collapses_whitespace() {
    let result = sanitize_query("hello???   world!!!");
    // Multiple special chars + spaces should collapse to single spaces
    assert!(!result.contains("  "), "no double spaces: '{result}'");
}

#[test]
fn test_sanitize_query_preserves_plain_text() {
    let input = "BSV wallet payment protocol";
    assert_eq!(sanitize_query(input), input);
}

#[test]
fn test_extract_query_hints_sanitizes_output() {
    // extract_query_hints should return sanitized text (no tantivy special chars)
    let result = extract_query_hints("what are your capabilities?", None);
    assert!(
        !result.contains('?'),
        "question mark should be stripped: '{result}'"
    );
    // "what", "are", "your" are stop words — "capabilities" should survive
    assert!(result.contains("capabilities"));
}

// ===========================================================================
// CI.3: BM25 key-term extraction tests
// ===========================================================================

#[test]
fn test_stop_words_filtered() {
    let result = extract_key_terms("the payment is going through and the wallet has been updated");
    // "the", "is", "going", "through", "and", "has", "been" are stop words
    assert!(!result.contains(" the "));
    assert!(!result.contains(" is "));
    assert!(!result.contains(" and "));
    assert!(!result.contains(" been "));
    // "payment", "wallet", "updated" should survive
    assert!(
        result.contains("payment"),
        "should keep 'payment': '{result}'"
    );
    assert!(
        result.contains("wallet"),
        "should keep 'wallet': '{result}'"
    );
    assert!(
        result.contains("updated"),
        "should keep 'updated': '{result}'"
    );
}

#[test]
fn test_proper_nouns_kept() {
    let result =
        extract_key_terms("we need to integrate Bitcoin and NanoStore for the payment system");
    // Proper nouns should be preserved and ranked highly
    assert!(
        result.contains("Bitcoin"),
        "should keep 'Bitcoin': '{result}'"
    );
    assert!(
        result.contains("NanoStore"),
        "should keep 'NanoStore': '{result}'"
    );
}

#[test]
fn test_domain_terms_kept() {
    let result = extract_key_terms(
        "The BRC31 authentication protocol uses x402 for wallet payment handling",
    );
    // Domain terms with mixed case/numbers should be preserved
    assert!(result.contains("BRC31"), "should keep 'BRC31': '{result}'");
    assert!(result.contains("x402"), "should keep 'x402': '{result}'");
    assert!(
        result.contains("authentication"),
        "should keep 'authentication': '{result}'"
    );
}

#[test]
fn test_empty_after_filtering() {
    // All stop words — should fall back gracefully
    let result = extract_key_terms("the and is are was were been being");
    // Should produce something (fallback) rather than empty
    assert!(!result.is_empty(), "should not be empty after fallback");
}

#[test]
fn test_short_input_passthrough() {
    // 1-3 word input should pass through as-is (after sanitization)
    let result = extract_key_terms("BSV wallet");
    assert_eq!(result, "BSV wallet");

    let result = extract_key_terms("payment");
    assert_eq!(result, "payment");

    let result = extract_key_terms("x402 payment protocol");
    assert_eq!(result, "x402 payment protocol");
}

#[test]
fn test_key_term_extraction_improves_recall() {
    // E2E quality test: store a memory with specific terms, then verify
    // that key-term extraction from a verbose query finds it.
    let entries = vec![
        make_entry(
            "e1",
            "BRC-31 Authrite authentication enables mutual identity verification between agents and services",
            MemoryCategory::Knowledge,
            1,
        ),
        make_entry(
            "e2",
            "The tantivy search engine provides BM25 ranked full-text retrieval",
            MemoryCategory::Knowledge,
            2,
        ),
        make_entry(
            "e3",
            "Budget tracking uses per-task and per-hour spending limits with JSONL audit",
            MemoryCategory::Knowledge,
            1,
        ),
    ];
    let (_dir, idx) = build_test_index(&entries);

    // Verbose query that would suffer from BM25 dilution without key terms
    let verbose_query = "I need to understand how the authentication protocol works and what BRC-31 means for the agent identity verification process";
    let key_terms = extract_key_terms(verbose_query);

    // Key terms should include the distinctive words
    assert!(
        key_terms.contains("authentication"),
        "key terms should include 'authentication': '{key_terms}'"
    );
    assert!(
        key_terms.contains("BRC"),
        "key terms should include 'BRC': '{key_terms}'"
    );

    // Search with extracted key terms should find the auth entry
    let results = idx.search(&key_terms, 5).unwrap();
    assert!(!results.is_empty(), "key-term search should return results");
    assert!(
        results.iter().any(|s| s.content.contains("Authrite")),
        "Should find the BRC-31 auth entry with key terms. Results: {:?}",
        results.iter().map(|s| &s.content).collect::<Vec<_>>()
    );
}

#[test]
fn test_extract_query_hints_no_regression() {
    // Existing extraction still works — task + recent text produces reasonable hints
    let result = extract_query_hints(
        "Configure the wallet for micropayments",
        Some("We previously set up BRC-31 authentication and x402 payment flow"),
    );
    // Should contain domain-specific terms, not just stop words
    assert!(!result.is_empty());
    assert!(
        result.contains("wallet") || result.contains("micropayments"),
        "should contain meaningful terms: '{result}'"
    );
    assert!(
        result.contains("BRC") || result.contains("x402"),
        "should contain domain terms from recent text: '{result}'"
    );
}

#[test]
fn test_key_terms_deduplication() {
    // Repeated words should be deduplicated
    let result = extract_key_terms("payment payment payment wallet wallet protocol");
    // Should not repeat "payment" multiple times
    let count = result.matches("payment").count();
    assert_eq!(count, 1, "payment should appear only once: '{result}'");
}

#[test]
fn test_key_terms_acronyms_scored_highly() {
    let result = extract_key_terms(
        "The BSV network uses the LLM API for inference through payment channels",
    );
    // Acronyms (all-caps, 2+ chars) should be scored highly
    assert!(result.contains("BSV"), "should keep 'BSV': '{result}'");
    assert!(result.contains("LLM"), "should keep 'LLM': '{result}'");
    assert!(result.contains("API"), "should keep 'API': '{result}'");
}
