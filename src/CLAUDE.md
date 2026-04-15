# src/
> Core modules for the autonomous BSV agent — loop, inference, wallet, persistence, HTTP server, and web UI.

## File inventory

Top-level files in `src/`, plus twenty-three domain module directories. Subdirectory modules have their own CLAUDE.md files.

| File / Module | Lines | Purpose |
|---------------|-------|---------|
| `runner/` | ~5034 (8 files) | Agent loop: OBSERVE→THINK→ACT→RECORD→BUDGET. Split into `mod.rs` (types, construction, helpers, MCP tool registration, `apply_model_capabilities()`), `lifecycle.rs` (setup_task, run_loop, teardown_task, run), `step.rs` (per-iteration phases: observe, build, think, record), `execute.rs` (tool execution: parallel dispatch via `tokio::task::JoinSet`, `generate_smart_preview()` for large tool results, tool result offloading >8KB to workspace files), `moderation.rs` (engine construction, `moderate_content()` helper, transcript recording), `approval.rs` (file-based manual approval workflow for high-risk tools, `wait_for_approval()`), `text_extract.rs` (reasoning model fallback for extracting tool calls from text), `escalation.rs` (auto-escalation to human when agent is stuck). **Content moderation**: `moderate_content()` wraps `ModerationEngine` at 4 points (inbox, LLM, tool input, tool output). **Auto-escalation**: detects tool loops, budget threshold, error threshold, explicit uncertainty. **Advisory budget mode**: cert-driven soft spending caps. |
| `server/` | ~8273 (26 files) | Axum 0.8 HTTP server. 69 routes. Split into `mod.rs` (router setup, middleware), `app_state.rs` (AppState, router building, MCP wallet client init, Prometheus metrics, rate limiter, model capability cache, skill telemetry, escalation status), `pdf_template.rs` (HTML template rendering for PDF export), `types.rs` (shared request/response types with tags, time estimates, `ModelInfo` struct), `task_spawner.rs` (unified `spawn_task()` with FIFO queuing, cert revocation gate, MCP tool injection, model capability application), `auth.rs` (BRC-31 server auth helpers), `ui_assets.rs` (embedded UI serving via rust-embed, feature-gated `embed-ui`), and `handlers/` subdirectory with 15 domain-specific handler modules: `agent.rs`, `analytics.rs`, `audit.rs`, `budget.rs`, `chat.rs`, `compliance.rs`, `conversations.rs`, `marketplace.rs`, `memory.rs`, `misc.rs`, `replay.rs`, `schedules.rs`, `staging.rs`, `wallet_ops.rs`, and `tasks/` (subdirectory with 4 files: `lifecycle.rs`, `audit.rs`, `events.rs`, `proofs.rs`). |
| `session/` | ~2193 (4 files) | Session lifecycle. `conversation.rs`: BRC-60 hash-chained multi-turn conversations. `transcript.rs`: append-only JSONL event log. `events.rs`: `StepEvent` enum for SSE streaming. `mod.rs`: re-exports. |
| `onchain/` | ~2627 (4 files) | On-chain state and proofs. `state.rs`: BRC-48 PushDrop state tokens with periodic consistency checks. `budget.rs`: per-service spending tracker with JSONL audit, certificate-derived limit overrides. `proofs.rs`: BRC-18 OP_RETURN proofs including custody proofs. `mod.rs`: re-exports. |
| `config/` | ~1156 (4 files) | TOML config + `WORM_*` env overrides + hot reload. Split into `schema.rs` (config sections + sub-structs, includes `[llm.routing]` and `[llm.fanout]` subsections), `loader.rs` (68+ env overrides), `watcher.rs` (filesystem watcher), `mod.rs` (re-exports + `reload_safe_fields()`). |
| `heartbeat/` | ~2107 (4 files) | Event-driven scheduler. Split into `mod.rs` (Scheduler, PriorityInbox, main run loop), `wake.rs` (WakeReason, WakeRequest, SystemEvent), `sources.rs` (poll_messagebox, scan_continuations, scan_schedules), `features.rs` (active hours, reflection, HEARTBEAT.md checklist, stale task reaping, certificate revocation check). |
| `replay/` | ~576 (3 files) | Interactive replay and fork execution from JSONL transcripts. `viewer.rs`: `ReplayViewer` builds `ReplayTimeline` with per-event cost, iteration, elapsed time, tool usage. `fork.rs`: `ForkExecutor` reconstructs conversation state up to a given event index for branching. |
| `templates/` | ~627 (2 files) | Pre-configured agent archetypes. TOML-based templates override select `WormConfig` fields for specific roles (researcher, coder, trader, etc.). `schema.rs`: `AgentTemplate` struct with 6 sections, validation, config application. `mod.rs`: directory scanner and case-insensitive lookup. |
| `eval/` | ~1541 (4 files) | Agent evaluation framework. `trajectory.rs`: JSONL transcript parser into structured trajectories. `grader.rs`: 4 built-in rubrics (TaskCompletion, Efficiency, Safety, Cost) with weighted criteria. `report.rs`: evaluation reports with baseline regression detection. |
| `think/` | ~1154 (4 files) | Dual-provider LLM inference via BRC-31 auth + x402 payment. Split into `mod.rs` (ThinkResult, routing, circuit breaker, re-exports), `openai.rs` (request construction, response parsing), `claude.rs` (format conversion, request/response), `capabilities.rs` (model limits for 6 families). `resolve_endpoint()` routes by model name or config. `alternate_endpoint()` provides failover. Reasoning model detection via `is_reasoning_model()`. `think_with_tools()` for tool-enabled inference. Refund auto-internalization. **Extended thinking**: `thinking_budget` for Claude models. **Reasoning effort**: `reasoning_effort` for OpenAI o-series/gpt-5. **Text extraction fallback**: post-processing extracts tool calls from reasoning model text output. |
| `analytics/` | ~1175 (5 files) | Efficiency analytics — pure computation from transcript data. Split into `mod.rs` (combined `CostAnalysis` entry point), `efficiency.rs` (`EfficiencyReport` with 30-day trend, per-model breakdown, period-over-period), `cost.rs` (what-if model cost replay), `roi.rs` (ROI reporting with quality scores), `benchmarks.rs` (provider performance tracking via JSONL). |
| `certificates/` | ~1032 (4 files) | BRC-52 agent authorization certificates. Split into `mod.rs` (re-exports, tests), `types.rs` (CertBudgetLimits, CertModerationPolicy, CertToolApproval, CertTagPolicy, CertificateStatus, revocation helpers), `lifecycle.rs` (CertificateManager CRUD, revocation checking with 3-source fallback, boot-time validation), `policy.rs` (5 async policy readers for budget/rate/moderation/approval/tags from cert fields). |
| `audit/` | ~199 (2 files) | BRC-69 key linkage revelations with deterministic hashing and append-only audit logging. `mod.rs` re-exports. `revelation.rs`: `Revelation` struct (counterparty/specific types), `RevelationLog` (JSON file persistence), `compute_revelation_hash()` (SHA-256 tamper detection). |
| `wallet/` | ~3333 (4 files) | BRC-100 wallet interface with trait abstraction. `mod.rs`: `WalletBackend` trait (35 async methods), `WalletClient` type alias. `http.rs`: `HttpWalletClient` — reqwest HTTP client for `bsv-wallet-cli`, all 28 endpoints, retry logic, SPEND_SEMAPHORE. `embedded.rs`: `EmbeddedWalletClient` — in-process `bsv-wallet-toolbox-rs` (feature-gated `embedded-wallet`). `types.rs`: `CreateActionResult`, `ANYONE_KEY`. |
| `moderation.rs` | 299 | Content moderation engine — cert-driven policy for filtering agent inputs and outputs. Separate from `sanitize.rs` (injection defense). `ModerationEngine` provides PII detection (SSN, credit card, email), keyword blocking, and regex pattern matching. Policy resolved from BRC-52 cert fields with fallback to `ModerationConfig`. Cert fields can only tighten policy. Three modes: `Off`, `Flag` (log + continue), `Block` (reject). |
| `metrics.rs` | 139 | Prometheus metrics registry. `MetricsRegistry` holds 7 metrics: `tokens_total` (by model), `request_latency_seconds` (by provider), `budget_spent_sats` (by service), `tasks_total` (by status), `tool_calls_total` (by tool name), `errors_total` (by error type), `active_tasks` gauge. `gather()` produces Prometheus text-format exposition. Exposed at `GET /metrics`. |
| `time_estimate.rs` | 129 | Human-equivalent time-saved estimation for ROI. Two modes: tag-based override (`time-saved:Xh`/`time-saved:Xm` from task tags) and default heuristic (`min(iterations * 5 + tool_calls * 2, 480)` minutes). `aggregate_time_saved()` computes weekly/monthly/total aggregates. Capped at 480 min (8 hours) per task. |
| `discovery.rs` | 353 | BRC-56 Peer Discovery. Wraps wallet's `discoverByIdentityKey` and `discoverByAttributes` endpoints for agent-to-agent service delegation, sender verification, and fleet management. `PeerDiscovery` struct with `PeerInfo` results. |
| `delivery.rs` | 386 | Persistent delivery queue for failed outbound MessageBox messages. `DeliveryQueue` backed by JSON files in `workspace/delivery_queue/`. Exponential backoff: 5s→25s→120s→600s→600s cap. After `MAX_RETRIES` (5) failures, moves to `delivery_queue/failed/` for manual inspection. Scheduler retries due items on each tick. |
| `main.rs` | 651 | CLI entry point. 9 commands: `run`, `status`, `think`, `fund`, `receive`, `serve`, `mcp`, `audit` (BRC-69 key linkage revelations), `verify-work` (offline custody proof verification). Wallet health check on startup. Session summary printed at end. Includes `base58_encode()` for BSV address display. |
| `sanitize.rs` | 516 | 5-layer injection defense + credential leak detection for external messages. **Injection defense**: `sanitize_external_content()` strips control chars + truncates to 2000 chars. `wrap_with_boundary()` adds random-ID boundary markers. `external_tool_allowlist()` returns 10-tool safe set. `detect_injections()` uses Aho-Corasick automaton for O(n) matching against 17 injection patterns. `strip_zero_width()` defeats invisible-character bypass. **Credential leak detection**: `scan_for_leaks()` detects 4 credential patterns. `redact_leaks()` replaces with `[REDACTED:pattern_name]`. |
| `logging.rs` | 422 | Structured logging with `RedactingWriter` that scrubs sensitive data from log output. 6 byte-scanning pattern matchers for API keys, Bearer tokens, BSV auth headers, authrite tokens, base64 blobs (≥200 chars), hex privkeys (64 chars). Redaction toggled via `WORM_LOG_REDACT` env var (default enabled). |
| `error.rs` | 221 | `WormError` enum: 10 variants (Wallet, Payment, Tool, Budget, Config, Loop, Memory, Auth, MessageBox, Conversation). Each carries `ErrorContext` hashmap. `thiserror` derive. `WormResult<T>` alias. |
| `types.rs` | 82 | Newtype ID wrappers: `TaskId`, `SessionId`, `ConversationId`, `ProofTxid`. Macro-generated (`newtype_id!`) with Serde transparent serialization, `Deref` to `&str`, `Display`, `Clone`, `Hash`, `Eq`. Compile-time safety prevents mixing ID types. |
| `cli.rs` | 125 | Clap derive CLI definition. Subcommands: Run, Status, Think, Fund, Receive, Serve, Mcp, Audit (with RevealLinkage/ListRevelations sub-actions), VerifyWork. `AuditArgs` and `AuditAction` types for enterprise compliance CLI. |
| `lib.rs` | 62 | Re-exports all modules organized by domain: core (config, error, replay, runner, think, types), auth & payments (auth, wallet, x402), on-chain (onchain with budget/proofs/state re-exports), session (session with conversation/events/transcript re-exports), analytics (analytics, time_estimate), audit, memory & intelligence (memory, context, skills, templates, loop_detect), communication (messagebox, delivery, certificates, discovery, moderation, sanitize), evaluation (eval), infrastructure (cli, heartbeat, logging, metrics, mcp, server, tools). |

