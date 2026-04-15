# src/wallet/
> BRC-100 wallet interface — trait abstraction with HTTP and embedded backends.

## Overview

This module is the **only boundary** between bsv-worm and the BSV wallet. Every wallet operation in the codebase goes through the `WalletBackend` trait defined here. Two implementations exist: `HttpWalletClient` (talks to `bsv-wallet-cli` over HTTP at localhost:3322) and `EmbeddedWalletClient` (calls `bsv-wallet-toolbox-rs` in-process, feature-gated behind `embedded-wallet`). The trait covers 35 async methods spanning key management, transaction creation, cryptographic operations, certificates, discovery, key linkage, and chain status.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 278 | `WalletBackend` trait (35 methods), re-exports, backward-compat `WalletClient` type alias |
| `http.rs` | 1420 | `HttpWalletClient` — reqwest-based HTTP client for `bsv-wallet-cli`. All 35 trait methods as inherent methods + trait delegation. Retry logic, SPEND_SEMAPHORE, balance pagination, WOC funding, BEEF parsing helpers |
| `embedded.rs` | 1621 | `EmbeddedWalletClient` — in-process `bsv-wallet-toolbox-rs` wrapper. Feature-gated (`embedded-wallet`). Same 35 methods via direct `WalletInterface` calls. Spending lock, AtomicBEEF construction, type adapters |
| `types.rs` | 14 | `CreateActionResult` struct, `ANYONE_KEY` constant (secp256k1 generator point G) |

## Key Exports

| Export | Description |
|--------|-------------|
| `WalletBackend` | Async trait — 35 methods covering the full BRC-100 interface |
| `HttpWalletClient` | Default implementation — HTTP client for `bsv-wallet-cli` at configurable URL |
| `EmbeddedWalletClient` | Feature-gated (`embedded-wallet`) — in-process wallet via toolbox library |
| `WalletClient` | Type alias for `HttpWalletClient` (backward compatibility) |
| `CreateActionResult` | Return type for `create_action()`/`spend_output()` — contains `txid`, `tx` bytes, `raw` JSON |
| `ANYONE_KEY` | `"0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"` — secp256k1 generator point, used for `fund_from_woc` internalization |

## WalletBackend trait — method categories

### Key Management
- `get_identity_key()` — wallet's 66-char hex compressed public key
- `get_public_key()` — BRC-42 key derivation with protocol_id, key_id, counterparty

### Transaction Creation
- `create_action()` — create a transaction with outputs (serialized via spend semaphore/lock)
- `spend_output()` — spend a specific UTXO via createAction with inputs (73-byte unlocking script)
- `internalize_action()` — internalize an incoming AtomicBEEF transaction

### Output Management
- `get_balance()` — total spendable satoshis (paginated, 100 per page)
- `get_balance_and_count()` — balance + spendable output count
- `list_outputs()` — list UTXOs by basket with pagination
- `relinquish_output()` — release a tracked output

### Cryptographic Operations
- `create_signature()` / `verify_signature()` — BRC-42 + ECDSA signing
- `encrypt()` / `decrypt()` — BRC-42 derived key encryption
- `create_hmac()` / `verify_hmac()` — BRC-42 derived key HMAC

### Certificate Operations
- `acquire_certificate()` / `list_certificates()` / `prove_certificate()` / `relinquish_certificate()`

### Discovery
- `discover_by_identity_key()` / `discover_by_attributes()` — BRC-56 peer discovery

### Key Linkage
- `reveal_counterparty_key_linkage()` / `reveal_specific_key_linkage()` — BRC-69/70 audit revelations

### Status / Metadata
- `is_authenticated()` / `get_height()` / `get_network()` / `get_version()` / `wait_for_authentication()`

### Chain / Action Management
- `get_header_for_height()` — block header by height
- `sign_action()` / `abort_action()` — deferred transaction signing
- `list_actions()` — transaction history with label filtering

### Funding
- `receive_address()` — derive a P2PKH address the wallet can later spend
- `fund_from_woc()` — fetch BEEF from WhatsOnChain, verify script, build AtomicBEEF, internalize
- `raw_call()` — generic endpoint bridge for the `wallet_call` tool

## Architecture

```
                     ┌──────────────────────────┐
                     │     WalletBackend trait   │  ← mod.rs (35 async methods)
                     └────────────┬─────────────┘
                          ┌───────┴───────┐
                          │               │
               ┌──────────▼──┐    ┌───────▼──────────┐
               │ HttpWallet  │    │ EmbeddedWallet   │
               │  Client     │    │  Client          │
               │ (http.rs)   │    │ (embedded.rs)    │
               └──────┬──────┘    └───────┬──────────┘
                      │                   │
                 reqwest HTTP        WalletInterface
                      │              (in-process)
                      ▼                   ▼
              bsv-wallet-cli        bsv-wallet-toolbox-rs
              (:3322 HTTP)               → SQLite
```

### bsv-rs dependency

