# Context
> System prompt construction, context window management, and context forking for LLM calls.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 5 | Re-exports `fork`, `manager`, and `prompt` submodules |
| `prompt.rs` | 369 | System prompt builder — assembles 10 sections from `PromptContext`, including BRC-52 certs (with budget limits), soul/identity, external message warning, spendable output count, and on-chain basket health |
| `manager.rs` | 677 | Context window manager — token tracking, dynamic per-message limits, content compaction, truncation, tool-pair sanitization, file offloading, output token reservation, LLM-driven compaction with memory flush |
| `fork.rs` | 226 | Context forking — isolates memory-intensive operations (e.g., full memory dump) in a cloned context without polluting the main context window |

## Public API

### `prompt.rs`

- **`PromptContext`** — Runtime state injected into the system prompt:
  - `identity_key: String` — agent's public key
  - `balance_sats: u64` — current wallet balance
  - `model: String` — LLM model name
  - `tools: Vec<ToolDesc>` — available tools (name + description + category)
  - `memory_summary: String` — from memory system
  - `available_files: Vec<String>` — workspace files listed in environment section
  - `task: String` — current task description
  - `budget_remaining: u64` — sats left for this task
  - `low_power: bool` — enables cost-saving behavior hints
  - `inbox_count: usize` — unread messages from OBSERVE phase
  - `skills_section: String` — pre-formatted from `SkillRegistry::format_for_prompt()`
  - `workspace_path: String` — absolute path to workspace directory, shown in environment section
  - `certificate_info: Option<CertificateInfo>` — BRC-52 authorization certificate summary
  - `has_external_messages: bool` — true when task includes messages from external (untrusted) agents; triggers external message warning section
  - `identity_soul: Option<String>` — agent's identity/soul content from memory (Knowledge category, tag "identity"); inserts a Soul section when present
  - `auto_recall_ids: Vec<String>` — memory IDs auto-recalled for this iteration (Phase 9.2); used for proof enrichment
  - `basket_health: HashMap<String, u64>` — UTXO counts per basket for on-chain vital signs; rendered in the Environment section's "On-Chain State" subsection when non-empty as raw counts (no warning thresholds — UTXO counts grow naturally with task volume)
  - `spendable_output_count: u64` — total spendable outputs in the default basket; rendered in the Wallet section when > 0

- **`CertificateInfo`** — BRC-52 authorization certificate summary rendered in the Identity section:
  - `certifier: String` — certifier's identity key
  - `self_signed: bool` — true when certifier == subject (bootstrap certificate)
  - `capabilities: String` — capabilities granted by the certificate
  - `name: String` — agent name from the certificate
  - `budget_per_task: Option<u64>` — cert-enforced budget limit per task (sats); None = use config defaults
  - `budget_per_hour: Option<u64>` — cert-enforced budget limit per hour (sats); None = use config defaults
  - `budget_per_day: Option<u64>` — cert-enforced budget limit per day (sats); None = use config defaults
  - `budget_per_week: Option<u64>` — cert-enforced budget limit per week (sats); None = use config defaults
  - `budget_per_month: Option<u64>` — cert-enforced budget limit per month (sats); None = use config defaults
  - `budget_lifetime: Option<u64>` — cert-enforced lifetime budget limit (sats); None = use config defaults
  - `budget_enforcement: Option<String>` — enforcement mode from certificate; None = use config default; overrides local config

- **`ToolDesc`** — `{ name, description, category }` for prompt rendering.

- **`build_system_prompt(ctx: &PromptContext) -> String`** — Assembles 10 sections, filters empties, joins with double newline.

### `manager.rs`