## Module directory layout

```
runner/
  mod.rs          (1334) — Types (WormLoop, LoopState, ContinuationState, ReplyObligation), construction, helpers, register_mcp_tools(), search_tools registration, apply_model_capabilities()
  lifecycle.rs    (759)  — setup_task() (cert budget limits, tool approval, calls build_moderation_engine()), run_loop(), teardown_task(), run() entry point
  step.rs         (986)  — Per-iteration: observe (moderate_content()), build, think (moderate_content() + advisory budget), record
  execute.rs      (856)  — Tool execution: execute_tools (parallel via tokio::task::JoinSet, moderate_content()), execute_tool_standalone(), generate_smart_preview() for large tool results, tool result offloading (>8KB → workspace files with smart previews)
  moderation.rs   (120)  — ModerationOutcome enum, build_moderation_engine(), moderate_content(), record_moderation_event()
  approval.rs     (164)  — Tool approval gate: wait_for_approval() for staged tool calls, file-based manual approval workflow
  text_extract.rs (330)  — extract_tool_calls_from_text() fallback for reasoning models
  escalation.rs   (485)  — EscalationDetector: tool loops, budget threshold, error threshold, explicit uncertainty. EscalationEvent/Resolution/Status types

server/
  mod.rs          (953)  — Router setup, middleware, module declarations, ~900 lines of inline tests
  app_state.rs    (1177) — AppState (with MCP wallet, rate limiter, Prometheus metrics, escalation status, skill telemetry, model_capabilities cache, skill_definitions), create_app_state(), router building, serve()
  pdf_template.rs (469)  — HTML template rendering for PDF export of audit trails and budget reports (XSS-safe)
  types.rs        (394)  — Shared request/response types (with tags, time_saved_minutes, normalize_tags(), ModelInfo struct for available models)
  task_spawner.rs (664)  — Unified spawn_task() with cert revocation gate, per-conversation FIFO queuing, MCP tool injection, rate limiter, Prometheus metrics, skill telemetry merge, model capability application
  auth.rs         (356)  — BRC-31 auth helpers: check_brc31_auth(), signed_json_response(), signed_raw_response()
  ui_assets.rs    (47)   — Embedded UI assets via rust-embed (feature-gated embed-ui), SPA fallback
  handlers/
    mod.rs        (17)   — Re-exports 15 handler modules
    agent.rs      (737)  — /health, /agent (with available_models + default_model), /certificates, /certificates/{issue,revoke,relinquish}, /analytics/skills
    analytics.rs  (215)  — /analytics/{efficiency,cost-comparison,roi,benchmarks}. Cost comparison with 9-model pricing table + prefix-match lookup for versioned model names
    audit.rs      (618)  — BRC-69 key linkage revelation, cross-agent proof cross-referencing, full-text audit search, revelation listing
    budget.rs     (414)  — /budget, /budget/detail, /budget/export, /rates/bsv-usd (multi-source with CoinGecko fallback)
    chat.rs       (459)  — POST /chat (with idempotency + tag policy), /chat/history, /v1/chat/completions
    compliance.rs (277)  — /lifecycle/status (UTXO basket counts), /compliance/report (date-range filtering)
    conversations.rs (254) — /conversations, /conversations/{id}, verify, sync, compact. Self-healing: reconstructs missing assistant messages from task transcripts
    marketplace.rs (196) — Plugin marketplace CRUD: /marketplace/plugins (list, get, create, delete)
    memory.rs     (188)  — /memory, /memory/search, /memory/{id}
    misc.rs       (87)   — /message, /heartbeat/trigger
    replay.rs     (200)  — /task/{id}/replay (structured timeline), /task/{id}/fork (fork execution)
    schedules.rs  (89)   — /schedules, /schedules/{id}
    staging.rs    (291)  — /staged (list, approve, abort) — wallet signing + tool approval modes
    wallet_ops.rs (171)  — /output/{basket}/{txid} (UTXO + PushDrop), /decrypt
    tasks/
      mod.rs      (11)   — Re-exports 4 task handler submodules
      lifecycle.rs (568) — /task, /tasks, /status, /task/{id}, cancel, time-saved, file serving
      audit.rs    (615)  — /task/{id}/audit, /task/{id}/audit/export, transcript/verify, html_to_pdf()
      events.rs   (342)  — /task/{id}/events, escalation status/resolve/proof
      proofs.rs   (588)  — /task/{id}/proofs, verify, custody, receipts

session/
  mod.rs          (5)    — Re-exports
  conversation.rs (1313) — ConversationManager, BRC-60 hash chains, wallet sync, compaction
  transcript.rs   (608)  — Append-only JSONL event log, message reconstruction, analytics
  events.rs       (267)  — StepEvent enum for SSE streaming, truncation, formatting

onchain/
  mod.rs          (5)    — Re-exports
  budget.rs       (775)  — BudgetTracker, JSONL audit trail, six-tier limits, cert-derived limit overrides, merge support
  proofs.rs       (804)  — BRC-18 OP_RETURN proofs (Decision, TaskCompletion, Custody, BRC-18 delivery proofs, etc.), ProofCommitment with hash chaining
  state.rs        (1043) — BRC-48 PushDrop state tokens (TaskCommitment, BudgetAllocation, etc.), BASKET_REVOCATION, periodic consistency check against LoopState, compute_state_root()

config/
  mod.rs          (83)   — Re-exports all 23+ types + reload_safe_fields() impl on WormConfig
  schema.rs       (599)  — Config sections + sub-structs with serde derives and Default impls (includes [llm.routing] and [llm.fanout] subsections)
  loader.rs       (399)  — load_config() + apply_env() with 68+ env overrides
  watcher.rs      (75)   — ConfigWatcher — notify crate fs watcher with 500ms debounce

heartbeat/
  mod.rs          (998)  — Scheduler, PriorityInbox, TaskPriority, InboxItem, main run() loop, delivery retry, config hot-reload, liveness tick, certificate revocation check
  wake.rs         (96)   — WakeReason (6 variants), WakeRequest, SystemEvent (5 variants), HEARTBEAT_CONVERSATION_ID
  sources.rs      (332)  — poll_messagebox(), scan_continuations(), scan_schedules()
  features.rs     (681)  — is_within_active_hours(), read_heartbeat_checklist(), should_respond_to_message(), scan_reflection(), reap_stale_tasks(), check_certificate_revocation()

replay/
  mod.rs          (14)   — Re-exports ReplayViewer, ReplayEvent, ReplayTimeline, ForkExecutor, ForkParams, ForkResult
  viewer.rs       (344)  — ReplayViewer: build_timeline(), cost timeline, tool usage pairing, per-event metadata
  fork.rs         (218)  — ForkExecutor: prepare_fork(), message reconstruction up to event index

templates/
  mod.rs          (232)  — load_builtin_templates(), find_template() case-insensitive lookup, 7 tests
  schema.rs       (395)  — AgentTemplate with 6 TOML sections, load/validate/apply_to_config, 9 tests

eval/
  mod.rs          (17)   — Re-exports: Trajectory, Rubric, Score, EvalReport, RegressionFlag
  trajectory.rs   (536)  — JSONL parser → Trajectory with metadata (tokens, cost, rounds, tools, duration)
  grader.rs       (645)  — 4 built-in rubrics × 3 criteria each (TaskCompletion, Efficiency, Safety, Cost)
  report.rs       (343)  — EvalReport with baseline regression detection (default 10% threshold)

think/
  mod.rs          (470)  — ThinkResult, routing (resolve_endpoint, alternate_endpoint), think(), think_with_tools(), think_with_circuit_breaker(), model detection, shared error/refund helpers, re-exports
  openai.rs       (202)  — build_openai_body() request construction, parse_openai_response(), extract_payment_info() shared payment/refund extraction
  claude.rs       (396)  — build_claude_body(), parse_claude_response(), convert_messages_for_claude(), convert_tools_for_claude(), flush_tool_results()
  capabilities.rs (86)   — ModelCapabilities struct, model_input_limit() and model_output_limit() hardcoded fallbacks for 6 model families

analytics/
  mod.rs          (66)   — Re-exports, CostAnalysis combined report struct, compute_cost_analysis() entry point
  efficiency.rs   (456)  — compute_efficiency(), EfficiencyReport (30-day trend, per-model breakdown, week-over-week comparison)
  cost.rs         (185)  — cost_replay() what-if model comparison, CostComparison, ModelPricing
  roi.rs          (226)  — compute_roi() spending vs outcomes, RoiReport with quality scores from eval.json
  benchmarks.rs   (242)  — Provider performance tracking, BenchmarkEntry JSONL persistence, aggregate_benchmarks(), backfill from transcripts

certificates/
  mod.rs          (117)  — Re-exports all public types and functions, unit tests for CertificateStatus serde
  types.rs        (135)  — CertBudgetLimits, CertModerationPolicy, CertToolApproval, CertTagPolicy, CertificateStatus, CertCheckResult, revocation outpoint helpers
  lifecycle.rs    (585)  — CertificateManager CRUD (acquire, relinquish, prove, list), revocation checking (is_revoked with paginated UTXO scan, find_revocation_outpoint 3-source fallback), check_authorization() boot-time validation
  policy.rs       (195)  — 5 async policy readers: read_cert_budget_limits(), read_cert_rate_limits(), read_cert_moderation_policy(), read_cert_tool_approval(), read_cert_tag_policy()

audit/
  mod.rs          (3)    — Module declaration, re-exports revelation submodule
  revelation.rs   (196)  — Revelation struct (counterparty/specific types with UUID, hash, timestamp), RevelationLog append-only JSON file persistence, compute_revelation_hash() SHA-256, RevelationsListResponse

wallet/
  mod.rs          (278)  — WalletBackend trait (35 async methods), re-exports, WalletClient type alias for HttpWalletClient
  http.rs         (1420) — HttpWalletClient: reqwest HTTP client, SPEND_SEMAPHORE, retry logic, balance pagination, WOC funding, BEEF parsing helpers
  embedded.rs     (1621) — EmbeddedWalletClient: in-process bsv-wallet-toolbox-rs wrapper (feature-gated embedded-wallet), dual bsv-rs strategy
  types.rs        (14)   — CreateActionResult struct, ANYONE_KEY constant (secp256k1 generator point G)
```

