# server/handlers/tasks
> Task-domain HTTP handlers — lifecycle, events, audit, and proof endpoints for the agent task system.

## Overview

This submodule contains all task-related route handlers, split from what was originally a single `tasks.rs` file into four domain-focused files. Handlers cover the full task lifecycle (submit, status, list, cancel, resume, artifacts, file serving), real-time event streaming and escalation management, audit trail generation and export, and on-chain proof verification. All handlers follow the standard pattern: BRC-31 auth check → transcript/state read → `signed_json_response()`.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 11 | Re-exports four submodules. `audit`, `events`, `lifecycle` as `pub(crate)`, `proofs` as `pub` (types used outside server) |
| `lifecycle.rs` | 1027 | Task submission, status, listing (memory + disk), cancel, resume, conversation reconstruction, conversation-id lookup, artifacts (per-task + global), file serving, time-saved analytics |
| `events.rs` | 342 | Poll-based transcript event streaming, escalation status/resolution/proof endpoints |
| `audit.rs` | 618 | Audit chain from transcript, export (CSV/signed JSON/PDF), transcript HMAC verification, `html_to_pdf()` helper |
| `proofs.rs` | 586 | Proof listing (BRC-18 + BRC-48 split), on-chain hash verification, custody proof with structured billing fields, BEEF payment receipts |

## Handler → Route Mapping

### lifecycle.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `submit_task` | `POST /task` | BRC-31 | Submit task with optional tags → `spawn_task()`, returns 202. Enforces cert-driven tag policy |
| `get_status` | `GET /status` | BRC-31 | All in-memory tasks + active count + total sats |
| `get_task` | `GET /task/{id}` | BRC-31 | Single task info by ID |
| `list_tasks` | `GET /tasks` | BRC-31 | All tasks (in-memory + disk scan), with tokens, tags, time estimates, conversation_id. Supports `?tag=X,Y` AND filter |
| `get_time_saved` | `GET /analytics/time-saved` | BRC-31 | Aggregate human-equivalent time saved across all tasks (in-memory + disk) |
| `get_conversation` | `GET /task/{id}/conversation` | BRC-31 | Reconstructed conversation messages from transcript |
| `cancel_task` | `POST /task/{id}/cancel` | BRC-31 | Set cancel flag on running/queued task. Returns 409 if already finished |
| `resume_task` | `POST /task/{id}/resume` | BRC-31 | Resume an interrupted task — spawns new task on the same conversation with prior context. Returns 409 if not `Interrupted` |
| `get_task_conversation_id` | `GET /task/{id}/conversation-id` | BRC-31 | Lightweight lookup of conversation_id for a task (returns `{ conversation_id }`) |
| `get_artifacts` | `GET /task/{id}/artifacts` | BRC-31 | List artifacts in a task workspace — reads `artifacts.json` manifest + scans directory for unlisted files. Adds download URLs |
| `get_all_artifacts` | `GET /artifacts` | BRC-31 | List artifacts across ALL tasks. Scans every task directory, enriches with `task_id`/`task_text`/URL. Supports `?type=image` filter. Sorted by `created_at` descending |
| `serve_task_file` | `GET /files/{task_id}/{filename}` | none | Static file serving with path traversal protection and MIME type detection. Falls back to `uploads/` subdirectory |

**Task listing** (`list_tasks`): Two-phase collection — (1) in-memory tasks from `task_mgr.tasks` with transcript-derived sats/tokens/tool counts for consistency, (2) disk scan of `workspace/tasks/` for historical tasks not in memory, parsing transcripts for metadata. Results merged, sorted by `started_at` descending, then filtered by tags if `?tag=` is present. Each `TaskSummary` includes `time_saved_minutes` via `crate::time_estimate` and `conversation_id` from `task_sessions`.

**Tag policy** (`submit_task`): Normalizes tags via `normalize_tags()`, then checks `read_cert_tag_policy()` — if `tags_required == true` and no tags provided, returns 400.

**Task resume** (`resume_task`): Only tasks with `TaskStatus::Interrupted` can be resumed. Reconstructs session state (iterations, sats_spent) from the interrupted transcript via `transcript.reconstruct_state()`. Spawns a new task with tags `["resume", "resumed-from:{id}"]` and origin `"resume"`. Marks the original task as `Complete` with result pointing to the new task. Returns 202 with `resumed_task_id`, `session_id`, and prior iteration/sats context.

