# tests/memory

> 109 tests across 5 files covering the memory subsystem: markdown+YAML store, tantivy BM25 search, auto-recall with MMR diversity, post-store processing, scheduled maintenance, and tamper detection.

## Overview

Tests the full memory stack from low-level storage (markdown files on disk organized by category) through BM25 indexing (tantivy) to high-level features like automatic recall with diversity ranking, heartbeat-driven maintenance, and cryptographic tamper detection. Each file targets a distinct layer: `test_memory.rs` tests the store+index foundation and session summarization, `test_auto_recall.rs` tests the intelligent retrieval pipeline, `test_memory_processing.rs` tests post-store quality checks, `test_memory_maintenance.rs` tests the scheduled cleanup system, and `test_memory_tamper.rs` tests SHA-256 hash integrity with proof sidecars.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_auto_recall.rs | 49 | BM25 auto-recall: `extract_query_hints`, `extract_key_terms`, `sanitize_query`, `format_recalled_memories`, `auto_recall` (MMR diversity, recency weighting, exclude_ids), `jaccard_similarity`, pipeline integration |
| test_memory.rs | 23 | MemoryStore CRUD (roundtrip, list, delete, sort, special chars), MemoryIndex (create, search, ranking, delete, rebuild, reopen), store+index integration, SessionSummarizer, MemoryEntry construction, WormError::memory |
| test_memory_tamper.rs | 14 | Tamper detection: `compute_memory_hash` (deterministic SHA-256), `verify_memory_hash`, proof sidecar creation/reading, tamper detection on recall, `format_tamper_warnings`, backward compatibility for entries without sidecars |
| test_memory_processing.rs | 12 | Post-store processing: `check_duplicates` (exact, near-match, distinct topics), `extract_tags` (hashtags, caps terms, max-5 limit), `validate_quality` (too short, whitespace, valid), `post_process` non-fatal integration |
| test_memory_maintenance.rs | 11 | Heartbeat-driven cleanup: stale session/execution flagging, knowledge immunity, duplicate detection via BM25 similarity, health report counts, empty memory safety, MemoryConfig defaults/TOML parsing, mixed stale+fresh |

## What each file tests

### test_memory.rs — Store, index, and session summarization (23 tests)

**MemoryStore** (8 tests): Directory creation (`knowledge/`, `sessions/`, `execution/`), roundtrip read/write for Knowledge and Session categories, `list()` with optional category filter, `delete()`, newest-first sort order (uses `thread::sleep` for timestamp gaps), markdown special characters (code fences, headings, bullets, quotes), `MemoryCategory` case-insensitive parsing including plural form (`"sessions"` → `Session`).

**MemoryIndex** (6 tests): tantivy index creation and `entry_count()`, BM25 search with content matching, search result ranking (most relevant first by score), entry deletion, `rebuild()` from entry list (clears and re-adds), index persistence across reopen.

**Store+Index integration** (2 tests): Store entries then index then search (verifies cross-layer roundtrip), rebuild index from store listing (verifies `list()` feeds `rebuild()` correctly).

**SessionSummarizer** (4 tests): Summarize transcript events into text (captures task name, tool names), `create_session_entry()` produces `MemoryCategory::Session` entry, empty events produce valid output, error events captured in summary (budget exhausted).

**Edge cases** (3 tests): `WormError::memory` variant formatting, UUID generation uniqueness (36-char format), timestamp recency (within 1 second of `Utc::now()`).

### test_auto_recall.rs — Intelligent retrieval pipeline (49 tests)

**`extract_query_hints`** (7 tests): Empty task returns empty string, task-only extracts key terms (filters stop words like "an", "of", "a"), task+recent text combines domain terms from both sources, long text truncated to ≤500 chars, empty/whitespace recent text falls back to task only, output is sanitized (no tantivy special chars).

**`extract_key_terms`** (8 tests): Stop word filtering ("the", "is", "and", "been" removed; "payment", "wallet", "updated" kept), proper noun preservation ("Bitcoin", "NanoStore"), domain term preservation ("BRC31", "x402"), all-stop-word fallback (non-empty result), short input passthrough (≤3 words returned as-is), deduplication (repeated words appear once), acronym scoring ("BSV", "LLM", "API" kept), end-to-end quality test (verbose query finds BRC-31 auth entry).

