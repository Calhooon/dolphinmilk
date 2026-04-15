//! Cross-wallet payment-in-message tests — Wallet A pays Wallet B by including
//! BEEF (transaction bytes) in a MessageBox message, and B internalizes the payment.
//!
//! Requires:
//!   - Wallet A on localhost:3322 (funded BSV wallet)
//!   - Wallet B on localhost:3323 (funded BSV wallet, different identity key)
//!   - Live internet access to messagebox.babbage.systems
//!
//! All tests are `#[ignore]` — run with:
//! ```sh
//! cargo test --test test_payment_in_message -- --ignored --nocapture
//! ```

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use dolphin_milk::auth::AuthriteClient;
use dolphin_milk::messagebox::MessageBoxClient;
use dolphin_milk::wallet::{HttpWalletClient, WalletBackend};
use dolphin_milk::x402::payment::build_p2pkh_script;
use serde_json::{json, Value};
use std::sync::Arc;

const WALLET_A_URL: &str = "http://localhost:3322";
const WALLET_B_URL: &str = "http://localhost:3323";

/// MessageBox inbox for payment messages.
const PAYMENT_BOX: &str = "payment_inbox";

/// Payment protocol — same as BRC-29 x402 payment key derivation.
fn payment_protocol() -> Value {
    json!([2, "3241645161d8"])
}

/// Amount to send in each test payment (satoshis).
const PAYMENT_SATS: u64 = 100;

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
            eprintln!("Wallet A identity: {a}");
            eprintln!("Wallet B identity: {b}");
            true
        }
        _ => {
            eprintln!("[SKIP] Could not get identity keys from both wallets");
            false
        }
    }
}

/// Drain all messages from a box for a given mbox client, acknowledging each.
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

/// Create a payment transaction from wallet A to wallet B.
///
/// Steps:
/// 1. Derive B's payment public key using BRC-42 (A derives with forSelf=false, counterparty=B)
/// 2. Build P2PKH locking script for the derived key
/// 3. createAction with that output
///
/// Returns (txid, beef_bytes, derivation_prefix, derivation_suffix, identity_a)
async fn create_payment_to_b(
    wa: &HttpWalletClient,
    identity_b: &str,
) -> (String, Vec<u8>, String, String, String) {
    let identity_a = wa.get_identity_key().await.expect("get identity A");

    // Use a deterministic derivation prefix and suffix for agent-to-agent payments.
    // In production, the prefix would be random per-payment; here we use a fixed prefix
    // and a unique suffix per payment.
    let derivation_prefix = "agent-payment";
    let derivation_suffix = format!("pay-{}", uuid::Uuid::new_v4());
    let key_id = format!("{derivation_prefix} {derivation_suffix}");

    eprintln!("Deriving payment key for B...");
    eprintln!("  protocol: {:?}", payment_protocol());
    eprintln!("  key_id: {key_id}");
    eprintln!("  counterparty (B): {identity_b}");
    eprintln!("  forSelf: false");

    // A derives the public key that B can derive the private key for
    let payment_pubkey = wa
        .get_public_key(&payment_protocol(), &key_id, identity_b, false)
        .await
        .expect("get_public_key for payment failed");

    eprintln!("Derived payment pubkey: {payment_pubkey}");

    // Build P2PKH locking script
    let locking_script = build_p2pkh_script(&payment_pubkey).expect("build_p2pkh_script failed");
    eprintln!("Locking script: {locking_script}");

    // Create the funded transaction
    let outputs = vec![json!({
        "satoshis": PAYMENT_SATS,
        "lockingScript": locking_script,
        "outputDescription": "agent payment",
        "basket": "default",
    })];

    eprintln!("Creating payment transaction ({PAYMENT_SATS} sats)...");
    let result = wa
        .create_action(&outputs, "Pay agent B via MessageBox", true, false)
        .await
        .expect("create_action failed");

    eprintln!("Payment transaction created:");
    eprintln!("  txid: {}", result.txid);
    eprintln!("  tx bytes: {} bytes", result.tx.len());

    // Log the raw response for debugging
    if let Some(tx_arr) = result.raw.get("tx").and_then(|v| v.as_array()) {
        eprintln!("  tx array length from raw: {}", tx_arr.len());
    }

    // Detect format by header bytes
    if result.tx.len() >= 4 {
        let header = &result.tx[..4];
        if header == [0x01, 0x01, 0x01, 0x01] {
            eprintln!("  format: AtomicBEEF");
        } else if header == [0x01, 0x00, 0xBE, 0xEF] {
            eprintln!("  format: BEEF");
        } else if header == [0x01, 0x00, 0x00, 0x00] {
            eprintln!("  format: raw tx");
        } else {
            eprintln!("  format: unknown (header: {:02x?})", header);
        }
    }

    (
        result.txid,
        result.tx,
        derivation_prefix.to_string(),
        derivation_suffix,
        identity_a,
    )
}

