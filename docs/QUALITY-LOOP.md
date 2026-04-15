# Quality Loop

> Repeatable inspection protocol for Dolphin Milk. Run after every wave, before closing issues.
> Never trust assertion pass/fail alone. Read transcripts.

## 1. The 4-Step Loop

Every quality pass follows the same structure, regardless of what you are testing:

```
BUILD   -->  RUN   -->  INSPECT   -->  VERDICT
  |                                       |
  +---- fix issues, loop back to BUILD ---+
```

**When to run:**
- After every MW+A wave (before closing issues)
- After any feature that affects agent behavior
- After any Rust code change (at minimum: cargo test + clippy)
- Before merging to main

**Expect 2-3 loops per wave.** The first run finds issues. The second verifies fixes. The third is the clean pass.

## 2. Cargo Test Quality Loop

The fastest feedback loop. Run after any Rust code change.

```bash
# All ~2700 tests
cargo test

# Zero warnings required
cargo clippy -- -D warnings
```

Both must pass clean before proceeding to integration testing. If either fails, fix before moving on.

## 3. E2E Quality Loop (Playwright)

Single-agent tests against the live UI. Every scenario pays real sats.

**When to run:** After any feature that affects agent behavior, LLM interaction, tool execution, or UI rendering.

**Prerequisites:**
- `cargo build --release`
- worm server running: `cargo run --release -- serve --port 8080`
- Funded wallet on `localhost:3322`

```bash
cd tests/integration && npm install   # first time only

# Tier progression -- run in order, stop if any tier fails
node run.js --canary              # Health check (~$0.01)
node run.js --tier trivial        # Quick validation (~$0.03)
node run.js --tier medium         # Memory, tools, wallet
node run.js --tier complex        # Multi-step, analytics, proofs
node run.js                       # Full 51-scenario suite (~$0.30-0.50)

# UI view validation (free, no LLM calls)
node test-ui-views.js
```

Full run costs ~$0.30-0.50. Results saved to `tests/integration/results/`. See `tests/integration/CLAUDE.md` for details.

## 4. Multi-Worm Quality Loop

Cross-agent testing. This is where false passes hide.

### Prerequisites

Alice on 8080/3322, Bob on 8081/3323, parent wallet (MetaNet Client) on 3321. See `tests/multi-worm/config.json` for full port mapping.

```bash
cargo build --release

# Start wallets, then Dolphin Milk instances:
./target/release/dolphin-milk serve --port 8080
DOLPHIN_MILK_WALLET_URL=http://localhost:3323 \
  ./target/release/dolphin-milk serve --port 8081 --workspace ./test-workspaces/bob

# Verify both agents healthy, different identity keys, wallets funded:
cd tests/multi-worm && node orchestrate.js --status-only
```

### Running Scenarios

```bash
cd tests/multi-worm

# Smoke test first (free, no LLM calls)
node run-scenarios.js --tier canary

# Run scenarios individually (avoids MessageBox contamination)
node run-scenarios.js --id 10
node run-scenarios.js --id 11

# Dry run -- see what would execute
node run-scenarios.js --dry-run
```

**Message contamination warning:** Agents persist MessageBox state between scenarios within the same run AND across runs on the external relay. Always run messaging scenarios individually with `--id`. Scenario prompts include inbox-ignore language but reply obligations still add 2-3 iterations of overhead from stale messages.

### Three Validation Layers

Quality validation has three layers with different trust levels:

**Layer 1: Assertions** (`run-scenarios.js`) — "Did the HTTP calls return what we expected?"
Checks status codes, field existence, tool success. Fast, deterministic, but shallow. The false-pass problem lives here — assertions pass but behavior is wrong.

**Layer 2: Grader** (`grade.js`) — "Did the agents use the right tools with the right parameters?"
Parses event transcripts. Catches things like "send_message was never called" or "encrypt wasn't set to true" that assertions miss. 7/10 criteria are fully deterministic. 3 soft criteria flag `needs_review: true` with collected evidence. Instant, no wallet/network needed.

**Layer 3: Transcript reading** — "Did the agent actually behave correctly?"
Reading session.jsonl to catch: agent sent to wrong key, hallucinated content, wasted iterations, injection that didn't trigger tools (grader missed it), stale inbox corruption. The grader doesn't replace this — it triages it. Instead of reading all 21 transcripts, you read transcripts only for failures, partials, and security-tier spot checks.

```bash
# The workflow:
node run-scenarios.js --id 17        # 1. Run
node grade.js                        # 2. Grade (instant, free)
node report.js                       # 3. Report
# 4. Read transcripts only for: failures, partials, security spot-checks
```

### Transcript Inspection Protocol

For scenarios flagged by the grader or security-tier spot checks. Wave 3 taught us that 2 out of 4 scenarios were false passes caught only by reading transcripts. The grader catches most of these now, but transcript reading remains the final word.

**Where transcripts live:**

