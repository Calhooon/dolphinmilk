# bsv-x402-llm-bridge

Sell LLM inference to other agents via BSV x402 micropayments.

This crate wraps upstream LLM APIs (OpenAI, Anthropic Claude) behind x402 payment verification. An agent running this bridge becomes a paid LLM inference provider: other agents pay per-request in satoshis, and the bridge forwards their requests to the underlying LLM API.

## Architecture

```
Client Agent                 LLM Bridge                    Upstream LLM
+-----------+     x402      +----------------+    API     +----------+
| bsv-worm  | ---------->  | payment verify | --------> | OpenAI   |
| pays sats |   BRC-29     | route to       |           | Claude   |
+-----------+              | provider       | <-------- | etc.     |
      ^                    +----------------+    resp    +----------+
      |                           |
      +---------------------------+
            response + usage
```

## Endpoints

| Method | Path | Cost | Description |
|--------|------|------|-------------|
| POST | `/chat` | dynamic | Chat completion (OpenAI-compatible format) |
| GET | `/models` | free | List available models and pricing |
| GET | `/health` | free | Health check with provider status |
| GET | `/.well-known/x402-info` | free | Service manifest |

## Pricing Models

Three pricing models are supported, configured via environment variables:

### Per-Request (default)
Fixed price regardless of token count.
```bash
X402_BRIDGE_PRICING=per-request
X402_BRIDGE_PRICE_SATS=500
```

### Per-Token
Price based on actual prompt and completion token usage.
```bash
X402_BRIDGE_PRICING=per-token
X402_BRIDGE_PROMPT_SATS_PER_1K=10
X402_BRIDGE_COMPLETION_SATS_PER_1K=30
```

### Per-Model
Different fixed prices for different models.
```bash
X402_BRIDGE_PRICING=per-model
X402_BRIDGE_PRICE_SATS=500           # default for unlisted models
X402_BRIDGE_MODEL_PRICE_GPT_4O=1000
X402_BRIDGE_MODEL_PRICE_GPT_4O_MINI=200
X402_BRIDGE_MODEL_PRICE_CLAUDE_SONNET_4_20250514=800
```

## Configuration

All configuration is via environment variables. API keys are required for at least one provider.

| Variable | Default | Description |
|----------|---------|-------------|
| `X402_BRIDGE_PORT` | `3403` | Server port |
| `X402_BRIDGE_IDENTITY_KEY` | secp256k1 generator | Server identity public key |
| `X402_BRIDGE_PRICING` | `per-request` | Pricing model |
| `X402_BRIDGE_PRICE_SATS` | `500` | Default price in satoshis |
| `OPENAI_API_KEY` | - | OpenAI API key (enables OpenAI provider) |
| `OPENAI_BASE_URL` | `https://api.openai.com/v1` | OpenAI API base URL |
| `OPENAI_DEFAULT_MODEL` | `gpt-4o-mini` | Default OpenAI model |
| `ANTHROPIC_API_KEY` | - | Anthropic API key (enables Claude provider) |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com/v1` | Anthropic API base URL |
| `ANTHROPIC_DEFAULT_MODEL` | `claude-sonnet-4-20250514` | Default Claude model |

## Usage as a Library

```rust,no_run
use bsv_x402_llm_bridge::config::BridgeConfig;
use bsv_x402_llm_bridge::build_router;

#[tokio::main]
async fn main() {
    let config = BridgeConfig::from_env();
    let app = build_router(config);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3403").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

## Integration with bsv-worm

A bsv-worm agent can call this bridge using the `x402_call` tool or the `think` module. The agent's wallet handles BRC-29 payment construction automatically. The bridge responds in OpenAI-compatible format, so existing LLM parsing code works unchanged.

## License

Apache-2.0
