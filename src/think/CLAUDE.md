# src/think/
> Dual-provider LLM inference via BRC-31 auth + x402 micropayments, normalized to a common `ThinkResult` format.

## Overview

This module is the worm's LLM inference boundary. It sends messages to OpenAI or Claude providers via authenticated, paid HTTP requests and normalizes all responses so the rest of the codebase (runner, transcript, conversation) is provider-agnostic. Every LLM call flows through here: BRC-31 authentication, x402 payment, provider-specific request construction, response parsing, refund internalization, and tool call extraction.

The internal format convention is OpenAI-style everywhere. Claude messages and tool definitions are converted at the boundary — callers never see Claude's native content blocks or `tool_use`/`tool_result` types.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 470 | Entry points (`think()`, `think_with_tools()`, `think_with_circuit_breaker()`), `ThinkResult` struct, endpoint routing (`resolve_endpoint()`, `alternate_endpoint()`), model detection (`is_reasoning_model()`, `is_claude_model()`), shared error/refund helpers, re-exports |
| `openai.rs` | 202 | `build_openai_body()` request construction, `parse_openai_response()` response parsing, `extract_payment_info()` shared payment/refund extraction |
| `claude.rs` | 396 | `build_claude_body()` request construction, `parse_claude_response()` response parsing, `convert_messages_for_claude()` and `convert_tools_for_claude()` format conversion, `flush_tool_results()` grouping |
| `capabilities.rs` | 87 | `ModelCapabilities` struct, `model_input_limit()` and `model_output_limit()` hardcoded fallbacks for 5 named model families + default (values verified against x402-info manifests 2026-03-24) |

## Key Exports

All public items are re-exported from `mod.rs` for backward compatibility (`crate::think::X`).

### Types

| Export | Description |
|--------|-------------|
| `ThinkResult` | Uniform response: text, model, sats paid/effective/refunded, token counts, finish_reason, duration_ms, tool_calls (OpenAI format), payment_txid, refund status, text-extraction flag |
| `ThinkRequest` | Parameter bundle for `think_with_circuit_breaker()`: auth, messages, model, max_tokens, temperature, tools, config, thinking_budget, reasoning_effort |
| `ModelCapabilities` | Discovered or hardcoded model limits: context_window, max_output/input_tokens, supports_tools, supports_vision |

### Functions

| Function | Description |
|----------|-------------|
| `think()` | Simple inference: messages + model + max_tokens. Delegates to `think_with_tools()` with no tools |
| `think_with_tools()` | Inference with function calling. Routes to correct provider, applies text extraction fallback for reasoning models |
| `think_with_circuit_breaker()` | Wraps `think_to_endpoint()` with circuit breaker failover (OpenAI <-> Claude). Records success/failure per provider |
| `resolve_endpoint()` | Routes model to provider URL. `claude-*` always goes to Claude regardless of config. Falls back to `config.llm.default_provider` |
| `alternate_endpoint()` | Returns the opposite provider for failover (OpenAI <-> Claude) |
| `is_reasoning_model()` | Detects o1/o3/o4-*, gpt-5.*, gpt-4.1.* — these need `max_completion_tokens` instead of `max_tokens` |
| `is_claude_model()` | Checks `claude-` prefix |
| `build_openai_body()` | Constructs OpenAI chat completions request. Handles reasoning model params (`max_completion_tokens`, no `temperature`, `reasoning_effort`) |
| `build_claude_body()` | Constructs Claude Messages API request. Converts messages/tools, handles `thinking_budget` with auto-adjusted `max_tokens` |
| `convert_messages_for_claude()` | Converts OpenAI-format messages to Claude format: extracts system messages, converts `tool` role to `tool_result` content blocks, converts assistant `tool_calls` to `tool_use` blocks |
| `convert_tools_for_claude()` | Converts OpenAI tool definitions (`{ type: "function", function: { name, description, parameters } }`) to Claude format (`{ name, description, input_schema }`) |
| `flush_tool_results()` | Groups consecutive tool-role messages into a single user message with multiple `tool_result` content blocks (Claude requirement) |
| `model_input_limit()` | Hardcoded max input tokens by model family (6 families, 128K default) |
| `model_output_limit()` | Hardcoded max output tokens by model family (16K default) |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `OPENAI_AGENT_URL` | `https://openai-chat.x402agency.com/chat` | OpenAI x402 proxy endpoint |
| `CLAUDE_AGENT_URL` | `https://claude-chat.x402agency.com/chat` | Claude x402 proxy endpoint |
| `DEFAULT_MODEL` | `gpt-5-mini` | Fallback model name |
| `DEFAULT_MAX_TOKENS` | `4096` | Fallback max tokens |

## Request Flow

```
think() / think_with_tools() / think_with_circuit_breaker()
    │
    ├─ resolve_endpoint()          → pick OpenAI or Claude URL
    │
    ├─ build_openai_body()         → if OpenAI provider
    │  OR build_claude_body()      → if Claude provider (converts messages + tools)
    │
    ├─ payment::authenticated_paid_request()   → BRC-31 auth + x402 payment + retry
    │
    ├─ parse_openai_response()     → if OpenAI provider
    │  OR parse_claude_response()  → if Claude (normalizes tool_use → tool_calls)
    │
    ├─ extract_payment_info()      → sats paid/effective/refunded, refund internalization
    │
    └─ text extraction fallback    → if tools provided but no tool_calls in response,
                                     tries extract_tool_calls_from_text() from runner/text_extract.rs
```

## Provider Differences