## HTTP routes (server/)

69 routes across 15 handler modules (+ `tasks/` subdirectory with 4 submodules). BRC-31 Authrite auth required on `/chat`, `/budget`, `/chat/history`, `/heartbeat/trigger`, `/v1/chat/completions`. Other endpoints enforce BRC-31 when `ParentConfig.identity_key` is set (dev mode skips auth).

| Method | Path | Auth | Handler | Description |
|--------|------|------|---------|-------------|
| GET | /health | none | agent | Liveness check (version, uptime, scheduler tick age, wallet connectivity) |
| POST | /.well-known/auth | none | auth | BRC-31 Authrite handshake |
| POST | /task | optional | tasks | Submit task with optional tags, returns 202 |
| GET | /status | optional | tasks | All tasks + active count + total sats |
| GET | /task/{id} | optional | tasks | Single task status |
| GET | /tasks | optional | tasks | All tasks (memory + disk), supports `?tag=X,Y` filter |
| GET | /task/{id}/audit | optional | tasks | Full transcript audit chain + summary |
| GET | /task/{id}/audit/export | optional | tasks | Downloadable audit report (CSV, signed JSON, or PDF) |
| GET | /task/{id}/proofs | optional | tasks | Proof details with hash, type, iteration |
| GET | /task/{id}/proofs/verify | optional | tasks | Recompute hashes + check on-chain |
| GET | /task/{id}/proofs/custody | optional | tasks | Custody proof for billing verification |
| GET | /task/{id}/receipts | optional | tasks | BEEF payment receipts |
| GET | /task/{id}/conversation | optional | tasks | Reconstructed conversation messages |
| GET | /task/{id}/events | optional | tasks | Incremental events since offset (poll-based UI) |
| GET | /task/{id}/transcript/verify | optional | tasks | Verify transcript HMAC integrity via wallet |
| GET | /task/{id}/replay | optional | replay | Structured replay data (events, cost timeline, tool usage) |
| POST | /task/{id}/fork | optional | replay | Fork execution from event index into new task |
| POST | /task/{id}/cancel | optional | tasks | Cancel a running task |
| GET | /task/{id}/escalation | optional | tasks | Get escalation status for a task |
| GET | /task/{id}/escalation/proof | optional | tasks | Escalation BRC-18 proofs for a task |
| POST | /task/{id}/escalation/resolve | optional | tasks | Resolve escalation with human guidance |
| GET | /agent | optional | agent | Identity, balance, tools, cert status |
| POST | /message | optional | misc | Forward MessageBox message as task |
| GET | /files/{task_id}/{filename} | none | tasks | Serve task workspace files (path traversal protected) |
| GET | /output/{basket}/{txid} | none | wallet_ops | Fetch UTXO + parse PushDrop fields |
| POST | /decrypt | optional | wallet_ops | Decrypt ciphertext via wallet |
| GET | /certificates | optional | agent | Certificate status |
| POST | /certificates/issue | optional | agent | Acquire parent-signed cert (with budget limits) |
| POST | /certificates/revoke | optional | agent | Revoke agent cert by spending revocation UTXO |
| POST | /certificates/relinquish | optional | agent | Relinquish self-signed cert (parent-issued blocked, 403) |
| GET | /conversations | optional | conversations | List all conversations |
| GET | /conversations/{id} | optional | conversations | Metadata + messages (optional verify) |
| GET | /conversations/{id}/verify | optional | conversations | BRC-60 hash chain integrity check |
| POST | /conversations/{id}/sync | optional | conversations | Encrypt + store on-chain via wallet |
| POST | /conversations/{id}/compact | optional | conversations | LLM-summarize old messages |
| GET | /memory | optional | memory | List with category filter + pagination |
| GET | /memory/search | optional | memory | BM25 search |
| GET | /memory/{id} | optional | memory | Single memory entry |
| GET | /budget/detail | optional | budget | Spending breakdown by service/operation + rate limit status |
| GET | /budget/export | optional | budget | Downloadable budget report (JSON or PDF) |
| GET | /rates/bsv-usd | none | budget | Multi-source BSV/USD rate (WhatsOnChain + CoinGecko fallback) |
| GET | /metrics | optional | app_state | Prometheus metrics endpoint |
| GET | /analytics/skills | optional | agent | Skill usage telemetry |
| GET | /analytics/efficiency | optional | analytics | Efficiency metrics from transcript data |
| GET | /analytics/cost-comparison | optional | analytics | Replay task with alternative model pricing |
| GET | /analytics/roi | optional | analytics | ROI report for current session |
| GET | /analytics/benchmarks | optional | analytics | Provider benchmark summary |
| GET | /analytics/time-saved | optional | tasks | Aggregate human-equivalent time saved |
| GET | /schedules | optional | schedules | List recurring schedules |
| GET | /schedules/{id} | optional | schedules | Single schedule detail |
| GET | /lifecycle/status | none | compliance | UTXO lifecycle — basket counts, oldest ages, sweep config |
| GET | /compliance/report | optional | compliance | Compliance status with date-range filtering |
| GET | /audit/search | none | audit | Full-text search across all task transcripts |
| POST | /audit/key-linkage/counterparty | BRC-31 | audit | BRC-69 counterparty key linkage revelation |
| POST | /audit/key-linkage/specific | BRC-31 | audit | BRC-70 specific key linkage revelation |
| GET | /audit/revelations | optional | audit | List all past key linkage revelations |
| GET | /audit/cross-reference/{hash} | optional | audit | Find proofs referencing a message hash |
| GET | /staged | optional | staging | List staged high-value transactions |
| POST | /staged/{ref}/approve | optional | staging | Approve staged transaction (wallet signing or tool approval) |
| POST | /staged/{ref}/abort | optional | staging | Abort staged transaction |
| GET | /marketplace/plugins | optional | marketplace | List all plugin listings |
| POST | /marketplace/plugins | optional | marketplace | Create or update a plugin listing |
| GET | /marketplace/plugins/{name} | optional | marketplace | Get a specific plugin |
| DELETE | /marketplace/plugins/{name} | optional | marketplace | Delete a plugin listing |
| POST | /heartbeat/trigger | BRC-31 | misc | Manual scheduler task injection |
| POST | /chat | BRC-31 | chat | Submit message with optional tags, returns `{ task_id, session_id }` |
| GET | /chat/history | BRC-31 | chat | Conversation history from transcript |
| GET | /budget | BRC-31 | budget | Budget report |
| POST | /v1/chat/completions | BRC-31 | chat | OpenAI-compatible (conditional on config) |

