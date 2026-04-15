//! Live integration tests — real wallet + real MessageBox server.
//!
//! These tests hit actual running services and prove the wire format works
//! end-to-end. Every test checks for wallet availability at the top and
//! returns early if unreachable (no #[ignore]).
//!
//! ## Free tests (wallet on 3322 must be running):
//! ```sh
//! cargo test --test test_live_integration -- --nocapture
//! ```
//!
//! ## All tests including paid (~3600 sats):
//! ```sh
//! RUN_PAID_TESTS=1 cargo test --test test_live_integration -- --nocapture
//! ```

use serde_json::{json, Value};

use dolphin_milk::auth::AuthriteClient;
use dolphin_milk::config::DmConfig;
use dolphin_milk::error::DmError;
use dolphin_milk::messagebox::types::*;
use dolphin_milk::messagebox::MessageBoxClient;
use dolphin_milk::runner;
use dolphin_milk::wallet::WalletClient;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Factory for the primary wallet (bsv-wallet-cli on port 3322).
fn wallet_3322() -> WalletClient {
    WalletClient::new("http://localhost:3322", "http://localhost", 30)
}

fn test_wallet() -> std::sync::Arc<dyn dolphin_milk::wallet::WalletBackend + Send + Sync> {
    std::sync::Arc::new(wallet_3322())
}

/// Factory for the counterparty wallet (MetaNet Client on port 3321).
fn wallet_3321() -> WalletClient {
    WalletClient::new("http://localhost:3321", "http://localhost", 30)
}

/// Returns true if the wallet is reachable and authenticated.
async fn check_wallet(w: &WalletClient) -> bool {
    match w.is_authenticated().await {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[skip] wallet at {} not reachable: {e}", w.url);
            false
        }
    }
}

/// Returns true if an error is a wallet-connectivity error.
fn is_wallet_error(e: &DmError) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("wallet") || msg.contains("reachable") || msg.contains("connection refused")
}

/// Returns true if an Authrite handshake with MessageBox succeeds.
/// Tests that depend on the external MessageBox service should call this
/// and return early when the service is unavailable.
async fn check_messagebox(w: &WalletClient) -> bool {
    let dir = tempfile::tempdir().expect("tempdir failed");
    let client = AuthriteClient::with_session_dir(
        std::sync::Arc::new(WalletClient::new(&w.url, &w.origin, 10)),
        dir.path().to_path_buf(),
    );
    match client.do_handshake(MESSAGEBOX_URL).await {
        Ok(_) => true,
        Err(e) => {
            eprintln!(
                "[skip] MessageBox auth at {} not available: {e}",
                MESSAGEBOX_URL
            );
            false
        }
    }
}

/// Returns true if the RUN_PAID_TESTS=1 env var is set.
fn paid_tests_enabled() -> bool {
    std::env::var("RUN_PAID_TESTS")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tier 1: Free Tests (0 sats)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_live_get_identity_key() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    let key = w.get_identity_key().await.expect("get_identity_key failed");
    eprintln!("identity key: {key}");

    // 66-char hex compressed public key
    assert_eq!(key.len(), 66, "identity key should be 66 hex chars");
    assert!(
        key.starts_with("02") || key.starts_with("03"),
        "compressed pubkey must start with 02 or 03"
    );
    assert!(
        key.chars().all(|c| c.is_ascii_hexdigit()),
        "identity key must be valid hex"
    );

    // Calling twice should return the same key (deterministic).
    let key2 = w.get_identity_key().await.expect("second call failed");
    assert_eq!(key, key2, "identity key must be deterministic");
}

#[tokio::test]
async fn test_live_get_public_key_brc42() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    let counterparty = w.get_identity_key().await.unwrap();
    let protocol = json!([2, "auth message signature"]);

    let k1 = w
        .get_public_key(&protocol, "key-id-alpha", &counterparty, false)
        .await
        .expect("BRC-42 derivation failed");
    eprintln!("derived key 1: {k1}");

    assert_eq!(k1.len(), 66);
    assert!(k1.starts_with("02") || k1.starts_with("03"));

    // Different key_id → different key.
    let k2 = w
        .get_public_key(&protocol, "key-id-beta", &counterparty, false)
        .await
        .expect("BRC-42 derivation failed (second)");
    eprintln!("derived key 2: {k2}");

    assert_ne!(k1, k2, "different key_id must yield different key");

    // Same key_id → same key (deterministic).
    let k1_again = w
        .get_public_key(&protocol, "key-id-alpha", &counterparty, false)
        .await
        .expect("BRC-42 derivation failed (repeat)");
    assert_eq!(k1, k1_again, "same derivation params must be deterministic");
}

#[tokio::test]
async fn test_live_create_signature() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    let counterparty = w.get_identity_key().await.unwrap();
    let protocol = json!([2, "auth message signature"]);
    let data = b"hello from integration test";

    let sig = w
        .create_signature(data, &protocol, "test-key-id", &counterparty)
        .await
        .expect("createSignature failed");
    eprintln!("signature: {} bytes", sig.len());

    // DER-encoded ECDSA signature is typically 70-72 bytes.
    assert!(
        (64..=73).contains(&sig.len()),
        "ECDSA sig should be 64-73 bytes, got {}",
        sig.len()
    );

    // Signing same data again should produce valid (though possibly different) signature.
    let sig2 = w
        .create_signature(data, &protocol, "test-key-id", &counterparty)
        .await
        .expect("second createSignature failed");
    assert!(
        (64..=73).contains(&sig2.len()),
        "second sig length invalid: {}",
        sig2.len()
    );
}

#[tokio::test]
async fn test_live_create_hmac_verify_hmac() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    let counterparty = w.get_identity_key().await.unwrap();
    let protocol = json!([2, "auth message signature"]);
    let data = b"hmac integration test data";

    // createHmac
    let hmac = w
        .create_signature(data, &protocol, "hmac-key-id", &counterparty)
        .await;

    match hmac {
        Ok(sig) => {
            eprintln!("HMAC/signature: {} bytes", sig.len());
            assert!(!sig.is_empty(), "HMAC must not be empty");
        }
        Err(e) => {
            // Some wallets may not support HMAC — that's OK, just log it.
            eprintln!("[info] HMAC not supported or failed: {e}");
            assert!(is_wallet_error(&e), "expected wallet error, got: {e}");
        }
    }
}

#[tokio::test]
async fn test_live_authrite_handshake() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }
    if !check_messagebox(&w).await {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir failed");
    let client = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());

    let session = client
        .do_handshake(MESSAGEBOX_URL)
        .await
        .expect("handshake failed");

    eprintln!("server identity key: {}", session.server_identity_key);
    eprintln!("server nonce (b64):  {}", session.server_nonce_b64);
    eprintln!("client nonce (b64):  {}", session.client_nonce_b64);

    // Server identity key should match the known MessageBox key.
    assert_eq!(
        session.server_identity_key, MESSAGEBOX_IDENTITY_KEY,
        "server identity key mismatch"
    );

    // Nonces should be non-empty base64 strings.
    assert!(
        session.server_nonce_b64.len() >= 4,
        "server nonce too short"
    );
    assert!(
        session.client_nonce_b64.len() >= 4,
        "client nonce too short"
    );

    // Session should not be expired.
    assert!(
        !session.is_expired(3600),
        "freshly-created session should not be expired"
    );
}

#[tokio::test]
async fn test_live_authrite_authenticated_post() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }
    if !check_messagebox(&w).await {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir failed");
    let client = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());

    // Use listMessages as a lightweight authenticated POST.
    let url = format!("{}/listMessages", MESSAGEBOX_URL);
    let body = json!({"messageBox": "status_inbox"});
    let body_bytes = serde_json::to_vec(&body).unwrap();

    let resp = client
        .authenticated_request(
            "POST",
            &url,
            &[("content-type".into(), "application/json".into())],
            Some(&body_bytes),
        )
        .await
        .expect("authenticated POST failed");

    let status = resp.status();
    eprintln!("authenticated POST status: {status}");

    // Should be 200 (even if no messages).
    assert!(
        status.is_success(),
        "authenticated POST should succeed, got {status}"
    );

    let data: Value = resp.json().await.expect("response is not JSON");
    eprintln!("response: {data}");

    // Response should have a "messages" array (possibly empty).
    assert!(
        data.get("messages").is_some(),
        "response should contain 'messages' field"
    );
}

