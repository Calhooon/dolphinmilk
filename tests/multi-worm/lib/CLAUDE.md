# Utilities — Multi-Worm Test Infrastructure

> Shared JavaScript libraries for BRC-31 authenticated requests and scenario schema validation in multi-worm integration tests.

## Overview

Two zero-dependency Node.js modules (built-ins only) used by `orchestrate.js` and `run-scenarios.js` to authenticate against worm servers and validate scenario JSON files.

## Files

| File | Lines | Purpose |
|------|------:|---------|
| auth.js | 345 | BRC-31/103/104 authenticated HTTP client — signs requests via parent wallet, sends to worm servers |
| validate.js | 479 | Schema validator for `scenarios.json` — validates scenario structure, steps, assertions, and constraints |

## auth.js

BRC-31 Authrite client that signs requests through the parent wallet (MetaNet Client on port 3321) and sends them directly to worm HTTP servers. This is **not** AuthFetch — AuthFetch routes through the wallet; this client signs via the wallet but sends to the target server itself.

### Exported Functions

```javascript
const { authGet, authPost, authRequest, clearAuthCache } = require('./lib/auth');
```

#### `authRequest(method, url, parentWalletPort, bodyObj?)`
Core authenticated request. Performs the full BRC-31 flow:
1. Gets parent identity key via `getPublicKey` from the wallet
2. Performs BRC-31 handshake with the worm server (`/.well-known/auth`) if no cached session
3. Generates per-request nonce and request ID (32 random bytes each)
4. Serializes the request in BRC-104 binary format for signing (body excluded — see note below)
5. Signs via wallet's `createSignature` with protocol `[2, 'auth message signature']`
6. Sends to the worm server with `x-bsv-auth-*` headers

Returns `{ status: number, body: any }`.

#### `authGet(url, parentWalletPort)`
Convenience wrapper — calls `authRequest('GET', ...)`.

#### `authPost(url, data, parentWalletPort)`
Convenience wrapper — calls `authRequest('POST', ...)`.

#### `clearAuthCache()`
Clears the in-memory BRC-31 session cache. Call between test runs if sessions become stale.

### Internal Functions

| Function | Purpose |
|----------|---------|
| `writeVarint(n)` | Bitcoin-style varint encoding (1/3/5/9 bytes) per BRC-104 spec |
| `serializeRequest(requestId, method, path, query, headers, body)` | BRC-104 binary serialization of an HTTP request for signing |
| `filterSignableHeaders(headers)` | Filters to `x-bsv-*` (excluding `x-bsv-auth-*`), `authorization`, `content-type` (media type only); sorts alphabetically |
| `walletPost(walletPort, endpoint, body)` | Low-level HTTP POST to the MetaNet Client wallet API with 10s timeout |
| `handshake(wormPort, identityKey)` | BRC-31 initial handshake — cached per `localhost:{port}` in a module-level `Map` |

### Key Implementation Detail

The request body is **not** included in the BRC-104 signature. The worm server's axum handler consumes the body via `Json` extractor before auth verification, so both client and server serialize with `body = null`. This is documented in the code at lines 268-270.

### BRC-31 Auth Header Format

Signed requests include these headers:
- `x-bsv-auth-version`: `"0.1"`
- `x-bsv-auth-identity-key`: hex-encoded public key
- `x-bsv-auth-message-type`: `"general"`
- `x-bsv-auth-nonce`: base64-encoded 32-byte random nonce
- `x-bsv-auth-your-nonce`: server's nonce from handshake
- `x-bsv-auth-signature`: hex-encoded ECDSA signature
- `x-bsv-auth-request-id`: base64-encoded 32-byte request ID

### Signing Key Derivation

The wallet `createSignature` call uses:
- `protocolID`: `[2, 'auth message signature']`
- `keyID`: `"{messageNonce} {serverNonce}"` (space-separated)
- `counterparty`: server's identity key from handshake

## validate.js

Schema validator for `scenarios.json` files. Validates the complete multi-worm scenario format including steps, assertions, agents, budget constraints, and naming conventions. Usable as a module or standalone CLI.

### Exported Functions

```javascript
const { validateScenario, validateScenarios, validateScenariosFile } = require('./lib/validate');
```

#### `validateScenario(scenario, index?)`
Validates a single scenario object. Returns `{ valid: boolean, errors: string[] }`.

