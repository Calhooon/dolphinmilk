# src/session/
> Session lifecycle — multi-turn conversations, append-only transcripts, and SSE event streaming.

## Overview

This module manages four distinct but related concerns: **conversations** (persistent multi-turn dialogue with BRC-60 hash chain integrity), **transcripts** (append-only per-task JSONL audit logs), **messages** (typed overlay on transcript events for pattern matching and state reconstruction), and **events** (real-time SSE streaming to the web UI). Conversations span multiple tasks and represent the user-facing dialogue history. Transcripts are per-task internal audit logs. Messages provide type-safe traversal and session recovery from transcripts. Events are ephemeral broadcast messages for live UI updates.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 6 | Re-exports `conversation`, `events`, `message`, `transcript` submodules |
| `conversation.rs` | 1376 | BRC-60 hash-chained multi-turn conversations with wallet sync/restore, advisory file locking |
| `transcript.rs` | 643 | Append-only JSONL event log with 19 event types, replay, and message reconstruction |
| `message.rs` | 572 | Typed `Message` enum (20 variants) over transcript events, `SessionState` for crash recovery |
| `events.rs` | 267 | `StepEvent` enum (12 variants) for SSE streaming to web UI, with output truncation |

## Key Exports

### conversation.rs

| Export | Description |
|--------|-------------|
| `ConversationMessage` | Single message: `seq`, `role`, `content`, `prev_hash`, `hash`, `ts`, optional `task_id`/`tool_calls`/`tool_call_id`/`name` |
| `Conversation` | Metadata: `id` (`conv-{uuid}`), `participant_key`, `title`, `head_hash`, `message_count`, `total_sats`, `task_ids`, `compaction_summary`/`compaction_seq` |
| `ConversationManager` | CRUD operations over `workspace/conversations/{conv_id}/` directories |
| `ChainVerification` | Result of BRC-60 hash chain integrity check with per-message `MessageVerification` entries, plus `meta_count_valid`/`meta_head_hash_valid` metadata consistency checks |
| `ConversationDetail` | Metadata + messages bundle (used for wallet sync serialization) |
| `SyncResult` | Wallet sync result: `txid` + `conversation_id` |
| `compute_genesis_hash()` | SHA-256 of `"CONVERSATION" || conv_id` — deterministic chain anchor |
| `compute_message_hash()` | SHA-256 of `prev_hash || role || content || ts` — BRC-60 chain link |
| `generate_title()` | Truncates first message to 60 chars at word boundary |
| `conversation_key_id()` | Per-conversation encryption key: `"conv-{conv_id}"` |
| `BASKET_CONVERSATIONS` | BRC-46 basket name: `"worm-conversations"` |

**ConversationLock** (internal): Advisory file lock for per-conversation concurrency control. Uses `File::lock()` (stabilized in Rust 1.84) on a `.lock` file in the conversation directory. Serializes writes to `messages.jsonl` and `meta.json`. Used by `update_meta()`, `set_compaction_summary()`, and `sync_meta_after_append()`.

**ConversationManager methods:**

| Method | Description |
|--------|-------------|
| `new(workspace)` | Creates `workspace/conversations/` if missing |
| `create(participant_key, first_message)` | New conversation with UUID, appends first user message |
| `create_with_id(id, participant_key, title)` | Named conversation (e.g., `conv-heartbeat`), idempotent |
| `load(conv_id)` | Load `meta.json`, returns `Option<Conversation>` |
| `load_messages(conv_id)` | Parse `messages.jsonl`, skips corrupt lines |
| `append_user_message(conv_id, content)` | Extend hash chain with a user message. Auto-syncs `meta.json` via `sync_meta_after_append()`. |
| `append_from_transcript(conv_id, transcript, task_id)` | Extract `think_response` + `tool_result` events from transcript, append as assistant/tool messages. Tracks `pending_tool_calls` to avoid orphaned tool results. Compacts tool output via `compact_content()`. Auto-syncs `meta.json` after all appends. |
| `to_openai_messages(conv_id)` | Convert to OpenAI API format (user/assistant/tool), applying `compact_content()` |
| `update_meta(conv_id, sats_spent, task_id)` | Update `head_hash`, `message_count`, `total_sats`, `task_ids`, `updated_at`. Uses `ConversationLock`. |
| `set_compaction_summary(conv_id, summary, through_seq)` | Store LLM-generated summary when context truncation occurs. Uses `ConversationLock`. |
| `find_by_participant(participant_key)` | Most recently updated conversation for a participant |
| `list()` | All conversations, newest-first by `updated_at` |
| `verify_chain(conv_id)` | Recompute every hash, report breaks. Skips messages at or below `compaction_seq` (trusted region). Validates `meta.json` consistency (`message_count`, `head_hash`). |
| `verify_chain_from_messages(messages, conv_id)` | Same verification from in-memory slice (used after wallet restore). No metadata checks (`meta_count_valid`/`meta_head_hash_valid` are `None`). |
| `sync_to_wallet(wallet, conv_id)` | Encrypt + PushDrop token → `worm-conversations` basket |
| `restore_from_wallet(wallet)` | List basket outputs, decrypt, write `meta.json` + `messages.jsonl`. Tries legacy shared key first, then per-conversation key. Verifies hash chain after restore. |

