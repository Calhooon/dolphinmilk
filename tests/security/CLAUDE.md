# security/

> 215 tests across 7 files covering BRC-31 auth, 4-layer injection defense, BRC-52 certificates, BRC-69 key linkage revelation, log redaction, credential leak detection, and cryptographic trust (HMAC transcripts, transaction staging).

## Overview

Tests the security perimeter of Dolphin Milk: authentication protocols, prompt injection defense for external messages, certificate revocation, structured key linkage revelation with audit logging, sensitive data redaction in logs and tool outputs, and cryptographic audit trail integrity. All tests run without a live wallet -- HTTP calls are mocked with `mockito`, and wallet method existence is verified via compile-time checks against unreachable ports.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_injection_defense.rs | 65 | 4-layer injection defense: boundary markers, content sanitization, tool allowlist, Aho-Corasick pattern detection, zero-width char stripping, parent bypass, system prompt warning |
| test_cryptographic_trust.rs | 33 | HMAC transcript integrity, key linkage revelation endpoints, transaction staging config/endpoints/lifecycle, wallet method compile-time checks |
| test_certificates.rs | 29 | BRC-52 certificate revocation: null outpoint detection, outpoint parsing, CertificateStatus serde, parent-relinquish guard, server endpoint existence, is_revoked field |
| test_log_redaction.rs | 25 | `redact()` function: 6 pattern matchers (API keys, Bearer tokens, BRC-31 auth headers, authrite tokens, base64 BEEF blobs, hex private keys) |
| test_key_linkage.rs | 24 | BRC-69 key linkage revelation: Revelation struct, deterministic hashing, RevelationLog disk persistence, CLI `audit` subcommand parsing, `GET /audit/revelations` endpoint |
| test_auth.rs | 23 | BRC-31 Authrite: varint encoding/decoding, request/response serialization roundtrips, header filtering, auth header construction, session persistence/expiry/clear, handshake mock |
| test_leak_detect.rs | 16 | `scan_for_leaks()` and `redact_leaks()`: 4 credential patterns (api_key, bearer_token, auth_header, authrite_token), false positive prevention, idempotent redaction |

## Key patterns tested

### 4-layer injection defense (test_injection_defense.rs)

Tests the defense pipeline from `src/sanitize.rs` against prompt injection from external MessageBox messages:

- **Layer 1 -- Boundary markers**: `generate_boundary_id()` produces 16-char hex IDs, `wrap_with_boundary()` wraps content in `<<<EXTERNAL_UNTRUSTED id="..."/<<<END id="...">>>` pairs. Tests verify uniqueness, matching open/close IDs, and that attacker-crafted fake END markers don't escape the boundary.
- **Layer 2 -- Content sanitization**: `sanitize_external_content()` strips null bytes and ASCII control chars (preserves `\n` and `\t`), collapses triple+ newlines to double, truncates at `MAX_EXTERNAL_CONTENT_LEN` with `...[truncated]` suffix.
- **Layer 3 -- Tool allowlist**: `external_tool_allowlist()` returns 10 safe tools (asserted via `EXTERNAL_TOOL_ALLOWLIST.len()`): `memory_search`, `memory_store`, `check_inbox`, `send_message`, `list_conversations`, `read_conversation`, `search_tools`, `continue_task`, `discover_services`, `discover_endpoints`. Tests verify `ToolRegistry.set_allowlist()` blocks 14 dangerous tools (`execute_bash`, `file_read`, `file_write`, `file_search`, `web_fetch`, `wallet_call`, `wallet_encrypt`, `wallet_decrypt`, `x402_call`, `generate_image`, `upload_to_nanostore`, `create_schedule`, `list_schedules`, `cancel_schedule`) and that blocked tools return "not allowed" errors on execute.
- **Layer 4 -- System prompt warning**: `build_system_prompt()` injects "External Message Warning" section (positioned between Identity and Environment) when `has_external_messages` is true. Warning tells LLM not to follow embedded instructions.
- **Aho-Corasick detection**: `detect_injections()` matches 18 patterns (asserted via `INJECTION_PATTERNS.len()`) case-insensitively (e.g., "ignore all previous instructions", `[system]`, `<|im_start|>`, `\n\nhuman:`, `<|im_end|>`, `\n\nassistant:`). Zero-width Unicode chars (`\u200B`, `\u200C`, `\u200D`, `\uFEFF`, `\u2060`, `\u00AD`) are stripped via `strip_zero_width()` before matching. Tests verify no false positives on normal English or code comments, and that every individual pattern is detectable.
- **Parent bypass**: `is_parent_sender()` compares sender key to parent key. Empty parent key (dev mode) never matches.
- **LoopState integration**: `LoopState.auth.external_origin` defaults to `false`. Tests verify the field can be set to `true` for external-origin tasks.

