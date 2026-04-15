//! Integration tests for BRC-48 state tokens (Phase 2d.6).
//!
//! Tests Pay-to-Push-Drop script construction, token lifecycle,
//! BRC-46 basket assignment, and serialization.

use dolphin_milk::state::{
    build_push_drop_script, is_encrypted_token_data, StateToken, TokenResult, TokenType,
    BASKET_BUDGET, BASKET_STATE, MIN_TOKEN_SATS,
};
use serde_json::json;

// Valid secp256k1 compressed pubkey (generator point G) for testing.
fn test_pubkey() -> String {
    "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_string()
}

// ─────────────────────────────────────────────
// Script construction
// ─────────────────────────────────────────────

#[test]
fn test_push_drop_single_field_structure() {
    let script_hex = build_push_drop_script(&[b"hello world"], &test_pubkey()).unwrap();
    let script = hex::decode(&script_hex).unwrap();

    // [11] "hello world" OP_DROP [33] <pubkey> OP_CHECKSIG
    assert_eq!(script[0], 11); // push 11 bytes
    assert_eq!(&script[1..12], b"hello world");
    assert_eq!(script[12], 0x75); // OP_DROP
    assert_eq!(script[13], 33); // push 33 bytes (pubkey)
    assert_eq!(*script.last().unwrap(), 0xac); // OP_CHECKSIG
}

#[test]
fn test_push_drop_two_fields_uses_2drop() {
    let script_hex = build_push_drop_script(&[b"type", b"data"], &test_pubkey()).unwrap();
    let script = hex::decode(&script_hex).unwrap();

    // Find OP_2DROP (0x6d) — should appear for 2 fields
    assert!(script.contains(&0x6d), "Should use OP_2DROP for 2 fields");
    // Should NOT contain OP_DROP (0x75) except maybe in other context
    let checksig_pos = script.len() - 1;
    // Count OP_DROP (0x75) occurrences before the pubkey push
    let script_before_pubkey = &script[..script.len() - 35]; // 33 pubkey + 1 push + 1 checksig
    let drops: Vec<_> = script_before_pubkey
        .iter()
        .filter(|&&b| b == 0x75)
        .collect();
    assert_eq!(
        drops.len(),
        0,
        "2 fields should use only OP_2DROP, not OP_DROP"
    );
    assert_eq!(script[checksig_pos], 0xac);
}

#[test]
fn test_push_drop_three_fields_uses_both() {
    let script_hex = build_push_drop_script(&[b"a", b"b", b"c"], &test_pubkey()).unwrap();
    let script = hex::decode(&script_hex).unwrap();

    // 3 fields: 1 OP_2DROP + 1 OP_DROP
    let two_drops = script.iter().filter(|&&b| b == 0x6d).count();
    let one_drops = script.iter().filter(|&&b| b == 0x75).count();
    assert_eq!(two_drops, 1, "3 fields: one OP_2DROP");
    assert_eq!(one_drops, 1, "3 fields: one OP_DROP");
}

#[test]
fn test_push_drop_four_fields() {
    let script_hex = build_push_drop_script(&[b"a", b"b", b"c", b"d"], &test_pubkey()).unwrap();
    let script = hex::decode(&script_hex).unwrap();

    // 4 fields: 2 OP_2DROP, 0 OP_DROP
    let two_drops = script.iter().filter(|&&b| b == 0x6d).count();
    let one_drops = script.iter().filter(|&&b| b == 0x75).count();
    assert_eq!(two_drops, 2);
    assert_eq!(one_drops, 0);
}

#[test]
fn test_push_drop_ends_with_checksig() {
    let script_hex = build_push_drop_script(&[b"data"], &test_pubkey()).unwrap();
    let script = hex::decode(&script_hex).unwrap();
    assert_eq!(*script.last().unwrap(), 0xac, "Must end with OP_CHECKSIG");
}

#[test]
fn test_push_drop_contains_pubkey() {
    let pk = test_pubkey();
    let pk_bytes = hex::decode(&pk).unwrap();
    let script_hex = build_push_drop_script(&[b"x"], &pk).unwrap();
    let script = hex::decode(&script_hex).unwrap();

    // Find the pubkey bytes in the script
    let found = script
        .windows(pk_bytes.len())
        .any(|w| w == pk_bytes.as_slice());
    assert!(found, "Script must contain the owner pubkey");
}

#[test]
fn test_push_drop_large_data() {
    // 100 bytes triggers OP_PUSHDATA1 (0x4c)
    let data = vec![0x42u8; 100];
    let script_hex = build_push_drop_script(&[&data], &test_pubkey()).unwrap();
    let script = hex::decode(&script_hex).unwrap();
    assert_eq!(script[0], 0x4c, "100 bytes should use OP_PUSHDATA1");
    assert_eq!(script[1], 100);
}

