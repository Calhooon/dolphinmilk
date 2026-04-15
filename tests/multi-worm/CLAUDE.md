# multi-worm

> Multi-agent orchestration test suite: spawns multiple Dolphin Milk instances, discovers identities via BRC-31, and executes cross-agent scenarios with assertions.

## Overview

This directory contains a Node.js test harness for multi-agent scenarios. It spawns real `dolphin-milk` server processes (one per agent), performs BRC-31 Authrite handshakes to discover identities, issues BRC-52 certificates, and runs step-by-step scenarios that test cross-agent communication, encryption, signing, payments, and security boundaries. Every HTTP request is authenticated via BRC-31 using the parent wallet (MetaNet Client on port 3321). Results are saved as timestamped JSON files in `results/`.

Unlike the Playwright E2E tests in `tests/integration/` which drive the web UI, these tests interact directly with the HTTP API -- no browser required.

## Files

| File | Lines | Purpose |
|------|------:|---------|
| orchestrate.js | 551 | `WormOrchestrator` class: spawns agents, health-checks, discovers identities, issues BRC-52 certs, template variable resolution |
| run-scenarios.js | 1042 | Scenario runner: step execution (task/wait/assert/api), assertion checking (11 types), BRC-31 auth wrappers, results persistence, CLI |
| grade.js | ~350 | Programmatic transcript grader: 10 criteria evaluated by parsing events. 7 fully automatic, 3 soft criteria flagged for Claude Code review. |
| report.js | ~250 | Report generator: markdown reports, regression detection against baselines, needs-review section for quality loop |
| scenarios.json | 992 | 16 test scenarios across 5 tiers with steps, assertions, and budget constraints |
| schema.json | 237 | JSON Schema (draft 2020-12) defining the scenario file format |
| config.json | 23 | Agent configuration: ports, wallets, workspaces for alice/bob/charlie |
| package.json | 22 | npm scripts for running tests, validation, grading, reporting |
| .gitignore | 2 | Excludes `node_modules/` and `results/` |
| lib/auth.js | 345 | BRC-31/104 authentication client: varint encoding, request serialization, wallet signing, session management |
| lib/validate.js | 479 | Scenario file validator: checks structure, types, enums, duplicates without external dependencies |
| TEST_SPEC_3_LAYER_CASCADE.md | 220 | Spec for 3-layer cascade (Captain → Coordinator → Worker) + NanoStore data sharing E2E tests. Implementation tracked separately; depends on worker wallet `:3324` (provisioned 2026-04-13). |
| test_two_agent_handshake.js | — | Standalone 2-agent handshake regression. Run via `node test_two_agent_handshake.js`. |
| test_captain_coral_delegation.js | — | Standalone Captain→Coral delegation E2E (EPIC #329 Phase 3). Run via `node test_captain_coral_delegation.js`. |

## Architecture

```
CLI (run-scenarios.js)
  │
  ├── Load & filter scenarios from scenarios.json
  ├── Determine required agents from scenario definitions
  ├── Create WormOrchestrator (config.json, filtered to required agents)
  │     ├── Spawn dolphin-milk processes (cargo binary)
  │     ├── Health-check via GET /health (wallet_connected required)
  │     ├── Issue BRC-52 certificates via parent wallet (if agent lacks valid cert)
  │     └── Discover identities via GET /agent (BRC-31 authed)
  │
  ├── Build template vars: {{alice.identity_key}}, {{bob.worm_url}}, etc.
  ├── Set BRC-31 auth context (parent_wallet_port from config)
  │
  ├── Execute scenarios sequentially (all requests BRC-31 authenticated)
  │     ├── Step: task  → POST /task, wait for completion via GET /task/{id}/events
  │     ├── Step: wait  → Poll GET /status for event (task_complete, message_received, etc.)
  │     ├── Step: api   → Direct GET/POST to agent endpoint
  │     └── Step: assert → Deferred inline assertion, evaluated after all steps
  │
  ├── Check assertions (inline + post-scenario, 11 assertion types)
  └── Save results to results/run-{timestamp}.json
```

### BRC-31 Authentication (lib/auth.js)

All requests to worm servers are authenticated via BRC-31 Authrite using the parent wallet (MetaNet Client). The auth flow:

1. Get parent identity key via `getPublicKey` on the wallet
2. BRC-31 handshake with the worm server (`POST /.well-known/auth`) -- sessions are cached per port
3. Generate per-request nonce and request ID (32 random bytes each)
4. Serialize request into BRC-104 binary format (varint-prefixed fields: method, path, query, headers)
5. Sign via wallet's `createSignature` with protocol `[2, "auth message signature"]`
6. Attach `x-bsv-auth-*` headers and send to worm server

**Important**: The request body is NOT included in the signature. The worm server consumes the body via axum's `Json` extractor before auth verification, so both client and server serialize with `body = null`.

## Agent Configuration (config.json)

```json
{
  "binary": "./target/release/dolphin-milk",
  "health_timeout_ms": 30000,
  "health_poll_ms": 1000,
  "parent_wallet_port": 3321,
  "agents": {
    "alice": { "worm_port": 8080, "wallet_port": 3322, "workspace": "./test-workspaces/alice" },
    "bob":   { "worm_port": 8081, "wallet_port": 3323, "workspace": "./test-workspaces/bob" },
    "charlie": { "worm_port": 8082, "wallet_port": 3323, "workspace": "./test-workspaces/charlie" }
  }
}
```

Each agent gets its own HTTP port, wallet port, and workspace directory. The `parent_wallet_port` (3321) is the MetaNet Client wallet used for BRC-31 signing. Alice and Bob use different wallet ports (3322, 3323); Bob and Charlie share a wallet (3323).

### Prerequisites

- **Rust binary**: `cargo build --release` (the orchestrator looks for `./target/release/dolphin-milk`)
- **3 wallet instances**: MetaNet Client on port 3321 (parent), bsv-wallet-cli on 3322 (alice), bsv-wallet-cli on 3323 (bob/charlie)
- **Funded wallets**: Each agent wallet needs sats for LLM inference via x402
- **@bsv/sdk**: Linked from `../../../ts-sdk` via package.json file dependency

## Scenarios (scenarios.json)

16 scenarios organized into 5 tiers:

| Tier | IDs | Scenarios | Focus |
|------|-----|----------:|-------|
| canary | 1 | 1 | Health check: both agents respond to /health and /agent, different identity keys |
| basic | 2, 3, 7, 8 | 4 | Plaintext messaging, identity exchange, turn-taking protocol, delivery reliability |
| security | 4, 10, 11, 12 | 4 | Cross-agent injection defense, BRC-77 signed messages, BRC-78 encrypted messages, sign+encrypt roundtrip |
| economic | 5, 13, 14, 16 | 4 | Paid inbox delivery, spam filter (500 sat fee), service discovery, budget enforcement |
| full | 6, 9, 15 | 3 | Encrypted task delegation, multi-hop relay (3 agents), marketplace task delegation |

### Scenario schema

Each scenario has:

| Field | Required | Description |
|-------|----------|-------------|
| `id` | yes | Unique integer identifier |
| `tier` | yes | One of: canary, basic, security, economic, full |
| `name` | yes | Snake_case identifier |
| `description` | yes | Human-readable description |
| `agents` | yes | Array of agent names (minimum 2), must match config.json |
| `steps` | yes | Ordered array of step objects |
| `assertions` | no | Post-scenario assertions evaluated after all steps |
| `budget` | no | `max_total_sats` and/or `max_per_agent_sats` constraints |
| `tags` | no | Freeform tags for filtering (e.g. `brc-77`, `messaging`) |
| `skip` | no | Boolean to skip the scenario |
| `timeout_seconds` | no | Scenario-level timeout override |
| `note` | no | Implementation notes |

### Step types

| Action | Required fields | Description |
|--------|----------------|-------------|
| `task` | `message` | Submit task via `POST /task`, poll `GET /task/{id}/events` until complete |
| `wait` | `event` | Poll `GET /status` for event: `task_complete`, `message_received`, `proof_created`, `budget_updated` |
| `api` | `path` | Direct HTTP call to agent (default GET, optional `method` and `body`) |
| `assert` | `condition` | Deferred assertion, evaluated after all steps complete |

All steps support `store_as` (snake_case key to save result for later assertions) and `timeout_seconds`.

### Assertion types

Assertions are used both inline (as `assert` steps with a `condition`) and as post-scenario `assertions`. Both inline and post-scenario assertions support the same 11 types:

| Type | Checks |
|------|--------|
| `status_code` | HTTP status equals expected value |
| `field_exists` | Nested field path is present and non-null |
| `field_equals` | Nested field equals expected value |
| `response_contains` | JSON-stringified result contains substring (case-insensitive); supports `values` array for multiple needles |
| `response_not_contains` | JSON-stringified result does NOT contain substring |
| `response_matches` | Field or full result matches regex pattern |
| `agents_different_keys` | All agents have distinct identity keys |
| `cost_within` | Total cost under max_sats (placeholder -- always passes) |
| `session_no_error` | Task's `session_end` event has no error (inspects transcript) |
| `tool_succeeded` | Named tool was called and its last result doesn't start with "error" |
| `tool_result_contains` | Named tool's last result contains a substring |
| `no_tool_errors` | No tool results in the task transcript start with "error" |

The transcript-inspection types (`session_no_error`, `tool_succeeded`, `tool_result_contains`, `no_tool_errors`) reference stored step results by `step` name and inspect the task's event stream for tool execution outcomes.

### Template variables

Step messages and API paths support `{{agent.field}}` placeholders resolved from the orchestrator:

- `{{alice.identity_key}}` -- agent's secp256k1 public key (66 hex chars)
- `{{alice.worm_url}}` -- agent's HTTP base URL (e.g. `http://localhost:8080`)
- `{{bob.identity_key}}`, `{{bob.worm_url}}`, etc.

## Usage

```bash
cd tests/multi-worm && npm install   # first time only

# Start agents and print status (stops immediately)
node orchestrate.js --status-only

# Start agents interactively (Ctrl-C to stop)
node orchestrate.js

# Start only specific agents
node orchestrate.js --agents alice,bob

# Run all non-skipped scenarios
node run-scenarios.js

# Run by tier
node run-scenarios.js --tier canary      # Health check only (free)
node run-scenarios.js --tier basic       # Simple cross-agent messaging
node run-scenarios.js --tier security    # Injection, signing, encryption

# Run specific scenarios by ID
node run-scenarios.js --id 1             # Health check
node run-scenarios.js --id 10,11,12      # BRC-77/78 crypto scenarios

# Dry run (show what would execute, no agents started)
node run-scenarios.js --dry-run

# Validate scenario file structure
npm run validate

# Grade transcripts via AI (pays sats via x402)
node grade.js                          # Grade latest result file
node grade.js --id 10,11,12            # Grade specific scenarios
node grade.js --dry-run                # Show criteria mapping only

# Generate report from graded results
node report.js                         # Report from latest graded result
node report.js --save-baseline         # Save current results as baseline
```

### npm scripts

| Script | Command |
|--------|---------|
| `npm start` | `node orchestrate.js` |
| `npm run status` | `node orchestrate.js --status-only` |
| `npm test` | `node run-scenarios.js` |
| `npm run test:canary` | `node run-scenarios.js --tier canary` |
| `npm run test:basic` | `node run-scenarios.js --tier basic` |
| `npm run test:full` | `node run-scenarios.js` (all scenarios) |
| `npm run dry-run` | `node run-scenarios.js --dry-run` |
| `npm run validate` | Validate scenarios.json against schema |
| `npm run grade` | `node grade.js` (grade latest result via x402) |
| `npm run grade:dry-run` | `node grade.js --dry-run` (show criteria mapping) |
| `npm run report` | `node report.js` (generate markdown report) |
| `npm run report:baseline` | `node report.js --save-baseline` (save baseline) |

### Cost expectations

| Tier | Approximate cost |
|------|-----------------|
| canary | Free (API calls only, no LLM inference) |
| basic | ~0.5-2M sats per scenario (LLM calls + messaging) |
| security | ~1.5-3M sats per scenario |
| economic | ~1-3M sats per scenario |
| full | ~3-5M sats per scenario (multi-step delegation) |

### Message contamination warning

Agents persist MessageBox state between scenarios within the same run. For reliable results with messaging scenarios, run them individually:

```bash
node run-scenarios.js --id 10    # Run alone
node run-scenarios.js --id 11    # Run alone
```

## Key exports

### orchestrate.js

| Export | Description |
|--------|-------------|
| `WormOrchestrator` | Class: manages agent lifecycle (start, health-check, identity, certificates, stop) |
| `httpGet(url)` | Promise-based HTTP GET with 5s timeout, auto-parses JSON |

The orchestrator also has a local `httpPost()` helper (not exported) used internally for certificate issuance and health checks.

### run-scenarios.js

| Export | Description |
|--------|-------------|
| `runScenarios({ orchestrator, scenarios, dryRun })` | Run filtered scenarios against live agents |
| `executeStep(step, vars)` | Execute a single step (task/wait/assert/api) |
| `checkAssertions(scenario, vars, stepResults)` | Evaluate inline + post-scenario assertions (11 assertion types) |
| `httpPost(url, body)` | Promise-based HTTP POST with 30s timeout |
| `waitForTaskComplete(agentUrl, taskId, timeoutSecs)` | Poll task events until `active === false` |

Internally, `run-scenarios.js` uses `authedGet()` and `authedPost()` wrappers that enforce BRC-31 authentication on every request via the parent wallet port. The port is set from the orchestrator config at the start of each run.

### lib/auth.js

| Export | Description |
|--------|-------------|
| `authGet(url, parentWalletPort)` | BRC-31 authenticated GET |
| `authPost(url, data, parentWalletPort)` | BRC-31 authenticated POST |
| `authRequest(method, url, parentWalletPort, bodyObj)` | Generic BRC-31 authenticated request |
| `clearAuthCache()` | Clear cached BRC-31 sessions |

### lib/validate.js

| Export | Description |
|--------|-------------|
| `validateScenario(scenario, index)` | Validate a single scenario object |
| `validateScenarios(data)` | Validate the full scenarios file structure |
| `validateScenariosFile(filePath)` | Load, parse, and validate a scenarios JSON file |

### grade.js

Programmatic grading — no LLM calls, no wallet needed, runs in milliseconds.

**10 criteria** (7 fully automatic, 3 soft with `needs_review` flag):

| Criterion | Method | What it checks |
|-----------|--------|----------------|
| `message_delivered` | auto | send_message tool_result indicates success |
| `message_was_encrypted` | auto | send_message called with `encrypt=true` |
| `message_was_signed` | auto | send_message called with `sign=true` |
| `reply_obligation_met` | auto | Recipient has check_inbox + send_message pattern |
| `audit_trail_complete` | auto | checkpoint_created + budget_check + session_end events |
| `budget_respected` | auto | spent_session < task_limit in budget_check events |
| `turn_taking_terminated` | auto | All sessions ended cleanly (session_end, no error) |
| `no_plaintext_in_transit` | **review** | Collects encrypted body text as evidence for Claude Code |
| `prompt_injection_blocked` | **review** | Collects agent responses + tool calls for Claude Code |
| `cert_verification_performed` | **review** | Looks for cert-related tool calls, flags if absent |

Each criterion produces `{ verdict, reasoning, evidence[], needs_review }`.

**Quality loop integration:** After `node grade.js`, Claude Code reads the graded output and evaluates `needs_review` criteria using the collected evidence. No paid LLM calls needed.

### report.js

Generates markdown + JSON from graded results. No LLM calls.

Output: `artifacts/report-{timestamp}.md` + `artifacts/demo-{timestamp}.json`.

The report includes a "Needs Review (Claude Code)" section with evidence for soft criteria, making it easy for Claude Code to evaluate during the quality loop.

## Results

Run results are saved to `results/run-{ISO-timestamp}.json` with structure:

```json
{
  "timestamp": "2026-03-30T03:43:01.372Z",
  "results": [
    {
      "pass": true,
      "scenario": { "id": 1, "name": "health_check", "tier": "canary" },
      "stepResults": [...],
      "assertionResults": [...],
      "elapsed_seconds": 4.2,
      "error": null
    }
  ],
  "passed": 1,
  "failed": 0,
  "skipped": 0,
  "elapsed_seconds": 4.2
}
```

The `results/` directory is gitignored. A `.gitkeep` file preserves the directory in the repo.

### Graded results

Graded results are saved to `results/graded-{ISO-timestamp}.json`:

```json
{
  "timestamp": "2026-04-01T13:04:01Z",
  "source_result": "run-2026-03-30T03-43-01-372Z.json",
  "scenarios": [{
    "id": 10, "name": "brc77_signed_message", "tier": "security",
    "overall_verdict": "pass", "needs_review": false,
    "criteria": [{
      "name": "message_delivered",
      "verdict": "pass",
      "reasoning": "send_message returned success",
      "evidence": ["alice: send_message OK — ..."],
      "needs_review": false
    }]
  }],
  "summary": { "total_scenarios": 3, "passed": 1, "partial": 2, "needs_review": 2 }
}
```

### Artifacts

Report artifacts are saved to `artifacts/`:
- `report-{timestamp}.md` — markdown report with tables, failed scenario details, regressions
- `demo-{timestamp}.json` — machine-readable: criteria roll-up, cost data, conversation highlights

### Baselines

`baselines.json` stores expected results for regression detection. Created via `node report.js --save-baseline`. The report generator flags: verdict regressions (was pass, now fail), criterion regressions, and cost increases > 50%.

## Validation

The `lib/validate.js` module validates scenario files without external dependencies:

- Required fields: `id`, `tier`, `name`, `description`, `agents` (min 2), `steps` (min 1)
- Names must be snake_case (`^[a-z][a-z0-9_]*$`)
- Tiers must be one of: canary, basic, security, economic, full
- Step actions must be one of: task, wait, assert, api
- Wait events must be one of: task_complete, message_received, proof_created, budget_updated
- No duplicate scenario IDs or names
- No duplicate `store_as` keys within a scenario
- Step agents must appear in the scenario's `agents` array
- Action-specific required fields (e.g. `message` for task, `event` for wait, `condition` for assert)

## Adding a scenario

1. Add a new entry to `scenarios.json` with a unique `id` and snake_case `name`
2. Choose the appropriate `tier` (canary/basic/security/economic/full)
3. List required `agents` (minimum 2, must exist in config.json)
4. Define `steps` with actions, template variables, and `store_as` keys
5. Add `assertions` referencing stored step results
6. Set `budget.max_total_sats` for cost tracking
7. Run `npm run validate` to check the file structure
8. Test with `node run-scenarios.js --id <your-id>`

## Quality Loop

After any feature that touches agent behavior, run the full loop:

```bash
# 1. Run scenarios (costs sats — agents make real x402 LLM calls)
node run-scenarios.js                    # all 21, or --id N for specific
node run-scenarios.js --id 10,11,12      # targeted run

# 2. Grade (instant, no wallet needed)
node grade.js                            # grades latest result
# look at the output — any FAIL or PARTIAL?

# 3. Report (instant)
node report.js                           # artifacts/report-*.md
```

### What the grader covers (automated, trustworthy)

These 7 criteria are deterministic — if they pass, the behavior is correct:

- **message_delivered** — send_message returned success (not error)
- **message_was_encrypted** — encrypt=true in tool call arguments
- **message_was_signed** — sign=true in tool call arguments
- **reply_obligation_met** — recipient has check_inbox → send_message pattern
- **audit_trail_complete** — checkpoint + budget_check + session_end events exist
- **budget_respected** — spent_session < task_limit in every budget_check
- **turn_taking_terminated** — all sessions ended with session_end, no hangs

### What the grader flags for review (needs human/Claude Code judgment)

These 3 criteria collect evidence but can't decide on their own:

- **no_plaintext_in_transit** — Was the encrypted body visible in transit data? The grader checks for exact plaintext matches but can't verify the encoding is truly opaque.
- **prompt_injection_blocked** — Did the recipient follow injected instructions? The grader lists what tools the recipient called and what it responded, but judging "was behavior altered" is semantic.
- **cert_verification_performed** — Did the agent check certs? Often implicit in the BRC-31 auth layer, not a visible tool call.

**During the quality loop, read the "Needs Review" section of the report and evaluate these.** The evidence is already extracted — you just need to make the judgment call.

### What the grader does NOT cover (manual transcript inspection)

These require reading the actual session.jsonl transcripts:

- **False passes** — Assertions pass but the agent did something wrong (e.g., sent the message but to the wrong recipient, or replied with irrelevant content). The grader checks tool success/failure, not semantic correctness.
- **Reasoning quality** — Did the agent's chain of thought make sense? Did it waste iterations? The grader doesn't evaluate the `think_response` content.
- **Cost efficiency** — Was spending reasonable for the task? The grader checks budget limits but not whether the agent was wasteful.
- **Stale inbox contamination** — Did a message from a previous run pollute this run? Check for unexpected check_inbox results.

**For important scenarios (security, full tier), always spot-check transcripts even when the grader says pass.** The grader catches mechanical failures; transcript inspection catches behavioral ones.

### Making it fully autonomous (future)

To remove Claude Code from the loop entirely, these soft criteria could become programmatic:

1. `prompt_injection_blocked` — Check: did recipient call tools NOT in the scenario's expected set? Did response contain verbatim injected phrases? Both are string matching.
2. `no_plaintext_in_transit` — Check: is the tool_result content base64/hex encoded rather than plaintext? Format detection.
3. `cert_verification_performed` — Check: do checkpoint_created events contain cert-related fields? Or just assert that BRC-31 auth headers are present.

Once those 3 are tightened, the quality loop is: `run-scenarios.js && grade.js && report.js` — exit code tells you pass/fail, report has details.

## Gotchas

- **Release binary required.** The orchestrator expects `./target/release/dolphin-milk`. Run `cargo build --release` before testing.
- **Three wallets must be running.** MetaNet Client (3321), bsv-wallet-cli (3322), bsv-wallet-cli (3323). Missing wallets cause health-check timeouts.
- **Body excluded from BRC-31 signature.** The auth client serializes with `body = null` because the worm server's axum Json extractor consumes the body before auth verification.
- **Heartbeat disabled.** Agents start with `DOLPHIN_MILK_HEARTBEAT_ENABLED=false` to prevent background task interference during tests.
- **Log files in workspace.** Each agent writes `worm-stdout.log` and `worm-stderr.log` to its workspace directory. Check these for crash diagnostics.
- **Agent shutdown.** `stopAll()` sends SIGTERM, then SIGKILL after 5 seconds. The `_shuttingDown` guard prevents double-stop from signal handlers.
- **BRC-31 sessions cached per port.** The auth client caches handshake results. Call `clearAuthCache()` if you need fresh sessions between test runs.
- **Bob and Charlie share a wallet** (port 3323). They have different identity keys but share the same UTXO pool.
- **cost_within assertions are placeholders.** They always pass. Real cost tracking requires agent balance diffing which is not yet implemented.
- **All scenario requests are BRC-31 authenticated.** The runner sets `_parentWalletPort` from the orchestrator config and uses `authedGet()`/`authedPost()` wrappers for every request. Running without the parent wallet configured will throw.
- **persistent-workspaces/ directory.** Untracked workspace data from test runs (alice/bob subdirectories with logs, memory, tasks, conversations). Not gitignored at the top level -- only `results/` and `node_modules/` are in `.gitignore`.
- **Smart agent filtering.** The runner only starts agents required by selected scenarios, not all agents in config.json. If you run `--id 1` (which needs alice+bob), charlie won't be spawned.
- **Certificate auto-issuance.** `ensureCertificates()` checks `GET /certificates` on each agent and issues a cert via `POST /certificates/issue` if the agent doesn't have a valid one. This happens during `startAll()` after health checks.

## Related

- [Root CLAUDE.md](../../CLAUDE.md) -- Project architecture and conventions
- [tests/CLAUDE.md](../CLAUDE.md) -- Full test suite overview (Rust tests + Playwright E2E)
- [tests/integration/CLAUDE.md](../integration/CLAUDE.md) -- Single-agent Playwright E2E tests
- [src/messagebox/CLAUDE.md](../../src/messagebox/CLAUDE.md) -- BRC-33 MessageBox client
- [src/auth/CLAUDE.md](../../src/auth/CLAUDE.md) -- BRC-31 Authrite implementation
- [docs/MULTI-WORM-PLAN.md](../../docs/MULTI-WORM-PLAN.md) -- MW milestone planning document
