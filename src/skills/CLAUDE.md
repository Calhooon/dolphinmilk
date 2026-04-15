# src/skills/
> SKILL.md file loader, registry, hooks, and composition — hot-loadable agent behavior customization without recompilation.

## Overview

Skills are markdown files with YAML frontmatter that inject instructions into the agent's system prompt at runtime. The `SkillRegistry` loads all `SKILL.md` files from a directory tree, validates dependency graphs, and auto-activated skills are formatted into the LLM prompt on every iteration. This lets you change agent behavior by editing text files rather than recompiling Rust code.

Three subsystems extend the base skill concept:
- **Skill-local state** — each skill gets an optional `config.json` (read-only) and a persistent data directory at `working/skills/{name}/` for runtime files
- **Hooks** — `PreToolUseHook` handlers that fire before tool execution, can Allow/Deny/Modify tool inputs
- **Composition** — `requires`/`optional` dependency declarations with cycle detection and transitive auto-activation

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 333 | `Skill`, `SkillState`, `SkillRegistry`, `SkillActivationStats`, `SkillTelemetry` — registration, lookup, dependency validation, hook dispatch, prompt formatting, telemetry |
| `loader.rs` | 226 | `SKILL.md` parser — splits YAML frontmatter from markdown body, recursive directory walker, `config.json` loading, dependency field parsing |
| `composition.rs` | 387 | `SkillDependencies`, `DependencyError`, dependency validation, cycle detection (3-color DFS), activation ordering (topological sort) |
| `hooks.rs` | 458 | `PreToolUseHook`, `HookResult`, `HookCallback`, `RegisteredHook`, `HookRegistry` — glob-pattern tool matching, priority-ordered evaluation, chained modifications |

## Key Exports

### `Skill` (struct)

Loaded skill definition with eight fields:

| Field | Type | Required in YAML | Default |
|-------|------|-----------------|---------|
| `name` | `String` | **yes** | — |
| `description` | `String` | no | `""` |
| `auto_activate` | `bool` | no | `false` |
| `tools` | `Vec<String>` | no | `[]` |
| `instructions` | `String` | — | markdown body after `---` |
| `source` | `String` | — | file path (set by loader) |
| `state` | `SkillState` | — | config from `config.json` + data dir |
| `dependencies` | `SkillDependencies` | no | empty requires/optional |

### `SkillState` (struct)

Persistent skill-local state:

| Field | Type | Purpose |
|-------|------|---------|
| `config` | `Option<serde_json::Value>` | Read-only JSON from `skills/{name}/config.json` |
| `data_dir` | `PathBuf` | Runtime data directory (`working/skills/{name}/`) |

Methods: `read_data(filename)` returns `Option<String>`, `write_data(filename, content)` returns `bool` (creates dir lazily, best-effort).

### `SkillDependencies` (struct, from `composition`)

| Field | Type | YAML Key | Purpose |
|-------|------|----------|---------|
| `requires` | `Vec<String>` | `requires` | Must exist at load time; auto-activated with parent |
| `optional` | `Vec<String>` | `optional` | Used if present, silently ignored if absent |

Method: `has_dependencies()` — whether any deps are declared.

### `SkillRegistry` (struct)

In-memory collection of loaded skills with hook dispatch and telemetry. Fields: `skills: Vec<Skill>`, `telemetry: SkillTelemetry`, `hook_registry: HookRegistry`.

| Method | Description |
|--------|-------------|
| `new()` | Empty registry |
| `register(Skill)` | Add a skill |
| `load_from_dir(dir)` | Load all `SKILL.md` files recursively, sorted by name, then validate dependencies |
| `find(name)` | Case-insensitive lookup |
| `auto_activated()` | Filter to `auto_activate: true` skills |
| `all()` | All registered skills |
| `len()` / `is_empty()` | Count helpers |
| `format_for_prompt(inbox_count)` | Render auto-activated skills as markdown; skips `messaging` when `inbox_count == 0` |
| `validate_dependencies()` | Check missing required deps + cycle detection (logs warnings, graceful degradation) |
| `activate_with_deps(name)` | Activate a skill with transitive required deps in topological order |
| `register_hook(hook, callback)` | Register a PreToolUse hook |
| `remove_hooks_for_skill(name)` | Remove all hooks from a skill |
| `evaluate_hooks(tool_name, input)` | Run matching hooks in priority order → Allow/Deny/Modify |
| `hooks()` | Access the `HookRegistry` |
| `record_activation(name, context)` | Record activation for telemetry (capped at 10 context entries) |
| `telemetry()` | Access `SkillTelemetry` data |

### Telemetry types

