# src/heartbeat/
> Event-driven scheduler with priority inbox, concurrency limiting, and wake coalescing.

## Overview

The heartbeat module is the background scheduler that keeps the agent alive between HTTP requests. It polls MessageBox inboxes for new messages, resumes paused continuation tasks, fires recurring schedules, processes the HEARTBEAT.md checklist, triggers autonomous reflection, retries failed deliveries, reaps stale tasks, periodically checks certificate revocation, sweeps stale UTXO tokens, and runs periodic memory maintenance. All work flows through a `PriorityInbox` and is dispatched via `spawn_task()` with concurrency limiting.

The scheduler uses `tokio::select!` with a 250ms coalescing window to batch concurrent wake events into a single processing pass, deduplicating by `WakeReason`.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 998 | `Scheduler` struct, `PriorityInbox`, `TaskPriority`, `InboxItem`, main `run()` loop, `process_wake_batch()`, `drain_inbox()`, `drain_system_events()`, delivery retry, config hot-reload, liveness tick. Also: `HeartbeatDaemon` backward-compat alias for tests. Unit tests for priority inbox, wake coalescing, and system events. |
| `wake.rs` | 96 | `WakeReason` enum (6 variants), `WakeRequest` struct, `SystemEvent` enum (5 variants) with `to_wake_request()` conversion. `HEARTBEAT_CONVERSATION_ID` constant (`"conv-heartbeat"`). |
| `sources.rs` | 332 | Three work-source scan methods on `Scheduler`: `poll_messagebox()`, `scan_continuations()`, `scan_schedules()`. Each pushes `InboxItem`s into the priority inbox. |
| `features.rs` | 681 | Free functions: `is_within_active_hours()` / `is_within_active_hours_at()`, `read_heartbeat_checklist()`, `should_respond_to_message()` (turn-taking protocol), `estimate_proofs_per_day()`, `run_memory_maintenance()` + `MaintenanceSummary`. Scheduler methods: `check_heartbeat_file()`, `scan_reflection()`, `check_certificate_revocation()`, `maybe_sweep_tokens()`, `maybe_log_basket_status()`, `reap_stale_tasks()`, `maybe_run_memory_maintenance()`. |

## Key Exports

### Types (from `mod.rs`)

| Type | Description |
|------|-------------|
| `Scheduler` | Main scheduler struct. Owns `PriorityInbox`, `MessageBoxClient`, `DeliveryQueue`, config, and `Arc<AppState>`. Created via `Scheduler::new()` which returns `(Scheduler, Sender<WakeRequest>)`. Fields include `last_revocation_check` for periodic certificate checks, `last_sweep` for periodic UTXO lifecycle sweeps, `last_memory_maintenance` for periodic memory maintenance, and `last_basket_log` for periodic basket monitoring. |
| `PriorityInbox` | Sorted Vec-backed priority queue. Items ordered by priority (highest first), then FIFO within same priority. Methods: `push()`, `pop()`, `len()`, `is_empty()`. |
| `TaskPriority` | 4-level enum: `Highest` (parent messages), `Normal` (MessageBox, schedules), `Low` (continuations, checklist), `Lowest` (reflection). Derives `Ord`. |
| `InboxItem` | Queued task: `message`, `priority`, `session_id`, `model_override`, `submitted_at`, `reply_to`, `max_iterations`, `origin` (how the task was spawned: `"message"`, `"schedule"`, `"continuation"`, `"reflection"`, `"checklist"`, `"trigger"`). |
| `HeartbeatDaemon` | Legacy simple poller preserved for test compatibility. Uses mpsc channel instead of `PriorityInbox`. |

### Wake system (from `wake.rs`)

