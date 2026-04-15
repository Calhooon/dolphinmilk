# server/handlers
> Domain-split HTTP route handlers for the Axum REST API.

## Overview

This module contains all route handler functions for the HTTP API, organized by domain. Each file exports `pub(crate)` async handler functions that receive Axum extractors (`State`, `Path`, `Query`, `HeaderMap`, `Bytes`, `Json`) and return signed JSON responses. Every handler follows the same pattern: BRC-31 auth check → business logic → `signed_json_response()`.

The handlers are thin — they read from `AppState`, delegate to domain modules (`wallet.rs`, `transcript.rs`, `conversation.rs`, `memory/`, `certificates.rs`), and format responses. No business logic lives here that doesn't belong to request/response translation.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 17 | Re-exports all 15 handler submodules as `pub(crate)` |
| `agent.rs` | 748 | Agent identity, health, BRC-52 certificate CRUD + revocation, skill analytics, available models list |
| `analytics.rs` | 248 | Efficiency trends, cost comparison (9-model pricing with prefix lookup), ROI, provider benchmarks |
| `audit.rs` | 615 | BRC-69/70 key linkage revelation (with append-only revelation log), cross-agent proof cross-referencing, revelation listing, full-text audit search |
| `budget.rs` | 412 | Budget reports, spending breakdown, multi-source BSV/USD rate (WhatsOnChain + CoinGecko), rate margin/staleness, budget export (JSON/PDF) |
| `chat.rs` | 495 | Chat submission with command deduplication + tag policy enforcement, history, OpenAI-compat endpoint |
| `compliance.rs` | 282 | UTXO lifecycle management status, compliance report with date-range filtering |
| `conversations.rs` | 657 | Multi-turn conversation CRUD, BRC-60 chain verification, wallet sync, LLM compaction, self-healing from transcripts, enriched detail with summary stats, conversation-scoped artifact listing |
| `marketplace.rs` | 245 | Plugin marketplace CRUD — list, get, create/update, delete plugin listings. Live x402 service catalog |
| `memory.rs` | 188 | Memory listing with pagination/filtering, BM25 search, single entry retrieval |
| `misc.rs` | 88 | Message forwarding, heartbeat trigger |
| `replay.rs` | 201 | Structured task replay with cost/tool timelines, task forking from event index |
| `schedules.rs` | 89 | Schedule listing and detail endpoints |
| `staging.rs` | 287 | Transaction staging — list, approve, abort (wallet signing + tool approval modes) |
| `tasks/` | 2584 | Task directory — split into 4 submodules (see below) |
| `wallet_ops.rs` | 456 | UTXO output lookup, PushDrop parsing, BRC-42 decryption, funding address, UTXO check + auto-internalize, UTXO set with tier summaries |

### tasks/ submodules

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 11 | Re-exports all 4 task handler submodules |
| `lifecycle.rs` | 1027 | Task submission/status/listing (with tag filtering + time estimates + conversation_id), cancel, resume interrupted tasks, conversation reconstruction, per-task and global artifact listing, file serving |
| `audit.rs` | 618 | Audit trail + export (CSV/signed JSON/PDF), transcript HMAC verification, `html_to_pdf()` helper |
| `events.rs` | 342 | Transcript event polling, escalation status/resolution/proofs |
| `proofs.rs` | 586 | Proof listing, on-chain verification, custody proofs, BEEF payment receipts |

## Handler → Route Mapping

### agent.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `health` | `GET /health` | none | Liveness check — version, uptime, scheduler last tick, wallet connectivity (2s timeout) |
| `get_agent` | `GET /agent` | BRC-31 | Agent identity key, balance (30s TTL cache), tool registry, certificate status + revocation check, lifetime stats, default model, available models |
| `get_certificates` | `GET /certificates` | BRC-31 | Current certificate status via `CertificateManager`, enriched with `is_revoked` and `valid` (true when status != "none") |
| `issue_certificate` | `POST /certificates/issue` | BRC-31 | Acquire parent-signed BRC-52 cert with optional budget limits, auto-relinquish existing agent-auth certs |
| `revoke_certificate` | `POST /certificates/revoke` | BRC-31 | Parent revokes agent cert by spending `worm-revocation` basket UTXO + BRC-18 proof, with relinquish fallback |
| `relinquish_certificate` | `POST /certificates/relinquish` | BRC-31 | Relinquish self-signed cert by serial number or first agent-auth cert |
| `get_skill_analytics` | `GET /analytics/skills` | BRC-31 | Skill usage telemetry — activation counts, last activated timestamps, contexts per skill |

