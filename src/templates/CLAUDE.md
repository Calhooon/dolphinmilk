# src/templates/
> Pre-configured agent archetypes — TOML-based templates that override select `WormConfig` fields for specific roles.

## Overview

Templates define personality, skills, tools, budget, and LLM defaults for common agent roles (researcher, coder, trader, etc.). They are **optional** and **additive** — a template overrides only non-zero/non-empty fields in `WormConfig`, leaving everything else at the base config values. Template TOML files live in `templates/` at the workspace root; this module provides the schema, loader, validator, and config applicator.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 232 | Re-exports schema types, `load_builtin_templates()` directory scanner, `find_template()` case-insensitive lookup. 7 tests covering empty/missing dirs, valid/invalid TOML, sorted loading, case-insensitive search, and built-in template validation. |
| `schema.rs` | 395 | `AgentTemplate` struct with 6 TOML sections, `load()` / `validate()` / `apply_to_config()` methods, `TemplateError` enum, `ValidationError` struct. 9 tests covering defaults, validation failures, config application, and zero-value preservation. |

## Key Exports

| Type | Description |
|------|-------------|
| `AgentTemplate` | Top-level struct deserialized from a `.toml` file. Contains `template`, `personality`, `skills`, `tools`, `budget`, `defaults` sections. Methods: `load()`, `validate()`, `apply_to_config()`, `name()`, `description()`. |
| `TemplateMetadata` | Name, description, version (default `"1.0"`), author (default `"bsv-worm"`). |
| `PersonalityConfig` | `system_prompt` (prepended to agent context) and `style` hint (e.g., `"analytical"`, `"concise"`). |
| `SkillsConfig` | `active: Vec<String>` — skill names to activate (must match `skills/` directory entries). |
| `ToolsConfig` | `enabled` and `disabled` tool name lists. Disabled takes precedence. Validation rejects tools in both lists. |
| `TemplateBudgetConfig` | `daily_limit_sats`, `per_task_limit_sats`, `warning_threshold` (0.0–1.0). Zero values mean "use default". |
| `DefaultsConfig` | `model`, `temperature` (0.0–2.0), `max_tokens`. Zero/empty values mean "use default". |
| `TemplateError` | Enum: `Io`, `Parse` (TOML), `Validation`. |
| `ValidationError` | Struct with `field` and `message` for granular validation feedback. |
| `load_builtin_templates(dir)` | Scans a directory for `.toml` files, parses each, skips failures with warnings, returns sorted by name. |
| `find_template(templates, name)` | Case-insensitive lookup by template name. |

## TOML Template Format

```toml
[template]
name = "researcher"
description = "Deep research agent with web search and persistent memory"
version = "1.0"       # optional, defaults to "1.0"
author = "bsv-worm"   # optional, defaults to "bsv-worm"

[personality]
system_prompt = "You are a thorough research agent..."
style = "analytical"

[skills]
active = ["browser", "x402"]

[tools]
enabled = ["browser", "web_fetch", "memory_store"]
disabled = []

[budget]
daily_limit_sats = 50000
per_task_limit_sats = 15000
warning_threshold = 0.8

[defaults]
model = "gpt-5"
temperature = 0.3
max_tokens = 8192
```

All sections except `[template]` are optional and default to empty/zero values.

## Built-in Templates

10 templates in `templates/` at the workspace root:

| Template | Role |
|----------|------|
| `researcher` | Deep research with browser, x402, and memory |
| `coder` | Software development |
| `analyst` | Data analysis |
| `trader` | Financial trading |
| `content-creator` | Content generation |
| `customer-support` | Customer service |
| `data-pipeline` | Data processing workflows |
| `security-auditor` | Security analysis |
| `devops` | Infrastructure and operations |
| `personal-assistant` | General-purpose assistant |

## Validation Rules

`validate()` checks:
- `template.name` must be non-empty
- `template.description` must be non-empty
- `budget.warning_threshold` must be in `[0.0, 1.0]`
- `defaults.temperature` must be in `[0.0, 2.0]`
- No tool name may appear in both `tools.enabled` and `tools.disabled`

Returns `Err(Vec<ValidationError>)` with all issues found (not just the first).

## Config Application

`apply_to_config(&self, config: &mut WormConfig)` overrides only non-zero/non-empty fields:

| Template field | Config field overridden |
|---------------|----------------------|
| `budget.daily_limit_sats` (> 0) | `config.budget.max_per_day` |
| `budget.per_task_limit_sats` (> 0) | `config.budget.max_per_task` |
| `defaults.model` (non-empty) | `config.llm.default_model` |
| `defaults.max_tokens` (> 0) | `config.llm.max_tokens` |

`personality`, `skills`, `tools`, `temperature`, and `warning_threshold` are available on the struct but not yet wired into `apply_to_config()` — they are read by consumers directly from the template.

## Usage

```rust
use bsv_worm::templates::{AgentTemplate, load_builtin_templates, find_template};

// Load all templates from the templates/ directory
let templates = load_builtin_templates(Path::new("templates"))?;

// Find a specific template by name (case-insensitive)
if let Some(tmpl) = find_template(&templates, "researcher") {
    tmpl.validate().map_err(|errs| /* handle */)?;
    tmpl.apply_to_config(&mut config);
}

// Load a single template directly
let tmpl = AgentTemplate::load(Path::new("templates/coder.toml"))?;
```

## Related

- [../config/CLAUDE.md](../config/CLAUDE.md) — `WormConfig` that templates override
- [../CLAUDE.md](../CLAUDE.md) — Parent module docs with full file inventory
- [../../CLAUDE.md](../../CLAUDE.md) — Root project docs