### BRC-31 Authrite (test_auth.rs)

Tests the `src/auth/` module:

- **Varint codec**: Boundary values (0, 252, 253, 0xFFFF, 0x10000, u64::MAX) with expected byte lengths. Error cases for truncated data.
- **EMPTY_SENTINEL**: Verifies the sentinel value is exactly 9 bytes of `0xFF`.
- **Serialization**: Request and response roundtrips via `serialize_request()`/`deserialize_request()` and `serialize_response()`/`deserialize_response()`. Verifies raw 32-byte request_id (not varint-prefixed), optional path/query/headers/body. Response roundtrip tested across 8 status codes (200, 201, 400, 401, 402, 403, 404, 500).
- **Header filtering**: `filter_signable_headers()` includes `x-bsv-payment`, `authorization`, `content-type` (stripped of params); excludes `x-bsv-auth-*`, `Accept`, `User-Agent`, `Cache-Control`. Output sorted alphabetically.
- **Auth header construction**: `build_auth_headers()` produces 7 headers: `x-bsv-auth-version`, `x-bsv-auth-identity-key`, `x-bsv-auth-message-type`, `x-bsv-auth-nonce`, `x-bsv-auth-your-nonce`, `x-bsv-auth-signature`, `x-bsv-auth-request-id`.
- **Sessions**: Save/load roundtrip via `save_session_to()`/`load_session_from()` with tempdir. TTL-based expiry auto-deletes stale sessions. Clear single session and clear all sessions.
- **AuthriteClient construction**: Default session dir contains `brc31-sessions`. Custom dir via `AuthriteClient::with_session_dir()`.
- **Handshake**: Mock `POST /.well-known/auth` endpoint. Dual-outcome pattern: success if wallet running, wallet error if not. Bad status (500) correctly rejected.

### BRC-52 certificates (test_certificates.rs)

Tests `src/certificates.rs` and certificate-related server endpoints:

- **Null outpoint**: 72 zeros or empty string → `is_null_revocation_outpoint()` returns true. Partial zeros (64 zeros + non-zero vout) returns false.
- **Outpoint parsing**: `parse_revocation_outpoint()` splits 72-char hex into 64-char txid + u32 vout. Returns None for null, wrong length, or invalid hex.
- **Constants**: `BASKET_REVOCATION` = `"worm-revocation"`, `CERT_TYPE_AGENT_AUTH` = `"agent-authorization"`.
- **CertificateStatus serde**: Roundtrip with `identity_key` field. Parent-signed certs include `revocationOutpoint`.
- **Parent-relinquish guard**: Detects parent-issued certs (certifier != subject) vs self-signed (certifier == subject). Parent-issued certs blocked from relinquish.
- **Revocation pagination**: `is_revoked_handles_various_outpoint_formats` tests null short-circuit, valid parsing, and large vout (u32::MAX).
- **Server endpoints**: Axum oneshot tests verify route existence for `POST /certificates/revoke` (405 on GET), `POST /certificates/relinquish`, `GET /certificates` (includes `is_revoked` field).

### BRC-69 key linkage revelation (test_key_linkage.rs)

