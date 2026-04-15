//! Tests for x402 — payment construction, BEEF encoding, refund parsing.
//! Tests the pure functions that don't require a live wallet.

use reqwest::header::HeaderMap;
use serde_json::json;

use dolphin_milk::x402::payment::{
    build_p2pkh_script, hash160, parse_402_response, payment_protocol, raw_tx_to_atomic_beef,
    raw_tx_to_beef,
};
use dolphin_milk::x402::refund::parse_refund;

// -- Payment tests --

#[test]
fn test_payment_protocol() {
    let proto = payment_protocol();
    assert!(proto.is_array());
    let arr = proto.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0], 2);
    assert_eq!(arr[1], "3241645161d8");
}

#[test]
fn test_hash160() {
    // hash160 of a known value — RIPEMD160(SHA256(data))
    let data = b"hello";
    let result = hash160(data);
    assert_eq!(result.len(), 20); // RIPEMD160 output is 20 bytes
                                  // Verify it's not all zeros
    assert!(result.iter().any(|&b| b != 0));
}

#[test]
fn test_hash160_deterministic() {
    let data = b"test data";
    let r1 = hash160(data);
    let r2 = hash160(data);
    assert_eq!(r1, r2);
}

#[test]
fn test_build_p2pkh_script_valid() {
    // Use a valid compressed public key (33 bytes = 66 hex chars)
    let pubkey = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let script = build_p2pkh_script(pubkey).unwrap();

    // P2PKH script: OP_DUP(76) OP_HASH160(a9) OP_PUSH20(14) <20-byte-hash> OP_EQUALVERIFY(88) OP_CHECKSIG(ac)
    assert!(script.starts_with("76a914"), "script: {script}");
    assert!(script.ends_with("88ac"), "script: {script}");
    assert_eq!(script.len(), 50); // 25 bytes = 50 hex chars
}

#[test]
fn test_build_p2pkh_script_invalid_length() {
    let result = build_p2pkh_script("02aabb");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("length"));
}

#[test]
fn test_build_p2pkh_script_invalid_prefix() {
    // Valid length but bad prefix (not 02 or 03)
    let bad_key = format!("04{}", "aa".repeat(32));
    let result = build_p2pkh_script(&bad_key);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("prefix"));
}

#[test]
fn test_raw_tx_to_beef() {
    let raw_tx = vec![0x01, 0x00, 0x00, 0x00, 0xFF]; // fake raw tx
    let beef = raw_tx_to_beef(&raw_tx);

    // BEEF_V1 header: [0x01, 0x00, 0xBE, 0xEF]
    assert_eq!(&beef[..4], &[0x01, 0x00, 0xBE, 0xEF]);
    // 0 BUMPs (varint 0)
    assert_eq!(beef[4], 0x00);
    // 1 transaction (varint 1)
    assert_eq!(beef[5], 0x01);
    // Raw tx follows
    assert_eq!(&beef[6..11], &raw_tx);
    // hasBump = false
    assert_eq!(*beef.last().unwrap(), 0x00);
}

#[test]
fn test_raw_tx_to_atomic_beef() {
    let raw_tx = vec![0x01, 0x00, 0x00, 0x00];
    let txid_hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let atomic = raw_tx_to_atomic_beef(&raw_tx, txid_hex);

    // AtomicBEEF header
    assert_eq!(&atomic[..4], &[0x01, 0x01, 0x01, 0x01]);
    // Reversed txid follows (32 bytes)
    // The first byte of the reversed txid should be the last byte of the original
    assert_eq!(atomic.len(), 4 + 32 + raw_tx.len() + 7); // header + txid + beef_wrapped
}

#[test]
fn test_parse_402_response_valid() {
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert("x-bsv-payment-satoshis-required", "5000".parse().unwrap());
    headers.insert("x-bsv-payment-derivation-prefix", "abc123".parse().unwrap());

    let result = parse_402_response(&headers).unwrap();
    assert_eq!(result.satoshis, 5000);
    assert_eq!(result.derivation_prefix, "abc123");
}

#[test]
fn test_parse_402_response_missing_version() {
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-satoshis-required", "5000".parse().unwrap());
    headers.insert("x-bsv-payment-derivation-prefix", "abc123".parse().unwrap());

    let result = parse_402_response(&headers);
    assert!(result.is_err());
}

#[test]
fn test_parse_402_response_wrong_version() {
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-version", "2.0".parse().unwrap());
    headers.insert("x-bsv-payment-satoshis-required", "5000".parse().unwrap());
    headers.insert("x-bsv-payment-derivation-prefix", "abc123".parse().unwrap());

    let result = parse_402_response(&headers);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("version"));
}

#[test]
fn test_parse_402_response_zero_satoshis() {
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert("x-bsv-payment-satoshis-required", "0".parse().unwrap());
    headers.insert("x-bsv-payment-derivation-prefix", "abc".parse().unwrap());

    let result = parse_402_response(&headers);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("> 0"));
}

#[test]
fn test_parse_402_response_missing_satoshis() {
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert("x-bsv-payment-derivation-prefix", "abc".parse().unwrap());

    let result = parse_402_response(&headers);
    assert!(result.is_err());
}

// -- Refund tests --

#[test]
fn test_parse_refund_valid() {
    let body = json!({
        "excessRefund": {
            "transaction": "AQEBAQA=",
            "derivationPrefix": "abc",
            "derivationSuffix": "def",
            "senderIdentityKey": format!("02{}", "a".repeat(64)),
            "satoshis": 200,
            "txid": "f".repeat(64),
        }
    });
    let info = parse_refund(&body).unwrap();
    assert_eq!(info.satoshis, 200);
    assert_eq!(info.transaction, "AQEBAQA=");
    assert_eq!(info.derivation_prefix, "abc");
    assert_eq!(info.derivation_suffix, "def");
}

#[test]
fn test_parse_refund_missing() {
    let body = json!({"choices": []});
    assert!(parse_refund(&body).is_none());
}

#[test]
fn test_parse_refund_already_refunded() {
    let body = json!({
        "excessRefund": {
            "already_refunded": true,
            "transaction": "AQEBAQA=",
            "derivationPrefix": "abc",
            "derivationSuffix": "def",
            "senderIdentityKey": format!("02{}", "a".repeat(64)),
            "satoshis": 200,
        }
    });
    assert!(parse_refund(&body).is_none());
}

#[test]
fn test_parse_refund_incomplete() {
    // Missing required fields
    let body = json!({
        "excessRefund": {
            "transaction": "AQEBAQA=",
            // missing other fields
        }
    });
    assert!(parse_refund(&body).is_none());
}
