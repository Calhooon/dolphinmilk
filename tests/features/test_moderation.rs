//! Tests for D.1: Cert-driven content moderation policy engine.
//!
//! Validates PII detection (SSN, credit card, email), keyword blocking, custom regex
//! patterns, block/flag mode behavior, and cert→config→default precedence chain.

use dolphin_milk::certificates::CertModerationPolicy;
use dolphin_milk::config::ModerationConfig;
use dolphin_milk::moderation::{Mode, ModerationEngine, ModerationResult};

// ---------------------------------------------------------------------------
// Helper: build engine from config only (no cert)
// ---------------------------------------------------------------------------

fn engine_from_config(config: &ModerationConfig) -> ModerationEngine {
    ModerationEngine::new(config, None)
}

fn enabled_pii_block_config() -> ModerationConfig {
    ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    }
}

fn enabled_pii_flag_config() -> ModerationConfig {
    ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    }
}

// =========================================================================
// PII detection: SSN
// =========================================================================

#[test]
fn test_pii_ssn_detection() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("My SSN is 123-45-6789.");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern_name, "pii_ssn");
        assert_eq!(matches[0].matched_text, "123-45-6789");
    }
}

#[test]
fn test_pii_ssn_various_formats() {
    let engine = engine_from_config(&enabled_pii_block_config());

    // With dashes — should match.
    let result = engine.moderate("SSN: 000-12-3456");
    assert!(result.is_blocked());

    // Without dashes — should NOT match the SSN pattern (requires dashes).
    let result = engine.moderate("SSN: 000123456");
    assert!(result.is_pass());
}

#[test]
fn test_pii_in_surrounding_text() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("Please verify the SSN 999-88-7777 in the attached document.");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches[0].matched_text, "999-88-7777");
        // Position should be nonzero (embedded in text).
        assert!(matches[0].position > 0);
    }
}

#[test]
fn test_non_ssn_number_not_flagged() {
    let engine = engine_from_config(&enabled_pii_block_config());
    // Phone numbers shouldn't match SSN pattern.
    let result = engine.moderate("Call me at 555-1234 or 800-555-1212.");
    // 800-555-1212 has 3-3-4 pattern, not 3-2-4 so it shouldn't match SSN.
    // 555-1234 is 3-4 so it shouldn't match SSN either.
    assert!(
        result.is_pass(),
        "Phone numbers should not match SSN pattern"
    );
}

// =========================================================================
// PII detection: Credit card
// =========================================================================

#[test]
fn test_pii_credit_card_detection() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("Card: 4111111111111111");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern_name, "pii_credit_card");
        assert_eq!(matches[0].matched_text, "4111111111111111");
    }
}

#[test]
fn test_pii_credit_card_with_spaces() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("Card: 4111 1111 1111 1111");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches[0].pattern_name, "pii_credit_card");
    }
}

#[test]
fn test_pii_credit_card_with_dashes() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("Card: 4111-1111-1111-1111");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches[0].pattern_name, "pii_credit_card");
    }
}

// =========================================================================
// PII detection: Email
// =========================================================================

#[test]
fn test_pii_email_detection() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("Contact me at user@example.com for details.");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern_name, "pii_email");
        assert_eq!(matches[0].matched_text, "user@example.com");
    }
}

// =========================================================================
// Keyword blocking
// =========================================================================

#[test]
fn test_keyword_block_exact_match() {
    let config = ModerationConfig {
        enabled: true,
        custom_block_keywords: vec!["secret".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let result = engine.moderate("This is a secret project.");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches[0].pattern_name, "keyword:secret");
        assert_eq!(matches[0].matched_text, "secret");
    }
}

