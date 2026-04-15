# Architecture

This document describes the internal architecture of Dolphin Milk for engineers who want to understand, modify, or extend the system. It assumes familiarity with Rust, async programming, and basic blockchain concepts.

## System Overview

Dolphin Milk is a single-binary Rust application that runs an autonomous AI agent loop. The agent pays for LLM inference using BSV micropayments via the x402 protocol, executes tools, records on-chain proofs, and manages its own budget.

```
                       dolphin-milk
┌─────────────────────────────────────────────────────────────────┐
│                                                                 │
│   User / Parent                                                 │
│     │                                                           │
│     ▼                                                           │
│   ┌──────────────┐     ┌──────────────┐     ┌───────────────┐  │
│   │  HTTP API     │────▶│ Task Spawner │────▶│  Agent Loop   │  │
│   │  (49 routes)  │     │              │     │  (Runner)     │  │
│   │  + Web UI     │     └──────────────┘     └───┬───┬───┬───┘  │
│   └──────────────┘                               │   │   │      │
│         ▲                                        │   │   │      │
│         │ SSE events                             │   │   │      │
│         │                          ┌─────────────┘   │   │      │
│         │                          ▼                 │   │      │
│   ┌─────┴────────┐     ┌──────────────┐             │   │      │
│   │  Transcript   │◀────│    Think     │             │   │      │
│   │  (JSONL)      │     │   (LLM)     │             │   │      │
│   └──────────────┘     └──────┬───────┘             │   │      │
│                               │                      │   │      │
│                               ▼                      ▼   │      │
│                        ┌──────────────┐   ┌──────────┐   │      │
│                        │    x402      │   │  Tools   │   │      │
│                        │  (payment)   │   │  (30)    │   │      │
│                        └──────┬───────┘   └──────────┘   │      │
│                               │                          │      │
│                               ▼                          ▼      │
│                        ┌──────────────┐   ┌──────────────────┐  │
│                        │   Auth       │   │  On-chain        │  │
│                        │  (BRC-31)    │   │  (proofs, state, │  │
│                        └──────┬───────┘   │   budget)        │  │
│                               │           └────────┬─────────┘  │
│                               ▼                    │            │
│                        ┌──────────────┐            │            │
│                        │   Wallet     │◀───────────┘            │
│                        │  (BRC-100)   │                         │
│                        └──────┬───────┘                         │
│                               │                                 │
└───────────────────────────────┼─────────────────────────────────┘
                                │ HTTP
                                ▼
                         bsv-wallet-cli
                        (localhost:3322)
```

## Core Loop

The agent operates on a fixed cycle, up to 50 iterations per task:

```
OBSERVE  ──▶  BUILD  ──▶  THINK  ──▶  ACT  ──▶  RECORD  ──▶  BUDGET CHECK
   │                        │           │           │               │
   │ Poll inbox,            │ x402      │ Execute   │ BRC-18 proof, │ Check
   │ partition messages,    │ payment   │ tool      │ transcript,   │ limits,
   │ build obligations      │ to LLM    │ calls     │ BEEF receipt  │ low-power
   │                        │           │           │               │
   └────────────────────────┴───────────┴───────────┴───────────────┘
                              (repeat until done or budget exhausted)
```

**Entry points:**
- `dolphin-milk run "task"` -- single task, exits when done
- `dolphin-milk serve --port 8080` -- HTTP daemon, accepts tasks via API, runs heartbeat scheduler
- `dolphin-milk mcp` -- MCP server over stdio for external AI client integration

The loop is implemented across four files in `src/runner/`:
- `mod.rs` -- types (`WormLoop`, `LoopState`, `ContinuationState`), construction, helpers
- `lifecycle.rs` -- `setup_task()`, `run_loop()`, `teardown_task()`, `run()`
- `step.rs` -- per-iteration phases: observe, build, think, act, record
- `text_extract.rs` -- fallback extraction of tool calls from reasoning model text output

## Module Map

### Core Agent (`src/`)

