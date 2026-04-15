# x402/

> 79 tests across 4 files covering x402 payment construction, service discovery, circuit breaker resilience, and response caching.

## Overview

Tests the x402 payment subsystem without requiring a live wallet or network. Payment tests validate BEEF encoding, P2PKH script construction, and HTTP 402 header parsing. Discovery tests use mockito to simulate the `/.well-known/agents` registry and `/.well-known/x402-info` service manifests. Circuit breaker tests verify the Closed→Open→HalfOpen state machine with error classification and per-endpoint isolation. Cache tests cover LRU eviction, TTL expiry, deterministic keying, and tool-call exclusion.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_circuit_breaker.rs | 24 | Circuit breaker state machine: threshold trips, recovery window, HalfOpen probe, error classification (transient vs client), per-endpoint isolation via registry, thread safety |
| test_response_cache.rs | 19 | LRU+TTL response cache: hit/miss, SHA-256 key determinism (model, messages, temperature, max_tokens), TTL expiry, LRU eviction, tool_call exclusion, stats tracking, concurrent access |
| test_x402_discovery.rs | 19 | Service discovery: agent registry list/resolve/cache, manifest fetch/parse, `format_manifest_summary` (polling, output schema, payment tiers, refund, timing sections) |
| test_x402.rs | 17 | Payment primitives: `payment_protocol()`, `hash160()`, `build_p2pkh_script()`, `raw_tx_to_beef()`/`raw_tx_to_atomic_beef()`, `parse_402_response()`, `parse_refund()` |

## What each file tests

### test_x402.rs -- Payment primitives (17 tests)

Tests pure functions from `bsv_worm::x402::payment` and `bsv_worm::x402::refund`:

- **`payment_protocol()`** -- Returns `[2, "3241645161d8"]` array for BRC-29 key derivation
- **`hash160()`** -- RIPEMD160(SHA256) produces 20-byte deterministic output
- **`build_p2pkh_script()`** -- Generates OP_DUP OP_HASH160 ... OP_EQUALVERIFY OP_CHECKSIG (50 hex chars). Rejects invalid length (not 66 hex) and invalid prefix (not 02/03)
- **`raw_tx_to_beef()`** -- Wraps raw tx in BEEF_V1 envelope: `[01 00 BE EF]` header + 0 BUMPs + 1 tx + hasBump=false
- **`raw_tx_to_atomic_beef()`** -- AtomicBEEF with `[01 01 01 01]` header + reversed txid + BEEF body
- **`parse_402_response()`** -- Extracts `satoshis` and `derivation_prefix` from `x-bsv-payment-*` headers. Validates version=1.0, satoshis>0, all required headers present
- **`parse_refund()`** -- Parses `excessRefund` from LLM response body. Returns `None` for missing, `already_refunded: true`, or incomplete fields

### test_circuit_breaker.rs -- Resilience (24 tests)

Tests `CircuitBreaker`, `CircuitBreakerRegistry`, and `is_transient_message` from `bsv_worm::x402::circuit_breaker`:

**State machine (tests 1-7):**
- Starts `Closed` with `failure_count=0`
- Single transient failure stays `Closed`
- `DEFAULT_FAILURE_THRESHOLD` (5) transient failures → `Open`
- `Open` rejects via `should_allow() == false`
- After recovery window elapses, `should_allow()` transitions to `HalfOpen`
- `HalfOpen` + success → `Closed` (failure_count reset)
- `HalfOpen` + failure → `Open` again

**Error classification (tests 8-10, 16-18, 22-24):**
- Client errors (400, 401, 403, 404) → non-transient, don't increment failure count
- Server errors (500, 502, 503, 504) → transient, trip the breaker
- Timeouts, connection refused/reset, DNS failures → transient
- Rate limits (429, "Rate limit") → transient
- `ERR_SERVICE_FAILED_REFUND_ISSUED` → transient
- Budget, config, loop, tool, memory errors → non-transient
- Mixed transient/non-transient: only transient failures count toward threshold
- Wallet connection errors → transient; auth signature errors → non-transient

**Registry (tests 11, 19, 21):**
- Per-endpoint isolation: tripping one URL doesn't affect another
- Base URL extraction: strips path and port, keeps scheme+host
- Unknown endpoints default to `Closed`

**Configuration and concurrency (tests 12-15, 20):**
- Success resets failure count in `Closed` state
- Configurable threshold and recovery window
- 10 threads × 50 ops with no panics or deadlocks
- Default constants: threshold=5, recovery=30s

### test_response_cache.rs -- LRU+TTL cache (19 tests)

Tests `ResponseCache<ThinkResult>` from `bsv_worm::x402::cache`:

**Basic operations (tests 16-17, 32, 34):**
- Cache miss returns `None`
- Cache hit returns stored `ThinkResult` with all fields preserved
- Overwrite same key updates the value

**Key determinism (tests 18-21, 29):**
- `cache_key()` produces deterministic 64-char SHA-256 hex
- Keys differ by model, messages, temperature, and max_tokens
- `None` temperature produces different key than any `Some(T)`

**Eviction and expiry (tests 22-23, 28, 31):**
- Zero TTL disables caching (`put` returns false, `get` always `None`)
- Short TTL entries expire after timeout
- LRU evicts least-recently-used when capacity exceeded (max 2 → third evicts first)
- Eviction counter tracked in stats

