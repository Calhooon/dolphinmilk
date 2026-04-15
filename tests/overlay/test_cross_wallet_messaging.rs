//! Cross-wallet BRC-33 messaging tests — two real wallets exchange messages
//! via the live MessageBox relay at messagebox.babbage.systems.
//!
//! Requires:
//!   - Wallet A on localhost:3322 (funded BSV wallet)
//!   - Wallet B on localhost:3323 (funded BSV wallet, different identity key)
//!   - Live internet access to messagebox.babbage.systems
//!
//! All tests are `#[ignore]` — run with:
//! ```sh
//! cargo test --test test_cross_wallet_messaging -- --ignored --nocapture
//! ```

use dolphin_milk::auth::AuthriteClient;
use dolphin_milk::messagebox::{compute_message_hash, MessageBoxClient};
use dolphin_milk::wallet::{HttpWalletClient, WalletBackend};
use serde_json::json;
use std::sync::Arc;

const WALLET_A_URL: &str = "http://localhost:3322";
const WALLET_B_URL: &str = "http://localhost:3323";

/// Inbox box used for cross-wallet tests.
const TEST_BOX: &str = "task_inbox";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn wallet_a() -> HttpWalletClient {
    HttpWalletClient::new(WALLET_A_URL, "http://localhost", 30)
}

fn wallet_b() -> HttpWalletClient {
    HttpWalletClient::new(WALLET_B_URL, "http://localhost", 30)
}

fn mbox_for(wallet: Arc<dyn WalletBackend + Send + Sync>) -> MessageBoxClient {
    let dir = tempfile::tempdir().expect("tempdir failed");
    let path = dir.path().to_path_buf();
    // Keep dir alive — intentionally leak so the session directory persists for the test duration.
    std::mem::forget(dir);
    let auth = AuthriteClient::with_session_dir(wallet, path);
    MessageBoxClient::new(auth)
}

/// Check both wallets are reachable and return distinct identity keys.
async fn check_both_wallets() -> bool {
    let wa = wallet_a();
    let wb = wallet_b();
    let (a, b) = tokio::join!(wa.is_authenticated(), wb.is_authenticated());

    if let Err(e) = a {
        eprintln!("[SKIP] Wallet A at {WALLET_A_URL} not reachable: {e}");
        return false;
    }
    if let Err(e) = b {
        eprintln!("[SKIP] Wallet B at {WALLET_B_URL} not reachable: {e}");
        return false;
    }

    // Verify they are actually different wallets
    let key_a = wallet_a().get_identity_key().await;
    let key_b = wallet_b().get_identity_key().await;
    match (key_a, key_b) {
        (Ok(a), Ok(b)) => {
            if a == b {
                eprintln!(
                    "[SKIP] Wallet A and B have the same identity key — need two distinct wallets"
                );
                return false;
            }
            eprintln!("Wallet A identity: {}", &a[..20]);
            eprintln!("Wallet B identity: {}", &b[..20]);
            true
        }
        _ => {
            eprintln!("[SKIP] Could not get identity keys from both wallets");
            false
        }
    }
}

