# x402 Tools
> Generic x402 service call, discovery, and recipe tools for paid API interactions.

## Overview

This module provides 5 tools (3 generic + 2 recipe) that let the agent discover, inspect, and call any x402 paid service. The generic tools (`discover_services`, `discover_endpoints`, `x402_call`) form a discover-learn-call workflow. Recipe tools (`generate_image`, `upload_to_nanostore`) wrap common multi-step flows into single atomic calls. All tools share a `do_x402_request()` helper that handles BRC-31 auth, 402 payment, refund internalization, and payment metadata injection. Optional rate limiting (via `RateLimiterRegistry`) is supported for server mode.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 405 | `do_x402_request()` shared helper, `all_x402_tools()` / `all_x402_tools_with_rate_limiter()` and `all_x402_recipe_tools()` / `all_x402_recipe_tools_with_rate_limiter()` registration functions, `ManifestCache` type alias, shared `Arc<Mutex<_>>` state for circuit breaker/discovery/manifests/tips |
| `discovery.rs` | 691 | `discover_services_impl` (registry browse with category filter), `discover_endpoints_impl` (manifest fetch + circuit breaker reset + provider tips), `ValidationRule` struct, parameter validation engine, provider tips/constraints/section extraction helpers, `extract_provider_name()` |
| `call.rs` | 1086 | `x402_call_impl` (full call flow: resolve → validate → circuit breaker → auth+pay → auto-poll → upload script → error enrichment), `poll_for_result()` async poller, `find_endpoint()` manifest matcher, `extract_base_url()`, `generate_upload_script()` |
| `recipe.rs` | 506 | `generate_image_impl` (banana auto-poll wrapper), `upload_to_nanostore_impl` (two-step reserve + PUT), `mime_from_extension()`, `sniff_content_type()` |

## Key Exports

### `do_x402_request(auth, url, body) -> Result<Value, DmError>` (mod.rs)

Shared helper used by both `x402_call` and recipe tools. Flow:
1. Serialize body to JSON bytes
2. Call `payment::authenticated_paid_request()` (BRC-31 auth + 402 payment)
3. Parse refund info from response, internalize via wallet if present
4. Inject `payment_txid` and `sats_paid` into response JSON

### `all_x402_tools(wallet_url, registry_url) -> Vec<ToolDef>` (mod.rs)

Convenience wrapper that delegates to `all_x402_tools_with_rate_limiter()` with `rate_limiter: None`.

### `all_x402_tools_with_rate_limiter(wallet_url, registry_url, rate_limiter) -> Vec<ToolDef>` (mod.rs)

Creates 3 generic tool definitions. `registry_url` is captured at registration time and passed to `discover_services` for agent listing. `rate_limiter: Option<Arc<RateLimiterRegistry>>` — when provided (server mode), `x402_call` resolves the service URL and acquires a rate limit token before each request. Shared state across tools:
- `discovered: Arc<Mutex<HashSet<String>>>` — providers already auto-discovered
- `failures: Arc<Mutex<HashMap<String, u32>>>` — per-endpoint failure counts (circuit breaker)
- `manifests: ManifestCache` — cached `ServiceManifest` per provider
- `tips_shown: Arc<Mutex<HashSet<String>>>` — providers whose tips have been shown

### `all_x402_recipe_tools(wallet_url) -> Vec<ToolDef>` (mod.rs)

Convenience wrapper that delegates to `all_x402_recipe_tools_with_rate_limiter()` with `rate_limiter: None`.

### `all_x402_recipe_tools_with_rate_limiter(wallet_url, rate_limiter) -> Vec<ToolDef>` (mod.rs)

Creates 2 recipe tool definitions. `rate_limiter: Option<Arc<RateLimiterRegistry>>` — when provided, each recipe tool resolves its target URL and acquires a rate limit token before calling `do_x402_request`. Each gets its own `AuthriteClient` per call (no shared state across invocations).

## Tools

### Generic Tools (always-on, in every system prompt)

| Tool | Cost | `deferred` | Description |
|------|------|------------|-------------|
| `discover_services` | FREE | false | Browse agent registry with optional category filter |
| `discover_endpoints` | FREE | false | Fetch `/.well-known/x402-info` manifest for a service, reset circuit breaker, append provider tips |
| `x402_call` | PAID | false | Call any endpoint with auto auth + payment + refund handling |

### Recipe Tools (discoverable via `search_tools`)

| Tool | Cost | `deferred` | `search_hint` | Description |
|------|------|------------|---------------|-------------|
| `generate_image` | ~$0.19 | true | "Generate images via x402 (~$0.19)" | Banana image generation with auto-poll (1-3 min) |
| `upload_to_nanostore` | ~730 sats/MB/yr | true | "Upload files to NanoStore (~730 sats/MB/yr)" | Two-step reserve + PUT upload with auto-execution |

All `ToolDef` structs set `always_load: false` and `cleanup: None`. Generic tools use `deferred: false`; recipe tools use `deferred: true` with `search_hint` for progressive discovery via `search_tools`.

## Rate Limiting (mod.rs)

All 5 tools support optional rate limiting via `RateLimiterRegistry` from `crate::x402::rate_limit`. The pattern is:
1. The `_with_rate_limiter` constructor captures `Option<Arc<RateLimiterRegistry>>` in each tool closure
2. Before making an x402 request, the tool resolves the service shorthand to a full URL via `registry::resolve()`
3. Calls `rl.acquire(&url)` to obtain a rate limit token — blocks if the rate limit is exceeded
4. On rate limit error, returns an error string immediately without making the x402 call