| Type | Description |
|------|-------------|
| `WakeReason` | 6-variant enum: `MessageReceived`, `ContinuationReady`, `ScheduleDue`, `HeartbeatFile`, `ExternalTrigger`, `TimerExpired`. Hashable for dedup. |
| `WakeRequest` | `{ reason: WakeReason, priority: u8, timestamp: Instant }`. Priority ordering: retry(0) < interval(1) < default(2) < action(3). |
| `SystemEvent` | 5-variant enum emitted by agent loop and API handlers: `TaskCompleted`, `ScheduleFired`, `MessageReceived`, `ConfigChanged`, `ContinuationReady`. Each converts to a `WakeRequest` via `to_wake_request()`. |
| `HEARTBEAT_CONVERSATION_ID` | Constant `"conv-heartbeat"` — dedicated conversation for checklist and trigger tasks. |

### Free functions (from `features.rs`)

| Function | Description |
|----------|-------------|
| `is_within_active_hours(active_hours)` | Returns `true` if current time is within configured window. `true` if unconfigured. Supports IANA timezones via `chrono_tz`, overnight ranges (e.g., 22:00→06:00). |
| `is_within_active_hours_at(active_hours, now)` | Testable variant accepting explicit timestamp. |
| `read_heartbeat_checklist(file, workspace)` | Reads `HEARTBEAT.md` (or configured file), returns `Some(task_desc)` if it has meaningful non-header content. Returns `None` if disabled, missing, or empty. |
| `should_respond_to_message(body)` | Turn-taking guard for structured `agent_message` payloads. Returns `false` if `done == true` or `turn >= max_turns` (default 5). Non-structured messages always pass. |
| `run_memory_maintenance(memory_dir, config)` | Run memory maintenance: flag stale session/execution entries, detect near-duplicates via BM25+Jaccard similarity. Conservative — NEVER deletes anything. Knowledge entries are never flagged as stale. Returns `MaintenanceSummary`. |
| `estimate_proofs_per_day(count, oldest_age_hours)` | Estimate proof creation rate from total count and oldest proof age. Returns `None` if too recent to estimate (<0.01 hours). Returns `Some(0.0)` if no proofs. Used by `maybe_log_basket_status()` for growth rate reporting. |
| `MaintenanceSummary` | Struct: `total_entries`, `knowledge_count`, `session_count`, `execution_count`, `stale_sessions_flagged`, `duplicate_pairs`, `stale_ids`, `duplicate_id_pairs`. |

## Scheduler Loop Architecture

```
Scheduler::run()
│
├── Pre-drain: drain_system_events() (non-blocking)
├── Block on: wake_rx.recv() OR timer fallback (inbox_poll_secs)
├── Coalescing window: drain wake_rx for 250ms
├── Post-drain: drain_system_events() again
├── Deduplicate by WakeReason (HashSet)
├── Sort by priority (lower u8 = higher urgency)
│
└── process_wake_batch()
    ├── Config hot-reload check (notify watcher via config_rx)
    ├── Guard: active hours check → skip if outside window
    ├── Guard: capacity check → skip if active >= max_concurrent
    ├── poll_messagebox() → InboxItems at Normal/Highest priority
    ├── scan_continuations() → InboxItems at Low priority
    ├── scan_schedules() → InboxItems at Normal priority
    ├── check_heartbeat_file() → InboxItem at Low priority (with SHA-256 dedup + cooldown)
    ├── scan_reflection() → InboxItem at Lowest priority (interval + concurrency guards)
    ├── retry_due_deliveries() → retries failed MessageBox sends
    ├── Drain /heartbeat/trigger channel → InboxItems at Highest priority
    ├── drain_inbox() → spawn_task() while has_capacity()
    ├── reap_stale_tasks() → mark 0-iteration tasks stuck >30min as Error + clean cancel_flags
    ├── maybe_sweep_tokens() → periodic UTXO lifecycle sweep (LifecycleConfig interval)
    ├── check_certificate_revocation() → every 10 minutes, check parent cert validity
    ├── maybe_run_memory_maintenance() → periodic stale/duplicate detection (MemoryConfig interval)
    ├── maybe_log_basket_status() → periodic UTXO basket size logging (LifecycleConfig interval)
    └── record_liveness_tick() → epoch seconds for /health monitoring
```

## AppState Sub-Structs

The scheduler accesses `AppState` fields through two sub-structs:

- **`app_state.task_mgr`** — Task management: `active_task_count` (AtomicUsize), `tasks` (Mutex), `cancel_flags` (Mutex), `conversation_semaphores` (Mutex of HashMap to Semaphore).
- **`app_state.scheduler`** — Scheduler plumbing: `system_event_tx`/`system_event_rx` (mpsc channel for SystemEvents), `heartbeat_rx` (mpsc for `/heartbeat/trigger`), `last_scheduler_tick` (AtomicU64 for liveness).
- **`app_state.auth`** — Auth state: `server_identity_key` (used by `poll_messagebox()` to detect self-sent messages).

## Work Sources (sources.rs)

**`poll_messagebox()`**: Polls all MessageBox inboxes. Skips self-sent messages (feedback loop prevention) by comparing `msg.sender` against `app_state.auth.server_identity_key`. Applies `should_respond_to_message()` turn-taking guard. Parent messages (`config.parent.identity_key` match) get `Highest` priority. Looks up existing conversations by sender key via `ConversationManager::find_by_participant()`. Acknowledges processed messages after queuing.

**`scan_continuations()`**: Reads JSON files from `workspace/continuations/`. Checks `wake_at` timestamp — skips if not yet due, resumes immediately if `wake_at` is null. Moves processed files to `continuations/completed/`. Priority: `Low`.

**`scan_schedules()`**: Reads JSON files from `workspace/schedules/`. Supports three schedule types: `Once` (disabled after first run), `Cron` (computes next via `compute_next_cron_run()`), and `Interval` (next = now + interval_secs). Atomic write-back via `.json.tmp` → rename. Priority: `Normal`.

## Features (features.rs)

**Active hours**: IANA timezone-aware time window enforcement. Handles normal ranges (09:00→22:00) and overnight ranges (22:00→06:00). Gracefully degrades to always-active on invalid input.

**Checklist dedup**: SHA-256 hash of `HEARTBEAT.md` content compared against `last_checklist_hash`. Skipped if unchanged within `checklist_cooldown_secs` (default 600s). Tasks capped at 5 iterations (`max_iterations` on `InboxItem`).

**Reflection**: Autonomous review tasks at configurable interval (`reflection_interval_secs`). Uses separate model (`reflection_model`). Capped at `reflection_max_iterations`. Skipped if a reflection task is already running — checked via `conversation_semaphores` (semaphore permit availability for the `"reflection"` session ID; `available_permits() == 0` means in-progress).

**Certificate revocation check**: Periodic check (every 10 minutes, `REVOCATION_CHECK_INTERVAL_SECS = 600`) for parent-signed certificate validity. Uses `CertificateManager::certificate_status()` to check if a parent-signed cert exists, then `is_revoked()` to verify the revocation UTXO hasn't been spent. Logs an error if revoked but does not force-quit — the operator can re-issue a certificate. Skips silently if no parent-signed certificate exists.

**UTXO lifecycle sweep**: Periodic sweep of stale BRC-48 state tokens via `maybe_sweep_tokens()`. Controlled by `LifecycleConfig`: `auto_sweep_enabled` (guard), `sweep_interval_minutes` (minimum time between sweeps), `budget_token_max_age_hours` (max age for BudgetAllocation tokens), `checkpoint_max_age_hours` (max age for Checkpoint tokens). BRC-18 proofs in `worm-proofs` are NEVER swept (immutable audit trail). When compliance mode is enabled, the configured `default_retention_days` is respected as a minimum retention period. Uses `sweep_stale_tokens_with_retention()` from `onchain::state`.

**Basket monitoring**: Periodic logging of UTXO basket sizes via `maybe_log_basket_status()`. Fetches counts for all three baskets (`worm-state`, `worm-budget`, `worm-proofs`) and logs at INFO level. Includes a proofs-per-day growth rate estimate via `estimate_proofs_per_day()`. Controlled by `LifecycleConfig::basket_monitoring_interval_secs` (0 = disabled).

