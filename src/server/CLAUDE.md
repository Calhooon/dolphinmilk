# src/server/
> Axum 0.8 HTTP server — ~79 REST routes, BRC-31 mutual auth, unified task spawning, Prometheus metrics, and static UI serving.

## Overview

The HTTP server is the primary external interface for the Dolphin Milk daemon. It exposes task management, chat (with image attachments), budget tracking, memory browsing, conversation management, certificate management, audit/compliance (including revelation audit logging and custody proofs), analytics (efficiency, ROI, cost comparison, benchmarks, context analysis), plugin marketplace, x402 service catalog, task replay/forking, task resume (interrupted recovery), artifact management, escalation management, wallet operations (funding, UTXOs), Prometheus metrics, and transaction staging endpoints. All authenticated routes use BRC-31 Authrite mutual authentication. Tasks are spawned via a single unified `spawn_task()` function that handles conversation tracking, continuation detection, image attachment processing, conversation-level FIFO queuing, MCP wallet tool injection, rate limiting, circuit breaker injection, Prometheus metrics recording, skill telemetry merging, and reply routing. Certificate revocation blocks new task creation. The Lit frontend is served as static files at `/ui/`.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 958 | Module declarations, re-exports (`types::*`, `app_state::*`, `spawn_task`, `AuditSearchResponse`/`AuditSearchResult`, `PluginListing`/`PluginListResponse`, `Revelation`/`RevelationsListResponse`), ~900 lines of inline tests |
| `app_state.rs` | 1321 | `AppState` and sub-structs (`TaskManager`, `AuthState`, `SchedulerChannels`, `LifetimeStats`, `StagedTransaction`), shared `WalletBackend` (HTTP-first, embedded fallback), `create_app_state()` async constructor (10 phases: wallet backend selection, wallet init, cert check, rate limiter, scheduler channels, historical task scan, MCP wallet connect, tool cache, skill definitions, model capability pre-fetch), `build_router_with_state()`, `build_router()`, `serve()` entry point, chat command dedupe, BSV/USD rate refresh, Prometheus `/metrics` endpoint, CORS configuration |
| `types.rs` | 438 | Shared request/response DTOs: `TaskInfo` (with `tags`, `origin`, `conversation_id`), `TaskStatus` (incl. `Interrupted`), `TaskRequest`/`TaskResponse`, `ChatRequest` (with `tags`, `attachments`), `ChatAttachment` + attachment constants (`MAX_ATTACHMENT_SIZE` 5MB, `MAX_ATTACHMENTS` 10, `ALLOWED_ATTACHMENT_MIMES`), `MessageRequest`/`MessageResponse`, `HeartbeatTriggerRequest`, `HealthResponse` (with `wallet_connected`, `wallet_url`), `StatusResponse`, `AgentResponse` (with `default_model` + `available_models`), `ModelInfo`, `AuditResponse`/`AuditEvent`/`AuditSummary`, `ProofsResponse`/`ProofDetail`, `ConversationResponse`, `BudgetDetailResponse`/`ServiceBreakdown`/`OperationStats`, `MemoryListResponse`/`MemoryDetailResponse`/`MemorySearchResponse`, `TaskListResponse`/`TaskSummary` (with `tags`, `time_saved_minutes`, `origin`, `conversation_id`), `normalize_tags()` helper |
| `task_spawner.rs` | 781 | Unified `spawn_task()` — single entry point for all task creation (chat, HTTP, heartbeat, WebSocket). Certificate revocation gate, per-conversation semaphore FIFO queuing, continuation detection, conversation create/resume, image attachment processing (base64 decode → workspace uploads + artifacts.json manifest), model override, MCP wallet tool registration, rate limiter + circuit breaker injection, Prometheus metrics wiring, staged-transactions approval map, discovered model capability application, BRC-60 hash chain verification at conversation-bound task start, prior message injection, external-origin marking, proof chain restoration, conversation integrity proofs, wallet sync, reply routing via MessageBox, delivery queue fallback, budget merge into global tracker, skill telemetry merge, Prometheus task completion recording, system event emission, and panic-recovery supervisor |
| `auth.rs` | 359 | BRC-31 Authrite helpers: `headermap_to_pairs()` converts axum headers, `check_brc31_auth()` verifies request headers against stored sessions, `signed_json_response()` signs JSON response bodies with `x-bsv-auth-*` headers, `signed_raw_response()` signs non-JSON responses (CSV/JSON exports) with auth headers, `brc31_handshake()` handles `POST /.well-known/auth` |
| `pdf_template.rs` | 469 | HTML template rendering for PDF export. `render_audit_html()` generates styled audit trail HTML from events/proofs/summary. `render_budget_html()` generates budget report HTML from spending data. `html_escape()` XSS-safe escaping. Used by audit export and budget export handlers with `html_to_pdf()` in tasks/audit.rs |
| `ui_assets.rs` | 51 | Embedded UI asset serving via `rust-embed`. `UiAssets` struct embeds `ui/dist` at compile time. `serve_embedded_ui()` serves files with MIME detection and SPA fallback (returns `index.html` for unknown routes). Feature-gated behind `embed-ui` |
| `handlers/mod.rs` | 17 | Re-exports all 15 handler submodules as `pub(crate)` |
| `handlers/agent.rs` | 748 | `health` (with `wallet_connected`, `wallet_url`), `get_agent` (with available models list), `get_certificates`, `issue_certificate` (with budget limits), `revoke_certificate`, `relinquish_certificate`, `get_skill_analytics` |
| `handlers/analytics.rs` | 248 | `get_efficiency`, `get_cost_comparison` (with default pricing for 9 models + prefix-match lookup for versioned names), `get_roi`, `get_context_analysis` (context window utilization), `get_benchmarks` (with transcript backfill) |
| `handlers/audit.rs` | 615 | BRC-69/70 key linkage revelation (with append-only revelation log), `list_revelations`, cross-agent proof cross-referencing, full-text audit search across all transcripts |
| `handlers/budget.rs` | 412 | `get_budget`, `get_budget_detail` (with rate limit status), `get_bsv_usd_rate` (multi-source: WhatsOnChain + CoinGecko fallback, margin, staleness tracking), `get_budget_export` (JSON/PDF) |
| `handlers/chat.rs` | 495 | `chat` (with idempotency dedupe + tag policy enforcement + image attachments), `chat_history`, `openai_chat_completions` (non-streaming) |
| `handlers/compliance.rs` | 282 | `lifecycle_status` (UTXO basket counts, oldest ages, proofs growth rate, sweep config), `compliance_report` (date-range filtering, task summaries, retention check, USD totals) |
| `handlers/conversations.rs` | 657 | `list_conversations`, `get_conversation_detail` (with self-healing, enriched summary stats, per-task breakdown), `verify_conversation`, `sync_conversation`, `compact_conversation`, `get_conversation_artifacts` (cross-task artifact listing with provenance and type filter) |
| `handlers/marketplace.rs` | 245 | Plugin marketplace CRUD: `list_plugins`, `get_plugin`, `create_plugin`, `delete_plugin`. Live x402 `services_catalog` from agent registry. Stores JSON in `workspace/marketplace/` |
| `handlers/memory.rs` | 188 | `list_memories`, `search_memories`, `get_memory` |
| `handlers/misc.rs` | 88 | `receive_message` (forward MessageBox message as task), `trigger_heartbeat` (manual scheduler injection) |
| `handlers/replay.rs` | 201 | `get_replay` (structured timeline with cost/tool usage), `fork_task` (fork execution from event index into new task) |
| `handlers/schedules.rs` | 89 | `list_schedules_endpoint` (list all schedule JSONs sorted by `next_run`), `get_schedule_endpoint` (single schedule by ID) |
| `handlers/staging.rs` | 287 | `list_staged`, `approve_staged` (wallet signing + tool approval modes), `abort_staged`. Two-phase commit for high-value operations |
| `handlers/wallet_ops.rs` | 456 | `get_output` (UTXO lookup + PushDrop parsing), `decrypt_data` (BRC-42 wallet decryption), `get_funding_address` (BSV P2PKH address), `check_funding` (WoC UTXO lookup + auto-internalize), `get_wallet_utxos` (spendable UTXO set with tier summaries) |
| `handlers/tasks/mod.rs` | 11 | Re-exports 4 task handler submodules (`audit`, `events`, `lifecycle` as `pub(crate)`, `proofs` as `pub`) |
| `handlers/tasks/lifecycle.rs` | 1027 | `submit_task`, `get_status`, `get_task`, `list_tasks` (with tag filtering + time estimates), `get_time_saved`, `get_conversation`, `cancel_task`, `resume_task` (interrupted recovery), `get_task_conversation_id`, `get_artifacts` (per-task), `get_all_artifacts` (global with type filter), `serve_task_file` |
| `handlers/tasks/audit.rs` | 618 | `get_audit`, `get_audit_export` (CSV/signed JSON/PDF), `verify_transcript`, `html_to_pdf` helper |
| `handlers/tasks/events.rs` | 342 | `get_task_events` (poll-based), `get_escalation`, `resolve_escalation`, `get_escalation_proof` |
| `handlers/tasks/proofs.rs` | 586 | `get_proofs`, `verify_proofs`, `get_custody_proof`, `get_receipts` |

