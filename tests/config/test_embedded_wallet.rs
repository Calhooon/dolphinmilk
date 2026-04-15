//! Tests for EmbeddedWalletClient — in-process wallet via bsv-wallet-toolbox.
//!
//! These tests exercise construction, key derivation, cryptographic operations,
//! and error handling without requiring a running wallet process.
//!
//! Only compiled when `--features embedded-wallet` is active.
#![cfg(feature = "embedded-wallet")]

use bsv_wallet_toolbox_rs::Chain;
use dolphin_milk::wallet::{EmbeddedWalletClient, ANYONE_KEY};
use serde_json::json;
use tempfile::TempDir;

/// Private key = 1 (secp256k1 generator point G). Well-known test key.
const TEST_ROOT_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";

/// Expected identity public key for private key = 1.
const TEST_IDENTITY_KEY: &str =
    "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

/// Standard BRC-42 protocol ID used in most tests.
/// Note: The SDK validates protocol names -- they must NOT end with " protocol".
fn test_protocol_id() -> serde_json::Value {
    json!([2, "worm test"])
}

/// Create a fresh EmbeddedWalletClient in a temp directory.
/// Returns (client, _dir) -- keep `_dir` alive to prevent cleanup.
async fn make_wallet() -> (EmbeddedWalletClient, TempDir) {
    let dir = TempDir::new().expect("create tempdir");
    let db_path = dir.path().join("test.sqlite");
    let client = EmbeddedWalletClient::init(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Main)
        .await
        .expect("init embedded wallet");
    (client, dir)
}

// ── Construction tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_init_creates_wallet() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("init.sqlite");
    let result =
        EmbeddedWalletClient::init(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Main).await;
    assert!(result.is_ok(), "init should succeed: {:?}", result.err());
}

#[tokio::test]
async fn test_open_existing_db() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("open.sqlite");

    // First init to create the DB.
    let _client = EmbeddedWalletClient::init(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Main)
        .await
        .expect("init");

    // Then open the existing DB.
    let result =
        EmbeddedWalletClient::open(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Main).await;
    assert!(result.is_ok(), "open should succeed: {:?}", result.err());
}

#[tokio::test]
async fn test_init_invalid_root_key() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("bad.sqlite");
    let result = EmbeddedWalletClient::init(
        db_path.to_str().unwrap(),
        "not_a_valid_hex_key",
        Chain::Main,
    )
    .await;
    assert!(result.is_err());
    let err = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(
        err.contains("invalid root key"),
        "error should mention invalid root key: {err}"
    );
}

#[tokio::test]
async fn test_open_nonexistent_db() {
    // Opening a DB path that does not exist — StorageSqlx::open may create it
    // or fail. Either way, the function should not panic.
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("nonexistent_subdir/wallet.sqlite");
    let result =
        EmbeddedWalletClient::open(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Main).await;
    // We don't assert success or failure — just that it doesn't panic.
    // The result depends on whether SQLite creates intermediate dirs.
    let _ = result;
}

#[tokio::test]
async fn test_init_with_testnet_chain() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("testnet.sqlite");
    let result =
        EmbeddedWalletClient::init(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Test).await;
    assert!(
        result.is_ok(),
        "testnet init should succeed: {:?}",
        result.err()
    );
}

// ── Identity / key management ───────────────────────────────────────

#[tokio::test]
async fn test_get_identity_key() {
    let (client, _dir) = make_wallet().await;
    let key = client.get_identity_key().await.expect("get_identity_key");
    assert_eq!(key, TEST_IDENTITY_KEY);
    assert_eq!(key.len(), 66); // 33 bytes compressed = 66 hex chars
    assert!(key.starts_with("02") || key.starts_with("03"));
}

#[tokio::test]
async fn test_get_public_key_derives_key() {
    let (client, _dir) = make_wallet().await;
    let key = client
        .get_public_key(&test_protocol_id(), "test-key", "self", false)
        .await
        .expect("get_public_key");
    // Derived key should be a valid compressed public key.
    assert_eq!(key.len(), 66);
    assert!(key.starts_with("02") || key.starts_with("03"));
    // Derived key differs from identity key (different derivation path).
    assert_ne!(key, TEST_IDENTITY_KEY);
}