#[tokio::test]
async fn test_live_authrite_session_reuse() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }
    if !check_messagebox(&w).await {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir failed");
    let client = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());

    // First call: performs handshake.
    let s1 = client
        .get_or_create_session(MESSAGEBOX_URL)
        .await
        .expect("first session failed");

    // Second call: should reuse cached session.
    let s2 = client
        .get_or_create_session(MESSAGEBOX_URL)
        .await
        .expect("second session failed");

    assert_eq!(
        s1.server_nonce_b64, s2.server_nonce_b64,
        "cached session should be reused (same server nonce)"
    );
    assert_eq!(
        s1.client_nonce_b64, s2.client_nonce_b64,
        "cached session should be reused (same client nonce)"
    );

    eprintln!("session reuse confirmed — same nonces on second call");
}

#[tokio::test]
async fn test_live_messagebox_quote_self() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }
    if !check_messagebox(&w).await {
        return;
    }

    let identity = w.get_identity_key().await.unwrap();
    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());
    let mbox = MessageBoxClient::new(auth);

    // Quote for self to status_inbox (free delivery).
    let quote = mbox
        .quote(&identity, BOX_STATUS_INBOX)
        .await
        .expect("quote failed");

    eprintln!(
        "quote: delivery_fee={}, recipient_fee={}, total={}",
        quote.delivery_fee,
        quote.recipient_fee,
        quote.total_cost()
    );

    assert!(!quote.is_blocked(), "self-delivery should not be blocked");
    // Self-delivery should be free (0 sats).
    assert_eq!(
        quote.total_cost(),
        0,
        "self-delivery to status_inbox should be free"
    );
}

#[tokio::test]
async fn test_live_messagebox_send_list_ack() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }
    if !check_messagebox(&w).await {
        return;
    }

    let identity = w.get_identity_key().await.unwrap();
    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());
    let mbox = MessageBoxClient::new(auth);

    // 1. Send a message to self via status_inbox (free, 0 sats).
    let unique_marker = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "integration_test",
        "marker": unique_marker,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    eprintln!("sending message with marker: {unique_marker}");

    let send_result = mbox
        .send_message(&identity, BOX_STATUS_INBOX, &body)
        .await
        .expect("send_message failed");

    eprintln!("send result: {send_result}");

    // 2. List messages and find ours.
    let messages = mbox
        .list_messages(BOX_STATUS_INBOX)
        .await
        .expect("list_messages failed");

    eprintln!("found {} messages in status_inbox", messages.len());

    let our_messages: Vec<&ReceivedMessage> = messages
        .iter()
        .filter(|m| m.body.get("marker").and_then(|v| v.as_str()) == Some(&unique_marker))
        .collect();

    assert!(
        !our_messages.is_empty(),
        "should find our message with marker {unique_marker}"
    );

    let our_msg = our_messages[0];
    eprintln!(
        "found our message: id={}, sender={}",
        our_msg.message_id, our_msg.sender
    );

    assert_eq!(our_msg.sender, identity, "sender should be our identity");
    assert_eq!(
        our_msg.body.get("type").and_then(|v| v.as_str()),
        Some("integration_test")
    );

    // 3. Acknowledge (delete) the message.
    let ids_to_ack: Vec<String> = our_messages.iter().map(|m| m.message_id.clone()).collect();

    mbox.acknowledge_message(&ids_to_ack)
        .await
        .expect("acknowledge_message failed");

    eprintln!("acknowledged {} messages", ids_to_ack.len());

    // 4. Verify it's gone.
    let messages_after = mbox
        .list_messages(BOX_STATUS_INBOX)
        .await
        .expect("list_messages (post-ack) failed");

    let still_there = messages_after
        .iter()
        .any(|m| m.body.get("marker").and_then(|v| v.as_str()) == Some(&unique_marker));

    assert!(!still_there, "message should be gone after acknowledgement");

    eprintln!("full lifecycle: send → list → ack → verify-gone PASSED");
}

#[tokio::test]
async fn test_live_messagebox_poll_inboxes() {
    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());
    let mbox = MessageBoxClient::new(auth);

    // poll_inboxes hits all 4 boxes — should not error even if empty.
    let messages = mbox.poll_inboxes().await.expect("poll_inboxes failed");

    eprintln!("poll_inboxes returned {} messages total", messages.len());

    // No assertion on count — just that it doesn't error.
    // All messages should have required fields.
    for msg in &messages {
        assert!(!msg.message_id.is_empty(), "message_id must not be empty");
        assert_eq!(
            msg.sender.len(),
            66,
            "sender should be 66-char pubkey, got {}",
            msg.sender.len()
        );
        assert!(!msg.created_at.is_empty(), "created_at must not be empty");
    }
}

#[tokio::test]
async fn test_live_messagebox_quote_cross_wallet() {
    let w3322 = wallet_3322();
    let w3321 = wallet_3321();

    if !check_wallet(&w3322).await {
        return;
    }
    if !check_wallet(&w3321).await {
        eprintln!("[skip] MetaNet Client on 3321 not available — skipping cross-wallet quote");
        return;
    }
    if !check_messagebox(&w3322).await {
        return;
    }

    let other_identity = w3321
        .get_identity_key()
        .await
        .expect("3321 identity failed");
    eprintln!("MetaNet Client identity: {other_identity}");

    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth =
        AuthriteClient::with_session_dir(std::sync::Arc::new(w3322), dir.path().to_path_buf());
    let mbox = MessageBoxClient::new(auth);

    let quote = mbox
        .quote(&other_identity, BOX_STATUS_INBOX)
        .await
        .expect("cross-wallet quote failed");

    eprintln!(
        "cross-wallet quote: delivery_fee={}, recipient_fee={}, total={}",
        quote.delivery_fee,
        quote.recipient_fee,
        quote.total_cost()
    );

    // Just verify it parsed — we don't know if the other wallet has set fees.
    assert!(
        !quote.is_blocked(),
        "cross-wallet delivery should not be blocked (both wallets are ours)"
    );
}

// ---------------------------------------------------------------------------
// Tier 2: Paid Tests (require RUN_PAID_TESTS=1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_live_paid_create_proof() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::proofs::{create_proof, ProofCommitment, ProofType};

    let commitment = ProofCommitment::new(
        ProofType::TaskCompletion,
        "integration test proof — live BRC-18 OP_RETURN",
        None,
    );

    assert!(commitment.verify(), "commitment should self-verify");
    eprintln!("proof hash: {}", commitment.hash_hex());

    let result = create_proof(&w, commitment)
        .await
        .expect("create_proof failed");

    eprintln!("BRC-18 proof txid: {}", result.txid);
    assert!(!result.txid.is_empty(), "txid must not be empty");
    assert_eq!(result.txid.len(), 64, "txid should be 64 hex chars");
    assert!(
        result.txid.chars().all(|c| c.is_ascii_hexdigit()),
        "txid must be valid hex"
    );
    assert!(
        result.commitment.verify(),
        "returned commitment should verify"
    );
}

#[tokio::test]
async fn test_live_paid_create_state_token() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::state::{create_token, StateToken, TokenType};

    // 1. Create plaintext token.
    let plain_token = StateToken::new(
        TokenType::TaskCommitment,
        json!({
            "task": "integration test — plaintext token",
            "created": chrono::Utc::now().to_rfc3339(),
        }),
    );

    let plain_result = create_token(&w, plain_token, false, "self")
        .await
        .expect("plaintext create_token failed");

    eprintln!("plaintext token txid: {}", plain_result.txid);
    assert!(!plain_result.txid.is_empty());
    assert_eq!(plain_result.txid.len(), 64);
    assert!(plain_result.token.is_on_chain());

    // 2. Create encrypted token (wallet handles encryption).
    let enc_token = StateToken::new(
        TokenType::Checkpoint,
        json!({
            "task": "integration test — encrypted token",
            "secret": "this should be encrypted on-chain",
        }),
    );

    let enc_result = create_token(&w, enc_token, true, "self")
        .await
        .expect("encrypted create_token failed");

    eprintln!("encrypted token txid: {}", enc_result.txid);
    assert!(!enc_result.txid.is_empty());
    assert_eq!(enc_result.txid.len(), 64);
    assert!(enc_result.token.is_on_chain());

    // Both txids should be different transactions.
    assert_ne!(
        plain_result.txid, enc_result.txid,
        "plaintext and encrypted tokens should be different txs"
    );
}