/// Drain all messages from a box for a given mbox client, acknowledging each.
/// This prevents pollution between test runs.
async fn drain_box(mbox: &MessageBoxClient, box_name: &str) {
    match mbox.list_messages(box_name).await {
        Ok(msgs) if !msgs.is_empty() => {
            let ids: Vec<String> = msgs.iter().map(|m| m.message_id.clone()).collect();
            eprintln!(
                "  Draining {} existing message(s) from {box_name}",
                ids.len()
            );
            let _ = mbox.acknowledge_message(&ids).await;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Test 1: Plain message A -> B
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_send_plain_message_a_to_b() {
    if !check_both_wallets().await {
        return;
    }

    let wa: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb.get_identity_key().await.expect("get identity B");
    let identity_a = wa.get_identity_key().await.expect("get identity A");

    let mbox_a = mbox_for(wa);
    let mbox_b = mbox_for(wb);

    // Drain B's inbox to start clean
    drain_box(&mbox_b, TEST_BOX).await;

    // Build a unique test message body
    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "test_plain",
        "nonce": nonce,
        "greeting": "Hello from Wallet A"
    });

    // A sends to B
    eprintln!("Sending plain message from A to B...");
    let send_result = mbox_a
        .send_message(&identity_b, TEST_BOX, &body)
        .await
        .expect("send_message A->B failed");

    eprintln!(
        "Send result: {}",
        serde_json::to_string_pretty(&send_result).unwrap()
    );
    assert!(
        send_result.get("sentMessageId").is_some(),
        "send result should contain sentMessageId"
    );
    assert!(
        send_result.get("message_hash").is_some(),
        "send result should contain message_hash"
    );

    // B lists messages
    eprintln!("B listing messages from {TEST_BOX}...");
    let messages = mbox_b
        .list_messages(TEST_BOX)
        .await
        .expect("listMessages on B failed");

    eprintln!("B received {} message(s)", messages.len());

    // Find our test message by nonce
    let our_msg = messages
        .iter()
        .find(|m| m.body.get("nonce").and_then(|v| v.as_str()) == Some(&nonce));

    assert!(
        our_msg.is_some(),
        "B should have received the message with our nonce"
    );
    let msg = our_msg.unwrap();

    // Verify sender
    assert_eq!(
        msg.sender, identity_a,
        "sender should be wallet A's identity key"
    );

    // Verify body content
    assert_eq!(
        msg.body.get("greeting").and_then(|v| v.as_str()),
        Some("Hello from Wallet A"),
        "message body should match what A sent"
    );

    eprintln!("Plain message body matches! msg_id={}", msg.message_id);

    // Acknowledge to clean up
    mbox_b
        .acknowledge_message(std::slice::from_ref(&msg.message_id))
        .await
        .expect("acknowledge failed");

    eprintln!("test_send_plain_message_a_to_b PASSED");
}

// ---------------------------------------------------------------------------
// Test 2: BRC-77 signed message A -> B
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_send_signed_message_a_to_b() {
    if !check_both_wallets().await {
        return;
    }

    let wa: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa);
    let mbox_b = mbox_for(wb);

    drain_box(&mbox_b, TEST_BOX).await;

    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "test_signed",
        "nonce": nonce,
        "data": "This message is BRC-77 signed"
    });

    // A sends signed message to B
    eprintln!("Sending BRC-77 signed message from A to B...");
    let send_result = mbox_a
        .send_signed_message(&identity_b, TEST_BOX, &body)
        .await
        .expect("send_signed_message A->B failed");

    eprintln!(
        "Signed send result: {}",
        serde_json::to_string_pretty(&send_result).unwrap()
    );

    // B lists and finds the message
    let messages = mbox_b
        .list_messages(TEST_BOX)
        .await
        .expect("listMessages on B failed");

    // The signed message body is a SignedMessage struct — find by looking for our nonce inside it
    let our_msg = messages.iter().find(|m| {
        // The body is a SignedMessage { body, signature, sender_key }
        // The inner body contains our nonce
        let inner = m.body.get("body");
        inner.and_then(|b| b.get("nonce")).and_then(|v| v.as_str()) == Some(&nonce)
    });

    assert!(
        our_msg.is_some(),
        "B should have received the signed message"
    );
    let msg = our_msg.unwrap();

    // Process the received message — should detect signature and verify
    eprintln!("B processing received message...");
    let processed = mbox_b
        .process_received_message(msg)
        .await
        .expect("process_received_message failed");

    eprintln!(
        "Processed: was_signed={}, signature_valid={:?}, was_encrypted={}",
        processed.was_signed, processed.signature_valid, processed.was_encrypted
    );

    assert!(processed.was_signed, "message should be detected as signed");
    assert_eq!(
        processed.signature_valid,
        Some(true),
        "BRC-77 signature should verify successfully"
    );
    assert!(!processed.was_encrypted, "message should NOT be encrypted");

    // Verify the unwrapped body matches original
    assert_eq!(
        processed.body.get("nonce").and_then(|v| v.as_str()),
        Some(nonce.as_str()),
        "processed body should contain original nonce"
    );
    assert_eq!(
        processed.body.get("data").and_then(|v| v.as_str()),
        Some("This message is BRC-77 signed"),
        "processed body should contain original data"
    );

    // Acknowledge
    mbox_b
        .acknowledge_message(std::slice::from_ref(&msg.message_id))
        .await
        .expect("acknowledge failed");

    eprintln!("test_send_signed_message_a_to_b PASSED");
}

