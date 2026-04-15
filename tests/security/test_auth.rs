//! Integration tests for BRC-31 Authrite client.

use dolphin_milk::auth::serialization::*;
use dolphin_milk::auth::session::*;
use dolphin_milk::auth::AuthriteClient;
use dolphin_milk::wallet::WalletClient;

use std::path::PathBuf;

// -----------------------------------------------------------------------
// Varint encoding
// -----------------------------------------------------------------------

#[test]
fn test_varint_boundary_values() {
    // Test all boundary values between encoding sizes
    let cases = vec![
        (0u64, 1usize),
        (1, 1),
        (252, 1),
        (253, 3), // first 2-byte
        (254, 3),
        (0xFFFF, 3),
        (0x10000, 5), // first 4-byte
        (0xFFFF_FFFF, 5),
        (0x1_0000_0000, 9), // first 8-byte
        (u64::MAX, 9),
    ];

    for (value, expected_len) in cases {
        let mut buf = Vec::new();
        write_varint(&mut buf, value);
        assert_eq!(
            buf.len(),
            expected_len,
            "varint({value}) should be {expected_len} bytes, got {}",
            buf.len()
        );
        let (decoded, consumed) = read_varint(&buf).unwrap();
        assert_eq!(decoded, value, "varint roundtrip failed for {value}");
        assert_eq!(consumed, expected_len);
    }
}

#[test]
fn test_varint_read_errors() {
    // Empty data
    assert!(read_varint(&[]).is_err());

    // Truncated 2-byte
    assert!(read_varint(&[0xFD, 0x00]).is_err());

    // Truncated 4-byte
    assert!(read_varint(&[0xFE, 0x00, 0x00, 0x00]).is_err());

    // Truncated 8-byte
    assert!(read_varint(&[0xFF, 0x00, 0x00]).is_err());
}

// -----------------------------------------------------------------------
// EMPTY_SENTINEL
// -----------------------------------------------------------------------

#[test]
fn test_empty_sentinel_is_nine_ff_bytes() {
    assert_eq!(EMPTY_SENTINEL, [0xFF; 9]);
}

// -----------------------------------------------------------------------
// Request serialization
// -----------------------------------------------------------------------

#[test]
fn test_request_serialization_full() {
    let request_id = [0xABu8; 32];
    let headers = vec![
        ("authorization".into(), "Bearer xyz".into()),
        ("content-type".into(), "application/json".into()),
    ];
    let body = b"{\"key\":\"value\"}";

    let serialized = serialize_request(
        &request_id,
        "POST",
        Some("/api/v1/resource"),
        Some("?limit=10"),
        &headers,
        Some(body),
    );

    // Verify it starts with raw request_id (32 bytes)
    assert_eq!(&serialized[..32], &[0xABu8; 32]);

    // Roundtrip
    let deserialized = deserialize_request(&serialized).unwrap();
    assert_eq!(deserialized.request_id, request_id);
    assert_eq!(deserialized.method, "POST");
    assert_eq!(deserialized.path, "/api/v1/resource");
    assert_eq!(deserialized.query, "?limit=10");
    assert_eq!(deserialized.headers.len(), 2);
    assert_eq!(deserialized.body.as_deref(), Some(body.as_ref()));
}

#[test]
fn test_request_serialization_no_optionals() {
    let request_id = [0u8; 32];
    let serialized = serialize_request(&request_id, "GET", None, None, &[], None);
    let deserialized = deserialize_request(&serialized).unwrap();

    assert_eq!(deserialized.method, "GET");
    assert!(deserialized.path.is_empty());
    assert!(deserialized.query.is_empty());
    assert!(deserialized.headers.is_empty());
    assert!(deserialized.body.is_none());
}

#[test]
fn test_request_id_not_varint_prefixed() {
    // The request_id should be raw 32 bytes, NOT varint-prefixed.
    // This means the serialized data starts directly with the 32 bytes.
    let request_id = [0x42u8; 32];
    let serialized = serialize_request(&request_id, "GET", None, None, &[], None);

    // First 32 bytes should be the request_id verbatim
    assert_eq!(&serialized[..32], &request_id);
    // Byte 32 should be the varint for method length (3 for "GET")
    assert_eq!(serialized[32], 3); // varint(3) = single byte 3
}

// -----------------------------------------------------------------------
// Response serialization
// -----------------------------------------------------------------------