## Key Exports

### AppState and sub-structs (`app_state.rs`)

```rust
pub struct AppState {
    pub config: DmConfig,
    pub workspace: PathBuf,
    pub wallet: Arc<dyn WalletBackend + Send + Sync>,
    pub budget: Mutex<BudgetTracker>,
    pub started_at: Instant,
    pub events_tx: broadcast::Sender<(String, StepEvent)>,
    pub task_mgr: TaskManager,
    pub auth: AuthState,
    pub scheduler: SchedulerChannels,
    pub stats: LifetimeStats,
    pub config_rx: Option<watch::Receiver<DmConfig>>,
    pub usd_rate_cache: RwLock<Option<CachedRate>>,
    pub cached_tool_names: Vec<String>,
    pub cached_balance: RwLock<Option<(u64, Instant)>>,
    pub staged_transactions: Arc<Mutex<HashMap<String, StagedTransaction>>>,
    pub wallet_mcp: Option<Arc<McpClient>>,
    pub wallet_mcp_tools: Vec<(String, String, serde_json::Value)>,
    pub certificate_revoked: Arc<AtomicBool>,
    pub rate_limiter: Arc<RateLimiterRegistry>,
    pub skill_telemetry: Mutex<SkillTelemetry>,
    pub skill_definitions: Vec<(String, String, bool)>,
    pub escalation_status: Mutex<HashMap<String, EscalationStatus>>,
    pub metrics: Arc<MetricsRegistry>,
    pub model_capabilities: Arc<RwLock<HashMap<String, ModelCapabilities>>>,
    pub circuit_breakers: Arc<CircuitBreakerRegistry>,
}
```

