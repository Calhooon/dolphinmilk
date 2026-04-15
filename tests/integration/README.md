# Integration Tests

Real-BSV tests that validate Dolphin Milk end-to-end via Playwright browser automation.

Every test sends a real message through the UI, pays real sats for LLM inference via x402, and verifies the response. This is not a mock — it tests the full stack.

## Quick Start

```bash
# Prerequisites
cd tests/integration
npm install
npx playwright install chromium

# Start the Dolphin Milk server (separate terminal)
cargo run --release -- serve --port 8080

# Run all tests (~$0.20 in BSV, ~10 minutes)
npm test

# Run just one test as a health check (~$0.01)
npm run canary

# Run a specific tier
npm run test:trivial    # 5 tests, ~$0.03
npm run test:medium     # 5 tests, ~$0.05
npm run test:complex    # 5 tests, ~$0.10
npm run test:edge       # 4 tests, ~$0.03

# Compare against baselines
npm run compare

# See what would run without executing
npm run dry-run
```

## What It Tests

### Tier 1 — Trivial (5 tests)
Basic inference, arithmetic, instruction following. Validates x402 payment flows.
Expected: 100% pass, < 50K sats each.

### Tier 2 — Medium (5 tests)
Memory store/recall (cross-session!), wallet balance, budget awareness, tool listing.
Expected: > 80% pass, < 150K sats each.

### Tier 3 — Complex (5 tests)
Web search, creative writing + memory, audit trail introspection, self-description, certificates.
Expected: > 60% pass, < 500K sats each.

### Tier 4 — Edge Cases (4 tests + 1 skipped)
Empty input (UI rejects), prompt injection (agent refuses), multi-task, on-chain proof txid.
Expected: graceful handling, no crashes, no data leaks.

## Results

Results are saved as JSON in `results/` after each run:

```
results/
  run-2026-03-23T12-45-00-000Z.json
  run-2026-03-24T09-00-00-000Z.json
```

Each result file contains:
- Timestamp, wallet balances (before/after), total sats spent
- Per-test: response text, sats spent, tokens, latency, pass/fail

## Adding New Tests

Edit `scenarios.json`. Each scenario has:

```json
{
  "id": 21,
  "tier": "medium",
  "name": "my_new_test",
  "message": "The message to send to the agent",
  "expected_contains": ["word1", "word2"],
  "expected_contains_any": ["either", "this", "or_that"],
  "expected_not_contains": ["bad_word"],
  "expected_pattern": "\\d+ sats",
  "max_cost_sats": 100000
}
```

Validation rules:
- `expected_contains` — ALL must appear in response (case-insensitive)
- `expected_contains_any` — at least ONE must appear
- `expected_not_contains` — NONE should appear
- `expected_pattern` — regex must match somewhere in response
- `max_cost_sats` — test fails if it costs more than this
- `expected_behavior: "ui_rejects_send"` — for empty input tests

## Use Cases

| Use Case | Command | Cost | When |
|----------|---------|------|------|
| Pre-release gate | `npm test` | ~$0.20 | Before every deploy |
| Health check | `npm run canary` | ~$0.01 | Hourly cron |
| Model comparison | `npm test` (switch model, run again) | ~$0.40 | When evaluating models |
| Feature validation | `npm test -- --id 21,22` | ~$0.02 | After shipping new feature |
| Cost regression | `npm run compare` | ~$0.20 | Weekly |

## Cost Reference (from Round 2.5 baseline)

| Tier | Avg Cost | Avg Latency |
|------|----------|-------------|
| Trivial | 31,138 sats ($0.004) | 20s |
| Medium | 47,451 sats ($0.007) | 30s |
| Complex | 119,685 sats ($0.017) | 45s |
| Edge | 27,391 sats ($0.004) | 22s |
| **Full run** | **~1.2M sats ($0.18)** | **~10 min** |
