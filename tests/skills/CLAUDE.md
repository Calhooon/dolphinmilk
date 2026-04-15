# skills/

> 75 tests across 4 files covering SKILL.md parsing, registry operations, skill-local state persistence, pre-tool-use hooks, dependency composition, and activation telemetry.

## Overview

Tests for the skills subsystem: loading SKILL.md files with YAML frontmatter, registering them in a `SkillRegistry`, injecting auto-activated skills into the system prompt (with conditional messaging exclusion), persisting per-skill config and data, composing skills via dependency graphs, intercepting tool calls with priority-ordered hooks, and tracking activation telemetry.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_skills.rs | 25 | SKILL.md parsing, `SkillRegistry` CRUD, `format_for_prompt`, directory loading, system prompt integration, conditional messaging exclusion |
| test_skill_state.rs | 21 | `SkillState` config.json loading, `read_data`/`write_data` persistence, data directory isolation between skills, edge cases |
| test_skill_hooks_composition.rs | 16 | `PreToolUseHook` registration and priority-ordered firing, `HookResult` variants (Allow/Deny/Modify), dependency parsing, cycle detection, `activate_with_deps` topological ordering |
| test_skill_telemetry.rs | 13 | `SkillTelemetry` activation counting, timestamp tracking, context cap (max 10), `SkillActivationStats` serde, transcript `skill_activated` event, telemetry merge |

## Key imports

| Type / Function | Module | Purpose |
|----------------|--------|---------|
| `Skill` | `bsv_worm::skills` | Core skill struct: name, description, auto_activate, tools, instructions, source, state, dependencies |
| `SkillRegistry` | `bsv_worm::skills` | Registry with register, find (case-insensitive), auto_activated, format_for_prompt, hooks, telemetry |
| `SkillState` | `bsv_worm::skills` | Per-skill state: optional `config` (serde_json::Value), `data_dir` (PathBuf), `read_data`/`write_data` |
| `SkillDependencies` | `bsv_worm::skills` | Required and optional dependency lists parsed from SKILL.md frontmatter |
| `parse_skill_content` | `bsv_worm::skills::loader` | Parses SKILL.md string into `Skill` — YAML frontmatter + markdown body |
| `load_skills_from_dir` | `bsv_worm::skills::loader` | Recursively finds SKILL.md files, loads config.json siblings, returns sorted Vec<Skill> |
| `PreToolUseHook` | `bsv_worm::skills` | Hook descriptor: skill_name, tool_pattern (glob), priority (i32) |
| `HookResult` | `bsv_worm::skills::hooks` | Hook outcome enum: Allow, Deny(reason), Modify(new_input) |
| `validate_required_deps` | `bsv_worm::skills::composition` | Checks required deps exist in available set, returns `Vec<DependencyError>` |
| `detect_cycles` | `bsv_worm::skills::composition` | Detects cycles in dependency graph, returns `Result<(), DependencyError::Cycle>` |
| `activation_order` | `bsv_worm::skills::composition` | Topological sort for dependency-first activation |
| `DependencyError` | `bsv_worm::skills::composition` | Error enum: MissingRequired { skill, missing }, Cycle { path } |
| `SkillTelemetry` | `bsv_worm::skills` | Telemetry accumulator: HashMap<String, SkillActivationStats> |
| `SkillActivationStats` | `bsv_worm::skills` | Per-skill stats: count, last_activated timestamp, contexts (capped at 10) |
| `Transcript` | `bsv_worm::transcript` | JSONL transcript for recording `skill_activated` events |
| `build_system_prompt` | `bsv_worm::context::prompt` | System prompt builder — skills section injected via `PromptContext.skills_section` |
| `PromptContext` | `bsv_worm::context::prompt` | Prompt context struct — skills tests set `skills_section`, `inbox_count`, and `tools`; other fields (`basket_health`, `spendable_output_count`, etc.) are zeroed defaults |
| `ToolDesc` | `bsv_worm::context::prompt` | Tool descriptor (name, description, category) — used in system prompt integration tests |

## Test coverage by area

### Parsing (test_skills.rs)

- Valid SKILL.md with all frontmatter fields (name, description, auto_activate, tools)
- Missing frontmatter, missing `name` field, no closing `---` delimiter
- Minimal skill (only name required), empty body, multiline body
- Frontmatter body newline preservation

### Registry (test_skills.rs)

- `new()`, `register()`, `find()` (case-insensitive), `is_empty()`, `len()`, `all()`
- `auto_activated()` filters by `auto_activate: true`
- `format_for_prompt(inbox_count)` — generates `## Active Skills` / `### <name>` sections
- Empty registry produces empty prompt string
- No auto-activated skills produces empty prompt string
- `load_from_dir()` convenience method