- **`wallet`** — Shared `WalletBackend` trait object created once at startup. Strategy: HTTP-first (respects running wallet at configured URL), embedded fallback (in-process `bsv-wallet-toolbox-rs` if HTTP unreachable and `embedded-wallet` feature enabled). Never opens embedded wallet when external wallet is running (avoids SQLite lock contention).
- **`TaskManager`** — In-memory task registry (`tasks`), cancel flags, task-to-session mapping (`task_sessions`), per-conversation semaphores (`conversation_semaphores` — 1 permit each, FIFO queuing), active task count for scheduler limiting, and chat command idempotency map.
- **`AuthState`** — BRC-31 session store, server identity key (fetched from wallet at startup), SDK `HttpWalletJson` for response signing.
- **`SchedulerChannels`** — Heartbeat trigger channel, system event channel, last tick timestamp. All `Option` — `None` when heartbeat disabled.
- **`LifetimeStats`** — Atomic counters for total sats spent and task count, loaded from disk transcript scan at startup.
- **`StagedTransaction`** — High-value transaction awaiting manual approval. Fields: reference, amount_sats, service, created_at, status (`"pending"` / `"approved"` / `"aborted"`), optional task_id and description. Wrapped in `Arc<Mutex>` so it can be shared with the runner for tool approval workflows.
- **`wallet_mcp`** — Optional shared MCP client for the wallet process (`bsv-wallet-toolbox`). `None` if the binary is not found. Initialized once at startup and shared across all tasks.
- **`wallet_mcp_tools`** — Cached MCP tool summaries `(name, description, input_schema)` discovered at startup. Registered into each task's `ToolRegistry` in `spawn_task()`.
- **`certificate_revoked`** — `Arc<AtomicBool>` set by boot cert check, heartbeat periodic check, or `POST /certificates/revoke`. When `true`, `spawn_task()` returns 403 Forbidden. Cleared by `POST /certificates/issue` on successful new cert acquisition.
- **`rate_limiter`** — `Arc<RateLimiterRegistry>` constructed from config + cert overrides at startup. Shared across all tasks via `runner::create_loop_with_rate_limiter()`. Disabled when rate limiting is off.
- **`skill_telemetry`** — Accumulated skill usage telemetry (activation counts, timestamps, contexts) merged from each task's runner after completion.
- **`skill_definitions`** — Skill metadata `(name, description, auto_activate)` loaded once at startup.
- **`escalation_status`** — Per-task escalation status map used by `GET /task/{id}/escalation` and `POST /task/{id}/escalation/resolve`.
- **`metrics`** — `Arc<MetricsRegistry>` for Prometheus metrics. Shared with each runner. Exposed at `GET /metrics`.
- **`model_capabilities`** — `Arc<RwLock<HashMap<String, ModelCapabilities>>>` pre-fetched from x402-info manifests at startup. Maps model name → capabilities (context_window, max_output_tokens, supports_tools, supports_vision). Applied to each runner in `spawn_task()` via `apply_model_capabilities()` to override hardcoded context limits. Best-effort — empty on fetch failure.
- **`circuit_breakers`** — `Arc<CircuitBreakerRegistry>` for x402 LLM provider failover. Shared across all tasks. When a provider fails repeatedly, `think_with_circuit_breaker()` routes to the alternate provider. Initialized once at startup.