// ---------------------------------------------------------------------------
// Test 3: BRC-78 encrypted message A -> B
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_send_encrypted_message_a_to_b() {
    if !check_both_wallets().await {
        return;
    }

    let wa: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa);
    let mbox_b = mbox_for(wb);

    drain_box(&mbox_b, TEST_BOX).await;

    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "test_encrypted",
        "nonce": nonce,
        "secret": "This is a BRC-78 encrypted secret"
    });

    // A sends encrypted message to B
    eprintln!("Sending BRC-78 encrypted message from A to B...");
    let send_result = mbox_a
        .send_encrypted_message(&identity_b, TEST_BOX, &body)
        .await
        .expect("send_encrypted_message A->B failed");

    eprintln!(
        "Encrypted send result: {}",
        serde_json::to_string_pretty(&send_result).unwrap()
    );

    // B lists and finds the encrypted message
    let messages = mbox_b
        .list_messages(TEST_BOX)
        .await
        .expect("listMessages on B failed");

    // Encrypted message body has { encrypted: true, ciphertext: "...", sender_key: "..." }
    let our_msg = messages.iter().find(|m| {
        m.body
            .get("encrypted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && m.body.get("ciphertext").is_some()
    });

    assert!(
        our_msg.is_some(),
        "B should have received an encrypted message"
    );
    let msg = our_msg.unwrap();

    // The raw body should NOT contain the plaintext
    let raw_body_str = serde_json::to_string(&msg.body).unwrap();
    assert!(
        !raw_body_str.contains("This is a BRC-78 encrypted secret"),
        "encrypted body should NOT contain plaintext"
    );

    // Process the received message — should decrypt
    eprintln!("B processing (decrypting) received message...");
    let processed = mbox_b
        .process_received_message(msg)
        .await
        .expect("process_received_message failed");

    eprintln!(
        "Processed: was_encrypted={}, was_signed={}, signature_valid={:?}",
        processed.was_encrypted, processed.was_signed, processed.signature_valid
    );

    assert!(
        processed.was_encrypted,
        "message should be detected as encrypted"
    );
    assert!(!processed.was_signed, "message should NOT be signed");

    // Verify decrypted body matches original
    assert_eq!(
        processed.body.get("nonce").and_then(|v| v.as_str()),
        Some(nonce.as_str()),
        "decrypted body should contain original nonce"
    );
    assert_eq!(
        processed.body.get("secret").and_then(|v| v.as_str()),
        Some("This is a BRC-78 encrypted secret"),
        "decrypted body should contain original secret"
    );

    // Acknowledge
    mbox_b
        .acknowledge_message(std::slice::from_ref(&msg.message_id))
        .await
        .expect("acknowledge failed");

    eprintln!("test_send_encrypted_message_a_to_b PASSED");
}