### transcript.rs

| Export | Description |
|--------|-------------|
| `TranscriptEvent` | Single event: `ts` (f64), `event_type`, `id` (8-char UUID), `data` (flattened HashMap) |
| `Transcript` | Append-only JSONL log with in-memory cache |
| `EVENT_TYPES` | 19 valid types (see below) |

**Transcript methods:**

| Method | Description |
|--------|-------------|
| `new(path)` | Load existing JSONL if present, skip corrupt lines |
| `record(event_type, data)` | Append event to memory + flush to disk immediately (crash-safe) |
| `record_system(content)` | Convenience: `system` event |
| `record_user(content)` | Convenience: `user` event |
| `record_think_request(messages, model, max_tokens)` | Outgoing LLM request (message count, not full messages) |
| `record_think_response(...)` | LLM response: text, model, sats_paid/effective/refunded, tokens, tool_calls, finish_reason, duration_ms |
| `record_tool_call(call_id, name, arguments)` | Tool invocation |
| `record_tool_result(call_id, name, result, success, sats_paid)` | Tool result |
| `record_budget(balance, spent_session, spent_hour, task_limit)` | Budget snapshot |
| `record_error(error, context)` | Error event |
| `record_continuation_save(id, reason, wake_at)` | Pause/continuation |
| `record_continuation_resume(id, original_task, paused_iteration)` | Resume from continuation |
| `record_proof_created(txid, proof_type, hash_hex, ...)` | On-chain proof with optional prev_hash, basket, iteration, sats_cost |
| `record_receipt_stored(path, txid, sats)` | BEEF receipt stored to disk |
| `record_checkpoint_created(txid, token_type, basket, data)` | BRC-48 checkpoint token |
| `record_memory_stored(memory_id, category, tags, preview)` | Memory storage event |
| `record_skill_activated(name, context)` | Skill activation event |
| `record_state_checkpoint(iteration, sats_spent, last_proof_hash, tools_used, model)` | Periodic state snapshot for crash recovery (written every N iterations by runner) |
| `replay()` | All events in order (`&[TranscriptEvent]`) |
| `to_messages()` | Reconstruct OpenAI-format message list, matching tool results to pending calls |
| `events_since(index)` | Slice from index (for poll-based UI streaming) |
| `event_count()` / `last_event()` | Length and tail access |
| `get_events_by_type(event_type)` | Filter events by type |
| `get_tool_result_content(call_id)` | Full content of a `tool_result` event by `tool_call_id` (returns `Option<String>`) |
| `read_bytes()` | Read raw JSONL file bytes from disk (used for HMAC computation) |
| `last_think_response_text()` | Last assistant response text (used by auto-recall) |
| `total_sats_spent()` | Sum of `sats_effective` (LLM) + `sats_paid` (tools) |
| `total_tokens()` | Sum of `(prompt_tokens, completion_tokens)` across think events |
| `to_typed_messages()` | Convert all events to typed `Message` variants (extension from `message.rs`) |
| `reconstruct_state()` | Rebuild `SessionState` from transcript (extension from `message.rs`) |

**19 event types:**
`system`, `user`, `think_request`, `think_response`, `tool_call`, `tool_result`, `budget_check`, `loop_warning`, `error`, `session_start`, `session_end`, `continuation_save`, `continuation_resume`, `proof_created`, `receipt_stored`, `checkpoint_created`, `memory_stored`, `skill_activated`, `state_checkpoint`

### message.rs

