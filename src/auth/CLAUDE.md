# auth
> BRC-31 Authrite mutual authentication — client-side request signing, server-side request verification, and response signing.

## Public API

Exported from `mod.rs`:
- `AuthriteClient` — Client struct for making authenticated requests (from `client.rs`)
- `AuthSession` — Client-side session data struct (from `session.rs`)
- `Brc31AuthContext` — Context from a verified request, needed for response signing (from `server.rs`)
- `Brc31AuthParams` — Auth parameters extracted from `x-bsv-auth-*` headers (from `server.rs`)
- `Brc31ServerSession` — Server-side session created during handshake (from `server.rs`)
- `Brc31SessionStore` — In-memory HashMap store for server sessions (from `server.rs`)
- `HandshakeRequest` — JSON body for `POST /.well-known/auth` (from `server.rs`)
- `HandshakeResponse` — JSON body for handshake response (from `server.rs`)

## Files

### `client.rs` (451 lines)
`AuthriteClient` — orchestrates the full client-side BRC-31 flow: handshake, session management, payload serialization, signing, and sending.

**Struct:**
- `AuthriteClient { wallet: WalletClient, http: Client, pub session_dir: PathBuf }`

**Key methods:**
- `new(wallet)` — Creates client with per-wallet session dir (`~/.local/share/brc31-sessions/{wallet_hash}/`) where `wallet_hash = SHA-256(wallet_url)[:8]`. Prevents session collisions when multiple worms share the same filesystem.
- `with_session_dir(wallet, dir)` — Creates client with custom session dir (for testing)
- `wallet()` — Accessor for the underlying `WalletClient`
- `get_or_create_session(server_url)` — Loads cached session or performs handshake
- `do_handshake(server_url)` — Full BRC-31 handshake: generate client nonce, POST to `/.well-known/auth`, parse response, persist session
- `authenticated_request(method, url, headers, body)` — Main entry point for all auth'd HTTP calls. Handles URL parsing, session lookup, JSON normalization, nonce generation, BRC-104 serialization, wallet signing, and sending
- `post_json(url, body)` — Convenience wrapper for authenticated POST with JSON body
- `get(url)` — Convenience wrapper for authenticated GET

**Constants:**
- `AUTH_VERSION = "0.1"` — BRC-31 protocol version
- `HANDSHAKE_PATH = "/.well-known/auth"` — Standard handshake endpoint

### `server.rs` (656 lines)
Server-side BRC-31 authentication — handshake handling, in-memory session store, header extraction, request signature verification, and response signing. Used by `src/server/` (the axum HTTP server) to authenticate parent console requests and sign responses. Signing functions use `bsv::wallet::substrates::HttpWalletJson` directly (with `CreateSignatureArgs`, `Protocol`, `Counterparty` from `bsv::wallet`); verification uses `WalletClient`.

**Structs:**
- `Brc31ServerSession { client_identity_key, client_nonce_b64, server_nonce_b64, created_at: Instant }` — Server-side session, 1-hour TTL via `Instant` elapsed check. Has `is_expired()` method.
- `Brc31SessionStore` — `HashMap<String, Brc31ServerSession>` keyed by server nonce (base64). Implements `Default`.
- `HandshakeRequest { version, message_type, identity_key, initial_nonce }` — camelCase serde for JSON deserialization from `POST /.well-known/auth`
- `HandshakeResponse { version, message_type, identity_key, initial_nonce, your_nonce, signature }` — camelCase serde for JSON serialization back to client. `signature` is `Option<Vec<u8>>`, serialized as JSON array of byte values, skipped if `None`. Required by SDK's `Peer.processInitialResponse()`.
- `Brc31AuthParams { version, identity_key, nonce, your_nonce, signature, request_id, message_type }` — Extracted from `x-bsv-auth-*` request headers. 6 required + `message_type` defaults to `"general"` when absent.
- `Brc31AuthContext { client_identity_key, client_session_nonce, server_nonce, request_id_b64 }` — Context extracted from a verified BRC-31 request. Carries the nonces and request ID needed to sign the response. Created by the server after verifying an inbound request, passed to `sign_response_payload()`.

