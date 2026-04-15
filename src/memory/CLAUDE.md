# memory
> Persistent knowledge store with BM25 search, post-store processing, encrypted sync, and UHRP backup.

## Overview

The memory module gives the agent durable knowledge across sessions. Memories are markdown files with YAML frontmatter, organized by category (`knowledge/`, `sessions/`, `execution/`). A tantivy BM25 index enables fast ranked retrieval. Post-store processing detects duplicates, extracts tags, and validates quality. Encrypted mirrors (`.enc` files) provide wallet-native backup, with UHRP cloud sync via NanoStore (BRC-31 auth + x402 payment).

**Components and their roles:**
- `MemoryStore` — CRUD on disk (the source of truth). Identity entries use the `IDENTITY_TAG` convention.
- `MemoryIndex` — tantivy search index (ephemeral, rebuilt from `.md` files)
- `auto_recall` — automatic per-iteration memory recall with MMR diversity, recency weighting, and key term extraction
- `verify_recalled_memories` / `TamperWarning` — tamper detection via `.proof` sidecar files (checks recalled memories against on-chain content hashes)
- `post_process` / `ProcessingResult` — post-store duplicate detection, tag extraction, quality validation (non-fatal)
- `SessionSummarizer` — transforms JSONL transcript events into session memory entries
- `MemorySync` — encrypts entries and manages the `encrypted/` mirror
- `encrypt` — dual wallet-native + legacy encryption
- `NanoStoreClient` / `UhrpIndex` — UHRP cloud storage client with BRC-31 auth + x402 payment (upload, list, find all wired)

All errors use `WormError::memory(msg)`. The wallet at `localhost:3322` handles all cryptographic operations (BRC-42 key derivation, AES-256-GCM) — the memory module never touches raw keys in the wallet-native path.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 24 | Module root. Re-exports `MemoryIndex`, `MemorySnippet`, `TamperWarning`, `SessionSummarizer`, `MemoryCategory`, `MemoryEntry`, `MemoryStore`, `IDENTITY_TAG`, `MemorySync`, `post_process`, `ProcessingResult`. |
| `store.rs` | 566 | File-based memory store. `MemoryCategory` enum, `MemoryEntry` struct with YAML frontmatter serialization, `MemoryStore` for CRUD on `.md` files plus `.proof` sidecar read/write. `IDENTITY_TAG` constant for identity entries. |
| `search.rs` | 1171 | Tantivy BM25 full-text search + auto-recall with MMR diversity. `MemoryIndex` for indexing, `auto_recall()` for per-iteration memory injection, `extract_key_terms()` for BM25 query condensation, `extract_query_hints()` + `sanitize_query()` for safe query construction, `format_recalled_memories()` for prompt formatting. Memory tamper detection: `TamperWarning`, `verify_recalled_memories()`, `format_tamper_warnings()`. |
| `processing.rs` | 461 | Post-store memory processing. `check_duplicates()` via BM25 score threshold, `extract_tags()` from hashtags/ALL_CAPS/quoted terms, `validate_quality()` for content checks. `post_process()` orchestrates all three — non-fatal, runs inline after store. |
| `session.rs` | 349 | Session summarization. `SessionSummarizer` extracts structured markdown summaries from JSONL transcript events (task, tools, sats, errors, outcome). |
| `encrypt.rs` | 410 | Dual encryption: wallet-native (`encrypt_with_wallet`/`decrypt_with_wallet` with counterparty) and legacy (`legacy_encrypt`/`legacy_decrypt`). `is_legacy_format()` auto-detects. |
| `sync.rs` | 448 | `MemorySync` orchestrates encrypt → write `.enc` file → track in `UhrpIndex`. Handles legacy format detection on decrypt. `SyncSummary` for diagnostics. |
| `uhrp.rs` | 609 | `NanoStoreClient` for UHRP/NanoStore upload/download/list/find with BRC-31 auth + x402 payment. `UhrpUploadResult` for two-step upload responses. `UhrpIndex` and `UhrpIndexEntry` for tracking stored files. |

## Public API

### store.rs