// ---------------------------------------------------------------------------
// Test 4: BRC-77 signed + BRC-78 encrypted message (full stack)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_send_signed_encrypted_message() {
    if !check_both_wallets().await {
        return;
    }

    let wa: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa);
    let mbox_b = mbox_for(wb);

    drain_box(&mbox_b, TEST_BOX).await;

    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "test_signed_encrypted",
        "nonce": nonce,
        "payload": "Signed then encrypted — full BRC-77 + BRC-78 stack"
    });

    // A sends signed+encrypted message to B
    eprintln!("Sending BRC-77+BRC-78 signed+encrypted message from A to B...");
    let send_result = mbox_a
        .send_signed_encrypted_message(&identity_b, TEST_BOX, &body)
        .await
        .expect("send_signed_encrypted_message A->B failed");

    eprintln!(
        "Signed+encrypted send result: {}",
        serde_json::to_string_pretty(&send_result).unwrap()
    );

    // B lists
    let messages = mbox_b
        .list_messages(TEST_BOX)
        .await
        .expect("listMessages on B failed");

    // Find the encrypted message
    let our_msg = messages.iter().find(|m| {
        m.body
            .get("encrypted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    });

    assert!(
        our_msg.is_some(),
        "B should have received a signed+encrypted message"
    );
    let msg = our_msg.unwrap();

    // Process — should decrypt first, then verify signature
    eprintln!("B processing (decrypt + verify) received message...");
    let processed = mbox_b
        .process_received_message(msg)
        .await
        .expect("process_received_message failed");

    eprintln!(
        "Processed: was_encrypted={}, was_signed={}, signature_valid={:?}",
        processed.was_encrypted, processed.was_signed, processed.signature_valid
    );

    assert!(
        processed.was_encrypted,
        "message should be detected as encrypted"
    );
    assert!(
        processed.was_signed,
        "inner message should be detected as signed"
    );
    assert_eq!(
        processed.signature_valid,
        Some(true),
        "BRC-77 signature should verify after decryption"
    );

    // Verify unwrapped body matches original
    assert_eq!(
        processed.body.get("nonce").and_then(|v| v.as_str()),
        Some(nonce.as_str()),
        "final body should contain original nonce"
    );
    assert_eq!(
        processed.body.get("payload").and_then(|v| v.as_str()),
        Some("Signed then encrypted — full BRC-77 + BRC-78 stack"),
        "final body should contain original payload"
    );

    // Acknowledge
    mbox_b
        .acknowledge_message(std::slice::from_ref(&msg.message_id))
        .await
        .expect("acknowledge failed");

    eprintln!("test_send_signed_encrypted_message PASSED");
}

// ---------------------------------------------------------------------------
// Test 5: Message hash matches on both sides
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_message_hash_matches_both_sides() {
    if !check_both_wallets().await {
        return;
    }

    let wa: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa);
    let mbox_b = mbox_for(wb);

    drain_box(&mbox_b, TEST_BOX).await;

    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "test_hash",
        "nonce": nonce,
        "data": "Hash verification test"
    });

    // Sender-side: compute hash before sending
    let sender_hash = compute_message_hash(&body);
    eprintln!("Sender-side hash: {sender_hash}");

    // A sends plain message
    let send_result = mbox_a
        .send_message(&identity_b, TEST_BOX, &body)
        .await
        .expect("send_message A->B failed");

    // The send result should include the message_hash
    let result_hash = send_result
        .get("message_hash")
        .and_then(|v| v.as_str())
        .expect("send result should contain message_hash");

    assert_eq!(
        sender_hash, result_hash,
        "sender-computed hash should match send_result hash"
    );
    eprintln!("Send result hash matches sender hash: {result_hash}");

    // B receives the message
    let messages = mbox_b
        .list_messages(TEST_BOX)
        .await
        .expect("listMessages on B failed");

    let our_msg = messages
        .iter()
        .find(|m| m.body.get("nonce").and_then(|v| v.as_str()) == Some(&nonce));

    assert!(
        our_msg.is_some(),
        "B should have received the hash test message"
    );
    let msg = our_msg.unwrap();

    // Receiver-side: compute hash from received body
    let receiver_hash = compute_message_hash(&msg.body);
    eprintln!("Receiver-side hash: {receiver_hash}");

    assert_eq!(
        sender_hash, receiver_hash,
        "sender and receiver should compute the same SHA-256 hash from the message body"
    );

    eprintln!(
        "Hash match confirmed: sender={} receiver={}",
        &sender_hash[..16],
        &receiver_hash[..16]
    );

    // Acknowledge
    mbox_b
        .acknowledge_message(std::slice::from_ref(&msg.message_id))
        .await
        .expect("acknowledge failed");

    eprintln!("test_message_hash_matches_both_sides PASSED");
}