Rate limiting is used in server mode to enforce cert-driven rate throttling. CLI mode passes `None` (no rate limiting).

## x402_call Flow (call.rs)

1. **Parse params** — extract `service`, `method` (default POST), `parameters`/`body`. Auto-collects stray top-level keys into body. Parses JSON string bodies back to objects.
2. **Pre-flight: empty body check** — rejects POST with empty `parameters`, instructs LLM to call `discover_endpoints` first.
3. **Pre-flight: provider validation** — loads `ValidationRule`s from provider tips, validates against rules (required, >=, <=, >, <, in operators).
4. **Auto-discover** — on first use of a provider, fetches and caches its manifest.
5. **Circuit breaker** — after 2+ consecutive failures for the same `service:method`, blocks further calls and suggests `discover_endpoints`. Reset when `discover_endpoints` is called for that provider.
6. **Resolve URL** — `registry::resolve()` turns shorthand (e.g. `banana/generate`) into full URL.
7. **Execute** — GET uses `authenticated_paid_request` directly; POST uses `do_x402_request`.
8. **Auto-poll** — if cached manifest indicates `async-poll` delivery, saves `sats_paid`/`payment_txid` from original paid response, then polls status endpoint until terminal state. Payment fields are re-injected into the polled result via `entry().or_insert()` (preserves polled values if already present).
9. **Upload script** — if response contains `uploadURL`/`presignedUrl`/`upload_url`, generates a ready-to-run curl PUT command.
10. **Error enrichment** — on first failure for a provider, appends full provider tips; on subsequent failures, appends key constraints only.

## Provider Tips System (discovery.rs)

Provider tips are markdown files at `skills/x402/providers/{name}.md`. The system extracts structured data from them:

- **`load_provider_tips(name)`** — reads full `.md` file (checks `skills/` then `working/skills/`), skips `_`-prefixed files
- **`load_key_constraints(name)`** — extracts `## Key Constraints` section
- **`load_validation_rules(name)`** — parses `## Validation Rules` section into `ValidationRule` structs
- **`extract_provider_name(service)`** — resolves shorthand (`banana/generate` -> `banana`) or URL (`https://nano-banana-pro.x402agency.com/generate` -> `banana`) using `known_provider_names()` scan of `.md` files
- **`extract_section(text, name)`** — generic markdown `## Section` extractor

### ValidationRule Format

Rules are parsed from provider tip lines: `- field operator value | error message`

```
- retentionPeriod >= 180 | retentionPeriod must be >= 180 minutes
- fileSize required | fileSize is a required field
- resolution in [1K,2K,4K] | resolution must be 1K, 2K, or 4K
```

Operators: `required`, `>=`, `>`, `<=`, `<`, `in`.

## Recipe Tool Details

### generate_image (recipe.rs)

1. Validates `prompt` (required), reads `resolution` (default "1K") and `aspect_ratio` (default "1:1")
2. Resolves `banana/generate` via registry
3. Calls `do_x402_request` with prompt/resolution/aspect_ratio body
4. Fetches manifest polling config (falls back to hardcoded defaults: 15s interval, 300s max)
5. Polls until terminal state
6. Extracts `output` URL from final response (handles both string and array formats)
7. Returns `{ success, url, sats_paid, payment_txid, message }`

### upload_to_nanostore (recipe.rs)

1. Reads content from `file_path` or `content` param (one required)
2. Validates `retention_minutes >= 180` (default 525600 = 1 year)
3. Resolves `nanostore/upload` via registry
4. **Step 1**: `do_x402_request` to reserve upload slot with `{ fileSize, retentionPeriod }`
5. **Step 2**: Extracts `uploadURL`/`presignedUrl`/`upload_url` and `requiredHeaders` from response
6. Resolves content type: explicit param > file extension (`mime_from_extension`) > content sniffing (`sniff_content_type`) > `application/octet-stream`
7. Executes `curl -s -X PUT` with headers and content directly (no manual step)
8. Returns `{ success, public_url, content_sha256, sats_paid, payment_txid, retention_minutes, file_size, message }`

## Auto-Poll System (call.rs)

`poll_for_result(auth, base_url, response, polling_config)` handles async services:

1. Extracts ID from response (checks: `id`, `prediction_id`, `task_id`, `job_id`)
2. Parses polling config: `endpoint` template, `interval_seconds` (default 15), `max_wait_seconds` (default 180), `terminal_states`
3. Builds status URL from template with `{prediction_id}`, `{id}`, `{task_id}`, `{job_id}` substitution
4. Polls via `authenticated_paid_request` GET until terminal state or timeout
5. On failure terminal states, attempts refund internalization from the polled response before returning `Err`
6. Returns `Ok(body)` for success states (`succeeded`, `completed`), `Err` for failure states (`failed`, `canceled`, `error`)

## Related

- `../CLAUDE.md` — parent tools module overview
- `../../x402/` — payment flow (`payment.rs`), service discovery (`discovery.rs`), registry (`registry.rs`), refund handling (`refund.rs`), rate limiting (`rate_limit.rs`)
- `skills/x402/SKILL.md` — auto-activated skill that teaches the LLM the discover-learn-call workflow
- `skills/x402/providers/` — per-provider tip files (banana, nanostore, x-research, whisper, veo, kling, 1sat, openai, claude, polymirror)