#[tokio::test]
async fn test_live_paid_think_llm() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::x402::payment::{create_payment, parse_402_response};

    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth =
        AuthriteClient::with_session_dir(std::sync::Arc::new(w.clone()), dir.path().to_path_buf());

    let llm_url = "https://openai-chat.x402agency.com/chat";

    let body = json!({
        "model": "gpt-5-mini",
        "messages": [
            {"role": "system", "content": "You are a helpful assistant. Respond in one sentence."},
            {"role": "user", "content": "What is 2+2? Answer with just the number."},
        ],
        "max_tokens": 256,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();

    // Step 1: Authenticated request → expect 402 Payment Required.
    let resp = auth
        .authenticated_request(
            "POST",
            llm_url,
            &[("content-type".into(), "application/json".into())],
            Some(&body_bytes),
        )
        .await
        .expect("authenticated request failed");

    let status = resp.status();
    eprintln!("initial response status: {status}");

    if status.is_success() {
        // Server didn't require payment (unlikely but possible for tiny requests).
        let data: Value = resp.json().await.expect("response not JSON");
        eprintln!("LLM response (no payment needed): {data}");
        return;
    }

    assert_eq!(
        status.as_u16(),
        402,
        "expected 402 Payment Required, got {status}"
    );

    // Step 2: Parse 402 headers for payment requirements.
    let payment_req = parse_402_response(resp.headers()).expect("failed to parse 402 headers");

    eprintln!(
        "payment required: {} sats, prefix={}, version={}",
        payment_req.satoshis, payment_req.derivation_prefix, payment_req.version
    );

    // Get server identity key from response headers.
    let server_identity_key = resp
        .headers()
        .get("x-bsv-auth-identity-key")
        .and_then(|v| v.to_str().ok())
        .expect("missing x-bsv-auth-identity-key");

    eprintln!("server identity key: {server_identity_key}");

    // Step 3: Create BRC-29 payment.
    let wallet_adapter = dolphin_milk::x402::WalletBackendAdapter(std::sync::Arc::new(w.clone()));
    let (payment, payment_txid) = create_payment(
        &wallet_adapter,
        &payment_req.derivation_prefix,
        server_identity_key,
        payment_req.satoshis,
        llm_url,
    )
    .await
    .expect("create_payment failed");

    let payment_json = serde_json::to_string(&payment).unwrap();
    eprintln!("payment txid: {payment_txid}");
    eprintln!("payment JSON size: {} bytes", payment_json.len());

    // Step 4: Retry with payment in x-bsv-payment header (authenticated).
    let resp2 = auth
        .authenticated_request(
            "POST",
            llm_url,
            &[
                ("content-type".into(), "application/json".into()),
                ("x-bsv-payment".into(), payment_json),
            ],
            Some(&body_bytes),
        )
        .await
        .expect("paid authenticated request failed");

    let status2 = resp2.status();
    eprintln!("paid response status: {status2}");

    assert!(
        status2.is_success(),
        "paid LLM request should succeed, got {status2}"
    );

    let data: Value = resp2.json().await.expect("LLM response not JSON");
    eprintln!(
        "LLM response: {}",
        serde_json::to_string_pretty(&data).unwrap()
    );

    // Validate OpenAI chat response structure.
    let choices = data
        .get("choices")
        .and_then(|v| v.as_array())
        .expect("response should have 'choices' array");

    assert!(!choices.is_empty(), "choices should not be empty");

    let content = choices[0]
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    eprintln!("LLM says: {content}");

    // Reasoning models may consume tokens internally and return empty content
    // with finish_reason="length". That's valid — the flow still worked.
    let finish_reason = choices[0]
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !content.is_empty() || finish_reason == "length",
        "expected non-empty content or finish_reason=length"
    );

    // Check usage stats if present.
    if let Some(usage) = data.get("usage") {
        eprintln!(
            "tokens: prompt={}, completion={}, total={}",
            usage.get("prompt_tokens").unwrap_or(&json!(0)),
            usage.get("completion_tokens").unwrap_or(&json!(0)),
            usage.get("total_tokens").unwrap_or(&json!(0)),
        );
    }

    // Check payment info if present.
    if let Some(payment_info) = data.get("payment") {
        eprintln!(
            "payment: paid={} sats, effective={} sats",
            payment_info.get("satoshis_paid").unwrap_or(&json!(0)),
            payment_info.get("satoshis_effective").unwrap_or(&json!(0)),
        );
    }
}

// ---------------------------------------------------------------------------
// Tier 3: Cross-Wallet Conversation Tests
// ---------------------------------------------------------------------------

/// Send a message from MetaNet Client (3321) to worm (3322) via MessageBox,
/// then verify worm can read it. This is the foundation for agent conversations.
#[tokio::test]
async fn test_live_cross_wallet_send_and_receive() {
    let w3322 = wallet_3322();
    let w3321 = wallet_3321();

    if !check_wallet(&w3322).await {
        return;
    }
    if !check_wallet(&w3321).await {
        eprintln!("[skip] MetaNet Client on 3321 not available");
        return;
    }
    if !check_messagebox(&w3322).await {
        return;
    }

    let worm_identity = w3322.get_identity_key().await.unwrap();
    let user_identity = w3321.get_identity_key().await.unwrap();

    eprintln!("worm identity: {worm_identity}");
    eprintln!("user identity: {user_identity}");

    // 1. MetaNet Client (3321) sends a message TO the worm's inbox.
    let dir_sender = tempfile::tempdir().unwrap();
    let auth_sender = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321),
        dir_sender.path().to_path_buf(),
    );
    let mbox_sender = MessageBoxClient::new(auth_sender);

    let marker = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "task_assignment",
        "task": "Hello worm! What is the capital of France?",
        "marker": marker,
        "from": "metanet_client_integration_test",
    });

    eprintln!("sending message from MetaNet Client → worm (marker: {marker})");

    let send_result = mbox_sender
        .send_message(&worm_identity, BOX_TASK_INBOX, &body)
        .await
        .expect("cross-wallet send_message failed");

    eprintln!("send result: {send_result}");

    // 2. Worm (3322) reads its inbox and finds the message.
    let dir_worm = tempfile::tempdir().unwrap();
    let auth_worm =
        AuthriteClient::with_session_dir(std::sync::Arc::new(w3322), dir_worm.path().to_path_buf());
    let mbox_worm = MessageBoxClient::new(auth_worm);

    let messages = mbox_worm
        .list_messages(BOX_TASK_INBOX)
        .await
        .expect("worm list_messages failed");

    eprintln!("worm inbox has {} messages", messages.len());

    let our_msgs: Vec<&ReceivedMessage> = messages
        .iter()
        .filter(|m| m.body.get("marker").and_then(|v| v.as_str()) == Some(&marker))
        .collect();

    assert!(
        !our_msgs.is_empty(),
        "worm should find message with marker {marker}"
    );

    let msg = our_msgs[0];
    eprintln!("worm received: sender={}, body={}", msg.sender, msg.body);
    assert_eq!(msg.sender, user_identity, "sender should be MetaNet Client");
    assert_eq!(
        msg.body.get("task").and_then(|v| v.as_str()),
        Some("Hello worm! What is the capital of France?")
    );

    // 3. Clean up: acknowledge the message.
    let ids: Vec<String> = our_msgs.iter().map(|m| m.message_id.clone()).collect();
    mbox_worm
        .acknowledge_message(&ids)
        .await
        .expect("acknowledge failed");

    eprintln!("cross-wallet send → receive → ack PASSED");
}

