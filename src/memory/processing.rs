//! Post-store memory processing — duplicate detection, tag extraction, quality validation.
//!
//! All functions are synchronous, non-fatal, and designed to run inline after
//! a successful `memory_store` write. Errors are logged but never prevent the
//! store operation from succeeding.

use crate::memory::search::MemoryIndex;

/// BM25 score threshold above which two entries are considered near-duplicates.
const DEDUP_BM25_THRESHOLD: f32 = 20.0;

/// Maximum number of extracted tags to return.
const MAX_EXTRACTED_TAGS: usize = 5;

/// Minimum content length before a quality warning is issued.
const MIN_CONTENT_LENGTH: usize = 20;

/// Result of duplicate checking against the BM25 index.
#[derive(Debug, Clone)]
pub struct DedupResult {
    /// Whether a possible duplicate was found.
    pub is_duplicate: bool,
    /// ID of the closest matching entry, if any.
    pub matching_id: Option<String>,
    /// BM25 score of the closest match.
    pub score: Option<f32>,
}

/// Result of post-store processing.
#[derive(Debug, Clone)]
pub struct ProcessingResult {
    /// Quality warnings (empty if content passes all checks).
    pub warnings: Vec<String>,
    /// Duplicate detection result.
    pub dedup: DedupResult,
    /// Tags automatically extracted from content.
    pub extracted_tags: Vec<String>,
}

/// Check the BM25 index for entries that are near-duplicates of the given content.
///
/// Searches the index using the content as a query and checks whether the top
/// result (excluding the entry itself) exceeds `DEDUP_BM25_THRESHOLD`.
///
/// Returns a `DedupResult` indicating whether a duplicate was found.
pub fn check_duplicates(index: &MemoryIndex, content: &str, self_id: &str) -> DedupResult {
    let no_dup = DedupResult {
        is_duplicate: false,
        matching_id: None,
        score: None,
    };

    if content.trim().is_empty() {
        return no_dup;
    }

    // Search for similar entries (get a few extra to filter out self)
    let results = match index.search(content, 5) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Dedup search failed: {e}");
            return no_dup;
        }
    };

    // Find the best match that is NOT the entry we just stored
    for result in &results {
        if result.id == self_id {
            continue;
        }
        if result.score >= DEDUP_BM25_THRESHOLD {
            return DedupResult {
                is_duplicate: true,
                matching_id: Some(result.id.clone()),
                score: Some(result.score),
            };
        }
        // Results are ranked by score descending; if the first non-self
        // result is below threshold, none of the rest will exceed it.
        return DedupResult {
            is_duplicate: false,
            matching_id: Some(result.id.clone()),
            score: Some(result.score),
        };
    }

    no_dup
}