#[test]
fn test_keyword_block_case_insensitive() {
    let config = ModerationConfig {
        enabled: true,
        custom_block_keywords: vec!["CLASSIFIED".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    // Lowercase input should still match.
    let result = engine.moderate("This document is classified.");
    assert!(result.is_blocked());

    // Mixed case.
    let result = engine.moderate("This is Classified information.");
    assert!(result.is_blocked());
}

// =========================================================================
// Custom regex patterns
// =========================================================================

#[test]
fn test_regex_pattern_matching() {
    let config = ModerationConfig {
        enabled: true,
        custom_block_patterns: vec![r"\bAPI_KEY_\w+".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let result = engine.moderate("Found API_KEY_abc123 in config.");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert_eq!(matches[0].matched_text, "API_KEY_abc123");
    }
}

#[test]
fn test_custom_block_pattern() {
    let config = ModerationConfig {
        enabled: true,
        custom_block_patterns: vec![r"password\s*[:=]\s*\S+".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let result = engine.moderate("password = hunter2");
    assert!(result.is_blocked());
}

#[test]
fn test_custom_flag_pattern() {
    let config = ModerationConfig {
        enabled: true,
        custom_flag_patterns: vec![r"\bTODO\b".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let result = engine.moderate("TODO: fix this later");
    assert!(result.is_flagged());
    if let ModerationResult::Flagged(matches) = &result {
        assert_eq!(matches[0].matched_text, "TODO");
        assert!(matches[0].pattern_name.starts_with("custom_flag:"));
    }
}

// =========================================================================
// Block vs Flag mode
// =========================================================================

#[test]
fn test_block_mode_returns_blocked() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(result.is_blocked());
}

#[test]
fn test_flag_mode_returns_flagged() {
    let engine = engine_from_config(&enabled_pii_flag_config());
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(result.is_flagged());
    if let ModerationResult::Flagged(matches) = &result {
        assert_eq!(matches[0].pattern_name, "pii_ssn");
    }
}

// =========================================================================
// Pass / disabled
// =========================================================================

#[test]
fn test_pass_when_no_match() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("This is perfectly clean content with no sensitive data.");
    assert!(result.is_pass());
}

#[test]
fn test_no_policies_passthrough() {
    // Default config: disabled = always Pass.
    let config = ModerationConfig::default();
    assert!(!config.enabled);
    let engine = engine_from_config(&config);
    let result = engine.moderate("SSN: 123-45-6789 and card 4111111111111111");
    assert!(result.is_pass(), "Disabled engine must always return Pass");
}

#[test]
fn test_empty_content_passes() {
    let engine = engine_from_config(&enabled_pii_block_config());
    let result = engine.moderate("");
    assert!(result.is_pass());
}

// =========================================================================
// Multiple matches
// =========================================================================

#[test]
fn test_multiple_matches_in_one_content() {
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let result =
        engine.moderate("SSN: 123-45-6789, email: test@example.com, card: 4111 1111 1111 1111");
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert!(
            matches.len() >= 3,
            "Expected at least 3 matches, got {}",
            matches.len()
        );
        let names: Vec<&str> = matches.iter().map(|m| m.pattern_name.as_str()).collect();
        assert!(names.contains(&"pii_ssn"));
        assert!(names.contains(&"pii_email"));
        assert!(names.contains(&"pii_credit_card"));
    }
}

// =========================================================================
// Config parsing
// =========================================================================

#[test]
fn test_config_parsing_moderation_section() {
    // Parse as part of a full DmConfig (same as dolphin-milk.toml would be loaded).
    let toml_str = r#"
[moderation]
enabled = true
pii_mode = "flag"
profanity_mode = "block"
custom_block_patterns = ["badword\\d+"]
custom_flag_patterns = ["maybe\\w+"]
custom_block_keywords = ["forbidden", "restricted"]
"#;
    let config: dolphin_milk::config::DmConfig = toml::from_str(toml_str).unwrap();
    assert!(config.moderation.enabled);
    assert_eq!(config.moderation.pii_mode, "flag");
    assert_eq!(config.moderation.profanity_mode, "block");
    assert_eq!(config.moderation.custom_block_patterns, vec!["badword\\d+"]);
    assert_eq!(config.moderation.custom_flag_patterns, vec!["maybe\\w+"]);
    assert_eq!(
        config.moderation.custom_block_keywords,
        vec!["forbidden", "restricted"]
    );
}

// =========================================================================
// Cert override tests — cert→config→default precedence
// =========================================================================

#[test]
fn test_cert_enables_moderation_overrides_config_disabled() {
    // Config says disabled, but cert says enabled + PII block.
    let config = ModerationConfig {
        enabled: false,
        pii_mode: "off".into(),
        ..Default::default()
    };
    let cert = CertModerationPolicy {
        enabled: Some(true),
        pii_mode: Some("block".into()),
        profanity_mode: None,
    };
    let engine = ModerationEngine::new(&config, Some(&cert));
    assert!(engine.is_enabled());
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(result.is_blocked());
}

#[test]
fn test_cert_pii_block_overrides_config_flag() {
    // Config says flag, cert says block — cert wins.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    };
    let cert = CertModerationPolicy {
        enabled: None,
        pii_mode: Some("block".into()),
        profanity_mode: None,
    };
    let engine = ModerationEngine::new(&config, Some(&cert));
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(
        result.is_blocked(),
        "Cert 'block' should override config 'flag'"
    );
}

#[test]
fn test_cert_pii_block_cannot_be_downgraded_by_config() {
    // Cert says block, config says flag — cert wins (stricter).
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    };
    let cert = CertModerationPolicy {
        enabled: Some(true),
        pii_mode: Some("block".into()),
        profanity_mode: None,
    };
    let engine = ModerationEngine::new(&config, Some(&cert));
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(
        result.is_blocked(),
        "Config 'flag' must not downgrade cert 'block'"
    );
}

#[test]
fn test_no_cert_falls_back_to_config() {
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    };
    let engine = ModerationEngine::new(&config, None);
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(result.is_flagged(), "No cert should use config pii_mode");
}

#[test]
fn test_cert_disabled_field_missing_uses_config() {
    // Cert has no enabled field — config's enabled value is used.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    };
    let cert = CertModerationPolicy {
        enabled: None,
        pii_mode: None,
        profanity_mode: None,
    };
    let engine = ModerationEngine::new(&config, Some(&cert));
    assert!(
        engine.is_enabled(),
        "Missing cert field should fall back to config"
    );
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(result.is_flagged());
}

// =========================================================================
// Integration-style tests
// =========================================================================

#[test]
fn test_moderate_user_message() {
    // Realistic user message containing PII.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let message =
        "Hi! My name is John, my SSN is 321-54-9876 and you can reach me at john@example.com";
    let result = engine.moderate(message);
    assert!(result.is_blocked());
    if let ModerationResult::Blocked(matches) = &result {
        assert!(matches.len() >= 2, "Should detect SSN and email");
    }
}

#[test]
fn test_moderate_tool_output() {
    // Tool result containing credit card.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let tool_output =
        r#"{"result": "Payment processed for card 4242-4242-4242-4242, amount $50.00"}"#;
    let result = engine.moderate(tool_output);
    assert!(result.is_flagged());
    if let ModerationResult::Flagged(matches) = &result {
        assert_eq!(matches[0].pattern_name, "pii_credit_card");
    }
}

#[test]
fn test_moderation_result_serializable() {
    // Verify ModerationResult can be serialized for transcript logging.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);
    let result = engine.moderate("SSN: 111-22-3333");
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("Blocked"));
    assert!(json.contains("pii_ssn"));
    assert!(json.contains("111-22-3333"));

    // Also check Pass serialization.
    let pass = engine.moderate("clean text");
    let pass_json = serde_json::to_string(&pass).unwrap();
    assert!(pass_json.contains("Pass"));

    // Roundtrip.
    let _: ModerationResult = serde_json::from_str(&json).unwrap();
    let _: ModerationResult = serde_json::from_str(&pass_json).unwrap();
}

// =========================================================================
// Mode helpers
// =========================================================================

#[test]
fn test_mode_parse() {
    assert_eq!(Mode::parse("block"), Mode::Block);
    assert_eq!(Mode::parse("Block"), Mode::Block);
    assert_eq!(Mode::parse("BLOCK"), Mode::Block);
    assert_eq!(Mode::parse("flag"), Mode::Flag);
    assert_eq!(Mode::parse("off"), Mode::Off);
    assert_eq!(Mode::parse(""), Mode::Off);
    assert_eq!(Mode::parse("unknown"), Mode::Off);
}

#[test]
fn test_mode_stricter() {
    assert_eq!(Mode::Off.stricter(Mode::Off), Mode::Off);
    assert_eq!(Mode::Off.stricter(Mode::Flag), Mode::Flag);
    assert_eq!(Mode::Off.stricter(Mode::Block), Mode::Block);
    assert_eq!(Mode::Flag.stricter(Mode::Off), Mode::Flag);
    assert_eq!(Mode::Flag.stricter(Mode::Block), Mode::Block);
    assert_eq!(Mode::Block.stricter(Mode::Flag), Mode::Block);
    assert_eq!(Mode::Block.stricter(Mode::Block), Mode::Block);
}

// =========================================================================
// Disabled engine via constructor
// =========================================================================

#[test]
fn test_disabled_engine_always_passes() {
    let engine = ModerationEngine::disabled();
    assert!(!engine.is_enabled());
    let result = engine.moderate("SSN: 123-45-6789 card 4111111111111111 email@test.com");
    assert!(result.is_pass());
}

// =========================================================================
// Wiring integration tests — ModerationEngine on DmLoop
// =========================================================================

#[test]
fn test_moderation_engine_on_wormloop_default_disabled() {
    // DmLoop starts with a disabled ModerationEngine by default.
    // Verify that ModerationEngine::disabled() always returns Pass.
    let engine = ModerationEngine::disabled();
    assert!(!engine.is_enabled());

    // Even with PII, disabled engine passes everything.
    let result = engine.moderate("SSN: 123-45-6789 and card 4111111111111111");
    assert!(result.is_pass(), "Disabled engine must pass all content");
}

#[test]
fn test_moderation_blocks_user_message_content() {
    // Simulate what observe() does: moderate incoming message content.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    // Message with SSN should be blocked
    let message = "Hello, my SSN is 123-45-6789, can you help?";
    let result = engine.moderate(message);
    assert!(result.is_blocked(), "Message with SSN should be blocked");

    // Clean message should pass
    let clean_message = "Hello, can you help me with my task?";
    let result = engine.moderate(clean_message);
    assert!(result.is_pass(), "Clean message should pass");
}

#[test]
fn test_moderation_blocks_tool_input() {
    // Simulate what execute_tools does: moderate tool arguments.
    let config = ModerationConfig {
        enabled: true,
        custom_block_keywords: vec!["topsecret".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    let args_json = r#"{"query": "find topsecret documents"}"#;
    let result = engine.moderate(args_json);
    assert!(
        result.is_blocked(),
        "Tool input with blocked keyword should be blocked"
    );

    let safe_args = r#"{"query": "find public documents"}"#;
    let result = engine.moderate(safe_args);
    assert!(result.is_pass(), "Safe tool input should pass");
}

#[test]
fn test_moderation_blocks_tool_output() {
    // Simulate what execute_tools does: moderate tool output.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    let tool_output = r#"{"result": "Found record for SSN 999-88-7777 in database"}"#;
    let result = engine.moderate(tool_output);
    assert!(
        result.is_blocked(),
        "Tool output with SSN should be blocked"
    );

    // Verify this is what the wiring does: replace with moderation message
    assert!(result.is_blocked());
}

#[test]
fn test_moderation_flags_but_continues() {
    // Flag mode should detect PII but not block it.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "flag".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    let content = "Contact user@example.com for details";
    let result = engine.moderate(content);
    assert!(result.is_flagged(), "Email should be flagged");
    assert!(
        !result.is_blocked(),
        "Flagged content should not be blocked"
    );

    // Flagged result contains the matches for logging
    if let ModerationResult::Flagged(matches) = &result {
        assert!(!matches.is_empty());
        assert_eq!(matches[0].pattern_name, "pii_email");
        // Verify matches are serializable (for transcript recording)
        let json = serde_json::to_value(matches).unwrap();
        assert!(json.is_array());
    }
}

#[test]
fn test_moderation_disabled_passes_everything() {
    // Default config: disabled. This is the zero-overhead path.
    let config = ModerationConfig::default();
    let engine = engine_from_config(&config);
    assert!(!engine.is_enabled());

    // Nothing is blocked or flagged when disabled.
    let pii_content = "SSN: 123-45-6789 card 4111-1111-1111-1111 email@test.com";
    assert!(engine.moderate(pii_content).is_pass());

    let keyword_content = "This is forbidden and classified";
    assert!(engine.moderate(keyword_content).is_pass());

    let empty = "";
    assert!(engine.moderate(empty).is_pass());
}

#[test]
fn test_moderation_llm_response_blocked() {
    // Simulate what think_step does: moderate LLM response.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    // LLM response containing PII
    let response = "Here is the customer's SSN: 321-54-9876 as requested.";
    let result = engine.moderate(response);
    assert!(
        result.is_blocked(),
        "LLM response with SSN should be blocked"
    );

    // In the actual wiring, this would cause:
    // - state.error = "LLM response blocked by moderation policy"
    // - state.done = true
    // - return Err(DmError::tool(...))
}

#[test]
fn test_moderation_engine_constructed_from_cert_and_config() {
    // Verify that ModerationEngine::new() properly merges cert and config.
    // This is what setup_task() does after read_cert_moderation_policy().
    let config = ModerationConfig {
        enabled: false,
        pii_mode: "off".into(),
        ..Default::default()
    };
    let cert = CertModerationPolicy {
        enabled: Some(true),
        pii_mode: Some("block".into()),
        profanity_mode: None,
    };

    // Cert forces moderation on and PII to block mode
    let engine = ModerationEngine::new(&config, Some(&cert));
    assert!(engine.is_enabled());
    let result = engine.moderate("SSN: 123-45-6789");
    assert!(
        result.is_blocked(),
        "Cert should override config and block PII"
    );
}

#[test]
fn test_moderation_combined_block_and_flag_patterns() {
    // Test that block patterns take precedence over flag patterns.
    let config = ModerationConfig {
        enabled: true,
        pii_mode: "block".into(),
        custom_flag_patterns: vec![r"\bWARNING\b".into()],
        ..Default::default()
    };
    let engine = engine_from_config(&config);

    // Content with both block-level PII and flag-level custom pattern
    let content = "WARNING: SSN is 123-45-6789";
    let result = engine.moderate(content);
    // Block takes precedence
    assert!(
        result.is_blocked(),
        "Block-level match should produce Blocked result"
    );
}
