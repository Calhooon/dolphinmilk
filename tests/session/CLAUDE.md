# session/

> Tests for multi-turn conversations, JSONL transcripts, and scheduled tasks.

## Overview

Three test files covering the session layer: conversation storage with BRC-60 hash chains, append-only JSONL transcripts for event recording, and the schedule tool system (interval, cron, one-shot). All tests use `tempfile` for filesystem isolation and require no running wallet.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_conversation.rs | 57 | ConversationManager CRUD, BRC-60 hash chain integrity, OpenAI format conversion, title generation, conversation key derivation, participant lookup, compaction, introspection tools, transcript append, metadata sync, file locking, self-healing |
| test_schedule.rs | 45 | Interval parsing (`30s`, `5m`, `1h`, `7d`), Schedule serde, tool registration (3 tools), tool execution (create/list/cancel), scan simulation, cron expressions, one-shot schedules, backward compat |
| test_transcript.rs | 31 | TranscriptEvent serde, record/replay, `to_messages()` conversion, sats/token accounting, proof and checkpoint data, `events_since()` slicing, `get_tool_result_content()` lookup |

## Test domains

### Conversations (test_conversation.rs)

**Storage & CRUD** -- Create conversations with `ConversationManager::create()`, append messages, load round-trip, list, and update metadata (sats, task IDs). Tests verify `meta.json` and `messages.jsonl` are written correctly. Corrupt JSONL lines are skipped on load.

**BRC-60 hash chain** -- Every message has a `hash` computed from `(prev_hash, role, content, ts)`. Tests verify:
- `compute_genesis_hash()` is deterministic per conversation ID
- `compute_message_hash()` is deterministic and content-sensitive
- `verify_chain()` detects tampered hashes and reports break indices
- `verify_chain_from_messages()` matches disk-based `verify_chain()`
- Compaction-aware verification trusts messages at or below `compaction_seq`
- Metadata consistency: stale `message_count` or `head_hash` in `meta.json` is flagged

**Title generation** -- `generate_title()` truncates at 60 characters on a word boundary. Tests: short strings pass through, exactly 60 chars pass through, long strings are truncated at a word boundary (no trailing spaces), strings without spaces are hard-truncated at 60.

**Conversation key derivation** -- `conversation_key_id()` returns `"conv-{id}"` for per-conversation encryption key derivation. Tests verify format, uniqueness per conversation ID, and determinism.

**OpenAI format conversion** -- `to_openai_messages()` converts stored messages to `{role, content}` format. Tool messages include `tool_call_id` and `name`. Orphaned tool results are not an issue at the conversation level (handled at transcript level).

**Transcript append** -- `append_from_transcript()` bridges `Transcript` events into conversation messages. Verifies assistant messages (from `think_response`) and tool results are appended with correct roles, `tool_calls`, `tool_call_id`, `name`, and `task_id`. Hash chain remains valid after append.

**Participant lookup** -- `find_by_participant()` finds conversations by `participant_key`, returning the most recent match. Tested with single/multiple keys and empty state.

**Compaction** -- `set_compaction_summary()` stores a summary string and sequence number in `meta.json`. Defaults to `None`. Compacted regions are trusted during chain verification.

**Introspection tools** -- Tests `all_conversation_tools()` which returns `list_conversations` and `read_conversation` tools. Verifies listing with limit, reading messages with truncation (content > 500 chars gets `...[truncated]`), and non-existent conversation handling.

**`create_with_id`** -- Idempotent creation with a specific conversation ID (used for heartbeat system conversation). Second call with same ID returns existing conversation without overwriting title.

**Metadata sync** -- `append_user_message()` and `append_from_transcript()` auto-update `meta.json` with correct `message_count` and `head_hash`.

**File locking** -- Smoke test for concurrent append safety. Sequential appends (10 messages) under a single manager produce valid chains with correct `message_count` and `head_hash`. Verifies the locking code path doesn't break normal single-threaded usage.

**Self-healing** -- Detects conversations with `task_ids` but missing assistant messages (e.g., from a failed `append_from_transcript`). Re-runs append from the task's transcript file. Skips when assistant messages already exist to avoid duplication.

### Schedules (test_schedule.rs)

**Interval parsing** -- `parse_interval()` accepts `Ns`, `Nm`, `Nh`, `Nd` suffixes. Rejects invalid suffixes, non-numeric input, empty strings, and zero values.

**Schedule struct serde** -- Round-trip JSON serialization of `Schedule` with all fields. `conversation_id` and `last_run` use `skip_serializing_if = "Option::is_none"`. `run_history: Vec<ScheduleRun>` uses `skip_serializing_if = "Vec::is_empty"` (capped at 50 entries). `ScheduleType` enum: `Interval`, `Cron`, `Once`.

**Tool registration** -- `all_schedule_tools()` returns 3 tools: `create_schedule`, `list_schedules`, `cancel_schedule`. All in `"schedule"` category with non-empty descriptions and object parameters.

**Tool execution (async)** -- Tests the actual tool closures:
- `create_schedule`: writes `{id}.json` to `workspace/schedules/`, supports `conversation_id`, validates required fields
- `list_schedules`: returns all schedules, handles empty directory
- `cancel_schedule`: sets `enabled: false` on disk, errors for non-existent or missing ID