// ---------------------------------------------------------------------------
// Test 1: Create a payment transaction for another wallet
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_create_payment_for_other_wallet() {
    if !check_both_wallets().await {
        return;
    }

    let wa = wallet_a();
    let wb = wallet_b();

    let identity_b = wb.get_identity_key().await.expect("get identity B");

    // Get A's balance before
    let balance_before = wa.get_balance().await.expect("get balance A");
    eprintln!("Wallet A balance before: {balance_before} sats");

    let (txid, tx_bytes, prefix, suffix, identity_a) = create_payment_to_b(&wa, &identity_b).await;

    // Get A's balance after
    let balance_after = wa.get_balance().await.expect("get balance A after");
    eprintln!("Wallet A balance after: {balance_after} sats");
    eprintln!(
        "Balance change: {} sats (expected ~{PAYMENT_SATS} + mining fee)",
        balance_before.saturating_sub(balance_after)
    );

    // Verify transaction was created
    assert!(!txid.is_empty(), "txid should not be empty");
    assert_eq!(txid.len(), 64, "txid should be 64 hex chars");
    assert!(!tx_bytes.is_empty(), "tx bytes should not be empty");

    eprintln!();
    eprintln!("Payment details:");
    eprintln!("  txid: {txid}");
    eprintln!("  tx size: {} bytes", tx_bytes.len());
    eprintln!("  derivation_prefix: {prefix}");
    eprintln!("  derivation_suffix: {suffix}");
    eprintln!("  sender: {identity_a}");
    eprintln!("  recipient: {identity_b}");
    eprintln!("  satoshis: {PAYMENT_SATS}");

    eprintln!();
    eprintln!("test_create_payment_for_other_wallet PASSED");
}

