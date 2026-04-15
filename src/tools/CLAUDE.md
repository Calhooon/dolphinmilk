# Tools
> Stateless tool closures the agent LLM invokes via OpenAI function calling.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 17 | Re-exports all 15 submodules (including `browser_tools`, `discovery_tools`, `orchestration_tools`, `fleet_tools/`, and `x402_tools/`) |
| `registry.rs` | 547 | `ToolDef` struct (with `cleanup`, deferred loading fields), `ToolRegistry`, `ALWAYS_ON_TOOLS` constant, `required_capability()`, `DeferralMode` enum, `ToolSearchResult`, `ToolSnapshot` — register, lookup, execute (with capability enforcement), allowlist, prompt filtering, deferred schema loading, OpenAI format, cleanup lifecycle |
| `sandbox.rs` | 662 | `execute_bash`, `file_read`, `file_write`, `file_search`, `web_fetch`, `read_tool_output`, `continue_task` — 7 tools + path/command protection + proportional output capping + search exclusions + file size guard |
| `wallet_tools.rs` | 533 | 8 wallet tools — balance, identity, encrypt, decrypt, check_certificates, receive_address, fund_from_tx, wallet_call. Phase 2 consolidation (14→6 core + 2 deposit workflow tools). |
| `memory_tools.rs` | 392 | `memory_store`, `memory_search` — 2 tools + post-store processing (dedup, tag extraction, quality validation) (includes inline tests) |
| `messagebox_tools.rs` | 505 | `send_message`, `check_inbox`, `set_permission` — 3 tools + BRC-77 signing + BRC-78 encryption + BRC-52 certificate proofs + turn-taking envelope + inbox fee/block management (includes inline tests) |
| `x402_tools/` | 2688 (4 files) | 3 generic x402 tools + 2 recipe tools + `do_x402_request()` helper. Split: `mod.rs` (405), `call.rs` (1086), `discovery.rs` (691), `recipe.rs` (506). Has its own `CLAUDE.md`. |
| `schedule_tools.rs` | 686 | `create_schedule`, `list_schedules`, `cancel_schedule` — 3 tools + `Schedule` struct + `ScheduleType` enum (interval/cron/once) + `parse_interval()` + `compute_next_cron_run()` (includes inline tests) |
| `conversation_tools.rs` | 121 | `list_conversations`, `read_conversation` — 2 tools for agent self-introspection of conversation history |
| `browser_tools.rs` | 1225 | `browser` — 1 tool with 8 actions (navigate, snapshot, click, type, select, evaluate, screenshot, close). Headless Chrome via chromiumoxide CDP. `BrowserManager` with cleanup lifecycle. |
| `discovery_tools.rs` | 235 | `discover_agent`, `verify_agent` — 2 BRC-56 peer discovery tools. Discoverable, NOT always-on. (includes inline tests) |
| `introspect_tools.rs` | 1192 | `introspect` — 1 tool with 5 actions (recent_proofs, task_costs, task_detail, activity_summary, verify_my_state). Self-querying proofs, costs, and task history. Discoverable, NOT always-on. (includes inline tests) |
| `analytics_tools.rs` | 113 | `cost_analysis` — 1 tool with 4 actions (summary, roi, benchmarks, compare). Spending pattern analysis via `crate::analytics`. Discoverable, NOT always-on. |
| `verification_tools.rs` | 222 | `verify_output` — 1 tool for analyzing previous tool outputs for consistency. Structural checks (error detection, JSON validity, non-empty, tool-specific). Discoverable, NOT always-on. (includes inline tests) |
| `orchestration_tools.rs` | 259 | `spawn_agent`, `check_agent`, `list_agents`, `kill_agent` — 4 sub-agent orchestration tools. Discoverable, NOT always-on. Uses `AgentSpawner` trait via `Arc<Mutex<>>`. |
| `fleet_tools/` | 885 (3 files) | `fleet_status` — 1 tool for multi-agent fleet status aggregation. Split: `mod.rs` (332), `status.rs` (351), `types.rs` (202). Has its own `CLAUDE.md`. (includes inline tests) |

**Total: 41 static tools across 15 categories.** Plus `search_tools` (registered in `runner/`) = 42 total. 15 always-on in prompts, 27 discoverable via `search_tools`. The x402 skill (`skills/x402/SKILL.md`, auto-activated) teaches the agent how to discover and use any x402 service via the 3 generic tools. Recipe tools provide atomic workflows for common services.

## Registry (`registry.rs`)

`ToolDef` is the core struct:

```rust
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,       // JSON schema for OpenAI function calling
    pub execute: ToolFunc,       // Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output = String>>>>
    pub category: String,        // sandbox, system, wallet, memory, messagebox, x402, schedule, conversation, browser, discovery, introspect, analytics, verification, orchestration, fleet
    pub cleanup: Option<CleanupFunc>,  // Called at task teardown (e.g. shut down Chrome)
    pub deferred: bool,          // Schema deferred — only hint in prompt, discovered via search_tools
    pub always_load: bool,       // Override deferred — always include full schema
    pub search_hint: Option<String>, // ~10-token hint for deferred tools
}
```

Builder pattern: `ToolDef::new(name, desc, params, execute, category)` sets defaults (`deferred: false`, `always_load: false`, `search_hint: None`). Chain with `.with_deferred(true)`, `.with_always_load(true)`, `.with_search_hint("...")`, `.with_cleanup(func)`.

`CleanupFunc` type: `Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>`. Used by the browser tool to shut down Chrome on task end.

**Deferred loading types**:

`DeferralMode` enum controls how tool schemas enter the LLM context:
- `Always` — all non-`always_load` tools are deferred (hints only)
- `Auto { threshold_pct }` — defer when total schema tokens exceed % of context window (default 50%)
- `Never` — all tools get full schemas (legacy behavior)

`ToolSnapshot` — lightweight search metadata: `name`, `description`, `category`, `deferred`, `always_load`, `hint` (derived from `search_hint` or first sentence of description via `derive_hint()`).

`ToolSearchResult` — scored result from `weighted_search()`: `name`, `description`, `category`, `score`, `match_reasons`.

`ALWAYS_ON_TOOLS` constant (15 tools shown in every system prompt):
- Sandbox (6): `execute_bash`, `file_read`, `file_write`, `file_search`, `web_fetch`, `continue_task`
- Memory (2): `memory_store`, `memory_search`
- Wallet core (3): `wallet_balance`, `wallet_identity`, `wallet_call`
- x402 generic (3): `discover_services`, `discover_endpoints`, `x402_call`
- Discovery (1): `search_tools`

**Capability enforcement** (`required_capability()`): Maps tool categories to BRC-52 certificate capabilities. Used by `execute()` to check if the caller's certificate grants access. Mapping: `sandbox`/`system`/`browser`/`discovery`/`analytics`/`introspect`/`orchestration`/`x402` → `"tools"`, `wallet` → `"wallet"`, `messagebox`/`conversation` → `"messaging"`, `memory` → `"memory"`, `schedule` → `"schedule"`. Default catch-all maps unknown categories to `"tools"`. `"all"` bypasses all checks.

`ToolRegistry` API:
- `register(tool)` — add a tool (warns on overwrites)
- `register_many(tools)` — add multiple tools at once
- `remove(name)` — remove a tool by name
- `has(name)` — check if a tool is registered
- `get(name)` → `Option<&ToolDef>` — lookup by name
- `execute(name, args, capabilities)` → `Result<String, WormError>` — run a tool, checks allowlist and optional capability enforcement. `capabilities: Option<&[String]>` — if `Some`, checks `required_capability()` mapping; `None` skips (CLI, tests, MCP backward compat)
- `is_allowed(name)` — check allowlist (defaults to all-allowed if no allowlist set)
- `set_allowlist(names)` — restrict which tools can execute
- `list_descriptions()` → `Vec<ToolDesc>` — all allowed tools for system prompt construction
- `list_prompt_tools()` → `Vec<ToolDesc>` — only `ALWAYS_ON_TOOLS` intersection with allowed tools
- `all_tool_summaries()` → `Vec<(String, String, String)>` — (name, description, category) for all registered tools, used by `search_tools` snapshot
- `to_openai_tools()` → `Vec<Value>` — OpenAI function calling format
- `tool_schema(name)` → `Option<Value>` — get full JSON schema for a single tool (used by deferred loading)
- `tool_count()` / `tool_names()` — introspection
- `cleanup_all()` — run all registered cleanup functions (e.g. shut down Chrome). Called at task teardown.

Free functions for deferred search: `derive_hint(description)` extracts first sentence as ~10-token hint. `weighted_search(query, snapshots)` scores tools by name/description/category match with bonus for exact matches. `select_tools(query, snapshots, top_k)` returns top-K results.

## Tool Inventory

### Sandbox (5 tools — `sandbox.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `execute_bash` | `command` (req), `timeout`, `cwd` | Run `sh -c` command, max 120s, proportional output cap |
| `file_read` | `path` (req), `offset`, `limit` | Read file, proportional size cap, pre-flight size guard, optional line range |
| `file_write` | `path` (req), `content` (req) | Create/overwrite file, auto-creates parent dirs. **Protected path check.** |
| `file_search` | `pattern`, `directory`, `content_pattern`, `max_results` | Glob + grep search, max 50 results, excluded directories filtered |
| `web_fetch` | `url` (req) | Simple HTTP GET, proportional size cap, 30s timeout. FREE (no auth/payment). |

**Proportional output capping**: Tool result sizes are computed dynamically from the context window via `max_tool_result_chars(context_window_tokens)`. Each tool result is capped at 30% of the context window (in chars, at ~4 chars/token), with a hard ceiling of 400K chars. `truncate_tool_result()` truncates at newline boundaries with a `[truncated]` marker. Constants: `MAX_TIMEOUT = 120`, `MAX_TOOL_RESULT_CONTEXT_SHARE = 0.3`, `HARD_MAX_TOOL_RESULT_CHARS = 400_000`, `CHARS_PER_TOKEN = 4`.

**File read size guard**: `file_read` checks file metadata before reading. Files larger than 4x the output cap are rejected immediately with a size error, preventing multi-GB binaries from being loaded into memory. The error suggests using `offset`/`limit` to read a portion.

**Search exclusions**: `file_search` filters out build artifacts and runtime directories via `SEARCH_EXCLUDED_DIRS`: `target`, `node_modules`, `.git`, `.claude`, `.claude-docs-logs`, `.playwright-mcp`. The `is_excluded_path()` function is `pub` for use by other modules.

**Path protection** (`file_write` only):
- `PROTECTED_PATHS`: `worm.toml`, `Cargo.toml`, `Cargo.lock` — agent cannot overwrite these.
- `BLOCKED_PATTERNS`: `.ssh`, `.gnupg`, `.aws`, `/etc/`, `.env` — agent cannot access paths containing these.
- `is_protected_path(path)` normalizes slashes and checks both lists before any write.

**Command protection** (`execute_bash` only):
- `FORBIDDEN_PATTERNS`: `rm -rf /`, fork bomb, `mkfs.`, `shutdown`, `reboot`, `dd if=/dev/zero`, `chmod -R 777 /`, `curl | sh`, etc.
- `is_forbidden_command(cmd)` normalizes and checks before execution.