Static frontend served at `GET /ui/*`.

"optional" auth = BRC-31 enforced when `config.parent.identity_key` is set; skipped in dev mode. Response signing via `signed_json_response()` or `signed_raw_response()` wraps authenticated responses with BRC-31 signatures.

## Dual-provider LLM (think/)

`think/` (4 files, ~1154 lines) is the normalization boundary — the rest of the codebase sees a uniform `ThinkResult` response format regardless of provider.

1. **Routing** (mod.rs): `resolve_endpoint()` picks OpenAI or Claude endpoint based on `config.llm.default_provider` or model name (`claude-*` → Claude, everything else → OpenAI). `alternate_endpoint()` provides the opposite provider for failover.
2. **OpenAI path** (openai.rs): Standard chat completions format. `build_openai_body()` constructs the request. Reasoning models get `max_completion_tokens` instead of `max_tokens`, plus optional `reasoning_effort` field (e.g., "high", "low") for o-series and gpt-5 models. `parse_openai_response()` extracts response and `extract_payment_info()` handles payment/refund data.
3. **Claude path** (claude.rs): `convert_messages_for_claude()` extracts system messages, converts `tool`-role messages to `tool_result` content blocks, and converts assistant `tool_calls` to `tool_use` blocks. `flush_tool_results()` groups consecutive tool-role messages into a single user message with multiple `tool_result` content blocks. `convert_tools_for_claude()` adapts tool definitions. `build_claude_body()` constructs the request with optional `thinking_budget` for extended thinking (auto-adjusts `max_tokens` to satisfy the 1024-token buffer requirement). `parse_claude_response()` normalizes tool_use blocks back to OpenAI-format tool_calls.
4. **Payment**: Both providers use the same `authenticated_paid_request()` flow from `x402/payment.rs`. Refund auto-internalization is best-effort. Error responses also attempt refund recovery via `try_internalize_refund_from_error()`.
5. **Circuit breaker integration** (mod.rs): `ThinkRequest` struct carries model, messages, tools, config, thinking_budget, reasoning_effort. `think_with_circuit_breaker()` picks primary or alternate provider based on circuit state, records success/failure per provider.
6. **Text extraction fallback** (mod.rs): `think_with_tools()` applies post-processing when tools are provided but no structured tool_calls returned — uses `extract_tool_calls_from_text()` from `runner/text_extract.rs`. Sets `was_text_extracted = true` on result.
7. **Model capabilities** (capabilities.rs): `ModelCapabilities` struct with hardcoded fallbacks for 6 model families (gpt-5, o3/o4, claude-haiku/sonnet/opus, unknown). `model_input_limit()` and `model_output_limit()` provide per-family context/output limits.

## Event streaming architecture

The web UI receives real-time updates via transcript polling:

1. **Runner emits events**: `step()` (in `runner/step.rs`) sends `StepEvent` variants through an optional `broadcast::Sender<(String, StepEvent)>` paired with the task ID. Events are also recorded in the JSONL transcript.
2. **Transcript polling**: `TranscriptPoller` in the UI polls `/task/{id}/events` for transcript events since a given index. This is the primary streaming mechanism.
3. **Chat submission**: `POST /chat` returns JSON `{ task_id, session_id }`. The UI then starts polling the returned `task_id` for events.
4. **Tool output is truncated for events**: `truncate_for_sse()` caps tool call output at 2000 chars for `ToolCallComplete` events. Response and Done events send full text (no truncation). Full output always stays in the JSONL transcript.
5. **Unified spawn path**: `spawn_task()` (in `server/task_spawner.rs`) is the single entry point for all task creation. All tasks use `run()` with event streaming, get conversation tracking, and are registered in `task_sessions`. Per-conversation semaphore FIFO queuing prevents concurrent tasks on the same conversation.
6. **Session tracking**: `spawn_task()` records `task_id → session_id` in `AppState.task_sessions`. The `/task/{id}/events` endpoint includes `session_id` in its response for session recovery.

## Certificate management (certificates/)

BRC-52 agent authorization for operator→agent delegation (4 files, ~1032 lines), with revocation, 6-tier budget limits, moderation policy, and rate limits:

1. **Boot-time check**: `ensure_authorization()` runs during `create_app_state()`. Attempts parent-signed cert first, falls back to self-signed. Non-fatal — agent starts regardless. Validates revocation status at boot.
2. **Parent-signed flow**: Agent sends CSR to parent wallet → parent signs with protocol `[2, "certificate signing"]` → agent acquires the signed certificate. Fields: name, capabilities, deployed_at, version, 6 budget tiers (`budget_per_task`/`budget_per_hour`/`budget_per_day`/`budget_per_week`/`budget_per_month`/`budget_lifetime`), `budget_enforcement` (strict/advisory), moderation policy, rate limits, tag policy, approval tools. Unique serial numbers include timestamp.
3. **Self-signed fallback**: If parent wallet is unreachable, agent self-signs with counterparty `"self"`. `relinquish_self_signed()` cleans up once a parent-signed cert is acquired.
4. **Budget limits in certificates**: `read_cert_budget_limits()` reads all 6 budget tiers + enforcement mode. At task start, `runner/lifecycle.rs` calls `budget_tracker.apply_cert_limits()` to override config-based limits.
5. **Moderation policy**: `CertModerationPolicy` struct reads `moderation_enabled`, `moderation_pii_mode`, `moderation_profanity_mode` from cert fields. Used by `ModerationEngine` to tighten content filtering.
6. **Rate limits**: `read_cert_rate_limits()` reads `rate_limit_default_rpm` and per-service overrides from cert fields. Merged into the shared `RateLimiterRegistry`.
7. **Selective reveal**: `prove_authorization()` proves specific fields without exposing the full certificate (BRC-100 prove endpoint).
8. **Revocation with fallback discovery**: `find_revocation_outpoint()` tries three sources: (1) the cert's `revocationOutpoint` field (fast path), (2) scanning `worm-revocation` basket for matching serial number, (3) scanning `worm-state` basket (legacy). `is_revoked()` paginates through revocation basket outputs (1000 per page) to check UTXO existence.
9. **Periodic revocation check**: The heartbeat scheduler runs `check_certificate_revocation()` every 10 minutes, logging errors if a parent-signed cert has been revoked.
10. **Relinquish guard**: `relinquish_certificate` refuses to relinquish parent-issued certificates (certifier != subject), returning 403. Parent-issued certs can only be revoked.
11. **Server integration**: Four routes (`/certificates`, `/certificates/issue`, `/certificates/revoke`, `/certificates/relinquish`). `AgentResponse` includes `certificate_status`. Issuance pre-relinquishes existing certs to avoid UNIQUE constraint errors.

