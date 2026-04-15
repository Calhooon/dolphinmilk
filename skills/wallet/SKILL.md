---
name: wallet
description: "Generic bridge to any BRC-100 wallet endpoint via wallet_call. Use for discovery, certificates, HMAC, key linkage, and transaction management — endpoints not covered by existing dedicated wallet tools."
auto_activate: true
tools: [wallet_call]
---

# BRC-100 Wallet Endpoints

`wallet_call` is a generic bridge to the local BRC-100 wallet. One tool, any endpoint.

```json
wallet_call({"endpoint": "discoverByIdentityKey", "params": {"identityKey": "03...", "limit": 10}})
```

## Common Workflows

### Receive BSV from Anyone

Use `receive_address` (discoverable via `search_tools`) to generate a BSV address:

```json
receive_address({"suffix": "1"})
// → { "address": "1A1zP1...", "public_key": "03...", "suffix": "1" }
```

Share the address with the sender. After they send, internalize via one of two paths:

**If the sender gives you the BEEF directly** (e.g. via chat or MessageBox):
```json
fund_from_tx({"beef_hex": "0100beef...", "vout": 0, "suffix": "1"})
```

**If you only have the txid** (fetch BEEF from WhatsOnChain):
```json
fund_from_tx({"txid": "abc123...64hex...", "vout": 0, "suffix": "1"})
```

The `suffix` must match between `receive_address` and `fund_from_tx`. Use different suffixes to generate different addresses (e.g. for different senders or purposes).

### Send BSV to an Address

Use `wallet_call` with `createAction` to build and broadcast a payment:

```json
wallet_call({"endpoint": "createAction", "params": {
  "description": "Send 10000 sats",
  "outputs": [{"lockingScript": "76a914<hash160>88ac", "satoshis": 10000, "outputDescription": "payment"}]
}})
```

### Create an On-Chain OP_RETURN Proof

To record data on-chain via OP_RETURN, build the locking script as `006a` + push-data-length + hex-encoded data. OP_RETURN outputs **must have 0 satoshis** (miners reject non-zero OP_RETURN):

```json
wallet_call({"endpoint": "createAction", "params": {
  "description": "On-chain proof",
  "outputs": [{"lockingScript": "006a...<hex-data>", "satoshis": 0, "outputDescription": "runtime proof", "basket": "worm-runtime-proofs"}],
  "options": {"acceptDelayedBroadcast": false, "randomizeOutputs": false}
}})
```

The locking script format: `00` (OP_FALSE) + `6a` (OP_RETURN) + push opcode + data bytes. For ≤75 bytes, the push opcode is the data length as a single byte (e.g., `24` = 36 bytes, `20` = 32 bytes).

**Basket rule:** Use `worm-runtime-proofs` for user-requested proofs. Never use `worm-proofs` — that basket is reserved for the system's internal audit trail.

### Check Transaction History

```json
wallet_call({"endpoint": "listActions", "params": {"labels": [], "includeOutputs": true, "limit": 10}})
```

### List UTXOs in a Basket

```json
wallet_call({"endpoint": "listOutputs", "params": {"basket": "default", "limit": 25}})
```

## When to Use wallet_call

Use it for endpoints that do NOT have a dedicated tool: discovery, certificates, HMAC, key linkage, deferred transaction signing, and status checks.

## When NOT to Use wallet_call

These dedicated tools exist and have better error handling:

| Task | Use this tool instead |
|------|-----------------------|
| Check balance | `wallet_balance` |
| Get identity/derive key | `wallet_identity` |
| Encrypt/decrypt | `wallet_encrypt` / `wallet_decrypt` |
| Check certificates | `check_certificates` |
| Receive BSV | `receive_address` (discoverable) |
| Internalize payment | `fund_from_tx` (discoverable) |
| Make x402 payment | `x402_call` |

## Endpoint Catalog

### Discovery (FREE / READ-ONLY)

The key use case. Find other agents and their capabilities.

| Endpoint | Purpose | Example params |
|----------|---------|----------------|
| `discoverByIdentityKey` | Find certificates for a known identity | `{"identityKey": "03abc...", "limit": 10}` |
| `discoverByAttributes` | Find identities by certificate attributes | `{"attributes": {"name": "value"}, "limit": 10}` |

Both accept an optional `certificateType` field to filter by type.

**Workflows:**
- "Who is this agent?" -> `discoverByIdentityKey` with their pubkey
- "Find agents with capability X" -> `discoverByAttributes` with `{"capabilities": "X"}`

### Certificate Management (WRITE)