### System (2 tools — `sandbox.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `read_tool_output` | `call_id` (req) | Read the full, original output of a previous tool call from the session transcript. Use when a tool result was large and got truncated/previewed. |
| `continue_task` | `reason` (req), `delay_seconds` | Save progress and schedule resumption via heartbeat daemon |

Constructor: `all_sandbox_tools(workspace: PathBuf, context_window_tokens: usize)` returns all 5 sandbox tools plus `read_tool_output` and `continue_task` (7 tools total). The workspace path and context window size are captured via clone.

`read_tool_output` searches the task's JSONL transcript (`transcript.jsonl`, falling back to `session.jsonl`) for a `tool_result` event matching the given `call_id`, and returns the full original content. This lets the agent recover data lost to proportional output truncation.

`continue_task` writes a JSON file to `{workspace}/continuations/{uuid}.json` containing `id`, `reason`, `wake_at` (RFC 3339 if delay > 0, otherwise null), and `created_at`. The heartbeat daemon's `scan_continuations()` picks these up and resumes the task.

### Wallet (8 tools — `wallet_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `wallet_balance` | _(none)_ | Check balance (satoshis + BSV) |
| `wallet_identity` | `protocol_id`, `key_id`, `counterparty` | Get identity key or derive BRC-42 public key |
| `wallet_encrypt` | `plaintext` (req), `protocol_id`, `key_id`, `counterparty` | BRC-42 encrypt → base64 ciphertext |
| `wallet_decrypt` | `ciphertext_base64` (req), `protocol_id`, `key_id`, `counterparty` | BRC-42 decrypt base64 → plaintext |
| `check_certificates` | _(none)_ | Check BRC-52 authorization certificate status (parent-signed, self-signed, or absent) |
| `receive_address` | `suffix` | Generate a BSV receive address via BRC-29 key derivation. Different suffixes produce different addresses. |
| `fund_from_tx` | `txid`, `beef_hex`, `vout`, `suffix` | Internalize an external BSV payment. Two modes: `beef_hex` (direct BEEF from sender) or `txid` (fetch from WhatsOnChain). Suffix must match `receive_address`. |
| `wallet_call` | `endpoint` (req), `params` | Call any BRC-100 wallet endpoint by name (e.g. createAction, createSignature, listActions, getHeaderForHeight) |

**Phase 2 consolidation (14→6) + deposit tools**: The 8 removed tools (`wallet_pay`, `wallet_info`, `wallet_sign`, `wallet_verify`, `wallet_list_actions`, `wallet_list_outputs`, `wallet_create_action`, `wallet_header`) are accessible via `wallet_call` guided by the wallet skill (`skills/wallet/SKILL.md`). Their implementation functions are retained internally. `check_certificates` is kept because it wraps `CertificateManager` logic (not a simple `raw_call`). `receive_address` and `fund_from_tx` were added for the BSV deposit workflow.

**`receive_address`**: Uses BRC-29 protocol with derivation path `[2, "3241645161d8"]` / key ID `"worm-fund {suffix}"` / counterparty `ANYONE_KEY`. Derives a public key, computes P2PKH script, and converts to a Base58Check BSV address via `script_to_address()`. Helper functions `base58_encode()` and `script_to_address()` are defined locally.

**`fund_from_tx`**: Two internalization paths:
1. **Direct BEEF** (`beef_hex`): Decodes hex → optionally wraps in AtomicBEEF header → calls `wallet.internalize_action()` with `paymentRemittance` output descriptor.
2. **WOC fetch** (`txid`): Calls `wallet.fund_from_woc()` which fetches the transaction from WhatsOnChain and internalizes it.

Both paths use the same derivation suffix to match the address generated by `receive_address`.

All wallet tools use a hardcoded `client()` helper that creates `WalletClient::new("http://localhost:3322", "http://localhost", 30)` per call.

### Memory (2 tools — `memory_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory_store` | `content` (req), `category`, `tags`, `source` | Store a memory entry (knowledge/execution/session) |
| `memory_search` | `query` (req), `limit` | BM25 search over memory index (default 5 results) |

Constructor: `all_memory_tools(memory_dir: PathBuf)`. The `PathBuf` is captured via `Arc` by both closures.

`memory_store` writes to disk via `MemoryStore`, runs post-store processing, then indexes via `MemoryIndex`. **Post-store processing** (`memory/processing.rs`): dedup check via BM25 score threshold (searches existing entries before indexing), tag extraction, and content quality validation. All non-fatal — processing errors never prevent the store operation. Response includes optional `warnings`, `possible_duplicate`, and `extracted_tags` fields. Search result snippets are truncated to 500 chars (`MAX_SNIPPET_LEN`). `memory_search` auto-rebuilds the index from files if the index is empty (e.g. after server restart).

### MessageBox (3 tools — `messagebox_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `send_message` | `recipient` (req), `body` (req), `message_box`, `sign`, `encrypt`, `conversation_ref`, `turn`, `max_turns`, `done`, `prove_identity` | Send to another agent's inbox. Validates 66-char hex pubkey. Supports BRC-77 signing, BRC-78 encryption, turn-taking envelopes, and BRC-52 certificate proofs. |
| `check_inbox` | `message_box`, `decrypt` | Check inbox(es). If box specified, checks one; otherwise polls all. `decrypt=true` auto-decrypts BRC-78 and verifies BRC-77 signatures. |
| `set_permission` | `recipient_fee` (req), `message_box`, `sender` | Set a fee or block senders for a MessageBox inbox. `recipient_fee` in sats (0 = free, -1 = block sender). `sender` targets a specific identity key; omit for global fee. |