**Agent info** (`get_agent`): Response includes `default_model` from config and `available_models` list. Models are sourced from `model_capabilities` cache (discovered from x402-info manifests at startup) with a hardcoded fallback of 9 models (gpt-5-nano, gpt-5-mini, gpt-5, gpt-5.2, o4-mini, gpt-5.2-pro, claude-haiku-4-5, claude-sonnet-4-6, claude-opus-4-6). Each `ModelInfo` includes id, provider, context_window, max_output_tokens, max_input_tokens, supports_tools, supports_vision.

**Certificate issuance** (`issue_certificate`): Accepts optional `IssueCertRequest` body with `name`, `capabilities`, `budget_per_task`, `budget_per_hour`, `budget_per_day`, `budget_per_week`, `budget_per_month`, `budget_lifetime`, `budget_enforcement`. Pre-relinquishes all existing agent-auth certs to avoid UNIQUE constraint errors. On success, syncs budget limits from the request to the global `BudgetTracker` via `apply_cert_limits()`.

**Certificate revocation** (`revoke_certificate`): Two-step revocation: (1) relinquish the revocation UTXO from `worm-revocation` basket (falls back to `worm-state`), (2) create an on-chain BRC-18 `CertificateRevocation` proof. Uses `mgr.find_revocation_outpoint(&cert)` which tries the cert's `revocationOutpoint` field first, then scans baskets. If no revocation UTXO exists (legacy certs), falls back to `mgr.relinquish(&cert)` directly.

**Relinquish guard**: `relinquish_certificate` refuses to relinquish parent-issued certificates via a 3-condition check: (1) `certifier != subject`, (2) certifier present but subject missing, (3) non-null `revocationOutpoint`. Returns 403 Forbidden. Exception: allows relinquishing already-revoked parent-issued certs.

### analytics.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_efficiency` | `GET /analytics/efficiency` | BRC-31 | Efficiency metrics computed from transcript data via `crate::analytics::compute_efficiency()` |
| `get_cost_comparison` | `GET /analytics/cost-comparison` | BRC-31 | Replay a task with alternative model pricing (`?task_id=X&alt_model=Y`) |
| `get_roi` | `GET /analytics/roi` | BRC-31 | ROI report for current session via `crate::analytics::compute_roi()` |
| `get_benchmarks` | `GET /analytics/benchmarks` | BRC-31 | Provider benchmark summary — loads from file, backfills from transcripts if empty |

**Cost comparison** (`get_cost_comparison`): Uses `default_pricing()` table with approximate sats-per-1K-token blended rates for 9 models (gpt-5-nano, gpt-5-mini, gpt-5, gpt-5.2, o4-mini, gpt-5.2-pro, claude-haiku-4-5, claude-sonnet-4-6, claude-opus-4-6). Model lookup via `lookup_pricing()` tries exact match first, then longest prefix match for versioned names (e.g., `gpt-5-mini-2025-08-07` → `gpt-5-mini`).

### audit.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `reveal_counterparty_linkage` | `POST /audit/key-linkage/counterparty` | BRC-31 | BRC-69 counterparty key linkage revelation |
| `reveal_specific_linkage` | `POST /audit/key-linkage/specific` | BRC-31 | BRC-70 specific key linkage revelation |
| `cross_reference_message` | `GET /audit/cross-reference/{hash}` | BRC-31 | Find proofs referencing a 64-char hex SHA-256 message hash across all task transcripts |
| `list_revelations` | `GET /audit/revelations` | BRC-31 | List all past key linkage revelations from the append-only audit log |
| `audit_search` | `GET /audit/search` | none | Full-text search across all task transcripts with pagination |

**Key linkage**: Both endpoints accept `KeyLinkageRequest` (counterparty, verifier, optional protocol_id/key_id, privileged flag). Both build a `Revelation` struct with a deterministic SHA-256 hash, persist it to `audit_revelations/` via `RevelationLog`, and log to budget JSONL for audit trail.

**Audit search** (`audit_search`): `GET /audit/search?q=<query>&limit=20&offset=0`. Case-insensitive substring search across all task transcripts. Returns `AuditSearchResult` entries with task_id, event_type, timestamp, matched content snippet (200 char window centered on match), and iteration number. Limit capped at 100.