| Export | Description |
|--------|-------------|
| `Message` | Tagged enum (20 variants) — typed overlay on `TranscriptEvent` for pattern matching and structured field access |
| `SessionState` | Reconstructed loop state from a sequence of messages: task, iterations, sats_spent, proof chain, tools, model, completion status |

**Message variants** (1:1 with event types + `Unknown` fallback):
`System`, `User`, `ThinkRequest`, `ThinkResponse`, `ToolCall`, `ToolResult`, `BudgetCheck`, `LoopWarning`, `Error`, `SessionStart`, `SessionEnd`, `ContinuationSave`, `ContinuationResume`, `ProofCreated`, `ReceiptStored`, `CheckpointCreated`, `MemoryStored`, `SkillActivated`, `StateCheckpoint`, `Unknown`

**Message methods:**

| Method | Description |
|--------|-------------|
| `from_event(event)` | Convert a `TranscriptEvent` into a typed `Message`. Unrecognized types → `Unknown` with all data preserved |
| `ts()` | Timestamp of this message (all variants carry `ts: f64`) |
| `is_terminal()` | True if `SessionEnd` |
| `is_truncated()` | True if `ThinkResponse` with `finish_reason` "length" or "max_tokens" |
| `finish_reason()` | Returns `finish_reason` for `ThinkResponse`, `None` otherwise |

**SessionState fields:** `task`, `iterations`, `sats_spent`, `last_proof_hash`, `last_balance`, `completed`, `error`, `last_finish_reason`, `truncation_count`, `proof_txids`, `checkpoint_txids`, `tool_call_count`, `tools_used`, `last_model`

**SessionState methods:**

| Method | Description |
|--------|-------------|
| `from_messages(messages)` | Rebuild state by iterating messages in order. Accumulates iterations (from `ThinkRequest`), sats (from `ThinkResponse.sats_effective` + `ToolResult.sats_paid`), proofs, tools. `SessionEnd` totals override accumulated values when non-zero. |
| `is_interrupted()` | True if no `SessionEnd` event was recorded (crash/kill) |
| `was_truncated()` | True if last LLM response hit max_tokens |

