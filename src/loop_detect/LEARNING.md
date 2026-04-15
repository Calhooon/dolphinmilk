# Learning — loop_detect

## Lessons Learned
- **[2026-02-24]** `check()` increments `total_calls` unconditionally — if you call it speculatively without executing the tool, the circuit breaker counter still advances. This means a cautious runner that checks multiple candidate tools before picking one will hit the breaker faster than expected. Design callers to check once per actual dispatch, not per candidate.
- **[2026-02-24]** `no_progress` is the odd one out: it only emits `tracing::warn!`, never returns a `LoopCheckResult`. If you're writing tests expecting all detectors to surface through `check()` or `record_outcome()`, you won't catch no-progress without inspecting logs. Consider promoting it to a real result if the runner needs to act on it.
- **[2026-02-24]** The `_sats_spent_session` parameter on `check_budget_drain()` is a dead arg — the function only uses the sliding 1-hour window from internal history. Don't pass meaningful data there expecting it to matter; it's reserved for a future session-total check.

## Debugging Insights
- **[2026-02-24]** When a loop fires unexpectedly early, check `total_calls` vs history length. `total_calls` counts `check()` invocations while history only grows on `record_outcome()`. A mismatch means tools were checked but not executed, which silently inflates the breaker counter.
- **[2026-02-24]** Ping-pong detection requires exactly 4+ history entries with an ABAB pattern in the last 4 slots, then counts ALL matching entries across full history against `warn_threshold`. Short test histories (< 10 entries with default thresholds) won't trigger it even with a clear alternating pattern.
- **[2026-02-24]** Budget drain uses `SystemTime::now()` — tests that create records with `ts` values far in the past won't register in the 1-hour window. Either mock timestamps or set `ts` to recent values in test fixtures.

## Pattern Notes
- **[2026-02-24]** The two-phase API (`check` before, `record_outcome` after) is the key design pattern. It lets the runner abort before wasting a paid tool call. Any new detector should follow this split: pre-execution checks in `check()`, post-execution analysis in `record_outcome()`.
- **[2026-02-24]** Hashing uses truncated SHA-256 (first 8 bytes → 16 hex chars). This is plenty for equality comparison in a short-lived history buffer but not collision-resistant in a cryptographic sense. Don't reuse these hashes for anything security-sensitive.
- **[2026-02-24]** The warning/critical/breaker escalation (10/20/30 defaults) all set `stuck = true` — the runner must distinguish severity by reading the `level` field. Treating any `stuck = true` as a hard stop is safe but aggressive; injecting the message as a system prompt at warning level gives the LLM a chance to self-correct.
- **[2026-02-24]** `reset()` between tasks is essential. Without it, a legitimate 25-call task followed by a 6-call task would hit the breaker at call 31. The runner calls `reset()` at task boundaries to isolate detection state.
