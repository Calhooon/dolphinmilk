# Learning — auth

## Lessons Learned
- **[2026-02-24]** Connection pooling must be disabled (`pool_max_idle_per_host(0)`) for Cloudflare Workers targets. Pooled connections silently hang on reuse because Workers drop keep-alive between requests. This manifests as random timeouts, not connection errors.
- **[2026-02-24]** JSON body normalization before signing is critical. `authenticated_request` re-serializes JSON to compact form because pretty-printed whitespace changes the signed bytes. If you ever skip this step, signatures will intermittently fail depending on how the caller formatted the JSON.
- **[2026-02-24]** The handshake response field names are not standardized across servers. `identityKey` vs `identity_key`, `initialNonce` vs `initial_nonce` vs `nonce` — the `get().or_else()` chains in `client.rs:144-161` handle all known variants. When adding a new BRC-31 server, check what field names it actually returns.
- **[2026-02-24]** The `yourNonce` anti-replay check in `do_handshake` is optional — some servers don't return it. The code only validates if present (`client.rs:164-174`). Don't make it mandatory or you'll break compatibility.

## Debugging Insights
- **[2026-02-24]** Signature mismatches: check three things in order — (1) is the JSON body compact-serialized? (2) is Content-Type stripped to media type only (no `;charset=...`)? (3) are `x-bsv-auth-*` headers excluded from signing while other `x-bsv-*` headers are included? The `filter_signable_headers` function in `serialization.rs:396` handles all three rules.
- **[2026-02-24]** Handshake storms (repeated re-handshakes) usually mean expired sessions. Check clock sync between client and server — the 1-hour TTL (`session.rs:14`) uses `SystemTime::now()` comparison, so clock skew directly eats into session lifetime.
- **[2026-02-24]** Session files live at `~/.local/share/brc31-sessions/{sha256(url)[:16]}.json`. To debug session state: `ls ~/.local/share/brc31-sessions/` and `cat` any file — they're human-readable JSON with timestamp, nonces, and server identity key.
- **[2026-02-24]** The 300s request timeout (`client.rs:327`) is intentionally long for LLM inference. If you see timeout errors on non-LLM endpoints, the real issue is likely the connection pooling problem, not the timeout being too short.

## Pattern Notes
- **[2026-02-24]** The signing key derivation uses `key_id = "{msg_nonce} {server_nonce}"` (space-separated, both base64) with protocol `[2, "auth message signature"]` and the server's identity key as counterparty. This matches the TS SDK's AuthFetch. Getting any of these three parameters wrong produces valid-looking but rejected signatures.
- **[2026-02-24]** EMPTY_SENTINEL (`[0xFF; 9]`) in BRC-104 binary format represents absent optional fields. It's 9 bytes, not 8 — the `0xFF` prefix byte plus 8 bytes of `0xFF` data mimics a varint-encoded `u64::MAX`. The `is_empty_sentinel` check at `serialization.rs:177` is offset-aware; test it with non-zero offsets.
- **[2026-02-24]** The `_to` / `_from` suffix pattern on session functions (e.g., `save_session` vs `save_session_to`) keeps the public API clean while making tests fully deterministic with `tempfile::tempdir()`. Follow this pattern when extending session management.
- **[2026-02-24]** Request ID bytes are raw 32 bytes in the binary format (NOT varint-prefixed), unlike every other field. This is the one exception in `serialize_request` — see `serialization.rs:124`. Easy to get wrong when adding new serialization code.
