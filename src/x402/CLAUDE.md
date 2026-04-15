# x402
> BRC-29 payment construction, 402 auto-retry flow, and service discovery for paid HTTP endpoints.

## Overview

This module handles the x402 payment protocol: when an HTTP request returns 402 Payment Required, it parses the server's payment requirements, constructs a funded BSV transaction via the wallet, and retries the request with payment attached. The module never touches private keys — all cryptography is delegated to `WalletClient`.

Additionally, the module provides service discovery via two mechanisms: per-service manifests (`/.well-known/x402-info`) and a central agent registry (`x402agency.com/.well-known/agents`).

## Files

### `mod.rs` (12 lines)
Module root. Exports seven submodules: `cache`, `circuit_breaker`, `discovery`, `payment`, `refund`, `registry`, `schema`.

### `payment.rs` (692 lines)
The core payment flow. Contains:
- **`PaymentRequired`** struct — parsed from 402 response headers (`satoshis`, `derivation_prefix`, `version`, `transports`). `supports_multipart()` method checks if server advertises multipart transport.
- **`parse_402_response()`** — extracts payment requirements from `x-bsv-payment-*` headers. Validates version is `"1.0"` and satoshis > 0. Parses `x-bsv-payment-transports` header for transport capabilities.
- **`create_payment()`** — full BRC-29 payment construction:
  1. Generate random 32-byte derivation suffix (base64-encoded).
  2. Derive payment pubkey via BRC-42 (`wallet.get_public_key()`).
  3. Build P2PKH locking script for the derived key.
  4. Create funded transaction via `wallet.create_action()`.
  5. 3-way format detection: AtomicBEEF passes through, BEEF wraps in AtomicBEEF, raw tx wraps in BEEF+AtomicBEEF. Unknown formats are wrapped as raw tx (best-effort).
  6. Base64-encode the transaction.
  7. Return payment JSON + txid.