```
test-workspaces/
  alice/
    tasks/{task_id}/session.jsonl    # Per-task transcript
    worm-stdout.log                  # Process stdout
    worm-stderr.log                  # Process stderr (errors, warnings)
  bob/
    tasks/{task_id}/session.jsonl
    worm-stdout.log
    worm-stderr.log
```

**For every scenario that involves LLM calls, inspect these five things:**

1. **Tool calls succeeded.** Not just "tool was called" but the result was meaningful.

```bash
# List all tool calls and their outcomes for a task
TASK="test-workspaces/alice/tasks/{TASK_ID}/session.jsonl"
jq 'select(.event_type=="tool_call") | {name: .data.name}' "$TASK"
jq 'select(.event_type=="tool_result") | {name: .data.name, ok: (.data.output | test("^error"; "i") | not), preview: (.data.output | tostring | .[:120])}' "$TASK"
```

2. **LLM understood the task.** The agent's response should address the actual task, not hallucinate or go off-track.

```bash
# See what the LLM said (think_response content)
jq -r 'select(.event_type=="think_response") | .data.content | .[:200]' "$TASK"
```

3. **No error events.** Check for explicit errors and session_end with error.

```bash
# Any errors at all?
jq 'select(.event_type=="error" or (.event_type=="session_end" and .data.error != null and .data.error != ""))' "$TASK"
```

4. **Expected economic flows.** Sats were spent on LLM inference, tools that should be paid were paid.

```bash
# Total sats from LLM calls
jq -s '[.[] | select(.event_type=="think_response") | .data.sats_paid // 0] | add // 0' "$TASK"
```

5. **Cross-agent consistency.** For messaging scenarios, both sides must tell the same story.

```bash
# Merge both agents' transcripts by timestamp
jq -s 'sort_by(.timestamp)' \
  test-workspaces/alice/tasks/*/session.jsonl \
  test-workspaces/bob/tasks/*/session.jsonl \
  | jq '.[] | {time: .timestamp, type: .event_type, detail: (.data.name // .event_type)}'
```

**Red flags -- stop and investigate if you see any of these:**

| Symptom | Likely Cause |
|---------|-------------|
| Tool result starts with "Error" or "error" | Tool execution failed, assertion may still pass on other criteria |
| Empty LLM response (0 content chars) | x402 payment failed or model returned nothing |
| Wrong tool called (e.g., `web_search` instead of `send_message`) | LLM misunderstood the task, prompt issue |
| Budget exhaustion mid-task | Runaway loop, excessive retries, or model too expensive |
| Self-message (sender == recipient in `send_message`) | Self-message loop bug |
| Agent A's plaintext in Agent B's logs | Encryption leak |
| Iteration count > 10 for a simple task | Agent stuck in a loop |
| `session_end` with error but scenario "passed" | False pass -- assertion checked something other than success |

### Example: Good Transcript vs Bad Transcript

**Good** -- BRC-77 signed message scenario:

```jsonl
{"event_type":"session_start","data":{"task":"Send a signed message to 03def...","model":"gpt-5-mini"}}
{"event_type":"tool_call","data":{"name":"send_message","arguments":"{\"recipient\":\"03def...\",\"body\":\"Alice reporting: BSV network status nominal.\",\"sign\":true}"}}
{"event_type":"tool_result","data":{"name":"send_message","output":"Message sent successfully to task_inbox. MessageID: abc123. Signed: true."}}
{"event_type":"think_response","data":{"content":"I have sent a signed message to the agent...","sats_paid":45000}}
{"event_type":"session_end","data":{"iterations":2,"sats_spent":90000,"error":null}}
```

Clear tool call with correct parameters, successful result, reasonable cost, clean end.

**Bad** -- same scenario, false pass:

```jsonl
{"event_type":"session_start","data":{"task":"Send a signed message to 03def...","model":"gpt-5-mini"}}
{"event_type":"think_response","data":{"content":"I'll send a signed message to the agent.","sats_paid":45000}}
{"event_type":"tool_call","data":{"name":"send_message","arguments":"{\"recipient\":\"03def...\",\"body\":\"Hello\",\"sign\":false}"}}
{"event_type":"tool_result","data":{"name":"send_message","output":"Message sent successfully to task_inbox."}}
{"event_type":"session_end","data":{"iterations":3,"sats_spent":135000,"error":null}}
```

Assertions might pass (`response_contains "send_message"` is true), but: `sign` was `false` (should be `true`), message body doesn't match the scenario's expected content, and cost is 50% higher than expected suggesting an extra iteration. This is a **false pass**.

## 5. Assertion Validation Checklist

Each assertion type, what it actually checks, and known false-positive patterns.