### budget.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_budget` | `GET /budget` | BRC-31 | Budget report via `BudgetTracker::report_with_balance()` (includes live wallet balance) |
| `get_bsv_usd_rate` | `GET /rates/bsv-usd` | none | Cached BSV/USD rate with source metadata, margin, and staleness tracking |
| `get_budget_detail` | `GET /budget/detail` | BRC-31 | Per-service/operation spending breakdown, optional `?task_id=` filter, includes rate limit status |
| `get_budget_export` | `GET /budget/export` | BRC-31 | Downloadable budget report — JSON (default) or PDF (`?format=pdf`) |

**Multi-source rate fetching**: `fetch_rate_multi_source()` tries WhatsOnChain first, falls back to CoinGecko. Both sources validate within sane range [$1, $100,000]. Falls back to `DEFAULT_BSV_USD_RATE` ($20) if both sources fail.

### chat.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `chat` | `POST /chat` | BRC-31 | Submit message → `spawn_task()` → returns `{ task_id, session_id }` |
| `chat_history` | `GET /chat/history` | BRC-31 | Reconstructed messages from transcript (user, think_response, tool_result, error) |
| `openai_chat_completions` | `POST /v1/chat/completions` | BRC-31 | OpenAI-format wrapper — spawns task, waits up to 300s for Done/Error event |

**Command deduplication**: `chat` supports `client_command_id` for idempotent submissions. Duplicate requests wait on a `Notify` and return the same `task_id`. Keyed by `{requester}:{client_command_id}`.

**Tag policy enforcement**: Both `chat` and `submit_task` normalize tags via `normalize_tags()` and enforce cert-driven tag policy — if `tags_required` is `true` in the certificate, requests without tags are rejected with 400.

**OpenAI compat**: Non-streaming only (`stream: true` → 501). Maps `user` field to `session_id`, `max_tokens` to `max_iterations` (capped at 50).

### compliance.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `lifecycle_status` | `GET /lifecycle/status` | none | UTXO lifecycle management — basket counts, oldest ages, sweep config, proofs growth rate |
| `compliance_report` | `GET /compliance/report` | BRC-31 | Compliance status with date-range filtering (`?from=&to=`), task summaries, retention check, USD totals |

### conversations.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `list_conversations` | `GET /conversations` | BRC-31 | All conversations, newest first |
| `get_conversation_detail` | `GET /conversations/{id}` | BRC-31 | Metadata + messages + summary stats, optional `?verify=true` for inline chain verification, self-healing from transcripts |
| `verify_conversation` | `GET /conversations/{id}/verify` | BRC-31 | BRC-60 SHA-256 hash chain integrity check |
| `sync_conversation` | `POST /conversations/{id}/sync` | BRC-31 | Encrypt and store on-chain via wallet |
| `compact_conversation` | `POST /conversations/{id}/compact` | BRC-31 | LLM-generated summary of older messages exceeding `max_history_turns` |
| `get_conversation_artifacts` | `GET /conversations/{id}/artifacts` | BRC-31 | All artifacts from all tasks in a conversation, with provenance metadata and type filter |

**Self-healing** (`get_conversation_detail`): If a conversation has task_ids but zero assistant messages, reconstructs from task transcripts on the fly via `conv_mgr.append_from_transcript()`. Metadata is reloaded after self-healing.

**Enriched detail** (`get_conversation_detail`): Response includes a `ConversationSummaryStats` with `total_iterations`, `total_proofs`, `total_artifacts`, `duration_secs`, per-task `ConversationTaskDetail` (id, status, task, iterations, sats_spent, proof_count, artifact_count, started_at, completed_at), `cost_by_service` map, and `models_used` list. Built by `build_conversation_summary()` which scans all task transcripts in the conversation.

**Conversation artifacts** (`get_conversation_artifacts`): Scans `artifacts.json` manifests and task directories across all tasks in a conversation. Enriches each artifact with `task_id`, `turn` number (derived from user messages), `turn_prompt`, and `/files/{task_id}/{name}` download URL. Supports `?type=image` filtering. Sorted by `created_at` ascending. Returns `by_type` counts.

### marketplace.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `list_plugins` | `GET /marketplace/plugins` | BRC-31 | List all plugins sorted by name, returns `{ plugins, count }` |
| `get_plugin` | `GET /marketplace/plugins/{name}` | BRC-31 | Single plugin by name |
| `create_plugin` | `POST /marketplace/plugins` | BRC-31 | Create or update a plugin listing (validates name/description/version required) |
| `delete_plugin` | `DELETE /marketplace/plugins/{name}` | BRC-31 | Delete a plugin listing |
| `services_catalog` | `GET /services/catalog` | none | Live x402 service catalog from the public agent registry via `bsv_x402_server::registry` |

