# src/audit/
> BRC-69 key linkage revelations with deterministic hashing and append-only audit logging.

## Overview

The audit module provides structured key linkage revelations for proving cryptographic key derivation relationships to verifiers. It wraps wallet key linkage data with audit metadata (requester, timestamp, deterministic SHA-256 hash) and persists revelations as individual JSON files in an append-only directory. The server handler layer (`server/handlers/audit.rs`) consumes this module for the `/audit/key-linkage/*` and `/audit/revelations` HTTP endpoints.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 3 | Module declaration, re-exports `revelation` submodule |
| `revelation.rs` | 196 | `Revelation` struct, `RevelationLog` append-only persistence, `compute_revelation_hash()`, `RevelationsListResponse` |

## Key Exports

### `Revelation` (struct)

Structured key linkage revelation with audit metadata. Fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | UUID v4 unique identifier |
| `revelation_type` | `String` | `"counterparty"` or `"specific"` |
| `counterparty` | `String` | The counterparty key involved |
| `protocol_id` | `Option<String>` | Protocol ID (e.g., `[2, "worm message signature"]`), specific revelations only |
| `key_id` | `Option<String>` | Key ID, specific revelations only |
| `requested_by` | `String` | The verifier who requested the revelation |
| `timestamp` | `String` | RFC 3339 timestamp of when the revelation was made |
| `linkage_data` | `serde_json::Value` | Raw key linkage data returned by the wallet |
| `revelation_hash` | `String` | Deterministic SHA-256 hex hash for tamper detection |

Two constructors:

- `Revelation::counterparty(counterparty, requested_by, linkage_data)` -- BRC-69 counterparty revelation. Proves keys for a given counterparty derive from the same master key without revealing keys for other counterparties.
- `Revelation::specific(counterparty, protocol_id, key_id, requested_by, linkage_data)` -- BRC-70 specific revelation. Proves derivation for a specific protocol+keyID pair without revealing linkage for other pairs.

### `compute_revelation_hash()` (function)

Deterministic SHA-256 hash over `type|counterparty|protocol_id|key_id|requested_by|timestamp|linkage_data`. Pipe-delimited, empty string for absent optional fields. Enables verifiers to confirm a revelation hasn't been tampered with.

### `RevelationLog` (struct)

Append-only audit log backed by a directory of JSON files.

| Method | Description |
|--------|-------------|
| `new(workspace)` | Creates log at `{workspace}/audit_revelations/`, auto-creates directory |
| `from_dir(log_dir)` | Creates log from an explicit directory path |
| `record(revelation)` | Writes `{revelation.id}.json` as pretty-printed JSON |
| `list()` | Reads all `.json` files, deserializes, returns sorted by timestamp ascending. Silently skips unparseable files. |

### `RevelationsListResponse` (struct)

Response DTO for `GET /audit/revelations`. Fields: `revelations: Vec<Revelation>`, `count: usize`.

## Usage

The audit module is consumed by `server/handlers/audit.rs` which exposes 5 HTTP endpoints:

| Route | Auth | Handler | Uses |
|-------|------|---------|------|
| `POST /audit/key-linkage/counterparty` | BRC-31 | `reveal_counterparty_linkage` | `Revelation::counterparty()` + `RevelationLog::record()` |
| `POST /audit/key-linkage/specific` | BRC-31 | `reveal_specific_linkage` | `Revelation::specific()` + `RevelationLog::record()` |
| `GET /audit/revelations` | optional | `list_revelations` | `RevelationLog::list()` |
| `GET /audit/cross-reference/{hash}` | optional | `cross_reference_message` | Transcript scanning (not in this module) |
| `GET /audit/search` | none | `audit_search` | Transcript scanning (not in this module) |

The handler flow for key linkage endpoints:
1. Verify BRC-31 auth
2. Call wallet `reveal_counterparty_key_linkage()` or `reveal_specific_key_linkage()`
3. Wrap result in a `Revelation` struct (auto-generates ID, timestamp, hash)
4. Persist via `RevelationLog::record()`
5. Log to budget JSONL for audit trail (service `"audit"`, operation `"counterparty_key_linkage"` or `"specific_key_linkage"`)
6. Return `KeyLinkageResponse` with `revelation_id` and `revelation_hash`

## Storage

Revelations are persisted as individual JSON files:

```
workspace/
  audit_revelations/
    {uuid}.json     # One file per revelation, pretty-printed JSON
```

Files are never modified or deleted -- the log is append-only. The `list()` method reads all files at query time and sorts by timestamp.

## Related

- [`../server/handlers/CLAUDE.md`](../server/handlers/CLAUDE.md) -- HTTP handlers that consume this module (audit.rs)
- [`../server/CLAUDE.md`](../server/CLAUDE.md) -- Server route table including all 5 `/audit/*` endpoints
- [`../wallet.rs`](../wallet.rs) -- `WalletClient` provides `reveal_counterparty_key_linkage()` and `reveal_specific_key_linkage()` via BRC-42 key derivation
- [`../onchain/CLAUDE.md`](../onchain/CLAUDE.md) -- BRC-18 proofs and BRC-48 state tokens (separate audit mechanism for on-chain proofs)
- [`../CLAUDE.md`](../CLAUDE.md) -- Parent module inventory