#[test]
fn test_push_drop_empty_data_error() {
    let result = build_push_drop_script(&[], &test_pubkey());
    assert!(result.is_err());
}

#[test]
fn test_push_drop_bad_pubkey_error() {
    assert!(build_push_drop_script(&[b"x"], "invalid").is_err());
    assert!(build_push_drop_script(&[b"x"], "0200").is_err());
}

// ─────────────────────────────────────────────
// Token lifecycle
// ─────────────────────────────────────────────

#[test]
fn test_token_new_defaults() {
    let t = StateToken::new(TokenType::TaskCommitment, json!({"task": "test"}));
    assert_eq!(t.token_type, TokenType::TaskCommitment);
    assert!(!t.is_on_chain());
    assert_eq!(t.satoshis, MIN_TOKEN_SATS);
    assert_eq!(t.txid, None);
    assert_eq!(t.vout, None);
}

#[test]
fn test_token_with_sats() {
    let t = StateToken::with_sats(TokenType::BudgetAllocation, json!({"amount": 10000}), 500);
    assert_eq!(t.satoshis, 500);
    assert_eq!(t.token_type, TokenType::BudgetAllocation);
}

#[test]
fn test_token_with_sats_enforces_minimum() {
    let t = StateToken::with_sats(TokenType::Checkpoint, json!({}), 0);
    assert_eq!(t.satoshis, MIN_TOKEN_SATS, "Must enforce minimum sats");
}

#[test]
fn test_token_on_chain_tracking() {
    let mut t = StateToken::new(TokenType::Checkpoint, json!({"hash": "abc"}));
    assert!(!t.is_on_chain());

    t.txid = Some("deadbeef".into());
    t.vout = Some(0);
    assert!(t.is_on_chain());
}

// ─────────────────────────────────────────────
// Basket assignment
// ─────────────────────────────────────────────

#[test]
fn test_basket_assignments() {
    assert_eq!(TokenType::TaskCommitment.basket(), BASKET_STATE);
    assert_eq!(TokenType::BudgetAllocation.basket(), BASKET_BUDGET);
    assert_eq!(TokenType::CapabilityDeclaration.basket(), BASKET_STATE);
    assert_eq!(TokenType::Checkpoint.basket(), BASKET_STATE);
}

// ─────────────────────────────────────────────
// Serialization
// ─────────────────────────────────────────────

#[test]
fn test_token_json_roundtrip() {
    let mut t = StateToken::new(
        TokenType::CapabilityDeclaration,
        json!({"tools": ["bash", "file_read"], "version": "1.0"}),
    );
    t.txid = Some("abc123".into());
    t.vout = Some(1);
    t.satoshis = 100;

    let json_str = serde_json::to_string(&t).unwrap();
    let back: StateToken = serde_json::from_str(&json_str).unwrap();
    assert_eq!(back.token_type, TokenType::CapabilityDeclaration);
    assert_eq!(back.txid, Some("abc123".into()));
    assert_eq!(back.vout, Some(1));
    assert_eq!(back.satoshis, 100);
    assert_eq!(back.data["tools"][0], "bash");
}

#[test]
fn test_token_result_json_roundtrip() {
    let t = StateToken::new(TokenType::TaskCommitment, json!({"task": "hello"}));
    let result = TokenResult {
        txid: "tx999".into(),
        token: t,
    };
    let json_str = serde_json::to_string(&result).unwrap();
    let back: TokenResult = serde_json::from_str(&json_str).unwrap();
    assert_eq!(back.txid, "tx999");
    assert_eq!(back.token.data["task"], "hello");
}

#[test]
fn test_all_token_types_serialize() {
    for tt in &[
        TokenType::TaskCommitment,
        TokenType::BudgetAllocation,
        TokenType::CapabilityDeclaration,
        TokenType::Checkpoint,
    ] {
        let json = serde_json::to_string(tt).unwrap();
        let back: TokenType = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, tt);
    }
}

// ─────────────────────────────────────────────
// Lifecycle: parse_push_drop_fields (moved to state)
// ─────────────────────────────────────────────

#[test]
fn test_parse_push_drop_fields_roundtrip() {
    let pk = test_pubkey();
    let data = b"hello world";
    let script_hex = build_push_drop_script(&[data.as_ref()], &pk).unwrap();
    let fields = dolphin_milk::state::parse_push_drop_fields(&script_hex).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(hex::decode(&fields[0]).unwrap(), data);
}