- **`IDENTITY_TAG`** — `"identity"` constant. Identity entries use `MemoryCategory::Knowledge` with this tag rather than a separate enum variant (avoids breaking serialization of existing entries).
- **`MemoryCategory`** — `Knowledge`, `Session`, `Execution`. `Display` outputs lowercase; `FromStr` accepts `"session"` and `"sessions"`. Uses `#[serde(rename_all = "lowercase")]`.
- **`MemoryEntry`** — Fields: `id` (UUID), `category`, `content`, `tags: Vec<String>`, `created: DateTime<Utc>`, `source`. Constructor: `new(category, content, tags, source)` generates UUID + timestamp.
- **`MemoryStore`** — `new(base_dir: PathBuf)` creates category subdirs. Methods: `store(&entry) → PathBuf`, `read(id)`, `list(Option<category>)`, `delete(id)`, `find_identity_entry() → Option<MemoryEntry>`, `base_dir() → &Path`, `category_dir(&category) → PathBuf`, `write_proof_sidecar(&entry, txid, content_hash) → PathBuf`, `read_proof_sidecar(id, &category) → Option<(String, String)>`. List returns newest-first.

### search.rs

- **`MemoryIndex`** — `new(index_dir)` opens or creates tantivy index, auto-rebuilds on schema mismatch. Methods: `add_entry(&entry)` (upserts by ID), `search(query, limit) → Vec<MemorySnippet>`, `delete_entry(id)`, `rebuild(&[MemoryEntry])`, `entry_count()`. Has custom `Debug` impl that omits the `Index` handle.
- **`MemorySnippet`** — Search result: `id`, `category`, `content`, `score: f32`, `tags: Vec<String>`, `created: String` (RFC 3339).
- **`auto_recall(index, query, limit, exclude_ids) → Vec<MemorySnippet>`** — Runs every agent iteration. Steps: (1) BM25 candidate pool (3x limit, min 15), (2) filter excluded IDs, (3) recency weighting (`1.0 / (1.0 + age_hours / 24.0)`), (4) MMR re-ranking (lambda=0.7), (5) sort by adjusted score.
- **`jaccard_similarity(a, b) → f64`** — Word-set Jaccard coefficient. Splits on whitespace, lowercases, returns `|A ∩ B| / |A ∪ B|`.
- **`extract_key_terms(raw_text) → String`** — Filters stop words, ranks remaining terms by specificity (proper nouns +30, mixed alphanumeric +25, all-caps acronyms +20, word length bonus), returns 3–8 key terms joined by spaces. Falls back to sanitized raw text if filtering removes everything. Short inputs (≤3 words) pass through as-is.
- **`extract_query_hints(task_description, recent_assistant_text: Option<&str>) → String`** — Combines task + last 200 chars of assistant response (if `Some`), truncates total to 500 chars at word boundaries, then calls `extract_key_terms()` for BM25 query condensation.
- **`sanitize_query(query) → String`** — Strips tantivy special characters (`?`, `:`, `(`, `)`, `"`, `~`, `^`, `*`, `+`, `-`, `[`, `]`, `{`, `}`, `\`) to prevent parse errors.
- **`format_recalled_memories(memories) → String`** — Formats auto-recalled memories as a markdown section (`## Recalled Memories (auto)`) with category, first line, relevance score, and truncated content (300 chars). Uses `floor_char_boundary()` for safe UTF-8 truncation.
- **`TamperWarning`** — Struct: `entry_id`, `expected_hash`, `actual_hash`. Emitted when a recalled memory's content hash doesn't match its `.proof` sidecar.
- **`verify_recalled_memories(memories, store) → Vec<TamperWarning>`** — Checks recalled snippets against `.proof` sidecar files. For each snippet: reads sidecar via `store.read_proof_sidecar()`, recomputes `SHA-256(content || created_at || category)` via `compute_memory_hash()`, compares hashes. Entries without sidecars are silently skipped (backward compatible).
- **`format_tamper_warnings(warnings) → String`** — Formats tamper warnings as a `## TAMPER WARNINGS` markdown section for system prompt injection. Returns empty string if no warnings. Warns the LLM not to trust tampered entries.
- Constants: **`STOP_WORDS`** (~130 common English stop words + agent-context filler), **`KEY_TERMS_MAX`** (8).
- Private: `mmr_select(pool, limit, lambda) → Vec<MemorySnippet>` — MMR selection from `(MemorySnippet, f64)` pairs. `compute_adjusted_score(snippet, now) → f64` — recency-adjusted BM25 score.

### processing.rs

- **`post_process(index, content, self_id) → ProcessingResult`** — Main entry point called after `memory_store`. Orchestrates quality validation, duplicate detection, and tag extraction. All errors are caught and logged — never prevents the store operation.
- **`ProcessingResult`** — Fields: `warnings: Vec<String>`, `dedup: DedupResult`, `extracted_tags: Vec<String>`.
- **`DedupResult`** — Fields: `is_duplicate: bool`, `matching_id: Option<String>`, `score: Option<f32>`.
- **`check_duplicates(index, content, self_id) → DedupResult`** — Searches BM25 index with content as query, checks if top non-self result exceeds `DEDUP_BM25_THRESHOLD` (20.0). Returns early once first non-self result is evaluated (results are score-descending).
- **`extract_tags(content) → Vec<String>`** — Extracts up to `MAX_EXTRACTED_TAGS` (5) unique lowercase tags via: (1) `#hashtags`, (2) `ALL_CAPS` terms >3 chars, (3) `"quoted terms"`.
- **`validate_quality(content) → Vec<String>`** — Returns warnings for: content <20 chars, all whitespace, no alphabetic characters. Empty vec = passes all checks.
- Constants: `DEDUP_BM25_THRESHOLD` (20.0), `MAX_EXTRACTED_TAGS` (5), `MIN_CONTENT_LENGTH` (20).

### session.rs

- **`SessionSummarizer`** — Stateless (zero fields). `summarize_events(&[Value]) → String` produces markdown with sections: Summary, Date, Iterations, Cost, Actions Taken, Key Findings, Errors, Outcome. `create_session_entry(events, session_id) → MemoryEntry` wraps it as a Session category entry with tool-name tags.
- Private helpers: `find_tool_result(events, call_id)` matches `tool_call`→`tool_result` by call_id; `extract_first_sentence(text)` finds first `.`/`!`/`?` after 10+ chars; `truncate_str(s, max)` safely truncates at char boundaries.

### encrypt.rs

- `encrypt_with_wallet(wallet, plaintext, protocol_id, key_id, counterparty) → Vec<u8>` — Wallet-native, current path. Delegates to `wallet.encrypt()`. `counterparty` is typically `"self"` for encrypt-to-self, or a hex pubkey for two-party BRC-42 ECDH.
- `decrypt_with_wallet(wallet, ciphertext, protocol_id, key_id, counterparty) → Vec<u8>` — Wallet-native, current path. Delegates to `wallet.decrypt()`. `counterparty` must match the value used during encryption.
- `is_legacy_format(blob) → bool` — Checks first 4 bytes for magic `0x42421033`.
- `legacy_encrypt(key, key_id, plaintext) → Vec<u8>` — Old format: `[VERSION:4][KEY_ID:32][IV:32][CIPHERTEXT:var][TAG:16]`.
- `legacy_decrypt(key, blob) → Vec<u8>` — Decrypts old format blobs.
- `legacy_extract_key_id(blob) → [u8; 32]` — Reads bytes 4..36 from a legacy blob.
- `legacy_derive_key(wallet, key_id) → [u8; 32]` — Signs `brc78-derive-encryption-key` with protocol `[2, "message encryption"]`, SHA-256 hashes signature.
- Constants: `ENCRYPT_VERSION` (`[0x42, 0x42, 0x10, 0x33]`), `KEY_ID_SIZE` (32), `HEADER_SIZE` (36), `MIN_SDK_BLOB` (48).

### sync.rs

- **`MemorySync`** — `new(memory_base_dir: &Path)` creates `encrypted/` subdir. Methods:
  - `encrypted_dir() → &Path` — Path to the `encrypted/` directory.
  - `index() → &UhrpIndex` — Reference to the in-memory UHRP index.
  - `encrypt_and_store(entry, wallet, key_id) → content_hash` — Encrypts `entry.content` (not frontmatter), writes `.enc`, updates `UhrpIndex`.
  - `decrypt_entry(entry_id, wallet, key_id) → String` — Auto-detects legacy vs wallet-native format.
  - `list_encrypted() → Vec<String>` — Lists `.enc` file IDs (strips `.enc` suffix).
  - `delete_encrypted(entry_id)` — Removes `.enc` file and index entry.
  - `save_index(wallet, key_id)` / `load_index(wallet, key_id)` — Encrypted index persistence at `encrypted/index.enc`.
  - `summary() → SyncSummary` — File count, index entries, last sync time.
- **`SyncSummary`** — Diagnostic struct: `encrypted_dir: PathBuf`, `file_count`, `index_entries`, `last_sync: Option<String>`.
- `sha256_hex(data: &[u8]) → String` — Public SHA-256 hex hash utility (used for content-addressing encrypted blobs).
- Private: `memory_protocol_id() → Value` — Returns `[2, "worm memory"]` JSON value used by all sync encrypt/decrypt calls.

### uhrp.rs

- **Constants**: `NANOSTORE_HOST` (`https://nanostore.babbage.systems`), `UHRP_CDN` (`https://storage.googleapis.com/prod-uhrp/cdn`), `DEFAULT_RETENTION_MINUTES` (525600 = 1 year).
- **`NanoStoreClient`** — `new(auth: AuthriteClient)` or `with_url(auth, base_url)`. Stores an `AuthriteClient` for BRC-31 auth on upload/list/find. Methods: `auth() → &AuthriteClient`, `download(hash)` (public CDN, no auth), `upload(encrypted_blob, retention_minutes: Option<u64>) → UhrpUploadResult` (two-step: BRC-31 auth + x402 payment → presigned GCS URL → PUT bytes), `list() → Vec<UhrpFile>` (BRC-31 auth), `find(hash) → Option<UhrpFile>` (BRC-31 auth, returns `None` for 404). Static: `content_hash(data) → String`, `cdn_url(hash) → String`.
- **`UhrpUploadResult`** — Fields: `file: UhrpFile`, `payment_txid: Option<String>`, `sats_paid: u64`. Returned by `upload()`.
- **`UhrpIndex`** — `entries: Vec<UhrpIndexEntry>`, `last_sync: Option<String>`. Implements `Default`. Methods: `upsert(entry)`, `remove(name) → bool`, `find(name) → Option<&UhrpIndexEntry>`, `to_bytes() → Vec<u8>` (JSON), `from_bytes(data) → Self`.
- **`UhrpIndexEntry`** — Fields: `name` (logical path, e.g. `"knowledge/patterns.md"`), `category`, `content_hash` (SHA-256 hex of encrypted blob), `original_size: u64`, `uploaded_at` (RFC 3339).
- **`UhrpFile`** — Fields: `hash`, `url`, `expiry: Option<u64>`, `size: Option<u64>`.

## Disk Layout

```
{memory_base_dir}/
  knowledge/              # MemoryCategory::Knowledge files
    {uuid}.md             # Markdown with YAML frontmatter
    {uuid}.proof          # Optional JSON sidecar: { txid, content_hash }
  sessions/               # MemoryCategory::Session files (note: plural)
    {uuid}.md
    {uuid}.proof
  execution/              # MemoryCategory::Execution files
    {uuid}.md
    {uuid}.proof
  encrypted/              # MemorySync encrypted mirror
    {uuid}.enc            # Encrypted content only (no frontmatter)
    index.enc             # Encrypted UhrpIndex (JSON, not a memory entry)
  {index_dir}/            # Tantivy index files (ephemeral, rebuilt from .md)
```

## Frontmatter Schema

Each `.md` file has YAML frontmatter:
```yaml
---
id: "550e8400-e29b-41d4-a716-446655440000"
category: knowledge          # knowledge | session | execution
tags:
  - x402
  - payment
created: "2026-02-23T14:30:00+00:00"  # RFC 3339
source: "session-2026-02-23-01"
---

The actual memory content goes here.
```

Identity entries use `category: knowledge` with `tags: [identity]` — the `IDENTITY_TAG` constant.

## Tantivy Index Schema

Five fields, all text:
| Field | Options | Purpose |
|-------|---------|---------|
| `id` | STRING + STORED | Exact match for upsert/delete |
| `category` | STRING + STORED | Exact match filtering |
| `content` | TEXT + STORED | Full-text BM25 search |
| `tags` | TEXT + STORED | Full-text (space-joined, not tokenized per-tag) |
| `created` | STRING + STORED | RFC 3339 timestamp |

Query parser searches `content` + `tags` fields simultaneously. Empty queries return zero results (short-circuit in `search()`). The reader uses `ReloadPolicy::OnCommitWithDelay` for near-real-time visibility of new entries.

## Auto-Recall Flow

Every agent iteration, `auto_recall()` injects relevant memories into the system prompt:

```
1. extract_query_hints(task_description, recent_assistant_text)
   → combine task + last 200 chars of assistant response (max 500 chars)
   → extract_key_terms(): filter stop words, rank by specificity, select 3-8 terms
   → condensed BM25 query (avoids query term dilution)

2. auto_recall(index, query, limit=5, exclude_ids)
   → BM25 candidate pool (3× limit, min 15)
   → filter excluded IDs
   → recency weighting: score × 1.0/(1.0 + age_hours/24.0)
   → MMR re-ranking (lambda=0.7, Jaccard word similarity)
   → sorted results

3. verify_recalled_memories(results, store)
   → for each snippet, check .proof sidecar if present
   → recompute SHA-256(content || created || category)
   → if mismatch → TamperWarning

4. format_recalled_memories(results)
   → markdown section injected into system prompt
   format_tamper_warnings(warnings)
   → TAMPER WARNINGS section (if any) injected into system prompt
```

## Post-Store Processing Flow

After every `memory_store` write, `post_process()` runs inline:

```
post_process(index, content, self_id) → ProcessingResult
  1. validate_quality(content) → warnings
     - Content < 20 chars, all whitespace, no alphabetic chars
  2. check_duplicates(index, content, self_id) → DedupResult
     - BM25 search with content as query (top 5 results)
     - First non-self result above DEDUP_BM25_THRESHOLD (20.0) → is_duplicate
  3. extract_tags(content) → extracted_tags
     - #hashtags, ALL_CAPS terms (>3 chars), "quoted terms"
     - Up to 5 unique lowercase tags
```

All processing is non-fatal — errors are logged but never prevent the store operation.

## Sync Flow

```
Write path:
  MemoryEntry → MemoryStore.store() → {category}/{id}.md
                MemorySync.encrypt_and_store() → encrypted/{id}.enc + UhrpIndex update
                NanoStoreClient.upload() → x402 payment → presigned URL → UHRP CDN

Read path:
  Boot → MemoryStore.list() → all .md files → MemoryIndex.rebuild()
  Boot → MemorySync.load_index() → decrypt encrypted/index.enc → UhrpIndex

Decrypt path:
  MemorySync.decrypt_entry() → read .enc → is_legacy_format()?
    yes → legacy_extract_key_id → legacy_derive_key → legacy_decrypt
    no  → decrypt_with_wallet (wallet-native)
```

## Agent Tool Integration

The agent accesses memory through two tools defined in `tools/memory_tools.rs`:

| Tool | Action | Flow |
|------|--------|------|
| `memory_store` | Write | params → `MemoryEntry::new()` → `MemoryStore::store()` → `MemoryIndex::add_entry()` → `post_process()` |
| `memory_search` | Read | query → `MemoryIndex::search()` → `Vec<MemorySnippet>` → JSON response |

At session end, `SessionSummarizer::create_session_entry()` auto-generates a Session memory from the transcript. The runner calls `MemoryStore::store()` + `MemoryIndex::add_entry()` for this entry.

Auto-recall runs per iteration via `auto_recall()` in the runner — no explicit tool call needed. Recalled memories appear in the system prompt under `## Recalled Memories (auto)`.

Encrypted sync is orthogonal — `MemorySync::encrypt_and_store()` can be called after any `MemoryStore::store()` to create an encrypted mirror. The encrypted path does not affect the plain-text read path.

## Encryption Protocol IDs

| Use case | `protocol_id` | `key_id` | `counterparty` |
|----------|---------------|----------|----------------|
| Memory content | `[2, "worm memory"]` | category name (`"knowledge"`, `"sessions"`, etc.) | `"self"` |
| Memory index | `[2, "worm memory"]` | `"index"` | `"self"` |
| Legacy derivation | `[2, "message encryption"]` | 32-byte key_id (from blob) | identity key |

## Session Summary Format

`SessionSummarizer::summarize_events` produces markdown:
```markdown
## Session Summary: {task description}
**Date**: {RFC 3339 timestamp}
**Iterations**: {think_response count}
**Cost**: {total sats_effective} sats

### Actions Taken
- {tool_name}: {OK|FAILED} — {truncated result}

### Key Findings
- {first sentence from think_response content, >20 chars, max 5}

### Errors
- {truncated error messages}

### Outcome
{last think_response content, truncated to 300 chars}
```

## Decisions

- **Files are the source of truth, not the index.** Tantivy is ephemeral — it's rebuilt from markdown files on startup via `MemoryIndex::rebuild()`. This avoids index corruption issues and keeps memories human-inspectable. Never treat the tantivy index as durable.
- **Category enum maps to directory names asymmetrically.** `MemoryCategory::Session` displays as `"session"` but its directory is `sessions/`. The `FromStr` impl accepts both `"session"` and `"sessions"`. This mismatch is intentional for natural language vs filesystem naming.
- **Two encryption paths coexist.** Wallet-native (`encrypt_with_wallet`) is the current path — the wallet handles BRC-42 key derivation + AES-256-GCM internally. Legacy (`legacy_encrypt`) uses local `SymmetricKey` with a custom binary format (magic `0x42421033`). Legacy exists solely to read old `.enc` files. `is_legacy_format()` auto-detects which path to use based on the first 4 bytes.
- **Legacy key derivation is unusual.** `legacy_derive_key` signs a fixed message (`brc78-derive-encryption-key`) with protocol `[2, "message encryption"]`, then SHA-256 hashes the signature to get an AES key. This was the pre-wallet-native approach — don't replicate it for new code.
- **Session summarizer is stateless.** `SessionSummarizer` has no fields — all methods take transcript events as `&[Value]` and extract structure (task, tools, sats, errors, outcome) purely from the JSONL shape. The first `user` event is assumed to be the task description.
- **UHRP upload uses two-step x402 payment flow.** `NanoStoreClient::upload` does: (1) POST `/upload` with BRC-31 auth + x402 payment via `authenticated_paid_request()` to reserve storage and get a presigned GCS URL, (2) PUT file bytes to the presigned URL (plain HTTP). Upload, list, and find all use BRC-31 Authrite authentication. Downloads are public via CDN (content is encrypted at rest).
- **MemorySync encrypts content only, not frontmatter.** `encrypt_and_store` encrypts `entry.content` bytes, not the full markdown-with-frontmatter. The `.enc` files contain only encrypted content; metadata is tracked in the `UhrpIndex` sidecar.
- **Auto-recall uses MMR for diversity.** Pure BM25 recall tends to return redundant results. MMR (Maximal Marginal Relevance) with Jaccard word similarity penalizes candidates too similar to already-selected results, ensuring diverse context injection.
- **Key term extraction prevents BM25 query dilution.** Full-length task descriptions dilute BM25 match scores because common words dominate. `extract_key_terms()` filters ~130 stop words and ranks remaining terms by specificity (proper nouns, technical terms, acronyms, word length), condensing to 3–8 key terms. Based on arscontexta research finding.
- **Query sanitization prevents tantivy parse errors.** `sanitize_query()` strips 15 special characters (`?:()\"~^*+-[]{}\`) from natural language queries rather than escaping them. This avoids accidental query syntax from task descriptions and assistant responses.
- **Identity entries use tags, not a new category.** `IDENTITY_TAG` marks Knowledge entries as identity rather than adding a 4th `MemoryCategory` variant, which would break serialization of existing entries.
- **Post-store processing is non-fatal by design.** `post_process()` catches all errors internally. Duplicate detection, tag extraction, and quality validation never prevent a store operation from succeeding. Warnings are logged, not propagated.
- **Duplicate detection uses BM25 self-search.** Rather than exact-match hashing, `check_duplicates()` searches the index using the new content as a query and checks if the top non-self result exceeds a score threshold (20.0). This catches semantic near-duplicates, not just exact copies.
- **Tamper detection uses proof sidecars, not the blockchain.** `.proof` sidecar files store a txid + content hash alongside each `.md` file. At recall time, `verify_recalled_memories()` recomputes the content hash and compares locally — no on-chain lookup needed for tamper detection. The txid in the sidecar provides an on-chain anchor for full verification if needed. Entries without sidecars are silently skipped for backward compatibility.
- **Proof sidecar hash covers content + metadata.** The content hash in `.proof` files is `SHA-256(content || created_at || category)` via `compute_memory_hash()` from `onchain/proofs.rs`, not just the content bytes. This means changing the created timestamp or category would also trigger a tamper warning.

## Gotchas

- **Tantivy writer uses 50MB heap.** Each `add_entry`/`delete_entry` call creates a new `IndexWriter` with a 50MB arena. For bulk operations, use `rebuild()` which shares a single writer across all entries.
- **Search parses queries against content + tags fields.** `QueryParser::for_index` uses both `content_field` and `tags_field`, so searching for a tag name will match both tag metadata and content text. Tags are stored space-joined, not as separate tokens.
- **Frontmatter parsing is BOM-aware.** `MemoryEntry::from_markdown` strips a leading UTF-8 BOM before parsing. If you see frontmatter parse failures, check for encoding issues.
- **`extract_first_sentence` has a minimum length.** Sentence boundaries (`.`, `!`, `?`) are only recognized after 10+ characters. Single-word sentences like "Done." won't be extracted as findings. Falls back to 200-char truncation.
- **Encrypted index file is `encrypted/index.enc`.** This is a special file — not a memory entry but a serialized `UhrpIndex`. `save_index`/`load_index` manage it separately from entry `.enc` files.
- **`list_encrypted()` includes `index` in results.** The method strips `.enc` suffixes from all files in the encrypted directory, including `index.enc`. This means `"index"` appears alongside entry UUIDs. Callers should filter it if only entry IDs are needed.
- **`delete_encrypted` uses `ends_with` pattern matching.** The index cleanup does `entries.retain(|e| !e.name.ends_with(&format!("/{id}.md")))`. This is a suffix match, not exact match — safe because UUIDs are unique, but don't use short or predictable IDs.
- **Legacy blob minimum size.** `HEADER_SIZE` (36 bytes: 4 version + 32 key_id) + `MIN_SDK_BLOB` (48 bytes: 32 IV + 16 GCM tag) = 84 bytes minimum for even an empty-plaintext legacy blob. `legacy_decrypt` rejects anything shorter.
- **MemoryStore.list() sorts newest-first.** Results are sorted by `created` descending (`b.created.cmp(&a.created)`). If you need chronological order, reverse the result.
- **MemoryStore.read() scans all categories.** There is no category parameter on `read(id)` — it iterates `Knowledge`, `Session`, `Execution` directories looking for `{id}.md`. IDs must be globally unique across categories.
- **SessionSummarizer.summarize_events caps findings at 5.** The `findings` vector stops collecting after 5 entries, and each finding must be >20 characters. Short assistant responses are silently skipped.
- **MemoryIndex schema mismatch triggers full rebuild.** If the tantivy index on disk has a different schema from what `new()` expects, it deletes all index files and recreates from scratch. This is safe because the index is ephemeral, but means adding a field to the schema requires an explicit `rebuild()` call to re-index existing entries.
- **`load_index` silently succeeds when no index file exists.** If `encrypted/index.enc` doesn't exist, `load_index` returns `Ok(())` with an empty index — it does not error. This allows clean first-run behavior.
- **Invalid tantivy queries return errors, not empty results.** Empty queries short-circuit to `Ok(vec![])`, but malformed queries (bad syntax) propagate a `WormError` from `QueryParser::parse_query`. The `sanitize_query()` function mitigates this for auto-recall queries.
- **`NanoStoreClient::find` returns `None` for 404.** Unlike other NanoStore methods which return `Err` on non-success status, `find` explicitly handles 404 as `Ok(None)`. Other error statuses still return `Err`.
- **`extract_query_hints` truncates to 500 chars then condenses.** The combined task description + recent assistant text is capped at 500 characters at word boundaries, then `extract_key_terms()` further condenses to 3–8 key terms. Very long task descriptions may lose their suffix before term extraction.
- **`extract_key_terms` short-circuits on ≤3 words.** Inputs of 3 or fewer words pass through sanitized but unfiltered. Stop word filtering only activates for longer inputs.
- **Dedup requires diverse index for realistic BM25 IDF.** BM25 IDF is near-zero in a single-document index, so even exact matches score below the dedup threshold. Tests use 20 filler entries to produce realistic scores.
- **`format_recalled_memories` uses `floor_char_boundary()`.** Truncation at 80-char (first line) and 300-char (content) boundaries uses `floor_char_boundary()` to avoid panicking on multi-byte UTF-8 characters like em-dashes. This was a regression fix — panics in `format_recalled_memories` killed tokio tasks silently.
- **`extract_tags` deduplicates case-insensitively.** Tags are lowercased and tracked in a `HashSet`, so `#Bitcoin` and `#bitcoin` produce a single `"bitcoin"` tag.
- **`.proof` sidecars are optional.** `read_proof_sidecar()` returns `None` if no `.proof` file exists. Pre-existing entries without sidecars are silently skipped during tamper verification. `write_proof_sidecar()` is called by the runner after storing a BRC-18 proof for a memory entry.
- **`verify_recalled_memories` depends on `onchain::proofs::compute_memory_hash`.** The hash function used for tamper detection lives in `onchain/proofs.rs`, creating a cross-module dependency. If the hash function changes, existing `.proof` files become stale.
- **`format_tamper_warnings` truncates hashes to 16 chars.** The warning display shows only the first 16 characters of expected/actual hashes for readability. Full hashes are in the `TamperWarning` struct for programmatic use.

## Testing Patterns

- **Filesystem tests**: Use `tempfile::tempdir()` for isolated store/index/sync directories. All tests clean up automatically.
- **Wallet mocks**: `mockito` server mocks for `/encrypt` and `/decrypt` endpoints. The encrypt mock returns a fixed ciphertext; the decrypt mock returns specified plaintext bytes. `WalletClient::new(&server.url(), ...)` points at the mock.
- **Valid secp256k1 pubkeys**: Wallet identity key mocks must use valid pubkeys (generator point G), never random hex strings.
- **Search tests**: Index entries in a temp dir, verify BM25 scores are ordered correctly. Empty query returns empty results.
- **Dedup tests**: Require 20 filler entries in the index (diverse topics) to produce realistic BM25 IDF scores. Without filler, even exact duplicates score below the threshold.

## Related

- [../CLAUDE.md](../CLAUDE.md) — Project architecture and conventions
- `../tools/memory_tools.rs` — Agent-facing `memory_store` and `memory_search` tools
- `../runner/` — Calls `auto_recall()` per iteration
- `../context/prompt.rs` — Injects `format_recalled_memories()` output into system prompt
- `../wallet.rs` — `WalletClient` that encrypt/decrypt/sync delegate to
- `../auth/` — `AuthriteClient` used by `NanoStoreClient` for BRC-31 auth
- `../x402/payment.rs` — `authenticated_paid_request()` used by `NanoStoreClient::upload()`
- `../onchain/proofs.rs` — `compute_memory_hash()` used by tamper detection in `verify_recalled_memories()`
- `../session/transcript.rs` — JSONL event source that `SessionSummarizer` consumes