**Artifacts** (`get_artifacts`): Reads `artifacts.json` manifest if present, then scans the task directory for any additional files not in the manifest. Skips system files (`session.jsonl`, `budget.jsonl`, `artifacts.json`, `fork_context.json`), hidden files, `tool_output_*` files, and directories. Infers artifact type from extension (image, video, audio, document, file). Adds `/files/{task_id}/{name}` download URLs.

**All artifacts** (`get_all_artifacts`): Same scanning logic as `get_artifacts` but across every task directory. Enriches each artifact with `task_id` and `task_text` (first 200 chars of the first user message, extracted via `extract_task_text()`). Supports `?type=` filtering (e.g., `?type=image`).

**File serving** (`serve_task_file`): Double path traversal protection — rejects `..`, `/`, `\` in both task_id and filename, then canonicalizes and verifies the resolved path is within the task directory. Falls back to `uploads/` subdirectory for user-supplied attachments if the file doesn't exist in the task root. Supports 14 MIME types (images, video, audio, JSON, text, PDF).

### events.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_task_events` | `GET /task/{id}/events` | BRC-31 | Incremental transcript events since offset (`?since=N`). Returns events, total count, active flag, session_id |
| `get_escalation` | `GET /task/{id}/escalation` | BRC-31 | Escalation status from `AppState.escalation_status` |
| `resolve_escalation` | `POST /task/{id}/escalation/resolve` | BRC-31 | Resolve with human guidance. Returns 409 if not escalated |
| `get_escalation_proof` | `GET /task/{id}/escalation/proof` | BRC-31 | Escalation BRC-18 proofs filtered from transcript `proof_created` events |

**Event polling** (`get_task_events`): Returns `TranscriptEventDto` entries (index, ts, type, id, data) starting from `?since=N`. Also reports `active` (task still running) and `session_id` for session recovery. Returns empty events with `active: true` for registered tasks whose transcript doesn't exist yet (task is starting up).

**Escalation resolution** (`resolve_escalation`): Accepts `ResolveEscalationRequest` with `guidance` (required) and `resolved_by` (optional, defaults to "unknown"). Clears the `escalated` flag and records an `EscalationResolution` with timestamp.

**Escalation proofs** (`get_escalation_proof`): Scans transcript for `proof_created` events where `proof_type` is "Escalation" or "escalation". Each proof includes txid, hash, timestamp, data, prev_hash chain, and the current resolution from `escalation_status` if available.

### audit.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_audit` | `GET /task/{id}/audit` | BRC-31 | Full transcript audit chain with `AuditSummary` (events, iterations, sats, proofs, tool calls, duration) + `conversation_id` |
| `get_audit_export` | `GET /task/{id}/audit/export` | BRC-31 | Downloadable report — CSV (default), signed JSON (`?format=json`), PDF (`?format=pdf`) |
| `verify_transcript` | `GET /task/{id}/transcript/verify` | BRC-31 | HMAC integrity verification via wallet |

**Audit response** (`get_audit`): Includes `conversation_id` from `task_sessions` in the `AuditResponse`.

**Audit export** (`get_audit_export`): Flattens transcript events into rows with timestamp, iteration, event_type, model, tool_name, sats_spent, proof_txid, and a one-line detail summary. Three output formats:
- **CSV** (default): RFC 4180 compliant with proper quote escaping. Content-Disposition attachment.
- **JSON** (`?format=json`): Wallet-signed export. Computes SHA-256 hash of `{events, proofs, summary}` payload, signs with `wallet.create_signature()` using protocol `[2, "dolphin milk audit"]` and key_id `"audit-export"`. Includes `signature`, `signature_hash`, and `agent_identity` in the output.
- **PDF** (`?format=pdf`): Renders HTML via `pdf_template::render_audit_html()` then converts with `html_to_pdf()`. Falls back to 503 if Chrome is unavailable.