## Multi-turn conversations (session/conversation.rs + server/ + runner/)

Conversations span multiple tasks and are linked by BRC-60 hash chains:

1. **ConversationManager** (session/conversation.rs): Manages `workspace/conversations/{conv_id}/` directories. Each has `meta.json` (Conversation struct) and `messages.jsonl` (append-only ConversationMessage records). Genesis hash: `SHA-256("CONVERSATION" || conv_id)`.
2. **Hash chain**: Each message hash = `SHA-256(prev_hash || role || content || ts)`. `verify_chain()` recomputes all hashes and reports breaks. Provides integrity verification without on-chain storage for every message.
3. **Chat flow** (server/handlers/chat.rs + server/task_spawner.rs): `ChatRequest.session_id` resumes an existing conversation or creates a new one. Per-conversation semaphores (1 permit each) provide FIFO queuing — tasks targeting the same conversation wait instead of being rejected. After task completion, `append_from_transcript()` extracts assistant/tool messages (with `compact_content()` applied) into the conversation, creates a ConversationIntegrity proof, and auto-syncs to wallet.
4. **Context injection** (runner/mod.rs): `WormLoop.prior_messages` field holds previous conversation turns. `set_prior_messages()` sets it; `step()` prepends these before each LLM call. Server calls `conv_mgr.to_openai_messages()` to convert stored messages to OpenAI API format.
5. **Compaction**: `set_compaction_summary()` stores an LLM-generated summary with a sequence cutoff. `compact_content()` strips data URIs from messages to control context size. Triggered via `POST /conversations/{id}/compact`.
6. **System conversations**: `create_with_id()` creates conversations with deterministic IDs (e.g., `conv-heartbeat` for heartbeat tasks).
7. **Wallet sync**: `sync_to_wallet()` encrypts conversation data with protocol `[2, "worm conversation"]` and stores it via the wallet. Basket: `worm-conversations`.

## Scheduler (heartbeat/)

The `Scheduler` is a priority-based, event-driven task scheduler with autonomous capabilities, split across 4 files:

1. **PriorityInbox**: Sorted by `TaskPriority` (Highest > Normal > Low > Lowest), then FIFO within the same priority. Parent identity key messages get Highest priority.
2. **Three scan sources** (sources.rs): MessageBox inboxes (Normal), continuations directory (Low), schedules directory (Normal). Each scan pushes `InboxItem` structs into the priority queue.
3. **Event-driven wake** (wake.rs): `tokio::select!` with 250ms coalescing window. `WakeReason` enum: MessageReceived, ContinuationReady, ScheduleDue, HeartbeatFile, ExternalTrigger, TimerExpired. `WakeRequest` has priority ordering (retry=0 < interval=1 < default=2 < action=3). System events and timers both feed the wake system.
4. **System events** (wake.rs): `SystemEvent` enum with 5 variants — TaskCompleted, ScheduleFired, MessageReceived, ConfigChanged, ContinuationReady. Emitted by the agent loop and API handlers via `system_event_tx` mpsc channel on AppState. Each event triggers an immediate scheduler wake (after coalescing).
5. **Concurrency limiter**: `active_task_count` on AppState tracks running tasks. `has_capacity()` checks against `max_concurrent_tasks` (default 3). Tasks that exceed capacity stay in the inbox.
6. **Active hours** (features.rs): `is_within_active_hours()` checks `ActiveHours` config (IANA timezone, HH:MM start/end). Ticks are skipped outside the configured window.
7. **Reflection tasks** (features.rs): Autonomous reflection on configurable interval (`reflection_interval_secs`, default 60s) with a separate model (`reflection_model`, default `claude-haiku-4-5-20251001`). Capped at `reflection_max_iterations` (default 3).
8. **Checklist handling** (features.rs): Reads `HEARTBEAT.md` (configurable via `checklist_file`), deduplicates items with content hash + cooldown (`checklist_cooldown_secs`, default 600s).
9. **Schedule scanning** (sources.rs): Reads `workspace/schedules/*.json`, checks `next_run` timestamps, updates `last_run` and computes next occurrence on execution.
10. **Hot-reload integration**: Listens on `config_rx` watch channel for config changes. `reload_safe_fields()` applies runtime-safe updates (heartbeat, budget, LLM params) without restarting.
11. **Delivery retry**: Scans `DeliveryQueue` for due items on each tick and retries failed outbound messages.
12. **Certificate revocation check** (features.rs): Every 10 minutes (`REVOCATION_CHECK_INTERVAL_SECS = 600`), checks if parent-signed certificate's revocation UTXO has been spent. Logs error if revoked but does not force-quit.
13. **Stale task reaping** (features.rs): Tasks stuck as `Running` with 0 iterations for >30 minutes are marked `Error` and have their `active_task_count` decremented.
14. **Panic recovery**: Scheduler spawned with infinite restart loop and 5s backoff on panic. Manual injection via `POST /heartbeat/trigger`.

## Configuration (config/)

TOML config + `WORM_*` env overrides, split across 4 files:

1. **Schema** (schema.rs, 599 lines): 17 config sections + 5 sub-structs — WalletConfig, BudgetConfig (6 tiers + enforcement + staging), LlmConfig (routing, fanout, caching, compaction, `thinking_budget`, `reasoning_effort`), LoggingConfig, MemoryConfig (maintenance), HeartbeatConfig (active hours, reflection, checklist), ParentConfig, McpConfig, ServerConfig, CertificateConfig, BrowserConfig, LifecycleConfig (UTXO sweep), ComplianceConfig, X402Config (rate limits), RatesConfig (BSV/USD margin/staleness), ModerationConfig (PII, profanity, custom patterns), ToolApprovalConfig. All have `#[serde(default)]` with sensible defaults. LlmConfig includes `[llm.routing]` (simple/complex model routing thresholds) and `[llm.fanout]` (parallel multi-provider queries) subsections.
2. **Loading** (loader.rs, 399 lines): `load_config()` reads `worm.toml` (or defaults if missing) then applies 68 `WORM_*` env overrides via `apply_env()`. Parse errors return `WormError::Config`.
3. **Hot reload** (mod.rs + watcher.rs): `ConfigWatcher` uses `notify` crate to watch `worm.toml` parent directory. Debounces 500ms. Broadcasts reloaded config via `tokio::sync::watch`. `reload_safe_fields()` selectively updates heartbeat, budget, LLM, browser, lifecycle, compliance, memory maintenance, rates, and moderation config — wallet URL, parent identity key, logging, MCP, server, certificate, tool approval, and x402 config are NEVER hot-reloaded.
4. **Key defaults**: Default model `gpt-5-mini`, max_tokens 16384, context_window 128000, budget per-task 20M sats (6 tiers: task/hour/day/week/month/lifetime), wallet at `localhost:3322`. `ParentConfig.identity_key` empty = dev mode (auth bypass). Budget enforcement default `strict` (also supports `advisory`). `thinking_budget` and `reasoning_effort` default to `None` (disabled).

## Self-continuation (runner/ + heartbeat/ + tools/sandbox.rs)

The agent can pause tasks and schedule resumption:

1. **`continue_task` tool** (sandbox.rs): Saves a `ContinuationState` JSON to `workspace/continuations/<id>.json` with task description, transcript path, iteration count, reason, and optional `wake_at` timestamp.
2. **Heartbeat scanning** (heartbeat/sources.rs): `scan_continuations()` reads JSON files from `workspace/continuations/`, parses `wake_at` via `chrono::DateTime::parse_from_rfc3339()`, and submits ready tasks as `CONTINUATION:<id>` prefixed descriptions via the priority inbox. Moves completed files to `completed/` subdirectory.
3. **Server detection** (server/task_spawner.rs): `spawn_task()` detects the `CONTINUATION:` prefix and routes to the continuation resume path.
4. **Protected paths** (sandbox.rs): `is_protected_path()` blocks writes to `worm.toml`, `Cargo.toml`, `Cargo.lock`, and patterns like `.ssh`, `.gnupg`, `.aws`, `/etc/`, `.env`. Additionally, `FORBIDDEN_PATTERNS` and `is_forbidden_command()` block dangerous shell commands in `execute_bash`. Bash defaults to workspace directory as cwd.

## Delivery queue (delivery.rs + heartbeat/)

Persistent retry for failed outbound MessageBox messages:

1. **Enqueue on failure**: When `spawn_task()` reply routing fails to send a message, the delivery is persisted as a JSON file in `workspace/delivery_queue/`.
2. **Exponential backoff**: 5s → 25s → 120s → 600s → 600s (capped). `next_retry_at` is recalculated on each failure.
3. **Max retries**: After 5 failures, the delivery is moved to `delivery_queue/failed/` for manual inspection.
4. **Scheduler integration**: `due_items()` returns deliveries whose `next_retry_at` has elapsed. Scheduler retries these on each tick.
5. **Crash recovery**: `scan_on_startup()` counts pending deliveries; no state is lost on restart since files are the source of truth.

## Memory auto-recall (runner/ + memory/)

BM25+vector hybrid memory retrieval injected into every agent iteration:

1. **Query construction**: Uses task description + last assistant response text (via `transcript.last_think_response_text()`) as search query.
2. **Hybrid search**: BM25 keyword search + vector cosine similarity (via x402 embeddings), combined with MMR diversity and recency weighting. Results ranked by hybrid score.
3. **Prompt injection**: Recalled memories injected into `PromptContext.recalled_section` as part of the system prompt. Separate knowledge and session subsections.
4. **ID tracking**: `auto_recall_ids` tracked per iteration on `LoopState` and included in BRC-18 Decision proofs for auditability.
5. **Static deduplication**: `get_static_memory_ids()` deduplicates auto-recalled entries against the static identity/soul summary already in the prompt.

## Identity bootstrap (runner/ + certificates.rs)

At task start, `run()` seeds an identity memory entry if none exists:

1. **Certificate fetch**: `fetch_certificate_info()` queries `CertificateManager` for agent name, capabilities, and certificate status (parent-signed, self-signed, or none).
2. **Identity entry**: Creates a `knowledge/identity` memory entry from certificate metadata. Only runs once — subsequent tasks find the existing entry.
3. **Soul context**: Certificate info is included in `PromptContext` for the system prompt's identity section.

## BRC-31 server auth (auth/server.rs + server/auth.rs)

Server-side authentication split across two locations:

- **`auth/server.rs`** — Core BRC-31 session management: `Brc31SessionStore` (in-memory, 1-hour TTL, auto-cleanup), `handle_handshake()` (32-byte random nonce), `extract_auth_params()` (7 `x-bsv-auth-*` headers), `verify_request()` (BRC-104 decode + wallet signature verification), `sign_handshake_response()` / `sign_response_payload()` (SDK-based response signing).
- **`server/auth.rs`** — Route-level helpers: `check_brc31_auth()` (middleware-like; returns `Ok(())` in dev mode, otherwise verifies headers + session + parent key match), `signed_json_response()` (wraps authenticated responses with BRC-31 signatures), `signed_raw_response()` (signs non-JSON responses like CSV/JSON exports with auth headers).

## MCP wallet bridge (server/ + runner/)

At startup, `create_app_state()` attempts to connect to a `bsv-wallet-toolbox` MCP server. If found, it discovers available tools and caches them as `wallet_mcp_tools` on AppState. In `spawn_task()`, these tools are converted to `ToolDef` structs and registered into each task's `ToolRegistry`. In the runner, `register_mcp_tools()` handles post-construction registration and rebuilds the `search_tools` snapshot. Tools named `wallet_balance` are excluded (already provided natively). This bridges typed wallet MCP tools into the agent's tool system without code changes to the runner.

## Content moderation (moderation.rs + runner/)

Cert-driven content moderation separate from injection defense (`sanitize.rs`):

1. **ModerationEngine**: Built once per task in `setup_task()` from `CertModerationPolicy` + `ModerationConfig`. Cert fields can only tighten policy (never relax). Three modes per category: `Off`, `Flag` (log + continue), `Block` (reject).
2. **Built-in PII patterns**: SSN (3-2-4 format), credit card (4×4 digit groups), email addresses. Compiled as regexes at engine construction.
3. **Custom patterns**: `custom_block_patterns` and `custom_flag_patterns` (regexes), `custom_block_keywords` (case-insensitive substring match).
4. **Four moderation points**: Inbox messages (observe), LLM response text (think_step), tool inputs (pre-execution), tool outputs (post-execution). Blocked content is rejected or replaced; flagged content is logged.

## Analytics (analytics/ + time_estimate.rs + server/handlers/analytics.rs)

Efficiency tracking and ROI reporting:

1. **Efficiency analytics** (analytics/, 5 files, ~1175 lines): Split into `efficiency.rs` (EfficiencyReport with 30-day trend, per-model breakdown, week-over-week), `cost.rs` (what-if model comparison), `roi.rs` (spending vs outcomes with quality scores), `benchmarks.rs` (provider performance tracking via JSONL). `compute_cost_analysis()` in `mod.rs` orchestrates all three sub-modules into a single `CostAnalysis` result.
2. **Time-saved estimation** (time_estimate.rs, 129 lines): Two modes — tag-based override (`time-saved:Xh`/`time-saved:Xm`) and default heuristic (`min(iterations * 5 + tool_calls * 2, 480)`). `aggregate_time_saved()` computes weekly/monthly/total aggregates.
3. **API routes**: 6 analytics endpoints (`/analytics/efficiency`, `/analytics/cost-comparison`, `/analytics/roi`, `/analytics/benchmarks`, `/analytics/time-saved`, `/analytics/skills`).

## Prometheus metrics (metrics.rs + server/)

Observable agent behavior via standard Prometheus metrics:

1. **MetricsRegistry** (metrics.rs, 139 lines): 7 metrics — `worm_tokens_total` (IntCounterVec by model), `worm_request_latency_seconds` (HistogramVec by provider), `worm_budget_spent_sats` (CounterVec by service), `worm_tasks_total` (IntCounterVec by status), `worm_tool_calls_total` (IntCounterVec by tool_name), `worm_errors_total` (IntCounterVec by error_type), `worm_active_tasks` (IntGauge).
2. **Recording**: Runner records tokens, latency, and spend per LLM call; tool call counts per execution; task completion in `spawn_task()`.
3. **Exposition**: `GET /metrics` serves Prometheus text-format output via `gather()`.

## Replay and fork (replay/ + server/handlers/replay.rs)

Interactive replay and fork execution from completed task transcripts:

1. **ReplayViewer** (viewer.rs): Builds `ReplayTimeline` from a transcript with per-event metadata (cumulative cost, iteration, elapsed time, tool name, model), cost timeline points, and tool usage entries (call→result pairing).
2. **ForkExecutor** (fork.rs): Reconstructs OpenAI-format conversation messages from a transcript up to a given event index. Produces `ForkResult` with `prior_messages` for injection into a new `WormLoop`.
3. **API routes**: `GET /task/{id}/replay` returns structured timeline; `POST /task/{id}/fork` spawns a new task branching from a given event index. Fork metadata written to `tasks/{new_id}/fork_context.json`.

## Agent templates (templates/)

Pre-configured agent archetypes for common use cases:

1. **TOML format**: 6 sections — `[template]` (name, description), `[personality]` (system_prompt, style), `[skills]`, `[tools]` (enabled/disabled lists), `[budget]`, `[defaults]` (model, temperature, max_tokens).
2. **10 built-in templates**: researcher, coder, analyst, trader, content-creator, customer-support, data-pipeline, security-auditor, devops, personal-assistant.
3. **Config application**: `apply_to_config()` overrides only non-zero/non-empty fields in `WormConfig`. Validation checks name/description required, temperature range, no tool in both enabled+disabled.
4. **Lookup**: `load_builtin_templates()` scans directory, `find_template()` provides case-insensitive search.

## Evaluation framework (eval/)

Agent evaluation from recorded JSONL session transcripts:

1. **Trajectory parsing** (trajectory.rs): `Trajectory::from_jsonl()` parses transcripts into structured steps with metadata (tokens, cost, rounds, tools, duration, error count).
2. **Grading rubrics** (grader.rs): 4 built-in rubrics with 3 weighted criteria each — TaskCompletion (response quality + error-free), Efficiency (token/round/tool efficiency), Safety (dangerous tools + budget + sensitive data), Cost (total/per-round/budget utilization).
3. **Regression detection** (report.rs): `EvalReport::generate_with_baseline()` compares against a `Baseline` and flags regressions exceeding a threshold (default 10%). Reports are serializable to JSON.

## Decisions

- **Agent loop owns all state**: `runner/` holds the transcript, budget tracker, memory index, loop detector, skills registry, and auth client. Tools are stateless closures that receive params and return results — no shared mutable state leaks into tool implementations.
- **Domain module directories**: `runner/`, `server/`, `session/`, `onchain/`, `config/`, `heartbeat/`, `replay/`, `templates/`, `eval/`, `think/`, `analytics/`, `certificates/`, `audit/` are split into multiple files by responsibility. Each directory has a `mod.rs` with types and re-exports, plus specialized files. This keeps individual files under ~1600 lines while maintaining logical cohesion.
- **Transcript drives message reconstruction**: LLM context is regenerated from the append-only transcript on every iteration via `to_messages()`, not cached. Tool calls are stored inside `think_response` events so replay can reconstruct the full OpenAI message history including `tool_calls` and `tool` role messages.
- **Auth+payment is a shared helper**: `think/` and all x402 tools use `payment::authenticated_paid_request()` from `x402/payment.rs`. This single function handles BRC-31 auth → 402 detection → payment creation → retry (up to 3 attempts). Callers just parse the response bytes.
- **Budget pre-check is conservative**: Runner hardcodes `estimated_sats = 500` before calling `think()`. Actual cost (often lower) comes back in `result.sats_effective`. The overestimate is intentional — it's better to reject a request early than overspend.
- **Budget survives restarts**: `from_config()` calls `replay_log()` to rebuild spending state from the JSONL audit trail. `lifetime_sats_spent` and `lifetime_task_count` on AppState are initialized from disk scan at startup.
- **Conversations separate from transcripts**: Conversations (`session/conversation.rs`) span multiple tasks and represent the user-facing dialogue history. Transcripts (`session/transcript.rs`) are per-task internal audit logs. A conversation may reference multiple task_ids. Hash chain verification is independent of wallet sync.
- **Certificates are best-effort**: `ensure_authorization()` at boot never fails the server. Missing parent wallet, wallet errors, or certificate failures are logged but the agent starts regardless. Self-signed certs provide a fallback identity.
- **Normalization at think/ boundary**: Claude API uses a different message format (content blocks, tool_use/tool_result) than OpenAI. All conversion happens in `think/` — the runner, tools, and transcript work exclusively with OpenAI-format messages and tool_calls.
- **Handler modules split by domain**: Server routes are organized by domain (agent, analytics, audit, budget, chat, compliance, conversations, marketplace, memory, misc, replay, schedules, staging, wallet_ops, tasks/) rather than by HTTP method or auth requirement. `tasks/` is further split into 4 submodules (lifecycle, audit, events, proofs). This makes it easy to find the handler for any given route.
- **SSE streams events, not tokens**: x402 payment is request-response (no streaming tokens). Instead, the server streams agent loop events (thinking started/complete, tool calls, responses) so the UI can show progress in real time.
- **Dev mode is auth-bypass**: When `ParentConfig.identity_key` is empty, all BRC-31 checks return `Ok(())` and CORS allows all origins/methods/headers. Production sets the parent key to lock down the chat and budget endpoints.
- **Continuations as JSON files**: Continuation state is persisted as plain JSON in `workspace/continuations/` rather than in a database. The heartbeat scans this directory on each poll cycle. Completed continuations are moved to a `completed/` subdirectory, not deleted, for auditability.
- **Hot reload is conservative**: Only operational fields are reloaded — wallet URL, parent identity, logging, and security-critical config are explicitly excluded from `reload_safe_fields()`. This prevents accidental privilege escalation or auth bypass via config edit.
- **Self-message prevention is defense-in-depth**: Three layers prevent self-messaging loops: (1) runner partitions inbox by sender identity — self-messages become delivery confirmations, (2) heartbeat sources.rs skips self-sent messages during inbox polling, (3) server/task_spawner.rs skips reply routing when the reply target is the agent itself.
- **Reply obligations replace raw inbox count**: `ReplyObligation` structs are created only for external senders, deduped by sender key. Nudge count only resets when all obligations are fulfilled. Tool-call-only iterations can set `done = true` when all obligations are cleared.
- **Auto-recall is per-iteration**: Memory is re-queried every iteration using the evolving conversation context. The cost is minimal since BM25 is local and vector search is cached.
- **External-origin tasks get restricted tools**: `set_external_origin()` flags tasks from external messages. The runner intersects the tool registry with `external_tool_allowlist()` (10 tools) to prevent untrusted senders from triggering dangerous operations.
- **Certificate revocation via UTXO spending**: Revocation is modeled as spending a UTXO from the `worm-revocation` basket, not deleting the certificate. `is_revoked()` checks UTXO existence. The heartbeat periodically verifies revocation status (every 10 minutes).
- **Proofs separated from checkpoints**: `GET /task/{id}/proofs` returns `ProofsResponse` with two separate arrays: `proofs` (BRC-18 OP_RETURN hash-chained proofs) and `checkpoints` (BRC-48 spendable state tokens). This distinction reflects the different on-chain mechanisms — proofs are immutable audit records, checkpoints are live state.
- **Certificate budget limits are cryptographic spending caps**: Budget limits in BRC-52 certificates provide parent-enforced spending caps that override config-based defaults. The chain is: parent issues cert with limits → agent reads limits at task start → `apply_cert_limits()` overrides tracker → budget checks enforce cert-derived limits. This ensures an operator can cap agent spending without runtime access to the agent's config.
- **Revocation fallback scanning**: `find_revocation_outpoint()` tries three sources in priority order because older certs may not have the `revocationOutpoint` field populated, and some certs stored revocation UTXOs in `worm-state` before the dedicated `worm-revocation` basket was introduced.
- **Per-conversation FIFO queuing**: `spawn_task()` uses per-conversation semaphores (1 permit each) so tasks targeting the same conversation queue in FIFO order instead of being rejected with 409. Tasks start as `Queued` while waiting for the permit.
- **Moderation is separate from sanitization**: `sanitize.rs` handles injection defense (prompt injection, credential leaks). `moderation.rs` handles policy-driven content filtering (PII, profanity, custom patterns). Both are applied but at different points — sanitization in observe(), moderation at 4 points throughout the iteration.
- **Cert fields can only tighten policy**: For moderation, rate limits, and budget enforcement, certificate fields can escalate restrictions (e.g., "flag" → "block") but the agent's local config cannot downgrade a cert-mandated mode. This ensures parent operators have final authority.
- **Analytics are pure computation**: `analytics/` and `time_estimate.rs` are stateless functions that read transcripts but never modify them. No side effects, no shared state. Easy to test and reason about.
- **Replay and eval read transcripts, never modify**: `replay/` and `eval/` consume the same JSONL transcript format produced by `session/transcript.rs`. They are read-only consumers with no coupling to the agent loop runtime.
- **Templates are additive overrides**: `apply_to_config()` only overrides non-zero/non-empty fields, so applying a template never removes config the user has set. Zero values mean "use default", not "set to zero".
- **Auto-escalation is detection-only**: `escalation.rs` detects stuck patterns and emits events, but the runner decides whether to actually pause. This keeps the escalation logic testable and separate from task lifecycle.
- **Tool approval is file-based**: The approval gate uses JSON files (`pending_approval/{call_id}-approved.json`) rather than in-memory signals, so approvals survive server restarts and can be triggered by external systems.
- **Tool result offloading preserves context budget**: Results >8KB are written to workspace files (`tool_output_{name}_{counter}.json`) and replaced with smart previews in the LLM context. `generate_smart_preview()` detects JSON arrays/objects/text and produces content-aware summaries. Retrieval tools in `NO_OFFLOAD_TOOLS` are exempt since their output is the point.
- **Model capabilities are discovered then cached**: `ModelCapabilities` are pre-fetched from x402-info manifests at startup and cached in `AppState.model_capabilities`. Resolution chain: discovered caps → hardcoded fallback → config cap. Applied per-task in `spawn_task()`.
- **Middleware pipeline is opt-in**: Router and FanOut middleware are only active when their config sections (`[llm.routing]`, `[llm.fanout]`) have `enabled = true`. By default, `think()` calls the provider directly with no middleware.