- **`estimate_tokens(text) -> usize`** — `len() / 4`, minimum 1.
- **`estimate_message_tokens(msg: &Value) -> usize`** — Content + tool_calls JSON + 4-token overhead.
- **`compact_content(content: &str) -> String`** — Convenience wrapper that calls `compact_content_with_limit()` with `DEFAULT_MAX_MESSAGE_CHARS` (8,000 chars). Suitable for conversation storage where a fixed limit is appropriate.
- **`compact_content_with_limit(content: &str, max_chars: usize) -> String`** — Strips base64 data URIs from content, then truncates if still oversized (> `max_chars`). Two-pass: first strips markdown image data URIs (`![alt](data:image/...)`), then strips bare `data:image/...` URIs. Truncation keeps first half of content with a type-specific hint (base64 image, JSON, HTML, or generic).
- **`ContextManager`** — Manages token budget, content compaction, truncation, and offloading:
  - `new(max_tokens, offload_dir)` — constructor with default limits (8 recent messages, 40 max turns, 16384 output tokens)
  - `with_limits(max_tokens, offload_dir, min_recent_messages, max_history_turns)` — constructor with configurable turn limits
  - `with_output_tokens(max_tokens, offload_dir, min_recent_messages, max_history_turns, output_tokens)` — constructor with configurable turn limits and output token reservation
  - `needs_compaction(history_len) -> bool` — true if history exceeds `max_history_turns`
  - `compaction_summary() -> Option<&str>` — get the current LLM-generated compaction summary
  - `set_compaction_summary(summary)` — set the compaction summary (from LLM call)
  - `set_output_tokens(output_tokens)` — update the output token reservation at runtime (used when discovered model capabilities provide more accurate limits than hardcoded defaults)
  - `output_tokens() -> usize` — accessor for the current output token reservation
  - `max_history_turns() -> usize` — accessor for the max turn limit
  - `max_message_chars() -> usize` — dynamic per-message character limit scaled to model context window (15% of input budget per message, floor 8K, ceiling 400K)
  - `build_messages(system_prompt, history, budget_tokens)` — fits messages into token budget with output token reservation, compaction summary injection, and two-stage truncation. When `budget_tokens` is None, input budget = `max_tokens - output_tokens`.
  - `should_offload(content) -> bool` — true if content > 2,000 tokens
  - `offload_to_file(content, label) -> String` — writes to disk, returns path
  - `offloaded_files() -> &[String]` — list of all offloaded file paths

#### Standalone async functions (`manager.rs`)

- **`pre_compaction_memory_flush(auth, messages_to_drop, model, config, memory_store, conversation_id) -> Vec<String>`** — Before compaction, uses an LLM call to extract durable knowledge (decisions, preferences, findings, action items) from messages about to be dropped, then stores each as a `Session`-category memory entry tagged with `compaction` and the conversation ID. Returns paths of stored entries. Safety net: even if the compaction summary is lossy, critical context survives in long-term memory.
- **`generate_compaction_summary(auth, messages_to_drop, model, config) -> Option<String>`** — Uses an LLM call to summarize dropped messages into key points (decisions, problems, approaches, preferences, next steps). The summary is stored on `ContextManager` and injected as a system message when the hard turn cap trims history.

### `fork.rs`

- **`ContextFork`** — Isolates memory-intensive operations from the main context window:
  - `new(context: &PromptContext, messages: &[Value])` — deep-clones both, fork is fully independent
  - `messages() -> &[Value]` — read-only access to forked messages
  - `push_message(message)` — append to forked context only
  - `inject_memory_summary(summary)` — replace memory summary in forked context (typical use: inject full memory dump)
  - `record_result(label, content)` — store an extracted result from the forked operation
  - `results() -> &[ForkResult]` — get all extracted results
  - `into_results() -> Vec<ForkResult>` — consume the fork, return results
  - `summary() -> String` — concatenate all results as `[label] content` pairs, suitable for injection back into main context
  - `message_count() -> usize` — number of messages in the fork

- **`ForkResult`** — `{ label: String, content: String }` — a labeled result extracted from a forked operation.

- **`is_memory_intensive_skill(frontmatter_yaml: &str) -> bool`** — Parses YAML frontmatter and checks for `memory_intensive: true`. Returns false on parse failure.

### Constants (`manager.rs`)

| Constant | Value | Purpose |
|----------|-------|---------|
| `CHARS_PER_TOKEN` | 4 | Rough char-to-token ratio |
| `DEFAULT_MAX_TOKENS` | 128,000 | Default context window budget |
| `OFFLOAD_THRESHOLD_TOKENS` | 2,000 | ~8K chars triggers file offload |
| `DEFAULT_MAX_MESSAGE_CHARS` | 8,000 | Default per-message content length cap (chars); used by `compact_content()` convenience wrapper and as the floor for `max_message_chars()` dynamic scaling |
| `DEFAULT_MIN_RECENT_MESSAGES` | 8 | Always keep last N messages during truncation |
| `DEFAULT_MAX_HISTORY_TURNS` | 40 | Hard cap on conversation length before oldest are trimmed |
| `DEFAULT_OUTPUT_TOKENS` | 16,384 | Tokens reserved for LLM response generation; deducted from context window to compute input budget |