#[tokio::test]
async fn test_get_public_key_for_self_flag() {
    let (client, _dir) = make_wallet().await;
    // Both for_self=true and for_self=false should succeed and return valid keys.
    let key_true = client
        .get_public_key(&test_protocol_id(), "my-key", "self", true)
        .await
        .expect("get_public_key for_self=true");
    let key_false = client
        .get_public_key(&test_protocol_id(), "my-key", "self", false)
        .await
        .expect("get_public_key for_self=false");
    // Both should be valid compressed public keys.
    assert_eq!(key_true.len(), 66);
    assert_eq!(key_false.len(), 66);
    assert!(key_true.starts_with("02") || key_true.starts_with("03"));
    assert!(key_false.starts_with("02") || key_false.starts_with("03"));
}

#[tokio::test]
async fn test_get_public_key_different_key_ids() {
    let (client, _dir) = make_wallet().await;
    let key_a = client
        .get_public_key(&test_protocol_id(), "key-a", "self", false)
        .await
        .expect("key-a");
    let key_b = client
        .get_public_key(&test_protocol_id(), "key-b", "self", false)
        .await
        .expect("key-b");
    assert_ne!(
        key_a, key_b,
        "different key_ids should derive different keys"
    );
}

#[tokio::test]
async fn test_get_public_key_counterparty_anyone() {
    let (client, _dir) = make_wallet().await;
    let key = client
        .get_public_key(&test_protocol_id(), "test-key", "anyone", false)
        .await
        .expect("counterparty anyone");
    assert_eq!(key.len(), 66);
}

#[tokio::test]
async fn test_get_public_key_counterparty_hex_key() {
    let (client, _dir) = make_wallet().await;
    // Use the ANYONE_KEY as a counterparty hex key.
    let key = client
        .get_public_key(&test_protocol_id(), "test-key", ANYONE_KEY, false)
        .await
        .expect("counterparty hex key");
    assert_eq!(key.len(), 66);
}

// ── Status / metadata ───────────────────────────────────────────────

#[tokio::test]
async fn test_is_authenticated() {
    let (client, _dir) = make_wallet().await;
    let result = client.is_authenticated().await.expect("is_authenticated");
    let auth = result["authenticated"].as_bool().unwrap_or(false);
    assert!(auth, "embedded wallet should always be authenticated");
}

#[tokio::test]
async fn test_get_version() {
    let (client, _dir) = make_wallet().await;
    let version = client.get_version().await.expect("get_version");
    // Version should be a non-empty string.
    assert!(!version.is_empty(), "version should not be empty");
}

#[tokio::test]
async fn test_get_network_mainnet() {
    let (client, _dir) = make_wallet().await;
    let network = client.get_network().await.expect("get_network");
    assert_eq!(network, "mainnet");
}

#[tokio::test]
async fn test_get_network_testnet() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("testnet.sqlite");
    let client = EmbeddedWalletClient::init(db_path.to_str().unwrap(), TEST_ROOT_KEY, Chain::Test)
        .await
        .expect("init testnet wallet");
    let network = client.get_network().await.expect("get_network");
    assert_eq!(network, "testnet");
}

// ── raw_call routing ────────────────────────────────────────────────

#[tokio::test]
async fn test_raw_call_is_authenticated() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .raw_call("isAuthenticated", None)
        .await
        .expect("raw_call isAuthenticated");
    assert!(result["authenticated"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn test_raw_call_get_version() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .raw_call("getVersion", None)
        .await
        .expect("raw_call getVersion");
    assert!(result["version"].is_string());
}

#[tokio::test]
async fn test_raw_call_get_network() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .raw_call("getNetwork", None)
        .await
        .expect("raw_call getNetwork");
    assert_eq!(result["network"].as_str().unwrap_or(""), "mainnet");
}

#[tokio::test]
async fn test_raw_call_get_public_key_identity() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .raw_call("getPublicKey", Some(json!({"identityKey": true})))
        .await
        .expect("raw_call getPublicKey identity");
    assert_eq!(
        result["publicKey"].as_str().unwrap_or(""),
        TEST_IDENTITY_KEY
    );
}

#[tokio::test]
async fn test_raw_call_get_public_key_derived() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .raw_call(
            "getPublicKey",
            Some(json!({
                "protocolID": [2, "worm test"],
                "keyID": "derived-key",
                "counterparty": "self",
                "forSelf": false
            })),
        )
        .await
        .expect("raw_call getPublicKey derived");
    let key = result["publicKey"].as_str().unwrap_or("");
    assert_eq!(key.len(), 66);
}

#[tokio::test]
async fn test_raw_call_unsupported_method() {
    let (client, _dir) = make_wallet().await;
    let result = client.raw_call("noSuchMethod", None).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("unsupported method"), "error: {err}");
}

// ── Cryptographic operations (signature roundtrip) ──────────────────

