# Learning — src

## Lessons Learned
- **[2026-02-24]** Wallet HTTP 200 responses can still carry errors — `wallet.rs:call()` checks for an `error` field in the JSON body even on success. Always check both HTTP status AND body error fields when adding new wallet endpoints.
- **[2026-02-24]** The nudge/force-send system in `runner.rs` is invisible in the transcript. If debugging why the agent "replied" without a tool call, check `nudge_count` and the force-send path at `runner.rs:620-662`.
- **[2026-02-24]** `update_token()` in `state.rs` creates a NEW token without spending the old UTXO. Old tokens accumulate in baskets until the wallet exposes UTXO selection. Don't assume token updates are atomic state transitions yet.
- **[2026-02-24]** The `time` crate is pinned to 0.3.39 for Rust 1.85 compatibility. Upgrading it will break the build. Check rustc version before bumping.

## Debugging Insights
- **[2026-02-24]** If `think()` fails with "LLM response has no choices", the payment may have been accepted but the model returned an empty response. Check `sats_paid` vs `sats_effective` in the transcript — you may have paid but gotten nothing back.
- **[2026-02-24]** Repeated 402 errors after payment: the x402 server can return 402 multiple times. `think.rs` retries up to 3 times with fresh payment parsing each attempt. If all 3 fail, inspect the `x-bsv-auth-identity-key` header — a missing server key aborts immediately.
- **[2026-02-24]** Tool call extraction from text (`runner.rs:685-721`) fires when reasoning models describe tool calls in prose instead of using function calling. Look for `text-extracted-N` IDs in the transcript to see when this fallback activated.
- **[2026-02-24]** Balance pagination in `wallet.rs:get_balance()` loops with limit=100. If the balance looks wrong, check whether the wallet has >100 UTXOs — each page requires a separate HTTP round-trip.

## Pattern Notes
- **[2026-02-24]** Two encryption formats coexist: legacy (magic `0x42421033`, 84-byte overhead, local SymmetricKey) and wallet-native (opaque blobs via wallet encrypt/decrypt endpoints). `is_legacy_format()` auto-detects. New code should always use wallet-native.
- **[2026-02-24]** Transcript drives message reconstruction, not caching. `to_messages()` replays ALL events each iteration. Tool calls are embedded in `think_response` events so the full OpenAI message history (including `tool_calls` + `tool` role) can be rebuilt. Don't try to cache messages — the transcript IS the cache.
- **[2026-02-24]** Budget pre-check uses a hardcoded 500 sat estimate (`runner.rs:442`). Actual cost is usually lower. This is intentional conservatism — better to reject early than overspend. If models get cheaper, this estimate should shrink.
- **[2026-02-24]** Reasoning model detection (`think.rs:25-39`) matches prefixes: `o1/o3/o4`, `gpt-5`, `gpt-4.1`. These models require `max_completion_tokens` (not `max_tokens`) and no `temperature`. Getting this wrong causes silent inference failures.
- **[2026-02-24]** Session summarization at loop end is best-effort — memory sync, proof creation, and session writes all log errors but never fail the loop. The agent's primary output (task result) is preserved regardless.
