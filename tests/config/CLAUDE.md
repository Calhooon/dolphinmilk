# tests/config

> 133 tests across 5 files covering TOML configuration loading, environment variable overrides, hot-reload safety, unified data directory, error hierarchy, newtype ID wrappers, wallet client construction, and embedded wallet operations.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_config.rs | 47 | `WormConfig` loading, defaults, partial TOML, env overrides, hot-reload safe fields, `dolphin-milk.toml.example` validation, unified `data_dir` path derivation |
| test_embedded_wallet.rs | 42 | `EmbeddedWalletClient` (feature-gated `embedded-wallet`): construction, key derivation, crypto roundtrips, raw_call routing, error handling, security levels |
| test_types.rs | 26 | Newtype ID wrappers (`TaskId`, `SessionId`, `ConversationId`, `ProofTxid`): construction, Display, From, Deref, serde, Hash, type safety |
| test_error.rs | 12 | `WormError` variants: construction helpers, Display format, `ErrorContext`, Send+Sync, pattern matching |
| test_wallet.rs | 6 | `WalletClient` construction, `ANYONE_KEY` constant, trailing-slash stripping, `from_config()` |

## What each file tests

### test_config.rs — TOML config loading, hot reload, and data directory (47 tests)

**Config loading** (6 tests): Default values, missing file falls back to defaults, full TOML parsing, partial TOML with default backfill, empty TOML, invalid TOML returns error.

**Section defaults** (3 tests): Budget hierarchy (`max_per_task < max_per_hour < max_per_day`), LLM defaults (`openai-agent`, no fallback), heartbeat defaults (enabled, 60s poll).

**Heartbeat & reflection config** (3 tests): Heartbeat overrides from TOML, reflection defaults (disabled, 60s interval, `claude-haiku-4-5-20251001` model, max 3 iterations), reflection overrides from TOML.

**Compaction config** (1 test): Compaction enabled by default, no default compaction model.

**Hot reload — `reload_safe_fields()`** (8 tests): Verifies which config sections are hot-reloadable and which are frozen:
- **Reloadable:** `heartbeat`, `budget`, `llm`, `rates`
- **Frozen:** `wallet`, `parent`, `logging`, `server`

**5-tier budget limits** (4 tests): `max_per_week`, `max_per_month`, `max_lifetime` defaults and TOML parsing, enforcement mode (`"strict"` default, `"advisory"` override), hot-reload of new budget fields.

**x402 registry config** (4 tests): Default registry URL matches `DEFAULT_REGISTRY_URL` constant, custom URL from TOML, `DOLPHIN_MILK_X402_REGISTRY_URL` env var overrides TOML, cache TTL default (300s).

**Rates config** (4 tests): Defaults (60s refresh, 0% margin, 300s stale), full TOML override, partial TOML with default backfill, hot-reload.

**dolphin-milk.toml.example validation** (2 tests): Example file parses to defaults (all values commented out), all 16 config sections present (`[wallet]`, `[budget]`, `[llm]`, `[logging]`, `[memory]`, `[heartbeat]`, `[heartbeat.active_hours]`, `[parent]`, `[mcp]`, `[server]`, `[certificates]`, `[browser]`, `[lifecycle]`, `[compliance]`, `[x402]`, `[rates]`).