Constructor: `all_messagebox_tools(wallet_url: String)`. The URL is captured via `Arc`.

MessageBox names: `task_inbox`, `status_inbox`, `results_inbox`, `worm_coordination`.

Internally creates `WalletClient` → `AuthriteClient` → `MessageBoxClient` per call.

**Signing/Encryption dispatch**: `send_message` dispatches based on `sign`/`encrypt` flags to one of four methods: plain, signed-only, encrypted-only, or signed+encrypted. Both default to `false`.

**Turn-taking**: When `conversation_ref` is provided, the body is wrapped in a structured envelope: `{ type: "agent_message", conversation_ref, turn, max_turns, done, body }`. This enables multi-turn agent-to-agent exchanges.

**Identity proofs**: When `prove_identity: true`, a BRC-52 certificate proof is attached via `CertificateManager::prove_authorization()` revealing `name` and `capabilities` fields. Non-fatal if certificate acquisition fails.

**Auto-processing**: `check_inbox` with `decrypt=true` calls `process_received_message()` on each message, which auto-decrypts BRC-78 bodies and verifies BRC-77 signatures. Returns `was_encrypted`, `was_signed`, `signature_valid` metadata. Falls back to raw body with `decrypt_error` on failure.

**Inbox permissions**: `set_permission` delegates to `MessageBoxClient::set_permission()`. Sets a fee (in satoshis) that senders must pay for delivery to the specified inbox, or blocks a specific sender entirely (`recipient_fee: -1`). Defaults to `task_inbox` if `message_box` is omitted. Returns `{ success, message_box, recipient_fee, result }`.

### Conversation (2 tools — `conversation_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `list_conversations` | `limit` | List recent conversations with titles, message counts, cost summaries, and linked task IDs (default 10) |
| `read_conversation` | `conversation_id` (req), `limit` | Read messages from a specific conversation, most recent first (default 20). Content truncated to 500 chars. |

Constructor: `all_conversation_tools(workspace: PathBuf)`. The workspace path is cloned for each closure.

These tools let the agent introspect its own conversation history via `ConversationManager`. `list_conversations` returns metadata (id, title, message_count, total_sats, updated_at, task_ids). `read_conversation` returns message details (seq, role, content, task_id, ts) in chronological order, taking the last N messages.

### Discovery (2 tools — `discovery_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `discover_agent` | `identity_key`, `attributes`, `limit` | Find agents by identity key (66-char hex) or certificate attributes (BRC-56). At least one of `identity_key` or `attributes` required. |
| `verify_agent` | `identity_key` (req) | Verify an agent's BRC-52 certificates. Returns certificate count, types, certifiers, and parent-signed status. |

Constructor: `all_discovery_tools(wallet_url: String)`. The URL is captured via `Arc`.

**Discoverable, NOT always-on.** Found via `search_tools`. Uses `PeerDiscovery` from `src/discovery.rs` which wraps wallet's `discoverByIdentityKey` and `discoverByAttributes` endpoints.

### x402 Generic (3 tools — `x402_tools/`)

| Tool | Parameters | Cost | Description |
|------|-----------|------|-------------|
| `discover_services` | `category` | FREE | List available x402 agents from registry |
| `discover_endpoints` | `agent` (req) | FREE | Get detailed endpoint info for a service (pricing, input schemas, delivery mode). Auto-appends provider tips. Also resets circuit breaker for this provider. |
| `x402_call` | `service` (req), `method`, `parameters`/`body` | varies | Generic x402 call with dynamic resolution, auto-poll, circuit breaker, and validation |

Constructor: `all_x402_tools(wallet_url: String)` returns 3 tools. Shared state across calls: `discovered` (HashSet), `failures` (HashMap), `manifests` (ManifestCache), `tips_shown` (HashSet) — all `Arc<Mutex<>>`.

The x402 tools directory has its own [CLAUDE.md](x402_tools/CLAUDE.md) with full details on `x402_call` features (auto-poll, circuit breaker, provider validation, stray param collection, error enrichment), provider tips system, and the `do_x402_request()` shared helper.

### x402 Recipe (2 tools — `x402_tools/recipe.rs`)

| Tool | Parameters | Cost | Description |
|------|-----------|------|-------------|
| `generate_image` | `prompt` (req), `resolution`, `aspect_ratio` | ~$0.19 | Image gen via banana. Auto-polls until completion. Returns URL. |
| `upload_to_nanostore` | `content`, `file_path`, `retention_minutes`, `content_type` | ~730 sats/MB/yr | Two-step upload: reserve slot + PUT content. Accepts inline content or a file path. Auto-detects MIME type. Returns public URL + SHA-256 hash. |

Constructor: `all_x402_recipe_tools(wallet_url: String)` returns 2 tools (separate from `all_x402_tools()`).

Recipe tools handle multi-step flows atomically: `generate_image` auto-polls the banana async-poll endpoint; `upload_to_nanostore` does reserve + PUT via curl subprocess (computes SHA-256 content hash). `upload_to_nanostore` accepts either inline `content` or a `file_path` (resolved from project root), and auto-detects MIME `content_type` from file extension or content when omitted.

### Schedule (3 tools — `schedule_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `create_schedule` | `description` (req), `schedule_type`, `interval`, `cron_expression`, `conversation_id` | Create a scheduled task. Three types: interval (default), cron, once. |
| `list_schedules` | _(none)_ | List all schedules sorted by next_run. |
| `cancel_schedule` | `id` (req) | Disable a schedule (sets `enabled: false`, preserves file). |

Constructor: `all_schedule_tools(workspace: PathBuf)` returns 3 tools.