- **`paid_request()`** — low-level entry point. Makes a raw HTTP request; if 402, enters a retry loop (up to 3 attempts) creating fresh payments each time. No BRC-31 auth. Supports BRC-105 multipart transport: when payment JSON exceeds `MULTIPART_THRESHOLD` (8KB) and server advertises multipart support, sends payment as `multipart/form-data` body part instead of header. Returns `(Response, Option<txid>, Option<sats_paid>)`.
- **`AuthPaidResponse`** struct — response from `authenticated_paid_request()`: `body` (raw bytes), `status`, `payment_txid`, `sats_paid`.
- **`authenticated_paid_request()`** — combines BRC-31 Authrite auth + x402 payment into a single call. Takes an `AuthriteClient` instead of raw `WalletClient`. Handles three scenarios: (a) initial 401 → clears stale session via `clear_session_from()` and retries with fresh handshake, (b) 402 → payment retry loop (up to 3 attempts), (c) 401 during payment loop → clears session and re-sends original request to get fresh 402 headers (doesn't count as a payment attempt, only retried once via `auth_retried` flag). Supports BRC-105 multipart transport for large payments. Returns `AuthPaidResponse` with consumed body bytes. This is the shared foundation for `think.rs` (LLM inference) and `tools/x402_tools.rs` (generic x402 calls).
- **`build_multipart_body()`** — (private) constructs a `multipart/form-data` body with payment JSON as the `x-bsv-payment` part and optional original request body as the `body` part. Returns `(body_bytes, content_type_with_boundary)`.
- **`payment_protocol()`** — returns `json!([2, "3241645161d8"])`, the BRC-42 key derivation protocol ID matching the TS SDK.
- **`build_p2pkh_script()`** — constructs a P2PKH locking script from a compressed public key hex string. Validates 66 hex chars with `02`/`03` prefix.
- **`hash160()`** — Bitcoin's RIPEMD160(SHA256(data)) hash.
- **`raw_tx_to_beef()`** — wraps a raw transaction in BEEF v1 format (BRC-62): header `0100BEEF`, 0 BUMPs, 1 tx, no hasBump flag.
- **`beef_to_atomic_beef()`** — wraps BEEF bytes in AtomicBEEF format: header `01010101`, reversed txid, then BEEF data.
- **`raw_tx_to_atomic_beef()`** — wraps a raw tx in BEEF then AtomicBEEF. Delegates to `beef_to_atomic_beef(raw_tx_to_beef(...))`.
- **`varint()`** — Bitcoin CompactSize/VarInt encoding (private helper).

### `refund.rs` (79 lines)
Excess refund parsing and internalization:
- **`RefundInfo`** struct — parsed refund data: `transaction` (base64), `derivation_prefix`, `derivation_suffix`, `sender_identity_key`, `satoshis`.
- **`parse_refund()`** — looks for `excessRefund` or `refund` key in response JSON. Returns `None` if `already_refunded` flag is set.
- **`process_refund()`** — decodes the base64 transaction and internalizes it via `wallet.internalize_action()` with `"wallet payment"` protocol and payment remittance metadata.

### `discovery.rs` (483 lines)
Per-service manifest parser. Fetches `/.well-known/x402-info` from any x402 agent:
- **`ServiceManifest`** struct — top-level manifest: `name`, `description`, `server_identity_key`, `auth_endpoint`, `auth_protocol`, `endpoints` (Vec), `pricing`, `capabilities`, plus `extra` via `#[serde(flatten)]`.
- **`EndpointInfo`** struct — single endpoint: `path`, `method`, `description`, `auth` (bool), `delivery`, `payment`, `input` (JSON schema), `output`, `hint`, `polling`, `refund`, plus `extra` via flatten. All fields default to empty/null for tolerant deserialization.
- **`EndpointInfo::has_payment()`** — returns true if payment field is non-null and non-false.
- **`fetch_manifest(base_url)`** — convenience wrapper: appends `/.well-known/x402-info` to `base_url` and delegates to `fetch_manifest_from_url()`.
- **`fetch_manifest_from_url(url)`** — plain HTTP GET to an explicit manifest URL. 15-second timeout. No auth or payment needed. Used when the manifest URL is known directly (e.g. from the registry's `x402_info` field for third-party agents with hosted manifests).
- **`format_manifest_summary(manifest)`** — formats a manifest as a human/LLM-readable text summary. Delegates to `schema::format_input_schema()` for input fields. Surfaces per-endpoint: method, path, auth/paid flags, full input schema, cost (static or dynamic), delivery mode, timing info (from `extra`), polling config (endpoint, method, interval, max_wait, terminal_states, identity_scoped, note), payment tiers (sats + USD), output schema (up to 15 fields with nested object summaries), and refund policy.
- Includes 10 unit tests (serde roundtrip, tolerant parsing, formatting, extra field preservation). Default impls for `EndpointInfo` and `ServiceManifest` in test module.

### `registry.rs` (424 lines)
Central agent registry client. Fetches `/.well-known/agents` from `x402agency.com` with disk-based caching:
- **`AgentEntry`** struct — `name`, `display_name`, `url`, `tagline`, `capabilities` (Vec<String>), plus `extra` via flatten.
- **`RegistryResponse`** struct — wrapper with `agents: Vec<AgentEntry>`.
- **`default_cache_path()`** — `$HOME/.local/share/brc31-sessions/x402-registry.json`.
- **`list_agents(cache_path?)`** — fetches agent list from registry, using disk cache with 5-minute TTL. Expired cache files are deleted.
- **`list_agents_from(registry_url, cache_path?)`** — same but with configurable registry URL (for testing).
- **`list_agents_with_ttl(registry_url, cache_path?, cache_ttl)`** — full-control variant with custom cache TTL in seconds.
- **`resolve(identifier, cache_path?)`** — resolves an agent identifier to a full URL:
  - Full URLs (`https://...`) pass through unchanged.
  - Bare names (`banana`) look up the agent and return its base URL.
  - Names with paths (`banana/generate`) resolve to URL + path.
  - Case-insensitive matching. Error message lists available agents on miss.
- **`resolve_x402_info(identifier, cache_path?)`** — resolves an agent identifier to its `/.well-known/x402-info` manifest URL. For agents with an `x402_info` field in the registry (third-party services with hosted manifests), returns that URL directly. Otherwise falls back to `{agent_url}/.well-known/x402-info`. Full URLs get the well-known path appended.
- **`resolve_from(identifier, registry_url, cache_path?)`** — same as `resolve()` but with configurable registry URL.
- Includes 12 tests (serde, caching, expiry, resolve passthrough, name lookup, path append, case insensitivity, unknown name error, extra fields).

### `schema.rs` (690 lines)
Converts x402 manifest input schemas to JSON Schema format and human-readable text:
- **`manifest_input_to_json_schema(input)`** — converts manifest `input` field to standard JSON Schema with `type: "object"`, `properties`, and `required` array. Handles three input variants: Variant A (wrapped in `"schema"` key with optional `"contentType"`), Variant B (flat, fields at top level), and standard JSON Schema style (with `"properties"` key at top level). Property definitions can be either structured objects or freetext strings (Variant C).
- **`parse_freetext_property(text)`** — (private) parses freetext string property descriptions into JSON Schema. Handles three patterns: pipe-delimited enums (`"OVERALL|POLITICS|SPORTS"` → `{type: "string", enum: [...]}`), numeric ranges (`"1-50"` → `{type: "integer", minimum: 1, maximum: 50}`), and plain descriptions (→ `{type: "string", description: ...}`). Extracts parenthetical defaults (`"(default: X)"`) from any pattern. Used for real-world manifests like polymirror that use string shorthand instead of structured property objects.
- **`convert_property(def)`** — (private) converts a single property definition. Delegates to `parse_freetext_property()` for string values (Variant C). For objects: maps `string[]`, `number[]`, `string | string[]`, `number | number[]` to proper JSON Schema array/oneOf types. Normalizes non-standard types (`int`, `float`, `bool`, `binary`, `AtomicBEEF (base64)`, etc.) to valid JSON Schema types (`integer`, `number`, `boolean`, `string`) with original format info appended to description. Handles nested `object` properties recursively. Passes through `description`, `default`, `enum`, `format`, `min`/`max`/`minimum`/`maximum`, `items`, `maxItems`. Adds default `items` schema for arrays (OpenAI requirement).
- **`format_input_schema(input)`** — formats an endpoint's input schema as human-readable text for LLM consumption. Shows each field with type (including array notation), REQUIRED marker, default, enum values, and min/max constraints. Used by `discovery::format_manifest_summary()`.
- Includes 20 tests: 13 for structured variants (both variants, empty/null input, array types, union types, min/max, required extraction, format output, properties passthrough, unknown type normalization, AtomicBEEF type normalization) + 7 for freetext parsing (pipe-delimited enums, enums with defaults, numeric ranges, plain strings, mixed freetext+objects, freetext formatting, polymirror full manifest).

### `cache.rs` (204 lines)
In-memory LRU+TTL response cache for LLM calls. Saves satoshis by avoiding duplicate x402 payments for identical prompts:
- **`CacheEntry`** struct — cached response with `response: ThinkResult`, `inserted_at: Instant`, `sats_cost: u64`.
- **`CacheStats`** struct — monitoring counters: `hits`, `misses`, `total_sats_saved`, `evictions`. `hit_rate_pct()` method.
- **`ResponseCache`** struct — thread-safe via `Mutex` (lock never held across `.await`). Wraps `LruCache<String, CacheEntry>` with configurable TTL and max entries.
  - **`new(ttl_secs, max_entries)`** — creates cache. TTL of 0 effectively disables it (all lookups miss). `max_entries` of 0 is clamped to 1 (LruCache requires NonZeroUsize).
  - **`cache_key(model, messages, temperature, max_tokens)`** — deterministic SHA-256 key from request parameters.
  - **`get(key)`** — returns cached `ThinkResult` on hit. TTL-expired entries are evicted on access. Promotes in LRU order on hit. Tracks `total_sats_saved` on hit.
  - **`put(key, response, sats_cost)`** — stores a response. **Critical invariant: responses containing tool_calls are NEVER cached** because tool side effects may differ between invocations. Returns `true` if stored, `false` if skipped.
  - **`stats()`** / **`len()`** / **`is_empty()`** / **`ttl()`** — accessors.

### `circuit_breaker.rs` (286 lines)
Closed→Open→HalfOpen state machine per endpoint to avoid hammering degraded providers:
- **`CircuitState`** enum — `Closed` (normal), `Open { opened_at }` (rejecting), `HalfOpen` (probe mode).
- **`CircuitBreaker`** struct — per-endpoint breaker with `state`, `failure_count`, `threshold`, `recovery_window`.
  - **`should_allow()`** — Closed: always true. Open: true if recovery window elapsed (transitions to HalfOpen). HalfOpen: true (one probe).
  - **`record_success()`** — resets failure count. HalfOpen→Closed on success.
  - **`record_failure(is_transient)`** — only transient failures count. Closed: increments count, opens at threshold. HalfOpen: re-opens immediately.
- **`is_transient_error(err)`** — classifies `WormError` variants. Transient: timeouts, connection failures, 5xx, 429, `ERR_SERVICE_FAILED`. Non-transient: 4xx (except 429), auth logic errors, budget, config, tool, memory, conversation errors.
- **`CircuitBreakerRegistry`** struct — thread-safe `HashMap<String, CircuitBreaker>` keyed by base URL. `Arc<Mutex>`.
  - **`should_allow(endpoint)`** / **`record_success(endpoint)`** / **`record_failure(endpoint, err)`** — auto-creates breakers on first access with configured threshold/recovery window.
  - **`get_state(endpoint)`** — returns current state (defaults to Closed for unknown endpoints).
  - **`base_url(url)`** — extracts `scheme://host` for keying.
  - **`with_config(threshold, recovery_window)`** — custom configuration constructor.

## Constants

| Name | Value | Location | Purpose |
|------|-------|----------|---------|
| `PAYMENT_VERSION` | `"1.0"` | payment.rs:32 | BRC-29 protocol version |
| `MAX_PAYMENT_ATTEMPTS` | `3` | payment.rs:35 | Max 402 retry attempts before error |
| `MULTIPART_THRESHOLD` | `8_192` | payment.rs:43 | 8KB — switch from header to multipart/form-data transport (BRC-105) when payment JSON exceeds this |
| `DEFAULT_REGISTRY_CACHE_TTL` | `300` | registry.rs:15 | 5-minute TTL for registry disk cache |
| `DEFAULT_REGISTRY_URL` | `"https://x402agency.com/.well-known/agents"` | registry.rs:18 | Default agent registry endpoint |
| `DEFAULT_FAILURE_THRESHOLD` | `5` | circuit_breaker.rs:131 | Consecutive transient failures before circuit opens |
| `DEFAULT_RECOVERY_WINDOW` | `30s` | circuit_breaker.rs:134 | Time circuit stays open before allowing a probe request |

## Public API

### Authenticated + paid request (preferred)
```
authenticated_paid_request(auth, method, url, headers, body?)
  → Ok(AuthPaidResponse { body, status, payment_txid, sats_paid })
```

### Low-level paid request (no auth)
```
paid_request(wallet, method, url, headers?, body?)
  → Ok((Response, Option<txid>, Option<sats_paid>))
```

### Payment construction
```
create_payment(wallet, derivation_prefix, server_identity_key, satoshis, server_url)
  → Ok((payment_json, txid))
```

### 402 parsing
```
parse_402_response(headers) → Ok(PaymentRequired)
```

### Refund handling
```
parse_refund(body_json) → Option<RefundInfo>
process_refund(wallet, refund_info) → Ok(Value)
```

### Service discovery
```
fetch_manifest(base_url) → Ok(ServiceManifest)
fetch_manifest_from_url(url) → Ok(ServiceManifest)
format_manifest_summary(manifest) → String
```

### Agent registry
```
list_agents(cache_path?) → Ok(Vec<AgentEntry>)
resolve(identifier, cache_path?) → Ok(String)
resolve_x402_info(identifier, cache_path?) → Ok(String)
```

### Schema conversion
```
schema::manifest_input_to_json_schema(input) → Value  // JSON Schema object
schema::format_input_schema(input) → String            // Human-readable text for LLM
```

### Response cache
```
cache::ResponseCache::new(ttl_secs, max_entries) → ResponseCache
cache::ResponseCache::cache_key(model, messages, temperature, max_tokens) → String
cache::ResponseCache::get(key) → Option<ThinkResult>
cache::ResponseCache::put(key, response, sats_cost) → bool  // false if tool_calls present
cache::ResponseCache::stats() → CacheStats
```

### Circuit breaker
```
circuit_breaker::CircuitBreakerRegistry::new() → CircuitBreakerRegistry
circuit_breaker::CircuitBreakerRegistry::should_allow(endpoint) → bool
circuit_breaker::CircuitBreakerRegistry::record_success(endpoint)
circuit_breaker::CircuitBreakerRegistry::record_failure(endpoint, err)
circuit_breaker::CircuitBreakerRegistry::get_state(endpoint) → CircuitState
circuit_breaker::is_transient_error(err) → bool
```

### Script/TX utilities
```
build_p2pkh_script(pubkey_hex) → Ok(script_hex)
hash160(data) → Vec<u8>
raw_tx_to_beef(raw_tx) → Vec<u8>
beef_to_atomic_beef(beef, txid_hex) → Vec<u8>
raw_tx_to_atomic_beef(raw_tx, txid_hex) → Vec<u8>
payment_protocol() → Value  // [2, "3241645161d8"]
```

## Payment flow diagram

```
Caller
  │
  ├─ authenticated_paid_request(auth, POST, url, headers, body)
  │   │
  │   ├─ auth.authenticated_request() → initial BRC-31 request
  │   │   ├─ Got 401? → clear_session_from() + retry with fresh handshake
  │   │   └─ Not 402? → return AuthPaidResponse (body consumed)
  │   │
  │   ├─ Got 402 → extract server identity key from headers
  │   │
  │   └─ Payment loop (up to 3 attempts):
  │       │
  │       ├─ parse_402_response() → PaymentRequired
  │       │
  │       ├─ create_payment(auth.wallet(), ...)
  │       │   ├─ Random 32-byte derivation suffix
  │       │   ├─ wallet.get_public_key() → derived pubkey
  │       │   ├─ build_p2pkh_script() → locking script
  │       │   ├─ wallet.create_action() → tx bytes + txid
  │       │   ├─ 3-way format detection:
  │       │   │   ├─ 01010101 → AtomicBEEF (pass-through)
  │       │   │   ├─ 0100BEEF → BEEF → beef_to_atomic_beef()
  │       │   │   ├─ 01000000 → raw tx → raw_tx_to_atomic_beef()
  │       │   │   └─ other    → unknown → raw_tx_to_atomic_beef() (best-effort)
  │       │   └─ Base64-encode → payment JSON
  │       │
  │       ├─ Transport selection:
  │       │   ├─ payment JSON > 8KB AND server supports multipart?
  │       │   │   └─ BRC-105 multipart: build_multipart_body() → form-data
  │       │   └─ Otherwise: x-bsv-payment header with payment JSON
  │       │
  │       ├─ auth.authenticated_request() → retry with auth + payment
  │       │   ├─ Got 401 (once)? → clear session, re-send original → fresh 402 → continue loop
  │       │   ├─ Still 402? → loop again
  │       │   └─ Success → return AuthPaidResponse (body consumed)
  │       │
  │       └─ All attempts failed → WormError::payment
```

Note: `paid_request()` follows the same pattern but without BRC-31 auth and without 401 handling. Both functions support BRC-105 multipart transport: when payment JSON exceeds 8KB (`MULTIPART_THRESHOLD`) and the server advertises multipart support via the `x-bsv-payment-transports` header, payment is sent as a `multipart/form-data` body part instead of the `x-bsv-payment` header.

## Service discovery flow

```
Agent identifier (e.g. "banana/generate")
  │
  ├─ registry::resolve("banana/generate")
  │   ├─ Full URL? → passthrough
  │   ├─ Split "banana" + "/generate"
  │   ├─ list_agents() → check disk cache (5-min TTL)
  │   │   └─ Cache miss? → fetch x402agency.com/.well-known/agents
  │   ├─ Find agent by name (case-insensitive)
  │   └─ Return "https://nano-banana-pro.x402agency.com/generate"
  │
  ├─ registry::resolve_x402_info("banana")
  │   ├─ Full URL? → append /.well-known/x402-info
  │   ├─ Lookup agent → check x402_info field in registry entry
  │   │   ├─ Has x402_info URL? → return hosted manifest URL directly
  │   │   └─ No x402_info? → return {agent_url}/.well-known/x402-info
  │
  ├─ discovery::fetch_manifest(base_url)
  │   └─ Delegates to fetch_manifest_from_url("{base_url}/.well-known/x402-info")
  │
  ├─ discovery::fetch_manifest_from_url(url)
  │   └─ GET {url} → ServiceManifest (15s timeout, no auth)
  │
  └─ discovery::format_manifest_summary() → human-readable text
      └─ Includes: input schemas, cost, delivery, timing, polling, tiers, output, refund
```

## 402 response headers

The server communicates payment requirements via these headers:

| Header | Required | Example |
|--------|----------|---------|
| `x-bsv-payment-version` | Yes | `"1.0"` |
| `x-bsv-payment-satoshis-required` | Yes | `"500"` |
| `x-bsv-payment-derivation-prefix` | Yes | `"aBcDeFgH..."` |
| `x-bsv-payment-transports` | No | `"header,multipart"` |
| `x-bsv-auth-identity-key` | Yes* | `"028045..."` |

\* Not validated by `parse_402_response()` but used by `paid_request()` and `authenticated_paid_request()` for BRC-42 key derivation. Falls back to empty string if missing.

The `x-bsv-payment-transports` header is comma-separated. When it includes `"multipart"`, the client may use BRC-105 multipart/form-data transport for large payments. If absent, only header transport is used (legacy behavior).

## Payment JSON format

Sent to the server via header transport (`x-bsv-payment` header) or BRC-105 multipart transport (`x-bsv-payment` form-data part):

```json
{
  "derivationPrefix": "<from 402 response>",
  "derivationSuffix": "<random 32-byte base64>",
  "transaction": "<base64-encoded AtomicBEEF>"
}
```

## Refund JSON format

Parsed from server response body:

```json
{
  "excessRefund": {
    "transaction": "<base64 tx>",
    "derivationPrefix": "...",
    "derivationSuffix": "...",
    "senderIdentityKey": "028045...",
    "satoshis": 100
  }
}
```

The key can be `"excessRefund"` or `"refund"`. If `"already_refunded": true` is present, `parse_refund()` returns `None`.

## Dependencies

| Crate | Used for |
|-------|----------|
| `reqwest` | HTTP client, Method, Response, StatusCode, HeaderMap |
| `base64` | Encoding transactions and derivation suffixes |
| `sha2` | SHA-256 (first half of hash160) |
| `ripemd` | RIPEMD-160 (second half of hash160) |
| `hex` | Pubkey decoding, script encoding |
| `rand` | Random derivation suffix generation |
| `serde` / `serde_json` | Manifest/registry/payment JSON (de)serialization |
| `url` | Parsing server URL for base extraction |
| `lru` | LRU eviction for response cache |
| `tracing` | Structured logging throughout |
| `tempfile` | Test-only: cache directory isolation |

Internal: `crate::error::WormError`, `crate::wallet::WalletClient`, `crate::auth::AuthriteClient`, `crate::think::ThinkResult`.

## Decisions

- **Wallet does all crypto**: This module never touches private keys. Key derivation (BRC-42), transaction funding, and signing all happen via `WalletClient` HTTP calls. The x402 module only orchestrates the request→402→pay→retry flow.
- **Protocol ID `[2, "3241645161d8"]`**: The payment key derivation protocol matches the TS SDK's `AuthFetch.createPaymentContext()` exactly. This hex string is not arbitrary — it must match for the server to derive the same key and verify payment.
- **3-way AtomicBEEF wrapping**: The wallet's `createAction` may return AtomicBEEF, BEEF, or a raw transaction. `create_payment()` detects the format by header bytes (`01010101` = AtomicBEEF pass-through, `0100BEEF` = BEEF → wrap, `01000000` = raw tx → double-wrap) and normalizes to AtomicBEEF. Unknown formats are wrapped as raw tx (best-effort).
- **Dual transport (BRC-105)**: Payment JSON is sent via the `x-bsv-payment` header by default. When payment JSON exceeds 8KB (`MULTIPART_THRESHOLD`) and the server advertises multipart support via `x-bsv-payment-transports: multipart`, both `paid_request()` and `authenticated_paid_request()` automatically switch to `multipart/form-data` body transport. The multipart body has two parts: `x-bsv-payment` (payment JSON) and optionally `body` (original request body). This avoids Cloudflare Workers' ~16KB typical header limit. MessageBox uses body-transport for different reasons (see `src/messagebox/`).
- **Two request entry points**: `paid_request()` handles raw HTTP+402 without auth. `authenticated_paid_request()` combines BRC-31 auth + 401 recovery + 402 payment into one call, consuming the response body and returning `AuthPaidResponse`. The latter is preferred for all x402 services since they all require BRC-31 auth.
- **401 recovery at two points**: Stale BRC-31 sessions can expire between requests (e.g. server restart, TTL expiry). `authenticated_paid_request()` handles 401 both on the initial request and during the payment retry loop. The payment-loop 401 doesn't count as a payment attempt since the payment itself was valid — only the auth expired. The `auth_retried` flag prevents infinite 401→retry loops.
- **8KB multipart threshold**: The `MULTIPART_THRESHOLD` (8KB) is conservative to stay well under typical HTTP header size limits. When exceeded, multipart transport is preferred but requires server support. If the server doesn't advertise multipart, header transport is used regardless of size (may fail at the CDN layer for very large payments).
- **Refund internalization is caller's responsibility**: `paid_request()` returns the response without consuming the body, so callers can parse refunds themselves. `authenticated_paid_request()` consumes the body into `AuthPaidResponse.body` bytes. The `refund.rs` module provides parsing and wallet internalization helpers.
- **Tolerant manifest/registry deserialization**: Both `ServiceManifest` and `AgentEntry` use `#[serde(default)]` on all fields and `#[serde(flatten)]` for extras. This lets them parse partial or extended JSON without errors — important since x402 agents evolve independently.
- **Registry disk cache**: Agent list is cached to `$HOME/.local/share/brc31-sessions/x402-registry.json` with a 5-minute TTL. Expired files are deleted on read. Cache writes are best-effort (failures silently ignored).
- **Schema type normalization**: Non-standard manifest types (`int`, `float`, `bool`, `binary`, `AtomicBEEF (base64)`) are normalized to valid JSON Schema types for OpenAI compatibility. Original type info is appended to the description field so context isn't lost.
- **Freetext property parsing (Variant C)**: Some x402 services (e.g. polymirror) describe input fields as plain strings instead of structured objects — e.g. `"category": "OVERALL|POLITICS|SPORTS (default: OVERALL)"` or `"limit": "1-50 (default: 25)"`. `parse_freetext_property()` extracts pipe-delimited enums, numeric ranges, and parenthetical defaults from these strings, converting them to proper JSON Schema. Mixed manifests (some fields freetext, some structured) are handled transparently.
- **Rich manifest summaries**: `format_manifest_summary()` extracts and formats all available endpoint metadata — polling config, payment tiers, output schema, timing, refund policy — not just the basic fields. This gives the LLM enough context to use services correctly without extra discovery calls.
- **LRU response cache for cost savings**: `cache.rs` caches LLM responses keyed by SHA-256 of request parameters. Responses with tool_calls are never cached because tool side effects may differ between invocations. TTL of 0 effectively disables the cache. Thread-safe via `Mutex` (never held across `.await`). Inspired by IronClaw's `CachedProvider` pattern.
- **Circuit breaker per endpoint**: `circuit_breaker.rs` prevents hammering degraded providers. After 5 consecutive transient failures the circuit opens, routing to the alternate LLM provider. After 30s, a single probe request is allowed (HalfOpen). Only transient errors (timeouts, 5xx, 429, connection failures, `ERR_SERVICE_FAILED`) count toward the threshold — client errors like 4xx (except 429) never trip the breaker. Registry keyed by base URL so all paths on the same host share one breaker.

## Gotchas

- **`paid_request()` is low-level**: It handles raw HTTP with 402 retry but does NOT handle BRC-31 Authrite headers or 401 session recovery. For authenticated+paid requests (the common case), use `authenticated_paid_request()` which combines auth + payment + stale session handling. `think.rs` and `x402_tools.rs` both use the authenticated variant.
- **`authenticated_paid_request()` consumes the body**: Unlike `paid_request()` which returns a `Response`, `authenticated_paid_request()` reads the full response body into `AuthPaidResponse.body` bytes. Callers parse this as JSON or whatever format they expect.
- **401 stale session recovery**: `authenticated_paid_request()` handles 401 in two places: (a) on the initial request, it clears the cached session file and retries with a fresh BRC-31 handshake; (b) during the payment loop, if a 401 occurs (e.g. server restarted mid-flow), it clears the session, re-sends the original request to get fresh 402 headers, and continues the loop. The payment-loop 401 retry only happens once (`auth_retried` flag) to prevent infinite loops.
- **Multipart transport gated on server support**: Both `paid_request()` and `authenticated_paid_request()` only use multipart transport when the server's 402 response includes `x-bsv-payment-transports: multipart`. Legacy servers without this header always get header transport, even for large payments (which may fail at the CDN layer).
- **Server identity key from 402 headers**: The server's `x-bsv-auth-identity-key` header from the 402 response is used for BRC-42 key derivation. If missing, payment key derivation uses an empty string as counterparty, which will fail server-side.
- **Refund keys**: `excessRefund` and `refund` are both checked when parsing refund responses. The `already_refunded` flag prevents double-internalization.
- **3 payment attempts max**: If the server keeps returning 402 after payment, both request functions retry up to 3 times with fresh payments each time. This handles race conditions but means a misbehaving server could drain up to 3x the requested amount.
- **P2PKH only**: `build_p2pkh_script()` validates compressed public keys (33 bytes / 66 hex chars, prefix `02` or `03`). No support for other script types.
- **`_server_url` unused**: The `create_payment()` function accepts `server_url` but doesn't use it (prefixed with `_`). It's kept in the signature for API compatibility.
- **Transport selection is per-attempt**: Each payment retry independently checks `supports_multipart()` against the latest 402 response headers. The transport used may differ across retries if server behavior changes.
- **Refund output index hardcoded to 0**: `process_refund()` always internalizes output index 0. This matches the current server behavior but would break if servers used different output indices.
- **Registry resolve requires live fetch on cache miss**: `resolve()` will make a network request if the cache is cold or expired. For offline testing, pre-populate the cache file.
- **Schema `format_input_schema()` used by discovery**: The human-readable schema formatting lives in `schema.rs` but is called by `discovery::format_manifest_summary()`. Changes to formatting affect all manifest summaries.
- **Cache skips tool_calls responses**: `ResponseCache::put()` returns `false` and does NOT store responses containing tool_calls. This is intentional — tool side effects may differ between invocations even with identical prompts.
- **Cache TTL 0 disables entirely**: If `ttl_secs` is 0, both `get()` and `put()` return immediately (miss/false). Used to disable caching without removing the cache from the call chain.
- **Circuit breaker is fail-open for unknown endpoints**: `get_state()` returns `Closed` for endpoints with no breaker entry. The breaker is only created on first `should_allow()` or `record_failure()` call.
- **Circuit breaker keys by base URL**: `base_url()` extracts `scheme://host` from a full URL. All paths on the same host share one breaker — a failure on `/v1/chat` affects `/v1/embeddings` too.
- **`is_transient_error` matches by message text**: Error classification uses case-insensitive substring matching on error messages (e.g. "timeout", "http 502", "rate limit"). WormError variants like Budget, Config, Tool, Memory are always non-transient regardless of message content.

## Related

- [`src/auth/`](../auth/CLAUDE.md) — BRC-31 Authrite sessions; `AuthriteClient` is passed to `authenticated_paid_request()`
- [`src/messagebox/`](../messagebox/CLAUDE.md) — Uses body-transport for payments (protocol requirement, not size-based)
- `src/think.rs` — Primary consumer; uses `authenticated_paid_request()` for paid LLM inference
- `src/wallet.rs` — All wallet HTTP calls (`getPublicKey`, `createAction`, `internalizeAction`)
- `src/onchain/budget.rs` — Tracks spending per-service after x402 payments complete
- `src/tools/x402_tools.rs` — 3 generic x402 tools (discover_services, discover_endpoints, x402_call) that use `authenticated_paid_request()`
- `skills/x402/SKILL.md` — Auto-activated skill teaching the agent how to discover and use x402 services via the 3 generic tools
