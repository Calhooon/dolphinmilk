# Loop Detect
> Circuit breaker for repetitive agent behavior — prevents runaway tool calls and budget drain.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 3 | Re-exports `detector` module |
| `detector.rs` | 280 | All detection logic: structs, thresholds, 4 detectors + budget drain |

Single-file module — all logic lives in `detector.rs`.

## Public API

### Structs

**`LoopDetector`** — Main detector. Holds history, thresholds, and call counter.
```rust
pub struct LoopDetector {
    pub warn_threshold: u32,       // default 10
    pub critical_threshold: u32,   // default 20
    pub breaker_threshold: u32,    // default 30
    pub max_sats_per_hour: u64,    // from config.budget.max_per_hour
    history: Vec<ToolCallRecord>,  // private
    total_calls: u32,              // private
}
```

**`LoopCheckResult`** — Returned by `check()` and `check_budget_drain()`.
```rust
pub struct LoopCheckResult {
    pub stuck: bool,      // true = loop detected
    pub level: String,    // "warning", "critical", or "breaker"
    pub message: String,  // human-readable explanation for LLM
    pub detector: String, // which detector fired: "generic_repeat", "ping_pong", "global_circuit_breaker", "budget_drain"
}
```
Default is `stuck: false` with empty strings (no detection).

**`ToolCallRecord`** — Internal history entry.
```rust
pub struct ToolCallRecord {
    pub name: String,        // tool name
    pub params_hash: String, // truncated SHA-256 of params JSON
    pub result_hash: String, // truncated SHA-256 of result string
    pub ts: f64,             // unix timestamp (seconds)
    pub sats_cost: u64,      // satoshis spent on this call
}
```

### Constructors

- **`new(warn, critical, breaker, max_sats_per_hour)`** — Custom thresholds.
- **`with_defaults(max_sats_per_hour)`** — Uses constants: warn=10, critical=20, breaker=30. This is what `runner.rs` uses.

### Methods

- **`check(tool_name, params) -> LoopCheckResult`** — Call BEFORE executing a tool. Increments `total_calls`, then runs circuit breaker → generic repeat → ping-pong checks in order. Returns on first match.
- **`record_outcome(tool_name, params, result, sats_cost)`** — Call AFTER executing a tool. Appends to history, then runs `check_no_progress()` (log-only).
- **`check_budget_drain(sats_spent_session) -> LoopCheckResult`** — Sums `sats_cost` from last hour of history against `max_sats_per_hour`. Called separately from `check()` — runner invokes it after recording budget.
- **`reset()`** — Clears history and `total_calls`. Used between tasks.
- **`total_calls() -> u32`** — Read-only accessor for the call counter.

## Detectors

### 1. `global_circuit_breaker`
Hard stop after `breaker_threshold` total `check()` calls. First check in `check()` — runs before pattern detectors. Returns `level: "breaker"`.

### 2. `generic_repeat`
Counts consecutive identical calls (same `name` + `params_hash`) at the tail of history. Walks history in reverse until a different call is found.
- `>= warn_threshold` consecutive → `level: "warning"`
- `>= critical_threshold` consecutive → `level: "critical"`

### 3. `ping_pong`
Detects ABAB alternation pattern. Requires:
1. At least 4 history entries
2. Last 4 entries form `keys[0]==keys[2]` and `keys[1]==keys[3]` with `keys[0]!=keys[1]`
3. Current call matches one of the two patterns
4. Total occurrences of both patterns across all history `>= warn_threshold`

Returns `level: "warning"` when triggered.

### 4. `no_progress`
Checks if the last `warn_threshold` recorded results all have the same `result_hash`. Fires a `tracing::warn!` only — does NOT return a `LoopCheckResult`. Informational; the runner relies on the other detectors for intervention.

### 5. `budget_drain`
Time-windowed spending check. Sums `sats_cost` from history entries in the last 3600 seconds. If sum exceeds `max_sats_per_hour`, returns `level: "critical"`.

## Thresholds

| Constant | Value | Used by |
|----------|-------|---------|
| `WARN_THRESHOLD` | 10 | `generic_repeat` warning, `ping_pong`, `no_progress` |
| `CRITICAL_THRESHOLD` | 20 | `generic_repeat` critical |
| `BREAKER_THRESHOLD` | 30 | `global_circuit_breaker` |

All configurable via the `new()` constructor. `with_defaults()` uses the constants above.

## Hashing

Parameters and results are compared by truncated SHA-256:
```
SHA-256(content) → first 8 bytes → hex-encoded (16 chars)
```
- `hash_params()` — serializes `serde_json::Value` to string, then hashes
- `hash_result()` — hashes the result string directly

Full content is never retained in the detector — only the 16-char hex digest.

## Data Flow

```
Runner                          LoopDetector
  │                                │
  ├─ check(name, params) ─────────►│ total_calls += 1
  │                                │ run: breaker → repeat → ping_pong
  │◄── LoopCheckResult ───────────┤
  │                                │
  │  [execute tool if !stuck]      │
  │                                │
  ├─ record_outcome(name,          │
  │    params, result, cost) ──────►│ push ToolCallRecord
  │                                │ run: no_progress (log only)
  │                                │
  ├─ check_budget_drain(spent) ────►│ sum last-hour costs
  │◄── LoopCheckResult ───────────┤
  │                                │
  ├─ reset() ──────────────────────►│ clear history + total_calls
  │  (between tasks)               │
```

## Runner Integration