Tests `src/audit/revelation.rs` for structured key linkage revelation with audit logging:

- **Revelation struct**: `Revelation::counterparty()` and `Revelation::specific()` constructors. Counterparty revelations omit `protocol_id`/`key_id` (skip_serializing_if None). Specific revelations include both. All revelations get a unique ID, timestamp, and 64-char SHA-256 `revelation_hash`. Full serde roundtrip verified.
- **Deterministic hashing**: `compute_revelation_hash()` produces consistent SHA-256 hex for the same inputs. Hash changes when any field differs: type, counterparty, requester, timestamp, or linkage data.
- **RevelationLog persistence**: `RevelationLog::from_dir()` writes individual `{id}.json` files. `list()` returns all revelations sorted by timestamp ascending. Skips non-JSON files. Auto-creates the log directory if it doesn't exist. Empty/nonexistent directories return empty vec.
- **CLI audit subcommand**: `dolphin-milk audit reveal-linkage <counterparty>` with optional `--verifier`, `--protocol`, `--key-id`, `--workspace` flags. `dolphin-milk audit list-revelations` with optional `--workspace`. Parsed via clap derive.
- **Server endpoint**: `GET /audit/revelations` returns `RevelationsListResponse { revelations, count }`. Empty workspace returns count=0. After writing a revelation to disk, endpoint reflects it.

### Cryptographic trust (test_cryptographic_trust.rs)

Tests HMAC transcript integrity, key linkage revelation endpoints, and transaction staging:

- **HMAC transcripts**: `Transcript.read_bytes()` for empty/populated/nonexistent files. Checkpoint events carry optional `hmac` field in `checkpoint_data`. Verification endpoint returns `{valid: false, error: "No HMAC found"}` when no checkpoint HMAC exists, and gracefully handles missing wallet.
- **Key linkage endpoints**: `POST /audit/key-linkage/counterparty` and `POST /audit/key-linkage/specific` routes exist. Missing `protocol_id` or `key_id` returns 400. GET returns 405.
- **Transaction staging**: `BudgetConfig.staging_threshold` defaults to 500,000 sats, configurable via TOML. `GET /staged` returns empty list. `POST /staged/{ref}/approve` and `POST /staged/{ref}/abort` return 404 for nonexistent refs. `StagedTransaction` serde with optional `task_id` and `description`.
- **Wallet method existence**: Compile-time verification that `sign_action`, `abort_action`, `create_hmac`, `verify_hmac`, `reveal_counterparty_key_linkage`, `reveal_specific_key_linkage` exist on `WalletClient` by calling them against port 9999 and asserting `is_err()`.

### Log redaction (test_log_redaction.rs)

Tests the `redact()` function from `src/logging.rs` with 6 pattern matchers:

| Pattern | Threshold | Example |
|---------|-----------|---------|
| API key (`sk-...`) | 20+ chars after prefix | `sk-abcdefghijklmnopqrstuvwx` → `[REDACTED]` |
| Bearer token | 20+ chars after "Bearer " | `Bearer eyJhbGci...` → `[REDACTED]` |
| BRC-31 auth header | any value after colon | `x-bsv-auth-nonce: abc123` → `[REDACTED]` |
| Authrite token | 10+ chars after prefix | `authrite-abcdefghij1234` → `[REDACTED]` |
| Base64 BEEF blob | 200+ chars | 250-char `A` string → `[REDACTED]` |
| Hex private key | exactly 64 hex chars at word boundary | `aaaa...aaaa` (64) → `[REDACTED]` |

Tests verify thresholds are respected (short values not redacted), multiple patterns in one line, case insensitivity for BRC-31 headers, BRC-31 headers with empty values not redacted, base64 with `+`/`/`/`=` chars, boundary markers not accidentally redacted, normal log lines unchanged, JSON structure preserved, and `is_redaction_enabled()` returns true by default.

### Credential leak detection (test_leak_detect.rs)