### Directory loading (test_skills.rs)

- Loads SKILL.md from subdirectories, returns sorted by name
- Nonexistent directory returns empty Vec (not error)
- Recursive discovery through nested directories
- Ignores non-SKILL.md files (e.g., README.md)
- Skips SKILL.md files with invalid frontmatter (warns, continues)

### Conditional messaging (test_skills.rs)

- Messaging skill excluded from prompt when `inbox_count == 0`
- Other auto-activated skills (e.g., x402) still appear when messaging excluded
- Messaging-only registry produces empty prompt when no inbox
- `build_system_prompt()` omits `# Skills` section entirely when `skills_section` is empty

### Skill state (test_skill_state.rs)

- `SkillState::default()` has `config: None`, empty `data_dir`
- `config.json` loaded as `serde_json::Value` alongside SKILL.md
- Invalid JSON produces `config: None` (warned, not fatal)
- Nested JSON objects, arrays, booleans parsed correctly
- Empty JSON object `{}` is valid config (not None)
- `data_dir` convention: `working/skills/{name}/`
- Data directory created lazily on first `write_data()` call
- `read_data()` / `write_data()` round-trip, overwrite, empty content
- `read_data()` returns `None` for nonexistent files
- Data persists across separate `SkillState` instances pointing at same directory
- Skills have isolated data directories — skill A cannot read skill B's files
- `Clone` preserves config and data_dir

### Hooks (test_skill_hooks_composition.rs)

- `register_hook()` with `PreToolUseHook` descriptor and closure
- Priority-ordered firing: lower priority fires first (-5 before 0 before 10)
- `HookResult::Deny` blocks execution and stops further hooks from firing
- `HookResult::Modify` transforms tool input (e.g., injecting fields)
- Tool pattern matching: `"*"` matches all, `"wallet_*"` matches wallet tools, `"execute_bash"` matches exact
- `remove_hooks_for_skill()` removes all hooks registered by a skill name

### Composition (test_skill_hooks_composition.rs)

- SKILL.md `requires` and `optional` fields parsed into `SkillDependencies`
- Skills without dependencies have empty requires/optional and `has_dependencies() == false`
- `validate_required_deps()` succeeds when all required deps available
- `validate_required_deps()` returns `DependencyError::MissingRequired` for missing required deps
- Missing optional deps do not produce errors
- `detect_cycles()` finds circular dependencies (a→b→c→a)
- Diamond-shaped graphs (a→b→d, a→c→d) are not cycles
- `activate_with_deps()` returns dependency-first topological order (memory, web-search, research)
- Standalone skills activate as single-element list
- `load_from_dir()` preserves dependency metadata
- `validate_dependencies()` logs warnings for missing deps without panicking

### Telemetry (test_skill_telemetry.rs)

- `SkillTelemetry::default()` starts empty
- `record_activation()` increments count, updates timestamp, appends context
- Multiple skills tracked independently
- Unknown (unregistered) skill names still tracked
- Context list capped at 10 entries (FIFO eviction)
- `SkillActivationStats` serde round-trip (count, last_activated, contexts)
- `skill_activated` transcript event with skill_name and context data fields
- Auto-activated vs explicit activation recording
- Telemetry merge pattern: accumulate per-task stats into global totals

## Helpers

| Helper | File | Purpose |
|--------|------|---------|
| `create_test_skill(dir, name, config)` | test_skill_state.rs | Creates skill directory with SKILL.md and optional config.json |
| `make_skill(name)` | test_skill_hooks_composition.rs | Minimal `Skill` with defaults |
| `make_skill_with_deps(name, requires, optional)` | test_skill_hooks_composition.rs | Skill with dependency lists |
| `make_skill(name, auto_activate)` | test_skill_telemetry.rs | Skill with configurable auto_activate flag |

## Running

```bash
# All skills tests
cargo test --test test_skills
cargo test --test test_skill_state
cargo test --test test_skill_hooks_composition
cargo test --test test_skill_telemetry

# By keyword
cargo test skill                      # Matches all 4 files
cargo test parse_skill                # Parsing tests only
cargo test skill_state                # State persistence tests
cargo test hook                       # Hook registration and firing
cargo test telemetry                  # Activation tracking
cargo test dependency                 # Composition/dependency tests
cargo test cycle                      # Cycle detection
```

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Parent test directory overview and conventions
- [src/CLAUDE.md](../../src/CLAUDE.md) -- Source module documentation
- [src/context/CLAUDE.md](../../src/context/CLAUDE.md) -- System prompt builder (skills_section injection)
- [skills/](../../skills/) -- Runtime SKILL.md files loaded by the skill system
