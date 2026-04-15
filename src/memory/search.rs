//! Tantivy BM25 full-text search over stored memories.
//!
//! Maintains a tantivy index alongside the file-based store,
//! enabling fast ranked retrieval by content similarity.

use std::path::PathBuf;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexWriter, ReloadPolicy, TantivyDocument};

use crate::error::DmError;
use crate::memory::store::MemoryEntry;

/// A search result snippet from the memory index.
#[derive(Debug, Clone)]
pub struct MemorySnippet {
    /// Memory entry ID.
    pub id: String,
    /// Category (knowledge, session, execution).
    pub category: String,
    /// Matched content (possibly truncated).
    pub content: String,
    /// BM25 relevance score.
    pub score: f32,
    /// Associated tags.
    pub tags: Vec<String>,
    /// Creation timestamp (RFC 3339).
    pub created: String,
}

/// Tantivy-backed BM25 search index for memory entries.
pub struct MemoryIndex {
    index: Index,
    schema: Schema,
    id_field: Field,
    category_field: Field,
    content_field: Field,
    tags_field: Field,
    created_field: Field,
}

impl std::fmt::Debug for MemoryIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryIndex")
            .field("schema", &self.schema)
            .finish()
    }
}

impl MemoryIndex {
    /// Create or open a tantivy index at the given directory.
    ///
    /// If the directory does not exist, it is created. If the index
    /// is corrupted, it is rebuilt from scratch.
    pub fn new(index_dir: PathBuf) -> Result<Self, DmError> {
        std::fs::create_dir_all(&index_dir).map_err(|e| {
            DmError::memory(format!(
                "failed to create index dir {}: {e}",
                index_dir.display()
            ))
        })?;

        let mut schema_builder = Schema::builder();
        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let category_field = schema_builder.add_text_field("category", STRING | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let tags_field = schema_builder.add_text_field("tags", TEXT | STORED);
        let created_field = schema_builder.add_text_field("created", STRING | STORED);
        let schema = schema_builder.build();

        // Try to open existing index, fall back to creating new one
        let index = match Index::open_in_dir(&index_dir) {
            Ok(idx) => {
                // Verify schema compatibility
                if idx.schema() == schema {
                    tracing::debug!("Opened existing memory index at {}", index_dir.display());
                    idx
                } else {
                    tracing::warn!(
                        "Memory index schema mismatch at {}, rebuilding",
                        index_dir.display()
                    );
                    Self::recreate_index(&index_dir, &schema)?
                }
            }
            Err(_) => {
                tracing::debug!("Creating new memory index at {}", index_dir.display());
                Index::create_in_dir(&index_dir, schema.clone())
                    .map_err(|e| DmError::memory(format!("failed to create tantivy index: {e}")))?
            }
        };

        Ok(Self {
            index,
            schema,
            id_field,
            category_field,
            content_field,
            tags_field,
            created_field,
        })
    }

    /// Destroy and recreate the index in a directory.
    fn recreate_index(index_dir: &PathBuf, schema: &Schema) -> Result<Index, DmError> {
        // Remove old index files
        if let Ok(entries) = std::fs::read_dir(index_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        Index::create_in_dir(index_dir, schema.clone())
            .map_err(|e| DmError::memory(format!("failed to recreate tantivy index: {e}")))
    }

    /// Get an index writer with a 50MB heap budget.
    fn writer(&self) -> Result<IndexWriter, DmError> {
        self.index
            .writer(50_000_000)
            .map_err(|e| DmError::memory(format!("failed to create index writer: {e}")))
    }

    /// Add or update a memory entry in the index.
    pub fn add_entry(&self, entry: &MemoryEntry) -> Result<(), DmError> {
        let mut writer = self.writer()?;

        // Delete existing document with this ID first (upsert)
        let id_term = tantivy::Term::from_field_text(self.id_field, &entry.id);
        writer.delete_term(id_term);

        let tags_str = entry.tags.join(" ");

        writer
            .add_document(doc!(
                self.id_field => entry.id.as_str(),
                self.category_field => entry.category.to_string(),
                self.content_field => entry.content.as_str(),
                self.tags_field => tags_str.as_str(),
                self.created_field => entry.created.to_rfc3339(),
            ))
            .map_err(|e| DmError::memory(format!("failed to add document: {e}")))?;

        writer
            .commit()
            .map_err(|e| DmError::memory(format!("failed to commit index: {e}")))?;

        // Wait for background merge threads to finish so that subsequent
        // readers see a fully consistent segment state.  Without this,
        // BM25 per-segment avgdl can vary depending on merge timing,
        // producing non-deterministic scores.
        writer
            .wait_merging_threads()
            .map_err(|e| DmError::memory(format!("failed waiting for merge threads: {e}")))?;

        tracing::debug!("Indexed memory entry {}", entry.id);
        Ok(())
    }

    /// Search the index with a text query, returning ranked results.
    ///
    /// Uses BM25 scoring on the content field.
    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<MemorySnippet>, DmError> {
        if query_str.trim().is_empty() {
            return Ok(Vec::new());
        }

        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| DmError::memory(format!("failed to create index reader: {e}")))?;

        let searcher = reader.searcher();

        let query_parser =
            QueryParser::for_index(&self.index, vec![self.content_field, self.tags_field]);

        let query = query_parser.parse_query(query_str).map_err(|e| {
            DmError::memory(format!("failed to parse search query '{query_str}': {e}"))
        })?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| DmError::memory(format!("search failed: {e}")))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| DmError::memory(format!("failed to retrieve document: {e}")))?;