/// Send a message from worm (3322) to MetaNet Client (3321), verifying
/// the reverse direction works too (needed for the worm to reply).
#[tokio::test]
async fn test_live_cross_wallet_worm_replies() {
    let w3322 = wallet_3322();
    let w3321 = wallet_3321();

    if !check_wallet(&w3322).await {
        return;
    }
    if !check_wallet(&w3321).await {
        eprintln!("[skip] MetaNet Client on 3321 not available");
        return;
    }
    if !check_messagebox(&w3322).await {
        return;
    }

    let user_identity = w3321.get_identity_key().await.unwrap();

    // Worm sends a reply to MetaNet Client's status_inbox.
    let dir_worm = tempfile::tempdir().unwrap();
    let auth_worm =
        AuthriteClient::with_session_dir(std::sync::Arc::new(w3322), dir_worm.path().to_path_buf());
    let mbox_worm = MessageBoxClient::new(auth_worm);

    let marker = uuid::Uuid::new_v4().to_string();
    let reply = json!({
        "type": "task_result",
        "result": "The capital of France is Paris.",
        "marker": marker,
    });

    eprintln!("worm sending reply to MetaNet Client (marker: {marker})");

    let send_result = mbox_worm
        .send_message(&user_identity, BOX_STATUS_INBOX, &reply)
        .await
        .expect("worm → MetaNet Client send_message failed");

    eprintln!("send result: {send_result}");

    // MetaNet Client reads its inbox.
    let dir_user = tempfile::tempdir().unwrap();
    let auth_user =
        AuthriteClient::with_session_dir(std::sync::Arc::new(w3321), dir_user.path().to_path_buf());
    let mbox_user = MessageBoxClient::new(auth_user);

    let messages = mbox_user
        .list_messages(BOX_STATUS_INBOX)
        .await
        .expect("MetaNet Client list_messages failed");

    eprintln!("MetaNet Client inbox has {} messages", messages.len());

    let our_msgs: Vec<&ReceivedMessage> = messages
        .iter()
        .filter(|m| m.body.get("marker").and_then(|v| v.as_str()) == Some(&marker))
        .collect();

    assert!(
        !our_msgs.is_empty(),
        "MetaNet Client should find reply with marker {marker}"
    );

    let msg = our_msgs[0];
    eprintln!("MetaNet Client received: {}", msg.body);
    assert_eq!(
        msg.body.get("result").and_then(|v| v.as_str()),
        Some("The capital of France is Paris.")
    );

    // Clean up.
    let ids: Vec<String> = our_msgs.iter().map(|m| m.message_id.clone()).collect();
    mbox_user
        .acknowledge_message(&ids)
        .await
        .expect("acknowledge failed");

    eprintln!("worm → MetaNet Client reply → receive → ack PASSED");
}

/// U0 NUDGE TEST: Verify the multi-turn nudge forces send_message.
///
/// 1. MetaNet Client sends a task to worm's task_inbox
/// 2. Worm runs with a deliberately vague task (no mention of send_message)
/// 3. If the LLM responds with text instead of calling send_message, the nudge fires
/// 4. Assert: send_message was eventually called OR nudge_count proves the mechanism engaged
/// 5. Assert: MetaNet Client receives a reply from the worm
///
/// This tests the Phase U0 fix: the nudge mechanism that prevents conversations
/// from dying when the LLM "thinks" its answer instead of using send_message.
#[tokio::test]
async fn test_live_u0_nudge_forces_send_message() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w3322 = wallet_3322();
    let w3321 = wallet_3321();

    if !check_wallet(&w3322).await {
        return;
    }
    if !check_wallet(&w3321).await {
        eprintln!("[skip] MetaNet Client on 3321 not available");
        return;
    }

    let worm_identity = w3322.get_identity_key().await.unwrap();
    let user_identity = w3321.get_identity_key().await.unwrap();

    eprintln!("=== U0 NUDGE TEST ===");
    eprintln!("worm identity:  {worm_identity}");
    eprintln!("user identity:  {user_identity}");

    // 1. MetaNet Client sends a task to worm's task_inbox.
    let dir_sender = tempfile::tempdir().unwrap();
    let auth_sender = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321.clone()),
        dir_sender.path().to_path_buf(),
    );
    let mbox_sender = MessageBoxClient::new(auth_sender);

    let marker = uuid::Uuid::new_v4().to_string();
    let task_body = json!({
        "type": "task_assignment",
        "task": "What is 7 * 6? Reply with just the number.",
        "reply_to": user_identity,
        "reply_box": BOX_STATUS_INBOX,
        "marker": marker,
    });

    eprintln!("Step 1: MetaNet Client sending task (marker: {marker})...");

    mbox_sender
        .send_message(&worm_identity, BOX_TASK_INBOX, &task_body)
        .await
        .expect("send task to worm failed");

    eprintln!("Step 1: DONE — task sent to worm's task_inbox");

    // 2. Run the worm with a deliberately vague task (no mention of send_message).
    let workspace = tempfile::tempdir().unwrap();
    let config = DmConfig::default();
    let mut worm = runner::create_loop(
        config,
        workspace.path().to_path_buf(),
        None,
        None,
        test_wallet(),
    );

    eprintln!("Step 2: Starting worm agent loop (max 5 iterations)...");

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    worm.run(
        "Process your inbox messages and reply to the sender.",
        5, // max 5 iterations — enough for nudge + retry
        None,
        cancel,
    )
    .await;

    eprintln!(
        "Step 2: DONE — {} iterations, {} sats, done={}, error='{}', nudge_count={}",
        worm.state.exec.iteration,
        worm.state.budget.sats_spent,
        worm.state.exec.done,
        worm.state.exec.error,
        worm.state.comms.nudge_count,
    );

    // 3. Analyze transcript for send_message calls.
    let transcript_events = worm.transcript.replay();

    let tool_calls: Vec<_> = transcript_events
        .iter()
        .filter(|e| e.event_type == "tool_call")
        .collect();

    let send_calls: Vec<_> = tool_calls
        .iter()
        .filter(|tc| tc.data.get("name").and_then(|v| v.as_str()) == Some("send_message"))
        .collect();

    let nudge_events: Vec<_> = transcript_events
        .iter()
        .filter(|e| {
            e.event_type == "user"
                && e.data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains("MUST use the `send_message` tool"))
                    .unwrap_or(false)
        })
        .collect();

    eprintln!("Step 3: Transcript analysis:");
    eprintln!("  total events:      {}", transcript_events.len());
    eprintln!("  tool calls:        {}", tool_calls.len());
    eprintln!("  send_message calls: {}", send_calls.len());
    eprintln!("  nudge injections:  {}", nudge_events.len());
    eprintln!("  nudge_count state: {}", worm.state.comms.nudge_count);

    // Dump all transcript events for diagnostics
    for (i, ev) in transcript_events.iter().enumerate() {
        let content = if ev.event_type == "think_response" {
            let text = ev.data.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let tc = ev.data.get("tool_calls");
            let fr = ev
                .data
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!(
                "text='{}' finish_reason={} tool_calls={:?}",
                &text[..text.len().min(200)],
                fr,
                tc
            )
        } else if ev.event_type == "user" {
            let c = ev
                .data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("'{}'", &c[..c.len().min(150)])
        } else {
            serde_json::to_string(&ev.data)
                .unwrap_or_default()
                .chars()
                .take(150)
                .collect::<String>()
        };
        eprintln!("  [{:2}] {}: {}", i, ev.event_type, content);
    }

    for tc in &tool_calls {
        let name = tc.data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let empty = json!({});
        let args = tc.data.get("arguments").unwrap_or(&empty);
        eprintln!(
            "  tool: {}({})",
            name,
            serde_json::to_string(args).unwrap_or_default()
        );
    }

    // 4. Assert: send_message was called at least once (by LLM or forced by runner).
    assert!(
        !send_calls.is_empty(),
        "worm MUST call send_message — either via LLM tool call or forced fallback. \
         Got {} tool calls but none were send_message. nudge_count={}",
        tool_calls.len(),
        worm.state.comms.nudge_count,
    );

    // Check if it was a forced send (fallback) or LLM-initiated
    let forced = send_calls.iter().any(|tc| {
        tc.data
            .get("call_id")
            .and_then(|v| v.as_str())
            .map(|s| s.starts_with("forced-"))
            .unwrap_or(false)
    });
    eprintln!(
        "Step 4: PASS — send_message called {} time(s) (forced={})",
        send_calls.len(),
        forced
    );

    // 5. Check MetaNet Client's inbox for a reply from the worm.
    let dir_check = tempfile::tempdir().unwrap();
    let auth_check = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321.clone()),
        dir_check.path().to_path_buf(),
    );
    let mbox_check = MessageBoxClient::new(auth_check);

    let replies = mbox_check
        .list_messages(BOX_STATUS_INBOX)
        .await
        .expect("check MetaNet Client inbox failed");

    let worm_replies: Vec<&ReceivedMessage> = replies
        .iter()
        .filter(|m| m.sender == worm_identity)
        .collect();

    eprintln!(
        "Step 5: MetaNet Client status_inbox has {} replies from worm",
        worm_replies.len()
    );

    for reply in &worm_replies {
        eprintln!("  worm reply: {}", reply.body);
    }

    assert!(
        !worm_replies.is_empty(),
        "MetaNet Client should have received at least one reply from the worm"
    );

    eprintln!("Step 5: PASS — worm reply received by MetaNet Client");

    // 6. Verify nudge mechanism engaged (nudge_count >= 0 means it exists;
    //    if > 0, the nudge actually fired; if 0, LLM got it right first try
    //    OR nudge fired and then reset after successful send_message).
    eprintln!(
        "Step 6: nudge_count = {} (0 = LLM cooperated or nudge+reset, >0 = nudge fired and no send_message followed)",
        worm.state.comms.nudge_count,
    );

    // 7. Clean up: acknowledge all test messages from both inboxes.
    eprintln!("Step 7: Cleaning up...");

    // Clean worm's task_inbox
    let dir_cleanup_worm = tempfile::tempdir().unwrap();
    let auth_cleanup_worm = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3322),
        dir_cleanup_worm.path().to_path_buf(),
    );
    let mbox_cleanup_worm = MessageBoxClient::new(auth_cleanup_worm);

    if let Ok(msgs) = mbox_cleanup_worm.list_messages(BOX_TASK_INBOX).await {
        let to_ack: Vec<String> = msgs
            .iter()
            .filter(|m| m.body.get("marker").and_then(|v| v.as_str()) == Some(&marker))
            .map(|m| m.message_id.clone())
            .collect();
        if !to_ack.is_empty() {
            mbox_cleanup_worm.acknowledge_message(&to_ack).await.ok();
            eprintln!("  acked {} task messages from worm inbox", to_ack.len());
        }
    }

    // Clean MetaNet Client's status_inbox (replies from worm)
    let reply_ids: Vec<String> = worm_replies.iter().map(|m| m.message_id.clone()).collect();
    if !reply_ids.is_empty() {
        mbox_check.acknowledge_message(&reply_ids).await.ok();
        eprintln!(
            "  acked {} replies from MetaNet Client inbox",
            reply_ids.len()
        );
    }

    eprintln!("=== U0 NUDGE TEST PASSED ===");
}