**Cron schedules** -- `create_schedule` with `schedule_type: "cron"` and `cron_expression`. Validates expression syntax. `compute_next_cron_run()` returns a future timestamp within the next period. Missing or invalid `cron_expression` is rejected.

**One-shot schedules** -- `schedule_type: "once"` sets `one_shot: true`. After firing, the schedule is disabled (`enabled: false`). Requires `interval` for the initial delay.

**Scan simulation** -- Tests simulate `scan_schedules` behavior: picking up due schedules (past `next_run`), skipping future and disabled ones, incrementing `run_count`, updating `last_run` and `next_run`. Atomic write pattern (`.tmp` + rename) verified.

**Backward compatibility** -- Old schedule JSON without `schedule_type`, `cron_expression`, or `one_shot` fields deserializes with defaults (`Interval`, `None`, `false`). Omitting `schedule_type` in tool params defaults to `"interval"`.

### Transcripts (test_transcript.rs)

**Event serde** -- `TranscriptEvent` round-trips through JSON with `ts`, `type`, `id`, and flattened data fields. Corrupt JSONL lines are skipped on load.

**Record & replay** -- `Transcript::new()` creates or loads from a JSONL file. `record()`, `record_user()`, `record_system()` append events. `replay()` returns all events in order. Persistence verified by creating a new `Transcript` instance from the same path.

**Message conversion** -- `to_messages()` converts transcript events to OpenAI-format `{role, content}` messages. `think_response` events with `tool_calls` produce `assistant` messages with tool call arrays. `tool_result` events produce `tool` messages with `tool_call_id`. Orphaned tool results (no matching `think_response` with tool calls) are skipped.

**Cost tracking** -- `total_sats_spent()` sums `sats_paid` from `think_response` events plus `sats_paid` from `tool_result` events (e.g., image generation). Backward compatible: old transcripts without `sats_paid` on tool results contribute 0. `total_tokens()` sums `prompt_tokens` and `completion_tokens`.

**Proof and checkpoint data** -- `record_proof_created()` stores `txid`, `proof_type`, `hash`, and optional enrichment fields (`proof_data`, `proof_timestamp`, `sats_cost`, `iteration`, `basket`, `prev_hash` for chain linking). Optional fields omitted when `None` (backward compat). Data persists across replay. `record_checkpoint_created()` stores `txid`, `token_type`, `basket`, and optional `checkpoint_data` JSON.

**`events_since()`** -- Returns events from index N onward. Returns empty for out-of-bounds index or empty transcript.

**`get_tool_result_content()`** -- Looks up tool result content by `call_id`. Returns first match. Returns `None` for non-existent call IDs or empty transcripts.

## Key helpers

| Helper | File | Purpose |
|--------|------|---------|
| `tmp_mgr()` | test_conversation.rs | Returns `(TempDir, ConversationManager)` -- keep `_dir` alive |
| `tmp_transcript()` | test_transcript.rs | Returns `(TempDir, PathBuf)` -- keep `_dir` alive |
| `write_schedule()` | test_schedule.rs | Writes schedule JSON to `workspace/schedules/` |
| `read_schedule()` | test_schedule.rs | Reads schedule JSON back from disk |

## Imports

| Source module | Used in |
|---------------|---------|
| `bsv_worm::conversation` | test_conversation.rs -- `ConversationManager`, `ChainVerification`, `compute_genesis_hash`, `compute_message_hash`, `conversation_key_id`, `generate_title` |
| `bsv_worm::transcript` | test_conversation.rs, test_transcript.rs -- `Transcript`, `TranscriptEvent`, `EVENT_TYPES` |
| `bsv_worm::tools::schedule_tools` | test_schedule.rs -- `all_schedule_tools`, `compute_next_cron_run`, `parse_interval`, `Schedule`, `ScheduleType` |
| `bsv_worm::tools::conversation_tools` | test_conversation.rs -- `all_conversation_tools` |

## Gotchas

- **Tempfile lifetime.** Both `tmp_mgr()` and `tmp_transcript()` return the `TempDir` as the first tuple element. Dropping it deletes the directory. Always bind it: `let (_dir, mgr) = tmp_mgr()`.
- **Sleep for timestamp ordering.** `test_find_by_participant_returns_most_recent` uses `thread::sleep(10ms)` between `create()` calls to ensure different `updated_at` timestamps. Don't remove.
- **Corrupt JSONL injection.** `test_corrupt_jsonl_recovery` and `test_corrupt_line_handled` write invalid JSON directly to JSONL files to test skip-on-parse-error behavior.
- **Hash tampering tests.** Several tests manually corrupt `messages.jsonl` by modifying the `hash` field in specific lines. They verify that `verify_chain()` correctly reports breaks.
- **Tool execution is async.** Schedule tool tests and conversation introspection tool tests use `#[tokio::test]` because tool closures are `async`.
- **Backward compat patterns.** Both transcript and schedule tests verify that old JSON formats (missing newer fields) deserialize with sensible defaults. This prevents breakage when loading files from earlier versions.

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Parent test directory overview and conventions
- [src/session/CLAUDE.md](../../src/session/CLAUDE.md) -- Session module source documentation
- [src/server/handlers/CLAUDE.md](../../src/server/handlers/CLAUDE.md) -- HTTP handlers that use conversations and transcripts
- [tests/integration/CLAUDE.md](../integration/CLAUDE.md) -- Playwright E2E tests that exercise conversations end-to-end