- **`SkillTelemetry`** — `activations: HashMap<String, SkillActivationStats>`
- **`SkillActivationStats`** — `count: u64`, `last_activated: Option<String>` (ISO-8601), `contexts: Vec<String>` (capped at 10)

### Hook types (from `hooks`)

- **`PreToolUseHook`** — `skill_name`, `tool_pattern` (glob), `priority` (lower fires first)
- **`HookResult`** — `Allow`, `Deny(reason)`, `Modify(new_input)`
- **`HookCallback`** — `Box<dyn Fn(&str, &serde_json::Value) -> HookResult + Send + Sync>`
- **`RegisteredHook`** — pairs `PreToolUseHook` metadata with its `HookCallback`. Custom `Debug` impl hides the callback.
- **`HookRegistry`** — sorted by priority, methods: `register()`, `remove_by_skill()`, `evaluate()`, `list_hooks()`, `len()`, `is_empty()`

### Composition functions (from `composition`)

| Function | Description |
|----------|-------------|
| `validate_required_deps(name, deps, available)` | Returns `Vec<DependencyError>` for missing required deps |
| `detect_cycles(graph)` | 3-color iterative DFS cycle detection on dependency graph |
| `activation_order(name, graph)` | Topological sort — dependency-first activation order |

### Loader functions (from `loader`)

| Function | Description |
|----------|-------------|
| `load_skills_from_dir(dir)` | Recursively find and parse all `SKILL.md` files; returns sorted `Vec<Skill>` |
| `parse_skill_file(path)` | Parse a single `SKILL.md` + load `config.json` from same directory |
| `parse_skill_content(content, source)` | Parse skill from string (testable without filesystem) |

## SKILL.md File Format

```text
---
name: skill-name
description: What this skill does
auto_activate: true
tools: [tool_a, tool_b]
requires: [other-skill]
optional: [nice-to-have]
---
# Markdown instructions here

These instructions are injected verbatim into the LLM system prompt
when auto_activate is true.
```

Frontmatter rules:
- Must start with `---` and end with `\n---`
- Only `name` is required; all other fields have defaults
- `tools` is a YAML sequence of tool name strings (informational, not enforced)
- `requires`/`optional` are YAML sequences of other skill names
- Body after the closing `---` becomes `instructions`, trimmed of leading/trailing whitespace

Optional companion file: `config.json` in the same directory as `SKILL.md` — arbitrary JSON loaded as `SkillState.config` (read-only after load).

## Hook System

Skills can register `PreToolUseHook` handlers that intercept tool execution before it happens.

**Hook lifecycle:**
1. Skill registers hooks via `registry.register_hook(hook, callback)` during activation
2. Before each tool call, runner calls `registry.evaluate_hooks(tool_name, input)`
3. Hooks matching the tool pattern fire in priority order (lowest first)
4. First `Deny` stops the chain immediately
5. `Modify` results chain — each successive hook sees the previous modification
6. If all hooks return `Allow` (or none match), the tool executes normally

**Pattern matching** (`matches_tool_pattern`):
- `*` — matches everything
- `prefix*` — starts-with
- `*suffix` — ends-with
- `prefix*suffix` — starts-with AND ends-with
- exact match (no wildcards)

## Composition System

Skills can declare dependencies on other skills via `requires` and `optional` frontmatter fields.

**At load time** (`load_from_dir`):
1. All skills loaded from directory
2. `validate_dependencies()` checks that all `requires` deps exist in the registry
3. `detect_cycles()` runs 3-color iterative DFS on the dependency graph
4. Missing deps and cycles produce warnings but don't prevent loading (graceful degradation)

**At activation time** (`activate_with_deps`):
1. `activation_order()` computes topological sort (dependency-first)
2. All skills in the chain get `record_activation()` called
3. Optional deps are informational — not included in activation order

## Usage

### Loading at startup (runner.rs)

```rust
let skills_dir = workspace.join("skills");
let skills = SkillRegistry::load_from_dir(&skills_dir);
```

The runner stores the registry in `WormLoop` and calls `format_for_prompt()` when building the `PromptContext` for each iteration.

### Prompt injection (context/prompt.rs)

`PromptContext.skills_section` receives the pre-formatted string from `format_for_prompt()`. The `section_skills()` function wraps it under a `# Skills` heading. When no auto-activated skills exist, the section is omitted entirely.

The formatted output for an auto-activated skill looks like:

```text
## Active Skills

### messaging
_Cross-wallet communication via BRC-33 MessageBox_
When you receive inbox messages, ALWAYS respond using the `send_message` tool.
...
```

### Current skills (skills/ directory)

