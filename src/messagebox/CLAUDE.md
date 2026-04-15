# messagebox
> BRC-33 MessageBox client for peer-to-peer agent messaging via Babbage relay server. Supports BRC-77 message signing and BRC-78 message encryption.

## Files

### `mod.rs` (11 lines)
Module root. Re-exports `MessageBoxClient` and `compute_message_hash` from `client`, and all types from `types`.

### `types.rs` (481 lines)
Message body schemas, protocol constants, and cryptographic message wrappers.

**Constants:**
- `MESSAGEBOX_URL` — `https://messagebox.babbage.systems`
- `MESSAGEBOX_IDENTITY_KEY` — Server's secp256k1 identity pubkey (66 hex chars)
- `BOX_TASK_INBOX`, `BOX_STATUS_INBOX`, `BOX_RESULTS_INBOX`, `BOX_WORM_COORDINATION` — Named box identifiers
- `INBOX_BOXES` — All four boxes in poll priority order (task first, coordination last)
- `TASK_INBOX_FEE` — 100 sats, intended spam filter fee (not yet enforced)

**Typed message structs** (all `Serialize + Deserialize`, use `#[serde(rename = "type")]` for `msg_type`):

| Struct | Box | Purpose | Constructor |
|--------|-----|---------|-------------|
| `TaskAssignment` | `task_inbox` | Assign work to a worm | `new(task_id, task, budget_sats, requester_key)` |
| `StatusUpdate` | `status_inbox` | Progress reports back to requester | `new(task_id, status)` |
| `TaskResult` | `results_inbox` | Final result or failure | `completed(task_id, result)`, `failed(task_id, reason, detail)` |
| `CoordinationSignal` | `worm_coordination` | Peer discovery, heartbeats | `heartbeat(sender_key)` |

**Other types:**
- `ReceivedMessage` — Server response: `message_id`, `body` (Value), `sender` (pubkey), `created_at`, optional `message_box` (set by client after retrieval)
- `DeliveryQuote` — Quote response: `delivery_fee`, `recipient_fee`. Methods: `total_cost()`, `requires_payment()`, `is_blocked()` (fee == -1 means blocked)

**Cryptographic message wrappers:**
- `SignedMessage` — BRC-77 signed message: `body` (original Value), `signature` (hex ECDSA), `sender_key` (66-char pubkey). Recipient uses `sender_key` to verify.
- `EncryptedMessage` — BRC-78 encrypted message: `ciphertext` (base64), `sender_key` (needed for ECDH decryption), `encrypted` (always `true`, marker for detection).
- `ProcessedMessage` — Result of receive-side processing: `body` (unwrapped), `was_encrypted`, `was_signed`, `signature_valid` (Option), `sender`.

### `client.rs` (969 lines)
`MessageBoxClient` — wraps `AuthriteClient` for BRC-31 authenticated messaging with BRC-77 signing, BRC-78 encryption, and BRC-29 paid delivery.

**Constants:**
- `signing_protocol_id()` — `[2, "worm message signature"]` (BRC-77)
- `encryption_protocol_id()` — `[2, "worm message encryption"]` (BRC-78)
- `MESSAGE_KEY_ID` — `"message"` (static key ID for both signing and encryption)
- `MAX_PAYMENT_ATTEMPTS` — `3` (retry limit for 402 payment loop)

**Public function:**
- `compute_message_hash(body: &Value) -> String` — SHA-256 hash of serialized message body for cross-agent proof chain linking. Both sender and receiver compute the same hash from the same body, enabling cross-reference in their respective proof chains.

**Construction:**
- `new(auth)` — Uses default `MESSAGEBOX_URL`
- `with_url(auth, url)` — Custom server URL (strips trailing slash)

**Core methods:**