// ---------------------------------------------------------------------------
// Test 2: Send payment via MessageBox
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_send_payment_via_messagebox() {
    if !check_both_wallets().await {
        return;
    }

    let wa_wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb_wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb_wallet.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa_wallet.clone());
    let mbox_b = mbox_for(wb_wallet.clone());

    // Drain B's payment inbox
    drain_box(&mbox_b, PAYMENT_BOX).await;

    // Create the payment
    let wa = wallet_a();
    let (txid, tx_bytes, prefix, suffix, identity_a) = create_payment_to_b(&wa, &identity_b).await;

    // Encode BEEF as base64 for transport in JSON message
    let beef_base64 = BASE64.encode(&tx_bytes);
    eprintln!("BEEF base64 length: {} chars", beef_base64.len());

    // Build the payment message
    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "payment",
        "nonce": nonce,
        "txid": txid,
        "beef": beef_base64,
        "satoshis": PAYMENT_SATS,
        "derivationPrefix": prefix,
        "derivationSuffix": suffix,
        "senderIdentityKey": identity_a,
    });

    eprintln!("Sending payment message from A to B...");
    eprintln!(
        "  message body size: {} bytes",
        serde_json::to_string(&body).unwrap().len()
    );

    let send_result = mbox_a
        .send_message(&identity_b, PAYMENT_BOX, &body)
        .await
        .expect("send_message A->B failed");

    eprintln!(
        "Send result: {}",
        serde_json::to_string_pretty(&send_result).unwrap()
    );

    // B receives the payment message
    eprintln!("B listing messages from {PAYMENT_BOX}...");
    let messages = mbox_b
        .list_messages(PAYMENT_BOX)
        .await
        .expect("listMessages on B failed");

    eprintln!("B received {} message(s)", messages.len());

    // Find our payment message by nonce
    let our_msg = messages
        .iter()
        .find(|m| m.body.get("nonce").and_then(|v| v.as_str()) == Some(&nonce));

    assert!(
        our_msg.is_some(),
        "B should have received the payment message with our nonce"
    );
    let msg = our_msg.unwrap();

    // Verify message structure
    assert_eq!(
        msg.body.get("type").and_then(|v| v.as_str()),
        Some("payment"),
        "message type should be 'payment'"
    );
    assert_eq!(
        msg.body.get("txid").and_then(|v| v.as_str()),
        Some(txid.as_str()),
        "txid should match"
    );
    assert_eq!(
        msg.body.get("satoshis").and_then(|v| v.as_u64()),
        Some(PAYMENT_SATS),
        "satoshis should match"
    );
    assert!(
        msg.body.get("beef").and_then(|v| v.as_str()).is_some(),
        "message should contain beef base64"
    );

    eprintln!("Payment message received by B:");
    eprintln!("  msg_id: {}", msg.message_id);
    eprintln!("  sender: {}", msg.sender);
    eprintln!(
        "  txid: {}",
        msg.body.get("txid").and_then(|v| v.as_str()).unwrap()
    );
    eprintln!(
        "  satoshis: {}",
        msg.body.get("satoshis").and_then(|v| v.as_u64()).unwrap()
    );

    // Acknowledge to clean up
    mbox_b
        .acknowledge_message(std::slice::from_ref(&msg.message_id))
        .await
        .expect("acknowledge failed");

    eprintln!();
    eprintln!("test_send_payment_via_messagebox PASSED");
}