**Tool-call exclusion (test 24):**
- `put_if()` with `should_cache` predicate rejects `ThinkResult` with non-empty `tool_calls`

**Stats tracking (tests 25-26, 30):**
- Hit counter and `total_sats_saved` accumulate per cache hit
- Miss counter increments on cache miss
- `hit_rate_pct()` computes percentage; 0.0 when empty
- Empty cache: all stats zero

**Configuration and concurrency (tests 27, 33):**
- `WormConfig` defaults: `cache_ttl_secs=3600`, `cache_max_entries=1000`
- 10 threads × 50 concurrent put+get ops with no panics

### test_x402_discovery.rs -- Registry and manifests (19 tests)

Tests `bsv_worm::x402::registry` and `bsv_worm::x402::discovery` with mockito HTTP mocks:

**Agent registry (7 tests):**
- `list_agents_from()` fetches `/.well-known/agents` and parses 3 agents (banana, whisper, x-research)
- Cache file prevents re-fetch when timestamp is recent
- Expired cache (timestamp=0) triggers re-fetch
- Server error (500) propagates as error
- `resolve_from()` maps agent name to URL
- Path appending: `"banana/generate"` → `"https://...x402agency.com/generate"`
- Case-insensitive: `"BANANA"` resolves same as `"banana"`

**Service manifests (3 tests):**
- `fetch_manifest()` fetches `/.well-known/x402-info` and parses name, endpoints, auth protocol
- `has_payment()` distinguishes paid vs free endpoints
- Missing fields default to empty (endpoints, description, server_identity_key)
- 404 propagates as error

**Manifest summary formatting (7 tests):**
- `format_manifest_summary()` renders human-readable text
- Basic: service name, HTTP method+path, auth/paid badges, input fields, delivery mode, hints
- Rich manifests include: polling config (endpoint, interval, max wait, terminal states, identity scoping), output schema fields, payment tiers with sats+USD, refund policy, timing info
- Sync endpoints omit polling section

**AgentEntry serde (2 tests):**
- Extra JSON fields preserved via `extra` HashMap
- Empty capabilities array roundtrips

## Test helpers

| Helper | File | Purpose |
|--------|------|---------|
| `payment_err(msg)` | test_circuit_breaker.rs | Creates `WormError::payment(msg)` |
| `budget_err(msg)` | test_circuit_breaker.rs | Creates `WormError::budget(msg)` |
| `is_transient_error(err)` | test_circuit_breaker.rs | Wraps `is_transient_message` for `WormError` |
| `make_result(text, sats)` | test_response_cache.rs | Minimal `ThinkResult` without tool_calls |
| `make_tool_call_result(text, sats)` | test_response_cache.rs | `ThinkResult` with one tool_call (never cached) |
| `should_cache(result)` | test_response_cache.rs | Predicate: `tool_calls.is_empty()` |
| `mock_agents_json()` | test_x402_discovery.rs | Mock `/.well-known/agents` with 3 agents |
| `mock_manifest_json()` | test_x402_discovery.rs | Mock `/.well-known/x402-info` with 2 endpoints |
| `rich_manifest_json()` | test_x402_discovery.rs | Manifest with polling, output, tiers, refund, timing |

## Running

```bash
# All x402 tests
cargo test --test test_x402
cargo test --test test_circuit_breaker
cargo test --test test_response_cache
cargo test --test test_x402_discovery

# By keyword
cargo test circuit_breaker
cargo test cache_key
cargo test manifest
cargo test refund

# Single test
cargo test --test test_circuit_breaker -- test_per_endpoint_isolation
```

## Source modules under test

| Test file | Source module | Key types/functions |
|-----------|-------------|---------------------|
| test_x402.rs | `src/x402/payment.rs` | `build_p2pkh_script`, `hash160`, `payment_protocol`, `raw_tx_to_beef`, `raw_tx_to_atomic_beef`, `parse_402_response` |
| test_x402.rs | `src/x402/refund.rs` | `parse_refund` |
| test_circuit_breaker.rs | `src/x402/circuit_breaker.rs` | `CircuitBreaker`, `CircuitBreakerRegistry`, `CircuitState`, `is_transient_message` |
| test_response_cache.rs | `src/x402/cache.rs` | `ResponseCache<T>`, `CacheStats` |
| test_x402_discovery.rs | `src/x402/registry.rs` | `AgentEntry`, `list_agents_from`, `resolve_from` |
| test_x402_discovery.rs | `src/x402/discovery.rs` | `ServiceManifest`, `fetch_manifest`, `format_manifest_summary` |

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Test suite overview, conventions, helpers index
- [src/x402/CLAUDE.md](../../src/x402/CLAUDE.md) -- x402 payment flow, BEEF encoding, service discovery source
- [src/CLAUDE.md](../../src/CLAUDE.md) -- Source module index
- [tests/core/CLAUDE.md](../core/CLAUDE.md) -- ThinkResult tests (consumer of x402 payment results)
- [tests/features/CLAUDE.md](../features/CLAUDE.md) -- Live integration tests including real x402 payments (Tier 5)