| Method | Endpoint | Description |
|--------|----------|-------------|
| `quote(recipient, message_box)` | `GET /permissions/quote` | Get delivery cost; parses nested `quote` object or top-level fields |
| `send_message(recipient, message_box, body)` | `POST /sendMessage` | Quote + send; handles 402 with body-transport payment loop (up to 3 retries). Returns `message_hash` for proof chain linking. |
| `list_messages(message_box)` | `POST /listMessages` | Returns unwrapped messages; sets `message_box` field on each |
| `acknowledge_message(message_ids)` | `POST /acknowledgeMessage` | Permanently deletes messages from server |
| `set_permission(message_box, recipient_fee, sender)` | `POST /permissions/set` | Set fee or block (`-1`) for a box; `sender` is optional (global if omitted) |
| `poll_inboxes()` | (calls `list_messages` x4) | Polls all `INBOX_BOXES` in priority order; `warn!` on per-box errors, continues |

**BRC-77 signing methods:**

| Method | Description |
|--------|-------------|
| `sign_message_body(body)` | **Broadcast mode**: sign body with ECDSA via BRC-42 key derivation (counterparty `"anyone"`). Any party can verify using sender's pubkey. Returns `SignedMessage`. |
| `sign_message_body_directed(body, recipient_key)` | **Directed mode**: sign body with recipient's pubkey as BRC-42 counterparty. Only the specific recipient can verify (via ECDH symmetry). Returns `SignedMessage`. |
| `verify_message_signature(signed, sender_key)` | Two-phase verification: (1) tries broadcast mode locally via `ProtoWallet::anyone()` — fast, no wallet call; (2) falls back to directed mode via wallet for recipient-specific signatures. Returns `bool`. |

**Private signing helper:**
- `sign_with_counterparty(body, counterparty)` — Internal method used by both `sign_message_body` and `sign_message_body_directed`. Serializes body deterministically, calls `wallet.create_signature()` with the given counterparty, returns `SignedMessage` with sender's identity pubkey.

**BRC-78 encryption methods:**

| Method | Description |
|--------|-------------|
| `encrypt_message_body(body, recipient_key)` | Encrypt via BRC-42 ECDH (counterparty = recipient). Returns `EncryptedMessage` with base64 ciphertext. |
| `decrypt_message_body(encrypted)` | Decrypt via BRC-42 ECDH (counterparty = sender from `encrypted.sender_key`). Returns JSON `Value`. |

**Convenience send methods:**

| Method | Security | Description |
|--------|----------|-------------|
| `send_signed_message(recipient, box, body)` | BRC-77 | Sign body → wrap as `SignedMessage` → send |
| `send_encrypted_message(recipient, box, body)` | BRC-78 | Encrypt body for recipient → wrap as `EncryptedMessage` → send |
| `send_signed_encrypted_message(recipient, box, body)` | BRC-77+78 | Sign → encrypt signed message → send. Recipient decrypts first, then verifies. |

**Receive-side processing:**
- `process_received_message(msg)` — Auto-detects and processes: (1) if `encrypted: true` + `ciphertext` field → decrypt, (2) if `signature` + `sender_key` fields → verify. Returns `ProcessedMessage`.

**Private helper:**
- `unwrap_message_body(body)` — Handles server wrapping: parses JSON strings, strips `{"message": ...}` envelope

## Send flow

```
send_message(recipient, box, body)
  │
  ├── 0. compute_message_hash(body) → SHA-256 for proof chain linking
  │
  ├── 1. quote(recipient, box)  →  GET /permissions/quote
  │     └── blocked? → Err
  │
  ├── 2. Generate UUID message_id
  │
  ├── 3. Build request: { message: { recipient, messageBox, messageId, body } }
  │
  ├── 4. POST /sendMessage (BRC-31 auth)
  │     ├── success → append sentMessageId + message_hash, return
  │     └── 402 → enter payment loop ↓
  │
  └── 5. Payment loop (up to MAX_PAYMENT_ATTEMPTS=3)
        ├── parse_402_response(headers) → payment requirements
        ├── create_payment(wallet, prefix, server_key, sats, url) → (payment_val, txid)
        ├── merge payment into request body (body-transport)
        ├── POST /sendMessage with payment
        │   ├── success → append sentMessageId + payment_txid + sats_paid + message_hash, return
        │   └── 402 again → retry
        └── exhausted → Err
```

## Signing + encryption composition