| Aspect | OpenAI | Claude |
|--------|--------|--------|
| System messages | Inline `role: "system"` | Extracted to top-level `system` param |
| Tool calls (response) | `message.tool_calls[].function.{name, arguments}` | `content[].{type: "tool_use", name, input}` — normalized to OpenAI format |
| Tool results (request) | `role: "tool"` messages with `tool_call_id` | `role: "user"` with `tool_result` content blocks (grouped by `flush_tool_results()`) |
| Tool definitions | `{ type: "function", function: { name, description, parameters } }` | `{ name, description, input_schema }` |
| Token limit param | `max_tokens` (standard) or `max_completion_tokens` (reasoning) | `max_tokens` always |
| Temperature | Supported (except reasoning models) | Always supported |
| Reasoning effort | `reasoning_effort` field for o-series/gpt-5 | N/A |
| Extended thinking | N/A | `thinking: { type: "enabled", budget_tokens: N }` — auto-adjusts `max_tokens >= budget + 1024` |
| Token usage fields | `prompt_tokens`, `completion_tokens`, `total_tokens` | `input_tokens`, `output_tokens` (total computed) |
| Finish reason | `stop`, `tool_calls`, `length` | `end_turn`→`stop`, `tool_use`→`tool_calls`, `max_tokens`→`length`, `stop_sequence`→`stop` |
| Empty content | Allowed | Rejected — empty strings replaced with `[empty]` (user) or `[continued]` (assistant) |

## Model Capabilities

`ModelCapabilities` resolves through a three-step chain:

1. **x402-info manifest** — discovered at startup from the provider's info endpoint, cached in `AppState.model_capabilities`
2. **Hardcoded fallback** — `from_hardcoded()` uses `model_input_limit()` / `model_output_limit()` tables
3. **Config cap** — user config (`config.llm.context_window`, `config.llm.max_tokens`) always acts as a ceiling

Hardcoded model families:

| Family | Context Window | Max Output | Max Input |
|--------|---------------|------------|-----------|
| `gpt-5*` | 400K | 128K | 272K |
| `o3*`, `o4*` | 200K | 100K | 100K |
| `claude-haiku*` | 200K | 64K | 136K |
| `claude-sonnet*` | 1M | 64K | 936K |
| `claude-opus*` | 1M | 128K | 872K |
| Unknown | 144K | 16K | 128K |

## Payment and Refund Handling

- Both providers use `payment::authenticated_paid_request()` from `x402/payment.rs` — handles BRC-31 auth, 402 detection, BRC-29 payment creation, and retry (up to 3 attempts)
- `extract_payment_info()` (in `openai.rs`, shared by both providers) reads `payment.satoshis_paid`, `payment.satoshis_effective`, and `excessRefund.satoshis` from the response
- Refund auto-internalization: `refund::parse_refund()` + `refund::process_refund()` via wallet. Best-effort — failure is logged, not fatal
- Error responses also attempt refund internalization via `try_internalize_refund_from_error()` before returning the error
- `parse_error_response()` handles both OpenAI-style (`code`/`description`) and Claude-style (`error.type`/`error.message`) error formats, plus upstream error detail extraction

## Circuit Breaker Integration

`think_with_circuit_breaker()` wraps inference with automatic failover:

1. Check if primary provider's circuit allows requests
2. If open, try alternate provider (OpenAI <-> Claude)
3. If both open, attempt primary anyway (better than returning error)
4. Record success/failure on the provider that was actually used

The `CircuitBreakerRegistry` is from `x402/circuit_breaker.rs`. The runner calls `think_with_circuit_breaker()` during the THINK phase of each iteration.

## Text Extraction Fallback

When tool definitions are provided but the LLM returns text without structured tool calls (common with reasoning models), `think_with_tools()` applies a post-processing step:

1. Extract known tool names from the provided tool definitions
2. Pass the response text to `extract_tool_calls_from_text()` (from `runner/text_extract.rs`)
3. If tool calls are found, populate `result.tool_calls` and set `result.was_text_extracted = true`

This is transparent to callers — they see tool calls regardless of whether they came from structured output or text extraction.

## Usage

```rust
// Simple inference (no tools)
let result = think(&auth, &messages, "gpt-5-mini", 4096, None, &config).await?;

// With tools
let result = think_with_tools(
    &auth, &messages, "gpt-5-mini", 4096, Some(0.7),
    Some(&tool_defs), &config, None, None,
).await?;

// With circuit breaker
let req = ThinkRequest {
    auth: &auth, messages: &msgs, model: "claude-sonnet-4-6",
    max_tokens: 8192, temperature: Some(0.7), tools: Some(&tool_defs),
    config: &config, thinking_budget: Some(4096), reasoning_effort: None,
};
let result = think_with_circuit_breaker(&req, &circuit_breakers).await?;

// Access uniform result
println!("Response: {}", result.text);
println!("Cost: {} sats", result.sats_effective);
for tc in &result.tool_calls {
    // Always OpenAI format: { id, type: "function", function: { name, arguments } }
}
```

## Related

- [../runner/CLAUDE.md](../runner/CLAUDE.md) — Agent loop calls `think_with_circuit_breaker()` during the THINK phase; `text_extract.rs` provides the text extraction fallback
- [../x402/CLAUDE.md](../x402/CLAUDE.md) — `payment::authenticated_paid_request()` handles BRC-31 auth + x402 payment; `refund` module handles refund internalization; `circuit_breaker` provides failover
- [../auth/CLAUDE.md](../auth/CLAUDE.md) — `AuthriteClient` provides BRC-31 session management for all LLM requests
- [../config/CLAUDE.md](../config/CLAUDE.md) — `WormConfig.llm` section controls default_model, default_provider, max_tokens, context_window, thinking_budget, reasoning_effort
- [../context/CLAUDE.md](../context/CLAUDE.md) — Context manager constructs the message array passed to `think()`
- [../../CLAUDE.md](../../CLAUDE.md) — Root project docs with full architecture overview