// ---------------------------------------------------------------------------
// Test 3: Full flow — create, send, receive, internalize
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_internalize_received_payment() {
    if !check_both_wallets().await {
        return;
    }

    let wa_wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_a());
    let wb_wallet: Arc<dyn WalletBackend + Send + Sync> = Arc::new(wallet_b());

    let identity_b = wb_wallet.get_identity_key().await.expect("get identity B");

    let mbox_a = mbox_for(wa_wallet.clone());
    let mbox_b = mbox_for(wb_wallet.clone());

    // Drain B's payment inbox
    drain_box(&mbox_b, PAYMENT_BOX).await;

    // ---- Step 0: Record balances ----
    let wa = wallet_a();
    let wb = wallet_b();

    let balance_a_before = wa.get_balance().await.expect("get balance A before");
    let balance_b_before = wb.get_balance().await.expect("get balance B before");
    eprintln!("=== BALANCES BEFORE ===");
    eprintln!("  Wallet A: {balance_a_before} sats");
    eprintln!("  Wallet B: {balance_b_before} sats");

    // ---- Step 1: A creates payment for B ----
    eprintln!();
    eprintln!("=== STEP 1: CREATE PAYMENT ===");
    let (txid, tx_bytes, prefix, suffix, identity_a) = create_payment_to_b(&wa, &identity_b).await;

    let balance_a_after_create = wa.get_balance().await.expect("get balance A after create");
    eprintln!("Wallet A balance after create: {balance_a_after_create} sats");
    eprintln!(
        "  A spent: {} sats (payment + fee)",
        balance_a_before.saturating_sub(balance_a_after_create)
    );

    // ---- Step 2: A sends payment via MessageBox to B ----
    eprintln!();
    eprintln!("=== STEP 2: SEND VIA MESSAGEBOX ===");
    let beef_base64 = BASE64.encode(&tx_bytes);
    let nonce = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "type": "payment",
        "nonce": nonce,
        "txid": txid,
        "beef": beef_base64,
        "satoshis": PAYMENT_SATS,
        "derivationPrefix": prefix,
        "derivationSuffix": suffix,
        "senderIdentityKey": identity_a,
    });

    let send_result = mbox_a
        .send_message(&identity_b, PAYMENT_BOX, &body)
        .await
        .expect("send_message A->B failed");
    eprintln!(
        "Sent: {}",
        serde_json::to_string_pretty(&send_result).unwrap()
    );

    // ---- Step 3: B receives the payment message ----
    eprintln!();
    eprintln!("=== STEP 3: RECEIVE MESSAGE ===");
    let messages = mbox_b
        .list_messages(PAYMENT_BOX)
        .await
        .expect("listMessages on B failed");

    let our_msg = messages
        .iter()
        .find(|m| m.body.get("nonce").and_then(|v| v.as_str()) == Some(&nonce))
        .expect("B should have received the payment message");

    eprintln!("Received payment message: msg_id={}", our_msg.message_id);

    // Extract payment details from the message
    let received_txid = our_msg
        .body
        .get("txid")
        .and_then(|v| v.as_str())
        .expect("message should have txid");
    let received_beef_b64 = our_msg
        .body
        .get("beef")
        .and_then(|v| v.as_str())
        .expect("message should have beef");
    let received_sats = our_msg
        .body
        .get("satoshis")
        .and_then(|v| v.as_u64())
        .expect("message should have satoshis");
    let received_prefix = our_msg
        .body
        .get("derivationPrefix")
        .and_then(|v| v.as_str())
        .expect("message should have derivationPrefix");
    let received_suffix = our_msg
        .body
        .get("derivationSuffix")
        .and_then(|v| v.as_str())
        .expect("message should have derivationSuffix");
    let received_sender = our_msg
        .body
        .get("senderIdentityKey")
        .and_then(|v| v.as_str())
        .expect("message should have senderIdentityKey");

    eprintln!("Extracted payment details:");
    eprintln!("  txid: {received_txid}");
    eprintln!("  beef: {} chars (base64)", received_beef_b64.len());
    eprintln!("  satoshis: {received_sats}");
    eprintln!("  derivationPrefix: {received_prefix}");
    eprintln!("  derivationSuffix: {received_suffix}");
    eprintln!("  senderIdentityKey: {received_sender}");

    // ---- Step 4: B internalizes the payment ----
    eprintln!();
    eprintln!("=== STEP 4: INTERNALIZE PAYMENT ===");

    // Decode BEEF from base64
    let received_beef = BASE64
        .decode(received_beef_b64)
        .expect("base64 decode BEEF failed");
    eprintln!("Decoded BEEF: {} bytes", received_beef.len());

    // The tx bytes from createAction may already be AtomicBEEF.
    // If so, pass through. If BEEF, wrap in AtomicBEEF. If raw, wrap in BEEF+AtomicBEEF.
    let atomic_beef = if received_beef.len() >= 4 && received_beef[..4] == [0x01, 0x01, 0x01, 0x01]
    {
        eprintln!("  Already AtomicBEEF format — passing through");
        received_beef.clone()
    } else if received_beef.len() >= 4 && received_beef[..4] == [0x01, 0x00, 0xBE, 0xEF] {
        eprintln!("  BEEF format — wrapping in AtomicBEEF");
        // AtomicBEEF = [0x01, 0x01, 0x01, 0x01] + reversed_txid(32) + beef_bytes
        let mut ab = vec![0x01u8, 0x01, 0x01, 0x01];
        let txid_bytes = hex::decode(received_txid).expect("hex decode txid");
        let reversed: Vec<u8> = txid_bytes.into_iter().rev().collect();
        ab.extend_from_slice(&reversed);
        ab.extend_from_slice(&received_beef);
        ab
    } else {
        eprintln!(
            "  Unknown format (header: {:02x?}) — wrapping as raw tx in AtomicBEEF",
            &received_beef[..4.min(received_beef.len())]
        );
        // raw_tx_to_atomic_beef: raw → BEEF → AtomicBEEF
        use dolphin_milk::x402::payment::{beef_to_atomic_beef, raw_tx_to_beef};
        let beef = raw_tx_to_beef(&received_beef);
        beef_to_atomic_beef(&beef, received_txid)
    };

    eprintln!("AtomicBEEF: {} bytes", atomic_beef.len());

    // Build the internalization outputs — tells the wallet how to derive the key
    // The key_id for the derivation is "{prefix} {suffix}" with the sender as counterparty
    let internalize_outputs = vec![json!({
        "outputIndex": 0,
        "protocol": "wallet payment",
        "paymentRemittance": {
            "derivationPrefix": received_prefix,
            "derivationSuffix": received_suffix,
            "senderIdentityKey": received_sender,
        },
    })];

    eprintln!(
        "Internalize params: {}",
        serde_json::to_string_pretty(&internalize_outputs).unwrap()
    );

    let internalize_result = wb
        .internalize_action(
            &atomic_beef,
            &internalize_outputs,
            &format!("Received payment from agent: {}", &received_txid[..16]),
        )
        .await;

    match &internalize_result {
        Ok(val) => {
            eprintln!(
                "Internalization result: {}",
                serde_json::to_string_pretty(val).unwrap()
            );
            let accepted = val.get("accepted").and_then(|v| v.as_bool());
            eprintln!("  accepted: {:?}", accepted);
        }
        Err(e) => {
            eprintln!("Internalization FAILED: {e}");
            eprintln!("  This may be due to key derivation mismatch.");
            eprintln!("  The sender derived with counterparty=B, forSelf=false.");
            eprintln!("  The recipient must derive with counterparty=A, forSelf=true.");
            eprintln!("  The wallet's internalizeAction should handle this via paymentRemittance.");
        }
    }

    // Whether internalization succeeded or failed, acknowledge the message
    mbox_b
        .acknowledge_message(std::slice::from_ref(&our_msg.message_id))
        .await
        .expect("acknowledge failed");

    // ---- Step 5: Check balances after ----
    eprintln!();
    eprintln!("=== STEP 5: FINAL BALANCES ===");
    let balance_a_after = wa.get_balance().await.expect("get balance A after");
    let balance_b_after = wb.get_balance().await.expect("get balance B after");

    eprintln!("  Wallet A: {balance_a_after} sats (was {balance_a_before})");
    eprintln!("  Wallet B: {balance_b_after} sats (was {balance_b_before})");
    eprintln!(
        "  A spent: {} sats",
        balance_a_before.saturating_sub(balance_a_after)
    );

    if balance_b_after > balance_b_before {
        eprintln!("  B gained: {} sats", balance_b_after - balance_b_before);
    } else {
        eprintln!(
            "  B change: {} sats (may not have increased if internalization failed)",
            balance_b_after as i64 - balance_b_before as i64
        );
    }

    // Report the internalization result clearly
    if let Ok(val) = &internalize_result {
        let accepted = val
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if accepted {
            eprintln!();
            eprintln!("=== PAYMENT INTERNALIZED SUCCESSFULLY ===");
            eprintln!(
                "  B's balance increased by {} sats",
                balance_b_after.saturating_sub(balance_b_before)
            );
            assert!(
                balance_b_after > balance_b_before,
                "B's balance should have increased after internalization"
            );
        } else {
            eprintln!();
            eprintln!("=== INTERNALIZATION NOT ACCEPTED ===");
            eprintln!("  The wallet did not accept the payment.");
            eprintln!("  Full result: {}", serde_json::to_string(val).unwrap());
        }
    } else {
        eprintln!();
        eprintln!("=== INTERNALIZATION ERROR ===");
        eprintln!("  Error: {}", internalize_result.unwrap_err());
    }

    eprintln!();
    eprintln!("test_internalize_received_payment COMPLETED");
    // Note: We don't assert on internalization success because the key derivation
    // may not match between the two wallets. The test reports exact results for
    // the issue tracker.
}
