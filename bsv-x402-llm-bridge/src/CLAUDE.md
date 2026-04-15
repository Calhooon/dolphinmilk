# bsv-x402-llm-bridge/src — LLM Inference Bridge

> Sell LLM inference to other agents via BSV x402 micropayments.

## Overview

This crate wraps upstream LLM APIs (OpenAI, Anthropic Claude) behind x402 payment verification. An agent running this bridge becomes a paid LLM inference provider: clients pay per-request in satoshis via `x-bsv-payment` headers, and the bridge forwards their requests to the underlying LLM API. All requests and responses use an OpenAI-compatible format, with the Claude provider handling format translation internally.

```text
Client Agent                 LLM Bridge                    Upstream LLM
+-----------+     x402      +----------------+    API     +----------+
| bsv-worm  | ──────────>   | payment verify | ────────> | OpenAI   |
| pays sats |   BRC-29      | route to       |           | Claude   |
+-----------+               | provider       | <──────── | etc.     |
      ^                     +----------------+    resp    +----------+
      |                            |
      └────────────────────────────┘
            response + usage
```

## Files

| File | Purpose |
|------|---------|
| `lib.rs` | Entry point: `build_router()`, HTTP handlers (`/chat`, `/health`, `/.well-known/x402-info`, `/models`), x402 payment verification |
| `config.rs` | `BridgeConfig` (env-var loading), `PricingModel` (3 variants), `ProviderConfig` |
| `providers/mod.rs` | `LlmProvider` trait, shared types (`ChatMessage`, `LlmRequest`, `LlmResponse`, `ProviderError`), provider factory |
| `providers/openai.rs` | OpenAI chat completions adapter with reasoning-model detection |
| `providers/claude.rs` | Anthropic messages API adapter with full format translation |

## Key Exports

### `build_router(config: BridgeConfig) -> Router`

The main entry point. Creates an axum `Router` with all endpoints wired up and providers instantiated from config. Usage:

```rust
use bsv_x402_llm_bridge::config::BridgeConfig;
use bsv_x402_llm_bridge::build_router;

let config = BridgeConfig::from_env();
let app = build_router(config);
let listener = tokio::net::TcpListener::bind("0.0.0.0:3403").await.unwrap();
axum::serve(listener, app).await.unwrap();
```

### `BridgeConfig`

Server configuration loaded from environment variables via `BridgeConfig::from_env()`. Fields:
- `port` — Listen port (`X402_BRIDGE_PORT`, default `3403`)
- `identity_key` — Server identity compressed pubkey (`X402_BRIDGE_IDENTITY_KEY`, defaults to generator point G)
- `pricing` — `PricingModel` enum (`X402_BRIDGE_PRICING`)
- `openai` — Optional `ProviderConfig` (enabled when `OPENAI_API_KEY` is set)
- `claude` — Optional `ProviderConfig` (enabled when `ANTHROPIC_API_KEY` is set)

Helper methods:
- `has_providers()` — Returns true if at least one provider is configured
- `available_models()` — Aggregates `available_models` from all configured providers

### `ProviderConfig`

Configuration for a single upstream LLM provider:
- `api_key` — API key (from env var, never hardcoded)
- `base_url` — Provider API base URL
- `default_model` — Fallback model when client doesn't specify one
- `max_tokens_limit` — Maximum tokens allowed per request
- `available_models` — Models advertised via `/models` and `/health` endpoints

Default available models (hardcoded in `from_env()`):
- **OpenAI:** `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1`, `o3-mini`
- **Anthropic:** `claude-sonnet-4-20250514`, `claude-opus-4-20250514`, `claude-3-5-haiku-20241022`

### `PricingModel`

Three pricing strategies:

| Variant | Env Config | Behavior |
|---------|-----------|----------|
| `PerRequest { satoshis }` | `X402_BRIDGE_PRICING=per-request`, `X402_BRIDGE_PRICE_SATS` | Fixed price per request (default: 500 sats) |
| `PerToken { prompt_sats_per_1k, completion_sats_per_1k }` | `X402_BRIDGE_PRICING=per-token`, `X402_BRIDGE_PROMPT_SATS_PER_1K`, `X402_BRIDGE_COMPLETION_SATS_PER_1K` | Price based on actual token usage (min 1 sat). Upfront estimate uses max_tokens. |
| `PerModel { model_prices, default_satoshis }` | `X402_BRIDGE_PRICING=per-model`, `X402_BRIDGE_MODEL_PRICE_<MODEL>=<sats>` | Per-model fixed rates with fallback default |

Key methods:
- `upfront_price(model, max_tokens)` — Price quoted before inference (used in 402 response)
- `actual_price(model, prompt_tokens, completion_tokens)` — Cost calculated after response (logged, included in `x402` response field)

### `LlmProvider` Trait

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn supports_model(&self, model: &str) -> bool;
    fn default_model(&self) -> &str;
    async fn chat(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError>;
}
```

Two implementations:
- **`OpenAiProvider`** — Handles all non-`claude-` models. Detects reasoning models (o1, o3, o4, gpt-5, gpt-4.1) and uses `max_completion_tokens` instead of `max_tokens` for them.
- **`ClaudeProvider`** — Handles `claude-*` models. Converts between OpenAI-compatible and Anthropic-native formats (messages, tools, tool results, stop reasons).

### `ProviderError`

Error enum with five variants: `Request`, `InvalidResponse`, `Auth`, `RateLimited`, `ModelNotAvailable`. Mapped to HTTP status codes in the chat handler (502, 429, 400).

## HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/chat` | x402 payment | Chat completion — verifies payment, routes to provider, returns OpenAI-compatible response with `x402` metadata |
| GET | `/health` | None | Health check — returns status, provider names, available models |
| GET | `/.well-known/x402-info` | None | Service manifest — identity key, pricing details, endpoint descriptions (used by `discover_endpoints` in bsv-worm) |
| GET | `/models` | None | Lists available models with provider names and per-model pricing |

### Payment Flow (`/chat`)

1. Parse request body (model, messages, max_tokens, temperature, tools)
2. Resolve provider via `resolve_provider(model, providers)` — first provider where `supports_model()` returns true
3. Calculate upfront price via `PricingModel::upfront_price()`
4. Verify `x-bsv-payment` header — must contain JSON with `derivationPrefix`, `derivationSuffix`, `transaction` (BRC-29 fields)
5. If header missing → return 402 with `x-bsv-payment-satoshis-required`, `x-bsv-payment-derivation-prefix`, `x-bsv-auth-identity-key` headers
6. Forward request to upstream provider
7. Return OpenAI-compatible response with `x402` extension field containing `satoshis_paid`, `satoshis_actual`, `provider`

### Response Format

```json
{
  "id": "bridge-<random-hex>",
  "object": "chat.completion",
  "model": "gpt-4o",
  "choices": [{ "index": 0, "message": { "role": "assistant", "content": "..." }, "finish_reason": "stop" }],
  "usage": { "prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150 },
  "x402": { "satoshis_paid": 500, "satoshis_actual": 25, "provider": "OpenAI" }
}
```

## Environment Variables

### Bridge Config

| Variable | Default | Purpose |
|----------|---------|---------|
| `X402_BRIDGE_PORT` | `3403` | Server listen port |
| `X402_BRIDGE_IDENTITY_KEY` | Generator point G | Server identity (compressed pubkey hex) |
| `X402_BRIDGE_PRICING` | `per-request` | Pricing model: `per-request`, `per-token`, or `per-model` |
| `X402_BRIDGE_PRICE_SATS` | `500` | Default satoshis per request |
| `X402_BRIDGE_PROMPT_SATS_PER_1K` | `10` | Per-token: prompt cost per 1000 tokens |
| `X402_BRIDGE_COMPLETION_SATS_PER_1K` | `30` | Per-token: completion cost per 1000 tokens |
| `X402_BRIDGE_MODEL_PRICE_<MODEL>` | — | Per-model: price for specific model (underscores → hyphens in model name) |