**Services catalog** (`services_catalog`): Fetches the live x402 agent registry via `registry::list_agents()`. Returns each service's name, display_name, url, tagline, capabilities, category, and description. Returns empty list with error message if registry is unavailable. No authentication required.

### memory.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `list_memories` | `GET /memory` | BRC-31 | Paginated listing with `?category=`, `?offset=`, `?limit=`. Returns per-category counts. |
| `search_memories` | `GET /memory/search` | BRC-31 | BM25 tantivy search with `?q=&limit=`. Falls back to substring match if index unavailable. |
| `get_memory` | `GET /memory/{id}` | BRC-31 | Single memory entry by ID |

### misc.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `receive_message` | `POST /message` | BRC-31 | Forward MessageBox message as a new task via `spawn_task()` |
| `trigger_heartbeat` | `POST /heartbeat/trigger` | BRC-31 | Inject task into scheduler's inbox channel. Returns 503 if heartbeat disabled |

### replay.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_replay` | `GET /task/{id}/replay` | BRC-31 | Structured replay data — events, cost timeline, tool usage, duration, totals |
| `fork_task` | `POST /task/{id}/fork` | BRC-31 | Fork execution from a given event index into a new task |

### schedules.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `list_schedules_endpoint` | `GET /schedules` | BRC-31 | List all schedule JSON files, sorted by `next_run` |
| `get_schedule_endpoint` | `GET /schedules/{id}` | BRC-31 | Single schedule by ID |

### staging.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `list_staged` | `GET /staged` | BRC-31 | List all staged high-value transactions with staging threshold |
| `approve_staged` | `POST /staged/{ref}/approve` | BRC-31 | Approve a pending staged transaction — wallet signing or tool approval file |
| `abort_staged` | `POST /staged/{ref}/abort` | BRC-31 | Abort a pending staged transaction — wallet abort or tool abort file |

**Transaction staging**: Two modes based on reference prefix: wallet transactions (default) call `wallet.sign_action()`/`wallet.abort_action()`, tool approvals (`tool-` prefix) write file-based signals to `tasks/{task_id}/pending_approval/`. Both check for `pending` status (409 Conflict if already approved/aborted).

### tasks/ (lifecycle.rs)

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `submit_task` | `POST /task` | BRC-31 | Submit task → `spawn_task()`, returns 202 Accepted (enforces tag policy) |
| `get_status` | `GET /status` | BRC-31 | All tasks + active count + total sats |
| `get_task` | `GET /task/{id}` | BRC-31 | Single task info |
| `list_tasks` | `GET /tasks` | BRC-31 | All tasks (in-memory + disk), with tokens, tags, time estimates, conversation_id. Supports `?tag=X,Y` filter (AND logic) |
| `get_time_saved` | `GET /analytics/time-saved` | BRC-31 | Aggregate human-equivalent time saved across all tasks |
| `get_conversation` | `GET /task/{id}/conversation` | BRC-31 | Reconstructed conversation from transcript |
| `cancel_task` | `POST /task/{id}/cancel` | BRC-31 | Set cancel flag, release conversation lock |
| `resume_task` | `POST /task/{id}/resume` | BRC-31 | Resume an interrupted task — spawns new task on same conversation with prior context |
| `get_task_conversation_id` | `GET /task/{id}/conversation-id` | BRC-31 | Lightweight lookup of conversation_id for a task |
| `get_artifacts` | `GET /task/{id}/artifacts` | BRC-31 | List artifacts in task workspace — manifest + directory scan, with download URLs |
| `get_all_artifacts` | `GET /artifacts` | BRC-31 | List artifacts across ALL tasks, supports `?type=` filter, sorted by `created_at` descending |
| `serve_task_file` | `GET /files/{task_id}/{filename}` | none | Static file serving with path traversal protection, falls back to `uploads/` subdirectory |

**Task resume** (`resume_task`): Only tasks with `TaskStatus::Interrupted` can be resumed. Reconstructs session state from the interrupted transcript via `transcript.reconstruct_state()`. Spawns a new task with tags `["resume", "resumed-from:{id}"]` and origin `"resume"`. Marks the original task as `Complete`. Returns 202 with `resumed_task_id`, `session_id`, and prior iteration/sats context.