```
send_signed_encrypted_message(recipient, box, body)
  │
  ├── 1. sign_message_body(body)       → SignedMessage { body, signature, sender_key }
  ├── 2. serialize SignedMessage to Value
  ├── 3. encrypt_message_body(value, recipient_key) → EncryptedMessage { ciphertext, sender_key, encrypted }
  └── 4. send_message(recipient, box, encrypted_value)

process_received_message(msg)
  │
  ├── 1. Detect encrypted? (encrypted==true + ciphertext field)
  │     └── yes → decrypt_message_body → unwrap to inner Value
  ├── 2. Detect signed? (signature + sender_key fields)
  │     └── yes → verify_message_signature → record valid/invalid
  └── 3. Return ProcessedMessage { body, was_encrypted, was_signed, signature_valid, sender }
```

## Consumers

- **`heartbeat/sources.rs`** — `poll_messagebox()` calls `poll_inboxes()` on adaptive timer, forwards messages as tasks via priority inbox
- **`tools/messagebox_tools.rs`** — Exposes `send_message`, `check_inbox` as agent tool closures (category: `messagebox`)
- **`runner/step.rs`** — Inbox partitioned by sender to prevent self-message loops; `compute_message_hash` used for cross-agent proof chain linking in send_message tool results
- **`server/handlers/tasks.rs`** — `/audit/cross-reference/{hash}` endpoint uses message hashes to cross-reference messages across agent proof chains

## Decisions

- **Body-transport payments, not header-transport**: Unlike most x402 services where payment goes in an HTTP header, MessageBox expects payment in the request body alongside the message. This is a BRC-33 protocol requirement. `send_message` handles this automatically via the 402 payment loop.
- **Server body unwrapping**: The MessageBox server wraps message bodies as `{"message": <original>}` and sometimes double-encodes them as JSON strings. `unwrap_message_body()` handles both cases (string parse then envelope strip). This is server behavior, not protocol spec.
- **Quote-before-send flow**: Every `send_message` first calls `/permissions/quote` to check delivery cost and whether the sender is blocked (`recipient_fee == -1`). This adds one round-trip per send but prevents sending messages that would be rejected.
- **Four named boxes with priority ordering**: `INBOX_BOXES` defines poll priority — `task_inbox` first, then `results_inbox`, `status_inbox`, `worm_coordination`. `poll_inboxes()` iterates in this order and continues on per-box errors so one failing box doesn't block the others.
- **Economic spam filter**: `TASK_INBOX_FEE` (100 sats) is the intended recipient fee for task submissions. Not enforced in code yet — requires `set_permission` to be called during agent setup.
- **UUID message IDs**: `send_message` generates a `Uuid::new_v4()` for each message and includes it in the response as `sentMessageId` so callers can correlate sends with later acknowledgements.
- **Sign-then-encrypt composition order**: `send_signed_encrypted_message` signs first, then encrypts. This means the recipient decrypts first (removing the encryption layer), then verifies the signature on the inner `SignedMessage`. Standard compose order for authenticated encryption.
- **Broadcast signing by default (BRC-77 cross-wallet fix)**: `sign_message_body()` uses counterparty `"anyone"` so any party can verify without needing the signer's private key. This matches the TS/Go SDK `SignedMessage` pattern. Prior to c27e5c3, signing used counterparty `"self"` which was unverifiable cross-wallet. Directed signing (`sign_message_body_directed`) is available for cases where only a specific recipient should be able to verify.
- **Broadcast-first verification**: `verify_message_signature()` first tries local verification via `ProtoWallet::anyone()` (ECDH = 1 * sender_pub = sender_pub). This is fast and requires no wallet call. Falls back to directed verification via the real wallet for recipient-specific signatures. Mode-1 ("self") signatures from other wallets are inherently unverifiable.
- **Field-based detection for received messages**: `process_received_message` detects encryption via `encrypted: true` + `ciphertext` and signing via `signature` + `sender_key` field presence. No explicit protocol version negotiation — just inspect the body shape.
- **Fallback server identity key**: The 402 payment loop extracts the server identity key from the `x-bsv-auth-identity-key` response header, falling back to the well-known `MESSAGEBOX_IDENTITY_KEY` constant if absent.
- **Message hash for cross-agent proof linking**: `compute_message_hash()` produces a deterministic SHA-256 of the serialized message body. Both sender and receiver can independently compute the same hash, enabling cross-reference between their respective on-chain proof chains without sharing private data.

