# bsv-x402-server/src
> Extracted x402 payment library — BRC-29 payment flow, service discovery, circuit breaker, rate limiting, and response caching.

## Overview

This is a standalone Rust crate (`bsv-x402-server`) extracted from the main `bsv-worm` codebase. It encapsulates the entire x402 payment protocol client: constructing BRC-29 payments, discovering x402 service manifests, caching responses to save satoshis, circuit-breaking degraded providers, and rate-limiting outbound requests. The crate is **protocol-agnostic** — it defines traits (`WalletApi`, `AuthClient`) that the consuming crate implements, decoupling payment logic from any specific wallet or auth implementation.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `lib.rs` | 19 | Crate root — re-exports all public modules |
| `error.rs` | 42 | `X402Error` enum: `Payment`, `Request`, `Discovery` variants via `thiserror` |
| `traits.rs` | 83 | `WalletApi` and `AuthClient` trait definitions — the crate's abstraction boundary |
| `payment.rs` | 689 | BRC-29 payment construction and 402 auto-retry flow (the core) |
| `discovery.rs` | 584 | `ServiceManifest` / `EndpointInfo` structs, `fetch_manifest()`, LLM-readable formatting, model capability parsing |
| `schema.rs` | 737 | Manifest input schema → JSON Schema conversion (3 variants + freetext parsing) |
| `cache.rs` | 213 | Generic LRU+TTL response cache with cost-tracking stats |
| `circuit_breaker.rs` | 276 | Closed→Open→HalfOpen circuit breaker per endpoint |
| `rate_limit.rs` | 341 | Token bucket rate limiter per service, with cert-driven overrides |
| `refund.rs` | 313 | Refund parsing, key derivation-based output matching, BEEF/AtomicBEEF envelope parsing via BSV SDK |
| `registry.rs` | 415 | x402agency.com agent registry client with disk-backed 5min TTL cache |

## Key Exports

### Traits (`traits.rs`)

- **`WalletApi`** — async trait for BRC-100 wallet operations needed by payment flow:
  - `get_public_key()` — BRC-42 key derivation
  - `create_action()` — build funded transactions
  - `internalize_action()` — accept incoming transactions (refunds)
- **`AuthClient`** — async trait for BRC-31 authenticated HTTP requests:
  - `authenticated_request()` — send request with Authrite headers
  - `wallet()` — access the underlying `WalletApi`
  - `session_dir()` — get session directory path (for cache management)
  - `clear_session()` / `base_url_from()` — session management
- **`CreateActionResult`** — wallet transaction output (`tx: Vec<u8>`, `txid: String`)
- **`AuthResponse`** — raw HTTP response (status, headers, body bytes)

### Payment (`payment.rs`)

- **`authenticated_paid_request()`** — the main entry point: BRC-31 auth + automatic 402 payment retry (up to 3 attempts), with 401 stale session recovery
- **`paid_request()`** — unauthenticated version: plain HTTP + 402 auto-retry
- **`create_payment()`** — BRC-29 payment construction: derive key → P2PKH script → createAction → AtomicBEEF wrapping → base64
- **`parse_402_response()`** — extract `PaymentRequired` from 402 response headers
- **`AuthPaidResponse`** — result struct with body, status, payment_txid, sats_paid
- **`payment_protocol()`** — returns BRC-29 payment key derivation protocol ID `[2, "3241645161d8"]`
- **`build_p2pkh_script()`** — compressed pubkey → OP_DUP OP_HASH160 ... OP_CHECKSIG
- **`hash160()`** — RIPEMD160(SHA256(data)) — Bitcoin's standard hash160
- **`raw_tx_to_beef()` / `raw_tx_to_atomic_beef()` / `beef_to_atomic_beef()`** — transaction format wrapping (BRC-62)
- **BRC-105 multipart transport** — payments >8KB switch from `x-bsv-payment` header to `multipart/form-data`

### Discovery (`discovery.rs`)

- **`ServiceManifest`** — parsed `/.well-known/x402-info` manifest (name, identity key, auth, endpoints, pricing)
- **`EndpointInfo`** — single endpoint: path, method, auth, payment, input/output schemas, polling, refund, hints
- **`fetch_manifest()` / `fetch_manifest_from_url()`** — HTTP GET manifest fetchers
- **`format_manifest_summary()`** — human/LLM-readable text: endpoints with flags, input schemas, costs, polling config, payment tiers, timing, output schema, refund policy
- **`DiscoveredModelCapability`** — parsed model capability from manifest pricing (model, context_window, max_output_tokens, supports_tools, supports_vision)
- **`parse_model_capabilities()`** — extract model capabilities from `manifest.pricing.models` array → `HashMap<String, DiscoveredModelCapability>`