**`format_recalled_memories`** (9 tests): Empty input returns empty string, single snippet formatting (`## Recalled Memories (auto)` header, `[category]` prefix, score display), multiple snippets with mixed categories, content truncation at ~300 chars with "...", first-line heading extraction, score formatting to 2 decimal places, short content not truncated, exactly-300-char boundary (no truncation), 301-char boundary (truncated).

**`jaccard_similarity`** (7 tests): Identical strings → 1.0, disjoint strings → 0.0, partial overlap → correct ratio (2/4 = 0.5), case insensitive, empty strings → 0.0, one empty → 0.0, subset ratio (2/3).

**`auto_recall`** (10 tests): Empty query returns empty, empty index returns empty, basic BM25 retrieval, `exclude_ids` filtering, exclude all returns empty, respects result limit, recency weighting (newer entry with same content ranks first), MMR diversity (similar entries don't crowd out different topics), zero limit returns empty, whitespace-only query returns empty.

**`sanitize_query`** (6 tests): Strips `?`, `:`, `(`, `)`, `"`, `*`, collapses whitespace, preserves plain text, verifies `extract_query_hints` output is sanitized (no tantivy special chars).

**Pipeline integration** (2 tests): `auto_recall` → `format_recalled_memories` end-to-end produces correct header and relevance scores, `extract_query_hints` no regression with task+recent text.

### test_memory_tamper.rs — Tamper detection and proof sidecars (14 tests)

**`compute_memory_hash`** (4 tests): Deterministic SHA-256 hashing (same inputs → same 64-char hex hash), different content produces different hash, different timestamp produces different hash, different category produces different hash.

**`verify_memory_hash`** (2 tests): Matching hash returns true, mismatched hash returns false (covers tampered content, tampered timestamp, tampered category, and completely wrong hash — all in one test).

**Proof sidecar creation/reading** (2 tests): `write_proof_sidecar()` creates `.proof` file on disk with txid and content hash, `read_proof_sidecar()` reads them back; missing sidecar returns `None`.

**Tamper detection on recall** (4 tests): Full round-trip — store entry, write sidecar, overwrite file with tampered content, `verify_recalled_memories()` detects the tamper (expected vs actual hash mismatch); entries without sidecars produce no warnings (backward compatibility); verified entries produce no warnings; mixed batch (verified + tampered + no-sidecar) only flags the tampered entry.

**`format_tamper_warnings`** (2 tests): Empty warnings produce empty string, non-empty warnings produce "TAMPER WARNINGS" header with entry ID, truncated hash previews (first 16 chars), and "DO NOT trust" message.

### test_memory_processing.rs — Post-store quality pipeline (12 tests)

**`check_duplicates`** (4 tests): Exact content match detected (score ≥ 20.0, returns matching ID), near-duplicate detected (minor rewording at end), distinct topics not flagged, empty index returns no duplicate safely. Uses `add_filler_entries()` helper to seed 20 diverse entries for realistic BM25 IDF scores.

**`extract_tags`** (4 tests): Hashtag extraction (`#bitcoin` → `"bitcoin"`), capitalized/domain terms (`BRC-31`, `AUTH` → lowercase tags), max 5 tags enforced, empty/whitespace content returns empty.

**`validate_quality`** (3 tests): Content below minimum length warns "very short", all-whitespace content warns "all whitespace", valid content returns no warnings.

**`post_process`** (1 test): Integration test — empty content produces warnings but no panic, valid content with tags succeeds, extremely long content (10k words) does not panic. Verifies the function is always non-fatal.

### test_memory_maintenance.rs — Heartbeat-driven cleanup (11 tests)

**Stale flagging** (5 tests): 31-day-old session entries flagged stale, fresh (1-day) sessions not flagged, knowledge entries never flagged regardless of age (60 days still immune), 35-day execution entries flagged, mixed fresh+stale correctly partitioned (only stale IDs in `stale_ids`).

**Duplicate detection** (1 test): Two near-duplicate knowledge entries (differ by 2 words) detected as duplicate pair, distinct entry not paired. Uses `MaintenanceSummary.duplicate_pairs`.

**Health reporting** (1 test): `MaintenanceSummary` correctly counts `total_entries`, `knowledge_count`, `session_count`, `execution_count`.

**Configuration** (3 tests): `MemoryConfig::default()` has maintenance disabled, correct default values (`maintenance_interval_secs: 3600`, `stale_session_days: 30`), TOML parsing with custom values.

**Safety** (1 test): Empty memory directory does not panic, returns zeroed summary.

## Key test helpers

| Helper | File | Purpose |
|--------|------|---------|
| `make_entry(id, content, category, age_hours)` | test_auto_recall.rs | Creates `MemoryEntry` with specific age offset |
| `make_entry_with_tags(id, content, tags, age_hours)` | test_auto_recall.rs | Creates Knowledge entry with tags and age |
| `make_snippet(id, category, content, score, age_hours)` | test_auto_recall.rs | Creates `MemorySnippet` with BM25 score and timestamp |
| `build_test_index(entries)` | test_auto_recall.rs | Returns `(TempDir, MemoryIndex)` — keep `_dir` alive |
| `make_entry(id, content)` | test_memory_processing.rs | Creates Knowledge entry with current timestamp |
| `add_filler_entries(idx)` | test_memory_processing.rs | Seeds 20 diverse entries for realistic BM25 IDF scores |
| `make_entry(category, content, tags)` | test_memory_tamper.rs | Creates `MemoryEntry` with UUID and current timestamp |
| `make_entry_aged(category, content, tags, days_ago)` | test_memory_maintenance.rs | Creates entry with specific creation date |
| `enabled_config()` | test_memory_maintenance.rs | Returns `MemoryConfig` with maintenance enabled |

## Source modules tested

| Source module | What's tested |
|---------------|---------------|
| `src/memory/store.rs` | `MemoryStore`, `MemoryEntry`, `MemoryCategory`, `write_proof_sidecar`, `read_proof_sidecar` |
| `src/memory/search.rs` | `MemoryIndex`, `MemorySnippet`, `auto_recall`, `extract_query_hints`, `extract_key_terms`, `sanitize_query`, `format_recalled_memories`, `jaccard_similarity`, `verify_recalled_memories`, `format_tamper_warnings`, `TamperWarning` |
| `src/memory/session.rs` | `SessionSummarizer::summarize_events`, `SessionSummarizer::create_session_entry` |
| `src/memory/processing.rs` | `check_duplicates`, `extract_tags`, `validate_quality`, `post_process` |
| `src/proofs/` | `compute_memory_hash`, `verify_memory_hash` |
| `src/heartbeat/` | `run_memory_maintenance`, `MaintenanceSummary` |
| `src/config/` | `MemoryConfig`, `WormConfig` (TOML parsing) |
| `src/error.rs` | `WormError::memory` variant |

## Patterns and conventions

- **Filesystem isolation**: Every test uses `tempfile::tempdir()` for both store and index directories
- **TempDir lifetime**: `build_test_index()` returns `(TempDir, MemoryIndex)` — bind `_dir` to keep the directory alive
- **Timestamp ordering**: `test_memory.rs` uses `thread::sleep(10ms)` between stores to guarantee filesystem timestamp order — don't remove these sleeps
- **Floating point**: Jaccard similarity tests use `f64::EPSILON` for exact comparisons; BM25 scores use `>=` threshold checks
- **BM25 IDF realism**: `test_memory_processing.rs` seeds 20 filler entries via `add_filler_entries()` because BM25 IDF is near-zero with only 1-2 documents, causing false negatives in duplicate detection
- **Proof sidecar pattern**: `test_memory_tamper.rs` writes `.proof` files alongside `.md` memory files; tests verify creation, reading, and tamper detection through the full store→sidecar→recall→verify pipeline
- **No network or wallet**: All tests are fully offline — no HTTP mocking needed, no wallet required

## Running

```bash
# All memory tests
cargo test --test test_memory --test test_auto_recall --test test_memory_processing --test test_memory_maintenance --test test_memory_tamper

# By file
cargo test --test test_auto_recall

# By pattern (matches function names across all files)
cargo test jaccard
cargo test dedup
cargo test stale
cargo test tamper
cargo test extract_key_terms
```

## Related

- [tests/CLAUDE.md](../CLAUDE.md) — Parent test directory overview, full helper index, test conventions
- [src/memory/CLAUDE.md](../../src/memory/CLAUDE.md) — Memory system source: tantivy, encryption, embeddings, store
- [src/config/CLAUDE.md](../../src/config/CLAUDE.md) — `MemoryConfig` definition and env overrides
- [src/server/CLAUDE.md](../../src/server/CLAUDE.md) — `/memory/search` HTTP endpoint