            let id = Self::get_text(&doc, self.id_field).unwrap_or_default();
            let category = Self::get_text(&doc, self.category_field).unwrap_or_default();
            let content = Self::get_text(&doc, self.content_field).unwrap_or_default();
            let tags_str = Self::get_text(&doc, self.tags_field).unwrap_or_default();
            let created = Self::get_text(&doc, self.created_field).unwrap_or_default();

            let tags: Vec<String> = tags_str.split_whitespace().map(|s| s.to_string()).collect();

            results.push(MemorySnippet {
                id,
                category,
                content,
                score,
                tags,
                created,
            });
        }

        tracing::debug!(
            "Search for '{}' returned {} results",
            query_str,
            results.len()
        );
        Ok(results)
    }

    /// Delete a document from the index by ID.
    pub fn delete_entry(&self, id: &str) -> Result<(), DmError> {
        let mut writer = self.writer()?;
        let id_term = tantivy::Term::from_field_text(self.id_field, id);
        writer.delete_term(id_term);
        writer
            .commit()
            .map_err(|e| DmError::memory(format!("failed to commit delete: {e}")))?;
        writer
            .wait_merging_threads()
            .map_err(|e| DmError::memory(format!("failed waiting for merge threads: {e}")))?;
        tracing::debug!("Deleted memory entry {} from index", id);
        Ok(())
    }

    /// Clear the entire index and re-add all entries.
    ///
    /// Useful for rebuilding from the file-based store.
    pub fn rebuild(&self, entries: &[MemoryEntry]) -> Result<(), DmError> {
        let mut writer = self.writer()?;

        // Clear all existing documents
        writer
            .delete_all_documents()
            .map_err(|e| DmError::memory(format!("failed to clear index: {e}")))?;

        // Re-add all entries using the same writer
        for entry in entries {
            let tags_str = entry.tags.join(" ");
            writer
                .add_document(doc!(
                    self.id_field => entry.id.as_str(),
                    self.category_field => entry.category.to_string(),
                    self.content_field => entry.content.as_str(),
                    self.tags_field => tags_str.as_str(),
                    self.created_field => entry.created.to_rfc3339(),
                ))
                .map_err(|e| DmError::memory(format!("failed to add document: {e}")))?;
        }

        writer
            .commit()
            .map_err(|e| DmError::memory(format!("failed to commit rebuild: {e}")))?;

        // Wait for background merge threads to complete so that the next
        // reader sees a single clean segment rather than stale pre-delete
        // segments.  This fixes flaky entry_count() after rebuild().
        writer
            .wait_merging_threads()
            .map_err(|e| DmError::memory(format!("failed waiting for merge threads: {e}")))?;

        tracing::info!("Rebuilt memory index with {} entries", entries.len());
        Ok(())
    }

    /// Count the number of documents in the index.
    pub fn entry_count(&self) -> Result<usize, DmError> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| DmError::memory(format!("failed to create index reader: {e}")))?;

        let searcher = reader.searcher();
        let count = searcher
            .segment_readers()
            .iter()
            .map(|sr| sr.num_docs() as usize)
            .sum();
        Ok(count)
    }

    /// Extract a text field value from a tantivy document.
    fn get_text(doc: &TantivyDocument, field: Field) -> Option<String> {
        doc.get_first(field)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Phase 9.2: Automatic BM25 Memory Recall
// ---------------------------------------------------------------------------

/// Compute Jaccard similarity between two strings based on their word sets.
///
/// Splits on whitespace, lowercases, computes |A ∩ B| / |A ∪ B|.
/// Returns 0.0 if both sets are empty.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;

    let set_a: HashSet<String> = a.split_whitespace().map(|w| w.to_lowercase()).collect();
    let set_b: HashSet<String> = b.split_whitespace().map(|w| w.to_lowercase()).collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Automatically recall relevant memories using BM25 search with MMR diversity
/// and recency weighting. Returns up to `limit` results.
///
/// This runs on every agent iteration to inject relevant memories into the
/// system prompt. The tantivy BM25 search is sub-millisecond, and the MMR
/// re-ranking operates on a small candidate pool (~15 entries).
///
/// Steps:
/// 1. Search the index with a generous initial limit to get a candidate pool
/// 2. Filter out entries whose `id` is in `exclude_ids` (de-duplication)
/// 3. Apply recency weighting to boost recent entries
/// 4. Apply MMR (Maximal Marginal Relevance) re-ranking for diversity
/// 5. Return selected results sorted by recency-adjusted score
pub fn auto_recall(
    index: &MemoryIndex,
    query: &str,
    limit: usize,
    exclude_ids: &[String],
) -> Vec<MemorySnippet> {
    if query.trim().is_empty() || limit == 0 {
        return Vec::new();
    }

    // Step 1: Get a generous candidate pool
    let pool_size = (limit * 3).max(15);
    let candidates = match index.search(query, pool_size) {
        Ok(results) => results,
        Err(e) => {
            tracing::warn!("Auto-recall search failed: {e}");
            return Vec::new();
        }
    };

    if candidates.is_empty() {
        return Vec::new();
    }

    // Step 2: Filter out excluded IDs
    let candidates: Vec<MemorySnippet> = candidates
        .into_iter()
        .filter(|s| !exclude_ids.contains(&s.id))
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    // Step 3: Apply recency weighting
    let now = chrono::Utc::now();
    let weighted: Vec<(MemorySnippet, f64)> = candidates
        .into_iter()
        .map(|snippet| {
            let age_hours = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&snippet.created) {
                let duration = now.signed_duration_since(dt);
                (duration.num_minutes() as f64 / 60.0).max(0.0)
            } else {
                // If we can't parse the date, assume moderate age (24h)
                24.0
            };
            let recency_weight = 1.0 / (1.0 + age_hours / 24.0);
            let adjusted_score = snippet.score as f64 * recency_weight;
            (snippet, adjusted_score)
        })
        .collect();

    // Step 4: MMR re-ranking
    let selected = mmr_select(&weighted, limit, 0.7);

    // Step 5: Sort by adjusted score (highest first) — already done by MMR,
    // but re-sort to ensure consistent ordering
    let mut result: Vec<MemorySnippet> = selected;
    result.sort_by(|a, b| {
        // Re-compute adjusted scores for sorting (lightweight)
        let score_a = compute_adjusted_score(a, now);
        let score_b = compute_adjusted_score(b, now);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    result
}

/// Compute the recency-adjusted score for a snippet.
fn compute_adjusted_score(snippet: &MemorySnippet, now: chrono::DateTime<chrono::Utc>) -> f64 {
    let age_hours = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&snippet.created) {
        let duration = now.signed_duration_since(dt);
        (duration.num_minutes() as f64 / 60.0).max(0.0)
    } else {
        24.0
    };
    let recency_weight = 1.0 / (1.0 + age_hours / 24.0);
    snippet.score as f64 * recency_weight
}

/// MMR (Maximal Marginal Relevance) selection for diverse results.
///
/// Given a pool of (snippet, adjusted_score) pairs, selects up to `limit`
/// results that balance relevance and diversity.
///
/// `lambda` controls the trade-off: 1.0 = pure relevance, 0.0 = pure diversity.
fn mmr_select(pool: &[(MemorySnippet, f64)], limit: usize, lambda: f64) -> Vec<MemorySnippet> {
    if pool.is_empty() {
        return Vec::new();
    }

    let mut selected: Vec<usize> = Vec::with_capacity(limit);
    let mut remaining: Vec<usize> = (0..pool.len()).collect();

    // Start with highest-scoring result
    let best_idx = remaining
        .iter()
        .copied()
        .max_by(|&a, &b| {
            pool[a]
                .1
                .partial_cmp(&pool[b].1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap(); // pool is non-empty

    selected.push(best_idx);
    remaining.retain(|&i| i != best_idx);

    // Select remaining via MMR
    while selected.len() < limit && !remaining.is_empty() {
        let mut best_mmr_score = f64::NEG_INFINITY;
        let mut best_candidate = remaining[0];

        for &candidate_idx in &remaining {
            let relevance = pool[candidate_idx].1;

            // Max similarity to any already-selected item
            let max_sim = selected
                .iter()
                .map(|&sel_idx| {
                    jaccard_similarity(&pool[candidate_idx].0.content, &pool[sel_idx].0.content)
                })
                .fold(0.0_f64, f64::max);

            let mmr_score = lambda * relevance - (1.0 - lambda) * max_sim;

            if mmr_score > best_mmr_score {
                best_mmr_score = mmr_score;
                best_candidate = candidate_idx;
            }
        }

        selected.push(best_candidate);
        remaining.retain(|&i| i != best_candidate);
    }

    selected.into_iter().map(|i| pool[i].0.clone()).collect()
}

/// Common English stop words filtered from BM25 queries.
///
/// Based on arscontexta research: "BM25 retrieval fails on full-length
/// descriptions because query term dilution reduces match scores."
/// Condensing to 3-5 key terms restores retrieval that full descriptions miss.
const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "aren't",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "can't",
    "cannot",
    "could",
    "couldn't",
    "did",
    "didn't",
    "do",
    "does",
    "doesn't",
    "doing",
    "don't",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "get",
    "got",
    "had",
    "hadn't",
    "has",
    "hasn't",
    "have",
    "haven't",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "i",
    "if",
    "in",
    "into",
    "is",
    "isn't",
    "it",
    "it's",
    "its",
    "itself",
    "just",
    "let",
    "me",
    "might",
    "more",
    "most",
    "mustn't",
    "my",
    "myself",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "please",
    "same",
    "she",
    "should",
    "shouldn't",
    "so",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "wasn't",
    "we",
    "were",
    "weren't",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "will",
    "with",
    "won't",
    "would",
    "wouldn't",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
    // Common filler in agent context
    "need",
    "want",
    "like",
    "going",
    "also",
    "using",
    "use",
    "make",
    "think",
    "know",
    "look",
    "help",
    "try",
    "take",
    "give",
    "tell",
    "say",
    "see",
    "come",
    "go",
    "find",
    "way",
    "thing",
    "things",
];

/// Target range for extracted key terms (max).
const KEY_TERMS_MAX: usize = 8;

/// Extract key terms from raw text for BM25 search.
///
/// Filters stop words and ranks remaining terms by specificity:
/// 1. Proper nouns (capitalized, not sentence-start) — e.g. "Bitcoin", "NanoStore"
/// 2. Hyphenated/technical terms — e.g. "BRC-31", "x402"
/// 3. Longer words (more specific)
/// 4. Shorter common words
///
/// Returns 3-8 key terms joined by spaces. Falls back to sanitized raw text
/// if filtering removes everything.
pub fn extract_key_terms(raw_text: &str) -> String {
    use std::collections::HashSet;

    let stop_set: HashSet<&str> = STOP_WORDS.iter().copied().collect();

    // Sanitize first to remove tantivy special chars
    let sanitized = sanitize_query(raw_text);
    let words: Vec<&str> = sanitized.split_whitespace().collect();

    // Short input (1-3 words): pass through as-is
    if words.len() <= 3 {
        return sanitized;
    }

    // Tokenize and filter stop words
    let mut terms: Vec<(&str, u32)> = Vec::new();
    let mut seen = HashSet::new();

    for (i, word) in words.iter().enumerate() {
        let lower = word.to_lowercase();
        if stop_set.contains(lower.as_str()) {
            continue;
        }
        // Skip very short words (1 char) unless they look technical
        if word.len() <= 1 && !word.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // Deduplicate (case-insensitive)
        if !seen.insert(lower.clone()) {
            continue;
        }

        // Score by specificity
        let mut score: u32 = 10; // base

        // Proper noun: capitalized and not sentence-start (index > 0 and previous word doesn't end with '.')
        let is_sentence_start = i == 0
            || words
                .get(i.wrapping_sub(1))
                .is_some_and(|prev| prev.ends_with('.'));
        if word.starts_with(|c: char| c.is_uppercase()) && !is_sentence_start {
            score += 30; // Proper nouns are highly specific
        }

        // Technical/hyphenated terms (original raw text had hyphen — sanitized strips it,
        // but we can detect patterns like "BRC" prefix, all-caps, digits mixed with letters)
        if word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(|c| c.is_alphabetic()) {
            score += 25; // e.g. "x402", "BRC31"
        }
        if word.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()) && word.len() >= 2 {
            score += 20; // Acronyms: "BSV", "LLM", "API"
        }

        // Longer words tend to be more specific
        score += (word.len() as u32).min(15);

        terms.push((word, score));
    }

    // If filtering removed everything, fall back to raw sanitized text (truncated)
    if terms.is_empty() {
        let fallback = sanitized.chars().take(200).collect::<String>();
        return fallback.trim().to_string();
    }

    // Sort by score descending, take top KEY_TERMS_MAX
    terms.sort_by(|a, b| b.1.cmp(&a.1));
    let take = terms.len().min(KEY_TERMS_MAX);
    let selected: Vec<&str> = terms[..take].iter().map(|(w, _)| *w).collect();

    selected.join(" ")
}

/// Extract search query hints from task description and recent context.
///
/// Returns a consolidated query string for BM25 search. Combines the task
/// description with the tail of the last assistant response, then extracts
/// key terms to avoid BM25 query dilution (arscontexta research finding).
pub fn extract_query_hints(task_description: &str, recent_assistant_text: Option<&str>) -> String {
    let mut query = task_description.to_string();

    if let Some(text) = recent_assistant_text {
        let text = text.trim();
        if !text.is_empty() {
            // Take the last 200 chars, trimmed to a word boundary
            let suffix = if text.len() > 200 {
                let start = text.len() - 200;
                // Find the first whitespace after `start` to avoid cutting mid-word
                let adjusted_start = text[start..]
                    .find(char::is_whitespace)
                    .map(|pos| start + pos + 1)
                    .unwrap_or(start);
                &text[adjusted_start..]
            } else {
                text
            };
            query.push(' ');
            query.push_str(suffix);
        }
    }

    // Truncate total to 500 chars max, respecting word boundaries
    if query.len() > 500 {
        let safe_end = query.floor_char_boundary(500);
        let end = query[..safe_end]
            .rfind(char::is_whitespace)
            .unwrap_or(safe_end);
        query.truncate(end);
    }

    // Extract key terms instead of passing raw text to BM25
    // (arscontexta: condensing to 3-8 key terms restores retrieval)
    extract_key_terms(&query)
}

/// Strip characters that are special in tantivy's query syntax.
///
/// Tantivy's QueryParser treats `?`, `:`, `(`, `)`, `"`, `~`, `^`, `*`,
/// `+`, `-`, `[`, `]`, `{`, `}`, `\` as operators. Natural language queries
/// (task descriptions, assistant responses) contain these frequently.
/// Rather than escaping them (which changes semantics), we strip them
/// so the parser sees only plain terms.
pub fn sanitize_query(query: &str) -> String {
    query
        .chars()
        .map(|c| match c {
            '?' | ':' | '(' | ')' | '"' | '~' | '^' | '*' | '+' | '[' | ']' | '{' | '}' | '\\' => {
                ' '
            }
            // Keep hyphens only between word chars (strip leading/trailing)
            '-' => ' ',
            _ => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Memory tamper detection (H.3)
// ---------------------------------------------------------------------------

/// A tamper warning emitted when a recalled memory's content hash does not match
/// the hash stored in its `.proof` sidecar file.
#[derive(Debug, Clone)]
pub struct TamperWarning {
    /// Memory entry ID.
    pub entry_id: String,
    /// Expected content hash from the proof sidecar.
    pub expected_hash: String,
    /// Actual computed content hash.
    pub actual_hash: String,
}

/// Verify recalled memory entries against their `.proof` sidecar files.
///
/// For each snippet, this function:
/// 1. Looks up the `.proof` sidecar via `store.read_proof_sidecar()`
/// 2. If a sidecar exists, recomputes `SHA-256(content || created_at || category)`
/// 3. If the computed hash mismatches the stored hash, produces a `TamperWarning`
/// 4. If no sidecar exists, the entry is silently skipped (backward compatible)
///
/// Returns a list of tamper warnings (empty if all entries are verified or have no sidecars).
pub fn verify_recalled_memories(
    memories: &[MemorySnippet],
    store: &crate::memory::store::MemoryStore,
) -> Vec<TamperWarning> {
    use crate::onchain::proofs::compute_memory_hash;
    use std::str::FromStr;

    let mut warnings = Vec::new();

    for snippet in memories {
        // Parse category from string back to enum for sidecar lookup
        let category = match crate::memory::store::MemoryCategory::from_str(&snippet.category) {
            Ok(cat) => cat,
            Err(_) => continue,
        };

        // Look up proof sidecar
        let sidecar = store.read_proof_sidecar(&snippet.id, &category);
        let (_txid, expected_hash) = match sidecar {
            Some(pair) => pair,
            None => continue, // No sidecar — skip verification (backward compat)
        };

        // Recompute content hash
        let actual_hash =
            compute_memory_hash(&snippet.content, &snippet.created, &snippet.category);

        if actual_hash != expected_hash {
            tracing::warn!(
                "TAMPER DETECTED: memory {} hash mismatch (expected={}, actual={})",
                snippet.id,
                expected_hash,
                actual_hash,
            );
            warnings.push(TamperWarning {
                entry_id: snippet.id.clone(),
                expected_hash,
                actual_hash,
            });
        }
    }

    warnings
}

/// Format tamper warnings as a markdown section for injection into the system prompt.
///
/// Returns an empty string if there are no warnings.
pub fn format_tamper_warnings(warnings: &[TamperWarning]) -> String {
    if warnings.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        "## TAMPER WARNINGS".to_string(),
        String::new(),
        "The following recalled memories have been modified since they were stored. \
         Their content hashes do not match the on-chain proof records. \
         DO NOT trust these entries."
            .to_string(),
        String::new(),
    ];

    for w in warnings {
        lines.push(format!(
            "- **{}**: expected hash `{}`, got `{}`",
            w.entry_id,
            &w.expected_hash[..16],
            &w.actual_hash[..16],
        ));
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Format auto-recalled memories as a markdown section for the system prompt.
///
/// Returns an empty string if `memories` is empty.
pub fn format_recalled_memories(memories: &[MemorySnippet]) -> String {
    if memories.is_empty() {
        return String::new();
    }

    let mut lines = vec!["## Recalled Memories (auto)".to_string(), String::new()];

    for snippet in memories {
        // Extract first line of content for the heading
        let first_line = snippet.content.lines().next().unwrap_or("").trim();

        // Truncate first line if too long (use floor_char_boundary to avoid
        // slicing inside multi-byte UTF-8 characters like em-dashes)
        let first_line = if first_line.len() > 80 {
            let safe_end = first_line.floor_char_boundary(80);
            let end = first_line[..safe_end]
                .rfind(char::is_whitespace)
                .unwrap_or(safe_end);
            format!("{}...", &first_line[..end])
        } else {
            first_line.to_string()
        };

        lines.push(format!(
            "**[{}] {}** (relevance: {:.2})",
            snippet.category, first_line, snippet.score
        ));

        // Truncate content to 300 chars at word boundary (use floor_char_boundary
        // to avoid slicing inside multi-byte UTF-8 characters)
        let content = if snippet.content.len() > 300 {
            let safe_end = snippet.content.floor_char_boundary(300);
            let end = snippet.content[..safe_end]
                .rfind(char::is_whitespace)
                .unwrap_or(safe_end);
            format!("{}...", &snippet.content[..end])
        } else {
            snippet.content.clone()
        };

        lines.push(content);
        lines.push(String::new());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::MemoryCategory;
    use chrono::Utc;

    fn make_entry(id: &str, content: &str, tags: Vec<&str>) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            category: MemoryCategory::Knowledge,
            content: content.to_string(),
            tags: tags.into_iter().map(String::from).collect(),
            created: Utc::now(),
            source: "test".to_string(),
        }
    }

    #[test]
    fn test_index_create_and_add() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        let entry = make_entry(
            "entry-001",
            "The x402 payment protocol enables micropayments for AI inference",
            vec!["x402", "payment"],
        );
        idx.add_entry(&entry).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 1);
    }

    #[test]
    fn test_search_returns_results() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        let e1 = make_entry(
            "e1",
            "Bitcoin SV uses the original Bitcoin protocol for micropayments",
            vec!["bsv", "bitcoin"],
        );
        let e2 = make_entry(
            "e2",
            "Tantivy is a full-text search engine library written in Rust",
            vec!["search", "rust"],
        );
        let e3 = make_entry(
            "e3",
            "The x402 protocol handles payment negotiation between agents",
            vec!["x402", "payment"],
        );
        idx.add_entry(&e1).unwrap();
        idx.add_entry(&e2).unwrap();
        idx.add_entry(&e3).unwrap();

        let results = idx.search("payment", 10).unwrap();
        assert!(!results.is_empty());
        // Payment-related entries should score higher
        assert!(results[0].content.contains("payment") || results[0].content.contains("Payment"));
    }

    #[test]
    fn test_search_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();
        let results = idx.search("", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_delete_entry() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        let entry = make_entry("del-001", "This should be deleted", vec![]);
        idx.add_entry(&entry).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 1);

        idx.delete_entry("del-001").unwrap();
        assert_eq!(idx.entry_count().unwrap(), 0);
    }

    #[test]
    fn test_rebuild_index() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        // Add some entries directly
        let e1 = make_entry("r1", "First entry", vec![]);
        let e2 = make_entry("r2", "Second entry", vec![]);
        idx.add_entry(&e1).unwrap();
        idx.add_entry(&e2).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 2);

        // Rebuild with different entries
        let e3 = make_entry("r3", "Third entry only", vec![]);
        idx.rebuild(&[e3]).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 1);
    }

    #[test]
    fn test_upsert_entry() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        let entry = make_entry("upsert-001", "Original content", vec![]);
        idx.add_entry(&entry).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 1);

        // Update same ID with new content
        let updated = make_entry("upsert-001", "Updated content", vec![]);
        idx.add_entry(&updated).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 1);

        let results = idx.search("Updated", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "upsert-001");
    }

    #[test]
    fn test_reopen_existing_index() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index");

        // Create and populate
        {
            let idx = MemoryIndex::new(index_path.clone()).unwrap();
            let entry = make_entry("persist-001", "Persistent data", vec!["persist"]);
            idx.add_entry(&entry).unwrap();
        }

        // Reopen
        let idx = MemoryIndex::new(index_path).unwrap();
        assert_eq!(idx.entry_count().unwrap(), 1);

        let results = idx.search("Persistent", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "persist-001");
    }

    #[test]
    fn test_format_recalled_memories_multibyte_utf8() {
        // Regression test: format_recalled_memories must not panic on
        // multi-byte UTF-8 characters (like em-dash '—') near the truncation
        // boundary. This was the root cause of tasks hanging after setup_task()
        // in the agent loop — the panic killed the tokio task silently.
        let mut content_300 = "a]".repeat(148);
        content_300.push('—'); // 3-byte char at bytes 296-298, total 299 bytes
        content_300.push('x'); // byte 299-299, total 300 bytes? No…
                               // Actually let's be precise: we need a multi-byte char straddling byte 300
        let mut content = String::new();
        // Fill to exactly byte 298 with ASCII
        for _ in 0..298 {
            content.push('a');
        }
        // '—' is U+2014, encoded as 3 bytes (0xE2 0x80 0x94), spanning bytes 298-300
        content.push('—');
        // This makes content.len() == 301, with byte 300 inside the em-dash.
        assert!(content.len() == 301);
        assert!(!content.is_char_boundary(300));

        let snippet = MemorySnippet {
            id: "test-utf8".to_string(),
            category: "session".to_string(),
            content,
            score: 0.5,
            tags: vec![],
            created: "2026-01-01T00:00:00Z".to_string(),
        };

        // This must not panic
        let result = format_recalled_memories(&[snippet]);
        assert!(result.contains("aaa"));
    }

    #[test]
    fn test_extract_query_hints_multibyte_utf8() {
        // Regression: extract_query_hints truncates to 500 chars — must not
        // panic when multi-byte chars straddle the boundary.
        let mut long_task = String::new();
        for _ in 0..498 {
            long_task.push('x');
        }
        long_task.push('—'); // 3-byte char at bytes 498-500
        long_task.push_str("extra");
        assert!(long_task.len() > 500);

        // Must not panic
        let result = extract_query_hints(&long_task, None);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_search_scores_are_ordered() {
        let dir = tempfile::tempdir().unwrap();
        let idx = MemoryIndex::new(dir.path().join("index")).unwrap();

        let e1 = make_entry(
            "s1",
            "payment payment payment payment payment protocol",
            vec!["payment"],
        );
        let e2 = make_entry(
            "s2",
            "This mentions payment once among other things like rust and search",
            vec![],
        );
        idx.add_entry(&e1).unwrap();
        idx.add_entry(&e2).unwrap();

        let results = idx.search("payment", 10).unwrap();
        assert!(results.len() >= 2);
        // Higher TF should score higher
        assert!(results[0].score >= results[1].score);
    }
}