**Artifacts** (`get_artifacts`, `get_all_artifacts`): Reads `artifacts.json` manifest if present, then scans the task directory for unlisted files. Skips system files (`session.jsonl`, `budget.jsonl`, `artifacts.json`, `fork_context.json`), hidden files, `tool_output_*` files, and directories. Infers artifact type from extension (image, video, audio, document, file). Adds `/files/{task_id}/{name}` download URLs. `get_all_artifacts` enriches each artifact with `task_id` and `task_text`.

### tasks/ (audit.rs)

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_audit` | `GET /task/{id}/audit` | BRC-31 | Full audit timeline from transcript with summary stats and `conversation_id` |
| `get_audit_export` | `GET /task/{id}/audit/export` | BRC-31 | Downloadable audit report — CSV (default), signed JSON (`?format=json`), or PDF (`?format=pdf`) |
| `verify_transcript` | `GET /task/{id}/transcript/verify` | BRC-31 | Verify transcript HMAC integrity via wallet |

**Audit export signed JSON**: Wallet-signed SHA-256 hash with protocol `[2, "dolphin milk audit"]` and key_id `"audit-export"`. Includes signature, agent identity, and proofs list.

**Transcript HMAC verification**: Calls `wallet.verify_hmac()` with protocol `[2, "dolphin milk transcript"]` and key_id = task_id.

### tasks/ (events.rs)

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_task_events` | `GET /task/{id}/events` | BRC-31 | Incremental transcript events for poll-based UI (`?since=N`) |
| `get_escalation` | `GET /task/{id}/escalation` | BRC-31 | Get escalation status for a task from `AppState.escalation_status` |
| `resolve_escalation` | `POST /task/{id}/escalation/resolve` | BRC-31 | Resolve escalation with human guidance — clears escalated flag, records resolution |
| `get_escalation_proof` | `GET /task/{id}/escalation/proof` | BRC-31 | Escalation BRC-18 proofs for a task |