### Public functions (`app_state.rs`)

| Function | Description |
|----------|-------------|
| `create_app_state(config, workspace)` | Async constructor — 10 phases: (0) wallet backend selection (HTTP-first, embedded fallback), (1) wallet init + identity key fetch, (2) BRC-52 cert check + budget limit application, (3) rate limiter from config + cert overrides, (4) scheduler channels, (5) historical task scan (lifetime stats, orphan recovery, TaskInfo reconstruction), (6) MCP wallet connect, (7) tool name cache, (8) skill definitions, (9) background model capability pre-fetch |
| `build_router_with_state(state)` | Builds axum Router with all ~79 routes, Prometheus `/metrics` endpoint, and CORS layer |
| `build_router(config, workspace)` | Convenience wrapper combining `create_app_state` + `build_router_with_state` |
| `serve(config, workspace, port)` | Full server startup — creates state, starts config watcher, spawns scheduler with panic-recovery, spawns BSV/USD rate refresh, binds to port |
| `spawn_rate_refresh(state)` | Background task refreshing BSV/USD rate at configurable interval (default 15 minutes) |

### Auth helpers (`auth.rs`)

| Function | Description |
|----------|-------------|
| `check_brc31_auth(state, method, path, query, headers, body)` | Verify BRC-31 request auth. Returns `Ok(None)` in dev mode (no parent key), `Ok(Some(Brc31AuthContext))` on success, `Err(401/403)` on failure |
| `signed_json_response(state, auth_ctx, status, body)` | Sign a JSON response with `x-bsv-auth-*` headers. Plain JSON if `auth_ctx` is `None` |
| `signed_raw_response(state, auth_ctx, status, content_type, body_bytes, extra_headers)` | Sign a non-JSON response (CSV, JSON export files) with BRC-31 auth headers. Accepts arbitrary content type and extra headers (e.g. `Content-Disposition`) |
| `brc31_handshake(state, req)` | `POST /.well-known/auth` handler — creates session, signs response if server has identity key |

### PDF templates (`pdf_template.rs`)

| Function | Description |
|----------|-------------|
| `html_escape(s)` | XSS-safe escaping for 5 critical chars: `& < > " '` |
| `render_audit_html(task_id, events, proofs, summary)` | Styled HTML document for audit trail with summary stats, proof cards, and event timeline table |
| `render_budget_html(detail)` | Styled HTML document for budget report with service breakdown and recent spending entries |

### Request/Response types (`types.rs`)

Core task types: `TaskInfo` (with `tags` for ROI tracking, `origin` for spawn source, `conversation_id`), `TaskStatus` (Queued/Running/Complete/Error/Cancelled/Interrupted), `TaskRequest` (with optional `tags`, `model`), `TaskResponse`, `TaskSummary` (includes `tokens`, `tags`, `time_saved_minutes`, `origin`, `conversation_id` fields), `TaskListResponse`.