Checks:
- Required fields: `id` (number ≥ 1), `tier`, `name` (snake_case), `description`, `agents` (≥ 2), `steps` (≥ 1)
- Agent names are snake_case, no duplicates
- Each step validated via `validateStep()`
- No duplicate `store_as` keys across steps
- Optional `assertions`, `budget`, `skip`, `timeout_seconds`, `tags`, `note`

#### `validateScenarios(data)`
Validates a full scenarios file (the parsed JSON root object). Returns `{ valid: boolean, errors: string[], scenarioCount: number }`.

Checks:
- `schema_version` must be `"2.0"`
- `scenarios` array required with ≥ 1 entry
- No duplicate scenario IDs or names across the file
- Each scenario validated via `validateScenario()`

#### `validateScenariosFile(filePath)`
Reads, parses, and validates a JSON file. Prints summary to stdout/stderr and sets `process.exitCode = 1` on failure.

### Valid Constants

| Constant | Values |
|----------|--------|
| `VALID_TIERS` | `canary`, `basic`, `security`, `economic`, `full` |
| `VALID_ACTIONS` | `task`, `wait`, `assert`, `api` |
| `VALID_EVENTS` | `task_complete`, `message_received`, `proof_created`, `budget_updated` |
| `VALID_METHODS` | `GET`, `POST`, `PUT`, `DELETE` |
| `VALID_ASSERTION_TYPES` | `response_contains`, `response_not_contains`, `response_matches`, `status_code`, `field_equals`, `field_exists`, `cost_within`, `agents_different_keys`, `session_no_error`, `tool_succeeded`, `tool_result_contains`, `no_tool_errors` |

### Step Validation Rules

| Action | Required Fields |
|--------|----------------|
| `task` | `agent`, `message` |
| `wait` | `agent`, `event` (must be one of `VALID_EVENTS`) |
| `api` | `agent`, `path`; optional `method` (must be one of `VALID_METHODS`) |
| `assert` | `agent`, `condition` (validated as an assertion object) |

All steps support optional `timeout_seconds` (≥ 1), `max_cost_sats` (≥ 0), `max_iterations` (≥ 1), and `store_as` (snake_case).

### Assertion Type Validation Rules

Each assertion type has specific required fields:

| Assertion Type | Required Fields |
|----------------|----------------|
| `response_contains`, `response_not_contains`, `response_matches` | `value` (string) |
| `status_code` | `value` (number) |
| `field_exists`, `field_equals` | `field` (string) |
| `cost_within` | `value` (number ≥ 0) |
| `session_no_error`, `no_tool_errors` | `step` (string — references a stored step result) |
| `tool_succeeded` | `step` (string), `tool_name` (string) |
| `tool_result_contains` | `step` (string), `tool_name` (string), `value` (string) |
| `agents_different_keys` | (no extra fields) |

The transcript-inspection types (`session_no_error`, `tool_succeeded`, `tool_result_contains`, `no_tool_errors`) reference stored step results by name and inspect the agent's task transcript for tool execution outcomes.

### Standalone CLI Usage

```bash
# Validate default scenarios.json
node lib/validate.js

# Validate a specific file
node lib/validate.js path/to/scenarios.json

# Via npm script (from tests/multi-worm/)
npm run validate
```

## Usage Patterns

### Authenticated chat in orchestrate.js

```javascript
const { authGet, authPost } = require('./lib/auth');

// Send authenticated chat message to a worm
const resp = await authPost(
  `http://localhost:${wormPort}/chat`,
  { message: 'What is your identity key?' },
  3321  // parent wallet port
);

// Check agent identity
const agent = await authGet(`http://localhost:${wormPort}/agent`, 3321);
```

### Schema validation in package.json

```json
{
  "scripts": {
    "validate": "node -e \"require('./lib/validate').validateScenariosFile('./scenarios.json')\""
  }
}
```

## Related

- [tests/CLAUDE.md](../../CLAUDE.md) — Test suite overview, conventions, and helpers
- [tests/integration/CLAUDE.md](../../integration/CLAUDE.md) — Playwright E2E tests (single-worm)
- [src/auth/CLAUDE.md](../../../src/auth/CLAUDE.md) — Rust BRC-31 Authrite implementation (server side)
- [scenarios.json](../scenarios.json) — The scenario definitions validated by `validate.js`
