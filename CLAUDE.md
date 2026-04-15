# Dolphin Milk (rust-bsv-worm)

Autonomous AI agent that pays for its own LLM inference via BSV micropayments (x402).

The agent runs a loop -- **OBSERVE, THINK, ACT, RECORD, BUDGET CHECK** -- where every LLM call is an authenticated HTTP request paid for with satoshis. Every action is recorded as an on-chain proof, creating a verifiable audit trail. The agent operates under real economic pressure: every thought costs money.

## Quick Start

### Prerequisites

- **Rust 1.85+** (2021 edition)
- **bsv-wallet-cli** running on `localhost:3322` with a funded BSV wallet

### Build and Run

```bash
# Build (default: embedded wallet + embedded UI)
cargo build

# Check wallet connectivity
cargo run -- status

# Single LLM call via x402 (costs a few hundred sats)
cargo run -- think "What is the capital of France?"

# Start agent with a task
cargo run -- run "Research BSV transaction fees and summarize"

# Start HTTP daemon (web UI at http://localhost:8080/ui/)
cargo run -- serve --port 8080

# Start MCP server for Claude Code / Codex integration
cargo run -- mcp
```

### Docker

```bash
docker compose up --build      # Build and start
open http://localhost:8080/ui/  # Open the web UI
```

The container connects to `bsv-wallet-cli` on the host via `host.docker.internal:3322`.

### Configuration

Copy `dolphin-milk.toml.example` to `dolphin-milk.toml` and adjust settings. Environment variables (`DOLPHIN_MILK_*`) override the config file. Key settings:

| Setting | Env | Default | Purpose |
|---------|-----|---------|---------|
| `wallet.url` | `DOLPHIN_MILK_WALLET_URL` | `http://localhost:3322` | Wallet API endpoint |
| `llm.default_model` | `DOLPHIN_MILK_LLM_MODEL` | `gpt-5-mini` | LLM model for inference |
| `llm.default_provider` | `DOLPHIN_MILK_LLM_PROVIDER` | `openai-agent` | LLM provider (`openai-agent` or `claude-chat`) |
| `budget.max_per_task` | `DOLPHIN_MILK_BUDGET_MAX_PER_TASK` | `20000000` | Max sats per task |
| `logging.level` | `DOLPHIN_MILK_LOG_LEVEL` | `INFO` | Log level |
| `heartbeat.enabled` | `DOLPHIN_MILK_HEARTBEAT_ENABLED` | `true` | Enable background scheduler |

See `dolphin-milk.toml.example` for all 60+ configurable options.

## Architecture Overview

```
User/API ──▶ HTTP Server (~79 routes) ──▶ Task Spawner ──▶ Agent Loop (DmLoop)
                                                              │
                              ┌────────────────────────┬──────┴──────┬──────────────┐
                              ▼                        ▼             ▼              ▼
                          Think (LLM)            Tools (38)    On-chain         Memory
                              │                                (proofs,        (tantivy
                              ▼                                 state,          BM25)
                          x402 Payment                         budget)
                              │
                              ▼
                          BRC-31 Auth
                              │
                              ▼
                      bsv-wallet-cli (:3322)
```

**Key boundaries:**
- `wallet/` is the only module that talks to the wallet API (HTTP or embedded). Everything else goes through it.
- `think/` normalizes all LLM responses to a common format (OpenAI + Claude providers). The rest of the codebase is provider-agnostic.
- Tools are stateless closures registered in `tools/registry.rs`. The runner dispatches by name.
- Memory files on disk are the source of truth; the tantivy search index is rebuilt from them at startup.

For a deep dive into internals, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Project Layout

Cargo workspace with 4 members: `dolphin-milk` (root), `bsv-x402-server`, `bsv-x402-llm-bridge`, `examples/x402-service`.