## System Prompt Sections

`build_system_prompt()` assembles 10 sections from `PromptContext`. Empty sections are filtered out automatically.

1. **Identity** — "You are a BSV worm" framing. Includes identity key if set. Renders BRC-52 certificate info when present (self-signed bootstrap vs parent-certified with `prove_identity` hint). When the certificate includes budget limits (`budget_per_task`, `budget_per_hour`, `budget_per_day`), they are rendered as a "Budget (cert-enforced)" line. Note: `CertificateInfo` also carries `budget_per_week`, `budget_per_month`, `budget_lifetime`, and `budget_enforcement` fields, but these are not yet rendered in the prompt — they are available for downstream enforcement logic.
2. **Soul** — Agent's identity/soul content from memory (Knowledge category, tag "identity"). Includes a hint that identity can be updated via `memory_store`. Empty if `identity_soul` is None.
3. **External Warning** — Injection defense for external agent messages. Tells the LLM to treat external content as DATA, not INSTRUCTIONS. Notes that tools are restricted. Empty if `has_external_messages` is false.
4. **Environment** — Sandbox capabilities (shell, files, packages). Shows workspace path if set. Includes CWD warning about bash vs file_read path resolution. Lists up to 20 available files. Includes code style guidance (early returns, flat structure, write scripts over chaining shell commands). When `basket_health` is non-empty, renders an **On-Chain State** subsection listing UTXO counts per basket (sorted alphabetically) as raw data — no warning thresholds, just counts for the agent to reason about.
5. **Wallet** — Balance, spendable output count (when > 0), budget, cost warnings. Low-power mode injects "minimize tool calls" hint. Includes an IMPORTANT instruction telling the LLM not to call `wallet_balance`, `wallet_call(listOutputs)`, etc. to repeat data already shown — only call wallet tools for data beyond what's in the prompt. References generic x402 tools (`discover_services`, `discover_endpoints`, `x402_call`) for paid services.
6. **Messaging** — BRC-33 MessageBox instructions. Explicit "use `send_message`, not plain text" nudge. Shows inbox count when > 0.
7. **Tools** — Bullet list of available tools (name + description). Appends a `search_tools` progressive discovery hint for additional tools (messaging, scheduling, encryption, etc.). Empty if no tools.
8. **Skills** — Injected from `SkillRegistry::format_for_prompt()`. Empty if no active skills.
9. **Memory** — Memory summary from the memory system. Empty if no memory.
10. **Principles** — 12 static working principles: budget-aware, verify computationally, file-based context, be concise, track progress, fail gracefully, use URLs not base64, refuse cleanly (don't echo declined request terms), answer directly (check data instead of asking for clarification), show your work (display created content in response), read full data before analysis (use `file_read` when tool results reference file paths for complete data), self-state is free (answer wallet/balance/UTXO questions from prompt data without tool calls).

## Message Building Pipeline

`build_messages()` applies four stages to fit history into the token budget:

1. **Content compaction** — `compact_large_messages()` runs `compact_content_with_limit()` on every message BEFORE any token estimation, using the dynamic `max_message_chars()` limit (15% of input budget per message, floor 8K, ceiling 400K). Strips base64 `data:image/...` URIs (both markdown `![alt](data:...)` and bare URIs) and truncates messages exceeding the limit. The truncation keeps the first half with a type-specific hint (base64 image, JSON, HTML, or generic large output). This prevents base64 blobs from blowing the context window while allowing larger models (e.g., 1M context) to retain more content per message.
2. **Hard turn cap + compaction summary** — If history exceeds `max_history_turns` (default 40), the oldest messages are sliced off. If a compaction summary exists (set via `set_compaction_summary()`), it is injected as a system message: `[Earlier conversation summary — N messages compacted]\n\n{summary}`. This preserves context across the hard trim boundary.
3. **Token-aware truncation** — System prompt is always included. The input budget is the context window minus the output token reservation (`DEFAULT_OUTPUT_TOKENS` = 16,384), or an explicit `budget_tokens` override. Remaining budget is split between recent messages (last `min_recent_messages`, default 8) and older messages. Older messages are added oldest-first until budget is exhausted; dropped messages get a `[N earlier messages truncated]` system marker. If even the recent messages exceed the budget, content is truncated mid-string with `[truncated]` appended.
4. **Tool-pair sanitization** — `sanitize_tool_pairs()` removes orphaned messages after truncation:
   - Assistant messages with `tool_calls` whose results were all truncated away → removed
   - Tool result messages whose parent assistant `tool_calls` entry was truncated → removed

   This prevents OpenAI API errors from mismatched tool call/result pairs. Applied both when history fits (to handle `prior_messages` injections) and after truncation.

## Context Forking

`ContextFork` enables memory-intensive operations without polluting the main context window. Designed for skills tagged `memory_intensive: true` in their YAML frontmatter.

**Workflow:**
1. Runner detects a memory-intensive skill via `is_memory_intensive_skill()`
2. Forks the current `PromptContext` and conversation messages via `ContextFork::new()`
3. Injects a large memory dump into the fork via `inject_memory_summary()`
4. Runs the heavy operation (e.g., full recall, analysis) on the fork
5. Extracts results via `record_result()` / `into_results()`
6. Injects a compact `summary()` back into the main context as a system message

The fork deep-clones both `PromptContext` and the message array, so mutations are fully isolated.

## Decisions

- **Sandbox-first framing**: The system prompt presents the agent as having "a computer and a wallet" rather than being a chatbot with tools. This framing (from the LLM-in-Sandbox paper) produces better tool-use behavior from the LLM.
- **Dynamic section assembly**: `build_system_prompt()` composes 10 sections from runtime state (`PromptContext`). Empty sections (no tools, no memory, no skills, no soul, no external messages) are filtered out automatically.
- **Char-based token estimation**: Uses `len() / 4` (~4 chars per English token). This is deliberately rough — accuracy doesn't matter, only order-of-magnitude for truncation decisions. The 128K default budget is conservative.
- **Output token reservation**: `DEFAULT_OUTPUT_TOKENS` (16,384) is deducted from the context window to compute the input budget. This prevents the input from consuming all available tokens, leaving none for the LLM's response. `with_output_tokens()` allows callers to override at construction; `set_output_tokens()` allows runtime updates when discovered model capabilities provide more accurate limits.
- **Content compaction before token estimation**: `compact_large_messages()` runs as the first pipeline stage, stripping data URIs and truncating oversized content before any token math. This prevents a single base64 blob from dominating the token budget and causing excessive truncation of useful messages.
- **Dynamic per-message character limit**: `max_message_chars()` scales the per-message compaction threshold to 15% of the model's available input budget (floor 8K, ceiling 400K chars). This means a 128K-context model allows ~17K chars/message while a 1M-context model allows ~400K. The fixed `DEFAULT_MAX_MESSAGE_CHARS` (8K) is only used by the standalone `compact_content()` convenience wrapper for conversation storage.
- **Two-stage truncation**: First a hard `max_history_turns` cap (default 40), then token-aware oldest-first truncation with a recency guarantee of 8 messages. The hard cap prevents unbounded history growth; the token-aware stage handles variable message sizes.
- **Tool-pair sanitization**: After truncation, orphaned tool call/result messages are cleaned up via `sanitize_tool_pairs()`. This is critical for multi-turn conversations where `prior_messages` injection can create incomplete tool pairs, and for truncation which may split an assistant tool_calls message from its results.
- **File offloading for large results**: Tool results over ~2,000 tokens (8K chars) are written to disk and replaced with a file path reference. This is the paper's "8x token reduction" insight — large outputs (e.g. `execute_bash` output) don't consume context window.
- **Configurable turn limits**: `with_limits()` and `with_output_tokens()` constructors allow callers to override limits from config (`WORM_LLM_MIN_RECENT_MESSAGES`, `WORM_LLM_MAX_HISTORY_TURNS`).
- **Skills section from registry**: The skills section is pre-formatted by `SkillRegistry::format_for_prompt()` and passed through as-is. This keeps prompt.rs decoupled from skill loading logic.
- **LLM-driven compaction with memory flush**: When the hard turn cap kicks in, two LLM calls are made (by the caller, typically runner.rs): one extracts durable knowledge into long-term memory (`pre_compaction_memory_flush`), another generates a summary injected into the context (`generate_compaction_summary`). This two-pronged approach ensures both persistent storage and immediate context continuity.
- **BRC-52 certificate in identity section**: The agent's authorization certificate is rendered in the system prompt so the LLM knows its granted capabilities and whether it has parent-backed authority or only a self-signed bootstrap cert. Certificate-enforced budget limits (per-task, per-hour, per-day) are shown in the prompt when present. `CertificateInfo` also carries week/month/lifetime limits and an enforcement mode for downstream budget enforcement, but these are not rendered in the prompt. This enables the agent to reason about its own permissions and spending constraints.
- **Soul section for agent identity**: The agent's self-authored identity/soul (from memory with tag "identity") is injected between Identity and External Warning. This lets the agent express persistent personality and values that survive across sessions.
- **External message warning section**: When `has_external_messages` is true, an explicit warning tells the LLM to treat external content as data, not instructions. Part of the 4-layer injection defense (Phase 8). Tools are restricted for external-message tasks.
- **Auto-recall IDs tracked on PromptContext**: `auto_recall_ids` captures which memory entries were auto-recalled for the current iteration. Not rendered in the prompt itself — used by `runner.rs` for proof enrichment (Phase 9.2).
- **On-chain vital signs in the prompt**: `basket_health` injects UTXO counts per basket into the Environment section so the agent can reason about its own on-chain state. Raw counts only — no warning thresholds. The caller populates the map; the prompt builder just renders it sorted alphabetically, keeping `prompt.rs` decoupled from wallet logic.
- **Spendable output count in wallet section**: `spendable_output_count` is rendered in the Wallet section (when > 0) alongside balance and budget. Combined with the IMPORTANT instruction to not call wallet tools redundantly, this reduces unnecessary tool calls for self-state queries.
- **Context forking for memory-intensive skills**: Rather than bloating the main context with large memory dumps, `ContextFork` creates an isolated copy where heavy operations run. Only a compact summary flows back. This preserves the main context window for the ongoing conversation.

## Gotchas

- **Messaging nudge is baked into the prompt**: The messaging section explicitly tells the LLM that plain text responses are NOT delivered — only `send_message` tool calls reach other agents. This is critical for cross-wallet conversation; without it the LLM "responds" to inbox messages with text that goes nowhere.
- **`inbox_count` drives reply behavior**: When `ctx.inbox_count > 0`, an extra line is injected telling the LLM it has unread messages. This is how the runner triggers the agent to reply without explicit tool dispatch.
- **Soul section is self-authored**: The `identity_soul` content comes from memory entries with category "knowledge" and tag "identity". The agent can update its own soul via `memory_store`, creating on-chain proofs of identity evolution.
- **External warning is a soft defense layer**: The `section_external_warning()` tells the LLM not to follow embedded instructions, but this is advisory — the LLM may still comply with crafted prompts. The hard defense is the tool allowlist in `sanitize.rs`.
- **`auto_recall_ids` is not rendered in the prompt**: It lives on `PromptContext` for convenience but is consumed by `runner.rs` for proof enrichment, not by `build_system_prompt()`.
- **Low-power mode changes agent behavior**: When `ctx.low_power` is true, the prompt adds a warning to use the cheapest model and minimize tool calls. This is a soft constraint — the LLM may or may not comply.
- **Basket health is data, not alarms**: UTXO counts per basket are rendered as raw numbers in the "On-Chain State" subsection. No warning thresholds — token accumulation across tasks is expected and intentional (each task leaves TaskCommitment + CapabilityDeclaration as historical on-chain state). Only rendered when `basket_health` is non-empty.
- **Wallet section discourages redundant tool calls**: The prompt includes an IMPORTANT block telling the LLM that balance, spendable output count, and on-chain state are already in the prompt. This prevents wasteful `wallet_balance` / `wallet_call(listOutputs)` calls that just repeat what's already shown.
- **Available files capped at 20**: The environment section lists at most 20 context files to avoid bloating the system prompt. Overflow gets a count summary.
- **Offload counter is per-session**: `offload_counter` increments monotonically per `ContextManager` instance, not globally. Multiple tasks in the same process share the counter.
- **Token estimation includes 4-token overhead per message**: Each message gets +4 tokens for OpenAI framing (role, delimiters). Tool call JSON is serialized and estimated separately.
- **Truncation can truncate mid-content**: When even the last 8 messages exceed the budget, `truncate_messages()` cuts content mid-string and appends `[truncated]`. The minimum preserved content is 100 chars.
- **`compact_content` vs `compact_content_with_limit`**: The standalone `compact_content()` uses a fixed 8K limit (for conversation storage). Inside `build_messages()`, `compact_large_messages()` uses the dynamic `max_message_chars()` limit which scales with the model's context window. Don't confuse the two — the fixed wrapper may be more aggressive than the dynamic limit on large-context models.
- **`compact_content_with_limit` is applied to ALL roles**: Not just tool results — user messages, assistant messages, and system messages all get compacted. This catches base64 images embedded in `prior_messages` from conversation history.
- **Data URI stripping is two-pass**: First pass handles markdown image syntax `![alt](data:image/...)`, second pass handles bare `data:image/...` URIs. The two-pass approach avoids regex complexity while catching both forms.
- **Compaction type hints are heuristic**: `compact_content()` guesses the content type (base64 image, JSON, HTML) by checking for magic strings (`/9j/` for JPEG, `iVBOR` for PNG, leading `{`/`[` for JSON, `<!`/`<html` for HTML). These are best-effort hints for the truncation summary, not reliable detection.
- **Offload errors are silently ignored**: Both `fs::create_dir_all` and `fs::write` in `offload_to_file()` use `let _ =`. If the offload dir is unwritable, the function still returns a path to a non-existent file.
- **Sanitization runs even when history fits**: `sanitize_tool_pairs()` is called on the full message list even when no truncation occurs. This handles orphaned pairs from `prior_messages` injection (multi-turn conversations).
- **Hard turn cap injects compaction summary if available**: When `max_history_turns` trims messages and a compaction summary has been set, a `[Earlier conversation summary — N messages compacted]` system message is injected. Without a summary, the trim is silent — no marker is inserted.
- **Compaction functions are standalone, not methods**: `pre_compaction_memory_flush` and `generate_compaction_summary` are free async functions in `manager.rs`, not methods on `ContextManager`. They need `AuthriteClient`, `WormConfig`, and `MemoryStore` which the manager doesn't own. The caller (runner.rs) orchestrates: check `needs_compaction()` → call flush + summary → `set_compaction_summary()` → then `build_messages()`.
- **Compaction memory entries are tagged**: Entries from `pre_compaction_memory_flush` are stored in the `Session` category with tags `["compaction", "{conversation_id}"]` and source `"compaction-{conversation_id}"`. This makes them searchable and attributable to specific conversations.
- **Output token reservation is subtracted from input budget**: `build_messages()` computes `budget = max_tokens - output_tokens` when no explicit `budget_tokens` is passed. If the caller provides `budget_tokens`, `output_tokens` is not applied — the caller is assumed to have already accounted for it.
- **Dynamic message limit depends on output token reservation**: `max_message_chars()` computes `input_budget = max_tokens - output_tokens`, so changing the output token reservation via `set_output_tokens()` also changes the per-message compaction threshold. A larger output reservation means a smaller input budget, which means more aggressive per-message compaction.
- **Fork deep-clones everything**: `ContextFork::new()` clones both `PromptContext` and the full message array. For large histories this is non-trivial — the fork should be short-lived.
- **`is_memory_intensive_skill` parses YAML**: Uses `serde_yaml::from_str()` on each call. Returns false on invalid YAML rather than erroring.
- **Fork summary concatenates all results**: `ContextFork::summary()` joins all results with `[label] content` format. If no results are recorded, returns an empty string.

## Related

- [Root CLAUDE.md](../../CLAUDE.md) — project architecture and conventions
- `src/runner/` — owns `ContextManager`, calls `build_messages()` before each `think()` call; uses `ContextFork` for memory-intensive skills
- `src/think.rs` — receives the built message list and sends it to the LLM
- `src/tools/` — tool results flow back through the context manager for potential offloading
- `src/skills/` — `SkillRegistry::format_for_prompt()` produces the `skills_section` string; `memory_intensive: true` frontmatter triggers context forking
- `src/session/conversation.rs` — `prior_messages` from conversations are injected into history before `build_messages()`
- `src/certificates.rs` — produces `CertificateInfo` for the prompt context
- `src/sanitize.rs` — 4-layer injection defense; `section_external_warning()` is the prompt-level layer
- `src/memory/` — provides `identity_soul` content (Knowledge category, tag "identity"), `auto_recall_ids`, and memory dumps for context forking