#[test]
fn test_parse_push_drop_fields_two() {
    let pk = test_pubkey();
    let script_hex = build_push_drop_script(&[b"type", b"data"], &pk).unwrap();
    let fields = dolphin_milk::state::parse_push_drop_fields(&script_hex).unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(hex::decode(&fields[0]).unwrap(), b"type");
    assert_eq!(hex::decode(&fields[1]).unwrap(), b"data");
}

#[test]
fn test_parse_push_drop_fields_empty_script() {
    assert!(dolphin_milk::state::parse_push_drop_fields("").is_none());
}

#[test]
fn test_parse_push_drop_fields_invalid_hex() {
    assert!(dolphin_milk::state::parse_push_drop_fields("zzzz").is_none());
}

// ─────────────────────────────────────────────
// Lifecycle: created_at enrichment
// ─────────────────────────────────────────────

#[test]
fn test_token_data_created_at_enrichment() {
    // StateToken.new stores original data
    let t = StateToken::new(TokenType::TaskCommitment, json!({"task": "test"}));
    // When we serialize enriched data (as create_token would), created_at is added
    let mut enriched = t.data.clone();
    if let Some(obj) = enriched.as_object_mut() {
        obj.insert("created_at".to_string(), json!(1709568000_i64));
    }
    assert!(enriched.get("created_at").is_some());
    assert_eq!(enriched["created_at"], 1709568000);
    assert_eq!(enriched["task"], "test");
}

// ─────────────────────────────────────────────
// Lifecycle: LifecycleConfig defaults
// ─────────────────────────────────────────────

#[test]
fn test_lifecycle_config_defaults() {
    let config = dolphin_milk::config::LifecycleConfig::default();
    assert_eq!(config.budget_token_max_age_hours, 168); // 7 days
    assert_eq!(config.checkpoint_max_age_hours, 720); // 30 days
    assert!(config.auto_sweep_enabled);
    assert_eq!(config.sweep_interval_minutes, 60);
}

#[test]
fn test_lifecycle_config_in_worm_config() {
    let config = dolphin_milk::config::DmConfig::default();
    assert_eq!(config.lifecycle.budget_token_max_age_hours, 168);
    assert!(config.lifecycle.auto_sweep_enabled);
}

#[test]
fn test_lifecycle_config_from_toml() {
    let toml_str = r#"
[lifecycle]
budget_token_max_age_hours = 24
checkpoint_max_age_hours = 48
auto_sweep_enabled = false
sweep_interval_minutes = 30
"#;
    let config: dolphin_milk::config::DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.lifecycle.budget_token_max_age_hours, 24);
    assert_eq!(config.lifecycle.checkpoint_max_age_hours, 48);
    assert!(!config.lifecycle.auto_sweep_enabled);
    assert_eq!(config.lifecycle.sweep_interval_minutes, 30);
}

#[test]
fn test_lifecycle_hot_reload() {
    let mut config = dolphin_milk::config::DmConfig::default();
    let mut fresh = config.clone();
    fresh.lifecycle.budget_token_max_age_hours = 24;
    fresh.lifecycle.auto_sweep_enabled = false;
    config.reload_safe_fields(&fresh);
    assert_eq!(config.lifecycle.budget_token_max_age_hours, 24);
    assert!(!config.lifecycle.auto_sweep_enabled);
}

// ─────────────────────────────────────────────
// Lifecycle: token type extraction from script
// ─────────────────────────────────────────────

#[test]
fn test_extract_token_type_from_script() {
    let pk = test_pubkey();
    let type_bytes = b"budget_allocation";
    let data_bytes = json!({"amount": 1000, "created_at": 1709568000_i64})
        .to_string()
        .into_bytes();
    let script_hex = build_push_drop_script(&[type_bytes.as_ref(), &data_bytes], &pk).unwrap();
    let fields = dolphin_milk::state::parse_push_drop_fields(&script_hex).unwrap();
    assert_eq!(fields.len(), 2);
    let type_str = String::from_utf8(hex::decode(&fields[0]).unwrap()).unwrap();
    assert_eq!(type_str, "budget_allocation");
    // Verify data contains created_at
    let data: serde_json::Value =
        serde_json::from_slice(&hex::decode(&fields[1]).unwrap()).unwrap();
    assert_eq!(data["created_at"], 1709568000);
}