```
src/                        # 158 Rust source files across 26 domain modules
  runner/                   # Agent loop (lifecycle, step, text extraction, escalation, approval)
  think/                    # LLM inference via x402 (middleware pipeline, OpenAI + Claude providers)
  wallet/                   # BRC-100 wallet client (HTTP + embedded backends)
  error.rs                  # DmError enum (10 variants)
  auth/                     # BRC-31 Authrite (client + server)
  x402/                     # Payment flow, discovery, cache, circuit breaker
  onchain/                  # Budget tracker, BRC-18 proofs, BRC-48 state tokens
  session/                  # Conversations, SSE events, JSONL transcripts
  memory/                   # Markdown+YAML store, tantivy BM25 search, encryption
  tools/                    # 38 tools (15 always-on, 23 discoverable) across 14 categories
  server/                   # Axum HTTP API (~79 routes, 19 handler modules)
  config/                   # TOML config + env overrides + hot-reload
  context/                  # System prompt builder + token-aware history management
  skills/                   # Skill loader and registry
  heartbeat/                # Priority scheduler with adaptive tick
  mcp/                      # MCP server + client (rmcp)
  messagebox/               # BRC-33 cross-agent messaging
  orchestration/            # Sub-agent spawning, worktree isolation, budget delegation
  hooks/                    # Event-driven hook system (config, events, executor)
  security/                 # Command analysis and security scanning
  analytics/                # Cost analytics, efficiency metrics, ROI reporting
  audit/                    # Audit trail, key linkage revelation
  certificates/             # BRC-52 agent authorization and revocation
  eval/                     # Evaluation framework (trajectory, grading, regression)
  loop_detect/              # Agent loop detection
  replay/                   # Task replay and fork
  templates/                # Task templates (10 built-in archetypes)
  moderation.rs             # Cert-driven content moderation
  delivery.rs               # Message delivery with retry queue
  sanitize.rs               # 5-layer prompt injection defense
  logging.rs                # Log redaction (scrubs keys, tokens, BEEF)
  discovery.rs              # BRC-56 peer discovery
  metrics.rs                # Prometheus metrics (7 metric types)
  time_estimate.rs          # Human-equivalent time-saved estimation for ROI
  banner.rs                 # Startup banner display
  cli.rs                    # CLI argument parsing
  types.rs                  # Newtype IDs: TaskId, SessionId, ConversationId, ProofTxid
  lib.rs                    # Module re-exports
skills/                     # 8 SKILL.md files loaded at runtime
  x402/                     # x402 payment decision tree (auto-activated)
  wallet/                   # Wallet operations skill (auto-activated)
  messaging/                # Cross-agent messaging (auto-activated, conditional on inbox)
  browser/                  # Headless Chrome via CDP
  code-analysis/            # Code analysis
  image-gen/                # Image generation recipes
  fleet/                    # Multi-agent fleet management
  verification/             # Output verification
tests/                      # 87 test files (~3200 tests) across 19 domain subdirectories
  integration/              # 80 Playwright E2E scenarios (real BSV payments)
  multi-worm/               # 22 multi-agent orchestration scenarios (Node.js)
ui/                         # Lit-based web frontend (62 TypeScript files, Vite + Lit)
bsv-worm-sdk/              # Rust SDK for building plugins (standalone crate, not in workspace)
bsv-x402-llm-bridge/       # x402 LLM bridge library (workspace member)
bsv-x402-server/           # x402 server components (workspace member)
examples/                   # Example configurations and usage (x402-service is a workspace member)
templates/                  # Task and project templates
docs/                       # Architecture docs, execution plans
marketing/                  # Marketing materials
regulatory/                 # Regulatory analysis documents
```

## Testing

```bash
# Run all tests (~3200 across 87 files)
cargo test

# Run a specific test file
cargo test --test test_wallet

# Run tests with output
cargo test -- --nocapture

# Lint (must be warning-free)
cargo clippy -- -D warnings
```

**Test conventions:**
- One `tests/test_*.rs` file per module, organized into 19 domain subdirectories
- Use `tempfile` for filesystem tests, `mockito` for HTTP mocking
- Valid secp256k1 public keys required in tests (use the generator point G), never fake hex strings
- Tests do not require a running wallet -- HTTP calls are mocked

### E2E Integration Tests (Playwright)

Real-BSV tests that send messages through the live UI and pay real sats. **Run these after any feature that affects agent behavior.**

```bash
cd tests/integration && npm install  # first time only

# Start server first: cargo run --release -- serve --port 8080

node run.js --canary          # Health check (~$0.01)
node run.js --tier trivial    # Quick validation (~$0.03)
node run.js --id 13,20,25,27  # Introspection scenarios
node run.js                   # Full 80-scenario suite (~$0.30-0.50)
node test-ui-views.js         # UI view validation (free)
```

See `tests/integration/CLAUDE.md` for full documentation, scenario schema, and shadow DOM scraping details.

### Multi-Worm Integration Tests

Multi-agent orchestration scenarios testing cross-agent communication (BRC-33/77/78). Requires two or more agent instances running.

```bash
cd tests/multi-worm && npm install  # first time only

node run-scenarios.js                  # Run all 22 scenarios
node orchestrate.js --scenario 1       # Run a specific scenario
```

See `tests/multi-worm/scenarios.json` for the full scenario catalog.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contribution guide. The short version:

1. Fork and create a feature branch
2. Write your code and add tests
3. `cargo test` -- all tests pass
4. `cargo clippy -- -D warnings` -- no warnings
5. Open a pull request with a clear description

### Code Conventions