### OpenAI Provider

| Variable | Default | Purpose |
|----------|---------|---------|
| `OPENAI_API_KEY` | — | Enables OpenAI provider when set |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | API base URL |
| `OPENAI_DEFAULT_MODEL` | `gpt-4o-mini` | Default model |
| `OPENAI_MAX_TOKENS` | `4096` | Max tokens limit |

### Anthropic Provider

| Variable | Default | Purpose |
|----------|---------|---------|
| `ANTHROPIC_API_KEY` | — | Enables Anthropic provider when set |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com/v1` | API base URL |
| `ANTHROPIC_DEFAULT_MODEL` | `claude-sonnet-4-20250514` | Default model |
| `ANTHROPIC_MAX_TOKENS` | `4096` | Max tokens limit |

## Claude Provider Format Translation

The Claude provider performs three conversions since the bridge's internal format is OpenAI-compatible but Anthropic uses a different API shape:

**Messages** (`convert_messages`): Extracts `system` role into a separate API parameter. Maps `tool` role messages to Anthropic `tool_result` content blocks under `user` role. Passes `user`/`assistant` through directly.

**Tools** (`convert_tools`): Unwraps OpenAI's `{ type: "function", function: { name, description, parameters } }` wrapper into Anthropic's flat `{ name, description, input_schema }` format.

**Tool uses** (`convert_tool_uses`): Converts Anthropic `tool_use` content blocks back to OpenAI `tool_calls` format with `{ id, type: "function", function: { name, arguments } }`.

**Stop reasons**: `end_turn` → `stop`, `max_tokens` → `length`, `tool_use` → `tool_calls`.

## Internal Types

### `AppState`

Shared server state held behind `Arc`:
- `config: BridgeConfig` — Server configuration
- `providers: Vec<Box<dyn LlmProvider>>` — Instantiated providers

### `PaymentInfo`

Result of successful payment verification:
- `derivation_prefix: String` — BRC-29 derivation prefix
- `derivation_suffix: String` — BRC-29 derivation suffix
- `satoshis_paid: u64` — Amount paid

## Dependencies

| Crate | Purpose |
|-------|---------|
| `bsv-x402-server` | Sibling crate (path dep) |
| `axum` 0.8 | HTTP framework |
| `reqwest` 0.12 | HTTP client for upstream providers |
| `async-trait` | Trait with async methods (`LlmProvider`) |
| `thiserror` 2 | `ProviderError` derive |
| `hex`, `rand` | Payment derivation prefix generation |
| `tracing` | Structured logging |

## Testing

Unit tests in `#[cfg(test)]` modules across all files:
- `lib.rs` — 402 response construction, payment verification (missing, valid, invalid JSON, missing fields), router build with no providers
- `config.rs` — All three `PricingModel` variants (upfront + actual), per-token minimum (1 sat floor), default pricing
- `providers/openai.rs` — Reasoning model detection, `supports_model` routing
- `providers/claude.rs` — System message extraction, tool conversion, tool result mapping, tool_use→tool_calls conversion, `supports_model` routing

All tests are pure (no network calls). Run with `cargo test -p bsv-x402-llm-bridge`.

## Related

- [providers/CLAUDE.md](providers/CLAUDE.md) — Detailed provider trait and adapter documentation
- [../bsv-x402-server/src/CLAUDE.md](../bsv-x402-server/src/CLAUDE.md) — x402 server framework (sibling crate dependency)
- Root [CLAUDE.md](../../CLAUDE.md) — Project conventions, architecture overview
- `src/think.rs` in bsv-worm — The client side that pays this bridge for LLM inference
- `src/tools/x402_tools/` in bsv-worm — Generic `x402_call` tool that can target this bridge