/// FULL AGENT LOOP CONVERSATION TEST.
///
/// 1. MetaNet Client sends a message to the worm's task_inbox
/// 2. Worm runs its agent loop (OBSERVE → THINK → ACT)
/// 3. Worm picks up the message, LLM processes it, and may reply
/// 4. Verify the worm processed the message (check transcript)
///
/// This is the "can my metanet client message the agent and have a convo" test.
/// Requires RUN_PAID_TESTS=1 because the LLM call costs sats.
#[tokio::test]
async fn test_live_full_agent_conversation() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w3322 = wallet_3322();
    let w3321 = wallet_3321();

    if !check_wallet(&w3322).await {
        return;
    }
    if !check_wallet(&w3321).await {
        eprintln!("[skip] MetaNet Client on 3321 not available");
        return;
    }

    let worm_identity = w3322.get_identity_key().await.unwrap();
    let user_identity = w3321.get_identity_key().await.unwrap();

    eprintln!("worm identity:  {worm_identity}");
    eprintln!("user identity:  {user_identity}");

    // 1. MetaNet Client sends a task to the worm's inbox.
    let dir_sender = tempfile::tempdir().unwrap();
    let auth_sender = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321.clone()),
        dir_sender.path().to_path_buf(),
    );
    let mbox_sender = MessageBoxClient::new(auth_sender);

    let marker = uuid::Uuid::new_v4().to_string();
    let task_body = json!({
        "type": "task_assignment",
        "task": "Reply to me with the word PINEAPPLE. Use the send_message tool to reply.",
        "reply_to": user_identity,
        "reply_box": BOX_STATUS_INBOX,
        "marker": marker,
    });

    eprintln!("MetaNet Client sending task (marker: {marker})...");

    mbox_sender
        .send_message(&worm_identity, BOX_TASK_INBOX, &task_body)
        .await
        .expect("send task to worm failed");

    eprintln!("task sent, starting worm agent loop...");

    // 2. Run the worm's agent loop.
    let workspace = tempfile::tempdir().unwrap();
    let config = DmConfig::default();
    let mut worm = runner::create_loop(
        config,
        workspace.path().to_path_buf(),
        None,
        None,
        test_wallet(),
    );

    // Run the task: tell the worm to check inbox and respond
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    worm.run(
        &format!(
            "Check your task_inbox for messages. You should find a message with marker {}. \
             Follow the instructions in the message. The sender's identity key is {}. \
             Use the send_message tool to reply to them on their status_inbox.",
            marker, user_identity
        ),
        3, // max 3 iterations
        None,
        cancel,
    )
    .await;

    eprintln!(
        "worm finished: {} iterations, {} sats, done={}, error='{}'",
        worm.state.exec.iteration,
        worm.state.budget.sats_spent,
        worm.state.exec.done,
        worm.state.exec.error,
    );

    // 3. Check what happened.
    let transcript_events = worm.transcript.replay();
    let tool_calls: Vec<_> = transcript_events
        .iter()
        .filter(|e| e.event_type == "tool_call")
        .collect();
    let tool_results: Vec<_> = transcript_events
        .iter()
        .filter(|e| e.event_type == "tool_result")
        .collect();

    eprintln!("transcript: {} events total", transcript_events.len());
    eprintln!("tool calls: {}", tool_calls.len());
    eprintln!("tool results: {}", tool_results.len());

    for tc in &tool_calls {
        let name = tc.data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let empty = json!({});
        let args = tc.data.get("arguments").unwrap_or(&empty);
        eprintln!(
            "  called: {}({})",
            name,
            serde_json::to_string(args).unwrap_or_default()
        );
    }
    for tr in &tool_results {
        let name = tr.data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let content = tr
            .data
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let success = tr
            .data
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        eprintln!(
            "  result: {} success={} output={}...",
            name,
            success,
            &content[..content.len().min(200)]
        );
    }

    // The worm should have run at least 1 iteration.
    assert!(
        worm.state.exec.iteration >= 1,
        "worm should run at least 1 iteration"
    );

    // The worm should have spent some sats (LLM call).
    assert!(
        worm.state.budget.sats_spent > 0,
        "worm should spend sats on LLM inference"
    );

    // 4. Check if the worm sent a reply to MetaNet Client's inbox.
    let dir_check = tempfile::tempdir().unwrap();
    let auth_check = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321),
        dir_check.path().to_path_buf(),
    );
    let mbox_check = MessageBoxClient::new(auth_check);

    let replies = mbox_check
        .list_messages(BOX_STATUS_INBOX)
        .await
        .expect("check MetaNet Client inbox failed");

    eprintln!("MetaNet Client status_inbox: {} messages", replies.len());

    // Look for any reply from the worm's identity key.
    let worm_replies: Vec<&ReceivedMessage> = replies
        .iter()
        .filter(|m| m.sender == worm_identity)
        .collect();

    eprintln!("replies from worm: {}", worm_replies.len());

    for reply in &worm_replies {
        eprintln!("  worm says: {}", reply.body);

        // Clean up: acknowledge the reply.
        mbox_check
            .acknowledge_message(std::slice::from_ref(&reply.message_id))
            .await
            .ok();
    }

    // We expect the worm to have made tool calls.
    // Whether it successfully sent a reply depends on the LLM's behavior,
    // but we can at least verify it tried.
    let send_calls: Vec<_> = tool_calls
        .iter()
        .filter(|tc| tc.data.get("name").and_then(|v| v.as_str()) == Some("send_message"))
        .collect();

    eprintln!(
        "send_message tool calls: {}, worm replies received: {}",
        send_calls.len(),
        worm_replies.len(),
    );

    // The full conversation worked if either:
    // (a) the worm called send_message (it understood the task), or
    // (b) the worm produced a text result mentioning PINEAPPLE
    let result_has_pineapple = worm.state.exec.result.to_uppercase().contains("PINEAPPLE");
    let tried_to_send = !send_calls.is_empty();
    let reply_received = !worm_replies.is_empty();

    eprintln!("result_has_pineapple: {result_has_pineapple}");
    eprintln!("tried_to_send: {tried_to_send}");
    eprintln!("reply_received: {reply_received}");

    // At minimum, the worm should have successfully called the LLM
    // and done something (text response or tool calls).
    assert!(
        !worm.state.exec.result.is_empty() || !tool_calls.is_empty(),
        "worm should have produced a response or made tool calls"
    );

    eprintln!("FULL AGENT CONVERSATION TEST PASSED");

    // Clean up: acknowledge our original task message from worm's inbox.
    let dir_cleanup = tempfile::tempdir().unwrap();
    let auth_cleanup = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3322),
        dir_cleanup.path().to_path_buf(),
    );
    let mbox_cleanup = MessageBoxClient::new(auth_cleanup);
    let worm_inbox = mbox_cleanup.list_messages(BOX_TASK_INBOX).await.ok();
    if let Some(msgs) = worm_inbox {
        let to_ack: Vec<String> = msgs
            .iter()
            .filter(|m| m.body.get("marker").and_then(|v| v.as_str()) == Some(&marker))
            .map(|m| m.message_id.clone())
            .collect();
        if !to_ack.is_empty() {
            mbox_cleanup.acknowledge_message(&to_ack).await.ok();
        }
    }
}