Tests `scan_for_leaks()` and `redact_leaks()` from `src/sanitize.rs` for tool output scanning:

- **4 patterns**: `api_key` (sk-..., 20+ chars), `bearer_token` (Bearer + 20+ chars), `auth_header` (x-bsv-auth-*), `authrite_token` (authrite-..., 10+ chars)
- `scan_for_leaks()` returns `Vec<LeakWarning>` with pattern name, start/end byte offsets
- `redact_leaks()` replaces matches with `[REDACTED:pattern_name]` tags
- Redaction is idempotent (double-redact produces same output)
- Word boundary enforcement: `prefix-sk-...` matches but `tsk-...` does not

## Helpers

| Helper | File | Purpose |
|--------|------|---------|
| `test_router()` | test_certificates.rs, test_cryptographic_trust.rs, test_key_linkage.rs | Axum router with leaked TempDir, dev mode (no auth) |
| `test_router_with_workspace()` | test_cryptographic_trust.rs, test_key_linkage.rs | Returns `(Router, PathBuf)` for workspace file creation |
| `tmp_transcript()` | test_cryptographic_trust.rs | Returns `(TempDir, PathBuf)` for JSONL transcript tests |
| `extract_boundary_id()` | test_injection_defense.rs | Extracts hex ID from `<<<EXTERNAL_UNTRUSTED id="...">>>` string |
| `make_prompt_ctx(bool)` | test_injection_defense.rs | Creates zeroed `PromptContext` with `has_external_messages` flag, empty `basket_health`, zero `spendable_output_count` |
| `PARENT_KEY` / `EXTERNAL_KEY` | test_injection_defense.rs | Valid secp256k1 pubkeys (generator point G and point 2G) |

## Gotchas

- **Revocation outpoint format**: 72-char hex string = 64-char txid + 8-char hex-encoded u32 vout. Tests assert exact length.
- **Redaction thresholds matter**: `sk-` needs 20+ chars, Bearer needs 20+ chars, base64 needs 200+ chars, hex keys need exactly 64 chars at word boundary. Tests verify both sides of each threshold.
- **Compile-time wallet method checks**: `test_cryptographic_trust.rs` calls wallet methods against port 9999. If method signatures change, the test fails to compile before it even runs.
- **Injection detection is log-only**: `sanitize_external_content()` detects and logs injection patterns but does NOT reject the content. The allowlist (Layer 3) is the enforcement layer.
- **Dual-outcome auth tests**: `test_auth.rs` handshake tests accept both success (wallet running) and wallet error (wallet not running) as valid outcomes.
- **TempDir leak pattern**: `test_router()` in `test_certificates.rs`, `test_cryptographic_trust.rs`, and `test_key_linkage.rs` uses `std::mem::forget(dir)` because AppState holds `Arc<PathBuf>` that outlives the test.
- **Revelation hash is SHA-256**: `compute_revelation_hash()` always produces a 64-char hex string. Tests assert exact length and determinism.
- **RevelationLog auto-creates directory**: `RevelationLog::from_dir()` calls `create_dir_all` on construction. Tests verify this by passing a nonexistent nested path.

## Related

- [tests/CLAUDE.md](../CLAUDE.md) -- Test suite overview, conventions, all helpers
- [src/sanitize.rs](../../src/sanitize.rs) -- 4-layer injection defense implementation
- [src/logging.rs](../../src/logging.rs) -- RedactingWriter with 6 pattern matchers
- [src/auth/](../../src/auth/) -- BRC-31 Authrite client (serialization, sessions, handshake)
- [src/certificates.rs](../../src/certificates.rs) -- BRC-52 certificate revocation
- [src/audit/](../../src/audit/) -- Key linkage revelation (revelation.rs), audit trail
- [src/server/](../../src/server/) -- Axum HTTP API (key linkage, staging, certificate, revelation endpoints)
- [integration/CLAUDE.md](../integration/CLAUDE.md) -- Playwright E2E tests that exercise auth and security in production