| Module | Files | Purpose |
|--------|-------|---------|
| `runner/` | 4 | Agent loop orchestration. `WormLoop` struct owns all per-task state. |
| `think.rs` | 1 | LLM inference via x402 payment. Dual-provider (OpenAI + Claude). Normalizes responses to a common format at this boundary. |
| `wallet.rs` | 1 | HTTP client to `bsv-wallet-cli`. All 28 BRC-100 endpoints. The **only** module that talks to the wallet. |
| `error.rs` | 1 | `WormError` enum with 10 variants (wallet, payment, tool, budget, config, loop, memory, auth, messagebox, conversation). All use `thiserror`. |
| `types.rs` | 1 | Newtype ID wrappers: `TaskId`, `SessionId`, `ConversationId`, `ProofTxid`. Compile-time safety. |
| `cli.rs` | 1 | Clap-derived CLI: `run`, `status`, `think`, `receive`, `fund`, `serve`, `mcp`. |
| `lib.rs` | 1 | Module re-exports organized by domain. |

### Authentication and Payments (`src/auth/`, `src/x402/`)

| Module | Files | Purpose |
|--------|-------|---------|
| `auth/` | 5 | BRC-31 Authrite mutual authentication. Client for outbound requests, server for inbound API auth. BRC-104 binary serialization. 1-hour session TTL. |
| `x402/` | 8 | Payment flow: parse 402 requirements, derive BRC-29 payment key, build P2PKH tx via wallet, retry with payment header. Also: service discovery (`/.well-known/x402-info`), service registry with 5-min disk cache, LRU response cache, circuit breaker (Closed/Open/HalfOpen per endpoint), refund parsing, BRC-105 multipart for payments over 8KB. |

### On-Chain State (`src/onchain/`)

| Module | Files | Purpose |
|--------|-------|---------|
| `onchain/proofs.rs` | 1 | BRC-18 OP_RETURN proofs. Creates `SHA-256(prev_hash \|\| data \|\| timestamp)` chain. Proof types: Decision, TaskCompletion, BudgetSnapshot, MemoryCommitment, CapabilityProof, ConversationIntegrity, CertificateRevocation. ~200 sats per proof. |
| `onchain/state.rs` | 1 | BRC-48 PushDrop state tokens. Types: TaskCommitment, BudgetAllocation, CapabilityDeclaration, Checkpoint. Managed in BRC-46 baskets (`worm-state`, `worm-budget`, `worm-proofs`). |
| `onchain/budget.rs` | 1 | Per-service spending tracker with JSONL audit log. Multi-tier limits (per-task, per-hour, per-day, per-week, per-month, lifetime). Strict and advisory enforcement modes. |

### Session Lifecycle (`src/session/`)

| Module | Files | Purpose |
|--------|-------|---------|
| `session/conversation.rs` | 1 | Persistent multi-turn conversations with BRC-60 hash chain linking. Storage: `workspace/conversations/{id}/meta.json` + `messages.jsonl`. Conversations span multiple tasks. |
| `session/events.rs` | 1 | `StepEvent` enum for SSE streaming to the web UI. 11 event types (ThinkingStarted, ThinkingComplete, ToolCallStarted, ToolCallComplete, Response, Done, Error, BudgetUpdate, BudgetAdvisory, SessionStarted, ApprovalRequired). |
| `session/transcript.rs` | 1 | Append-only JSONL transcript with 18 event types. Per-task audit log. Supports replay for session resume and crash recovery. |

### Tools (`src/tools/`)