#[tokio::test]
async fn test_create_and_verify_signature() {
    let (client, _dir) = make_wallet().await;
    let data = b"hello world";
    let protocol_id = test_protocol_id();

    let sig = client
        .create_signature(data, &protocol_id, "sig-key", "self")
        .await
        .expect("create_signature");
    assert!(!sig.is_empty(), "signature should not be empty");

    let valid = client
        .verify_signature(data, &sig, &protocol_id, "sig-key", "self")
        .await
        .expect("verify_signature");
    assert!(valid, "signature should be valid");
}

#[tokio::test]
async fn test_signature_wrong_data_fails_verify() {
    let (client, _dir) = make_wallet().await;
    let protocol_id = test_protocol_id();

    let sig = client
        .create_signature(b"original", &protocol_id, "sig-key", "self")
        .await
        .expect("create_signature");

    // The toolbox may return Ok(false) or an error for invalid signatures.
    let result = client
        .verify_signature(b"tampered", &sig, &protocol_id, "sig-key", "self")
        .await;
    if let Ok(valid) = result {
        assert!(!valid, "signature for different data should be invalid");
    } // error is also acceptable -- means verification rejected it
}

// ── Cryptographic operations (encrypt/decrypt roundtrip) ────────────

#[tokio::test]
async fn test_encrypt_and_decrypt_roundtrip() {
    let (client, _dir) = make_wallet().await;
    let plaintext = b"secret message";
    let protocol_id = test_protocol_id();

    let ciphertext = client
        .encrypt(plaintext, &protocol_id, "enc-key", "self")
        .await
        .expect("encrypt");
    assert!(!ciphertext.is_empty());
    assert_ne!(
        ciphertext, plaintext,
        "ciphertext should differ from plaintext"
    );

    let decrypted = client
        .decrypt(&ciphertext, &protocol_id, "enc-key", "self")
        .await
        .expect("decrypt");
    assert_eq!(decrypted, plaintext);
}

#[tokio::test]
async fn test_encrypt_empty_data() {
    let (client, _dir) = make_wallet().await;
    let protocol_id = test_protocol_id();

    let ciphertext = client
        .encrypt(b"", &protocol_id, "enc-key", "self")
        .await
        .expect("encrypt empty");
    let decrypted = client
        .decrypt(&ciphertext, &protocol_id, "enc-key", "self")
        .await
        .expect("decrypt empty");
    assert_eq!(decrypted, b"");
}

// ── Cryptographic operations (HMAC roundtrip) ───────────────────────

#[tokio::test]
async fn test_create_and_verify_hmac() {
    let (client, _dir) = make_wallet().await;
    let data = b"hmac me";
    let protocol_id = test_protocol_id();

    let hmac = client
        .create_hmac(data, &protocol_id, "hmac-key", "self")
        .await
        .expect("create_hmac");
    assert_eq!(hmac.len(), 32, "HMAC should be exactly 32 bytes");

    let valid = client
        .verify_hmac(data, &hmac, &protocol_id, "hmac-key", "self")
        .await
        .expect("verify_hmac");
    assert!(valid, "HMAC should verify");
}

#[tokio::test]
async fn test_hmac_wrong_data_fails_verify() {
    let (client, _dir) = make_wallet().await;
    let protocol_id = test_protocol_id();

    let hmac = client
        .create_hmac(b"original", &protocol_id, "hmac-key", "self")
        .await
        .expect("create_hmac");

    // The toolbox may return Ok(false) or an error for invalid HMACs.
    let result = client
        .verify_hmac(b"tampered", &hmac, &protocol_id, "hmac-key", "self")
        .await;
    if let Ok(valid) = result {
        assert!(!valid, "HMAC for different data should fail");
    } // error is also acceptable -- means verification rejected it
}

#[tokio::test]
async fn test_hmac_deterministic() {
    let (client, _dir) = make_wallet().await;
    let protocol_id = test_protocol_id();
    let data = b"deterministic";

    let hmac1 = client
        .create_hmac(data, &protocol_id, "hmac-key", "self")
        .await
        .expect("hmac1");
    let hmac2 = client
        .create_hmac(data, &protocol_id, "hmac-key", "self")
        .await
        .expect("hmac2");
    assert_eq!(hmac1, hmac2, "same inputs should produce same HMAC");
}

// ── Output / balance (empty wallet) ─────────────────────────────────

#[tokio::test]
async fn test_get_balance_empty_wallet() {
    let (client, _dir) = make_wallet().await;
    let balance = client.get_balance().await.expect("get_balance");
    assert_eq!(balance, 0, "fresh wallet should have zero balance");
}