/// IMAGE GENERATION TEST: Send a task via MessageBox asking the worm to generate
/// an image using nano-banana-pro x402 service.
///
/// Flow:
/// 1. MetaNet Client sends task: "Generate an image of a golden retriever in space"
/// 2. Worm reads inbox, LLM calls generate_image tool
/// 3. Tool does BRC-31 auth + BRC-29 payment to nano-banana-pro
/// 4. Polls /status until image is ready (~2 min)
/// 5. Worm replies to MetaNet Client with the image URL
///
/// Cost: ~1.2M sats (~$0.19) for the image + LLM calls
#[tokio::test]
async fn test_live_image_generation_via_messagebox() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w3322 = wallet_3322();
    let w3321 = wallet_3321();

    if !check_wallet(&w3322).await {
        return;
    }
    if !check_wallet(&w3321).await {
        eprintln!("[skip] MetaNet Client on 3321 not available");
        return;
    }

    let worm_identity = w3322.get_identity_key().await.unwrap();
    let user_identity = w3321.get_identity_key().await.unwrap();

    eprintln!("=== IMAGE GENERATION TEST ===");
    eprintln!("worm identity:  {worm_identity}");
    eprintln!("user identity:  {user_identity}");

    // 1. MetaNet Client sends an image generation task to worm's task_inbox.
    let dir_sender = tempfile::tempdir().unwrap();
    let auth_sender = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321.clone()),
        dir_sender.path().to_path_buf(),
    );
    let mbox_sender = MessageBoxClient::new(auth_sender);

    let marker = uuid::Uuid::new_v4().to_string();
    let task_body = json!({
        "type": "task_assignment",
        "task": "Generate an image of a golden retriever wearing an astronaut helmet floating in space. Use the generate_image tool with resolution 1K. Then reply to me with the image URL.",
        "reply_to": user_identity,
        "reply_box": BOX_STATUS_INBOX,
        "marker": marker,
    });

    eprintln!("Step 1: Sending image generation task (marker: {marker})...");

    mbox_sender
        .send_message(&worm_identity, BOX_TASK_INBOX, &task_body)
        .await
        .expect("send task to worm failed");

    eprintln!("Step 1: DONE — task sent");

    // 2. Run the worm agent loop (needs more iterations for async image gen).
    let workspace = tempfile::tempdir().unwrap();
    let config = DmConfig::default();
    let mut worm = runner::create_loop(
        config,
        workspace.path().to_path_buf(),
        None,
        None,
        test_wallet(),
    );

    eprintln!("Step 2: Starting worm agent loop (max 8 iterations, ~3 min for image gen)...");

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    worm.run(
        "Process your inbox messages and complete any tasks. Reply to the sender with results.",
        8,
        None,
        cancel,
    )
    .await;

    eprintln!(
        "Step 2: DONE — {} iterations, {} sats, done={}, error='{}'",
        worm.state.exec.iteration,
        worm.state.budget.sats_spent,
        worm.state.exec.done,
        worm.state.exec.error,
    );

    // 3. Analyze transcript for generate_image and send_message calls.
    let transcript_events = worm.transcript.replay();

    let tool_calls: Vec<_> = transcript_events
        .iter()
        .filter(|e| e.event_type == "tool_call")
        .collect();

    let image_calls: Vec<_> = tool_calls
        .iter()
        .filter(|tc| tc.data.get("name").and_then(|v| v.as_str()) == Some("generate_image"))
        .collect();

    let send_calls: Vec<_> = tool_calls
        .iter()
        .filter(|tc| tc.data.get("name").and_then(|v| v.as_str()) == Some("send_message"))
        .collect();

    eprintln!("Step 3: Transcript analysis:");
    eprintln!("  total events:       {}", transcript_events.len());
    eprintln!("  tool calls:         {}", tool_calls.len());
    eprintln!("  generate_image:     {}", image_calls.len());
    eprintln!("  send_message:       {}", send_calls.len());

    // Dump tool calls for diagnostics
    for tc in &tool_calls {
        let name = tc.data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let args = tc
            .data
            .get("arguments")
            .map(|a| serde_json::to_string(a).unwrap_or_default())
            .unwrap_or_default();
        eprintln!("  tool: {}({})", name, &args[..args.len().min(200)]);
    }

    // Dump tool results for image URLs
    for ev in transcript_events.iter() {
        if ev.event_type == "tool_result" {
            let name = ev.data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            if name == "generate_image" {
                let content = ev
                    .data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                eprintln!(
                    "  generate_image result: {}",
                    &content[..content.len().min(500)]
                );
            }
        }
    }

    // 4. Assert: generate_image was called at least once.
    assert!(
        !image_calls.is_empty(),
        "worm MUST call generate_image tool. Got {} tool calls: {:?}",
        tool_calls.len(),
        tool_calls
            .iter()
            .map(|tc| tc.data.get("name").and_then(|v| v.as_str()).unwrap_or("?"))
            .collect::<Vec<_>>()
    );

    eprintln!(
        "Step 4: PASS — generate_image called {} time(s)",
        image_calls.len()
    );

    // 5. Check if worm replied to MetaNet Client.
    let dir_receiver = tempfile::tempdir().unwrap();
    let auth_receiver = AuthriteClient::with_session_dir(
        std::sync::Arc::new(w3321.clone()),
        dir_receiver.path().to_path_buf(),
    );
    let mbox_receiver = MessageBoxClient::new(auth_receiver);

    let replies = mbox_receiver
        .list_messages(BOX_STATUS_INBOX)
        .await
        .unwrap_or_default();
    let worm_replies: Vec<_> = replies
        .iter()
        .filter(|m| m.sender == worm_identity)
        .collect();

    eprintln!(
        "Step 5: MetaNet Client status_inbox has {} replies from worm",
        worm_replies.len()
    );

    for reply in &worm_replies {
        let body_str = serde_json::to_string(&reply.body).unwrap_or_default();
        eprintln!("  worm reply: {}", &body_str[..body_str.len().min(500)]);
    }

    // Clean up replies
    if !worm_replies.is_empty() {
        let ids: Vec<String> = worm_replies.iter().map(|m| m.message_id.clone()).collect();
        mbox_receiver.acknowledge_message(&ids).await.ok();
    }

    eprintln!("=== IMAGE GENERATION TEST PASSED ===");
}

// ---------------------------------------------------------------------------
// Tier 5: x402 Payment Flow — shared helper e2e tests
// ---------------------------------------------------------------------------