**Stale task reaping**: Tasks stuck as `Running` with 0 iterations for >30 minutes are marked `Error` and have their `active_task_count` decremented. Cancel flags for reaped tasks are also cleaned up.

**Memory maintenance**: Periodic maintenance of the memory system via `maybe_run_memory_maintenance()`. Controlled by `MemoryConfig`: `maintenance_enabled` (guard, default OFF), `maintenance_interval_secs` (minimum time between runs), `stale_session_days` (age threshold for stale flagging). Two operations: (1) flag stale session/execution entries older than the threshold — knowledge entries are NEVER flagged, (2) detect near-duplicate pairs via BM25 search + Jaccard similarity (>0.6 threshold). Conservative — never deletes anything, only logs and reports. Uses a temporary tantivy index (`memory/.maintenance-index`) for dedup scanning. Returns `MaintenanceSummary` with counts and flagged IDs.

## Config Integration

Relevant fields from `HeartbeatConfig`:

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `true` | Enable/disable scheduler |
| `inbox_poll_secs` | `60` | Base timer interval |
| `max_concurrent_tasks` | `3` | Concurrency limit |
| `active_hours.start/end/timezone` | None | Operating window (IANA tz) |
| `checklist_file` | `"HEARTBEAT.md"` | Proactive check file |
| `checklist_cooldown_secs` | `600` | Dedup cooldown for unchanged checklist |
| `reflection_enabled` | `false` | Enable autonomous reflection |
| `reflection_interval_secs` | — | Minimum time between reflections |
| `reflection_model` | — | Model override for reflection tasks |
| `reflection_max_iterations` | — | Max iterations for reflection tasks |

Relevant fields from `MemoryConfig` (used by `maybe_run_memory_maintenance()`):

| Field | Default | Description |
|-------|---------|-------------|
| `maintenance_enabled` | `false` | Guard — maintenance only runs when true |
| `maintenance_interval_secs` | — | Minimum time between maintenance runs |
| `stale_session_days` | — | Age threshold (days) for flagging stale session/execution entries |

Relevant fields from `LifecycleConfig` (used by `maybe_sweep_tokens()`):

| Field | Description |
|-------|-------------|
| `auto_sweep_enabled` | Guard — sweep only runs when true |
| `sweep_interval_minutes` | Minimum time between sweeps |
| `budget_token_max_age_hours` | Max age for BudgetAllocation tokens before sweep |
| `checkpoint_max_age_hours` | Max age for Checkpoint tokens before sweep |
| `basket_monitoring_interval_secs` | Interval for basket size logging (0 = disabled) |

Hot-reload: `config_rx` watch channel from `ConfigWatcher` triggers `reload_safe_fields()` on the scheduler's config copy. Updates `max_concurrent` and logs the change.

## Related

- [`src/CLAUDE.md`](../CLAUDE.md) — Top-level module inventory and architecture
- [`src/server/CLAUDE.md`](../server/CLAUDE.md) — `AppState.scheduler` fields (`system_event_tx/rx`, `heartbeat_rx`, `last_scheduler_tick`), `spawn_task()`
- `src/delivery.rs` — `DeliveryQueue` used by `retry_due_deliveries()`
- `src/certificates.rs` — `CertificateManager` used by `check_certificate_revocation()`
- `src/onchain/state.rs` — `sweep_stale_tokens_with_retention()` used by `maybe_sweep_tokens()`, `BASKET_BUDGET`/`BASKET_STATE` constants
- `src/tools/schedule_tools.rs` — `Schedule` struct, `ScheduleType`, `compute_next_cron_run()`
- `src/config/schema.rs` — `HeartbeatConfig`, `ActiveHours`, `LifecycleConfig` structs
- `src/session/conversation.rs` — `ConversationManager::find_by_participant()` used for message routing
- `src/memory/store.rs` — `MemoryStore`, `MemoryCategory` used by `run_memory_maintenance()`
- `src/memory/search.rs` — `MemoryIndex`, `extract_key_terms()`, `jaccard_similarity()` used by duplicate detection
- `tests/test_heartbeat.rs` — Integration tests for scheduler behavior