**Key functions:**
- `handle_handshake(req, server_identity_key)` → `(HandshakeResponse, Brc31ServerSession)` — Generates 32-byte server nonce, creates session, returns response and session for caller to store
- `extract_auth_params(headers)` → `Option<Brc31AuthParams>` — Extracts 6 required auth headers + optional `message_type` (defaults to `"general"`), case-insensitive. Returns `None` if any required header is missing
- `verify_request(wallet: &WalletClient, params, session, method, path, query, headers, body)` → `bool` — Decodes request_id from base64, filters signable headers, serializes BRC-104 payload, verifies signature via `wallet.verify_signature()`. Takes `WalletClient` (not `HttpWalletJson`).
- `sign_handshake_response(wallet: &HttpWalletJson, ...)` → `WormResult<Vec<u8>>` — Signs `client_nonce_bytes || server_nonce_bytes` (64 bytes) for SDK compatibility. The signature proves the server's identity during handshake.
- `sign_response_payload(wallet: &HttpWalletJson, auth_ctx, status_code, response_headers, body)` → `WormResult<(nonce_b64, signature_hex)>` — Signs an HTTP response for BRC-31 mutual authentication. Generates response nonce, serializes response via BRC-104, signs with `key_id = "{response_nonce} {client_session_nonce}"`. Counterparty derived from `auth_ctx.client_identity_key`.

**Helper functions:**
- `base64_encode(data)` / `base64_decode(s)` — Standard base64 with padding. `pub(crate)` visibility.

**Session store methods:**
- `new()` — Create an empty store (also via `Default` impl)
- `insert(session)` — Store a session keyed by its server nonce
- `get(server_nonce)` → `Option<&Brc31ServerSession>` — Lookup by nonce, returns `None` if expired
- `cleanup()` — Remove all expired sessions
- `len()` / `is_empty()` — Count active (non-expired) sessions

**Constants:**
- `AUTH_VERSION = "0.1"` — Same as client
- `SESSION_TTL_SECS = 3600` — 1-hour session lifetime

### `session.rs` (330 lines)
Client-side session persistence — stores one JSON file per server, keyed by `SHA-256(server_url)[:16]`.

**Struct:**
- `AuthSession { server_url, server_identity_key, server_nonce_b64, client_nonce_b64, timestamp }`
- `new(server_url, server_identity_key, server_nonce_b64, client_nonce_b64)` — Constructor with auto-generated timestamp
- `is_expired(ttl)` — Check if session has exceeded TTL (uses `SystemTime` for disk persistence)

**Functions:**
- `save_session(session)` / `save_session_to(session, dir)` — Persist to disk as pretty-printed JSON
- `load_session(url, ttl)` / `load_session_from(url, ttl, dir)` — Load from disk, returns `None` if missing or expired (auto-deletes expired files)
- `clear_session(url)` / `clear_session_from(url, dir)` — Delete a specific session
- `clear_all_sessions_from(dir)` — Delete all `.json` session files in a directory
- `base_url_from(url)` — Extract base URL (`scheme://host[:port]`) from a full endpoint URL. Sessions are keyed by base URL, so callers with full endpoint URLs (e.g. `https://nanostore.babbage.systems/upload`) must extract the base before calling `clear_session_from` or `load_session_from`. Falls back to the original string on parse failure.

**Constants:**
- `DEFAULT_TTL = 3600` — 1 hour session lifetime

### `serialization.rs` (734 lines)
BRC-104 binary format for request/response signing. Bitcoin-style varint encoding throughout. Used by both client (`client.rs`) and server (`server.rs`).

**Serialization functions:**
- `serialize_request(request_id, method, path, query, headers, body)` — Request → binary for signing
- `serialize_response(request_id, status_code, headers, body)` — Response → binary for signing
- `deserialize_request(payload)` → `DeserializedRequest` — Binary → request fields
- `deserialize_response(payload)` → `DeserializedResponse` — Binary → response fields

**Header/auth helpers:**
- `filter_signable_headers(headers)` — Filters and sorts request headers for signing (includes `content-type`)
- `filter_response_signable_headers(headers)` — Filters and sorts response headers for signing (excludes `content-type`)
- `build_auth_headers(identity_key, nonce, your_nonce, signature, request_id)` — Builds the 7 `x-bsv-auth-*` headers