**Transcript HMAC verification** (`verify_transcript`): Extracts HMAC from the most recent `checkpoint_created` event's `checkpoint_data.hmac` field (base64-encoded). Decodes and calls `wallet.verify_hmac()` with protocol `[2, "dolphin milk transcript"]` and key_id = task_id. Returns `TranscriptVerifyResponse` with `valid`, `hmac`, and optional `error`. Handles missing HMAC, invalid base64, empty transcript, and wallet errors gracefully.

**`html_to_pdf()`**: Shared `pub(crate)` helper used by both audit and budget export. Launches headless Chrome via `chromiumoxide`, creates a blank page, sets HTML content, waits 200ms for CSS rendering, then calls `PrintToPdf` with background printing enabled. Chrome flags: `--headless=new`, `--disable-gpu`, `--no-sandbox`, `--disable-dev-shm-usage`.

### proofs.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_proofs` | `GET /task/{id}/proofs` | BRC-31 | All on-chain records split into `proofs` (BRC-18 OP_RETURN) and `checkpoints` (BRC-48 state tokens) |
| `verify_proofs` | `GET /task/{id}/proofs/verify` | BRC-31 | Recompute SHA-256 hashes + cross-check against `dm-proofs` basket on-chain |
| `get_custody_proof` | `GET /task/{id}/proofs/custody` | BRC-31 | Custody proof with parsed billing fields (customer_key, total_sats, agent_key, etc.) |
| `get_receipts` | `GET /task/{id}/receipts` | BRC-31 | BEEF payment receipts from `tasks/{id}/beef/*.json` |

**Proof listing** (`get_proofs`): Partitions transcript `proof_created` and `checkpoint_created` events into two arrays — `proofs` (BRC-18 hash-chained OP_RETURN proofs) and `checkpoints` (BRC-48 spendable state tokens). Each `ProofDetail` includes txid, proof_type, hash, timestamp, proof_data, checkpoint_data, sats_cost, basket, iteration, and prev_hash for chain verification.

**Proof verification** (`verify_proofs`): Two-stage verification per proof:
1. **Hash recompute**: If `proof_data` and `proof_timestamp` are available, recomputes SHA-256 hash (including `prev_hash` in computation to match `proofs.rs::compute_proof_hash()`). Reports "match", "mismatch", or "unavailable".
2. **On-chain check**: Paginates through `dm-proofs` basket outputs (100 per page), parses `OP_RETURN` scripts (`006a20` prefix + 32-byte hash), maps txid → hash. Cross-checks transcript hashes against on-chain hashes. Reports "match", "mismatch", or "not_found".

**Custody proof** (`get_custody_proof`): Finds the `proof_created` event with `proof_type == "custody"` or `"Custody"`. Parses structured billing fields from the proof data text using `parse_proof_field()` (line-prefix matching): `CUSTOMER_KEY`, `TASK_HASH`, `ITERATIONS`, `DURATION_SECS`, `TOTAL_SATS`, `RESULT_HASH`, `AGENT_KEY`, `PROOF_CHAIN_LENGTH`. Verifies hash consistency using `ProofCommitment::with_timestamp()`.

**Receipts** (`get_receipts`): Reads JSON files from `tasks/{id}/beef/` directory. Each `ReceiptDetail` has iteration, model, sats_paid, sats_effective, sats_refunded, tokens, timestamp. Sorted by iteration. Computes `total_sats_paid` and `total_sats_refunded` aggregates.

## Key Exports

### Public types (from proofs.rs, exported as `pub`)

| Type | Description |
|------|-------------|
| `ProofVerification` | Per-proof on-chain verification result with hash_recompute, on_chain_match, proof_type, iteration |
| `VerifyResponse` | `{ task_id, verifications: Vec<ProofVerification> }` |
| `ReceiptsResponse` | `{ task_id, receipts, total_sats_paid, total_sats_refunded }` |
| `ReceiptDetail` | Single BEEF receipt: iteration, model, sats_paid/effective/refunded, tokens, timestamp |

### `pub(crate)` types