### tasks/ (proofs.rs)

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_proofs` | `GET /task/{id}/proofs` | BRC-31 | Proof details from transcript (Decision, Checkpoint, etc.) with prev_hash chain |
| `verify_proofs` | `GET /task/{id}/proofs/verify` | BRC-31 | On-chain verification — recompute SHA-256 hashes, cross-check with `dm-proofs` basket |
| `get_custody_proof` | `GET /task/{id}/proofs/custody` | BRC-31 | Custody proof for billing verification |
| `get_receipts` | `GET /task/{id}/receipts` | BRC-31 | BEEF payment receipts from `tasks/{id}/beef/*.json` |

### wallet_ops.rs

| Handler | Route | Auth | Description |
|---------|-------|------|-------------|
| `get_output` | `GET /output/{basket}/{txid}` | none | Fetch UTXO locking script + parsed PushDrop data fields from wallet |
| `decrypt_data` | `POST /decrypt` | BRC-31 | Decrypt ciphertext via wallet BRC-42, returns plaintext + optional parsed JSON |
| `get_funding_address` | `GET /wallet/address` | none | Compute BSV P2PKH address from wallet receive script via Base58 encoding |
| `check_funding` | `POST /wallet/check-funding` | none | Check WhatsOnChain for incoming UTXOs at the funding address and auto-internalize via `fund_from_woc()` |
| `get_wallet_utxos` | `GET /wallet/utxos` | BRC-31 | Spendable UTXO set from default basket with tier summaries (dust/small/medium/large) |

**Funding address** (`get_funding_address`): Calls `wallet.receive_address("1")`, extracts the 20-byte pubkey hash from the P2PKH script, computes a BSV mainnet address (version byte 0x00 + double-SHA256 checksum) via `base58_encode()`.

**Check funding** (`check_funding`): Derives the funding address, queries WhatsOnChain for unspent UTXOs, and auto-internalizes each via `wallet.fund_from_woc()`. Returns `found`, `internalized` count, and current balance. If no UTXOs found, still returns the current balance.

**UTXO listing** (`get_wallet_utxos`): Paginates through all spendable outputs in the `default` basket. Computes tier summary: dust (<1K sats), small (1K-100K), medium (100K-1M), large (>1M). Returns full UTXO list with outpoint and satoshi amounts.

## Key Patterns

### Auth + Signed Response
Every handler follows: `check_brc31_auth()` → business logic → `signed_json_response()`. Dev mode (no parent key) returns `Ok(None)` from auth and plain JSON responses.

### Task Spawning
`chat`, `submit_task`, `receive_message`, `fork_task`, and `resume_task` all delegate to `spawn_task()` from `server/task_spawner.rs`. Both `chat` and `submit_task` enforce cert-driven tag policy before spawning.

### Transcript as Source of Truth
Multiple handlers read from JSONL transcripts at `working/tasks/{id}/session.jsonl`. The transcript is the authoritative record — in-memory `TaskInfo` is supplementary.

## Local Types

Each file defines its own request/response types rather than putting everything in `types.rs`:

- **agent.rs**: `RelinquishRequest`, `IssueCertRequest`
- **analytics.rs**: `CostComparisonParams`
- **audit.rs**: `KeyLinkageRequest`, `KeyLinkageResponse`, `CrossReferenceMatch`, `CrossReferenceResponse`, `AuditSearchParams`, `AuditSearchResult` (pub), `AuditSearchResponse` (pub)
- **budget.rs**: `CachedRate` (pub), `BudgetExportQuery`
- **chat.rs**: `OpenAIChatRequest`, `OpenAIMessage`, `OpenAIChatResponse`, `OpenAIChoice`, `OpenAIUsage`
- **compliance.rs**: `ComplianceReportQuery`
- **conversations.rs**: `ConversationDetailParams`, `ConversationDetailWithVerification`, `ConversationSummaryStats`, `ConversationTaskDetail`, `ConversationArtifactsParams`
- **marketplace.rs**: `PluginListing` (pub), `PluginListResponse` (pub)
- **replay.rs**: `ReplayResponse`, `ForkRequest`, `ForkResponse`
- **tasks/lifecycle.rs**: `TaskListParams`, `AllArtifactsParams`
- **tasks/audit.rs**: `AuditExportQuery`, `TranscriptVerifyResponse`
- **tasks/events.rs**: `TranscriptEventDto`, `TaskEventsResponse`, `EventsQuery`, `ResolveEscalationRequest`, `EscalationProofResponse`, `EscalationProofDetail`
- **tasks/proofs.rs**: `ProofVerification` (pub), `VerifyResponse`, `ReceiptsResponse`, `ReceiptDetail`
- **wallet_ops.rs**: `OutputResponse` (pub), `DecryptRequest` (pub), `DecryptResponse`, `WalletUtxosResponse` (pub), `UtxoInfo` (pub), `TierSummary` (pub), `TierInfo` (pub)

Shared types (`TaskInfo`, `TaskStatus`, `TaskRequest`, `ChatRequest`, `MemoryListParams`, etc.) live in `server/types.rs`.

## Dependencies

- `super::super::auth` — `check_brc31_auth()`, `signed_json_response()`, `signed_raw_response()`
- `super::super::task_spawner` — `spawn_task()`
- `super::super::types` — Shared request/response DTOs
- `super::super::AppState` — Shared server state
- `crate::wallet::WalletBackend` — Wallet HTTP client
- `crate::transcript::Transcript` — JSONL transcript reader
- `crate::conversation::ConversationManager` — Multi-turn conversation CRUD and verification
- `crate::memory::{MemoryStore, MemoryIndex}` — Memory storage and BM25 search
- `crate::certificates::CertificateManager` — BRC-52 certificate operations
- `crate::analytics` — Efficiency, cost replay, ROI, benchmarks computation
- `crate::replay::{ReplayViewer, ForkExecutor, ForkParams}` — Task replay and fork
- `crate::time_estimate` — Human-equivalent time-saved estimation
- `crate::runner::escalation` — Escalation types
- `crate::audit::revelation` — `Revelation`, `RevelationLog`
- `crate::onchain::state` — Basket constants, `parse_push_drop_fields()`
- `crate::server::pdf_template` — HTML template rendering for PDF export
- `crate::heartbeat::estimate_proofs_per_day` — Proofs growth rate estimation
- `bsv_x402_server::registry` — x402 agent registry (used by `services_catalog`)
- `chromiumoxide` — Headless Chrome for HTML-to-PDF conversion

## Related

- `../CLAUDE.md` — Parent server module (router setup, AppState, middleware)
- `../../auth/CLAUDE.md` — BRC-31 Authrite client + server
- `../../tools/CLAUDE.md` — Tool registry (used by `get_agent` to list tools)
- `../../memory/CLAUDE.md` — Memory storage and search index
- `../../context/CLAUDE.md` — Context window management and compaction
- `../../onchain/CLAUDE.md` — Budget tracker, proofs, state tokens