**Varint primitives:**
- `write_varint(buf, n)` / `read_varint(data)` — Bitcoin-style variable-length integers (1/3/5/9 bytes)

**Constants:**
- `EMPTY_SENTINEL = [0xFF; 9]` — "absent" marker for optional fields (path, query, body)

**Data structs:**
- `DeserializedRequest { request_id, method, path, query, headers, body }`
- `DeserializedResponse { request_id, status_code, headers, body }`

### `mod.rs` (20 lines)
Re-exports `AuthriteClient`, `AuthSession`, and server-side types (`Brc31AuthContext`, `Brc31AuthParams`, `Brc31ServerSession`, `Brc31SessionStore`, `HandshakeRequest`, `HandshakeResponse`).

## Client Auth Flow

```
1. Client calls authenticated_request(method, url, headers, body)
2. Parse URL → extract server_base, path, query
3. get_or_create_session(server_base):
   a. Try load_session_from(url, 3600, session_dir) — returns cached if valid
   b. If miss/expired → do_handshake():
      - Generate 32-byte client nonce (base64)
      - POST to {server}/.well-known/auth with {version, messageType, identityKey, initialNonce}
      - Parse response: server identityKey, initialNonce, yourNonce (anti-replay check)
      - Persist session as {wallet_hash}/SHA-256(url)[:16].json
4. Normalize JSON body to compact form (re-serialize via serde_json)
5. Generate per-message nonce (32 bytes, base64) and request ID (32 bytes, base64)
6. Filter and sort signable headers
7. Serialize request into BRC-104 binary format
8. Sign: wallet.create_signature(serialized, [2, "auth message signature"], key_id, counterparty)
   - key_id = "{msg_nonce_b64} {server_nonce_b64}"
   - counterparty = server's identity key
9. Attach 7 x-bsv-auth-* headers + original headers
10. Send with 300s timeout, return response
```

## Server Verification Flow

```
1. Server receives POST /.well-known/auth → deserialize HandshakeRequest
2. handle_handshake(req, server_identity_key):
   - Generate 32-byte server nonce
   - Build Brc31ServerSession, store in Brc31SessionStore
   - Return HandshakeResponse with server identity, nonce, yourNonce echo
   - Optionally: sign_handshake_response() to prove server identity (SDK compat)
3. On authenticated request:
   a. extract_auth_params(headers) → 7 required x-bsv-auth-* headers
   b. session_store.get(params.your_nonce) → find session by server nonce
   c. verify_request(wallet: &WalletClient, params, session, method, path, query, headers, body):
      - Decode request_id from base64 → 32 bytes
      - Filter signable headers (same rules as client)
      - Serialize request into BRC-104 binary format
      - key_id = "{msg_nonce} {server_nonce}" (matches client-side)
      - counterparty = client's identity key
      - wallet.verify_signature(serialized, sig_bytes, protocol_id, key_id, counterparty)
   d. Build Brc31AuthContext from verified params + session
4. On response:
   a. sign_response_payload(wallet: &HttpWalletJson, auth_ctx, status, headers, body)
      - Generate 32-byte response nonce
      - Filter response headers (excludes content-type)
      - Serialize response via BRC-104 binary format
      - key_id = "{response_nonce} {client_session_nonce}"
      - counterparty = client's identity key
      - Returns (nonce_b64, signature_hex) for auth headers
```

## BRC-104 Wire Format

### Request
```
[request_id:  32 raw bytes, NOT varint-prefixed]
[method:      varint(len) + UTF-8 bytes]
[path:        varint(len) + UTF-8 bytes, OR EMPTY_SENTINEL]
[query:       varint(len) + UTF-8 bytes, OR EMPTY_SENTINEL]
[headers:     varint(count) then for each: varint(key_len)+key + varint(val_len)+val]
[body:        varint(len) + bytes, OR EMPTY_SENTINEL]
```

### Response
```
[request_id:  32 raw bytes]
[status_code: varint]
[headers:     varint(count) + pairs]
[body:        varint(len) + bytes, OR EMPTY_SENTINEL]
```