| Module | Files | Purpose |
|--------|-------|---------|
| `registry.rs` | 1 | `ToolRegistry` -- register, lookup, execute tools. `ToolDef` struct: name, description, JSON schema parameters, async execute closure, category. 15 always-on tools in the system prompt, 15 discoverable via `search_tools`. |
| `sandbox.rs` | 1 | `execute_bash`, `file_read`, `file_write`, `file_search`, `web_fetch`, `continue_task`. Protected path enforcement and forbidden command blocking. |
| `wallet_tools.rs` | 1 | `wallet_balance`, `wallet_identity`, `wallet_encrypt`, `wallet_decrypt`, `check_certificates`, `wallet_call`. |
| `memory_tools.rs` | 1 | `memory_store`, `memory_search`. |
| `messagebox_tools.rs` | 1 | `send_message`, `check_inbox`. BRC-33/77/78. |
| `conversation_tools.rs` | 1 | `list_conversations`, `read_conversation`. |
| `x402_tools/` | 4 | `discover_services`, `discover_endpoints`, `x402_call` (generic). `generate_image`, `upload_to_nanostore` (recipe wrappers). Shared `do_x402_request()` helper. |
| `schedule_tools.rs` | 1 | `create_schedule`, `list_schedules`, `cancel_schedule`. |
| `browser_tools.rs` | 1 | `browser` -- headless Chrome via chromiumoxide CDP. Snapshot-ref-act pattern: navigate, read accessibility tree with refs, interact using refs. |
| `discovery_tools.rs` | 1 | `discover_agent`, `verify_agent`. BRC-56 peer discovery. |

### Memory (`src/memory/`)

| Module | Files | Purpose |
|--------|-------|---------|
| `store.rs` | 1 | File-based storage. Markdown files with YAML frontmatter in category dirs: `knowledge/`, `sessions/`, `execution/`. |
| `search.rs` | 1 | Tantivy BM25 search with auto-recall, MMR diversity reranking, and recency weighting. Index rebuilt from files on startup. |
| `encrypt.rs` | 1 | Wallet-native encryption. Protocol `[2, "worm memory"]`, counterparty `"self"`. |
| `sync.rs` | 1 | NanoStore backup for off-site memory persistence. |
| `uhrp.rs` | 1 | Content-addressed hashing (UHRP) for memory entries. |
| `session.rs` | 1 | Structured session summarization from transcript events. |
| `processing.rs` | 1 | Post-store processing: duplicate detection, tag extraction, quality validation. Non-fatal. |

### Infrastructure

| Module | Files | Purpose |
|--------|-------|---------|
| `server/` | 15 | Axum HTTP API. 49 routes across 9 handler modules. `AppState` with shared budget tracker, task manager, auth state, scheduler channels. BRC-31 auth on protected endpoints. Serves Lit frontend at `/ui/`. |
| `config/` | 4 | `WormConfig` loaded from `dolphin-milk.toml` with `DOLPHIN_MILK_*` env overrides. 14 config sections. Hot-reload via file watcher with 500ms debounce. |
| `context/` | 3 | `prompt.rs`: dynamic system prompt builder (identity, environment, wallet, tools, memory, skills, working principles). `manager.rs`: token tracking, history truncation, file-based offloading for large tool results. |
| `skills/` | 2 | YAML frontmatter + markdown `SKILL.md` files loaded from `skills/` directory. Auto-activated skills injected into every prompt. 6 built-in skills. |
| `heartbeat/` | 4 | Priority-aware scheduler. Polls MessageBox inboxes, scans continuations, scans schedules. Adaptive tick intervals (5s active, 10s idle, 60s quiet). Concurrency limiter. Active hours window. |
| `mcp/` | 3 | MCP server (`dolphin-milk mcp`) exposes ToolRegistry via rmcp over stdio. MCP client connects to external MCP servers (e.g., `bsv-wallet-mcp`) and injects their tools. |
| `messagebox/` | 3 | BRC-33 MessageBox client for cross-wallet agent communication. BRC-77 signing, BRC-78 encryption. |
| `delivery.rs` | 1 | Persistent disk-backed delivery queue for failed MessageBox sends. Exponential backoff (5s to 600s), max 5 retries. |
| `certificates.rs` | 1 | BRC-52 agent authorization certificates. Issue, list, revoke, relinquish. Revocation via UTXO spending. |
| `discovery.rs` | 1 | BRC-56 peer discovery. Discover agents by identity key or attributes. |
| `sanitize.rs` | 1 | 4-layer injection defense: content sanitization, boundary wrapping, tool allowlisting, parent bypass. Aho-Corasick multi-pattern matching for prompt injection detection. |
| `logging.rs` | 1 | `RedactingWriter` with 6 byte-scanning pattern matchers. Scrubs API keys, Bearer tokens, BRC-31 headers, authrite tokens, base64 BEEF, hex private keys. |
| `loop_detect/` | 2 | Circuit breaker for repetitive agent behavior. Warn, critical, and breaker thresholds. |
| `moderation.rs` | 1 | Content moderation engine (PII detection, profanity filtering). Policy-driven via config and certificate fields. |
| `analytics.rs` | 1 | Usage analytics and metrics collection. |
| `time_estimate.rs` | 1 | Task duration estimation. |