#[tokio::test]
async fn test_list_outputs_empty_wallet() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .list_outputs("default", "locking scripts", 100, 0)
        .await
        .expect("list_outputs");
    // Result should have an outputs array, possibly empty.
    let outputs = result["outputs"].as_array();
    assert!(
        outputs.is_none_or(|a| a.is_empty()),
        "fresh wallet should have no outputs"
    );
}

#[tokio::test]
async fn test_list_actions_empty_wallet() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .list_actions(&[], false, false, false, 100, 0)
        .await
        .expect("list_actions");
    // Should have a totalActions field or empty actions array.
    let total = result["totalActions"].as_u64().unwrap_or(0);
    assert_eq!(total, 0, "fresh wallet should have no actions");
}

#[tokio::test]
async fn test_list_certificates_empty_wallet() {
    let (client, _dir) = make_wallet().await;
    let result = client
        .list_certificates(&[], &[], 100, 0)
        .await
        .expect("list_certificates");
    // Should return successfully with empty or zero total.
    let total = result["totalCertificates"].as_u64().unwrap_or(0);
    assert_eq!(total, 0, "fresh wallet should have no certificates");
}

// ── Error handling (protocol_id validation) ─────────────────────────

#[tokio::test]
async fn test_invalid_protocol_id_not_array() {
    let (client, _dir) = make_wallet().await;
    let bad = json!("not an array");
    let result = client.get_public_key(&bad, "key", "self", false).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("protocol_id must be a JSON array"),
        "error: {err}"
    );
}

#[tokio::test]
async fn test_invalid_protocol_id_missing_level() {
    let (client, _dir) = make_wallet().await;
    let bad = json!(["not_a_number", "protocol"]);
    let result = client.get_public_key(&bad, "key", "self", false).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("protocol_id[0] must be an integer"),
        "error: {err}"
    );
}

#[tokio::test]
async fn test_invalid_protocol_id_missing_name() {
    let (client, _dir) = make_wallet().await;
    let bad = json!([2]);
    let result = client.get_public_key(&bad, "key", "self", false).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("protocol_id[1] must be a string"),
        "error: {err}"
    );
}

#[tokio::test]
async fn test_invalid_security_level() {
    let (client, _dir) = make_wallet().await;
    let bad = json!([99, "invalid level"]);
    let result = client.get_public_key(&bad, "key", "self", false).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid security level"), "error: {err}");
}

#[tokio::test]
async fn test_invalid_counterparty_key() {
    let (client, _dir) = make_wallet().await;
    let protocol_id = test_protocol_id();
    let result = client
        .get_public_key(&protocol_id, "key", "not_a_valid_key", false)
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid counterparty key"), "error: {err}");
}

#[tokio::test]
async fn test_verify_hmac_wrong_length() {
    let (client, _dir) = make_wallet().await;
    let protocol_id = test_protocol_id();
    let bad_hmac = vec![0u8; 16]; // 16 bytes instead of 32
    let result = client
        .verify_hmac(b"data", &bad_hmac, &protocol_id, "hmac-key", "self")
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("must be exactly 32 bytes"), "error: {err}");
}

// ── Clone behavior ──────────────────────────────────────────────────

#[tokio::test]
async fn test_clone_shares_state() {
    let (client, _dir) = make_wallet().await;
    let cloned = client.clone();

    // Both should return the same identity key.
    let key1 = client.get_identity_key().await.expect("key1");
    let key2 = cloned.get_identity_key().await.expect("key2");
    assert_eq!(key1, key2, "clone should share the same wallet");
}

// ── Security level variants ─────────────────────────────────────────

#[tokio::test]
async fn test_security_level_silent() {
    let (client, _dir) = make_wallet().await;
    let protocol = json!([0, "worm silent"]);
    let key = client
        .get_public_key(&protocol, "key", "self", false)
        .await
        .expect("silent level");
    assert_eq!(key.len(), 66);
}

#[tokio::test]
async fn test_security_level_app() {
    let (client, _dir) = make_wallet().await;
    let protocol = json!([1, "worm app"]);
    let key = client
        .get_public_key(&protocol, "key", "self", false)
        .await
        .expect("app level");
    assert_eq!(key.len(), 66);
}

#[tokio::test]
async fn test_security_level_counterparty() {
    let (client, _dir) = make_wallet().await;
    let protocol = json!([2, "worm counterparty"]);
    let key = client
        .get_public_key(&protocol, "key", "self", false)
        .await
        .expect("counterparty level");
    assert_eq!(key.len(), 66);
}
