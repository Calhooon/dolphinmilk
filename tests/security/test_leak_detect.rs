//! Tests for credential leak detection on tool outputs (CI.8 / issue #81).
//!
//! Verifies that `scan_for_leaks()` detects 4 credential patterns and
//! `redact_leaks()` replaces them with `[REDACTED:pattern_name]` tags.

use dolphin_milk::sanitize::{redact_leaks, scan_for_leaks};

// =========================================================================
// Detection tests
// =========================================================================

#[test]
fn test_scan_detects_api_key() {
    let output = "config: sk-abcdefghijklmnopqrstuvwxyz1234 is set";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].pattern, "api_key");
    // The matched text should start with "sk-"
    let matched = &output[warnings[0].start..warnings[0].end];
    assert!(matched.starts_with("sk-"));
}

#[test]
fn test_scan_detects_bearer_token() {
    let output = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.long_token_value_here";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].pattern, "bearer_token");
    let matched = &output[warnings[0].start..warnings[0].end];
    assert!(matched.starts_with("Bearer "));
}

#[test]
fn test_scan_detects_auth_header() {
    let output = "headers: x-bsv-auth-nonce=abc123def456 sent";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].pattern, "auth_header");
    let matched = &output[warnings[0].start..warnings[0].end];
    assert!(matched.starts_with("x-bsv-auth-"));
}

#[test]
fn test_scan_detects_authrite_token() {
    let output = "token: authrite-AbCdEfGhIj0123456789 was used";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].pattern, "authrite_token");
    let matched = &output[warnings[0].start..warnings[0].end];
    assert!(matched.starts_with("authrite-"));
}

#[test]
fn test_scan_clean_output_no_leaks() {
    let clean_outputs = [
        "Tool executed successfully. Result: 42",
        "{\"status\": \"ok\", \"data\": [1, 2, 3]}",
        "No credentials here, just normal text.",
        "",
        "sk- (too short)",
        "Bearer short",
    ];
    for output in &clean_outputs {
        let warnings = scan_for_leaks(output);
        assert!(
            warnings.is_empty(),
            "False positive on clean output: {:?} -> {:?}",
            output,
            warnings
        );
    }
}

#[test]
fn test_scan_multiple_leaks() {
    let output = "key=sk-aaaaaaaaaaaaaaaaaaaaaaaaa and token Bearer bbbbbbbbbbbbbbbbbbbbbbbb also authrite-cccccccccccccccccc";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 3, "Expected 3 leaks, got {:?}", warnings);

    let patterns: Vec<&str> = warnings.iter().map(|w| w.pattern.as_str()).collect();
    assert!(patterns.contains(&"api_key"));
    assert!(patterns.contains(&"bearer_token"));
    assert!(patterns.contains(&"authrite_token"));
}

// =========================================================================
// Redaction tests
// =========================================================================

#[test]
fn test_redact_replaces_with_tag() {
    let output = "my key is sk-abcdefghijklmnopqrstuvwx done";
    let redacted = redact_leaks(output);
    assert!(
        redacted.contains("[REDACTED:api_key]"),
        "Expected redaction tag, got: {}",
        redacted
    );
    assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwx"));
}

#[test]
fn test_redact_preserves_surrounding_text() {
    let output = "before sk-AAAAAAAAAAAAAAAAAAAAAAAA after";
    let redacted = redact_leaks(output);
    assert!(redacted.starts_with("before "));
    assert!(redacted.ends_with(" after"));
    assert!(redacted.contains("[REDACTED:api_key]"));
}

#[test]
fn test_redact_idempotent() {
    let output = "key: sk-xyzxyzxyzxyzxyzxyzxyzxyz end";
    let once = redact_leaks(output);
    let twice = redact_leaks(&once);
    assert_eq!(once, twice, "Redaction should be idempotent");
}

#[test]
fn test_no_false_positive_short_sk() {
    // "sk-" followed by fewer than 20 characters should NOT be flagged
    let output = "sk-short is not a key";
    let warnings = scan_for_leaks(output);
    assert!(
        warnings.is_empty(),
        "Short sk- prefix should not trigger: {:?}",
        warnings
    );
}

// =========================================================================
// Edge-case and additional coverage tests
// =========================================================================

#[test]
fn test_auth_header_case_insensitive() {
    let output = "X-BSV-AUTH-nonce=value123";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].pattern, "auth_header");
}

#[test]
fn test_bearer_requires_space() {
    // "Bearertoken" without space should not match
    let output = "Beareraaaaaaaaaaaaaaaaaaaaaaaaa";
    let warnings = scan_for_leaks(output);
    assert!(warnings.is_empty(), "Bearer without space should not match");
}

#[test]
fn test_authrite_short_suffix_no_match() {
    // authrite- with fewer than 10 suffix chars should not match
    let output = "authrite-short";
    let warnings = scan_for_leaks(output);
    assert!(
        warnings.is_empty(),
        "Short authrite suffix should not trigger: {:?}",
        warnings
    );
}

#[test]
fn test_redact_multiple_patterns() {
    let output = "a=sk-00000000000000000000000000 b=Bearer cccccccccccccccccccccccc c=x-bsv-auth-nonce=xyz d=authrite-DDDDDDDDDDDDDDDDDD";
    let redacted = redact_leaks(output);
    assert!(redacted.contains("[REDACTED:api_key]"));
    assert!(redacted.contains("[REDACTED:bearer_token]"));
    assert!(redacted.contains("[REDACTED:auth_header]"));
    assert!(redacted.contains("[REDACTED:authrite_token]"));
    // Original secrets should all be gone
    assert!(!redacted.contains("sk-000000000000000000000000"));
    assert!(!redacted.contains("cccccccccccccccccccccccc"));
    assert!(!redacted.contains("x-bsv-auth-nonce"));
}

#[test]
fn test_sk_with_word_boundary() {
    // "task-" followed by "sk-..." should still match (non-alphanumeric before 's')
    let output = "prefix-sk-AAAAAAAAAAAAAAAAAAAAAAAA end";
    let warnings = scan_for_leaks(output);
    assert_eq!(warnings.len(), 1, "sk- after hyphen should match");

    // But "tsk-..." (alphanumeric before 's') should not match
    let output2 = "tsk-AAAAAAAAAAAAAAAAAAAAAAAA end";
    let warnings2 = scan_for_leaks(output2);
    assert!(
        warnings2.is_empty(),
        "sk- preceded by letter should not match: {:?}",
        warnings2
    );
}

#[test]
fn test_redact_clean_is_noop() {
    let output = "Nothing sensitive here at all.";
    let redacted = redact_leaks(output);
    assert_eq!(redacted, output);
}