| Type | File | Description |
|------|------|-------------|
| `TaskListParams` | lifecycle.rs | Query params for `GET /tasks` with comma-separated tag filter |
| `AllArtifactsParams` | lifecycle.rs | Query params for `GET /artifacts` with optional `?type=` filter |
| `TranscriptEventDto` | events.rs | Single transcript event DTO (index, ts, type, id, data) |
| `TaskEventsResponse` | events.rs | Events response with offset, total, active flag, session_id |
| `EventsQuery` | events.rs | `?since=N` query param for event polling |
| `ResolveEscalationRequest` | events.rs | Escalation resolution body (guidance, resolved_by) |
| `EscalationProofResponse` | events.rs | `{ task_id, proofs: Vec<EscalationProofDetail> }` |
| `EscalationProofDetail` | events.rs | Single escalation proof with txid, hash, timestamp, data, prev_hash, resolution |
| `AuditExportQuery` | audit.rs | `?format=csv\|json\|pdf` query param |
| `TranscriptVerifyResponse` | audit.rs | `{ task_id, valid, hmac, error }` |
| `CustodyProofResponse` | proofs.rs | `{ task_id, proof: Option<CustodyProofDetail> }` |
| `CustodyProofDetail` | proofs.rs | Custody proof with parsed billing fields and hash verification |

### Helper functions

| Function | File | Visibility | Description |
|----------|------|------------|-------------|
| `html_to_pdf(html)` | audit.rs | `pub(crate)` | Headless Chrome HTML→PDF conversion. Used by audit and budget export |
| `parse_proof_field(data, prefix)` | proofs.rs | private | Extract `"KEY: value"` fields from proof data text |
| `deserialize_comma_separated` | lifecycle.rs | private | Serde deserializer for `?tag=X,Y,Z` query params |
| `extract_task_text(session_path)` | lifecycle.rs | private async | Extract first user message (truncated to 200 chars) from session.jsonl. Used by `get_all_artifacts` |

## Dependencies

- `crate::server::auth` — `check_brc31_auth()`, `signed_json_response()`, `signed_raw_response()`
- `crate::server::task_spawner` — `spawn_task()` (used by `submit_task` and `resume_task`)
- `crate::server::types` — Shared DTOs (`TaskInfo`, `TaskStatus`, `TaskRequest`, `TaskResponse`, `StatusResponse`, `AuditResponse`, `AuditEvent`, `AuditSummary`, `ProofsResponse`, `ProofDetail`, `ConversationResponse`, `TaskListResponse`, `TaskSummary`, `normalize_tags()`)
- `crate::server::AppState` — Shared state (task_mgr, workspace, config, escalation_status, budget)
- `crate::server::pdf_template` — `render_audit_html()` for PDF export
- `crate::wallet::WalletClient` — Wallet operations (list_outputs, create_signature, verify_hmac, get_identity_key)
- `crate::transcript::Transcript` — JSONL transcript reader (replay, events_since, event_count, total_sats_spent, total_tokens, get_events_by_type, to_messages, read_bytes, reconstruct_state)
- `crate::certificates` — `read_cert_tag_policy()` for tag enforcement
- `crate::time_estimate` — `estimate_time_saved()`, `default_estimate()`, `parse_time_saved_tag()`, `aggregate_time_saved()`, `TaskTimeSavedInput`
- `crate::proofs::ProofCommitment` — Hash verification in custody proof
- `crate::runner::escalation` — `EscalationResolution`, `EscalationStatus`
- `chromiumoxide` — Headless Chrome for PDF generation
- `sha2`, `hex`, `base64` — Hash computation and encoding for proof verification and audit signing

## Related

- [`../CLAUDE.md`](../CLAUDE.md) — Parent handlers module with route mapping for all 15 handler submodules
- [`../../CLAUDE.md`](../../CLAUDE.md) — Server module (AppState, router, task_spawner, auth)
- [`../../types.rs`](../../types.rs) — Shared request/response types used across all handlers
- [`../../task_spawner.rs`](../../task_spawner.rs) — Unified `spawn_task()` called by `submit_task` and `resume_task`
- [`../../pdf_template.rs`](../../pdf_template.rs) — HTML templates for PDF export
- [`../../../onchain/CLAUDE.md`](../../../onchain/CLAUDE.md) — BRC-18 proofs and BRC-48 state tokens
- [`../../../session/CLAUDE.md`](../../../session/CLAUDE.md) — Transcript and conversation modules