| Endpoint | Purpose | Example params |
|----------|---------|----------------|
| `listCertificates` | List certs by certifier/type | `{"certifiers": ["03..."], "types": ["type-id"], "limit": 10, "offset": 0}` |
| `proveCertificate` | Selective field reveal | `{"certificate": {...}, "fieldsToReveal": ["name", "capabilities"]}` |
| `acquireCertificate` | Acquire a new certificate | `{"type": "...", "certifier": "03...", "subject": "03...", "fields": {...}, "serialNumber": "...", "revocationOutpoint": "...", "signature": "..."}` |
| `relinquishCertificate` | Release a certificate | `{"certificate": {"type": "...", "certifier": "03...", "serialNumber": "..."}}` |

**Workflow — Prove identity to another agent:**
```json
wallet_call({"endpoint": "proveCertificate", "params": {
  "certificate": {"type": "...", "certifier": "03...", "serialNumber": "..."},
  "fieldsToReveal": ["name", "capabilities"]
}})
```

### Cryptographic Operations (FREE)

| Endpoint | Purpose | Example params |
|----------|---------|----------------|
| `createHmac` | Create HMAC with BRC-42 key | `{"data": [72,101,108,108,111], "protocolID": [2, "myproto"], "keyID": "1", "counterparty": "self"}` |
| `verifyHmac` | Verify HMAC | `{"data": [72,101,108,108,111], "hmac": [1,2,3,...], "protocolID": [2, "myproto"], "keyID": "1", "counterparty": "self"}` |

Note: `data` and `hmac` are byte arrays (not base64). Use numeric arrays like `[72, 101, 108]`.

### Transaction Management (WRITE)

| Endpoint | Purpose | Example params |
|----------|---------|----------------|
| `signAction` | Sign a deferred transaction | `{"reference": "ref-string"}` |
| `abortAction` | Cancel a deferred transaction | `{"reference": "ref-string"}` |
| `internalizeAction` | Accept incoming payment/token | `{"tx": [1,2,3,...], "outputs": [...], "description": "..."}` |

### Key Linkage (ADVANCED)

| Endpoint | Purpose | Example params |
|----------|---------|----------------|
| `revealCounterpartyKeyLinkage` | Prove key relationship to a verifier | `{"counterparty": "03...", "verifier": "03...", "privileged": false}` |
| `revealSpecificKeyLinkage` | Prove specific protocol key to a verifier | `{"counterparty": "03...", "verifier": "03...", "protocolID": [2, "protocol"], "keyID": "1", "privileged": false}` |

### Status (FREE / GET — no params)

| Endpoint | Purpose |
|----------|---------|
| `isAuthenticated` | Check if wallet is authenticated |
| `getHeight` | Current block height |
| `getNetwork` | Network name (mainnet/testnet) |
| `getVersion` | Wallet version |
| `waitForAuthentication` | Block until authenticated (use sparingly) |

For status endpoints, pass NO params — the tool auto-detects GET:
```json
wallet_call({"endpoint": "isAuthenticated"})
```

## Safety Tiers

| Tier | Endpoints | Risk |
|------|-----------|------|
| FREE / READ-ONLY | discovery, list, status, HMAC, verify | Safe to call anytime |
| WRITE | acquireCertificate, signAction, internalizeAction | Modifies wallet state |
| DESTRUCTIVE | relinquishCertificate, relinquishOutput, abortAction | Irreversible |

**Never call DESTRUCTIVE endpoints without explicit user confirmation.**

## Important Notes

- Params are always a JSON object (or omitted for GET endpoints)
- The tool auto-routes: no params = GET, with params = POST
- Returns raw wallet JSON — parse the response yourself
- Errors come as strings starting with `"Error:"`
- Use dedicated wallet tools when available — they have better error handling and formatting
- All pubkeys are 66-char compressed hex (e.g. `"03abc..."`)
- Protocol IDs are arrays like `[2, "protocol-name"]` (security level + key)
- `counterparty` is a pubkey, `"self"`, or `"anyone"`

## Gotchas

- **Always use `wallet_call` for non-core endpoints.** Never construct raw HTTP calls to `localhost:3322` — `wallet_call` handles auth, error formatting, and GET/POST routing.
- **Endpoint names must be exact BRC-100 names.** Use `createAction` not `create_action`, `listOutputs` not `list_outputs`. The wallet API uses camelCase.
- **`createAction` returns a transaction — always check for errors.** A missing output or bad script will return an error string starting with `"Error:"`, not throw an exception.
- **OP_RETURN outputs MUST have 0 satoshis.** ARC miners reject OP_RETURN with non-zero value ("output has non 0 value op return"). Always use `"satoshis": 0`.
- **OP_RETURN scripts must start with `006a` (OP_FALSE OP_RETURN).** Some miners reject scripts starting with just `6a` (OP_RETURN without OP_FALSE prefix).
