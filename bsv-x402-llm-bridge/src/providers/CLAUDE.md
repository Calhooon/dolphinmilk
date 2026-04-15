# Providers — Upstream LLM Provider Adapters

> Translates between the bridge's internal OpenAI-compatible request/response format and each upstream LLM provider's native API.

## Overview

This module defines the `LlmProvider` trait and two implementations — `OpenAiProvider` and `ClaudeProvider`. The bridge accepts requests in OpenAI-compatible format; each provider converts that to its native API format, makes the HTTP call with the real API key, and normalizes the response back.

## Files

| File | Purpose |
|------|---------|
| `mod.rs` | Shared types (`ChatMessage`, `LlmRequest`, `LlmResponse`, `ProviderError`), `LlmProvider` trait, provider factory |
| `openai.rs` | OpenAI chat completions adapter — passthrough format with reasoning-model handling |
| `claude.rs` | Anthropic messages API adapter — full format translation (messages, tools, tool results, stop reasons) |

## Core Types

### `ChatMessage`
Internal message representation (OpenAI-compatible). Fields:
- `role` — `"system"`, `"user"`, `"assistant"`, or `"tool"`
- `content` — Text content (optional, absent for tool-call-only messages)
- `name` — Optional name for multi-participant conversations
- `tool_calls` — Tool calls requested by the model (`Vec<Value>`, OpenAI format)
- `tool_call_id` — Links a `"tool"` role message back to a specific tool call

### `LlmRequest`
Request to any provider:
- `model` — Model identifier (e.g., `"gpt-4o"`, `"claude-sonnet-4-20250514"`)
- `messages` — Conversation history as `Vec<ChatMessage>`
- `max_tokens` — Maximum tokens for the completion
- `temperature` — Optional sampling temperature
- `tools` — Optional tool definitions (OpenAI function-calling format)

### `LlmResponse`
Normalized response from any provider:
- `text` — Generated text content
- `model` — Model actually used (may differ from requested)
- `prompt_tokens`, `completion_tokens`, `total_tokens` — Token usage
- `finish_reason` — `"stop"`, `"length"`, or `"tool_calls"` (OpenAI convention)
- `tool_calls` — Tool calls in OpenAI format (empty if none)

### `ProviderError`
Error enum with five variants:
- `Request` — Network or HTTP error
- `InvalidResponse` — Unparseable response body
- `Auth` — 401 (invalid/expired API key)
- `RateLimited` — 429 (rate limit exceeded)
- `ModelNotAvailable` — Requested model not supported

## Trait: `LlmProvider`

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn supports_model(&self, model: &str) -> bool;
    fn default_model(&self) -> &str;
    async fn chat(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError>;
}
```

Each provider is `Send + Sync` for use behind `Arc` in the server.

## Provider Details

### OpenAiProvider (`openai.rs`)

- **Model routing:** Handles all non-`claude-` models (`!model.starts_with("claude-")`)
- **Reasoning model detection:** `is_reasoning_model()` identifies o1, o3, o4, gpt-5, and gpt-4.1 series
- **Key difference:** Reasoning models use `max_completion_tokens` instead of `max_tokens`, and `temperature` is omitted
- **Auth:** `Authorization: Bearer {api_key}` header
- **Endpoint:** `{base_url}/chat/completions`
- **Format:** Messages pass through as-is (bridge internal format is OpenAI-compatible)

### ClaudeProvider (`claude.rs`)

- **Model routing:** Handles `claude-*` models (`model.starts_with("claude-")`)
- **Auth:** `x-api-key: {api_key}` + `anthropic-version: 2023-06-01` headers
- **Endpoint:** `{base_url}/messages`
- **Text extraction:** Joins all `text`-type content blocks from the response `content` array into a single string
- **Token field mapping:** Anthropic's `input_tokens`/`output_tokens` → `prompt_tokens`/`completion_tokens`; `total_tokens` computed as their sum (not read from response)
- **Format translation** (three conversions):

| Conversion | Method | What it does |
|-----------|--------|-------------|
| Messages | `convert_messages()` | Extracts `system` role into separate parameter; maps `tool` role to `tool_result` content blocks under `user` role; passes `user`/`assistant` through; unknown roles fall through as `user` messages |
| Tools | `convert_tools()` | OpenAI `function` wrapper → Anthropic flat `{name, description, input_schema}` |
| Tool uses | `convert_tool_uses()` | Anthropic `tool_use` content blocks → OpenAI `tool_calls` format with `{id, type: "function", function: {name, arguments}}` |

**Stop reason mapping:**

| Anthropic | → OpenAI |
|-----------|----------|
| `end_turn` | `stop` |
| `max_tokens` | `length` |
| `tool_use` | `tool_calls` |

## Factory Functions

### `resolve_provider(model, providers) -> Option<&dyn LlmProvider>`
Finds the first provider whose `supports_model()` returns true for the given model name.

### `create_providers(openai_config, claude_config) -> Vec<Box<dyn LlmProvider>>`
Constructs provider instances from `ProviderConfig` values. Only creates providers for which config is provided (`Some`). Called at server startup with configs from `BridgeConfig`.

## Configuration

Each provider is configured via `ProviderConfig` (defined in `../config.rs`):
- `api_key` — From env var (`OPENAI_API_KEY` / `ANTHROPIC_API_KEY`)
- `base_url` — API base URL (defaults: `https://api.openai.com/v1`, `https://api.anthropic.com/v1`)
- `default_model` — Fallback model
- `max_tokens_limit` — Per-request token cap
- `available_models` — Advertised model list

## Testing

Both providers have unit tests in `#[cfg(test)]` modules (8 tests total):
- `openai::tests` (2) — Reasoning model detection (`o1`/`o3`/`o4`/`gpt-5`/`gpt-4.1` vs `gpt-4o`), `supports_model` routing
- `claude::tests` (6) — `supports_model` routing, system message extraction (with and without system role), `convert_tools` OpenAI→Anthropic, `convert_tool_uses` Anthropic→OpenAI, `convert_messages` tool result mapping

All tests are pure (no network calls). Integration tests with mocked HTTP would live in `tests/`.

## Related

- `../config.rs` — `ProviderConfig`, `BridgeConfig`, `PricingModel` definitions
- `../lib.rs` — Bridge entry point that wires providers into the server