### Schema (`schema.rs`)

- **`manifest_input_to_json_schema()`** — converts x402 manifest input fields to standard JSON Schema (OpenAI function calling compatible). Handles 3 format variants:
  - **Variant A**: `{ "contentType": "...", "schema": { ... } }` (wrapped)
  - **Variant B**: `{ "query": { "type": "string" } }` (flat properties)
  - **Variant C**: `{ "category": "A|B|C (default: A)" }` (freetext strings parsed into enums/ranges)
- **`format_input_schema()`** — render schema as indented text for LLM consumption
- Type normalization: `int`→`integer`, `float`→`number`, `bool`→`boolean`, `string[]`→`array`, unknown→`string`

### Cache (`cache.rs`)

- **`ResponseCache<T>`** — generic LRU+TTL cache, thread-safe via `Mutex`
  - `cache_key()` — SHA-256 of `model|messages|temperature|max_tokens`
  - `get()` / `put()` / `put_if()` — lookup, store, conditional store
  - TTL=0 disables the cache entirely
- **`CacheStats`** — hits, misses, total_sats_saved, evictions, hit_rate_pct()

### Circuit Breaker (`circuit_breaker.rs`)

- **`CircuitBreaker`** — per-endpoint state machine: Closed → Open (after N transient failures) → HalfOpen (probe after recovery window) → Closed
- **`CircuitBreakerRegistry`** — thread-safe `HashMap<base_url, CircuitBreaker>` with auto-creation
  - `should_allow()` / `record_success()` / `record_failure_msg()` — per-endpoint operations
  - `get_state()` — inspect current circuit state for an endpoint
  - `with_config()` — custom threshold and recovery window
- **`is_transient_message()`** — classifies errors: timeouts, 5xx, 429, connection failures, `ERR_SERVICE_FAILED` = transient; 4xx = non-transient
- Defaults: threshold=5 consecutive failures, recovery_window=30s

### Rate Limiter (`rate_limit.rs`)

- **`RateLimiterRegistry`** — per-service token bucket, keyed by base URL
  - `acquire()` — async, blocks (via `tokio::time::sleep`) until a token is available
  - `disabled()` — create a no-limit rate limiter (default)
  - `status()` → `Vec<ServiceRateLimitStatus>` — current token state for all tracked services
  - `is_enabled()` — check if rate limiting is active
  - RPM=0 means unlimited; disabled by default for backward compatibility
- **`CertRateLimits`** — BRC-52 certificate-driven overrides (highest priority over config)
- **`RateLimitConfig`** / **`ServiceRateLimit`** — global enable, default RPM, per-service overrides
- **`ServiceRateLimitStatus`** — serializable snapshot: service, RPM, tokens_available, capacity
- **`base_url()`** — free function: extract `scheme://host[:port]` from full URL (shared keying strategy with circuit breaker)

### Refund (`refund.rs`)

- **`RefundInfo`** — parsed refund data: transaction (base64), derivation prefix/suffix, sender identity key, satoshis, optional `output_index`
- **`parse_refund()`** — extract `RefundInfo` from `excessRefund` or `refund` key in response JSON. Skips if `already_refunded` flag is set
- **`process_refund()`** — internalize a refund transaction into the wallet. Uses a 3-tier output index resolution:
  1. Derive expected BRC-29 refund key → compute hash160 → find matching P2PKH output (most reliable)
  2. Fall back to server-specified `outputIndex` if key derivation fails
  3. Fall back to index 0
- **`parse_tx_from_envelope()`** — parse a `Transaction` from AtomicBEEF, BEEF, or raw tx bytes using the BSV SDK (`bsv_rs::transaction::Beef::from_binary`)
- **`find_p2pkh_output()`** — scan transaction outputs for a P2PKH script matching an expected hash160 (25-byte script: `76 a9 14 <hash160> 88 ac`)

### Registry (`registry.rs`)