**`ScheduleType` enum**: `Interval` (default, backward-compatible), `Cron` (5-field cron expression via `croner` crate), `Once` (fires once then auto-disables via `one_shot` flag).

**`Schedule` struct**: `id` (`sched-{uuid}`), `description`, `schedule_type`, `interval_secs`, `cron_expression` (optional), `next_run` (RFC 3339), `conversation_id` (optional), `created_by`, `enabled`, `created_at`, `last_run` (optional), `run_count`, `one_shot`. Backward-compatible: old schedules without `schedule_type`/`cron_expression`/`one_shot` deserialize with defaults.

**`parse_interval(input)`**: Converts human-friendly intervals to seconds. Suffixes: `s` (seconds), `m` (minutes), `h` (hours), `d` (days). Examples: `"30m"` → 1800, `"1h"` → 3600, `"7d"` → 604800. Rejects zero and invalid suffixes.

**`compute_next_cron_run(expr)`**: Validates a 5-field cron expression and returns the next occurrence after now. Uses the `croner` crate.

The `Scheduler` in `heartbeat/` scans `{workspace}/schedules/` on each tick, enqueues due tasks, and updates `next_run`/`last_run`/`run_count`. For cron schedules, `next_run` is recomputed from the cron expression. For once schedules, `enabled` is set to `false` after first run.

### Browser (1 tool — `browser_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `browser` | `action` (req), `url`, `ref`, `text`, `value`, `script`, `selector`, `path` | Headless Chrome automation via CDP. 8 actions: navigate, snapshot, click, type, select, evaluate, screenshot, close. |

Constructor: `all_browser_tools(config: BrowserConfig)` returns 1 tool with a `cleanup` function that shuts down Chrome.

**Discoverable, NOT always-on.** Not in `external_tool_allowlist()` (security). Found via `search_tools`.

**Snapshot-ref-act pattern**: navigate → get accessibility tree with refs (e1, e2, ...) → interact using refs → snapshot again.

**`BrowserManager`**: Manages Chrome lifecycle via `Arc<Mutex<>>`. Lazy-starts Chrome on first use via `ensure_browser()`. `cleanup()` closes all pages and shuts down the browser process.

**Actions**:
- `navigate` — open a URL, returns page title + accessibility tree snapshot
- `snapshot` — get current accessibility tree with interactive refs
- `click` — click an element by ref (e.g. "e5")
- `type` — type text into a textbox ref, with optional `submit` flag
- `select` — select an option in a dropdown ref by value
- `evaluate` — execute JavaScript, returns result
- `screenshot` — capture page screenshot, saves to workspace, returns path
- `close` — close browser and release resources

**Accessibility tree**: `build_accessibility_tree()` fetches the full AX tree via CDP, skips `NOISE_ROLES` (none, generic, presentation, InlineTextBox, LineBreak), assigns refs to `INTERACTIVE_ROLES` (button, link, textbox, checkbox, radio, combobox, tab, menuitem, switch, slider, searchbox, spinbutton). Builds an indented text representation with `[ref]` markers for interactive elements.

**Config**: `BrowserConfig` with `chrome_path` (auto-detected if not set), `headless` (default true), `viewport_width`/`viewport_height` (default 1280x720).

### Introspect (1 tool — `introspect_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `introspect` | `action` (req), `count`, `task_id` | Self-query proofs, costs, and task history. 5 actions: recent_proofs, task_costs, task_detail, activity_summary, verify_my_state. |

Constructor: `create_introspect_tool(workspace: Arc<PathBuf>)` returns 1 tool. Category: `introspect`.

**Discoverable, NOT always-on.** Found via `search_tools`.

**Actions**:
- `recent_proofs` — list recent BRC-18 proofs across all tasks. Returns txid, proof_type, hash, sats_cost, iteration, proof_data per proof. Sorted by timestamp descending. Scans up to `MAX_SCAN_DIRS` (100) task directories.
- `task_costs` — list recent tasks with cost summaries (total_sats, iterations, model, duration_ms, status). Scans up to `MAX_COST_DIRS` (50) task directories.
- `task_detail` — deep dive into a specific task. Returns per-iteration cost breakdown (`per_iteration` array with sats + tools per iteration), proof txids, tools used, token counts (prompt + completion), and duration. Requires `task_id`.
- `activity_summary` — aggregate spending overview. Breaks down total_sats by service category (llm, proofs, x402_services, tools, tokens). Also reads `budget.jsonl` for a complementary budget journal view. Returns recent_task_ids.
- `verify_my_state` — read the agent's proof chain from the blockchain via `proofs::read_proof_chain()`. Returns `chain_length`, `recent_proofs` (txid, hash), `status`, and `last_proof_hash`. The agent can compare these against its own knowledge to detect inconsistencies. Uses WalletClient directly (localhost:3322).

**Pagination**: `count` parameter defaults to `DEFAULT_COUNT` (5), clamped to `MAX_COUNT` (20).

**Transcript reading**: `find_transcript()` tries `transcript.jsonl` first, falls back to `session.jsonl` (legacy). `read_transcript()` parses JSONL, skipping malformed lines with warnings.

### Analytics (1 tool — `analytics_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `cost_analysis` | `action`, `task_id`, `alt_model` | Spending pattern analysis: efficiency metrics, ROI, provider benchmarks, cost comparison. |

Constructor: `all_analytics_tools(workspace: PathBuf)` returns 1 tool. Category: `analytics`.

**Discoverable, NOT always-on.** Found via `search_tools`.

