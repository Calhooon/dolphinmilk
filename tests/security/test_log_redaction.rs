//! Tests for log redaction patterns in `src/logging.rs`.

use dolphin_milk::logging::redact;

// ---------------------------------------------------------------------------
// API key redaction (sk-...)
// ---------------------------------------------------------------------------

#[test]
fn redacts_openai_api_key() {
    let input = "Using key sk-abcdefghijklmnopqrstuvwx for auth";
    let result = redact(input);
    assert_eq!(result, "Using key [REDACTED] for auth");
}

#[test]
fn redacts_long_api_key() {
    let key = format!("sk-{}", "a".repeat(100));
    let input = format!("key={key}");
    let result = redact(&input);
    assert_eq!(result, "key=[REDACTED]");
}

#[test]
fn preserves_short_sk_prefix() {
    // sk- followed by fewer than 20 chars should NOT be redacted
    let input = "sk-short is fine";
    let result = redact(input);
    assert_eq!(result, input);
}

// ---------------------------------------------------------------------------
// Bearer token redaction
// ---------------------------------------------------------------------------

#[test]
fn redacts_bearer_token() {
    let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature";
    let result = redact(input);
    assert_eq!(result, "Authorization: [REDACTED]");
}

#[test]
fn preserves_short_bearer() {
    let input = "Bearer short";
    let result = redact(input);
    assert_eq!(result, input);
}

// ---------------------------------------------------------------------------
// BRC-31 auth header redaction (x-bsv-auth-*)
// ---------------------------------------------------------------------------

#[test]
fn redacts_bsv_auth_nonce() {
    let input = "x-bsv-auth-nonce: abc123def456";
    let result = redact(input);
    assert_eq!(result, "[REDACTED]");
}

#[test]
fn redacts_bsv_auth_signature() {
    let input = "x-bsv-auth-signature: 304402...deadbeef";
    let result = redact(input);
    assert_eq!(result, "[REDACTED]");
}

#[test]
fn redacts_uppercase_bsv_auth() {
    let input = "X-BSV-AUTH-NONCE: abc123def456";
    let result = redact(input);
    assert_eq!(result, "[REDACTED]");
}

#[test]
fn preserves_bsv_auth_without_value() {
    // No value after colon — should not redact
    let input = "x-bsv-auth-nonce: ";
    let result = redact(input);
    assert_eq!(result, input);
}

// ---------------------------------------------------------------------------
// Authrite token redaction
// ---------------------------------------------------------------------------

#[test]
fn redacts_authrite_token() {
    let input = "token=authrite-abcdefghij1234567890";
    let result = redact(input);
    assert_eq!(result, "token=[REDACTED]");
}

#[test]
fn preserves_short_authrite() {
    let input = "authrite-short";
    let result = redact(input);
    assert_eq!(result, input);
}

// ---------------------------------------------------------------------------
// Base64 BEEF data redaction (200+ chars)
// ---------------------------------------------------------------------------

#[test]
fn redacts_long_base64() {
    let base64_blob = "A".repeat(250);
    let input = format!("BEEF data: {base64_blob} end");
    let result = redact(&input);
    assert_eq!(result, "BEEF data: [REDACTED] end");
}

#[test]
fn preserves_short_base64() {
    let base64_blob = "A".repeat(100);
    let input = format!("data: {base64_blob} end");
    let result = redact(&input);
    assert_eq!(result, input);
}

#[test]
fn redacts_base64_with_special_chars() {
    // Base64 includes +, /, and = characters
    let base64_blob: String = (0..250)
        .map(|i| match i % 4 {
            0 => 'A',
            1 => '+',
            2 => '/',
            3 => '=',
            _ => unreachable!(),
        })
        .collect();
    let input = format!("payload: {base64_blob} done");
    let result = redact(&input);
    assert_eq!(result, "payload: [REDACTED] done");
}

// ---------------------------------------------------------------------------
// Hex private key redaction (64 hex chars at word boundary)
// ---------------------------------------------------------------------------

#[test]
fn redacts_hex_private_key() {
    let hex_key = "a".repeat(64);
    let input = format!("privkey: {hex_key} stored");
    let result = redact(&input);
    assert_eq!(result, "privkey: [REDACTED] stored");
}

#[test]
fn redacts_mixed_case_hex() {
    let hex_key = "aAbBcCdDeEfF00112233445566778899aAbBcCdDeEfF00112233445566778899";
    assert_eq!(hex_key.len(), 64);
    let input = format!("key={hex_key}!");
    let result = redact(&input);
    assert!(
        result.contains("[REDACTED]"),
        "Should redact 64-char hex: {result}"
    );
}

#[test]
fn preserves_hex_not_at_boundary() {
    // 64 hex chars embedded in a longer alphanumeric string — not a word boundary
    let hex_key = "a".repeat(64);
    let input = format!("prefix{hex_key}suffix");
    let result = redact(&input);
    // Should NOT redact because no word boundary
    assert!(
        !result.contains("[REDACTED]"),
        "Should not redact hex without word boundary: {result}"
    );
}

#[test]
fn preserves_short_hex() {
    let hex = "a".repeat(32);
    let input = format!("hash: {hex} done");
    let result = redact(&input);
    assert_eq!(result, input);
}

// ---------------------------------------------------------------------------
// Multiple patterns in one line
// ---------------------------------------------------------------------------

#[test]
fn redacts_multiple_patterns() {
    let input =
        "key=sk-abcdefghijklmnopqrstuvwx auth=Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature";
    let result = redact(input);
    assert_eq!(result, "key=[REDACTED] auth=[REDACTED]");
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_input() {
    assert_eq!(redact(""), "");
}

#[test]
fn normal_log_line_unchanged() {
    let input = "2026-03-03 INFO dolphin_milk::runner: Starting task abc-123";
    let result = redact(input);
    assert_eq!(result, input);
}

#[test]
fn preserves_json_structure() {
    let input = r#"{"level":"INFO","message":"task started","task_id":"abc-123"}"#;
    let result = redact(input);
    assert_eq!(result, input);
}

#[test]
fn redaction_enabled_check() {
    // Default is enabled
    assert!(dolphin_milk::logging::is_redaction_enabled());
}

#[test]
fn boundary_marker_in_message_preserved() {
    // Ensure we don't accidentally redact boundary markers from injection defense
    let input = "<<<EXTERNAL_UNTRUSTED id=\"abc123\">>> message here <<<END id=\"abc123\">>>";
    let result = redact(input);
    assert_eq!(result, input);
}

#[test]
fn partial_api_key_prefix_not_redacted() {
    let input = "The command was sk-ip this step";
    let result = redact(input);
    // "sk-ip" has only 2 chars after sk-, way below threshold
    assert_eq!(result, input);
}
