# Dolphin Milk

**The AI agent that answers to you. And can prove it.**

An open-source AI agent with its own wallet, no API keys, and a cryptographic receipt for every action it takes. Written in Rust. Single binary. You own it.

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![BSV](https://img.shields.io/badge/payments-BSV%20x402-2ec4b6.svg)](https://x402.org)

---

## Why Dolphin Milk?

Every AI agent today depends on API keys — permission slips from corporations. Revoke the key and the agent is brain-dead. Ask what the agent actually did and you get a log file. Maybe.

Dolphin Milk is architecturally different:

- **No API keys.** The agent pays for LLM inference over [x402](https://x402.org) — HTTP's payment layer. It discovers endpoints, pays from its own wallet, gets the response. No accounts. No signup. No vendor relationship.

- **On-chain proofs.** Every action — every tool call, every inference, every decision — produces a cryptographic proof on the blockchain. Not logs. Not traces. Proof. Timestamped. Signed. Verifiable by anyone.

- **Sovereignty.** Single Rust binary. Embedded wallet. Runs on your laptop. No Docker required. No cloud. No subscription. Download it. Fund the wallet. It's yours.

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/calhooon/dolphinmilk/main/install.sh | sh
```

One line. macOS or Linux. No Rust, no Docker, no API keys. Drops a single self-contained binary into `~/.local/bin/dolphin-milk`. The wallet, the web UI, and the agent all live inside that one binary.

Building from source instead? `cargo install --git https://github.com/calhooon/dolphinmilk` (requires Rust 1.85+).

## First Run

```bash
dolphin-milk serve
```

That's it. The agent boots, the wallet boots, the web UI boots — one command, one process.

Open **http://localhost:8080/ui/**. The UI shows your agent's funding address. Send a few thousand sats of BSV. The agent auto-detects the deposit, then you start chatting. Every message you send triggers a paid LLM call (~200 sats), creates an on-chain proof, and you can see the receipt scroll past in real time.

**Install → serve → fund → chat.** Four steps. No accounts, no signup, no config files to edit.

### Prefer the CLI?

```bash
dolphin-milk init                                   # create wallet, print funding address
# Send BSV to the address it prints
dolphin-milk fund <TXID>                            # internalize the funding tx
dolphin-milk think "What is the capital of France?" # one-shot paid LLM call
dolphin-milk run "Research BSV transaction fees"    # autonomous agent loop
```

### Wallet

The wallet runs **in-process**. Nothing else to install or run. Your wallet database lives at `~/.dolphin-milk/wallet.db`. Advanced users running their own `bsv-wallet-cli` can point dolphin-milk at it via `DOLPHIN_MILK_WALLET_URL` — but you don't need to.

### ⚠ Back Up Your Wallet

This is a real wallet holding real money. **Two files contain everything needed to recover your funds. Lose them and your BSV is gone forever.**

```
~/.dolphin-milk/.env          # ROOT_KEY — 32-byte master key
~/.dolphin-milk/wallet.db     # encrypted UTXO + history state
```

Both files together = full wallet. Copy them somewhere safe — a USB drive, an encrypted backup, a password manager — **before** you fund the wallet. There is no "forgot password" flow. There is no recovery service. You hold the keys.

The agent cares about your sovereignty. Sovereignty means custody. Custody means backups.

---

## How It Works

```
You ──> dolphin-milk start ──> Agent Loop
                                    |
                    +-------+-------+-------+--------+
                    v       v       v       v        v
                 Think    Tools   Proofs  Memory  Budget
                   |      (42)   (BRC-18) (BM25)  Check
                   v
              x402 Payment ──> Any LLM
                   |
                   v
              Own Wallet
            (no API keys)
```

The agent runs a loop: **OBSERVE > THINK > ACT > RECORD > BUDGET CHECK**

1. **OBSERVE** — Reads the task, checks memory, reviews context
2. **THINK** — Calls an LLM via x402 micropayment (no API key, just satoshis)
3. **ACT** — Executes tool calls (browse, search, analyze, message other agents)
4. **RECORD** — Writes a BRC-18 proof to the blockchain (cryptographic receipt)
5. **BUDGET CHECK** — Reviews spend vs. budget, decides whether to continue

Every step is paid for, recorded, and verifiable.

---

## Features

### No API Keys — x402 Protocol

The agent uses [x402](https://x402.org) to pay for services over HTTP. No API keys. No accounts. No billing dashboards. The agent discovers an endpoint, gets a price, pays from its own wallet, gets the response.

```
Agent ──> GET /.well-known/x402-info ──> discovers price
Agent ──> POST /chat (with payment)  ──> gets response
```

Works with any x402-compatible provider. Currently supports OpenAI and Claude inference endpoints.

### On-Chain Audit Trail

Every action produces a BRC-18 OP_RETURN proof containing:
- What the agent did (hashed)
- What model it used
- What it cost
- When it happened
- A chain link to the previous proof

Proofs are hash-chained across iterations. Tamper with one and the chain breaks. Verify any proof against the blockchain independently.

### 42 Tools, 15 Categories

The agent ships with a complete toolbox out of the box. **17 tools are always loaded** in the system prompt — the essentials for filesystem, memory, wallet, and HTTP. The other **25 are discoverable on demand** via `search_tools`, so the prompt stays small while the toolbox stays large.

| Category | What it does |
|---|---|
| **Sandbox** | `execute_bash`, `file_read`, `file_write`, `file_search`, `web_fetch` — the core unix toolkit, sandboxed |
| **System** | `read_tool_output`, `continue_task` — recover truncated output, schedule resumption |
| **Wallet** | `wallet_balance`, `wallet_identity`, `wallet_call`, `receive_address`, `fund_from_tx`, encrypt/decrypt — full BRC-100 surface |
| **Memory** | `memory_store`, `memory_search` — persistent agent memory with BM25 search |
| **x402** | `discover_services`, `discover_endpoints`, `x402_call`, `generate_image`, `upload_to_nanostore` — pay any HTTP endpoint that speaks x402 |
| **Browser** | `browser` — headless Chrome via CDP with snapshot/click/type/evaluate |
| **MessageBox** | `send_message`, `check_inbox`, `set_permission` — agent-to-agent messaging with BRC-77 signing + BRC-78 encryption |
| **Schedule** | `create_schedule`, `list_schedules`, `cancel_schedule` — interval, cron, or one-shot |
| **Introspect** | `introspect` — query your own proofs, costs, and task history from on-chain state |
| **Orchestration** | `spawn_agent`, `check_agent`, `list_agents`, `kill_agent` — sub-agents with carved-out budgets |
| **Delegation** | `delegate_task` — commission work to other agents with scoped BRC-52 certificates |
| **Discovery** | `discover_agent`, `verify_agent` — find and authenticate other agents (BRC-56) |
| **Conversation** | `list_conversations`, `read_conversation` — self-introspect dialogue history |
| **Analytics** | `cost_analysis` — efficiency, ROI, model-comparison replay |
| **Fleet** | `fleet_status` — multi-agent coordination |
| **Verification** | `verify_output` — structural consistency checks on prior tool results |

### Multi-Agent Communication

Agents discover each other (BRC-56), authenticate (BRC-31), sign messages (BRC-77), encrypt end-to-end (BRC-78), and relay through MessageBox (BRC-33). Spawn sub-agents with delegated authority via BRC-52 certificates.

### Web UI

Lit-based frontend served at `/ui/` with chat, dashboard, budget tracking, proof chain visualization, memory browser, and agent identity views.

### MCP Server

```bash
dolphin-milk mcp
```

Exposes agent tools via Model Context Protocol (stdio transport). Claude Code and Codex can use Dolphin Milk as a tool provider.

---

## Configuration

**Skip this section.** The defaults are designed to just work — embedded wallet, GPT-5 Mini for inference, 20M sat per-task budget cap, web UI on port 8080. Run `dolphin-milk serve` and you're done.

If you want to tune something, drop a `dolphin-milk.toml` next to your data directory or pass `DOLPHIN_MILK_*` environment variables. The most-tuned knobs:

| Setting | Env var | Default | What it does |
|---|---|---|---|
| `llm.default_model` | `DOLPHIN_MILK_LLM_MODEL` | `gpt-5-mini` | Which LLM to call. Any x402-served model works. |
| `llm.default_provider` | `DOLPHIN_MILK_LLM_PROVIDER` | `openai-agent` | `openai-agent` or `claude-chat`. |
| `budget.max_per_task` | `DOLPHIN_MILK_BUDGET_MAX_PER_TASK` | `20000000` | Hard cap in satoshis per task (~$3 at current prices). |
| `budget.max_per_day` | `DOLPHIN_MILK_BUDGET_MAX_PER_DAY` | `200000000` | Daily spend ceiling. |
| `heartbeat.enabled` | `DOLPHIN_MILK_HEARTBEAT_ENABLED` | `true` | Background scheduler (cron, schedules, inbox polling). |
| `wallet.url` | `DOLPHIN_MILK_WALLET_URL` | _(embedded)_ | Only set this to point at an external `bsv-wallet-cli` instance. |

70+ tunable settings in total — see `dolphin-milk.toml.example` for the full reference (active hours, certificate policies, rate limits, moderation, x402 routing, hot-reload behavior, and more).

---

## x402 Service Directory

All services use x402 payment. Discovery via `/.well-known/x402-info` manifests.

| Service | Endpoint | Cost |
|---------|----------|------|
| LLM (OpenAI models) | openai-chat.x402agency.com | ~200 sats/call |
| LLM (Claude models) | claude-chat.x402agency.com | ~200 sats/call |
| Image Generation | nano-banana-pro.x402agency.com | ~$0.19/image |
| Video Generation | veo-3-1-fast.x402agency.com | ~$0.19/sec |
| Transcription | whisper-large-v3-turbo.x402agency.com | ~$0.0006/min |
| X/Twitter Search | x-research.x402agency.com | ~$0.05/page |
| Storage | nanostore.babbage.systems | ~730 sats/MB/yr |

---

## Protocol Standards

Dolphin Milk implements 14 BSV Request for Comments (BRC) standards:

| BRC | Standard | Purpose |
|-----|----------|---------|
| 18 | OP_RETURN proofs | On-chain audit trail |
| 29 | Payment key derivation | x402 payment construction |
| 31 | Authrite | Mutual authentication |
| 33 | MessageBox | Cross-agent messaging relay |
| 42 | Key derivation | HMAC-SHA256 key derivation |
| 46 | Output baskets | UTXO organization |
| 48 | PushDrop tokens | Task lifecycle state |
| 52 | Agent certificates | Authorization + delegation |
| 56 | Peer discovery | Agent-to-agent discovery |
| 60 | Hash-chain messages | Conversation integrity |
| 77 | Message signing | ECDSA message signatures |
| 78 | Message encryption | End-to-end encryption |
| 100 | Wallet API | 28-endpoint wallet interface |
| 105 | Multipart transport | Large payment transport |

---

## Project Structure

```
src/                        # 156 Rust source files, 26 modules
  runner/                   # Agent loop (lifecycle, step, escalation)
  think/                    # LLM inference via x402 (OpenAI + Claude)
  wallet/                   # Wallet client (HTTP + embedded backends)
  tools/                    # 38 tools across 14 categories
  server/                   # Axum HTTP API (69 routes)
  context/                  # System prompt builder + context management
  onchain/                  # Budget, proofs, state tokens
  memory/                   # Markdown store + BM25 search
  auth/                     # BRC-31 Authrite
  x402/                     # Payment flow + circuit breaker
  session/                  # Conversations + transcripts
  mcp/                      # MCP server + client
  certificates/             # BRC-52 authorization
  heartbeat/                # Adaptive scheduler
  sanitize.rs               # 5-layer injection defense
  logging.rs                # Log redaction
skills/                     # 8 runtime skill definitions
tests/                      # ~3200 Rust tests across 87 files
  integration/              # 80 Playwright E2E scenarios (real BSV payments)
  multi-worm/               # multi-agent orchestration scenarios
ui/                         # Lit web frontend (50 TypeScript files)
```

---

## CLI Reference

```
dolphin-milk init                        # First-run setup: wallet, identity, funding address
dolphin-milk start [--port PORT]         # Start daemon + web UI (alias: serve)
dolphin-milk think MESSAGE [--model M]   # Single paid LLM call via x402
dolphin-milk run TASK [--max-iterations] # Autonomous agent loop
dolphin-milk status                      # Wallet connectivity + balance
dolphin-milk receive [--suffix N]        # Generate funding address
dolphin-milk fund TXID [--vout N]        # Internalize external funding
dolphin-milk split COUNT                 # Split balance into N equal UTXOs (parallel-agent funding)
dolphin-milk mcp                         # MCP server (stdio)
dolphin-milk audit                       # BRC-69 key linkage revelations
dolphin-milk verify-work                 # Offline custody proof verification
```

---

## Testing

```bash
# All ~3200 tests
cargo test

# Lint (must be warning-free)
cargo clippy -- -D warnings

# E2E tests (requires running server + funded wallet)
cd tests/integration && npm install
node run.js --canary           # Health check (~$0.01)
node run.js --tier trivial     # Quick pass (~$0.03)
node run.js                    # Full 80-scenario suite (~$0.30-0.50)
```

Tests don't require a running wallet — HTTP calls are mocked with `mockito`, filesystem tests use `tempfile`.

---

## Contributing

1. Fork and create a feature branch
2. Write code and add tests
3. `cargo test` — all tests pass
4. `cargo clippy -- -D warnings` — no warnings
5. Open a PR

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.

---

## License

[Apache License 2.0](LICENSE)