**Actions**:
- `summary` (default) — all efficiency metrics via `crate::analytics::compute_cost_analysis()`
- `roi` — ROI report via `crate::analytics::compute_roi()`
- `benchmarks` — provider performance benchmarks. Auto-backfills from transcripts if no benchmark entries exist. Uses `crate::analytics::aggregate_benchmarks()`.
- `compare` — cost replay for a specific task with an alternative model. Requires `task_id` and `alt_model`. Uses a hardcoded pricing table (sats/Mtok) for major models (GPT-5, GPT-4.1 family, o4-mini, Claude Sonnet/Haiku/Opus).

### Verification (1 tool — `verification_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `verify_output` | `original_tool` (req), `original_params`, `original_output` (req) | Analyze a previous tool result for consistency. Structural checks, not re-execution. |

Constructor: `all_verification_tools()` returns 1 tool (no constructor arguments). Category: `verification`.

**Discoverable, NOT always-on.** Found via `search_tools`.

**Checks performed** by `analyze_output()`:
- `error_detection` — scans for "Error:", `{"error"`, etc.
- `json_validity` — validates JSON structure (only reported if output is valid JSON)
- `non_empty` — ensures output is non-whitespace
- Tool-specific: `memory_search` checks for "results"/"found"; `execute_bash` checks for exit code info.

Returns `{ checks, all_passed, tool_name, output_length }` plus a `verification_note` suggesting re-execution for full verification.

### Orchestration (4 tools — `orchestration_tools.rs`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `spawn_agent` | `task` (req), `budget_sats` (req), `allowed_tools`, `max_iterations`, `model` | Spawn a sub-agent with carved-out budget and optional tool restrictions. Returns child's `task_id`. |
| `check_agent` | `task_id` (req) | Get current status of a child agent (status, iterations, sats_spent, result). |
| `list_agents` | _(none)_ | List all child agents with statuses and budget usage. |
| `kill_agent` | `task_id` (req) | Terminate a running child agent. |

Constructor: `all_orchestration_tools(spawner: Arc<Mutex<dyn AgentSpawner>>)` returns 4 tools. The `AgentSpawner` trait (from `crate::orchestration`) is shared via `Arc<Mutex<>>` so stateless closures can access the task registry and budget pool.

**Discoverable, NOT always-on.** Found via `search_tools`. Category: `orchestration`.

`spawn_agent` creates a `SpawnConfig` from the parameters and delegates to `spawner.spawn()`. `allowed_tools` is an optional array of tool names restricting the child's registry. `budget_sats` is carved from the parent's budget. `check_agent` and `kill_agent` use `TaskId` newtype for type safety.

### Fleet (1 tool — `fleet_tools/`)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `fleet_status` | `members` | Aggregate status across fleet members. Returns member statuses, budget totals, active task count. |

Constructor: `all_fleet_tools()` returns 1 tool (no constructor arguments). Category: `fleet`.

**Discoverable, NOT always-on.** Found via `search_tools`. Has its own [CLAUDE.md](fleet_tools/CLAUDE.md).

**Types** (`fleet_tools/types.rs`): `AgentStatus` enum (Running/Idle/Error/Stopped/Unknown), `FleetMember` struct, `FleetStatus` struct, `TaskAssignment` struct, `TaskPriority` enum (Low/Normal/High).

**Status aggregation** (`fleet_tools/status.rs`): `aggregate_fleet_status()` collects member data, applies `HEARTBEAT_TIMEOUT_SECS` (300) for stale heartbeat detection, aggregates budget totals. `format_fleet_status()` renders as human-readable text.

## Decisions