## Request Flow: User Message to Response

This traces a `POST /chat` request through the system.

```
1.  POST /chat { message: "Find the weather in Austin" }
        │
2.      ▼  server/handlers/chat.rs
    Parse request, extract session_id, BRC-31 auth verification
        │
3.      ▼  server/task_spawner.rs :: spawn_task()
    Acquire per-conversation semaphore (FIFO queuing)
    Create WormLoop with config, workspace, tools, skills
    Spawn tokio task, return { task_id, session_id }
        │
4.      ▼  runner/lifecycle.rs :: setup_task()
    Reset LoopState, apply tool allowlist if external origin
    Fetch BRC-52 certificate, build moderation engine
    Create BRC-48 TaskCommitment + BudgetAllocation tokens
    Record session_start in transcript
        │
5.      ▼  runner/lifecycle.rs :: run_loop()  (up to 50 iterations)
        │
        │   ┌──── runner/step.rs :: step() ────────────────────────┐
        │   │                                                       │
        │   │  OBSERVE: poll MessageBox inbox, cache wallet balance │
        │   │  BUILD:   build_system_prompt() with dynamic sections │
        │   │  THINK:   think() → auth + x402 payment → LLM call   │
        │   │  ACT:     parse tool_calls, execute in parallel       │
        │   │  RECORD:  BRC-18 Decision proof, transcript entry     │
        │   │  BUDGET:  check limits, update BudgetAllocation token │
        │   │                                                       │
        │   └───────────────────────────────────────────────────────┘
        │
6.      ▼  runner/lifecycle.rs :: teardown_task()
    Create TaskCompletion + BudgetSnapshot proofs
    Relinquish TaskCommitment token
    Create Checkpoint token
    Summarize session to memory
        │
7.      ▼  session/events.rs :: StepEvent::Done
    Broadcast SSE event to polling UI clients
```

The UI polls `GET /task/{id}/events?since=N` for incremental transcript events.

## Key Abstractions

### ToolDef and ToolRegistry

Every tool is a `ToolDef` with a stateless async closure:

```rust
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,        // JSON Schema
    pub execute: ToolFunc,        // Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output=String>>>>
    pub category: String,
    pub cleanup: Option<CleanupFunc>,
}
```

Tools are registered in a `ToolRegistry` (HashMap by name). The runner dispatches tool calls by name. Tools do not share mutable state -- all context flows through the JSON parameters.

Categories control capability-based access: a BRC-52 certificate can restrict an agent to specific categories (e.g., `"tools,memory"` but not `"wallet,x402"`).

### WormLoop

The `WormLoop` struct owns all per-task state:

- `config` -- loaded `WormConfig`
- `state` -- `LoopState` (iteration count, sats spent, result, error, task ID)
- `tools` -- `Arc<RwLock<ToolRegistry>>`
- `wallet` -- `WalletClient`
- `auth` -- `AuthriteClient`
- `messagebox` -- `MessageBoxClient`
- `transcript` -- `Transcript`
- `budget` -- `BudgetTracker`
- `memory_store` / `memory_index` -- persistent memory
- `skill_registry` -- loaded skills
- `context_manager` -- token tracking and history truncation
- `detector` -- loop detection circuit breaker