Chat types: `ChatRequest` (with `client_command_id` for idempotency, `model` override, `session_id` for conversation continuity, optional `tags`, optional `attachments`), `ChatAttachment` (base64 data, mime_type, filename), `ChatMessage`, `HistoryParams`. Attachment constants: `MAX_ATTACHMENT_SIZE` (5 MB), `MAX_ATTACHMENTS` (10), `ALLOWED_ATTACHMENT_MIMES` (png, jpeg, gif, webp).

Audit types: `AuditResponse`, `AuditEvent`, `AuditSummary`, `ProofsResponse`, `ProofDetail`.

Budget types: `BudgetDetailResponse`, `SpendingEntryDto`, `ServiceBreakdown`, `OperationStats`.

Memory types: `MemoryListResponse`, `MemoryListItem`, `MemoryDetailResponse`, `MemorySearchResponse`, `MemorySearchResultItem`.

Agent types: `AgentResponse` (includes `certificate_status`, `certificate_revoked`, `default_model`, and `available_models` fields), `ModelInfo` (id, provider, context_window, max_output_tokens, max_input_tokens, supports_tools, supports_vision).

Other: `MessageRequest`, `MessageResponse`, `HeartbeatTriggerRequest`, `HealthResponse`, `StatusResponse`, `ConversationResponse`, `normalize_tags()` helper.

## Route Table