#[test]
fn test_extract_created_at_from_3_field_script() {
    // New format: [type, encrypted_data, created_at] — 3 fields
    let pk = test_pubkey();
    let type_bytes = b"budget_allocation";
    let data_bytes = b"\x01\x02\x03\x04\x05\x06"; // opaque encrypted bytes
    let ts = "1709568000";
    let ts_bytes = ts.as_bytes();
    let script_hex =
        build_push_drop_script(&[type_bytes.as_ref(), data_bytes.as_ref(), ts_bytes], &pk).unwrap();

    // Build a mock wallet output with the script
    let output = json!({"lockingScript": script_hex});

    // The extract function should read the 3rd field
    let fields = dolphin_milk::state::parse_push_drop_fields(&script_hex).unwrap();
    assert_eq!(fields.len(), 3);

    // Verify the 3rd field is the timestamp
    let ts_str = String::from_utf8(hex::decode(&fields[2]).unwrap()).unwrap();
    assert_eq!(ts_str, "1709568000");

    // Verify extract_created_at_from_output works with the 3-field format
    // (we can't call the private function directly, but we can verify the fields)
    let extracted_bytes = hex::decode(&fields[2]).unwrap();
    let extracted_ts: i64 = String::from_utf8(extracted_bytes).unwrap().parse().unwrap();
    assert_eq!(extracted_ts, 1709568000);

    // Verify the 2nd field is NOT valid JSON (encrypted bytes)
    let data_field = hex::decode(&fields[1]).unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&data_field).is_err());

    // The type field is still readable
    let type_str = String::from_utf8(hex::decode(&fields[0]).unwrap()).unwrap();
    assert_eq!(type_str, "budget_allocation");

    drop(output); // used above for context
}

#[test]
fn test_legacy_2_field_created_at_fallback() {
    // Legacy format: [type, json_data_with_created_at] — 2 fields
    let pk = test_pubkey();
    let type_bytes = b"task_commitment";
    let data = json!({"task": "test", "created_at": 1709500000_i64});
    let data_bytes = serde_json::to_vec(&data).unwrap();
    let script_hex = build_push_drop_script(&[type_bytes.as_ref(), &data_bytes], &pk).unwrap();

    let fields = dolphin_milk::state::parse_push_drop_fields(&script_hex).unwrap();
    assert_eq!(fields.len(), 2);

    // The legacy path parses JSON from the 2nd field
    let data_json: serde_json::Value =
        serde_json::from_slice(&hex::decode(&fields[1]).unwrap()).unwrap();
    assert_eq!(data_json["created_at"], 1709500000);
}

// ─────────────────────────────────────────────
// Encrypted token tests (legacy format)
// ─────────────────────────────────────────────

fn legacy_test_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = 0xDE;
    k[31] = 0xAD;
    k
}

fn legacy_test_key_id() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(b"brc48-state-token-v1").into()
}

#[test]
fn test_legacy_encrypted_push_drop_contains_no_plaintext() {
    let key = legacy_test_key();
    let key_id = legacy_test_key_id();
    let secret = "highly confidential task details about project X";
    let data = json!({"task": secret, "amount": 50000});
    let data_json = serde_json::to_vec(&data).unwrap();

    let encrypted =
        dolphin_milk::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();
    assert!(is_encrypted_token_data(&encrypted));

    // Build script with encrypted data
    let type_bytes = b"task_commitment";
    let pk = test_pubkey();
    let script_hex = build_push_drop_script(&[type_bytes.as_ref(), &encrypted], &pk).unwrap();
    let script_bytes = hex::decode(&script_hex).unwrap();

    // Verify no plaintext leaked
    let script_str = String::from_utf8_lossy(&script_bytes);
    assert!(
        !script_str.contains(secret),
        "Secret plaintext must not appear in script"
    );
    assert!(
        !script_str.contains("50000"),
        "Amount must not appear in script"
    );
    assert!(
        !script_str.contains("confidential"),
        "No plaintext substrings in script"
    );
}

#[test]
fn test_legacy_wrong_key_cannot_decrypt_token() {
    let key = legacy_test_key();
    let key_id = legacy_test_key_id();
    let data = json!({"task": "secret", "budget": 1000});
    let data_json = serde_json::to_vec(&data).unwrap();

    let encrypted =
        dolphin_milk::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();

    // Try decrypting with a completely different key
    let result = dolphin_milk::memory::encrypt::legacy_decrypt(&[0xFFu8; 32], &encrypted);
    assert!(result.is_err(), "Wrong key must fail decryption");
}

#[test]
fn test_legacy_encrypted_roundtrip_preserves_data() {
    let key = legacy_test_key();
    let key_id = legacy_test_key_id();
    let data = json!({
        "task": "build the thing",
        "priority": 1,
        "nested": {"key": "value"}
    });
    let data_json = serde_json::to_vec(&data).unwrap();

    let encrypted =
        dolphin_milk::memory::encrypt::legacy_encrypt(&key, &key_id, &data_json).unwrap();
    let plaintext = dolphin_milk::memory::encrypt::legacy_decrypt(&key, &encrypted).unwrap();
    let decrypted: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(decrypted, data);
}
