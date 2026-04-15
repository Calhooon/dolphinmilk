//! Tests for wallet module — WalletClient construction and helpers.
//! Note: Most wallet methods require a live wallet. These tests cover
//! construction, configuration, and the ANYONE_KEY constant.

use dolphin_milk::config::WalletConfig;
use dolphin_milk::wallet::{WalletClient, ANYONE_KEY};

#[test]
fn test_anyone_key() {
    // ANYONE_KEY should be the secp256k1 generator point G (compressed)
    assert_eq!(ANYONE_KEY.len(), 66); // 33 bytes = 66 hex chars
    assert!(ANYONE_KEY.starts_with("02")); // compressed point starts with 02 or 03
    assert_eq!(
        ANYONE_KEY,
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    );
}

#[test]
fn test_wallet_client_new() {
    let client = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    assert_eq!(client.url, "http://localhost:3322");
    assert_eq!(client.origin, "http://localhost");
    assert_eq!(client.timeout.as_secs(), 30);
}

#[test]
fn test_wallet_client_strips_trailing_slash() {
    let client = WalletClient::new("http://localhost:3322/", "http://localhost", 30);
    assert_eq!(client.url, "http://localhost:3322");
}

#[test]
fn test_wallet_client_from_config() {
    let cfg = WalletConfig::default();
    let client = WalletClient::from_config(&cfg);
    assert_eq!(client.url, "http://localhost:3322");
    assert_eq!(client.origin, "http://localhost");
    assert_eq!(client.timeout.as_secs(), 120);
}

#[test]
fn test_wallet_client_clone() {
    let client = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let cloned = client.clone();
    assert_eq!(cloned.url, client.url);
    assert_eq!(cloned.origin, client.origin);
}

#[test]
fn test_wallet_client_custom_port() {
    let client = WalletClient::new("http://localhost:9999", "http://myapp", 60);
    assert_eq!(client.url, "http://localhost:9999");
    assert_eq!(client.origin, "http://myapp");
    assert_eq!(client.timeout.as_secs(), 60);
}