- **Stateless closures, not trait objects**: Each tool is a `Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync>`. No shared mutable state — the runner owns all state and passes what's needed via captured `Arc`s.
- **All tools return `String`, never `Result`**: Error handling is embedded in the return value (e.g. `"Error: no command provided"`). The runner doesn't need to distinguish success from failure — the LLM reads the text and decides.
- **Category-based registration**: Each file exposes `all_*_tools()` returning `Vec<ToolDef>`. The runner calls each and registers them all into `ToolRegistry`. Categories: `sandbox`, `system`, `wallet`, `memory`, `messagebox`, `x402`, `schedule`, `conversation`, `browser`, `discovery`, `introspect`, `analytics`, `verification`, `orchestration`, `fleet`.
- **ALWAYS_ON_TOOLS constant for prompt filtering**: `list_prompt_tools()` returns only the 15 always-on tools for the system prompt. `all_tool_summaries()` provides snapshots for `search_tools` progressive disclosure. This replaces per-tool `always_on` fields.
- **Cleanup lifecycle for stateful tools**: `ToolDef.cleanup` is `Option<CleanupFunc>`. `cleanup_all()` on `ToolRegistry` runs all registered cleanup functions at task teardown. Currently only used by the browser tool to shut down Chrome.
- **Phase 2 wallet consolidation (14→6) + deposit tools**: `wallet_call` with `raw_call()` replaces 8 dedicated wallet tools. The wallet skill guides the LLM on endpoint usage. `check_certificates` is kept because it wraps `CertificateManager` (not a raw wallet call). `receive_address` and `fund_from_tx` added for the deposit workflow — these require P2PKH address derivation and AtomicBEEF construction that don't map to a simple `raw_call`.
- **Proportional output capping**: Tool results are capped at 30% of the context window (in chars). This replaced fixed constants to adapt to different model context windows.
- **Wallet tools hardcode `localhost:3322`**: The `client()` helper in `wallet_tools.rs` creates a fresh `WalletClient` per call. Other tool modules (messagebox, x402, discovery) take `wallet_url: String` as a parameter to their `all_*_tools()` constructor.
- **Memory tools capture `PathBuf` via `Arc`**: Unlike sandbox/wallet tools which are fully self-contained, memory and messagebox tools close over configuration (memory dir, wallet URL) passed at registration time.
- **Service discovery is a three-tool pattern**: `discover_services` → browse registry, `discover_endpoints` → inspect manifest, `x402_call` → execute. This lets the LLM explore unknown services autonomously.
- **Recipe tools for common workflows**: `generate_image` and `upload_to_nanostore` are atomic wrapper tools that handle polling and two-step uploads internally.
- **`read_tool_output` and `continue_task` in sandbox, categorized as "system"**: Defined in `sandbox.rs` but use category `"system"` since they are agent lifecycle tools, not filesystem operations.
- **`read_tool_output` recovers truncated data**: When proportional output capping truncates a large tool result, the full output is still recorded in the JSONL transcript. `read_tool_output` retrieves it by `call_id`, giving the agent a way to access data that was too large for the context window.
- **Browser tool is discoverable, not always-on**: Keeps the always-on prompt small (15 tools). The browser is found via `search_tools` and not in the external tool allowlist (security boundary for untrusted external messages).
- **x402_tools split into directory**: `call.rs` (x402_call + circuit breaker + auto-poll), `discovery.rs` (discover_services + discover_endpoints + provider tips), `recipe.rs` (generate_image + upload_to_nanostore), `mod.rs` (shared types + do_x402_request + helpers).
- **Deferred schema loading**: `DeferralMode::Auto` (default, 50% threshold) keeps prompt token usage manageable as tool count grows. Tools marked `deferred: true` show only a ~10-token hint in the system prompt. The agent discovers full schemas via `search_tools` → `tool_schema()`. `always_load: true` overrides deferral for essential tools (sandbox, memory, wallet core, x402).
- **Orchestration tools use shared spawner**: `Arc<Mutex<dyn AgentSpawner>>` lets stateless closures access the spawner without breaking the no-shared-mutable-state tool pattern. The spawner holds references to the task registry and budget pool.
- **Capability enforcement via certificate mapping**: `required_capability()` maps tool categories to BRC-52 certificate capabilities. `execute()` checks capabilities when `Some` is passed, allowing certificate-based access control. `None` skips checks for backward compatibility (CLI, tests, MCP). Default catch-all maps unknown categories to `"tools"`.
- **Introspect tool reads transcripts directly**: Parses JSONL transcript files from `workspace/tasks/` instead of going through API endpoints. Supports legacy `session.jsonl` fallback. Per-iteration cost breakdown in `task_detail` is computed by tracking `think_response` boundaries.
- **Analytics tool delegates to `crate::analytics`**: Thin wrapper that calls analytics module functions. Benchmark backfill from transcripts is lazy — only happens when no benchmark entries exist.
- **Verification tool is structural, not re-execution**: `verify_output` analyzes output for consistency patterns rather than re-running the original tool. This avoids side effects and costs from re-execution.
- **Fleet tools as a subdirectory**: Types, status aggregation, and formatting are split into separate files for the same reasons as x402_tools — keeps individual files focused.

## Gotchas

