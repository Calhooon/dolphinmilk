# tests/server

> 113 tests across 3 files covering Axum HTTP endpoints, SSE event serialization, BRC-31 auth, task lifecycle, audit search, and export formats.

## Overview

Tests the HTTP server layer (`src/server/`) using Axum's `tower::ServiceExt::oneshot()` pattern — no TCP listener needed. All tests run against a dev-mode router backed by a leaked `TempDir` workspace. Tests cover endpoint behavior, request/response serialization, auth enforcement (dev mode vs parent-key mode), task reconstruction from disk transcripts, and conversation concurrency control.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_server.rs | 76 | HTTP endpoints, task lifecycle, reconstruction, audit export, caching, concurrency, PDF export |
| test_events.rs | 26 | SSE event serialization (9 variants), truncation, BRC-31 auth, chat types, dev/parent-key auth |
| test_audit_search.rs | 11 | `GET /audit/search` endpoint: transcript search, pagination, filtering, case-insensitive matching |

## Test categories

### test_server.rs — HTTP endpoints and task lifecycle (76 tests)

**Endpoint coverage:**

| Endpoint | Tests | What's verified |
|----------|------:|-----------------|
| `GET /health` | 5 | Status "ok", version, uptime, scheduler tick age, `wallet_connected` field |
| `GET /status` | 2 | Empty initial state, `active_count`/`total_sats` top-level fields (no nested budget) |
| `POST /task` | 2 | Returns 202 with UUID, rejects invalid body with 422 |
| `GET /task/{id}` | 1 | Returns 404 for nonexistent task |
| `POST /task/{id}/cancel` | 1 | Accepts cancel for running/queued tasks |
| `POST /message` | 1 | Accepts BRC-33 message with sender pubkey |
| `GET /budget` | 1 | Returns `BudgetReport` with limits |
| `GET /task/{id}/conversation` | 2 | Reconstructs messages from transcript, 404 for missing |
| `GET /conversations` | 1 | Returns empty list initially |
| `POST /conversations/{id}/compact` | 2 | 404 for missing, noop for short conversations |
| `POST /v1/chat/completions` | 4 | OpenAI compat: exists in dev mode, rejects no-user-message, 501 for streaming, config default |
| `GET /rates/bsv-usd` | 2 | Rate positive, enhanced fields (margin, source, stale, updated_at) |
| `POST /heartbeat/trigger` | 3 | Returns triggered, custom message delivery, 503 when disabled |
| `GET /task/{id}/audit/export` | 7 | CSV/JSON formats, default CSV, 404 missing, columns, summary, SHA-256 hash |
| `GET /task/{id}/audit/export?format=pdf` | 3 | Route exists, graceful 503 without Chrome, 404 for missing task |
| `GET /budget/export` | 1 | JSON default, PDF fallback |

**Task reconstruction from disk** (5 tests): Verifies that `build_router` / `create_app_state` reconstructs completed tasks from `session.jsonl` transcripts at startup — simulating server restart. Tests cover complete/error status, proof txid extraction, multiple task reconstruction, and session ID mapping from `session_start` events.

**Supervisor pattern** (4 tests): Validates status transitions on task panic (Running/Queued → Error) and guards against overwriting already-completed tasks.

**Conversation concurrency** (5 tests): Per-conversation semaphore tests — creation on demand, blocking concurrent access, FIFO ordering of waiters, and Queued status serialization/transitions.

**Caching** (3 tests): `cached_tool_names` populated and sorted at startup, matches direct registry, `cached_balance` starts as None.

**Certificate revocation** (5 tests): `AgentResponse.certificate_revoked` field presence, None omission, backward compat, null outpoint short-circuit, outpoint parsing.

**Type serialization** (5 tests): `TaskStatus` (Running/Complete/Error/Queued/Cancelled), `TaskInfo` None-field omission, `BudgetReport.balance`, margin calculation.

### test_events.rs — SSE events and auth endpoints (26 tests)

**StepEvent serialization** (4 tests): All 9 event variants serialize with `"type"` field, type names are snake_case, `SessionStarted` fields roundtrip correctly.

**SSE truncation** (4 tests): `truncate_for_sse` preserves text ≤2000 chars, truncates at 2001+ with `"...truncated"` suffix, boundary tests at exactly 2000 and 2001 chars.

**SSE formatting** (2 tests): `format_sse_event` produces valid JSON, Done event includes all fields.

**StepEvent traits** (2 tests): Clone produces identical JSON, Debug contains variant name and message.

**Chat types** (6 tests): `ChatRequest` default/custom `max_iterations`, optional `session_id`, `ChatMessage` None-field omission, tool message includes `tool_name`/`call_id`.

**BRC-31 handshake** (2 tests): `HandshakeRequest`/`HandshakeResponse` serde with camelCase field names.

**Auth endpoints via oneshot** (2 tests): `/.well-known/auth` handshake succeeds and returns correct nonce echo, rejects wrong identity key with 403.

**Dev mode vs parent-key mode** (4 tests): `/chat` and `/budget` accessible without auth in dev mode (empty parent key), return 401 when `parent.identity_key` is set.

### test_audit_search.rs — Audit search endpoint (11 tests)

All tests use `create_test_workspace()` which builds two task transcripts:
- **task-alpha**: `gpt-5-mini` model, `execute_bash` tool call, two think cycles
- **task-beta**: `claude-sonnet-4-20250514` model, `memory_search` tool call, error event