### Varint encoding
| Range | Format |
|-------|--------|
| 0–252 | 1 byte literal |
| 253–0xFFFF | `0xFD` + 2-byte LE |
| 0x10000–0xFFFFFFFF | `0xFE` + 4-byte LE |
| > 0xFFFFFFFF | `0xFF` + 8-byte LE |

## Header Signing Rules

### Request headers (`filter_signable_headers`)
1. **Include** `x-bsv-*` headers — EXCEPT `x-bsv-auth-*` (those carry the signature itself)
2. **Include** `authorization` — verbatim
3. **Include** `content-type` — media type only (strip `;charset=...` and other params)
4. **Exclude** all other headers silently
5. **Sort** result alphabetically by lowercase key

### Response headers (`filter_response_signable_headers`)
1. **Include** `x-bsv-*` headers — EXCEPT `x-bsv-auth-*`
2. **Include** `authorization` — verbatim
3. **Exclude** `content-type` (unlike request signing)
4. **Exclude** all other headers silently
5. **Sort** result alphabetically by lowercase key

## Auth Headers

`build_auth_headers` produces these 7 headers on every authenticated request:

| Header | Value |
|--------|-------|
| `x-bsv-auth-version` | `"0.1"` |
| `x-bsv-auth-identity-key` | Client's identity pubkey (hex) |
| `x-bsv-auth-message-type` | `"general"` |
| `x-bsv-auth-nonce` | Per-message nonce (base64) |
| `x-bsv-auth-your-nonce` | Server's nonce from session (base64) |
| `x-bsv-auth-signature` | Hex-encoded signature over BRC-104 payload |
| `x-bsv-auth-request-id` | Per-request ID (base64) |

## Decisions

- **Ported from Python, not the Rust SDK.** The Rust SDK's auth module didn't exist when this was built. The port is ~600 lines vs. 2,600 in Python. Server-side signing functions (`sign_handshake_response`, `sign_response_payload`) use `bsv::wallet::substrates::HttpWalletJson` directly (via `CreateSignatureArgs` with `Protocol::new(SecurityLevel::Counterparty, "auth message signature")` and `Counterparty::from_hex()`), passing `"bsv-worm"` as the originator. `verify_request` and client-side use `WalletClient`.
- **Connection pooling disabled.** `pool_max_idle_per_host(0)` in `client.rs` because Cloudflare Workers drop keep-alive connections between requests, causing pooled connections to hang until timeout.
- **JSON body normalization.** `authenticated_request` re-serializes JSON bodies to compact form before signing. The signed bytes must match what the server sees — pretty-printed JSON would cause signature mismatches.
- **Per-wallet session isolation.** Each wallet gets its own session subdirectory keyed by `SHA-256(wallet_url)[:8]`. Prevents session collisions when multiple worms share the same filesystem (e.g. two agents on different wallets). Within each subdirectory, sessions are stored as `SHA-256(server_url)[:16].json` files.
- **Session keyed by server nonce (server).** Server-side sessions stored in a `HashMap` keyed by the server nonce (base64). In-memory only — sessions are lost on restart (1-hour TTL makes this acceptable).
- **Signature key derivation.** The `key_id` for signing is `"{msg_nonce} {server_nonce}"` (space-separated), with protocol `[2, "auth message signature"]` and the counterparty's identity key. Both client and server use the same derivation. Response signing uses `"{response_nonce} {client_session_nonce}"`.
- **Trailing slash normalization.** Both `get_or_create_session` and `do_handshake` strip trailing slashes from server URLs before session lookup/creation. Prevents duplicate sessions.
- **Server uses `Instant` not `SystemTime`.** Server sessions use `std::time::Instant` for monotonic expiry (immune to clock adjustments). Client sessions use `SystemTime` since they persist to disk across restarts.
- **Response signing excludes content-type.** `filter_response_signable_headers` uses different rules from request signing — `content-type` is excluded. This matches the SDK's `SimplifiedFetchTransport` behavior.
- **Handshake response signature.** `sign_handshake_response()` signs `client_nonce_bytes || server_nonce_bytes` to match SDK's `Peer.processInitialResponse()`. The `HandshakeResponse.signature` field is `Option<Vec<u8>>` — serialized as a JSON array of byte values, omitted when `None`.

