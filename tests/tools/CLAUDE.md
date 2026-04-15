# tests/tools

> 163 tests across 4 files covering tool registry, sandbox tools, browser automation, introspection, and parallel execution.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_tools.rs | 61 | Tool registry, sandbox tools, wallet tools, ALWAYS_ON_TOOLS, file search exclusions, capability enforcement |
| test_browser.rs | 59 | Browser config, tool registration, role classification, AX tree formatting, URL validation, action validation, PDF templates, XSS prevention |
| test_introspect.rs | 34 | Introspect tool: `recent_proofs`, `task_costs`, `task_detail`, `activity_summary`, summary fields — all filesystem-based, no wallet needed |
| test_parallel_tools.rs | 9 | Parallel tool execution via `JoinSet`: concurrency timing, result ordering, panic recovery, error isolation |

## test_tools.rs — Registry, sandbox, wallet, capabilities (61 tests)

### Test groups

**ToolRegistry basics** (7 tests): `register`, `get`, `execute`, `execute_unknown_tool`, `allowlist`, `list_descriptions`, `to_openai_tools`, `tool_names`.

**Sandbox tools** (14 tests): `execute_bash` (echo, exit code, timeout, empty), `file_read` (read/write, not found, offset/limit, size guard), `file_write` (workspace resolution, absolute path, nested relative), `file_search` (glob patterns), `all_sandbox_tools` count assertion (7 tools).

**Web fetch** (4 tests): URL validation (missing, empty, bad scheme), category assertion.

**Wallet tools** (4 tests): Count assertion (8 tools), `wallet_call` registration, missing endpoint error, `createAction` not blocked.

**ALWAYS_ON_TOOLS** (3 tests): Count (15), essentials present (`execute_bash`, `memory_search`, `wallet_balance`, `x402_call`, `search_tools`), discoverable tools excluded (`wallet_encrypt`, `send_message`, `create_schedule`, `generate_image`).

**Prompt tool filtering** (3 tests): `list_prompt_tools` filters to ALWAYS_ON_TOOLS, respects allowlist, `all_tool_summaries` ignores allowlist.

**File search exclusions** (8 tests): `SEARCH_EXCLUDED_DIRS` contains `target`, `node_modules`, `.git`. `is_excluded_path()` correctly filters (target, node_modules, .git, normal paths). Glob and content grep skip excluded directories (target, node_modules).

**Capability enforcement** (18 tests): `required_capability()` maps categories to capabilities (sandbox/system/browser/x402 → `"tools"`, wallet → `"wallet"`, messagebox/conversation → `"messaging"`, memory → `"memory"`, schedule → `"schedule"`, unknown → `"tools"`). Execute with matching/missing/empty/`"all"` capabilities. `None` skips check (backward compat). Empty capabilities block all. Multiple capabilities. Capability check runs after allowlist. `tools+llm` cannot call messagebox.

### Key imports

```rust
use bsv_worm::tools::registry::{ToolDef, ToolRegistry, ALWAYS_ON_TOOLS, required_capability};
use bsv_worm::tools::sandbox::{all_sandbox_tools, is_excluded_path, SEARCH_EXCLUDED_DIRS};
use bsv_worm::tools::wallet_tools::all_wallet_tools;
```

## test_browser.rs — Browser automation + PDF templates (59 tests)

No Chrome required — tests validate config, parsing, formatting, and action validation only.

### Test groups

**Config** (4 tests): `BrowserConfig::default()` values (enabled, headless, 30s timeout, 3 max pages, 1280x720), presence in `WormConfig`, TOML deserialization with partial overrides, hot reload via `reload_safe_fields`.

**Tool registration** (4 tests): `all_browser_tools` returns 1 tool when enabled / 0 when disabled, `browser` not in `ALWAYS_ON_TOOLS`, parameter schema includes `action` (required), `url`, `ref`, `text`, `expression`, `page`.

**Role classification** (4 tests): `is_interactive_role()` true for button, link, textbox, checkbox, radio, combobox, tab, menuitem, switch, slider, searchbox, spinbutton. `is_noise_role()` true for none, generic, presentation, InlineTextBox, LineBreak.

**Text truncation** (4 tests): Short/exact strings unchanged, long strings truncated with `...`, empty string handled.

**URL validation** (7 tests): `https://` and `http://` allowed. `file://`, `javascript:`, `data:`, `ftp://` rejected with "not allowed". Invalid strings rejected.

**Accessibility tree formatting** (11 tests): `format_ax_tree()` produces page header line, assigns `[eN]` refs to interactive elements with `backend_node_id`, skips noise roles (promoting children), renders heading levels, textbox values, indentation by depth, truncates long text. Non-interactive elements and elements without `backend_node_id` get no ref.

**Action validation** (14 tests): `BrowserManager::execute()` without Chrome. Missing/unknown action errors. Navigate requires URL, rejects `file://` and `javascript:` schemes. Click requires `ref`, stale refs produce "not found" + snapshot hint. Type requires `ref` and `text`. Select requires `ref` and `value`. Evaluate requires `expression`. Snapshot/close on nonexistent page handled.

**External tool allowlist** (1 test): `browser` not in `external_tool_allowlist()`.

**PDF templates** (10 tests): `render_audit_html()` produces valid HTML with task ID, summary stats, proof txids/types/hashes, tool calls, iteration display, XSS-escaped user content. `render_budget_html()` includes service breakdown, operation details, sats totals, XSS-escaped service names. Empty events produce valid HTML. Content-type assertion.

### Key imports