## Gotchas

- **Acknowledge means delete**: `acknowledge_message` permanently removes messages from the server. There is no "mark as read" — once acknowledged, the message is gone. Process before acknowledging.
- **Message body field name collision**: If your message body has a top-level `"message"` key, `unwrap_message_body()` will strip it. Typed message structs (TaskAssignment, StatusUpdate, etc.) use `"type"` at top level so this doesn't happen in practice.
- **All requests are BRC-31 authenticated**: `MessageBoxClient` wraps an `AuthriteClient`. The Authrite session must be initialized before any MessageBox operation will succeed. The `AuthriteClient` handles session init lazily on first request.
- **Quote response format varies**: The quote endpoint sometimes returns `{ quote: { deliveryFee, recipientFee } }` and sometimes returns fields at top level. `quote()` handles both.
- **`list_messages` sets `message_box`**: The `message_box` field on `ReceivedMessage` is `Option<String>` and is `None` in server responses. The client sets it after retrieval so consumers know which box a message came from.
- **Error response parsing**: All methods check both `description` and `message` fields in error responses (`body.get("description").or(body.get("message"))`) since the server uses both inconsistently.
- **Signing uses deterministic serialization**: `sign_message_body`, `sign_message_body_directed`, and `verify_message_signature` all use `serde_json::to_string()` for canonical body serialization. If the body is re-serialized with different key ordering, verification will fail. Always pass the original `Value` through, not a re-parsed copy.
- **Broadcast vs directed signing matters for verification**: If a message is signed with `sign_message_body()` (broadcast, counterparty="anyone"), any party can verify. If signed with `sign_message_body_directed()` (counterparty=recipient_key), only that recipient can verify. `verify_message_signature()` tries broadcast first and falls back to directed, so it handles both transparently.
- **Payment retry burns sats**: Each 402 retry creates a new BRC-29 payment transaction. If the server keeps returning 402 (e.g., insufficient amount), up to 3 payments may be constructed before giving up. The earlier payments are not refunded automatically.
- **Message hash uses serde_json serialization**: `compute_message_hash` uses `serde_json::to_string()` which is deterministic for identical `Value` objects. If either side reconstructs the body from different sources, key ordering may differ and the hash won't match.

## Testing

- **Unit tests in `types.rs`** (16 tests) — Serde round-trips for all message types (including `SignedMessage`, `EncryptedMessage`, `ProcessedMessage`), constant validation, `DeliveryQuote` methods (free/paid/blocked), detection field patterns
- **Unit tests in `client.rs`** (16 tests) — `unwrap_message_body` (object, string, no-wrapping, plain string), client construction, protocol ID constants, payment body merge structure, signed/encrypted message structure validation, detection field patterns
- **Integration tests in `tests/test_messagebox.rs`** — Use `mockito` for HTTP mocking; cover quote, send, list, acknowledge, permissions, poll, error paths

## Not yet implemented

- **Permission enforcement on startup**: `TASK_INBOX_FEE` is defined but `set_permission` is never called automatically. Requires an agent setup/bootstrap step.

## Related

- [../auth/CLAUDE.md](../auth/CLAUDE.md) — BRC-31 Authrite session management (all MessageBox requests go through it)
- [../tools/CLAUDE.md](../tools/CLAUDE.md) — `messagebox_tools.rs` exposes send/check_inbox as agent tools
- [../x402/CLAUDE.md](../x402/CLAUDE.md) — Payment construction (`create_payment`, `parse_402_response`) used by the 402 payment loop
- [../heartbeat/CLAUDE.md](../heartbeat/CLAUDE.md) — Scheduler uses `MessageBoxClient::poll_inboxes()` for autonomous message processing