- **Sandbox `execute_bash` runs `sh -c`**: Not bash — portable shell. Max 120s timeout, proportional output truncation. No sandboxing beyond the timeout and forbidden command checks.
- **`file_read` rejects oversized files**: Files larger than 4x the output cap are rejected before reading. This prevents OOM from multi-GB binaries in `target/`.
- **`file_search` excludes build directories**: `SEARCH_EXCLUDED_DIRS` filters `target`, `node_modules`, `.git`, `.claude`, `.claude-docs-logs`, `.playwright-mcp` from both glob and grep results.
- **`file_write` creates parent directories**: Auto-calls `create_dir_all` before writing. Relative paths resolve against the workspace.
- **Registry allowlist defaults to "all registered"**: If `set_allowlist()` is never called, every registered tool is allowed. The allowlist is an opt-in restriction, not a default deny.
- **Default protocol IDs vary by tool**: `wallet_encrypt`/`wallet_decrypt` default to `[2, "worm encryption"]`, and the memory system uses `[2, "worm memory"]`. These are intentionally different key derivation paths.
- **`web_fetch` is a free sandbox tool**: Uses plain `reqwest` GET with no auth or payment. 30s timeout, proportional truncation. URL must start with `http://` or `https://`.
- **`send_message` validates recipient format**: Must be exactly 66 chars (compressed secp256k1 pubkey hex). Validation happens before any network call.
- **`x402_call` stray param collection**: LLMs sometimes put body fields at the top level (alongside `service`/`method`). The tool auto-collects unknown top-level keys into the body. Also auto-parses JSON string bodies.
- **`upload_to_nanostore` retention minimum**: `retention_minutes` must be >= 180 (NanoStore's minimum). Supports both inline `content` and `file_path`; at least one is required.
- **`read_tool_output` is NOT in ALWAYS_ON_TOOLS**: Despite being registered with every task, it's discoverable via `search_tools`. The agent finds it when it needs to recover truncated output.
- **`read_tool_output` searches transcript linearly**: Scans the full JSONL file for a matching `call_id`. No index — adequate for typical transcript sizes but O(n) in event count.
- **`continue_task` continuation files are ephemeral**: Written to `{workspace}/continuations/` as JSON. The heartbeat daemon reads and deletes them after scheduling resumption.
- **Path protection only applies to `file_write`**: The agent can still read protected files via `file_read` and execute arbitrary commands via `execute_bash`. Protection is a guardrail against accidental config overwrites, not a security boundary.
- **Schedule cancel is soft-delete**: `cancel_schedule` sets `enabled: false` but preserves the JSON file for history.
- **Schedule backward compatibility**: Old schedules without `schedule_type`/`cron_expression`/`one_shot` deserialize with defaults (`Interval`, `None`, `false`).
- **`prove_identity` is non-fatal**: If the wallet has no BRC-52 certificates, `send_message` still sends the message — it just logs a warning and omits the proof.
- **`set_permission` fee semantics**: `recipient_fee: 0` = free delivery, positive = required fee in sats, `-1` = block sender. Without `sender`, the fee applies globally to the inbox.
- **Browser tool cleanup is per-registry**: `cleanup_all()` runs all cleanup functions. If the browser was never used (lazy start), its cleanup is a no-op.
- **Browser accessibility tree skips noise roles**: `NOISE_ROLES` (none, generic, presentation, InlineTextBox, LineBreak) are filtered from the tree. Only `INTERACTIVE_ROLES` get clickable refs.
- **Browser is NOT in external tool allowlist**: Tasks from external messages (untrusted senders) cannot use the browser tool. Security boundary enforced by `sanitize.rs`.
- **Capability enforcement is opt-in**: `execute()` only checks capabilities when `Some(&[...])` is passed. CLI, tests, and MCP pass `None` to skip checks. Server tasks pass the certificate's capabilities list.
- **`wallet_call` blocklist is currently empty**: The `BLOCKED` array in `wallet_call` is `&[]` — no endpoints are blocked. All BRC-100 endpoints are accessible.
- **`receive_address` uses BRC-29 with `ANYONE_KEY`**: The derivation uses counterparty `ANYONE_KEY` so anyone can send to the address without knowing the agent's identity key. The `suffix` parameter must match between `receive_address` and `fund_from_tx`.
- **`fund_from_tx` AtomicBEEF wrapping**: When `beef_hex` is provided but not in AtomicBEEF format (missing `0x01010101` header), the tool attempts to prepend the header with the reversed txid. Falls back to raw bytes if txid is not available.
- **`fund_from_tx` requires either `beef_hex` or `txid`**: At least one must be provided. Both can be provided simultaneously (txid used for AtomicBEEF wrapping when beef_hex is raw).
- **Introspect `count` is clamped**: Values above `MAX_COUNT` (20) are silently clamped. Default is 5.
- **Introspect scans task directories by mtime**: `list_task_dirs()` sorts by filesystem modification time, not transcript timestamps. Results may differ from transcript-based ordering.
- **Analytics `compare` pricing table is hardcoded**: Model pricing for cost comparison is embedded in `analytics_tools.rs`, not loaded from config. Must be updated manually when prices change.
- **Verification tool is analysis-only**: `verify_output` does not re-execute the original tool — it performs structural analysis of the output. The `verification_note` field suggests re-execution for full verification.
- **Orchestration tools require spawner injection**: Unlike other tool constructors that take simple `PathBuf`/`String`, `all_orchestration_tools()` requires `Arc<Mutex<dyn AgentSpawner>>`. The spawner must be set up by the runner/server before tools can be registered.
- **`spawn_agent` budget is carved from parent**: The `budget_sats` parameter is deducted from the parent task's budget. If insufficient budget remains, the spawn fails.
- **Deferred tool hints are ~10 tokens**: `derive_hint()` takes the first sentence of the description. Custom hints via `with_search_hint()` should be similarly brief.
- **Fleet `HEARTBEAT_TIMEOUT_SECS` is 300**: Agents not reporting a heartbeat within 5 minutes are flagged as stale.

## Related

- [Root CLAUDE.md](../../CLAUDE.md) — project architecture and conventions
- [x402_tools/CLAUDE.md](x402_tools/CLAUDE.md) — detailed x402 tools documentation
- [fleet_tools/CLAUDE.md](fleet_tools/CLAUDE.md) — fleet tools types, status aggregation, and usage
- `src/runner/` — dispatches tool calls from LLM responses via `ToolRegistry::execute()`; registers `search_tools`; calls `cleanup_all()` at teardown
- `src/wallet.rs` — the `WalletClient` that wallet tools delegate to
- `src/memory/` — `MemoryStore` and `MemoryIndex` used by memory tools
- `src/messagebox/` — `MessageBoxClient` used by messagebox tools
- `src/discovery.rs` — `PeerDiscovery` used by discovery tools
- `src/orchestration/` — `AgentSpawner` trait and `SpawnConfig` used by orchestration tools
- `src/analytics/` — `compute_cost_analysis()`, `compute_roi()`, `load_benchmarks()`, `cost_replay()` used by analytics tools
- `src/x402/payment.rs` — `authenticated_paid_request()` used by x402 tools
- `src/x402/discovery.rs` — `fetch_manifest_from_url()` and `format_manifest_summary()` for service discovery
- `src/x402/registry.rs` — `list_agents()`, `resolve()`, and `resolve_x402_info()` for agent name resolution
- `src/session/conversation.rs` — `ConversationManager` used by conversation tools
- `src/certificates.rs` — `CertificateManager` used by `send_message` and `check_certificates`
- `src/auth/` — `AuthriteClient` used by messagebox and x402 tools for BRC-31 auth
- `src/context/prompt.rs` — `ToolDesc` struct used by `list_descriptions()` and `list_prompt_tools()`
- `src/heartbeat/` — `scan_continuations()` picks up files written by `continue_task`; `Scheduler` runs due schedules
- `src/config/` — `BrowserConfig` used by browser tools
- `skills/x402/SKILL.md` — auto-activated decision tree for x402 service usage
- `skills/x402/providers/*.md` — per-provider experiential knowledge
- `skills/wallet/SKILL.md` — guides `wallet_call` usage for all BRC-100 endpoints
- `skills/browser/SKILL.md` — browser automation skill (snapshot-ref-act pattern)
