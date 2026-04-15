# tests/integration
> Playwright-based end-to-end tests that send real messages through the Dolphin Milk UI and pay real sats for LLM inference via x402.

## Overview

This directory contains a Node.js test harness that drives the live Dolphin Milk UI via headless Chromium (Playwright). Unlike the Rust `tests/test_*.rs` files which use mocks, these tests exercise the full stack: UI rendering, HTTP API, agent loop, x402 payment, and LLM inference. Every test costs real BSV satoshis. Results are saved as timestamped JSON files for regression tracking.

There are two test scripts: `run.js` (scenario-driven agent tests) and `test-ui-views.js` (UI view rendering validation).

## Files

| File | Lines | Purpose |
|------|------:|---------|
| run.js | 200 | Thin orchestrator — CLI parsing, browser lifecycle, test loop, result aggregation. Imports helpers from `lib/` |
| lib/scraper.js | 76 | Shadow DOM scraping — `captureLastAssistantMessage()`, `getSatsBalance()` |
| lib/evaluator.js | 65 | Result validation — `evaluateResult()`, `compareWithBaseline()` |
| lib/navigation.js | 33 | Page navigation — `navigateToNewChat()`, `waitForEnabled()` |
| lib/helpers.js | 18 | CLI + formatting — `getArg()`, `formatCost()`, `formatDuration()` |
| test-ui-views.js | 278 | UI view validation — verifies 9 views/endpoints render correctly (main UI, telemetry, /metrics, replay, marketplace, escalation, model picker, model pricing, /agent models) |
| package.json | 19 | Node.js manifest — `playwright` dependency, npm script aliases for tier/canary/dry-run |
| scenarios.json | 785 | All 51 scenarios (IDs 1-51) across 4 tiers with baselines. 1 skipped (ID 18) |
| scenarios_P.json | 14 | Legacy cost baseline data for scenarios #26-27 |
| .gitignore | 2 | Excludes `node_modules/` and `results/run-*.json` |
| README.md | 113 | Setup, tier descriptions, cost reference, scenario schema docs |
| screenshots/ | — | UI comparison screenshots (before/after for chat and dashboard views) |
| working/ | — | Runtime workspace directory (conversations/, gitignored results) |

## Architecture

```
run.js                          (orchestrator)
  │
  ├── lib/helpers.js            CLI arg parsing, formatting
  ├── lib/navigation.js         Page navigation, input readiness
  ├── lib/scraper.js            Shadow DOM text extraction
  ├── lib/evaluator.js          Result validation, baseline comparison
  │
  ├── Load scenarios from scenarios.json
  ├── Launch headless Chromium (Playwright)
  ├── For each scenario:
  │     ├── navigateToNewChat()              [lib/navigation]
  │     ├── Fill message input + click Send
  │     ├── Wait for Stop button to disappear (task complete)
  │     ├── captureLastAssistantMessage()    [lib/scraper]
  │     ├── evaluateResult()                 [lib/evaluator]
  │     └── Cooldown between tests
  ├── Save results/run-{timestamp}.json
  └── Exit code 1 if any test failed

test-ui-views.js                (standalone, does not use lib/)
```

## Scenario Schema

Each scenario in `scenarios.json` is a JSON object:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | number | yes | Unique scenario ID (1-51) |
| `tier` | string | yes | `"trivial"`, `"medium"`, `"complex"`, or `"edge"` |
| `name` | string | yes | Snake_case identifier |
| `message` | string | yes | The prompt sent to the agent |
| `expected_contains` | string[] | no | ALL must appear in response (case-insensitive) |
| `expected_contains_any` | string[] | no | At least ONE must appear |
| `expected_not_contains` | string[] | no | NONE may appear |
| `expected_pattern` | string | no | Regex that must match somewhere |
| `max_cost_sats` | number | no | Fail if cost exceeds this |
| `expected_behavior` | string | no | Special behavior, e.g. `"ui_rejects_send"` |
| `timeout_ms` | number | no | Override default timeout (default 90000ms) |
| `skip` | boolean | no | Skip this scenario |
| `baseline_sats` | number | no | Baseline cost for `--compare` mode |
| `baseline_latency_ms` | number | no | Baseline latency for `--compare` mode |

## Key Functions

### run.js (orchestrator)