### Skill System

Skills are `SKILL.md` files with YAML frontmatter:

```yaml
---
name: x402
description: Discover and use any x402 paid service
auto_activate: true
tools: [discover_services, discover_endpoints, x402_call]
---
# x402 Service Discovery

Instructions for the agent on how to discover and call x402 services...
```

Skills marked `auto_activate: true` are injected into every system prompt. Others are activated on demand. The `SkillRegistry` loads skills from the `skills/` directory at startup. Each skill can have a `config.json` and a persistent data directory under `working/skills/{name}/`.

### Budget Tracker

Multi-tier spending enforcement:

| Limit | Default | Env Override |
|-------|---------|-------------|
| Per-task | 20M sats | `DOLPHIN_MILK_BUDGET_MAX_PER_TASK` |
| Per-hour | 50M sats | `DOLPHIN_MILK_BUDGET_MAX_PER_HOUR` |
| Per-day | 200M sats | `DOLPHIN_MILK_BUDGET_MAX_PER_DAY` |
| Per-week | 100M sats | `DOLPHIN_MILK_BUDGET_WEEKLY_SATS` |
| Per-month | 500M sats | `DOLPHIN_MILK_BUDGET_MONTHLY_SATS` |
| Staging threshold | 500K sats | `DOLPHIN_MILK_BUDGET_STAGING_THRESHOLD` |

Spending is tracked per-service (LLM, proofs, tokens, messagebox) with a JSONL audit log at `working/budget.jsonl`. The enforcement mode can be `"strict"` (blocks when limit hit) or `"advisory"` (warns but continues).

### Context Manager

The context manager (`context/manager.rs`) keeps the conversation within the LLM's token budget:

1. Estimate tokens per message (~4 chars/token heuristic)
2. When approaching the context window limit, truncate old history (keeping at least `min_recent_messages`)
3. Offload large tool results (>2000 tokens) to files on disk, replacing the message content with a file reference
4. Strip base64 data URIs and cap individual messages at 8000 characters

Configurable via `context_window`, `max_history_turns`, and `min_recent_messages` in `dolphin-milk.toml`.

## Extension Points

### Adding a New Tool

1. Create a function that returns `Vec<ToolDef>` in a new file under `src/tools/` (or add to an existing category file).

2. Each tool needs a name, description, JSON Schema parameters, category, and an async execute closure:

```rust
pub fn all_my_tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "my_tool".to_string(),
        description: "Does something useful".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "input": { "type": "string", "description": "The input" }
            },
            "required": ["input"]
        }),
        execute: Box::new(|params| Box::pin(async move {
            let input = params["input"].as_str().unwrap_or("");
            format!("Result: {input}")
        })),
        category: "my_category".to_string(),
        cleanup: None,
    }]
}
```

3. Register the tools in `runner/mod.rs` inside `create_loop()` and in `mcp/server.rs` for MCP exposure.

4. If the tool should be always-on in the system prompt, add it to `ALWAYS_ON_TOOLS` in `tools/registry.rs`. Otherwise it will be discoverable via `search_tools`.

5. Add a test file at `tests/test_my_tools.rs`.

### Adding a New Skill

1. Create a directory `skills/my-skill/` with a `SKILL.md` file.

2. Write YAML frontmatter specifying `name`, `description`, `auto_activate`, and `tools` (tool names the skill references).

3. Write markdown instructions that guide the agent's behavior when the skill is active.

4. Optionally add a `config.json` for read-only skill configuration.

5. The skill is automatically loaded by `SkillRegistry` at startup. If `auto_activate: true`, it appears in every system prompt.

### Adding a New LLM Provider

1. In `think.rs`, add a new endpoint constant and update `resolve_endpoint()` to route the model name to the new URL.

2. If the new provider has a different request/response format, add normalization logic at the `think()` boundary. The rest of the codebase expects OpenAI-style tool_calls format.

3. Update `alternate_endpoint()` for circuit breaker failover.