Both backends use a single `bsv-rs` dep from crates.io (aliased as `bsv`) pinned
to the version that `bsv-wallet-toolbox-rs` links against, so `WalletInterface`
argument types match without adapter layers.

### Concurrency control

- **HttpWalletClient**: Global `SPEND_SEMAPHORE` (LazyLock, permits=1) serializes `create_action()` and `spend_output()` across all client instances to prevent UTXO contention
- **EmbeddedWalletClient**: Per-instance `spending_lock` (tokio Mutex) serializes the same operations

### Retry logic (HTTP only)

`call_with_retry()` retries transient errors (SQLite BUSY, HTTP 500/503) with delays of 100ms, 250ms, 500ms. Used by state-modifying operations: `createAction`, `relinquishOutput`.

### Funding flow (`fund_from_woc` / `receive_address`)

Both backends share the same 5-step funding flow:
1. Derive expected payment key via BRC-42 (`protocol: [2, "3241645161d8"]`, `key_id: "worm-fund {suffix}"`, `counterparty: ANYONE_KEY`)
2. Fetch tx from WhatsOnChain, verify output script matches derived P2PKH
3. Fetch BEEF from WhatsOnChain
4. Build AtomicBEEF: `[0x01, 0x01, 0x01, 0x01]` + reversed txid (32 bytes) + BEEF bytes
5. Internalize with `"wallet payment"` protocol and derivation parameters

## Inherent methods vs trait delegation

Both implementations define all methods as **inherent `pub async fn`** methods first, then implement `WalletBackend` by delegating to them. This means callers do NOT need to import the `WalletBackend` trait to use the wallet — they can call methods directly on the concrete type. The trait is only needed when code must be generic over both backends (e.g., `Arc<dyn WalletBackend>`).

## Helper functions

### http.rs
- `parse_byte_array(value, field)` — extract a JSON byte array (`[u8]`) from wallet responses
- `parse_woc_beef(raw)` — parse WOC BEEF response (may be quoted hex string or raw bytes)

### embedded.rs
- `parse_protocol_id(value)` — convert `[level, "name"]` JSON to SDK `Protocol` struct
- `parse_counterparty(s)` — convert `"self"` / `"anyone"` / hex pubkey to SDK `Counterparty` enum
- `parse_output(value)` — convert output JSON to SDK `CreateActionOutput`
- `convert_create_action_result(result)` — convert SDK result to `CreateActionResult` with AtomicBEEF construction (falls back to raw tx on BEEF parse failure)
- `parse_woc_beef(raw)` — same logic as http.rs version

## Gotchas

- **Wallet HTTP 200 can still be an error**: `http.rs` checks for an `error` field in the JSON body even on 200 responses. Empty string `error` is treated as success.
- **Balance pagination loops**: `get_balance()` / `get_balance_and_count()` loop with limit=100 offsets to sum all spendable outputs. Not a single-call operation.
- **Empty body handling**: Some wallet endpoints return empty body on success (e.g., `relinquishCertificate`). `call()` converts empty text to `{}`.
- **AtomicBEEF wrapping is manual**: Both backends prepend `[0x01, 0x01, 0x01, 0x01]` + reversed txid before BEEF bytes. This is the AtomicBEEF format expected by `internalizeAction`.
- **`raw_call` on embedded is limited**: Only supports `getPublicKey`, `isAuthenticated`, `getHeight`, `getNetwork`, `getVersion`. Other methods return an error.
- **Certificate field name inconsistency**: `relinquish_certificate()` handles both `"type"` and `"certificateType"` field names in the input JSON.
- **HMAC verification requires exactly 32 bytes**: `EmbeddedWalletClient::verify_hmac()` enforces `hmac.len() == 32` via `try_into()`.
- **Embedded BEEF fallback**: `convert_create_action_result()` attempts AtomicBEEF from `result.beef` + txid. On parse failure, falls back to `result.tx` (raw tx bytes) with a warning log.

## Related

- [../CLAUDE.md](../CLAUDE.md) — Root `src/` documentation with module inventory
- [../tools/CLAUDE.md](../tools/CLAUDE.md) — `wallet_tools.rs` defines 5 wallet tools (balance, identity, encrypt, decrypt, wallet_call) that use this module
- [../auth/CLAUDE.md](../auth/CLAUDE.md) — BRC-31 auth client uses wallet for signing
- [../x402/CLAUDE.md](../x402/CLAUDE.md) — Payment flow uses wallet for key derivation and internalization
- [../onchain/CLAUDE.md](../onchain/CLAUDE.md) — Budget tracker, proofs, and state tokens all go through wallet for on-chain operations
- [../certificates/CLAUDE.md](../certificates/CLAUDE.md) — Certificate lifecycle uses wallet certificate endpoints
- [../discovery.rs](../discovery.rs) — BRC-56 peer discovery wraps wallet discovery endpoints
- [../../CLAUDE.md](../../CLAUDE.md) — Root project docs