#[test]
fn test_response_serialization_full() {
    let request_id = [0xCDu8; 32];
    let headers = vec![("x-bsv-result".into(), "ok".into())];
    let body = b"response content";

    let serialized = serialize_response(&request_id, 200, &headers, Some(body));
    let deserialized = deserialize_response(&serialized).unwrap();

    assert_eq!(deserialized.request_id, request_id);
    assert_eq!(deserialized.status_code, 200);
    assert_eq!(deserialized.headers.len(), 1);
    assert_eq!(deserialized.body.as_deref(), Some(body.as_ref()));
}

#[test]
fn test_response_various_status_codes() {
    for status in [200u16, 201, 400, 401, 402, 403, 404, 500] {
        let request_id = [0u8; 32];
        let serialized = serialize_response(&request_id, status, &[], None);
        let deserialized = deserialize_response(&serialized).unwrap();
        assert_eq!(deserialized.status_code, status);
    }
}

// -----------------------------------------------------------------------
// Header filtering
// -----------------------------------------------------------------------

#[test]
fn test_filter_includes_x_bsv_payment() {
    let headers = vec![("x-bsv-payment".into(), "payment-data".into())];
    let result = filter_signable_headers(&headers);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "x-bsv-payment");
}

#[test]
fn test_filter_excludes_x_bsv_auth() {
    let headers = vec![
        ("x-bsv-auth-nonce".into(), "nonce".into()),
        ("x-bsv-auth-signature".into(), "sig".into()),
        ("x-bsv-auth-version".into(), "0.1".into()),
    ];
    let result = filter_signable_headers(&headers);
    assert!(result.is_empty());
}

#[test]
fn test_filter_strips_content_type_params() {
    let headers = vec![(
        "Content-Type".into(),
        "application/json; charset=utf-8".into(),
    )];
    let result = filter_signable_headers(&headers);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "content-type");
    assert_eq!(result[0].1, "application/json");
}

#[test]
fn test_filter_includes_authorization() {
    let headers = vec![("Authorization".into(), "Bearer token123".into())];
    let result = filter_signable_headers(&headers);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "authorization");
    assert_eq!(result[0].1, "Bearer token123");
}

#[test]
fn test_filter_sorted_alphabetically() {
    let headers = vec![
        ("x-bsv-payment".into(), "pay".into()),
        ("Authorization".into(), "auth".into()),
        ("Content-Type".into(), "application/json".into()),
    ];
    let result = filter_signable_headers(&headers);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].0, "authorization");
    assert_eq!(result[1].0, "content-type");
    assert_eq!(result[2].0, "x-bsv-payment");
}

#[test]
fn test_filter_excludes_generic_headers() {
    let headers = vec![
        ("Accept".into(), "text/html".into()),
        ("User-Agent".into(), "dolphin-milk/0.1".into()),
        ("Cache-Control".into(), "no-cache".into()),
    ];
    let result = filter_signable_headers(&headers);
    assert!(result.is_empty());
}

// -----------------------------------------------------------------------
// Auth header construction
// -----------------------------------------------------------------------

#[test]
fn test_build_auth_headers_complete() {
    let headers = build_auth_headers(
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
        "bXNnX25vbmNl",
        "c2VydmVyX25vbmNl",
        "3045022100deadbeef",
        "cmVxdWVzdF9pZA==",
    );

    assert_eq!(headers.len(), 7);

    let headers_map: std::collections::HashMap<_, _> = headers.into_iter().collect();
    assert_eq!(headers_map["x-bsv-auth-version"], "0.1");
    assert_eq!(
        headers_map["x-bsv-auth-identity-key"],
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
    );
    assert_eq!(headers_map["x-bsv-auth-message-type"], "general");
    assert_eq!(headers_map["x-bsv-auth-nonce"], "bXNnX25vbmNl");
    assert_eq!(headers_map["x-bsv-auth-your-nonce"], "c2VydmVyX25vbmNl");
    assert_eq!(headers_map["x-bsv-auth-signature"], "3045022100deadbeef");
    assert_eq!(headers_map["x-bsv-auth-request-id"], "cmVxdWVzdF9pZA==");
}

// -----------------------------------------------------------------------
// Session persistence
// -----------------------------------------------------------------------

#[test]
fn test_session_save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().to_path_buf();

    let session = AuthSession::new(
        "https://messagebox.babbage.systems".into(),
        "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1".into(),
        "c2VydmVy".into(),
        "Y2xpZW50".into(),
    );

    save_session_to(&session, &session_dir).unwrap();
    let loaded = load_session_from(
        "https://messagebox.babbage.systems",
        DEFAULT_TTL,
        &session_dir,
    )
    .unwrap();

    assert_eq!(loaded.server_url, session.server_url);
    assert_eq!(loaded.server_identity_key, session.server_identity_key);
    assert_eq!(loaded.server_nonce_b64, session.server_nonce_b64);
    assert_eq!(loaded.client_nonce_b64, session.client_nonce_b64);
}