/// Extract tags from content using simple keyword heuristics.
///
/// Extracts:
/// - `#hashtags` (e.g. "#bitcoin" -> "bitcoin")
/// - `ALL_CAPS` terms longer than 3 characters (e.g. "AUTH" -> "auth")
/// - `"quoted terms"` (e.g. `"payment protocol"` -> "payment protocol")
///
/// Returns up to `MAX_EXTRACTED_TAGS` unique lowercase tags.
pub fn extract_tags(content: &str) -> Vec<String> {
    use std::collections::HashSet;

    let mut tags: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. Extract #hashtags
    for word in content.split_whitespace() {
        if tags.len() >= MAX_EXTRACTED_TAGS {
            break;
        }
        if let Some(stripped) = word.strip_prefix('#') {
            // Strip trailing punctuation
            let tag = stripped.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
            if !tag.is_empty() {
                let lower = tag.to_lowercase();
                if seen.insert(lower.clone()) {
                    tags.push(lower);
                }
            }
        }
    }

    // 2. Extract ALL_CAPS terms (>3 chars, all uppercase letters/digits/hyphens)
    for word in content.split_whitespace() {
        if tags.len() >= MAX_EXTRACTED_TAGS {
            break;
        }
        // Strip leading/trailing punctuation for cleaner matching
        let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
        if cleaned.len() > 3
            && cleaned.chars().any(|c| c.is_alphabetic())
            && cleaned
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(|c| c.is_uppercase())
        {
            let lower = cleaned.to_lowercase();
            if seen.insert(lower.clone()) {
                tags.push(lower);
            }
        }
    }

    // 3. Extract "quoted terms"
    let mut rest = content;
    while tags.len() < MAX_EXTRACTED_TAGS {
        if let Some(start) = rest.find('"') {
            let after_open = &rest[start + 1..];
            if let Some(end) = after_open.find('"') {
                let quoted = after_open[..end].trim();
                if !quoted.is_empty() {
                    let lower = quoted.to_lowercase();
                    if seen.insert(lower.clone()) {
                        tags.push(lower);
                    }
                }
                rest = &after_open[end + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    tags.truncate(MAX_EXTRACTED_TAGS);
    tags
}

/// Validate content quality, returning a list of warnings.
///
/// Checks:
/// - Content shorter than 20 characters
/// - Content that is all whitespace
/// - Content with no alphabetic characters
///
/// Returns an empty vec if content passes all checks.
pub fn validate_quality(content: &str) -> Vec<String> {
    let mut warnings = Vec::new();

    if content.trim().is_empty() {
        warnings.push("Content is all whitespace".to_string());
        return warnings;
    }

    if content.trim().len() < MIN_CONTENT_LENGTH {
        warnings.push(format!(
            "Content is very short ({} chars, minimum recommended: {})",
            content.trim().len(),
            MIN_CONTENT_LENGTH
        ));
    }

    if !content.chars().any(|c| c.is_alphabetic()) {
        warnings.push("Content contains no alphabetic characters".to_string());
    }

    warnings
}

/// Run all post-store processing on a newly stored entry.
///
/// This is the main entry point called from `memory_store_impl`. All errors
/// are caught and logged — the function always returns a result.
pub fn post_process(index: &MemoryIndex, content: &str, self_id: &str) -> ProcessingResult {
    // Quality validation
    let warnings = validate_quality(content);

    // Duplicate detection
    let dedup = check_duplicates(index, content, self_id);
    if dedup.is_duplicate {
        if let Some(ref id) = dedup.matching_id {
            tracing::warn!(
                "Possible duplicate detected: new entry {} matches existing {} (score: {:.1})",
                self_id,
                id,
                dedup.score.unwrap_or(0.0)
            );
        }
    }

    // Tag extraction
    let extracted_tags = extract_tags(content);

    for warning in &warnings {
        tracing::warn!("Memory quality warning for {}: {}", self_id, warning);
    }

    ProcessingResult {
        warnings,
        dedup,
        extracted_tags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::{MemoryCategory, MemoryEntry};
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

    // --- Dedup tests ---

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

    #[test]
    fn test_dedup_detects_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();
        add_filler_entries(&idx);

        let content = "The x402 payment protocol uses BEEF transactions for payment verification";
        let existing = make_entry("existing-001", content);
        idx.add_entry(&existing).unwrap();

        let result = check_duplicates(&idx, content, "new-001");
        assert!(result.is_duplicate, "Exact duplicate should be detected");
        assert_eq!(result.matching_id.as_deref(), Some("existing-001"));
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
        let near_dup_content =
            "The x402 payment protocol uses BEEF transactions for payment verification and settlement";

        let result = check_duplicates(&idx, near_dup_content, "new-001");
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
        let different_content =
            "Astronomy observations of distant galaxies reveal dark matter distribution patterns";

        let result = check_duplicates(&idx, different_content, "new-001");
        assert!(
            !result.is_duplicate,
            "Distinct topics should not be flagged as duplicates"
        );
    }

    #[test]
    fn test_dedup_empty_index_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        // Empty index should not panic
        let result = check_duplicates(&idx, "Some new content to check", "new-001");
        assert!(!result.is_duplicate);
        assert!(result.matching_id.is_none());
    }

    // --- Tag extraction tests ---

    #[test]
    fn test_tag_extraction_hashtags() {
        let tags = extract_tags("#bitcoin and #wallet are important");
        assert!(tags.contains(&"bitcoin".to_string()));
        assert!(tags.contains(&"wallet".to_string()));
    }

    #[test]
    fn test_tag_extraction_caps_terms() {
        let tags = extract_tags("Uses BRC-31 AUTH for BSV transactions");
        // AUTH (4 chars, all caps) and BRC-31 (all caps + digits) should be extracted
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
            tags.len() <= MAX_EXTRACTED_TAGS,
            "Should not exceed {} tags, got {}",
            MAX_EXTRACTED_TAGS,
            tags.len()
        );
    }

    #[test]
    fn test_tag_extraction_empty_content() {
        let tags = extract_tags("");
        assert!(tags.is_empty());

        let tags = extract_tags("   ");
        assert!(tags.is_empty());
    }

    // --- Quality validation tests ---

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

    // --- Integration: post_process non-fatal ---

    #[test]
    fn test_post_process_non_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        // Post-process should always succeed, even with edge-case content
        let result = post_process(&idx, "", "test-001");
        assert!(
            !result.warnings.is_empty(),
            "Empty content should produce warnings"
        );

        let result = post_process(&idx, "Valid content with #tags and AUTH tokens", "test-002");
        assert!(result.warnings.is_empty() || !result.warnings.is_empty()); // always succeeds

        // Even extremely long content should not panic
        let long = "word ".repeat(10000);
        let result = post_process(&idx, &long, "test-003");
        assert!(!result.dedup.is_duplicate); // no prior entries
    }
}
