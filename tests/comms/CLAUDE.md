# comms

> 78 tests across 2 files covering BRC-33 MessageBox messaging, BRC-56 peer discovery, BRC-77/BRC-42 cross-wallet signature verification, and cross-agent proof chain linking.

## Overview

Tests for the agent's cross-agent communication layer: message type serialization, paid delivery via x402, BRC-77 message signing, BRC-78 message encryption, sign-then-encrypt layering, BRC-42 cross-wallet signature verification (three counterparty modes), BRC-56 peer discovery structs and tool validation, SHA-256 message hashing for proof enrichment, and the `/audit/cross-reference/{hash}` endpoint.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_discovery.rs | 42 | BRC-56 peer discovery: `PeerInfo`/`PeerVerification` serde, `discover_agent`/`verify_agent` tool parameter validation, `compute_message_hash` determinism, message hash extraction from tool output, proof enrichment with `message_hashes`, cross-reference endpoint (axum oneshot), transcript scanning |
| test_messagebox.rs | 36 | BRC-33 MessageBox: `TaskAssignment`/`StatusUpdate`/`TaskResult`/`CoordinationSignal` serde roundtrips, `DeliveryQuote` cost logic, 402 header parsing for paid delivery, body-transport payment structure, `SignedMessage`/`EncryptedMessage` roundtrips, `ProcessedMessage` auto-processing, detection patterns, sign-then-encrypt layering, BRC-42 cross-wallet signature verification (broadcast/directed/self modes) |

## Key types tested

### From `bsv_worm::messagebox::types`

| Type | Purpose |
|------|---------|
| `TaskAssignment` | Parent-to-child task delegation (includes `task_id`, `budget_sats`, `response_box`) |
| `StatusUpdate` | Child-to-parent progress reports (`progress_pct`, `sats_spent`, `current_step`) |
| `TaskResult` | Completed/failed task results with `::completed()` and `::failed()` constructors |
| `CoordinationSignal` | Heartbeat broadcasts (`capabilities`, `available_budget_sats`, `current_load`) |
| `DeliveryQuote` | Fee breakdown with `total_cost()`, `requires_payment()`, `is_blocked()` methods |
| `ReceivedMessage` | Incoming message with `message_id`, `body` (string or object), `sender`, `createdAt` |
| `SignedMessage` | BRC-77 wrapper: `body` + `signature` + `sender_key` |
| `EncryptedMessage` | BRC-78 wrapper: base64 `ciphertext` + `sender_key` + `encrypted: true` |
| `ProcessedMessage` | Post-processing result: original `body` + `was_encrypted`/`was_signed`/`signature_valid` flags |

### From `bsv_worm::discovery`

| Type | Purpose |
|------|---------|
| `PeerInfo` | Discovered agent: `identity_key`, optional `name`/`capabilities`/`certificate_type`/`certifier`/`fields`/`raw` |
| `PeerVerification` | Certificate verification result: `has_certificates`, `certificate_count`, `has_parent_signed`, `certificate_types`, `certifiers` |
| `PeerDiscovery` | Client constructed from `WalletClient`, provides `discover_by_identity_key` and `discover_by_attributes` |

### From `bsv_worm::messagebox`

| Function | Purpose |
|----------|---------|
| `compute_message_hash` | SHA-256 of `serde_json::to_string(body)` — deterministic 64-char hex hash |

### From `bsv::wallet` (BSV SDK — cross-wallet verification tests)

