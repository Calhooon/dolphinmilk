# Learning — Tools

## Lessons Learned
- **[2026-02-24]** Tools return `String`, never `Result`. Errors are embedded as `"Error: ..."` prefixed strings. The LLM reads the text directly — there is no structured error channel. This means you cannot pattern-match on tool failures from the runner; grep for `"Error"` in the string if you need to detect failure programmatically.
- **[2026-02-24]** Wallet tools hardcode `localhost:3322` via `client()` helper, creating a fresh `WalletClient` per call. Messagebox and x402 tools take `wallet_url: String` at registration time and capture it via `Arc`. If you add new wallet-dependent tools, follow the `Arc<String>` pattern from messagebox/x402 — the hardcoded approach in wallet_tools doesn't compose well with configurable deployments.
- **[2026-02-24]** The `wallet_encrypt`/`wallet_decrypt` tools default to protocol `[2, "worm encryption"]`, but the memory module uses `[2, "worm memory"]`. These are intentionally different key derivation paths. Data encrypted by the tool cannot be decrypted by the memory system and vice versa unless you explicitly pass matching protocol IDs.
- **[2026-02-24]** `file_write` auto-creates parent directories via `create_dir_all`. The LLM can write to arbitrary paths with no sandboxing. This is by design but worth noting for any future security hardening.

## Debugging Insights
- **[2026-02-24]** If `generate_image` hangs, it's polling `/status/{id}` every 15s for up to 3 minutes (12 polls). The entire agent loop blocks during this time. Check tracing logs for `generate_image: polling status (N/12)` to see progress. A timeout returns a `WormError::payment` with the prediction_id for manual follow-up.
- **[2026-02-24]** `execute_bash` uses `sh -c`, not bash. Shell-specific features (arrays, `[[`) will fail silently or produce wrong results. The 120s timeout and 50K char truncation can mask the real error — check stderr section (after `--- stderr ---`) in the output.
- **[2026-02-24]** `memory_store` creates both a file (via `MemoryStore`) and a tantivy index entry. If indexing fails, the file is still written but a `tracing::warn` is emitted. Search will miss un-indexed entries until the index is rebuilt from files on restart.

## Pattern Notes
- **[2026-02-24]** The closure-wrapping pattern for capturing config is: `Arc::new(value)` at registration, `Arc::clone(&x)` in the `execute` block, dereference inside `Box::pin`. See `memory_tools.rs:189-194` for the canonical example. The `move` keyword on the outer closure captures the Arc; the inner clone is needed because each invocation needs its own owned copy.
- **[2026-02-24]** Every `all_*_tools()` function returns `Vec<ToolDef>`. To add a new tool category: create the file, add the `pub mod` in `mod.rs`, and call your `all_*_tools()` from the runner's registration code. No trait implementation needed.
- **[2026-02-24]** `send_message` validates recipient as exactly 66 chars (compressed pubkey hex). This is a length check only — no hex validation or curve point check. Invalid pubkeys will fail deeper in the Authrite/MessageBox layer with a less helpful error.
- **[2026-02-24]** The registry allowlist (`set_allowlist`) defaults to `None`, meaning all registered tools are allowed. It's opt-in restriction. If you set an allowlist, tools not in it silently become unavailable — `is_allowed()` returns false but `get()` still finds them. The `list_descriptions` and `to_openai_tools` methods both filter by allowlist, so the LLM won't even see restricted tools.