4. Add the provider to the x402 service directory in the README if it uses x402 payment.

### Adding a New HTTP API Route

1. Create a handler function in the appropriate file under `src/server/handlers/` (or create a new handler file).

2. Add the route in `server/app_state.rs` inside `build_router()`.

3. For BRC-31 protected endpoints, wrap the handler with the auth middleware from `server/auth.rs`.

4. Add integration tests in `tests/test_server.rs`.

## Data Flow Diagram: x402 Payment

```
think()
  │
  ▼
AuthriteClient::send_request(url, body)
  │
  ├── 200 OK ──▶ parse response, return ThinkResult
  │
  ├── 402 Payment Required
  │     │
  │     ▼
  │   Parse headers: x-bsv-payment-satoshis-required,
  │                   x-bsv-payment-derivation-prefix
  │     │
  │     ▼
  │   wallet.getPublicKey(BRC-42 derivation)
  │     │
  │     ▼
  │   Build P2PKH locking script from derived pubkey
  │     │
  │     ▼
  │   wallet.createAction(outputs, description)
  │     │  Returns: AtomicBEEF (tx + merkle proofs)
  │     ▼
  │   Package payment JSON: { derivationPrefix, derivationSuffix,
  │                           transaction (base64 BEEF), satoshis }
  │     │
  │     ├── < 8KB ──▶ x-bsv-payment header
  │     ├── >= 8KB ──▶ multipart/form-data body (BRC-105)
  │     │
  │     ▼
  │   Retry request with payment attached
  │     │
  │     ├── 200 OK ──▶ parse refund headers, internalize excess, return
  │     ├── 402 again ──▶ retry (up to 3 attempts)
  │     └── other ──▶ error
  │
  └── other status ──▶ error
```

## On-Chain Proof Chain

Each task creates a linked chain of BRC-18 proofs:

```
TaskCommitment (BRC-48)
    │
    ▼
Decision Proof (iter 1)  ──▶  Decision Proof (iter 2)  ──▶  ...
  hash = SHA-256(              hash = SHA-256(
    0x00..00 ||                  prev_hash ||
    data ||                      data ||
    timestamp)                   timestamp)
    │                                                          │
    ▼                                                          ▼
TaskCompletion Proof                              BudgetSnapshot Proof
    │
    ▼
Checkpoint (BRC-48)
```

Each proof's hash field chains to the previous one, creating a tamper-evident audit trail. The `data` field is enriched with: model name, satoshis paid, payment txid, and memory IDs that influenced the prompt.

## Runtime Directory Structure

```
working/
  budget.jsonl                    # Global spending audit log
  tasks/
    {task_id}/
      transcript.jsonl            # Per-task event log (18 event types)
      receipts/                   # BEEF payment receipts per iteration
      memory/                     # Task-specific memory files
  conversations/
    {conversation_id}/
      meta.json                   # Conversation metadata
      messages.jsonl              # BRC-60 hash-chain linked messages
  schedules/
    {schedule_id}.json            # Recurring schedule definitions
  skills/
    {skill_name}/                 # Per-skill persistent data directory
  continuations/                  # Paused task state for resume
```

## Technology Choices

| Choice | Rationale |
|--------|-----------|
| Single binary | No runtime dependencies beyond `bsv-wallet-cli`. Simple deployment. |
| Axum 0.8 | Async, tower middleware ecosystem, WebSocket support. |
| Tantivy | Pure-Rust full-text search. Index is ephemeral (rebuilt from files). No external search server. |
| chromiumoxide | Async/tokio-native CDP client. Accessibility tree support for snapshot-ref-act browser automation. |
| JSONL transcripts | Append-only, crash-safe, human-readable. No database for the agent itself. |
| File-based memory | Plain markdown files are the source of truth. Inspectable, greppable, version-controllable. |
| rmcp | Rust MCP implementation for server and client. stdio transport. |
| BRC-31 over API keys | Mutual authentication with cryptographic identity. No shared secrets to leak. |