**Design**: `Message` is a non-breaking typed overlay — `TranscriptEvent` remains the JSONL serialization format. Lossless round-trip: every event maps to exactly one variant (or `Unknown`). `SessionState::from_messages()` is the foundation for session recovery (#273) — everything needed to resume a crashed task is derived from the transcript without requiring the full `LoopState` struct.

### events.rs

| Export | Description |
|--------|-------------|
| `StepEvent` | Tagged enum (12 variants) serialized as `{"type": "snake_case", ...}` |
| `truncate_for_sse(output)` | Truncate to 2000 chars at char boundary, append truncation notice |
| `format_sse_event(event)` | Serialize to JSON string |

**StepEvent variants:**

| Variant | Fields | When emitted |
|---------|--------|-------------|
| `ThinkingStarted` | `iteration`, `model` | Before LLM call |
| `ThinkingComplete` | `iteration`, `text`, `sats_paid`, `tokens`, `has_tool_calls` | After LLM response |
| `ToolCallStarted` | `iteration`, `call_id`, `name`, `arguments` | Before tool execution |
| `ToolCallComplete` | `iteration`, `call_id`, `name`, `output`, `success` | After tool execution |
| `Response` | `iteration`, `text` | Final text (no more tool calls) |
| `Done` | `iterations`, `sats_spent`, `result` | Agent loop finished |
| `Error` | `iteration`, `message` | Error occurred |
| `BudgetUpdate` | `balance`, `spent`, `remaining` | After each iteration |
| `BudgetAdvisory` | `iteration`, `message`, `limit_type` | Budget limit exceeded in advisory mode (warn but don't block) |
| `SessionStarted` | `session_id`, `task_id`, `resumed` | Multi-turn session begins |
| `ApprovalRequired` | `iteration`, `call_id`, `name`, `arguments`, `staged_ref`, `timeout_secs` | Tool call requires manual approval before execution (use `/staged/{ref}/approve` or `/staged/{ref}/abort`) |
| `Escalation` | `iteration`, `reason`, `message` | Agent stuck, needs human intervention (resolve via `/task/{id}/escalation/resolve`) |

## Storage Layout

```
workspace/conversations/
  conv-{uuid}/
    .lock              # Advisory file lock (ConversationLock)
    meta.json          # Conversation struct (participant, title, head_hash, sats, task_ids)
    messages.jsonl     # ConversationMessage per line (BRC-60 hash chain)
  conv-heartbeat/      # Named system conversation for heartbeat tasks
    .lock
    meta.json
    messages.jsonl

workspace/tasks/{task_id}/
  transcript.jsonl     # TranscriptEvent per line (19 event types)
```

## BRC-60 Hash Chain

Each conversation is a linked list of SHA-256 hashes:

1. **Genesis**: `SHA-256("CONVERSATION" || conv_id)` — deterministic anchor per conversation
2. **Each message**: `SHA-256(prev_hash || role || content || ts)` — links to previous
3. **Verification**: `verify_chain()` recomputes every hash and reports `breaks` (sequence numbers where chain integrity fails). Also validates metadata consistency: `meta_count_valid` checks `meta.message_count` vs actual count, `meta_head_hash_valid` checks `meta.head_hash` vs last message hash.
4. **Compaction boundary**: If `compaction_seq` is set, messages at or below that sequence are treated as a trusted summary block — their hashes are accepted without recomputation. Verification resumes from the first message after the boundary.

The `head_hash` in `meta.json` is the hash of the most recent message. This allows integrity checking without reading the full message history.

## Concurrency Control

`ConversationLock` provides advisory file locking for per-conversation write serialization:

- Uses `File::lock()` (Rust 1.84, `flock` on Unix, `LockFileEx` on Windows)
- Lock file: `conv_dir/.lock`
- Acquired exclusively by `update_meta()`, `set_compaction_summary()`, `sync_meta_after_append()`
- Released automatically when the guard is dropped
- Prevents concurrent meta.json corruption from overlapping task completions

## Wallet Sync

Conversations sync to the wallet as encrypted PushDrop tokens:

- **Protocol**: `[2, "worm conversation"]`
- **Key ID**: `"conv-{conv_id}"` (per-conversation isolation; legacy uses shared `"conversations"` key)
- **Counterparty**: `"self"`
- **Basket**: `worm-conversations`
- **Script**: `["conversation", encrypted_data] OP_2DROP <pubkey> OP_CHECKSIG`

`restore_from_wallet()` handles legacy tokens encrypted with the shared key by trying the legacy key first, then re-encrypting with per-conversation keys on next sync.

## Usage Patterns

**Server task flow** (in `server/task_spawner.rs`):
1. `ConversationManager::create()` or `find_by_participant()` for session continuity
2. `append_user_message()` to record the incoming request (auto-syncs meta)
3. `to_openai_messages()` to build `prior_messages` for the runner
4. Runner creates `Transcript::new()` per task, records all events
5. After task: `append_from_transcript()` copies assistant/tool messages to conversation (auto-syncs meta)
6. `update_meta()` with sats spent and task ID

**Poll-based UI streaming** (in `server/handlers/tasks.rs`):
- `GET /task/{id}/events?since=N` → `transcript.events_since(N)`
- UI polls with last-seen index, receives only new events

**Conversation compaction** (in `server/handlers/conversations.rs`):
- `POST /conversations/{id}/compact` generates an LLM summary
- `set_compaction_summary()` stores it in meta.json
- `to_openai_messages()` can prepend the summary when loading prior context
- `verify_chain()` trusts messages at or below `compaction_seq`

**Session recovery** (via `message.rs`):
- `transcript.reconstruct_state()` rebuilds `SessionState` from JSONL transcript
- `state.is_interrupted()` detects crashed sessions (no `session_end` event)
- Runner uses `SessionState` fields (iterations, sats_spent, last_proof_hash, tools_used, last_model) to resume from where the agent left off
- `state_checkpoint` events provide periodic snapshots so recovery doesn't need to replay the entire transcript

**Transcript HMAC** (for audit integrity):
- `transcript.read_bytes()` returns raw JSONL file content for HMAC computation

## Related

- `../server/handlers/conversations.rs` — HTTP endpoints for conversation CRUD, verify, sync, compact
- `../server/handlers/tasks.rs` — Task audit, events polling, proof listing
- `../server/task_spawner.rs` — Unified task creation that integrates conversations + transcripts
- `../runner/step.rs` — Agent loop records events to transcript, emits SSE StepEvents
- `../runner/lifecycle.rs` — Uses `reconstruct_state()` for session recovery after crash
- `../context/manager.rs` — `compact_content()` used by conversation message conversion
- `../onchain/` — Proofs and state tokens recorded in transcript events