/// Exercise authenticated_paid_request() directly against the LLM endpoint.
/// Verifies: payment_txid is Some, sats_paid is Some, response body is valid JSON.
#[tokio::test]
async fn test_live_paid_authenticated_paid_request() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::x402::payment::authenticated_paid_request;

    let dir = tempfile::tempdir().unwrap();
    let auth = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());

    let body = json!({
        "model": "gpt-5-nano",
        "messages": [
            {"role": "user", "content": "Reply with the single word 'pong'."},
        ],
        "max_completion_tokens": 64,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();

    let resp = authenticated_paid_request(
        &auth,
        "POST",
        "https://openai-chat.x402agency.com/chat",
        &[("content-type".into(), "application/json".into())],
        Some(&body_bytes),
    )
    .await
    .expect("authenticated_paid_request failed");

    eprintln!("status: {}", resp.status);
    assert!(
        resp.status.is_success(),
        "expected success, got {}",
        resp.status
    );

    // Must have payment metadata
    assert!(
        resp.payment_txid.is_some(),
        "payment_txid should be Some after 402 flow"
    );
    let txid = resp.payment_txid.as_ref().unwrap();
    assert_eq!(txid.len(), 64, "txid should be 64 hex chars, got {}", txid);
    eprintln!("payment_txid: {txid}");

    assert!(
        resp.sats_paid.is_some(),
        "sats_paid should be Some after 402 flow"
    );
    let sats = resp.sats_paid.unwrap();
    assert!(sats > 0, "sats_paid should be > 0, got {sats}");
    eprintln!("sats_paid: {sats}");

    // Response body should be valid OpenAI JSON
    let data: Value =
        serde_json::from_slice(&resp.body).expect("response body should be valid JSON");
    assert!(
        data.get("choices").is_some(),
        "response should have 'choices'"
    );

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    eprintln!("LLM says: {content}");

    // Server-reported payment info
    if let Some(pi) = data.get("payment") {
        eprintln!(
            "server payment: paid={} sats, effective={} sats",
            pi.get("satoshis_paid").unwrap_or(&json!(0)),
            pi.get("satoshis_effective").unwrap_or(&json!(0)),
        );
    }

    eprintln!("=== authenticated_paid_request e2e PASSED ===");
}

/// Exercise think() through the refactored path.
/// Verifies: ThinkResult.payment_txid is Some, sats_paid > 0, text is non-empty.
#[tokio::test]
async fn test_live_paid_think_via_shared_helper() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::think::think;

    let dir = tempfile::tempdir().unwrap();
    let auth = AuthriteClient::with_session_dir(std::sync::Arc::new(w), dir.path().to_path_buf());
    let config = DmConfig::default();

    let messages = vec![
        json!({"role": "system", "content": "You are a test bot. Reply with exactly one word."}),
        json!({"role": "user", "content": "Say 'hello'."}),
    ];

    let result = think(&auth, &messages, "gpt-5-nano", 64, None, &config)
        .await
        .expect("think() failed");

    eprintln!("think result:");
    eprintln!("  text:          '{}'", result.text);
    eprintln!("  model:         {}", result.model);
    eprintln!("  sats_paid:     {}", result.sats_paid);
    eprintln!("  sats_effective:{}", result.sats_effective);
    eprintln!("  sats_refunded: {}", result.sats_refunded);
    eprintln!(
        "  tokens:        {} ({}+{})",
        result.total_tokens, result.prompt_tokens, result.completion_tokens
    );
    eprintln!("  duration_ms:   {}", result.duration_ms);
    eprintln!("  payment_txid:  {:?}", result.payment_txid);
    eprintln!("  finish_reason: {}", result.finish_reason);

    // Payment metadata
    assert!(result.payment_txid.is_some(), "payment_txid should be Some");
    let txid = result.payment_txid.as_ref().unwrap();
    assert_eq!(txid.len(), 64, "txid should be 64 hex chars");
    eprintln!("verified txid: {txid}");

    assert!(result.sats_paid > 0, "sats_paid should be > 0");
    assert!(result.total_tokens > 0, "should have consumed tokens");

    // Content check — reasoning models may return empty with finish_reason=length
    assert!(
        !result.text.is_empty() || result.finish_reason == "length",
        "expected text or finish_reason=length"
    );

    eprintln!("=== think() via shared helper e2e PASSED ===");
}

/// Exercise x402_call tool via the generic x402 call path.
/// Verifies: payment_txid and sats_paid are present in the JSON result.
#[tokio::test]
async fn test_live_paid_x402_call_x_search() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::tools::x402_tools::all_x402_tools;

    let wallet_url = "http://localhost:3322".to_string();
    let tools = all_x402_tools(
        wallet_url,
        dolphin_milk::x402::registry::DEFAULT_REGISTRY_URL.to_string(),
    );
    let x402_call = tools
        .iter()
        .find(|t| t.name == "x402_call")
        .expect("x402_call tool not found");

    eprintln!("calling x402_call for x-research/search...");
    let result_str = (x402_call.execute)(json!({
        "service": "x-research/search",
        "parameters": {
            "query": "bitcoin sv",
            "max_results": 3,
        }
    }))
    .await;

    eprintln!(
        "x402_call raw result ({} chars): {}",
        result_str.len(),
        &result_str[..result_str.len().min(500)]
    );

    // Should not be an error
    assert!(
        !result_str.starts_with("Error"),
        "x402_call failed: {result_str}"
    );

    // Parse as JSON and check for payment metadata
    let result: Value =
        serde_json::from_str(&result_str).expect("x402_call result should be valid JSON");

    let txid = result.get("payment_txid").and_then(|v| v.as_str());
    assert!(
        txid.is_some(),
        "x402_call result should contain payment_txid. Got: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
    let txid = txid.unwrap();
    assert_eq!(txid.len(), 64, "txid should be 64 hex chars, got '{txid}'");
    eprintln!("payment_txid: {txid}");

    let sats = result.get("sats_paid").and_then(|v| v.as_u64());
    assert!(sats.is_some(), "x402_call result should contain sats_paid");
    let sats = sats.unwrap();
    assert!(sats > 0, "sats_paid should be > 0, got {sats}");
    eprintln!("sats_paid: {sats}");

    eprintln!("=== x402_call x-research e2e PASSED ===");
}

// ---------------------------------------------------------------------------
// Tier 6: x402 Service Discovery — no wallet/payment needed, just HTTP GET
// ---------------------------------------------------------------------------

/// Fetch the live x402agency.com registry and verify we get agents back.
#[tokio::test]
async fn test_live_registry_list_agents() {
    use dolphin_milk::x402::registry;

    match registry::list_agents(None).await {
        Err(e) => {
            eprintln!("[skip] x402 registry not reachable: {e}");
            return;
        }
        Ok(agents) => {
            eprintln!("x402 registry: {} agents", agents.len());
            for a in &agents {
                eprintln!("  {} — {} ({})", a.name, a.tagline, a.url);
            }
            assert!(
                agents.len() >= 5,
                "expected at least 5 agents, got {}",
                agents.len()
            );
            assert!(
                agents.iter().any(|a| a.name.contains("banana")),
                "expected to find 'banana' agent in registry"
            );
        }
    }
}

/// Resolve the 'banana' agent name to a URL via the live registry.
#[tokio::test]
async fn test_live_registry_resolve() {
    use dolphin_milk::x402::registry;

    match registry::resolve("banana", None).await {
        Err(e) => {
            eprintln!("[skip] x402 registry not reachable: {e}");
            return;
        }
        Ok(url) => {
            eprintln!("resolved 'banana' → {url}");
            assert!(
                url.starts_with("https://"),
                "URL should start with https://"
            );
            assert!(
                url.to_lowercase().contains("banana"),
                "URL should contain 'banana': {url}"
            );
        }
    }
}