| Assertion | What It Checks | False-Positive Risk |
|-----------|---------------|---------------------|
| `status_code` | HTTP status equals expected value | Low. For task steps, checks POST /task response, not task outcome. |
| `field_exists` | Nested field is present and non-null | Medium. Field present does not mean operation succeeded. |
| `field_equals` | Nested field equals expected value (strict) | Low. Watch for type coercion (`"200"` != `200`). |
| `response_contains` | JSON-stringified result has substring (case-insensitive) | **High.** Searches entire JSON including metadata and events. `"send_message"` matches even if tool failed. `"signature_valid"` matches `"signature_valid": false`. Prefer `field_equals` or `response_matches` with `field`. |
| `response_not_contains` | Result does NOT contain substring | Low. Can false-negative if forbidden string appears in metadata. |
| `response_matches` | Field or full result matches regex | **High** without `field` -- same as `response_contains`. Always specify `field`. |
| `agents_different_keys` | All agents have distinct identity keys | None. Reliable structural check. |
| `session_no_error` | `session_end` event has no error string | **High.** Task can end "successfully" while doing the wrong thing entirely. |
| `tool_succeeded` | Tool called and result does not start with "error" | **High.** Tool succeeded but with wrong parameters (e.g., `sign: false` instead of `true`). |
| `tool_result_contains` | Tool result output contains substring | Medium. Substring could appear in error context. |
| `no_tool_errors` | No tool results start with "error" | Medium. All tools "succeeded" but task logic was wrong. |
| `cost_within` | **Nothing -- always passes.** Placeholder. | N/A. Not implemented yet. |

**Key takeaway:** `response_contains`, `session_no_error`, and `tool_succeeded` are the most common sources of false passes. Always pair them with transcript inspection.

## 6. Session Review Process

After running all scenarios for a wave, perform a full session review before closing issues.

**Wave completion checklist:**

- [ ] All scenarios in the wave pass (Layer 1: assertions)
- [ ] Grader run (`node grade.js`) — all scenarios green or yellow (Layer 2)
- [ ] Transcripts read for: failures, partials, and security-tier spot checks (Layer 3)
- [ ] No red flags from the red flags table
- [ ] Wallet balances make sense (total spent roughly matches sum of scenario costs)
- [ ] On-chain proofs created where expected (BRC-18)
- [ ] Report generated (`node report.js`), baseline saved if costs changed (`node report.js --save-baseline`)
- [ ] Results file saved to `tests/multi-worm/results/`
- [ ] `cargo test` still passes (no regressions from MW changes)
- [ ] `cargo clippy -- -D warnings` clean

**Wave sign-off format** (put in the GitHub issue or PR):

```
## Quality Loop Sign-Off: Wave N

- Scenarios: X/Y passed (assertions)
- Grader: X green, Y yellow (needs_review), Z red. [details on non-green]
- Transcripts: Read for [list scenarios spot-checked]. No false passes.
- Red flags: None / [list any and resolution]
- Cost: ~X sats total across all scenarios
- Cargo test: YYYY tests passing, 0 clippy warnings
- Commit: [sha]
```

## 7. Debugging Failed Scenarios

### Quick triage

```bash
# Check agent logs for crashes or panics
tail -50 test-workspaces/alice/worm-stderr.log
tail -50 test-workspaces/bob/worm-stderr.log

# Find the task ID from the results file
cat tests/multi-worm/results/run-*.json | jq '.results[] | select(.pass == false) | {id: .scenario.id, name: .scenario.name, error}'

# Read the failed task's transcript
TASK_ID="..."  # from above
jq '.' test-workspaces/alice/tasks/$TASK_ID/session.jsonl
```

### Common failure patterns

| Pattern | Cause | Fix |
|---------|-------|-----|
| Health check timeout | Wallet not running or wrong port | Start wallet daemons, check config.json ports |
| "Session not found for nonce" (401) | Stale BRC-31 auth session | `rm ~/.local/share/brc31-sessions/*.json` |
| Task timeout (never completes) | Agent stuck waiting for inbox or looping | Check iteration count in transcript, look for loop detection events |
| "No task_id in response" | POST /task returned unexpected format | Check worm-stderr.log for server errors |
| Message not received by Bob | MessageBox relay down or fee enforcement | Check MessageBox accessibility, verify paid delivery if required |
| Wrong identity key in template | Orchestrator cached stale identity | Restart orchestrator, clear auth cache |

### Re-running a single scenario

```bash
# Run just the failing scenario in isolation (avoids message contamination)
cd tests/multi-worm
node run-scenarios.js --id 10
```

For deeper debugging, see [DEBUG-GUIDE.md](../DEBUG-GUIDE.md) which covers transcript reading, auth troubleshooting, and payment failure investigation.

## Related

- [docs/MULTI-WORM-QUALITY-LOOP.md](MULTI-WORM-QUALITY-LOOP.md) -- Extended multi-worm inspection: cross-agent checklist, 5 analysis scripts, per-wave acceptance criteria
- [tests/multi-worm/CLAUDE.md](../tests/multi-worm/CLAUDE.md) -- Multi-worm test harness docs, scenario schema, assertion types
- [tests/integration/CLAUDE.md](../tests/integration/CLAUDE.md) -- Playwright E2E test docs, shadow DOM scraping
- [DEBUG-GUIDE.md](../DEBUG-GUIDE.md) -- Transcript reading, auth debugging, common failure patterns
- [docs/EXECUTION-PLAN-MW-A.md](EXECUTION-PLAN-MW-A.md) -- 5-wave plan referencing this quality loop