// ---------------------------------------------------------------------------
// Test 6: Full round-trip — A sends to B, B replies to A
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_roundtrip_a_sends_b_replies() {
    if !check_both_wallets().await {
        return;
    }

    let wa: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_a = wa.get_identity_key().await.expect("get identity A");
    let identity_b = wb.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa);
    let mbox_b = mbox_for(wb);

    // Drain both inboxes
    drain_box(&mbox_a, TEST_BOX).await;
    drain_box(&mbox_b, TEST_BOX).await;

    let nonce = uuid::Uuid::new_v4().to_string();

    // Step 1: A sends to B
    let outbound_body = json!({
        "type": "test_roundtrip_request",
        "nonce": nonce,
        "question": "What is the meaning of life?"
    });

    eprintln!("Step 1: A sends request to B...");
    mbox_a
        .send_message(&identity_b, TEST_BOX, &outbound_body)
        .await
        .expect("A->B send failed");

    // Step 2: B receives the message
    eprintln!("Step 2: B checking inbox...");
    let b_messages = mbox_b
        .list_messages(TEST_BOX)
        .await
        .expect("B listMessages failed");

    let request_msg = b_messages
        .iter()
        .find(|m| m.body.get("nonce").and_then(|v| v.as_str()) == Some(&nonce));

    assert!(request_msg.is_some(), "B should have received A's request");
    let request_msg = request_msg.unwrap();

    assert_eq!(request_msg.sender, identity_a, "request sender should be A");
    eprintln!(
        "B received request: msg_id={} question={}",
        request_msg.message_id,
        request_msg
            .body
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    // Acknowledge the request on B
    mbox_b
        .acknowledge_message(std::slice::from_ref(&request_msg.message_id))
        .await
        .expect("B acknowledge request failed");

    // Step 3: B replies to A
    let reply_nonce = uuid::Uuid::new_v4().to_string();
    let reply_body = json!({
        "type": "test_roundtrip_reply",
        "nonce": reply_nonce,
        "in_reply_to": nonce,
        "answer": "42"
    });

    eprintln!("Step 3: B sends reply to A...");
    mbox_b
        .send_message(&identity_a, TEST_BOX, &reply_body)
        .await
        .expect("B->A reply send failed");

    // Step 4: A receives the reply
    eprintln!("Step 4: A checking inbox for reply...");
    let a_messages = mbox_a
        .list_messages(TEST_BOX)
        .await
        .expect("A listMessages failed");

    let reply_msg = a_messages
        .iter()
        .find(|m| m.body.get("nonce").and_then(|v| v.as_str()) == Some(&reply_nonce));

    assert!(reply_msg.is_some(), "A should have received B's reply");
    let reply_msg = reply_msg.unwrap();

    assert_eq!(reply_msg.sender, identity_b, "reply sender should be B");
    assert_eq!(
        reply_msg.body.get("in_reply_to").and_then(|v| v.as_str()),
        Some(nonce.as_str()),
        "reply should reference original nonce"
    );
    assert_eq!(
        reply_msg.body.get("answer").and_then(|v| v.as_str()),
        Some("42"),
        "reply should contain the answer"
    );

    eprintln!(
        "A received reply: msg_id={} answer={}",
        reply_msg.message_id,
        reply_msg
            .body
            .get("answer")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    // Acknowledge the reply on A
    mbox_a
        .acknowledge_message(std::slice::from_ref(&reply_msg.message_id))
        .await
        .expect("A acknowledge reply failed");

    eprintln!("Full round-trip complete: A->B request + B->A reply");
    eprintln!("test_roundtrip_a_sends_b_replies PASSED");
}