- **Errors:** Use `DmError` variants via `thiserror`. Match broadly or narrowly.
- **Tools:** `ToolDef { name, description, parameters, execute, category }` with stateless async closures. No shared mutable state.
- **Config:** `dolphin-milk.toml` sections with `DOLPHIN_MILK_*` env overrides. Add new settings to `config/schema.rs`.
- **Skills:** `SKILL.md` with YAML frontmatter in `skills/<name>/SKILL.md`.
- **Transcripts:** Append-only JSONL. 18 event types. Never mutate past entries.
- **Protected paths:** The agent cannot edit `dolphin-milk.toml`, `Cargo.toml`, `Cargo.lock`. Enforced in `tools/sandbox.rs`.
- **IDs:** Use newtype wrappers (`TaskId`, `SessionId`, etc.) from `types.rs`, not raw strings.
- **Feature flags:** `embed-ui` (embeds frontend via rust-embed), `embedded-wallet` (in-process wallet via bsv-wallet-toolbox-rs). Both default-enabled.
- **BSV SDK:** Git dep from `github.com/Calhooon/bsv-rs`. Local dev override: `.cargo/config.toml` (gitignored, template at `.cargo/config.toml.dev`).

### Adding a Tool

Create a `ToolDef` with name, description, JSON schema, category, and an async closure. Register it in `runner/mod.rs` and `mcp/server.rs`. Add it to `ALWAYS_ON_TOOLS` if it should be in every prompt, otherwise it is discoverable via `search_tools`. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for details.

### Adding a Skill

Create `skills/my-skill/SKILL.md` with YAML frontmatter (`name`, `description`, `auto_activate`, `tools`) and markdown instructions. It will be loaded automatically at startup.

## CLI Reference

```
dolphin-milk run [TASK] [--max-iterations N]    # Run agent loop
dolphin-milk status                              # Check wallet connectivity + balance
dolphin-milk think MESSAGE [--model MODEL]       # Single LLM call via x402
dolphin-milk receive [--suffix N]                # Generate a BSV receive address
dolphin-milk fund TXID [--vout N] [--suffix N]   # Internalize funding from on-chain tx
dolphin-milk serve --port PORT [--workspace DIR] # Start HTTP daemon + scheduler
dolphin-milk mcp                                 # Start MCP server (stdio transport)
dolphin-milk audit                               # BRC-69 key linkage revelations
dolphin-milk verify-work                         # Offline custody proof verification
```

## HTTP API

~79 routes served by the Axum HTTP daemon. BRC-31 Authrite auth required on `/chat`, `/budget`, `/chat/history`, `/heartbeat/trigger`, and audit key-linkage endpoints.

Key endpoints:

| Method | Path | Description |
|--------|------|-------------|
| POST | /chat | Authenticated chat message (BRC-31) |
| POST | /task | Submit a task, returns task ID |
| GET | /task/{id}/events | Poll-based event streaming |
| GET | /task/{id}/audit | Full audit chain with proofs and BEEF refs |
| GET | /task/{id}/proofs/verify | Verify proof hashes against on-chain data |
| GET | /task/{id}/replay | Structured replay timeline with cost/tool usage |
| POST | /task/{id}/fork | Fork execution from event index into new task |
| GET | /task/{id}/escalation | Get escalation status for a task |
| GET | /agent | Agent identity, balance, uptime, tool registry |
| GET | /status | All tasks + budget summary |
| GET | /budget/detail | Detailed budget breakdown with 5-tier limits |
| GET | /conversations | Multi-turn conversation list |
| GET | /memory/search | BM25 memory search |
| GET | /metrics | Prometheus metrics endpoint |
| GET | /analytics/efficiency | Efficiency metrics from transcript data |
| GET | /analytics/roi | ROI report for current session |
| GET | /rates/bsv-usd | Multi-source exchange rate |
| GET | /health | Liveness check (version, uptime) |

The full route table is documented in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [src/server/CLAUDE.md](src/server/CLAUDE.md). The web UI is served at `/ui/`.

## BSV Protocol Standards

Dolphin Milk implements 14 BRC (Bitcoin Request for Comments) standards:

| BRC | Standard | Used For |
|-----|----------|----------|
| BRC-18 | OP_RETURN proofs | On-chain audit trail (~200 sats/proof) |
| BRC-29 | Payment key derivation | x402 payment construction |
| BRC-31 | Authrite mutual auth | API authentication, LLM payment auth |
| BRC-33 | MessageBox relay | Cross-agent messaging |
| BRC-42 | Key derivation | Wallet-native key derivation via HMAC-SHA256 |
| BRC-46 | Output baskets | UTXO organization (state, budget, proofs) |
| BRC-48 | PushDrop tokens | Task lifecycle state management |
| BRC-52 | Agent certificates | Parent-child authorization and capability delegation |
| BRC-56 | Peer discovery | Agent-to-agent service discovery |
| BRC-60 | Hash-chain messages | Conversation integrity verification |
| BRC-77 | Message signing | ECDSA signatures for cross-agent messages |
| BRC-78 | Message encryption | Wallet-native message encryption |
| BRC-100 | Wallet API | 28-endpoint wallet interface |
| BRC-105 | Multipart transport | Large payment transport (>8KB) |

## License

Apache-2.0. See [LICENSE](LICENSE).