**Unified data directory — `data_dir` and derived paths** (13 tests, Issue #160): Tests the `~/.dolphin-milk/` data directory system where all paths derive from a single root:
- **Resolution** (4): Default resolves to `~/.dolphin-milk`, explicit config path honored, env var simulation, TOML `data_dir` override with env cleanup
- **Tilde expansion** (1): `~/my-worm` expands to absolute path (no raw `~` in result)
- **Derived paths** (4): `workspace_dir()` → `{data_dir}/workspace`, `memory_dir()` → `{data_dir}/workspace/memory`, `config_path()` → `{data_dir}/dolphin-milk.toml`, `skills_dir()` → `{data_dir}/skills`
- **Memory dir overrides** (2): Explicit `memory.base_dir` honored; old default `/testbed/memory` treated as unset and falls through to derived path
- **Backward compat** (2): No `data_dir` set → memory resolves to `~/.dolphin-milk/workspace/memory`; default `memory.base_dir` changed from `/testbed/memory` to `"memory"`

### test_embedded_wallet.rs — EmbeddedWalletClient (42 tests, feature-gated)

Tests the in-process wallet via `bsv-wallet-toolbox-rs`. Only compiled with `--features embedded-wallet`. Uses private key = 1 (secp256k1 generator point G) as test root key. All tests are async (`#[tokio::test]`).

**Construction** (5 tests):
- `init()` creates new wallet in tempdir SQLite
- `open()` opens an existing DB created by prior `init()`
- Invalid root key (`"not_a_valid_hex_key"`) returns error mentioning "invalid root key"
- Non-existent DB path — no panic (result depends on SQLite behavior)
- Testnet chain (`Chain::Test`) initialization succeeds

**Identity & key management** (7 tests):
- `get_identity_key()` returns the expected compressed pubkey for private key = 1
- `get_public_key()` derives a key different from identity key (different BRC-42 derivation path)
- `for_self` flag: both `true` and `false` return valid 66-char compressed keys
- Different `key_id` values derive different keys
- Counterparty `"anyone"` and explicit hex counterparty (`ANYONE_KEY`) both work

**Status & metadata** (4 tests):
- `is_authenticated()` always returns `true` for embedded wallet
- `get_version()` returns non-empty string
- `get_network()` returns `"mainnet"` for `Chain::Main`, `"testnet"` for `Chain::Test`

**raw_call routing** (6 tests): Tests the generic `raw_call(method, params)` dispatch:
- `isAuthenticated`, `getVersion`, `getNetwork` — routed correctly, return expected JSON shapes
- `getPublicKey` with `identityKey: true` returns identity key; with protocol/key params returns derived key
- Unsupported method returns error containing "unsupported method"

**Cryptographic operations** (7 tests):
- **Signature** (2): `create_signature()` + `verify_signature()` roundtrip succeeds; wrong data fails verification (returns `Ok(false)` or `Err`)
- **Encrypt/decrypt** (2): Roundtrip for non-empty and empty data; ciphertext differs from plaintext
- **HMAC** (3): `create_hmac()` + `verify_hmac()` roundtrip; wrong data fails; same inputs produce deterministic 32-byte HMAC

**Output & balance (empty wallet)** (4 tests):
- `get_balance()` returns 0 for fresh wallet
- `list_outputs()` returns empty or no outputs array
- `list_actions()` returns `totalActions: 0`
- `list_certificates()` returns `totalCertificates: 0`

**Error handling** (6 tests): Protocol ID validation and input validation:
- Non-array protocol_id → "protocol_id must be a JSON array"
- Non-integer `protocol_id[0]` → "protocol_id[0] must be an integer"
- Missing `protocol_id[1]` → "protocol_id[1] must be a string"
- Invalid security level (99) → "invalid security level"
- Invalid counterparty key → "invalid counterparty key"
- HMAC with wrong length (16 bytes instead of 32) → "must be exactly 32 bytes"

**Clone** (1 test): Cloned client shares state — both return same identity key.

**Security levels** (3 tests): Levels 0 (silent), 1 (app), 2 (counterparty) all derive valid 66-char compressed keys.

### test_types.rs — Newtype ID wrappers (26 tests)

Tests the macro-generated ID types from `src/types.rs`. All four types (`TaskId`, `SessionId`, `ConversationId`, `ProofTxid`) share the same trait implementations, tested across all of them:

- **Construction** (4): `new()` from `&str` and `String`, `from_string()`, `into_inner()` returns owned String
- **Display** (2): Renders as inner string, works in format strings
- **From conversions** (2): `From<String>`, `From<&str>`
- **AsRef/Deref** (4): `AsRef<str>`, `Deref` enables `str` methods (`.to_lowercase()`, `.starts_with()`, `.len()`)
- **Serde** (4): Serializes as plain JSON string (transparent), deserializes from plain string, roundtrip, nested in structs
- **Hash & Eq** (4): Equal values are `==`, different values are `!=`, `HashSet` contains, `HashSet` deduplicates
- **Clone** (1): Independent clone
- **Type safety** (2): All four types have distinct `TypeId`, function parameterized on `TaskId` rejects `SessionId` at compile time
- **Edge cases** (3): Empty string, Unicode (emoji), whitespace preserved

### test_error.rs — WormError hierarchy (12 tests)

Tests the `WormError` enum and its convenience constructors:

- **Variant constructors** (6): `wallet()`, `payment()`, `tool()`, `budget()`, `config()`, `loop_err()` — each takes a message string
- **ErrorContext** (2): `wallet_with()` and `payment_with()` attach `serde_json::Value` metadata via `ErrorContext` map
- **Display format** (1): Verifies all variants format as `"{variant} error: {message}"`
- **WormResult type alias** (1): `WormResult<T>` is `Result<T, WormError>`
- **Send + Sync** (1): Compile-time check for async compatibility
- **Pattern matching** (1): Match on `WormError::Budget { message, .. }` to extract fields

### test_wallet.rs — WalletClient construction (6 tests)

Tests `WalletClient` creation without a running wallet:

- **ANYONE_KEY** (1): Verifies the secp256k1 generator point G is 66 hex chars, starts with `02`, matches the exact well-known value
- **Constructor** (2): `WalletClient::new()` sets `url`, `origin`, `timeout`; trailing slash is stripped from URL
- **From config** (1): `WalletClient::from_config(&WalletConfig::default())` produces correct defaults
- **Clone** (1): Cloned client has same `url` and `origin`
- **Custom values** (1): Non-default port and origin

## Config sections tested

| Section | Defaults tested | TOML parsing | Hot-reloadable |
|---------|:-:|:-:|:-:|
| `data_dir` | None (resolves to ~/.dolphin-milk) | yes | — |
| `wallet` | url, origin, timeout | yes | no |
| `budget` | 8 fields (task/hour/day/week/month/lifetime, low_power_threshold, enforcement) | yes | yes |
| `llm` | provider, fallback, compaction, model, max_tokens, context_window | yes | yes |
| `logging` | level, format | yes | no |
| `memory` | base_dir default ("memory"), explicit override, old default fallthrough | yes | — |
| `heartbeat` | enabled, poll_secs, reflection (4 fields), max_concurrent | yes | yes |
| `parent` | identity_key | — | no |
| `server` | openai_compat_enabled | — | no |
| `x402` | registry_url, cache_ttl | yes | — |
| `rates` | refresh_interval, margin_percent, stale_threshold | yes | yes |

## Patterns

- **TOML testing via tempfile**: Create a `TempDir`, write TOML content to a file, pass the path to `load_config()`. Always clean up automatically.
- **Env var testing**: Save current value, set override, run test, restore original. Note: env vars are process-global and can race with parallel tests — some tests use direct `toml::from_str()` parsing instead of `load_config()` to avoid this.
- **Default backfill**: Partial TOML files only set some fields; unset fields retain `WormConfig::default()` values.
- **Hot-reload safety**: `reload_safe_fields()` tests verify both what changes (budget, heartbeat, llm, rates) and what is frozen (wallet, parent, logging, server) — security-sensitive sections cannot be hot-swapped.
- **Embedded wallet via tempdir + SQLite**: `make_wallet()` helper creates a fresh `EmbeddedWalletClient` in a `TempDir` with private key = 1 and `Chain::Main`. Returns `(client, _dir)` — keep `_dir` alive to prevent cleanup. All embedded wallet tests are async with `#[tokio::test]`.
- **Feature gating**: `test_embedded_wallet.rs` is gated behind `#![cfg(feature = "embedded-wallet")]` — only compiled and run when the feature is active.
- **Protocol ID convention**: Embedded wallet tests use `[2, "worm test"]` as the standard BRC-42 protocol ID. Note: the SDK validates that protocol names must NOT end with " protocol".

## Running

```bash
cargo test --test test_config              # 47 config loading + data_dir tests
cargo test --test test_types               # 26 newtype ID tests
cargo test --test test_error               # 12 error hierarchy tests
cargo test --test test_wallet              # 6 wallet client tests
cargo test --test test_embedded_wallet \
  --features embedded-wallet               # 42 embedded wallet tests

cargo test config                          # Pattern match across all test files
```

## Related

- [tests/CLAUDE.md](../CLAUDE.md) — Parent test directory overview, conventions, helpers
- [src/config/CLAUDE.md](../../src/config/CLAUDE.md) — Config module implementation
- [src/error.rs](../../src/error.rs) — `WormError` enum definition
- [src/types.rs](../../src/types.rs) — Newtype ID macro and definitions
- [src/wallet/](../../src/wallet/) — `WalletClient` and `EmbeddedWalletClient` implementation
- [dolphin-milk.toml.example](../../dolphin-milk.toml.example) — Reference config with all sections