| Skill | Auto-activate | Tools | Purpose |
|-------|--------------|-------|---------|
| `browser` | no | `browser` | Headless Chrome automation for JS-heavy sites, form filling, clicking |
| `code-analysis` | no | `file_read`, `file_search`, `execute_bash` | Code review workflow with file:line references |
| `fleet` | no | `fleet_status`, `send_message`, `check_inbox` | Fleet management — spawn, monitor, coordinate child agents via BRC-33/BRC-52 |
| `image-generation` | no | `discover_endpoints`, `x402_call` | AI image generation via x402 with cost guidance |
| `messaging` | **yes** (conditional) | `send_message`, `check_inbox` | Ensures the agent replies via `send_message` instead of text-only responses. Only injected when `inbox_count > 0` |
| `verification` | no | `verify_output` | Double-check outputs by re-running queries and comparing results |
| `wallet` | **yes** | `wallet_call` | Generic bridge to any BRC-100 wallet endpoint via `wallet_call` |
| `x402` | **yes** | `discover_services`, `discover_endpoints`, `x402_call` | Decision tree for discovering and calling paid x402 services. Per-provider tips in `skills/x402/providers/` |

## Decisions

- **Only `name` is required**: All other frontmatter fields default to safe values (`auto_activate: false`, empty tools/description/deps). This makes creating a minimal skill trivial.
- **Auto-activate is opt-in**: Skills default to `auto_activate: false`. Only skills explicitly marked `true` consume system prompt tokens on every iteration. Currently three skills auto-activate: `x402`, `wallet`, and `messaging` (conditional).
- **Sorted by name on load**: `load_skills_from_dir()` sorts skills alphabetically for deterministic prompt ordering across runs.
- **Invalid files are skipped, not fatal**: A malformed `SKILL.md` logs a warning but doesn't prevent other skills from loading.
- **Tools list is informational only**: The `tools` field documents which tools a skill references but the registry doesn't enforce tool availability.
- **Conditional messaging injection**: `format_for_prompt(inbox_count)` skips the `messaging` skill when `inbox_count == 0`. This saves prompt tokens on iterations where there are no pending messages.
- **Dependency validation is graceful**: Missing required deps and cycles produce warnings but don't prevent loading. The agent runs with whatever skills loaded successfully.
- **Hooks are synchronous and side-effect-free**: `HookCallback` is a simple function pointer — no I/O, no network calls. This keeps hook evaluation fast and predictable.
- **Deny stops the chain**: A single `Deny` result from any hook aborts the tool call immediately. No further hooks are evaluated.
- **Chained modifications**: Multiple `Modify` results are applied in sequence — each hook sees the output of the previous one.
- **Skill-local state is best-effort**: `write_data()` creates directories lazily and logs errors but doesn't panic. A skill that can't persist state still functions.

## Gotchas

- **`split_frontmatter` requires `\n---`**: The closing delimiter must be on its own line preceded by a newline. Files without a leading `---` fail with "No YAML frontmatter found".
- **Case-insensitive find but case-preserving storage**: `find("MESSAGING")` matches `"messaging"`, but `skill.name` retains original casing.
- **Non-existent directory returns empty, not error**: `load_skills_from_dir()` on a missing path returns `Ok(vec![])`.
- **Body trimming**: The instructions field is `body.trim().to_string()` — leading/trailing whitespace stripped, interior formatting preserved.
- **No deduplication**: If two `SKILL.md` files declare the same `name`, both are registered. `find()` returns the first match.
- **config.json parse failure doesn't block loading**: If `config.json` exists but is invalid JSON, the skill loads with `config: None`.
- **Hook priority ties**: Equal-priority hooks maintain insertion order (stable sort).
- **Cycle detection is iterative**: Uses explicit stack, not recursion — safe for deep dependency chains.
- **Telemetry contexts capped at 10**: Older activation contexts are evicted FIFO when the cap is reached.
- **Optional deps not in activation order**: `activation_order()` only traverses `requires` edges. Optional deps are informational metadata.

## Related

- [../context/CLAUDE.md](../context/CLAUDE.md) — Prompt construction; `section_skills()` consumes the formatted output
- [../tools/CLAUDE.md](../tools/CLAUDE.md) — Tool definitions that skills reference by name
- [../../skills/](../../skills/) — Runtime skill files (8 skills: browser, code-analysis, fleet, image-gen, messaging, verification, wallet, x402)
- [../CLAUDE.md](../CLAUDE.md) — Root project docs with full architecture overview
- [../../tests/test_skills.rs](../../tests/test_skills.rs) — Tests covering parsing, registry, directory loading, and prompt integration