```rust
use bsv_worm::config::{BrowserConfig, WormConfig};
use bsv_worm::tools::browser_tools::{
    all_browser_tools, format_ax_tree, is_interactive_role, is_noise_role,
    truncate_text, validate_url, AXTreeNode, BrowserManager,
};
use bsv_worm::tools::registry::ALWAYS_ON_TOOLS;
use bsv_worm::server::pdf_template::{html_escape, render_audit_html, render_budget_html};
use bsv_worm::sanitize::external_tool_allowlist;
```

## test_introspect.rs — Self-inspection tool (34 tests)

Pure filesystem tests using `tempfile`. The `introspect` tool reads JSONL transcripts from `workspace/tasks/{task_id}/session.jsonl` and budget entries from `workspace/budget.jsonl`.

### Actions tested

**`recent_proofs`** (6 tests): Basic proof extraction from transcripts, cross-task aggregation (sorted by timestamp descending), count limit, empty workspace, task ID inclusion in results, count clamped to MAX_COUNT (20).

**`task_costs`** (6 tests): Basic cost aggregation (LLM + proof + tool sats), most-recent-first sorting, count limit, model name inclusion, empty workspace, tool spending (`sats_paid` from `tool_result` events).

**`task_detail`** (7 tests): Full task breakdown (task name, model, status, total sats, prompt/completion tokens), proof list with txids, deduped tools_used list, not-found error, required `task_id` param, per-iteration breakdown, `duration_ms` calculation.

**`activity_summary`** (4 tests): Aggregate totals (tasks, proofs, sats), empty workspace, service breakdown (LLM vs proofs from transcripts), budget journal integration (reads `budget.jsonl`), budget-journal-only scenario.

**Edge cases** (5 tests): Malformed JSONL lines skipped, missing optional fields handled gracefully (no panics), unknown action returns error, default count (5) used when omitted, `session.jsonl` fallback for legacy transcript naming.

**Summary fields** (5 tests): Each action returns a human-readable `summary` string. `recent_proofs` summary includes count and proof types (+ empty case). `task_costs` summary includes task count, total sats, task ID, and status. `task_detail` summary includes task ID, name, status, total sats, proof count, and tools used. `activity_summary` summary includes task count, proof count, total sats, and service breakdown.

**Registration** (1 test): Tool name is `"introspect"`, category is `"introspect"`, description mentions all 4 actions.

### Helpers

| Helper | Purpose |
|--------|---------|
| `create_test_workspace()` | Returns `(TempDir, PathBuf)` with `tasks/` subdir |
| `write_task_transcript()` | Writes JSONL events to `workspace/tasks/{task_id}/session.jsonl` |
| `write_budget_log()` | Writes JSONL entries to `workspace/budget.jsonl` |
| `call_introspect()` | Creates tool via `create_introspect_tool()`, executes with params, parses JSON result |

### Key imports

```rust
use bsv_worm::tools::introspect_tools::create_introspect_tool;
```

## test_parallel_tools.rs — JoinSet concurrency (9 tests)

Verifies that multiple tool calls execute concurrently via `tokio::task::JoinSet`, matching the pattern used in `runner/step.rs`.

### Test groups

**Sequential path** (2 tests): Single tool call executes fast (no JoinSet overhead). Zero tool calls produce empty results without panic.

**Parallel execution** (4 tests): Two 100ms sleep tools complete in ~100ms wall clock (not 200ms). Three 50ms tools complete in ~50ms. Results reordered by original index regardless of completion order. Each parallel result contains correct tool-specific data.

**Error handling** (3 tests): Panicking tool produces `JoinError` while good tool still succeeds. Failing tool (returns error string) doesn't block other tools. Unknown tool returns `Err("Unknown tool")`.

### Helpers

| Helper | Purpose |
|--------|---------|
| `make_echo_tool(name)` | Tool that returns `{"tool": name, "params": ...}` |
| `make_sleep_tool(name, ms)` | Tool that sleeps N ms then returns `{"tool": name, "slept_ms": N}` |
| `make_failing_tool(name)` | Tool that returns `"Error: intentional test failure"` |
| `build_registry(tools)` | Creates `Arc<RwLock<ToolRegistry>>` from a `Vec<ToolDef>` |
| `execute_tool(tools, name, args)` | Standalone tool execution (mirrors `step.rs` pattern) |

## Running tests

```bash
# All tools tests
cargo test --test test_tools
cargo test --test test_browser
cargo test --test test_introspect
cargo test --test test_parallel_tools

# By keyword across all test files
cargo test capability         # Capability enforcement tests
cargo test browser            # Browser + PDF tests
cargo test introspect         # Introspect tool tests
cargo test parallel           # Parallel execution tests

# Specific test function
cargo test --test test_tools -- test_execute_with_missing_capability
cargo test --test test_browser -- test_format_ax_tree_nested_structure
```

## Conventions

- **No Chrome needed**: Browser tests validate config, parsing, AX tree formatting, and action validation only — no browser process launched
- **No wallet needed**: All tests use mocked or filesystem data
- **Tempfile for isolation**: `TempDir` for workspace directories, dropped at test end
- **Timing assertions**: Parallel tool tests use wall-clock timing with generous margins (~80% of sequential time as upper bound)
- **PDF tests are in test_browser.rs**: PDF HTML template tests (`render_audit_html`, `render_budget_html`) live alongside browser tests since both relate to browser/rendering functionality

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Parent test directory overview and conventions
- [src/tools/CLAUDE.md](../../src/tools/CLAUDE.md) -- Tool definitions and registry implementation
- [src/server/CLAUDE.md](../../src/server/CLAUDE.md) -- Server handlers including PDF endpoints