#[test]
fn test_session_expired_auto_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().to_path_buf();

    let mut session = AuthSession::new(
        "https://expired.example.com".into(),
        "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1".into(),
        "bm9uY2U=".into(),
        "Y2xpZW50".into(),
    );
    session.timestamp -= 7200.0; // 2 hours ago

    save_session_to(&session, &session_dir).unwrap();

    // Load should return None and delete file
    assert!(load_session_from("https://expired.example.com", 3600, &session_dir).is_none());
}

#[test]
fn test_session_clear() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().to_path_buf();

    let session = AuthSession::new(
        "https://clear-test.example.com".into(),
        "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1".into(),
        "bm9uY2U=".into(),
        "Y2xpZW50".into(),
    );
    save_session_to(&session, &session_dir).unwrap();

    assert!(clear_session_from(
        "https://clear-test.example.com",
        &session_dir
    ));
    assert!(!clear_session_from(
        "https://clear-test.example.com",
        &session_dir
    ));
}

#[test]
fn test_clear_all_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().to_path_buf();

    for url in ["https://a.com", "https://b.com", "https://c.com"] {
        let session = AuthSession::new(
            url.into(),
            "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1".into(),
            "bm9uY2U=".into(),
            "Y2xpZW50".into(),
        );
        save_session_to(&session, &session_dir).unwrap();
    }

    assert_eq!(clear_all_sessions_from(&session_dir), 3);
}

// -----------------------------------------------------------------------
// AuthriteClient construction
// -----------------------------------------------------------------------

#[test]
fn test_authrite_client_default_session_dir() {
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let client = AuthriteClient::new(std::sync::Arc::new(wallet), "http://localhost:3322");
    assert!(client
        .session_dir
        .to_string_lossy()
        .contains("brc31-sessions"));
}

#[test]
fn test_authrite_client_custom_dir() {
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let dir = PathBuf::from("/tmp/custom-sessions");
    let client = AuthriteClient::with_session_dir(std::sync::Arc::new(wallet), dir.clone());
    assert_eq!(client.session_dir, dir);
}

// -----------------------------------------------------------------------
// Handshake mock test
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_handshake_parses_server_response() {
    let mut server = mockito::Server::new_async().await;
    let server_url = server.url();

    let _mock = server
        .mock("POST", "/.well-known/auth")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "version": "0.1",
                "messageType": "initialResponse",
                "identityKey": "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1",
                "initialNonce": "dGVzdF9zZXJ2ZXJfbm9uY2VfYmFzZTY0X2VuYw==",
            })
            .to_string(),
        )
        .create_async()
        .await;

    let dir = tempfile::tempdir().unwrap();
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let client =
        AuthriteClient::with_session_dir(std::sync::Arc::new(wallet), dir.path().to_path_buf());

    let result = client.do_handshake(&server_url).await;

    // If wallet is running, handshake succeeds. If not, wallet error.
    match result {
        Ok(session) => {
            assert_eq!(
                session.server_identity_key,
                "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1"
            );
            assert_eq!(
                session.server_nonce_b64,
                "dGVzdF9zZXJ2ZXJfbm9uY2VfYmFzZTY0X2VuYw=="
            );
        }
        Err(e) => {
            let err = e.to_string();
            assert!(
                err.contains("wallet") || err.contains("Wallet") || err.contains("reachable"),
                "Expected wallet error, got: {err}"
            );
        }
    }
}

#[tokio::test]
async fn test_handshake_rejects_bad_status() {
    let mut server = mockito::Server::new_async().await;
    let server_url = server.url();

    let _mock = server
        .mock("POST", "/.well-known/auth")
        .with_status(500)
        .with_body("Internal Server Error")
        .create_async()
        .await;

    let dir = tempfile::tempdir().unwrap();
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let client =
        AuthriteClient::with_session_dir(std::sync::Arc::new(wallet), dir.path().to_path_buf());

    let result = client.do_handshake(&server_url).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Wallet identity key fetch happens first; if wallet is up, handshake fails with 500
    assert!(
        err.contains("500")
            || err.contains("wallet")
            || err.contains("Wallet")
            || err.contains("reachable"),
        "Expected HTTP 500 or wallet error, got: {err}"
    );
}