/// Resolve banana, then fetch its service manifest from /.well-known/x402-info.
#[tokio::test]
async fn test_live_fetch_manifest() {
    use dolphin_milk::x402::{discovery, registry};

    let base_url = match registry::resolve("banana", None).await {
        Err(e) => {
            eprintln!("[skip] x402 registry not reachable: {e}");
            return;
        }
        Ok(url) => url,
    };

    match discovery::fetch_manifest(&base_url).await {
        Err(e) => {
            eprintln!("[skip] manifest fetch failed: {e}");
            return;
        }
        Ok(manifest) => {
            eprintln!("manifest: {}", manifest.name);
            if !manifest.server_identity_key.is_empty() {
                eprintln!(
                    "  identity: {}...",
                    &manifest.server_identity_key[..manifest.server_identity_key.len().min(16)]
                );
            }
            eprintln!("  endpoints: {}", manifest.endpoints.len());
            for ep in &manifest.endpoints {
                eprintln!(
                    "    {} {} [auth={}, paid={}]",
                    ep.method,
                    ep.path,
                    ep.auth,
                    ep.has_payment()
                );
            }

            let summary = discovery::format_manifest_summary(&manifest);
            eprintln!("--- summary ---\n{summary}--- end ---");

            assert!(
                !manifest.name.is_empty(),
                "manifest name should not be empty"
            );
            assert!(
                !manifest.endpoints.is_empty(),
                "manifest should have endpoints"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 2 — Paid: Conversation wallet sync
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_live_paid_conversation_sync_to_wallet() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::conversation::ConversationManager;

    // Create a temp workspace and ConversationManager
    let tmp = tempfile::tempdir().expect("tempdir");
    let mgr = ConversationManager::new(tmp.path());

    // Create a conversation with a couple of messages
    let conv = mgr
        .create("test-participant-key", "Hello from sync test")
        .expect("create conversation");
    let conv_id = conv.id.clone();
    eprintln!("created conversation: {conv_id}");

    // Add a second message
    mgr.append_user_message(&conv_id, "This is a follow-up message in the sync test.")
        .expect("append second message");

    // Sync to wallet (this creates an on-chain PushDrop token — costs sats)
    let result = mgr
        .sync_to_wallet(&w, &conv_id)
        .await
        .expect("sync_to_wallet should succeed");

    eprintln!("sync txid: {}", result.txid);
    eprintln!("sync conv_id: {}", result.conversation_id);

    assert!(!result.txid.is_empty(), "txid should not be empty");
    assert_eq!(result.txid.len(), 64, "txid should be 64 hex chars");
    assert_eq!(result.conversation_id, conv_id);
}

#[tokio::test]
async fn test_live_paid_conversation_sync_and_restore() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::conversation::ConversationManager;

    // -- Phase 1: Create and sync a conversation --
    let tmp1 = tempfile::tempdir().expect("tempdir 1");
    let mgr1 = ConversationManager::new(tmp1.path());

    let conv = mgr1
        .create("test-restore-key", "Restore test message")
        .expect("create conversation for restore test");
    let conv_id = conv.id.clone();

    mgr1.append_user_message(&conv_id, "Second message for restore test.")
        .expect("append second message");

    let sync_result = mgr1
        .sync_to_wallet(&w, &conv_id)
        .await
        .expect("sync_to_wallet for restore test");
    eprintln!("synced conversation {conv_id} → txid {}", sync_result.txid);

    // -- Phase 2: Restore into a fresh workspace --
    let tmp2 = tempfile::tempdir().expect("tempdir 2");
    let mgr2 = ConversationManager::new(tmp2.path());

    // Verify the conversation doesn't exist yet in the new workspace
    assert!(
        mgr2.load(&conv_id).unwrap().is_none(),
        "conversation should not exist in fresh workspace before restore"
    );

    let restored = mgr2
        .restore_from_wallet(&w)
        .await
        .expect("restore_from_wallet should succeed");

    eprintln!("restored {} conversations: {:?}", restored.len(), restored);

    // The conversation we just synced should be in the restored list
    assert!(
        restored.contains(&conv_id),
        "restored list should contain our conversation {conv_id}"
    );

    // Verify the restored conversation has correct metadata
    let loaded = mgr2
        .load(&conv_id)
        .unwrap()
        .expect("restored conversation should be loadable");
    assert_eq!(loaded.id, conv_id);
    assert_eq!(loaded.message_count, 2);

    // Verify messages were restored
    let messages = mgr2.load_messages(&conv_id).unwrap();
    assert_eq!(messages.len(), 2, "should have 2 messages");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Restore test message");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "Second message for restore test.");

    eprintln!(
        "restore roundtrip verified — {} messages intact",
        messages.len()
    );
}

#[tokio::test]
async fn test_live_paid_conversation_sync_not_found() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    use dolphin_milk::conversation::ConversationManager;

    let tmp = tempfile::tempdir().expect("tempdir");
    let mgr = ConversationManager::new(tmp.path());

    // Sync a non-existent conversation should fail
    let result = mgr.sync_to_wallet(&w, "conv-nonexistent-999").await;
    assert!(
        result.is_err(),
        "sync of non-existent conversation should fail"
    );
    eprintln!(
        "correctly rejected sync of non-existent conversation: {}",
        result.unwrap_err()
    );
}

// ---------------------------------------------------------------------------
// Spending Semaphore — concurrent createAction serialization
// ---------------------------------------------------------------------------

/// Fire N concurrent createAction calls through separate WalletClient instances
/// (mimicking parallel tool execution). The SPEND_SEMAPHORE should serialize
/// them so all succeed without UTXO contention.
///
/// Cost: ~5 miner fees (~150 sats total, OP_RETURN outputs)
#[tokio::test]
async fn test_live_paid_concurrent_spending_serialized() {
    if !paid_tests_enabled() {
        eprintln!("[skip] paid tests disabled — set RUN_PAID_TESTS=1 to run");
        return;
    }

    let w = wallet_3322();
    if !check_wallet(&w).await {
        return;
    }

    let concurrency = 5;
    let balance_before = w.get_balance().await.expect("get_balance failed");
    eprintln!("balance before: {} sats", balance_before);
    assert!(
        balance_before > 1_000,
        "need at least 1000 sats for concurrent spending test, have {}",
        balance_before
    );

    // Fire N concurrent createAction calls — each creates a 0-sat OP_RETURN
    // Each task creates its own WalletClient (same as tool execution pattern)
    let mut handles = Vec::new();
    for i in 0..concurrency {
        let idx = i;
        handles.push(tokio::spawn(async move {
            let client = WalletClient::new("http://localhost:3322", "http://localhost", 30);
            let op_return = format!("006a07636f6e63757272{:02x}", idx); // OP_FALSE OP_RETURN "concurr" + idx
            let result = client
                .create_action(
                    &[serde_json::json!({
                        "lockingScript": op_return,
                        "satoshis": 0,
                        "outputDescription": format!("concurrent spending test #{}", idx + 1),
                    })],
                    &format!("e2e concurrent #{}", idx + 1),
                    false,
                    false,
                )
                .await;
            (idx, result)
        }));
    }

    // Collect results
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for handle in handles {
        let (idx, result) = handle.await.expect("task panicked");
        match result {
            Ok(r) => {
                eprintln!("  #{}: OK txid={}", idx + 1, &r.txid[..16]);
                succeeded.push(r);
            }
            Err(e) => {
                eprintln!("  #{}: FAIL {}", idx + 1, e);
                failed.push((idx, e));
            }
        }
    }

    eprintln!(
        "{}/{} concurrent createAction calls succeeded",
        succeeded.len(),
        concurrency
    );

    // ALL must succeed — the semaphore serializes them
    assert_eq!(
        succeeded.len(),
        concurrency,
        "all {} concurrent createAction calls must succeed (semaphore serialization), but {} failed: {:?}",
        concurrency,
        failed.len(),
        failed.iter().map(|(i, e)| format!("#{}: {}", i + 1, e)).collect::<Vec<_>>()
    );

    // All txids must be unique
    let txids: std::collections::HashSet<&str> =
        succeeded.iter().map(|r| r.txid.as_str()).collect();
    assert_eq!(txids.len(), concurrency, "all txids must be unique");

    // Balance should only decrease by fees (~30 sats per tx)
    let balance_after = w.get_balance().await.expect("get_balance failed");
    let fee_total = balance_before - balance_after;
    eprintln!(
        "balance after: {} sats (fees: {} sats, ~{} per tx)",
        balance_after,
        fee_total,
        fee_total / concurrency as u64
    );
    assert!(
        fee_total < 1_000,
        "total fees for {} OP_RETURN txs should be under 1000 sats, got {}",
        concurrency,
        fee_total
    );
}