| Function | Purpose |
|----------|---------|
| `main()` | CLI parsing, scenario filtering, browser lifecycle, result aggregation |
| `runScenario(page, test, scenarios)` | Drives a single test: navigate, send message, wait for completion, capture response and cost |

### lib/scraper.js

| Function | Purpose |
|----------|---------|
| `captureLastAssistantMessage(page)` | 3-tier shadow DOM traversal: `worm-app` -> `worm-chat` -> `worm-message` elements, then tool card fallback, then broad page scrape |
| `getSatsBalance(page)` | Scrapes wallet balance from UI text matching `/\d{3},\d{3},\d{3} sats/` |

### lib/evaluator.js

| Function | Purpose |
|----------|---------|
| `evaluateResult(test, result)` | Applies all validation rules (contains, contains_any, not_contains, pattern, max_cost). Empty response = fail unless `expected_behavior` is set. |
| `compareWithBaseline(results, scenarios)` | Cost/latency delta percentages against baseline values, flags >50% cost or >100% latency drift |

### lib/navigation.js

| Function | Purpose |
|----------|---------|
| `navigateToNewChat(page, serverUrl)` | Fresh page load to `/ui/` + "New Chat" click for clean conversation state |
| `waitForEnabled(input, page, maxWaitMs)` | Polls until input is enabled (heartbeat tasks may keep it busy, up to 60s) |

### lib/helpers.js

| Function | Purpose |
|----------|---------|
| `getArg(args, flag)` | Extract value after a CLI flag from args array |
| `formatCost(sats)` | Format sats as `"1,234 sats"` or `"free"` |
| `formatDuration(ms)` | Format milliseconds as `"1.2s"` |

### test-ui-views.js

9 sequential tests, each validating a different view or endpoint:

| # | Test | What it checks |
|---|------|----------------|
| 1 | Main UI loads | `/ui/` renders and "Connecting..." disappears |
| 2 | Telemetry view | `/ui/#telemetry` — routes through `worm-agent` → `worm-telemetry` shadow DOM, checks metric keywords (Active, Token, Task, Latency) |
| 3 | /metrics endpoint | Prometheus format with `# HELP` lines and `worm_` prefix (or 401 for BRC-31 protected) |
| 4 | Replay view | `/ui/#task/test-id/replay` — sends a test message first, then verifies `worm-replay-view` component exists |
| 5 | Marketplace API | `GET /marketplace/plugins` returns `{ count, plugins[] }` (or 401) |
| 6 | Escalation endpoint | `GET /task/nonexistent/escalation` returns 401, 404, or 200 |
| 7 | Model picker models | Opens model flyout via shadow DOM (`worm-app` → `worm-chat` → `worm-chat-input`), verifies ≥9 models including GPT-5 Mini, GPT-5, Pro variants, and no phantom gpt-4.1 entries |
| 8 | Model pricing format | Checks all model costs use `input/output` format (contain `/`), no legacy `~$` format |
| 9 | /agent models API | `GET /agent` returns `available_models[]` (≥5) and `default_model` string |

## Usage

### Prerequisites

1. Dolphin Milk server running: `cargo run --release -- serve --port 8080`
2. Funded wallet on `localhost:3322`
3. Playwright Chromium: `cd tests/integration && npm install && npx playwright install chromium`

### Running Tests

```bash
cd tests/integration

# All non-skipped scenarios (50 of 51)
node run.js

# Filter by tier
node run.js --tier trivial
node run.js --tier medium
node run.js --tier complex

# Specific scenario IDs
node run.js --id 1,13,25,37

# Canary (just scenario #1, health check)
node run.js --canary

# Dry run — show what would execute
node run.js --dry-run

# Compare results against baselines
node run.js --compare

# UI view validation (separate script, 9 checks)
node test-ui-views.js

# npm script aliases
npm test                 # node run.js
npm run canary           # node run.js --canary
npm run test:trivial     # node run.js --tier trivial
npm run test:medium      # node run.js --tier medium
npm run test:complex     # node run.js --tier complex
npm run test:edge        # node run.js --tier edge
npm run compare          # node run.js --compare
npm run dry-run          # node run.js --dry-run
```

### Scenario Tiers