## Gotchas

- **Nudge system is obligation-driven**: If `think()` returns text-only but there are unfulfilled `ReplyObligation`s, the runner injects up to 2 nudge messages. After `MAX_NUDGES` exhausted, it **force-sends** the LLM's text to all senders with unfulfilled obligations. Self-messages never create obligations.
- **Self-messages are delivery confirmations, not tasks**: When the agent sends a message to itself, the inbox picks it up but it's partitioned out during OBSERVE. Self-messages are logged as `DELIVERY CONFIRMED` system messages, not injected as actionable messages.
- **402 can repeat after payment**: `payment::authenticated_paid_request()` retries up to `MAX_PAYMENT_ATTEMPTS = 3`, re-parsing payment requirements each time.
- **Wallet HTTP 200 can still be an error**: `wallet/http.rs` checks for an `error` field in the JSON body even on 200 responses.
- **`update_token()` doesn't spend the old UTXO**: Creates a new token but leaves the old one. Old tokens accumulate until explicitly consumed.
- **Reasoning models need different params**: `is_reasoning_model()` detects `o1/o3/o4-*`, `gpt-5.*`, `gpt-4.1.*` and switches to `max_completion_tokens` with no `temperature`.
- **Claude model detection**: `is_claude_model()` checks for `claude-` prefix. Misrouting produces parse errors.
- **Tool call text extraction fallback**: `text_extract.rs` in `runner/` parses tool calls from text via three strategies: code block JSON, `tool_calls` wrapper, inline JSON objects — all validated against known tool names.
- **Orphaned tool results are silently skipped**: During transcript replay, a `tool_result` with no matching `tool_call` is logged as a warning and excluded.
- **AtomicBEEF wrapping is manual**: `wallet/http.rs` and `wallet/embedded.rs` both prepend `[0x01, 0x01, 0x01, 0x01]` + reversed txid before BEEF bytes. `payment.rs` also wraps raw txs in AtomicBEEF when the wallet returns raw format.
- **Balance pagination**: `get_balance()` loops with limit=100 offsets to sum all spendable outputs.
- **Transcript event IDs are truncated UUIDs**: `Uuid::new_v4().to_string()[..8]` — 8 chars, not full UUIDs.
- **Tool spending is double-tracked**: The runner extracts `sats_paid` and `txid` from tool result JSON and records them in the budget tracker.
- **Broadcast channel capacity**: 4096 events. If the UI is slow to consume, events are dropped.
- **BRC-31 sessions are in-memory only**: Server restart invalidates all sessions — clients must re-handshake.
- **Protected path enforcement is write-only**: `is_protected_path()` blocks `file_write` but not `file_read`.
- **Continuation wake_at null means immediate**: If `wake_at` is null, the heartbeat resumes on the next poll cycle.
- **Claude tool_result grouping**: `flush_tool_results()` in think.rs groups consecutive tool-role messages into a single user message with multiple `tool_result` content blocks.
- **OpenAI compat is opt-in**: `/v1/chat/completions` is only registered when `config.server.openai_compat_enabled` is true.
- **ConfigWatcher watches parent directory**: Because editors write to temp files and rename, notify watches the parent directory of `worm.toml`. Debounce window is 500ms.
- **Hot reload never updates wallet/parent/logging**: `reload_safe_fields()` has an explicit exclusion list.
- **SystemEvent channel is mpsc, not broadcast**: Single consumer = scheduler. Multiple consumers would need a different channel type.
- **Delivery retry backoff is index-clamped**: After the schedule is exhausted, the last interval (600s) repeats until max retries.
- **Auto-recall query sanitization**: Tantivy special characters in recall queries are sanitized before search.
- **Vector embeddings use .vec sidecars**: Missing sidecars degrade gracefully to BM25-only search.
- **External origin tool restriction is intersection-based**: `set_external_origin()` intersects the full registry with the allowlist at dispatch time.
- **Log redaction is byte-scanning**: `RedactingWriter` buffers incomplete lines to avoid splitting patterns across write calls.
- **Revocation outpoint null placeholder**: `NULL_REVOCATION_OUTPOINT` is 72 zero-hex chars. `is_null_revocation_outpoint()` treats both empty and null-placeholder as "no revocation configured".
- **Signed raw responses for exports**: `signed_raw_response()` handles non-JSON content types (CSV, file downloads) with BRC-31 auth headers. Used by audit export endpoint.
- **Certificate budget limits are strings**: Budget fields (`budget_per_task`, etc.) are stored as string values in certificate fields, parsed via `str::parse::<u64>()` in `read_cert_budget_limits()`. Zero or absent values are treated as "no limit".
- **Revocation is_revoked is fail-open**: On wallet connectivity errors, `is_revoked()` returns `Ok(false)` (assume not revoked) to avoid blocking the agent on transient network issues.
- **MCP wallet tools exclude wallet_balance**: The native `wallet_balance` tool is preferred over the MCP version to avoid duplication.
- **Extended thinking requires output budget headroom**: `build_claude_body()` auto-adjusts `max_tokens` to be at least `thinking_budget + 1024`. If the config `max_tokens` is too low, it silently increases.
- **FanOut scoring is heuristic**: `score()` balances response length, cost, and token efficiency — not quality. The `"best"` strategy picks the highest-scoring response, not necessarily the best answer.
- **Smart preview for offloaded results**: JSON arrays get count + field names + first 2 items. JSON objects get key list + value preview. Plain text gets word count + truncated preview. All capped at 2000 chars.
- **Conversation self-healing**: `GET /conversations/{id}` detects missing assistant messages and auto-reconstructs from task transcripts. Silent recovery — only logged, not surfaced to client.
- **Cost comparison uses prefix matching**: Versioned model names (e.g., `gpt-5-mini-2025-08-07`) fall back to longest prefix match against the 9-model pricing table.

## Related

- [auth/CLAUDE.md](auth/CLAUDE.md) — BRC-31 session management (client + server), feeds into `think()` auth flow and server auth
- [x402/CLAUDE.md](x402/CLAUDE.md) — Payment construction, `authenticated_paid_request()` called by `think()` and x402 tools
- [memory/CLAUDE.md](memory/CLAUDE.md) — File storage, tantivy search, wallet-native encryption
- [messagebox/CLAUDE.md](messagebox/CLAUDE.md) — BRC-33 cross-wallet messaging (body-transport payments)
- [tools/CLAUDE.md](tools/CLAUDE.md) — Stateless tool definitions and registry (30 tools across 12 categories)
- [context/CLAUDE.md](context/CLAUDE.md) — Prompt construction and context window management
- [loop_detect/CLAUDE.md](loop_detect/CLAUDE.md) — Circuit breaker for repetitive agent behavior
- [config/CLAUDE.md](config/CLAUDE.md) — Configuration schema (17 sections), loading, hot reload
- [heartbeat/CLAUDE.md](heartbeat/CLAUDE.md) — Scheduler, wake system, work sources, features
- [wallet/CLAUDE.md](wallet/CLAUDE.md) — BRC-100 wallet interface (WalletBackend trait, HTTP + embedded backends)
- [server/CLAUDE.md](server/CLAUDE.md) — HTTP server (69 routes), AppState, route handlers
- [runner/CLAUDE.md](runner/CLAUDE.md) — Agent loop (8 files), moderation, escalation, tool approval, execution
- [think/CLAUDE.md](think/CLAUDE.md) — Dual-provider LLM inference (4 files), circuit breaker, format normalization
- [analytics/CLAUDE.md](analytics/CLAUDE.md) — Cost analytics, efficiency, ROI, benchmarks (5 files)
- [certificates/CLAUDE.md](certificates/CLAUDE.md) — BRC-52 certificates, revocation, policy readers (4 files)
- [audit/CLAUDE.md](audit/CLAUDE.md) — BRC-69 key linkage revelations, audit logging (2 files)
- [replay/CLAUDE.md](replay/CLAUDE.md) — Interactive replay and fork execution from transcripts
- [templates/CLAUDE.md](templates/CLAUDE.md) — Pre-configured agent archetypes (10 built-in templates)
- [eval/CLAUDE.md](eval/CLAUDE.md) — Evaluation framework (trajectory parsing, grading rubrics, regression detection)
- `src/mcp/` — MCP server via rmcp crate, bridges ToolRegistry to MCP protocol (stdio transport)
- [../CLAUDE.md](../CLAUDE.md) — Root project docs with full architecture overview