In `runner/`:
- **Construction** (`mod.rs`): `LoopDetector::with_defaults(config.budget.max_per_hour)` — stored as `runner.detector`.
- **check()** (`execute.rs:579`): Called after each tool execution with actual tool parameters (not call IDs). If `stuck && level == "critical"`, returns `WormError::loop_err`.
- **record_outcome()** (`execute.rs:586`): Called after each tool execution with tool name, actual params, output, and sats cost. Feeds `no_progress` and `ping_pong` detectors with real history.
- **check_budget_drain()** (`step.rs:554`): Called in the RECORD phase with `state.sats_spent`. If `stuck`, logs a `loop_warning` transcript event. If `level == "critical"`, returns `WormError::loop_err`.
- **All 5 detectors are active**: Both `check()` and `record_outcome()` are called per tool execution, so all pattern detectors (generic_repeat, ping_pong, no_progress, circuit breaker, budget drain) function in practice.

## Decisions

- **Four detectors plus budget drain**: `generic_repeat` (same tool+params N times), `no_progress` (identical results), `ping_pong` (alternating between two patterns), and `global_circuit_breaker` (hard stop at N total calls). Budget drain is a fifth check that's novel — not derived from OpenClaw.
- **Check before, record after**: `check()` is called BEFORE executing a tool (increments `total_calls`, runs repeat/ping-pong/breaker checks). `record_outcome()` is called AFTER (stores the result hash, runs no-progress check). This two-phase design lets the runner abort before wasting a tool call.
- **Truncated SHA-256 for comparison**: Params and results are hashed to 8-byte hex strings (16 chars) for storage efficiency. Full content is never retained in the detector.
- **Warning → Critical → Breaker escalation**: Thresholds default to 10/20/30. Warning-level results still set `stuck = true` — the runner decides whether to inject the message as a system prompt or hard-stop. Both warning and critical are actionable.
- **`no_progress` only logs, doesn't return a result**: Unlike the other detectors, `check_no_progress()` fires a `tracing::warn!` but doesn't produce a `LoopCheckResult`. It's informational — the runner relies on the other detectors for actual intervention.
- **Budget drain is time-windowed**: `check_budget_drain()` sums `sats_cost` from the last hour of history. It's called separately from `check()` — the runner must invoke it explicitly with session spend data.

## Gotchas

- **`check()` always increments `total_calls`**: Even if you call `check()` and then decide not to execute the tool, the counter still went up. This means the circuit breaker fires based on check attempts, not successful executions.
- **Ping-pong needs 4 history entries**: The detector looks at the last 4 recorded calls for an ABAB pattern, then counts total occurrences of both patterns across all history. It won't trigger until history has at least 4 entries AND the pattern count exceeds `warn_threshold`.
- **`check_budget_drain` takes `_sats_spent_session` but ignores it**: The parameter exists for a planned session-total check but the implementation only uses the sliding 1-hour window from history. Don't rely on the parameter doing anything.
- **`reset()` clears everything**: Both history and `total_calls`. Used between tasks in the agent loop so detection state doesn't leak across unrelated tasks.
- **Runner passes actual tool params**: The runner calls `check(name, arguments)` with the real tool parameters, so `generic_repeat` compares actual param hashes. Identical calls to the same tool with the same params will be detected.

## Tests

File: `tests/core/test_loop_detector.rs` — 16 tests.

| Test | What it verifies |
|------|-----------------|
| `test_no_loop_initially` | Fresh detector returns `stuck: false` |
| `test_generic_repeat_warning` | N identical records + check → warning level |
| `test_generic_repeat_critical` | N identical records at critical threshold → critical level |
| `test_different_params_no_loop` | Varying params don't trigger repeat detection |
| `test_global_circuit_breaker` | Breaker fires at `breaker_threshold` total calls |
| `test_reset` | `reset()` clears `total_calls` and history |
| `test_budget_drain_ok` | Spending under limit returns `stuck: false` |
| `test_budget_drain_excessive` | Spending over `max_sats_per_hour` triggers critical |
| `test_ping_pong_detection` | ABAB pattern doesn't crash (threshold-dependent trigger) |
| `test_with_defaults` | `with_defaults()` uses 10/20/30 constants |
| `test_end_to_end_check_then_record_no_loop` | Full check+record cycle with varying params — no false positive |
| `test_end_to_end_repeat_detection_with_actual_params` | Identical params trigger generic_repeat after threshold |
| `test_end_to_end_no_progress_detection` | Identical results trigger no_progress (log-only, no crash) |
| `test_end_to_end_ping_pong_with_record_outcome` | ABAB pattern with proper check+record cycle triggers ping_pong |
| `test_end_to_end_mixed_tools_no_false_positive` | Varied tool sequence produces no false positives |
| `test_record_outcome_tracks_sats_cost` | Budget drain uses sats from record_outcome history |

## Dependencies

- `sha2` — SHA-256 for param/result hashing
- `hex` — Encoding digest bytes to hex string
- `serde_json` — Serializing params for hashing
- `tracing` — `warn!` macro for no-progress logging
- `std::time` — `SystemTime` for budget drain time window

## Related

- [Root CLAUDE.md](../../CLAUDE.md) — project architecture and conventions
- `src/runner/execute.rs` — calls `check()` and `record_outcome()` after each tool execution
- `src/runner/step.rs` — calls `check_budget_drain()` in the RECORD phase
- `src/onchain/budget.rs` — separate budget tracker with JSONL audit; loop_detect's budget drain is a complementary rate-based check
- `src/error.rs` — `WormError::loop_err()` variant returned when detection triggers a hard stop