| Type | Purpose |
|------|---------|
| `ProtoWallet` | Lightweight wallet for signing/verification. `::new(Some(PrivateKey))` for identity, `::anyone()` for broadcast verification |
| `Counterparty` | BRC-42 key derivation mode: `Self_` (signer-only), `Anyone` (broadcast, ECDH with G), `Other(pubkey)` (directed, ECDH with recipient) |
| `Protocol` | Protocol ID for key derivation: `Protocol::new(SecurityLevel::Counterparty, "worm message signature")` |
| `CreateSignatureArgs` | Sign `data` with `protocol_id`, `key_id`, and `counterparty` mode |
| `VerifySignatureArgs` | Verify `signature` against `data` with `protocol_id`, `key_id`, and `counterparty` (sender's pubkey for verification) |

## Constants tested

| Constant | Value | Purpose |
|----------|-------|---------|
| `MESSAGEBOX_URL` | `https://messagebox.babbage.systems` | Production relay endpoint |
| `MESSAGEBOX_IDENTITY_KEY` | 66-char compressed pubkey (starts `02`) | Fallback identity key for 402 payment |
| `BOX_TASK_INBOX` | `task_inbox` | Parent-to-child task delivery |
| `BOX_STATUS_INBOX` | `status_inbox` | Child-to-parent status updates |
| `BOX_RESULTS_INBOX` | `results_inbox` | Child-to-parent completed results |
| `BOX_WORM_COORDINATION` | `worm_coordination` | Fleet-wide heartbeat/coordination |
| `INBOX_BOXES` | `[task, results, status, coordination]` | Priority-ordered inbox polling sequence |

## Test patterns

### Serde roundtrips
All message types are tested with serialize → deserialize → assert field equality. Covers required fields, optional fields with `skip_serializing_if`, and `msg_type`/`version` defaults.

### Paid delivery (x402 body-transport)
Tests verify the `{ message: {...}, payment: {...} }` merge structure that MessageBox expects when delivery requires payment. The 402 header parsing (`parse_402_response`) is tested with `x-bsv-payment-*` headers and validated against `DeliveryQuote.total_cost()`.

### BRC-77 signing / BRC-78 encryption
Sign-then-encrypt layering tested end-to-end: inner body → `SignedMessage` → serialize → base64-encode as `EncryptedMessage.ciphertext` → decrypt → deserialize → verify signature on inner body. Detection patterns tested: `encrypted` + `ciphertext` fields detect encryption, `signature` + `sender_key` detect signing.

### BRC-42 cross-wallet signature verification (three counterparty modes)
Three tests exercise the BSV SDK's `ProtoWallet` directly to verify the ECDH math behind BRC-77 message signatures:

| Mode | Counterparty | ECDH Shared Secret | Who Can Verify |
|------|-------------|-------------------|----------------|
| Broadcast | `Counterparty::Anyone` (G, scalar=1) | `alice_priv * G = alice_pub` | `ProtoWallet::anyone()` with sender's pubkey as counterparty |
| Directed | `Counterparty::Other(bob_pub)` | `alice_priv * bob_pub` | Bob only (ECDH symmetry: `bob_priv * alice_pub`) |
| Self-only | `Counterparty::Self_` | `alice_priv * alice_pub` | Only Alice (no other wallet can reproduce) |

Each test creates wallets with `ProtoWallet::new(Some(PrivateKey::random()))`, signs data, then asserts correct verify/reject behavior across wallets. These validate that the `counterparty="anyone"` fix (commit c27e5c3) works correctly — broadcast sigs are verifiable by `ProtoWallet::anyone()` but NOT by a different wallet's own key derivation.

### Discovery tool validation
`discover_agent` and `verify_agent` tools loaded via `all_discovery_tools()` and invoked directly. Tests assert error messages for missing params, short identity keys (must be 66-char hex), and empty attribute maps. Both tools are in the `"discovery"` category and are **not** in `ALWAYS_ON_TOOLS`.

### Message hash pipeline (cross-agent proof chain linking)
End-to-end pipeline tested: `compute_message_hash(body)` → extract from `send_message` tool output → embed in decision proof text as `message_hashes=[hash1,hash2]` → embed in reasoning section as `MESSAGE_HASHES:` → searchable via `/audit/cross-reference/{hash}` endpoint. Hash format validated: exactly 64 hex chars.

### Cross-reference endpoint (axum oneshot)
`/audit/cross-reference/{hash}` tested via `tower::ServiceExt::oneshot()`:
- Valid 64-char hex hash returns 200 with `{ message_hash, matches: [], count: 0 }`
- Invalid hash (too short) returns error JSON
- Response structure matches `CrossReferenceMatch`/`CrossReferenceResponse` from `audit.rs`

### Transcript scanning
`Transcript::new()` + `record("proof_created", data)` + `replay()` — verifies message hashes survive JSONL roundtrip in proof event data field.

## Gotchas

- **`std::mem::forget(workspace)`** in cross-reference endpoint tests — the `TempDir` is intentionally leaked because the Axum router holds the path via `Arc`. Same pattern as `test_server.rs`.
- **`ReceivedMessage.body` can be string or object** — `test_received_message_body_is_string` tests the string case; `list_messages()` in production handles unwrapping.
- **`DeliveryQuote.recipient_fee == -1` means blocked** — the `is_blocked()` method checks for negative fee, not just zero.
- **`compute_message_hash` is order-sensitive** — it hashes `serde_json::to_string()` output, which preserves insertion order. Same `Value` always produces the same hash, but construction order matters.
- **Discovery tools require 66-char hex keys** — tests assert the `"66-char"` error string for short keys. This matches secp256k1 compressed public key format.
- **`MESSAGEBOX_IDENTITY_KEY` must be valid hex** — `test_messagebox_identity_key_is_valid_pubkey` decodes it with `hex::decode()` to confirm.
- **Cross-wallet tests use random keys** — `ProtoWallet::new(Some(PrivateKey::random()))` generates fresh keypairs per test. These tests exercise real ECDSA signing and BRC-42 key derivation, not mocks.
- **`ProtoWallet::anyone()` is the broadcast verifier** — it uses scalar=1 as its private key, so `ECDH = 1 * alice_pub = alice_pub`, matching Alice's signing ECDH `= alice_priv * G`. A different wallet (Bob) with `bob_priv * alice_pub` gets a different shared secret and correctly fails verification.

## Related

- [tests/CLAUDE.md](../CLAUDE.md) — Parent test directory overview, test runner commands, conventions
- [tests/security/CLAUDE.md](../security/CLAUDE.md) — BRC-31 auth, cryptographic trust (related message signing/verification)
- [tests/x402/CLAUDE.md](../x402/CLAUDE.md) — x402 payment flow (paid delivery uses `parse_402_response`)
- [tests/core/CLAUDE.md](../core/CLAUDE.md) — Self-message loop prevention (`test_self_message.rs`)
- [src/messagebox/](../../src/messagebox/) — MessageBox client implementation
- [src/discovery.rs](../../src/discovery.rs) — BRC-56 peer discovery implementation
- [src/tools/discovery_tools.rs](../../src/tools/discovery_tools.rs) — `discover_agent` and `verify_agent` tool definitions
- [src/server/handlers/](../../src/server/handlers/) — `/audit/cross-reference/{hash}` endpoint handler