| Tier | Count | Skipped | Focus |
|------|------:|--------:|-------|
| trivial | 5 | 0 | Arithmetic, self-knowledge, instruction following, minimal response |
| medium | 22 | 0 | Memory store/recall, tool awareness, wallet, budget, model routing, UTXO health |
| complex | 19 | 0 | Web search, creative+memory, multi-step, cost analytics, replay, proofs |
| edge | 5 | 1 | Introspection, self-verification, context isolation |

### Cost Reference

| Tier | Avg Cost | Avg Latency |
|------|----------|-------------|
| Trivial | ~30K sats ($0.004) | 20s |
| Medium | ~47K sats ($0.007) | 30s |
| Complex | ~120K sats ($0.017) | 45s |
| Edge | ~27K sats ($0.004) | 22s |
| **Full run (50 active scenarios)** | **~2-3M sats ($0.30-0.50)** | **~25 min** |

## Output

Results are saved to `results/run-{timestamp}.json` (gitignored). Each file contains:

```json
{
  "timestamp": "2026-03-24T12:45:00.000Z",
  "walletStart": 50000000,
  "walletEnd": 48800000,
  "totalSats": 1200000,
  "passed": 36,
  "failed": 2,
  "total": 38,
  "results": [
    {
      "id": 1,
      "name": "arithmetic",
      "tier": "trivial",
      "pass": true,
      "response": "4",
      "satsSpent": 44289,
      "tokens": "234 tok",
      "latencyMs": 18200
    }
  ]
}
```

## Shadow DOM Traversal

The worm UI uses Lit web components with shadow DOM. `captureLastAssistantMessage()` navigates this hierarchy:

1. `document.querySelector('worm-app')` → `.shadowRoot`
2. `shadowRoot.querySelector('worm-chat')` → `.shadowRoot`
3. `shadowRoot.querySelectorAll('worm-message')` → filter by `role !== 'user'`
4. Last assistant message → `.shadowRoot` → deduplicated text from `p`, `li`, `code`, `td`, `pre` elements
5. Fallback: tool card output from `worm-tool-card` elements (`.output` property, then shadow DOM `.output`/`pre`/`code`)
6. Final fallback: broad page `p`/`li` text scraping

`test-ui-views.js` tests 7-8 traverse deeper: `worm-app` → `worm-chat` → `worm-chat-input` → shadow DOM model flyout (`.model-trigger`, `.model-option .model-name`, `.model-option .model-cost`).

## Gotchas

- **Real money.** Every test except dry-run and UI-rejection tests costs satoshis. Check wallet balance before running a full suite.
- **Server must be running.** Both scripts expect `localhost:8080` with the Dolphin Milk server serving the UI at `/ui/`. They exit with error if "Connecting..." never resolves.
- **Heartbeat contention.** The input field may be disabled while a heartbeat task is in progress. `waitForEnabled()` polls for up to 60 seconds before proceeding.
- **All 51 scenarios in one file.** `scenarios.json` contains all scenarios (IDs 1-51), 1 of which is skipped (ID 18). The separate `scenarios_P.json` is a legacy cost baseline file, not loaded by `run.js`.
- **Cooldown between tests.** `run.js` waits `cooldown_between_tests_ms` (3000ms, from `scenarios.json` config) between each test to avoid overwhelming the agent.
- **Flaky by nature.** LLM responses are non-deterministic. The validation rules (contains, pattern) are intentionally loose. Cost and latency vary by model load.
- **Input placeholder text.** Both `run.js` and `test-ui-views.js` locate the input via `getByRole('textbox', { name: 'Message Lobster Farm...' })`. If the placeholder text changes in the UI, tests break.
- **Model picker shadow DOM depth.** Tests 7-8 in `test-ui-views.js` traverse 4 levels deep (`worm-app` → `worm-chat` → `worm-chat-input` → model flyout). Changes to any component in the chain will break these tests.

## Related

- [lib/CLAUDE.md](lib/CLAUDE.md) — Detailed docs for the 4 shared utility modules (scraper, evaluator, navigation, helpers)
- [tests/CLAUDE.md](../CLAUDE.md) — Rust integration tests (73 files, ~1902 tests using mocks)
- [ui/](../../ui/) — Lit web frontend (the UI being tested)
- [src/server/](../../src/server/) — Axum HTTP API serving the endpoints tested by `test-ui-views.js`
- [README.md](README.md) — Setup instructions and tier descriptions