| Test | What's verified |
|------|-----------------|
| endpoint_exists | `GET /audit/search?q=pi` returns 200 |
| empty_query_returns_error | Missing, empty, and whitespace-only `q` all return 400 |
| finds_think_event | Model name search returns results from correct task |
| finds_tool_call | Tool name search finds `tool_call`/`tool_result` events |
| no_results | Nonexistent query returns empty array with total=0 |
| pagination | `limit` and `offset` params produce different result pages |
| across_tasks | Cross-task search finds results from both task-alpha and task-beta |
| result_format | Response fields: `query`, `results[]`, `total`, `limit`, `offset`; each result has `task_id`, `event_type`, `timestamp`, `matched_content` |
| case_insensitive | "GPT-5-MINI" finds "gpt-5-mini" events |
| limit_capped | `limit=999` gets capped to 100 |
| finds_error_events | "insufficient funds" finds error event in task-beta |

## Key patterns

### Shared test router (common/mod.rs)

All three files import helpers from `tests/common/mod.rs` via `#[path = "../common/mod.rs"] mod common;`. Three router factories are available:

```rust
// Fresh dev-mode router with leaked TempDir
pub async fn test_router() -> axum::Router;

// Same, but also returns the workspace path for inspection
pub async fn test_router_with_workspace() -> (axum::Router, PathBuf);

// Router backed by an existing workspace directory
pub async fn test_router_for_workspace(workspace: PathBuf) -> axum::Router;
```

The `TempDir` is intentionally leaked via `std::mem::forget()` because `AppState` holds an `Arc<PathBuf>` that outlives the test scope.

For tests that need pre-populated transcripts, workspace setup happens before `build_router`:

```rust
let task_dir = workspace.join(format!("tasks/{task_id}"));
std::fs::create_dir_all(&task_dir).unwrap();
let mut transcript = Transcript::new(task_dir.join("session.jsonl"));
transcript.record_user("What is 2+2?");
// ... more events ...
let app = server::build_router(WormConfig::default(), workspace).await;
```

### Oneshot request pattern

Every HTTP test follows the same pattern — build a `Request`, send via `oneshot()`, assert status and body:

```rust
let req = Request::builder()
    .method(http::Method::POST)
    .uri("/task")
    .header("content-type", "application/json")
    .body(Body::from(r#"{"task":"test","max_iterations":1}"#))
    .unwrap();
let resp = app.oneshot(req).await.unwrap();
assert_eq!(resp.status(), StatusCode::ACCEPTED);
```

### Dev mode vs parent-key auth

Dev mode (default `WormConfig` with empty `parent.identity_key`) allows unauthenticated access. Parent-key mode requires BRC-31 Authrite:

```rust
let mut config = WormConfig::default();
config.parent.identity_key = "034aa44668...".into(); // Sets parent-key mode
let app = server::build_router(config, path).await;
// Now /chat and /budget return 401 without auth headers
```

### AppState direct manipulation

Some tests use `create_app_state()` + `build_router_with_state()` to inspect or modify server state:

```rust
let state = server::create_app_state(WormConfig::default(), path).await;
state.scheduler.last_scheduler_tick.store(now - 3, Ordering::Relaxed);
let app = server::build_router_with_state(state);
```

## Key types tested

| Type | Module | Tests cover |
|------|--------|-------------|
| `StepEvent` | `events` | 9 variants, serde, Clone, Debug |
| `TaskStatus` | `server` | Running, Complete, Error, Queued, Cancelled serde |
| `TaskInfo` | `server` | Full lifecycle, None-field omission, proof txids, tags |
| `TaskRequest` / `TaskResponse` | `server` | Submission, default iterations |
| `ChatRequest` / `ChatMessage` | `server` | Session ID, iterations, tool messages |
| `HealthResponse` | `server` | Version, uptime, wallet_connected, scheduler tick |
| `StatusResponse` | `server` | Tasks list, active_count, total_sats |
| `ConversationResponse` | `server` | Task ID, message array |
| `AgentResponse` | `server` | Certificate status/revoked, tools, models |
| `BudgetReport` / `LimitsReport` | `budget` | Balance, per-window limits, enforcement |
| `AuditSearchResponse` | `server` | Query, results, total, limit, offset |
| `HandshakeRequest` / `HandshakeResponse` | `auth` | BRC-31 camelCase serde |
| `MessageResponse` | `server` | Accepted flag |

## Helpers

| Helper | File | Purpose |
|--------|------|---------|
| `test_router()` | common/mod.rs | Dev-mode router with leaked TempDir |
| `test_router_with_workspace()` | common/mod.rs | Router + workspace PathBuf for inspection |
| `test_router_for_workspace()` | common/mod.rs | Router from existing workspace path |
| `create_test_workspace()` | test_audit_search.rs | Two-task workspace with known transcript events |
| `router_with_task()` | test_server.rs | Router + task ID for PDF/audit export tests |
| `apply_margin()` | test_server.rs | Rate margin calculation (rate × (1 + margin%/100)) |

## Running

```bash
# All server tests
cargo test --test test_server
cargo test --test test_events
cargo test --test test_audit_search

# By pattern
cargo test audit_search          # All audit search tests
cargo test reconstruction        # Task reconstruction tests
cargo test heartbeat_trigger     # Heartbeat endpoint tests
cargo test openai_compat         # OpenAI compatibility tests
cargo test pdf_export            # PDF export tests
cargo test semaphore             # Concurrency tests
```

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Parent test directory overview, all 113 server tests listed
- [src/server/CLAUDE.md](../../src/server/CLAUDE.md) -- Server module implementation (66 routes, handler modules)
- [src/server/handlers/CLAUDE.md](../../src/server/handlers/CLAUDE.md) -- Handler implementations
- [src/session/CLAUDE.md](../../src/session/CLAUDE.md) -- Conversations and transcripts
- [tests/security/](../security/) -- BRC-31 auth tests, injection defense, certificate revocation
- [tests/integration/CLAUDE.md](../integration/CLAUDE.md) -- Playwright E2E tests that exercise the same endpoints live
