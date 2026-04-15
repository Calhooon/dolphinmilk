//! Live probe tests for the BRC-33 MessageBox relay at messagebox.babbage.systems.
//!
//! Verifies:
//! 1. The relay is reachable and responds to authenticated BRC-31 requests.
//! 2. The wallet on localhost:3322 can complete a BRC-31 handshake with the relay.
//! 3. `listMessages` succeeds on the `task_inbox` (even if empty).
//!
//! All tests are `#[ignore]` — run with:
//! ```sh
//! cargo test --test test_messagebox_probe -- --ignored --nocapture
//! ```

use dolphin_milk::auth::AuthriteClient;
use dolphin_milk::messagebox::types::*;
use dolphin_milk::messagebox::MessageBoxClient;
use dolphin_milk::wallet::{HttpWalletClient, WalletBackend};
use std::sync::Arc;

const WALLET_URL: &str = "http://localhost:3322";

/// Create a wallet client for the local bsv-wallet-cli.
fn wallet() -> HttpWalletClient {
    HttpWalletClient::new(WALLET_URL, "http://localhost", 30)
}

/// Check that the wallet is reachable and authenticated.
async fn check_wallet() -> bool {
    match wallet().is_authenticated().await {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[SKIP] wallet at {WALLET_URL} not reachable: {e}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Test 1: BRC-31 handshake with MessageBox relay
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_probe_messagebox_brc31_handshake() {
    if !check_wallet().await {
        return;
    }

    let w = wallet();
    let identity_key = w.get_identity_key().await.expect("get_identity_key failed");
    eprintln!("Wallet identity key: {identity_key}");

    // Create AuthriteClient with a temp session dir so we start fresh.
    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(
        Arc::new(wallet()) as Arc<dyn WalletBackend + Send + Sync>,
        dir.path().to_path_buf(),
    );

    // Perform a BRC-31 handshake with the MessageBox relay.
    let session = auth
        .do_handshake(MESSAGEBOX_URL)
        .await
        .expect("BRC-31 handshake with MessageBox failed");

    eprintln!("Handshake SUCCESS:");
    eprintln!("  server_url:          {}", session.server_url);
    eprintln!(
        "  server_identity_key: {}",
        &session.server_identity_key[..16]
    );
    eprintln!(
        "  server_nonce:        {}...{}",
        &session.server_nonce_b64[..8],
        &session.server_nonce_b64[session.server_nonce_b64.len() - 4..]
    );

    // The server identity key should be a valid 66-char compressed pubkey.
    assert_eq!(
        session.server_identity_key.len(),
        66,
        "server identity key should be 66 hex chars"
    );
    assert!(
        session.server_identity_key.starts_with("02")
            || session.server_identity_key.starts_with("03"),
        "server identity key must be a compressed pubkey"
    );

    // The server identity key should match the well-known MessageBox key.
    assert_eq!(
        session.server_identity_key, MESSAGEBOX_IDENTITY_KEY,
        "server identity key should match MESSAGEBOX_IDENTITY_KEY constant"
    );
}

// ---------------------------------------------------------------------------
// Test 2: list messages from task_inbox (BRC-31 authenticated POST)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_probe_messagebox_list_task_inbox() {
    if !check_wallet().await {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(
        Arc::new(wallet()) as Arc<dyn WalletBackend + Send + Sync>,
        dir.path().to_path_buf(),
    );
    let mbox = MessageBoxClient::new(auth);

    // List messages from task_inbox — even if empty, the call must succeed.
    let messages = mbox
        .list_messages(BOX_TASK_INBOX)
        .await
        .expect("listMessages on task_inbox failed — BRC-31 auth or relay issue");

    eprintln!(
        "task_inbox: {} message(s) currently waiting",
        messages.len()
    );

    // Validate structure of any returned messages.
    for msg in &messages {
        assert!(!msg.message_id.is_empty(), "message_id must not be empty");
        assert_eq!(
            msg.sender.len(),
            66,
            "sender should be 66-char pubkey, got len={}",
            msg.sender.len()
        );
        assert!(!msg.created_at.is_empty(), "created_at must not be empty");
        // message_box should be set by the client after retrieval
        assert_eq!(
            msg.message_box.as_deref(),
            Some(BOX_TASK_INBOX),
            "message_box should be set to task_inbox"
        );
        eprintln!(
            "  msg_id={} sender={}... created_at={}",
            &msg.message_id,
            &msg.sender[..16],
            &msg.created_at
        );
    }

    eprintln!("listMessages on task_inbox PASSED (BRC-31 auth works)");
}

// ---------------------------------------------------------------------------
// Test 3: poll all inboxes (BRC-31 authenticated, hits all 4 boxes)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_probe_messagebox_poll_all_inboxes() {
    if !check_wallet().await {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(
        Arc::new(wallet()) as Arc<dyn WalletBackend + Send + Sync>,
        dir.path().to_path_buf(),
    );
    let mbox = MessageBoxClient::new(auth);

    // poll_inboxes hits all 4 boxes in priority order.
    // Even if all boxes are empty, this must succeed without error.
    let messages = mbox
        .poll_inboxes()
        .await
        .expect("poll_inboxes failed — BRC-31 auth or relay issue");

    eprintln!(
        "poll_inboxes: {} message(s) total across all {} boxes",
        messages.len(),
        INBOX_BOXES.len()
    );

    // All returned messages must have valid structure.
    for msg in &messages {
        assert!(!msg.message_id.is_empty());
        assert_eq!(msg.sender.len(), 66);
        assert!(!msg.created_at.is_empty());
    }

    eprintln!("poll_inboxes PASSED (all 4 boxes probed successfully)");
}

// ---------------------------------------------------------------------------
// Test 4: quote delivery to self (free, verifies auth + relay logic)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_probe_messagebox_quote_self() {
    if !check_wallet().await {
        return;
    }

    let w = wallet();
    let identity_key = w.get_identity_key().await.expect("get_identity_key failed");

    let dir = tempfile::tempdir().expect("tempdir failed");
    let auth = AuthriteClient::with_session_dir(
        Arc::new(wallet()) as Arc<dyn WalletBackend + Send + Sync>,
        dir.path().to_path_buf(),
    );
    let mbox = MessageBoxClient::new(auth);

    // Quote for self-delivery to task_inbox (should be free).
    let quote = mbox
        .quote(&identity_key, BOX_TASK_INBOX)
        .await
        .expect("quote for self-delivery failed");

    eprintln!(
        "Quote (self -> task_inbox): delivery_fee={}, recipient_fee={}, total={}",
        quote.delivery_fee,
        quote.recipient_fee,
        quote.total_cost()
    );

    assert!(!quote.is_blocked(), "self-delivery should not be blocked");
    assert_eq!(
        quote.total_cost(),
        0,
        "self-delivery to task_inbox should be free (0 sats)"
    );

    eprintln!("quote for self-delivery PASSED");
}