## Gotchas

- **Handshake response field names vary.** Servers may return `identityKey` or `identity_key`, `initialNonce` or `initial_nonce` or `nonce`. The client checks all variants via `get().or_else()` chains.
- **EMPTY_SENTINEL is 9 bytes of 0xFF.** In BRC-104 binary format, optional fields (path, query, body) use `[0xFF; 9]` to mean "absent." Equivalent to `writeVarIntNum(-1)` in the TS SDK — looks like a varint-encoded `u64::MAX`. Don't confuse it with an actual 8-byte varint.
- **`x-bsv-auth-*` headers are excluded from signing.** `filter_signable_headers` includes `x-bsv-*` but explicitly skips `x-bsv-auth-*` since those carry the signature itself. Other `x-bsv-*` headers (like `x-bsv-payment`) are signed.
- **Content-Type params stripped (request only).** Only the media type is signed — `application/json; charset=utf-8` becomes `application/json`. Servers must do the same or signatures won't match. Response signing excludes content-type entirely.
- **Session TTL is 1 hour** on both client and server. Client auto-deletes expired files on load. Server `get()` filters expired sessions automatically; call `cleanup()` periodically to reclaim memory.
- **Request timeout is 300s (5 min)** for authenticated requests, 30s for handshakes. The long timeout accommodates slow LLM inference responses.
- **request_id is raw 32 bytes in BRC-104.** Unlike all other fields, the request ID is NOT varint-prefixed in the binary format. This matches the TS SDK's serialization.
- **Anti-replay via yourNonce.** The handshake response includes a `yourNonce` field echoing the client's nonce. The client validates this but doesn't fail if the field is absent (some servers omit it).
- **Session keys are base URLs, not endpoint URLs.** Sessions are keyed by `SHA-256(base_url)[:16]` where base URL is `scheme://host[:port]`. Callers clearing sessions after a 401 must use `base_url_from()` to extract the base URL first — passing a full endpoint URL (e.g. `https://nanostore.babbage.systems/upload`) produces a different hash and silently fails to clear the stale session.
- **Short server nonce rejection.** Server nonces shorter than 4 characters are rejected as suspicious. This guards against servers returning empty or trivial nonces.
- **Server `extract_auth_params` is case-insensitive.** Header name matching uses `eq_ignore_ascii_case`, so `X-BSV-Auth-Version` and `x-bsv-auth-version` both work. `x-bsv-auth-message-type` is optional — defaults to `"general"` when absent (SDK doesn't send it on requests).
- **HandshakeRequest/Response use camelCase serde.** `#[serde(rename_all = "camelCase")]` — JSON fields are `messageType`, `identityKey`, `initialNonce`, `yourNonce`.
- **HandshakeResponse.signature is Vec<u8>.** Serialized as a JSON array of byte values (e.g., `[1,2,3,4,255]`), not hex or base64. This matches the SDK's expected format.

## Testing

Client tests use `mockito` for HTTP mocking and `tempfile` for session directory isolation. The handshake mock test is wallet-conditional — succeeds if local wallet is running, otherwise asserts a wallet-related error. All serialization tests use roundtrip verification (serialize → deserialize → compare). Server tests are pure unit tests — session store, handshake, auth param extraction, serde, expiry, `Brc31AuthContext`, and handshake signature field serialization all tested without network or wallet. Response header filtering has dedicated tests verifying content-type exclusion.

## Related

- [../CLAUDE.md](../CLAUDE.md) — Project architecture and conventions
- `../wallet.rs` — `WalletClient` used by client-side and `verify_request`; server-side signing uses `bsv::wallet::substrates::HttpWalletJson` directly
- `../x402/` — Payment flow that composes with auth (auth request → 402 → create payment → auth retry)
- `../think/` — Primary client-side consumer; calls `authenticated_request` for LLM inference
- `../messagebox/` — Another client-side consumer; uses auth for BRC-33 message delivery
- `../server/` — Primary server-side consumer; `server/auth.rs` uses `handle_handshake`, `sign_handshake_response`, `Brc31SessionStore`, `extract_auth_params`, `verify_request`, `Brc31AuthContext`, and `sign_response_payload` for parent console auth