- **`list_agents()`** — fetch `/.well-known/agents` from x402agency.com, cached to `~/.local/share/brc31-sessions/x402-registry.json` with 5min TTL
- **`list_agents_from()`** — fetch from a specific registry URL (custom registry support)
- **`list_agents_with_ttl()`** — fetch with custom cache TTL
- **`resolve()`** — agent name → base URL (case-insensitive, supports `name/path` syntax, full URLs pass through)
- **`resolve_from()`** — resolve using a specific registry URL
- **`resolve_x402_info()`** — agent name → manifest URL (respects `x402_info` field for third-party hosted manifests)
- **`AgentEntry`** — name, display_name, url, tagline, capabilities, extra fields
- **`RegistryResponse`** — wrapper struct with `agents: Vec<AgentEntry>`
- Constants: `DEFAULT_REGISTRY_CACHE_TTL` (300s), `DEFAULT_REGISTRY_URL` (`https://x402agency.com/.well-known/agents`)

### Error (`error.rs`)

- **`X402Error`** — 3 variants: `Payment`, `Request`, `Discovery`. Convenience constructors: `X402Error::payment()`, `::request()`, `::discovery()`. Accessor: `message()` → `&str`

## Usage

The consuming crate (bsv-worm) implements `WalletApi` and `AuthClient`, then calls into this library:

```rust
// Make an authenticated + paid request (most common path)
let result = authenticated_paid_request(
    &auth_client,       // impl AuthClient
    "POST",
    "https://openai-chat.x402agency.com/chat",
    &[("content-type".into(), "application/json".into())],
    Some(body_bytes),
).await?;
// result.body, result.status, result.payment_txid, result.sats_paid

// Discover what an x402 service offers
let manifest = fetch_manifest("https://openai-chat.x402agency.com").await?;
let summary = format_manifest_summary(&manifest);  // LLM-readable text

// Convert manifest input schemas for OpenAI function calling
let json_schema = manifest_input_to_json_schema(&endpoint.input);

// Resolve agent names from the registry
let url = resolve("banana/generate", None).await?;
// → "https://nano-banana-pro.x402agency.com/generate"

// Check circuit breaker before making a request
if !circuit_breaker_registry.should_allow(endpoint_url) {
    // Route to alternate provider
}

// Rate-limit outbound requests
rate_limiter.acquire(service_url).await?;
```

## Payment Flow

The core payment sequence in `payment.rs`:

1. Send authenticated request via `AuthClient`
2. If **401**: clear stale BRC-31 session, retry once
3. If **402**: parse `x-bsv-payment-*` headers → `PaymentRequired`
4. Generate random derivation suffix (32 bytes, base64)
5. Derive payment public key via BRC-42 (`WalletApi::get_public_key`)
6. Build P2PKH locking script from derived key
7. Create funded transaction via `WalletApi::create_action` (with SQLite retry)
8. Detect tx format (AtomicBEEF / BEEF / raw) → wrap to AtomicBEEF
9. Base64-encode, build payment JSON
10. If payment >8KB and server supports it → BRC-105 multipart transport
11. Retry original request with `x-bsv-payment` header (or multipart body)
12. Up to 3 payment attempts if server returns 402 again

## Design Decisions

- **Trait-based abstraction**: `WalletApi` and `AuthClient` make this crate testable and reusable without depending on the full bsv-worm wallet implementation.
- **AtomicBEEF normalization**: Payment always wraps to AtomicBEEF regardless of what the wallet returns (raw tx, BEEF, or AtomicBEEF pass-through).
- **SQLite retry in create_payment**: Wallet uses SQLite with WAL mode; rapid sequential payments can hit `database is locked`. Retries up to 3x with linear backoff (500ms, 1000ms, 1500ms).
- **Cache keyed by SHA-256**: Deterministic key from model + messages + temperature + max_tokens avoids duplicate paid requests for identical LLM calls.
- **Circuit breaker per base URL**: Not per-endpoint — all endpoints on a provider share circuit state since they share infrastructure.
- **Rate limiter disabled by default**: No breaking change for existing users. Enable via config or BRC-52 cert.

## Related

- [Root CLAUDE.md](../../CLAUDE.md) — project conventions, architecture overview
- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — full system architecture
- `src/x402/` in the main crate — the integration layer that uses this library
- `src/tools/x402_tools/` — agent tools (`x402_call`, `discover_services`, `discover_endpoints`) built on top of this crate
- `src/think/` — LLM inference calls that go through `authenticated_paid_request()`
- `skills/x402/SKILL.md` — decision tree skill for the agent's x402 usage