| Method | Path | Handler | Auth | Description |
|--------|------|---------|------|-------------|
| GET | /health | `agent::health` | none | Liveness (version, uptime, scheduler tick age, wallet connectivity) |
| POST | /.well-known/auth | `auth::brc31_handshake` | none | BRC-31 Authrite handshake |
| POST | /task | `tasks::submit_task` | optional | Submit task with optional tags, returns 202 with task_id |
| GET | /status | `tasks::get_status` | optional | In-memory task list + budget summary |
| GET | /task/{id} | `tasks::get_task` | optional | Single task status |
| GET | /tasks | `tasks::list_tasks` | optional | All tasks (memory + disk), supports `?tag=X,Y` filter |
| GET | /task/{id}/audit | `tasks::get_audit` | optional | Full transcript audit chain + summary |
| GET | /task/{id}/audit/export | `tasks::get_audit_export` | optional | Downloadable audit report (CSV, signed JSON, or PDF) |
| GET | /task/{id}/proofs | `tasks::get_proofs` | optional | Proof txids with hash, type, iteration |
| GET | /task/{id}/proofs/verify | `tasks::verify_proofs` | optional | Recompute hashes + check on-chain |
| GET | /task/{id}/proofs/custody | `tasks::get_custody_proof` | optional | Custody proof for billing verification |
| GET | /task/{id}/receipts | `tasks::get_receipts` | optional | BEEF payment receipts per iteration |
| GET | /task/{id}/conversation | `tasks::get_conversation` | optional | Reconstructed messages from transcript |
| GET | /task/{id}/events | `tasks::get_task_events` | optional | Incremental events since offset (poll-based UI) |
| GET | /task/{id}/transcript/verify | `tasks::verify_transcript` | optional | Verify transcript HMAC integrity via wallet |
| GET | /task/{id}/replay | `replay::get_replay` | optional | Structured replay data (events, cost timeline, tool usage) |
| POST | /task/{id}/fork | `replay::fork_task` | optional | Fork execution from event index into new task |
| POST | /task/{id}/cancel | `tasks::cancel_task` | optional | Set cancel flag on running task |
| GET | /task/{id}/escalation | `tasks::get_escalation` | optional | Get escalation status for a task |
| GET | /task/{id}/escalation/proof | `tasks::get_escalation_proof` | optional | Escalation BRC-18 proofs for a task |
| POST | /task/{id}/escalation/resolve | `tasks::resolve_escalation` | optional | Resolve escalation with human guidance |
| GET | /agent | `agent::get_agent` | optional | Identity, balance, tools, cert status |
| POST | /message | `misc::receive_message` | optional | Forward MessageBox message as task |
| GET | /files/{task_id}/{filename} | `tasks::serve_task_file` | none | Serve task workspace files (path traversal protected) |
| GET | /output/{basket}/{txid} | `wallet_ops::get_output` | none | Fetch UTXO + parse PushDrop fields |
| POST | /decrypt | `wallet_ops::decrypt_data` | optional | Decrypt ciphertext via wallet |
| GET | /certificates | `agent::get_certificates` | optional | Certificate status |
| POST | /certificates/issue | `agent::issue_certificate` | optional | Acquire parent-signed certificate (with budget limits) |
| POST | /certificates/revoke | `agent::revoke_certificate` | optional | Parent revokes agent cert by spending revocation UTXO |
| POST | /certificates/relinquish | `agent::relinquish_certificate` | optional | Relinquish self-signed cert (parent-issued blocked, returns 403) |
| GET | /conversations | `conversations::list_conversations` | optional | List all conversations |
| GET | /conversations/{id} | `conversations::get_conversation_detail` | optional | Metadata + messages (optional verify) |
| GET | /conversations/{id}/verify | `conversations::verify_conversation` | optional | BRC-60 hash chain integrity check |
| POST | /conversations/{id}/sync | `conversations::sync_conversation` | optional | Encrypt + store on-chain via wallet |
| POST | /conversations/{id}/compact | `conversations::compact_conversation` | optional | LLM-summarize old messages |
| GET | /memory | `memory::list_memories` | optional | List with category filter + pagination |
| GET | /memory/search | `memory::search_memories` | optional | BM25 search (falls back to text match) |
| GET | /memory/{id} | `memory::get_memory` | optional | Single memory entry |
| GET | /budget/detail | `budget::get_budget_detail` | optional | Spending breakdown by service/operation + rate limit status |
| GET | /budget/export | `budget::get_budget_export` | optional | Downloadable budget report (JSON or PDF) |
| GET | /rates/bsv-usd | `budget::get_bsv_usd_rate` | none | Multi-source BSV/USD rate (WhatsOnChain + CoinGecko fallback) |
| GET | /metrics | `get_metrics` | optional | Prometheus metrics endpoint |
| GET | /analytics/skills | `agent::get_skill_analytics` | optional | Skill usage telemetry |
| GET | /analytics/efficiency | `analytics::get_efficiency` | optional | Efficiency metrics from transcript data |
| GET | /analytics/cost-comparison | `analytics::get_cost_comparison` | optional | Replay task with alternative model pricing |
| GET | /analytics/roi | `analytics::get_roi` | optional | ROI report for current session |
| GET | /analytics/benchmarks | `analytics::get_benchmarks` | optional | Provider benchmark summary |
| GET | /analytics/time-saved | `tasks::get_time_saved` | optional | Aggregate human-equivalent time saved |
| GET | /schedules | `schedules::list_schedules_endpoint` | optional | List recurring schedules |
| GET | /schedules/{id} | `schedules::get_schedule_endpoint` | optional | Single schedule detail |
| GET | /lifecycle/status | `compliance::lifecycle_status` | none | UTXO lifecycle — basket counts, oldest ages, sweep config |
| GET | /compliance/report | `compliance::compliance_report` | optional | Compliance status with date-range filtering |
| GET | /audit/search | `audit::audit_search` | none | Full-text search across all task transcripts |
| GET | /audit/revelations | `audit::list_revelations` | optional | List all past key linkage revelations |
| POST | /audit/key-linkage/counterparty | `audit::reveal_counterparty_linkage` | BRC-31 | BRC-69 counterparty key linkage revelation |
| POST | /audit/key-linkage/specific | `audit::reveal_specific_linkage` | BRC-31 | BRC-70 specific key linkage revelation |
| GET | /audit/cross-reference/{hash} | `audit::cross_reference_message` | optional | Find proofs referencing a message hash |
| GET | /staged | `staging::list_staged` | optional | List staged high-value transactions |
| POST | /staged/{ref}/approve | `staging::approve_staged` | optional | Approve a pending staged transaction |
| POST | /staged/{ref}/abort | `staging::abort_staged` | optional | Abort a pending staged transaction |
| GET | /marketplace/plugins | `marketplace::list_plugins` | optional | List all plugin listings |
| POST | /marketplace/plugins | `marketplace::create_plugin` | optional | Create or update a plugin listing |
| GET | /marketplace/plugins/{name} | `marketplace::get_plugin` | optional | Get a specific plugin |
| DELETE | /marketplace/plugins/{name} | `marketplace::delete_plugin` | optional | Delete a plugin listing |
| POST | /heartbeat/trigger | `misc::trigger_heartbeat` | BRC-31 | Manual scheduler task injection |
| POST | /chat | `chat::chat` | BRC-31 | Submit message with optional tags, returns task_id + session_id |
| GET | /chat/history | `chat::chat_history` | BRC-31 | Conversation from transcript JSONL |
| GET | /budget | `budget::get_budget` | BRC-31 | Budget report |
| POST | /v1/chat/completions | `chat::openai_chat_completions` | BRC-31 | OpenAI-compatible (conditional on config) |
| GET | /ui/* | (ServeDir) | none | Lit frontend static files |

"optional" auth = BRC-31 enforced when `config.parent.identity_key` is set; skipped in dev mode.

## Key Patterns

### Auth flow
Every auth-gated handler calls `check_brc31_auth()` → gets `Option<Brc31AuthContext>` → passes context to `signed_json_response()` (or `signed_raw_response()` for file downloads) which signs the response body. Dev mode (empty parent key) returns `Ok(None)` from auth and plain responses from the signing helpers. The handshake endpoint stores sessions keyed by server nonce with 1-hour TTL.

### Signed response variants
- `signed_json_response()` — Standard JSON API responses. Serializes `&T: Serialize` to bytes, signs, attaches auth headers.
- `signed_raw_response()` — Non-JSON responses (CSV exports, JSON file downloads, PDF). Accepts pre-built `Vec<u8>` body, custom `content_type`, and `extra_headers` (e.g. `Content-Disposition` for file downloads). Both functions degrade to plain responses when `auth_ctx` is `None`.

### Task spawning and conversation queuing
`spawn_task()` is the single entry point for all task creation. It first checks `certificate_revoked` — returning 403 if the agent's certificate is revoked or missing. It uses per-conversation semaphores (1 permit each) so tasks targeting the same conversation queue in FIFO order instead of being rejected. Tasks start as `Queued` if the semaphore isn't immediately available, transitioning to `Running` once the permit is acquired. Steps:
1. Checks `certificate_revoked` flag — rejects with 403 if set
2. Detects continuations (`CONTINUATION:` prefix) and restores proof chain linkage
3. Creates or resumes a conversation — appends user message to hash chain immediately (visible while queued)
4. Acquires per-conversation semaphore (waits if another task is running on this conversation)
5. Checks if cancelled while queued; if so, marks `Cancelled` and exits
6. Loads fresh prior messages AFTER acquiring permit (includes previous task's results)
7. Applies model override from request
8. Creates runner via `create_loop_with_rate_limiter()` injecting the shared rate limiter and circuit breaker registry
9. Registers MCP wallet tools into the task's `ToolRegistry` (if MCP client available)
10. Wires staged-transactions map for tool approval workflows and Prometheus metrics registry
11. Applies discovered model capabilities from `model_capabilities` cache to override hardcoded context limits (resolution chain: discovered → hardcoded → config cap)
12. Verifies BRC-60 hash chain at conversation-bound task start (if prior messages exist)
13. Marks external-origin tasks (from `POST /message`) for tool restriction
14. Spawns a tokio task running `worm.run()`
15. On completion: appends transcript to conversation, creates BRC-18 integrity proof, auto-syncs to wallet, merges runner budget entries into global tracker, merges skill telemetry into global accumulator, records Prometheus task completion metrics, updates task status and lifetime stats, routes replies via MessageBox, emits `TaskCompleted` system event
16. Spawns a panic-recovery supervisor that marks tasks as Error if the inner task panics, attempts best-effort conversation assembly from whatever transcript exists, and decrements active task count

### MCP wallet bridge
At startup, `create_app_state()` attempts to connect to a `bsv-wallet-toolbox` MCP server. If found, it discovers available tools and caches them as `wallet_mcp_tools`. In `spawn_task()`, these tools are converted to `ToolDef` structs via `mcp_tools_to_tooldefs()` and registered into each task's `ToolRegistry`. Tools named `wallet_balance` are excluded (already provided natively). This bridges the typed wallet MCP tools into the agent's tool system without code changes to the runner.

### Chat idempotency
`POST /chat` supports `client_command_id` for idempotency. Keyed by `"<requester>:<id>"`. Pending entries have 60s TTL, completed entries have 1-hour TTL. Concurrent requests with the same key wait on a `Notify` rather than spawning duplicate tasks.

### Tag policy enforcement
Both `POST /chat` and `POST /task` normalize tags via `normalize_tags()` (trim, lowercase, deduplicate) and enforce cert-driven tag policy — if `tags_required` is `true` in the certificate, requests without tags are rejected with 400.

### Startup recovery
`create_app_state()` scans `workspace/tasks/` at startup. Transcripts without `session_end` events get a synthetic `session_end` appended (orphan recovery). Lifetime sats and task counts are tallied from all historical transcripts. TaskInfo entries are reconstructed from transcripts so `GET /task/{id}` works after restart. A canonical `ToolRegistry` is built once and cached as `cached_tool_names` to avoid per-request rebuilds.

### Certificate revocation gate
`create_app_state()` runs `check_authorization()` at boot. If the cert is revoked or missing, `certificate_revoked` is set to `true` and all new tasks are blocked (403). `POST /certificates/issue` clears the flag on successful cert acquisition. `POST /certificates/revoke` sets it. The heartbeat periodically re-checks revocation status.

### Rate limiter initialization
`create_app_state()` reads cert-driven rate limits via `read_cert_rate_limits()` and merges them with config-based limits to construct the shared `RateLimiterRegistry`. This is injected into each runner via `create_loop_with_rate_limiter()`.

### Model capability discovery
`create_app_state()` spawns a background task that fetches x402-info manifests from both LLM providers (OpenAI and Claude). Discovered model capabilities (context_window, max_output_tokens, supports_tools, supports_vision) are cached in `model_capabilities`. In `spawn_task()`, these are applied to each runner via `apply_model_capabilities()`, overriding hardcoded context limits with discovered values. Best-effort — failures are logged at debug level and the runner falls back to hardcoded defaults.

### CORS
Dev mode: allow everything. Production: explicitly allow `x-bsv-auth-*` headers + `content-type`, expose `x-bsv-auth-*` response headers. Methods: GET, POST, OPTIONS.

### Scheduler integration
When heartbeat is enabled, `serve()` spawns the scheduler with a panic-recovery wrapper (restarts after 5s on crash). The scheduler and server share `AppState` via `Arc`. Tasks completed by the server emit `SystemEvent::TaskCompleted` so the scheduler can wake immediately.

## Related

- [`../CLAUDE.md`](../CLAUDE.md) — Parent module inventory and architecture overview
- [`handlers/CLAUDE.md`](handlers/CLAUDE.md) — Handler subdirectory documentation (15 handler modules)
- [`handlers/tasks/CLAUDE.md`](handlers/tasks/CLAUDE.md) — Task handler subdirectory documentation (4 submodules)
- `../auth/` — BRC-31 Authrite client + server implementation (`server.rs` used by `auth.rs`)
- `../runner/` — Agent loop that `spawn_task()` creates and runs
- `../session/` — `ConversationManager` (used by `spawn_task` and conversation handlers), `Transcript` (used by audit/events/history handlers)
- `../onchain/` — `BudgetTracker` (shared via `AppState`), proofs (created in `spawn_task`), basket status (lifecycle endpoint)
- `../heartbeat/` — `Scheduler` spawned by `serve()`, receives system events from task completion
- `../certificates/` — `CertificateManager` used by agent and certificate handlers, policy readers for budget/rate/tag enforcement
- `../mcp/` — `McpClient` for wallet bridge, `mcp_tools_to_tooldefs()` for tool conversion
- `../analytics/` — Efficiency, cost replay, ROI, benchmarks computation (used by analytics handlers)
- `../replay/` — `ReplayViewer` and `ForkExecutor` (used by replay handlers)
- `../metrics.rs` — `MetricsRegistry` for Prometheus metrics
- `../think/` — `ModelCapabilities` type used by `model_capabilities` cache, `apply_model_capabilities()` on runner
- `../x402/circuit_breaker.rs` — `CircuitBreakerRegistry` for LLM provider failover, shared via `AppState.circuit_breakers`
- `../audit/revelation.rs` — `Revelation`, `RevelationLog`, `RevelationsListResponse` for key linkage audit trail
