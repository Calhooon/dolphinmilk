# bsv-worm-sdk/src
> Public API crate for building Dolphin Milk plugins (tools, skills, providers).

## Overview

This is a standalone Rust crate (`bsv-worm-sdk`) that defines the trait interfaces and shared types third-party or internal code uses to extend Dolphin Milk. It has minimal dependencies (serde, async-trait, thiserror) and no dependency on the main crate, keeping the plugin boundary clean. Plugins implement one or more of the three traits — `Tool`, `Skill`, `Provider` — and the host agent loads them at runtime.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `lib.rs` | 56 | Crate root: module declarations, `prelude` module, top-level re-exports |
| `tool.rs` | 201 | `Tool` trait, `ToolInput`/`ToolResult`/`ToolError` types |
| `provider.rs` | 228 | `Provider` trait, `ThinkRequest`/`ThinkResult`/`ProviderError` types |
| `skill.rs` | 153 | `Skill` trait, `SkillConfig`/`SkillError` types |
| `types.rs` | 158 | Shared types: `Message`, `ToolDescription`, `PluginMetadata` |

## Key Exports

Everything is available via `use bsv_worm_sdk::prelude::*`, or individual modules can be imported directly. The three core traits are also re-exported at the crate root.

### Traits

| Trait | Module | Methods | Purpose |
|-------|--------|---------|---------|
| `Tool` | `tool` | `name`, `description`, `parameters_schema`, `category`, `execute` | Callable action the agent invokes via LLM function calling |
| `Provider` | `provider` | `name`, `models`, `supports_model`, `think` | LLM backend wrapper (OpenAI, Claude, local, etc.) |
| `Skill` | `skill` | `name`, `description`, `instructions`, `auto_activate`, `required_tools`, `activate`, `deactivate` | Behavioral module that injects instructions into the system prompt |

### Tool Types (`tool.rs`)

- **`ToolInput`** — Wraps `serde_json::Value` params from the LLM's tool call. Has typed accessors: `get_str()`, `get_i64()`, `get_f64()`, `get_bool()`.
- **`ToolResult`** — Contains `output: String` and `success: bool`. Constructors: `ToolResult::text(...)` for success, `ToolResult::error(...)` for failure.
- **`ToolError`** — 4 variants: `InvalidInput`, `ExecutionFailed`, `MissingParameter`, `External`.

### Provider Types (`provider.rs`)

- **`ThinkRequest`** — Builder-pattern struct: `messages`, `model`, `max_tokens` (default 4096), `temperature`, `tools`. Constructor: `ThinkRequest::new(messages, model)`, then chain `.with_max_tokens()`, `.with_temperature()`, `.with_tools()`.
- **`ThinkResult`** — LLM response: `text`, `model`, `prompt_tokens`, `completion_tokens`, `total_tokens`, `finish_reason`, `duration_ms`, `tool_calls`.
- **`ProviderError`** — 5 variants: `AuthFailed`, `RequestFailed`, `InvalidResponse`, `UnsupportedModel`, `RateLimited`.

### Skill Types (`skill.rs`)

- **`SkillConfig`** — Wrapper around optional `serde_json::Value` for arbitrary per-skill configuration.
- **`SkillError`** — 4 variants: `ActivationFailed`, `DeactivationFailed`, `InvalidConfig`, `StateError`.

### Shared Types (`types.rs`)

- **`Message`** — OpenAI chat completions format: `role`, `content`, `tool_calls`, `tool_call_id`. Constructors: `Message::system()`, `Message::user()`, `Message::assistant()`, `Message::tool_result()`.
- **`ToolDescription`** — Lightweight struct (`name`, `description`, `category`) for registration and prompt generation, distinct from the full `Tool` trait.
- **`PluginMetadata`** — Plugin identity: `name`, `version` (semver), `description`, `author`.

## Usage

### Implementing a Tool

```rust
use bsv_worm_sdk::prelude::*;
use async_trait::async_trait;
use serde_json::json;

struct WeatherTool;

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str { "weather_lookup" }
    fn description(&self) -> &str { "Look up current weather for a city" }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        })
    }
    fn category(&self) -> &str { "custom" }
    async fn execute(&self, input: ToolInput) -> Result<ToolResult, ToolError> {
        let city = input.get_str("city")
            .ok_or_else(|| ToolError::MissingParameter("city".into()))?;
        Ok(ToolResult::text(format!("Weather in {city}: sunny, 72F")))
    }
}
```

### Implementing a Provider

```rust
use bsv_worm_sdk::provider::{Provider, ProviderError, ThinkRequest, ThinkResult};
use async_trait::async_trait;

struct MyProvider;

#[async_trait]
impl Provider for MyProvider {
    fn name(&self) -> &str { "my-llm" }
    fn models(&self) -> Vec<String> { vec!["my-model-v1".into()] }
    async fn think(&self, req: ThinkRequest) -> Result<ThinkResult, ProviderError> {
        // Call your LLM backend, parse the response...
        Ok(ThinkResult {
            text: "response".into(),
            model: req.model,
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            finish_reason: "stop".into(),
            duration_ms: 100,
            tool_calls: Vec::new(),
        })
    }
}
```

### Implementing a Skill

```rust
use bsv_worm_sdk::skill::{Skill, SkillError};

struct PomodoroSkill;

impl Skill for PomodoroSkill {
    fn name(&self) -> &str { "pomodoro" }
    fn description(&self) -> &str { "Time-boxed focus sessions" }
    fn instructions(&self) -> &str {
        "Work in 25-minute focused intervals. Take 5-minute breaks."
    }
    fn auto_activate(&self) -> bool { false }
    fn required_tools(&self) -> Vec<String> { vec!["timer".into()] }
    fn activate(&self) -> Result<(), SkillError> { Ok(()) }
    fn deactivate(&self) -> Result<(), SkillError> { Ok(()) }
}
```

## Design Decisions

- **No host dependency**: This crate depends only on serde, async-trait, and thiserror. It never imports Dolphin Milk internals, ensuring plugins can compile independently.
- **`async_trait` for Tool and Provider**: Both traits use `#[async_trait]` because tool execution and LLM calls are inherently async. `Skill` is sync because activation/deactivation are lightweight state changes.
- **OpenAI message format**: `Message` and tool calling use OpenAI's chat completions format as the common wire format. Providers translate to/from their native format internally.
- **Builder pattern for ThinkRequest**: `ThinkRequest::new()` sets sensible defaults (max_tokens=4096, no temperature, no tools), with `with_*` methods for customization.
- **Category-based access control**: `Tool::category()` defaults to `"custom"`. The host agent uses categories for capability gating (e.g. sandbox tools vs wallet tools).

## Dependencies

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
thiserror = "2"
```

## Related

- [Root CLAUDE.md](../../CLAUDE.md) — Project-wide conventions, architecture overview, tool registration process
- `src/tools/registry.rs` in the main crate — Where `Tool` implementations get registered into the `ToolRegistry`
- `src/think.rs` in the main crate — Where `Provider` implementations are called during the agent loop
- `src/skills/` in the main crate — Where `Skill` implementations (SKILL.md files) are loaded
