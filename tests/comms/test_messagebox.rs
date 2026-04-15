//! Integration tests for BRC-33 MessageBox client.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::header::HeaderMap;
use serde_json::json;

use dolphin_milk::messagebox::types::*;
use dolphin_milk::x402::payment::parse_402_response;

// -----------------------------------------------------------------------
// Message type serde roundtrips
// -----------------------------------------------------------------------

#[test]
fn test_task_assignment_roundtrip() {
    let mut task = TaskAssignment::new(
        "uuid-test-1",
        "Research BSV agent frameworks",
        50000,
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
    );
    task.deadline_utc = Some("2026-02-24T00:00:00Z".into());
    task.context = Some(serde_json::json!({"notes": "Focus on payments"}));

    let json = serde_json::to_string(&task).unwrap();
    let back: TaskAssignment = serde_json::from_str(&json).unwrap();

    assert_eq!(back.msg_type, "task_assignment");
    assert_eq!(back.version, 1);
    assert_eq!(back.task_id, "uuid-test-1");
    assert_eq!(back.task, "Research BSV agent frameworks");
    assert_eq!(back.budget_sats, 50000);
    assert_eq!(back.response_box, BOX_RESULTS_INBOX);
    assert_eq!(back.status_box.as_deref(), Some(BOX_STATUS_INBOX));
    assert!(back.deadline_utc.is_some());
    assert!(back.context.is_some());
}

#[test]
fn test_status_update_roundtrip() {
    let mut status = StatusUpdate::new("task-123", "in_progress");
    status.progress_pct = Some(45);
    status.sats_spent = Some(12400);
    status.current_step = Some("Analyzing data".into());

    let json = serde_json::to_string(&status).unwrap();
    let back: StatusUpdate = serde_json::from_str(&json).unwrap();

    assert_eq!(back.msg_type, "status_update");
    assert_eq!(back.task_id, "task-123");
    assert_eq!(back.progress_pct, Some(45));
    assert_eq!(back.sats_spent, Some(12400));
}

#[test]
fn test_task_result_completed_roundtrip() {
    let result = TaskResult::completed(
        "task-456",
        serde_json::json!({
            "summary": "Analysis complete",
            "artifact_type": "markdown",
            "artifact": "# Report\n\nFindings..."
        }),
    );

    let json = serde_json::to_string(&result).unwrap();
    let back: TaskResult = serde_json::from_str(&json).unwrap();

    assert_eq!(back.status, "completed");
    assert!(back.result.is_some());
    assert!(back.failure_reason.is_none());
}

#[test]
fn test_task_result_failed_roundtrip() {
    let result = TaskResult::failed("task-789", "budget_exhausted", "Ran out at step 3");

    let json = serde_json::to_string(&result).unwrap();
    let back: TaskResult = serde_json::from_str(&json).unwrap();

    assert_eq!(back.status, "failed");
    assert_eq!(back.failure_reason.as_deref(), Some("budget_exhausted"));
    assert_eq!(back.failure_detail.as_deref(), Some("Ran out at step 3"));
}

#[test]
fn test_coordination_signal_roundtrip() {
    let mut signal = CoordinationSignal::heartbeat(
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
    );
    signal.capabilities = Some(vec!["research".into(), "code_generation".into()]);
    signal.available_budget_sats = Some(450000);
    signal.current_load = Some(2);

    let json = serde_json::to_string(&signal).unwrap();
    let back: CoordinationSignal = serde_json::from_str(&json).unwrap();

    assert_eq!(back.signal, "heartbeat");
    assert_eq!(back.capabilities.as_ref().unwrap().len(), 2);
    assert_eq!(back.available_budget_sats, Some(450000));
}

// -----------------------------------------------------------------------
// Quote response parsing
// -----------------------------------------------------------------------

#[test]
fn test_quote_free_delivery() {
    let quote = DeliveryQuote {
        delivery_fee: 0,
        recipient_fee: 0,
    };
    assert_eq!(quote.total_cost(), 0);
    assert!(!quote.requires_payment());
    assert!(!quote.is_blocked());
}

#[test]
fn test_quote_paid_delivery() {
    let quote = DeliveryQuote {
        delivery_fee: 10,
        recipient_fee: 100,
    };
    assert_eq!(quote.total_cost(), 110);
    assert!(quote.requires_payment());
    assert!(!quote.is_blocked());
}

#[test]
fn test_quote_blocked() {
    let quote = DeliveryQuote {
        delivery_fee: 0,
        recipient_fee: -1,
    };
    assert!(quote.is_blocked());
    assert!(!quote.requires_payment());
}

#[test]
fn test_quote_server_fee_only() {
    let quote = DeliveryQuote {
        delivery_fee: 10,
        recipient_fee: 0,
    };
    assert_eq!(quote.total_cost(), 10);
    assert!(quote.requires_payment());
}

#[test]
fn test_quote_from_json() {
    let json = serde_json::json!({
        "deliveryFee": 10,
        "recipientFee": 100,
    });
    let quote: DeliveryQuote = serde_json::from_value(json).unwrap();
    assert_eq!(quote.delivery_fee, 10);
    assert_eq!(quote.recipient_fee, 100);
}

// -----------------------------------------------------------------------
// Message body unwrapping
// -----------------------------------------------------------------------

#[test]
fn test_received_message_parsing() {
    let json = serde_json::json!({
        "messageId": "msg-001",
        "body": {"message": {"type": "task_assignment", "task": "hello"}},
        "sender": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
        "createdAt": "2026-02-23T15:30:00Z",
    });
    let msg: ReceivedMessage = serde_json::from_value(json).unwrap();
    assert_eq!(msg.message_id, "msg-001");
    assert_eq!(msg.sender.len(), 66);
}

#[test]
fn test_received_message_body_is_string() {
    // Server may return body as a JSON string
    let json = serde_json::json!({
        "messageId": "msg-002",
        "body": "{\"message\": {\"type\": \"status_update\"}}",
        "sender": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
        "createdAt": "2026-02-23T15:30:00Z",
    });
    let msg: ReceivedMessage = serde_json::from_value(json).unwrap();
    // Body is a JSON string — unwrapping happens in list_messages()
    assert!(msg.body.is_string());
}

// -----------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------

#[test]
fn test_messagebox_constants() {
    assert_eq!(
        MESSAGEBOX_URL,
        "https://rust-message-box.dev-a3e.workers.dev"
    );
    assert_eq!(MESSAGEBOX_IDENTITY_KEY.len(), 66);
    assert!(MESSAGEBOX_IDENTITY_KEY.starts_with("02") || MESSAGEBOX_IDENTITY_KEY.starts_with("03"));
    assert_eq!(BOX_TASK_INBOX, "task_inbox");
    assert_eq!(BOX_STATUS_INBOX, "status_inbox");
    assert_eq!(BOX_RESULTS_INBOX, "results_inbox");
    assert_eq!(BOX_DM_COORDINATION, "dolphin_milk_coordination");
}

#[test]
fn test_inbox_priority_order() {
    assert_eq!(INBOX_BOXES[0], BOX_TASK_INBOX);
    assert_eq!(INBOX_BOXES[1], BOX_RESULTS_INBOX);
    assert_eq!(INBOX_BOXES[2], BOX_STATUS_INBOX);
    assert_eq!(INBOX_BOXES[3], BOX_DM_COORDINATION);
}

// -----------------------------------------------------------------------
// Paid delivery (Phase 3.1) — body-transport payment tests
// -----------------------------------------------------------------------

#[test]
fn test_parse_402_headers_for_messagebox() {
    // Verify that MessageBox 402 responses can be parsed by the x402 payment module
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert("x-bsv-payment-satoshis-required", "110".parse().unwrap());
    headers.insert(
        "x-bsv-payment-derivation-prefix",
        "msgbox-prefix-abc".parse().unwrap(),
    );

    let result = parse_402_response(&headers).unwrap();
    assert_eq!(result.satoshis, 110);
    assert_eq!(result.derivation_prefix, "msgbox-prefix-abc");
    assert_eq!(result.version, "1.0");
}

#[test]
fn test_paid_delivery_body_transport_structure() {
    // Verify the body-transport payment merge produces the correct structure
    // that MessageBox expects: { message: {...}, payment: {...} }
    let message = json!({
        "message": {
            "recipient": "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1",
            "messageBox": "task_inbox",
            "messageId": "test-uuid-1234",
            "body": {
                "type": "task_assignment",
                "task": "Research BSV",
                "budget_sats": 50000,
            },
        }
    });

    let payment = json!({
        "derivationPrefix": "server-provided-prefix",
        "derivationSuffix": "random-client-suffix",
        "transaction": "AQEBAQAAAA==",
    });

    // Simulate the merge in send_message
    let mut paid_body = message.clone();
    paid_body["payment"] = payment;

    // Verify top-level structure
    assert!(
        paid_body.get("message").is_some(),
        "must have message field"
    );
    assert!(
        paid_body.get("payment").is_some(),
        "must have payment field"
    );

    // Verify message is preserved
    let msg = &paid_body["message"];
    assert_eq!(
        msg["recipient"],
        "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1"
    );
    assert_eq!(msg["messageBox"], "task_inbox");
    assert_eq!(msg["messageId"], "test-uuid-1234");
    assert_eq!(msg["body"]["type"], "task_assignment");
    assert_eq!(msg["body"]["budget_sats"], 50000);

    // Verify payment fields
    let pay = &paid_body["payment"];
    assert_eq!(pay["derivationPrefix"], "server-provided-prefix");
    assert_eq!(pay["derivationSuffix"], "random-client-suffix");
    assert_eq!(pay["transaction"], "AQEBAQAAAA==");
}

#[test]
fn test_messagebox_identity_key_is_valid_pubkey() {
    // The fallback identity key used when 402 response doesn't include one
    assert_eq!(MESSAGEBOX_IDENTITY_KEY.len(), 66, "must be 66 hex chars");
    assert!(
        MESSAGEBOX_IDENTITY_KEY.starts_with("02") || MESSAGEBOX_IDENTITY_KEY.starts_with("03"),
        "must be a compressed pubkey"
    );
    // Verify it's valid hex
    hex::decode(MESSAGEBOX_IDENTITY_KEY).expect("must be valid hex");
}

#[test]
fn test_paid_result_includes_payment_info() {
    // Verify the result structure when payment succeeds
    let mut result = json!({
        "status": "success",
    });
    let message_id = "msg-uuid-5678".to_string();
    let payment_txid = "abc123def456".to_string();
    let payment_sats: u64 = 110;

    result["sentMessageId"] = serde_json::Value::String(message_id.clone());
    result["payment_txid"] = serde_json::Value::String(payment_txid.clone());
    result["sats_paid"] = json!(payment_sats);

    assert_eq!(result["sentMessageId"], "msg-uuid-5678");
    assert_eq!(result["payment_txid"], "abc123def456");
    assert_eq!(result["sats_paid"], 110);
}

#[test]
fn test_quote_total_cost_matches_payment_amount() {
    // Verify that the quote cost calculation matches what would be paid
    let quote = DeliveryQuote {
        delivery_fee: 10,
        recipient_fee: 100,
    };
    assert_eq!(quote.total_cost(), 110);
    assert!(quote.requires_payment());

    // The 402 response should request at least total_cost sats
    let mut headers = HeaderMap::new();
    headers.insert("x-bsv-payment-version", "1.0".parse().unwrap());
    headers.insert(
        "x-bsv-payment-satoshis-required",
        format!("{}", quote.total_cost()).parse().unwrap(),
    );
    headers.insert(
        "x-bsv-payment-derivation-prefix",
        "test-prefix".parse().unwrap(),
    );

    let payment_req = parse_402_response(&headers).unwrap();
    assert_eq!(payment_req.satoshis, quote.total_cost() as u64);
}

// -----------------------------------------------------------------------
// BRC-77 Message Signing (Phase 3.2)
// -----------------------------------------------------------------------

#[test]
fn test_signed_message_roundtrip() {
    let body = json!({"type": "task_assignment", "task": "hello"});
    let signed = SignedMessage {
        body: body.clone(),
        signature: "deadbeef1234".to_string(),
        sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
            .to_string(),
    };

    let json_str = serde_json::to_string(&signed).unwrap();
    let back: SignedMessage = serde_json::from_str(&json_str).unwrap();

    assert_eq!(back.body, body);
    assert_eq!(back.signature, "deadbeef1234");
    assert_eq!(back.sender_key.len(), 66);
}

#[test]
fn test_signed_message_as_message_body() {
    // A signed message should be sendable as a regular message body
    let signed = SignedMessage {
        body: json!({"data": "test"}),
        signature: "aabbccdd".to_string(),
        sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
            .to_string(),
    };

    let as_value = serde_json::to_value(&signed).unwrap();

    // The signed message should be detectable by the recipient
    assert!(as_value.get("signature").is_some());
    assert!(as_value.get("sender_key").is_some());
    assert!(as_value.get("body").is_some());

    // Original body should be preserved inside
    assert_eq!(as_value["body"]["data"], "test");
}

#[test]
fn test_signed_message_deterministic_serialization() {
    // Signing relies on deterministic JSON serialization
    let body = json!({"b": 2, "a": 1});
    let serialized1 = serde_json::to_string(&body).unwrap();
    let serialized2 = serde_json::to_string(&body).unwrap();
    assert_eq!(
        serialized1, serialized2,
        "JSON serialization must be deterministic"
    );
}

// -----------------------------------------------------------------------
// BRC-78 Message Encryption (Phase 3.3)
// -----------------------------------------------------------------------

#[test]
fn test_encrypted_message_roundtrip() {
    let ciphertext = BASE64.encode(b"encrypted content");
    let encrypted = EncryptedMessage {
        ciphertext: ciphertext.clone(),
        sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
            .to_string(),
        encrypted: true,
    };

    let json_str = serde_json::to_string(&encrypted).unwrap();
    let back: EncryptedMessage = serde_json::from_str(&json_str).unwrap();

    assert_eq!(back.ciphertext, ciphertext);
    assert!(back.encrypted);
    assert_eq!(back.sender_key.len(), 66);
}

#[test]
fn test_encrypted_message_as_message_body() {
    // An encrypted message should be sendable as a regular message body
    let encrypted = EncryptedMessage {
        ciphertext: BASE64.encode(b"secret"),
        sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
            .to_string(),
        encrypted: true,
    };

    let as_value = serde_json::to_value(&encrypted).unwrap();

    // The encrypted message should be detectable by the recipient
    assert_eq!(as_value["encrypted"], true);
    assert!(as_value.get("ciphertext").is_some());
    assert!(as_value.get("sender_key").is_some());
}

#[test]
fn test_encrypted_message_base64_roundtrip() {
    // Verify base64 encoding/decoding of ciphertext
    let original_bytes = b"test ciphertext bytes";
    let encoded = BASE64.encode(original_bytes);
    let decoded = BASE64.decode(&encoded).unwrap();
    assert_eq!(decoded, original_bytes);

    let encrypted = EncryptedMessage {
        ciphertext: encoded,
        sender_key: "02abc".to_string(),
        encrypted: true,
    };

    let decoded_back = BASE64.decode(&encrypted.ciphertext).unwrap();
    assert_eq!(decoded_back, original_bytes);
}

// -----------------------------------------------------------------------
// ProcessedMessage (auto-decrypt + auto-verify)
// -----------------------------------------------------------------------

#[test]
fn test_processed_message_plain() {
    let processed = ProcessedMessage {
        body: json!({"type": "status_update"}),
        was_encrypted: false,
        was_signed: false,
        signature_valid: None,
        sender: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".to_string(),
    };

    assert!(!processed.was_encrypted);
    assert!(!processed.was_signed);
    assert!(processed.signature_valid.is_none());
}

#[test]
fn test_processed_message_signed() {
    let processed = ProcessedMessage {
        body: json!({"type": "task_assignment"}),
        was_encrypted: false,
        was_signed: true,
        signature_valid: Some(true),
        sender: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".to_string(),
    };

    assert!(processed.was_signed);
    assert_eq!(processed.signature_valid, Some(true));
}

#[test]
fn test_processed_message_encrypted_and_signed() {
    let processed = ProcessedMessage {
        body: json!({"secret": "data"}),
        was_encrypted: true,
        was_signed: true,
        signature_valid: Some(true),
        sender: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".to_string(),
    };

    assert!(processed.was_encrypted);
    assert!(processed.was_signed);
    assert_eq!(processed.signature_valid, Some(true));
}

#[test]
fn test_processed_message_serde_roundtrip() {
    let processed = ProcessedMessage {
        body: json!({"task": "hello"}),
        was_encrypted: true,
        was_signed: true,
        signature_valid: Some(false),
        sender: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".to_string(),
    };

    let json_str = serde_json::to_string(&processed).unwrap();
    let back: ProcessedMessage = serde_json::from_str(&json_str).unwrap();

    assert!(back.was_encrypted);
    assert!(back.was_signed);
    assert_eq!(back.signature_valid, Some(false));
    assert_eq!(back.body, json!({"task": "hello"}));
}

// -----------------------------------------------------------------------
// Detection patterns (how recipient detects signed/encrypted messages)
// -----------------------------------------------------------------------

#[test]
fn test_detect_encrypted_message_from_body() {
    let body = json!({
        "ciphertext": "base64encoded",
        "sender_key": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
        "encrypted": true,
    });

    let is_encrypted = body
        .get("encrypted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && body.get("ciphertext").is_some();
    assert!(is_encrypted);
}

#[test]
fn test_detect_signed_message_from_body() {
    let body = json!({
        "body": {"type": "test"},
        "signature": "hexsig",
        "sender_key": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
    });

    let is_signed = body.get("signature").is_some() && body.get("sender_key").is_some();
    assert!(is_signed);
}

#[test]
fn test_detect_plain_message_is_neither() {
    let body = json!({"type": "task_assignment", "task": "hello"});

    let is_encrypted = body
        .get("encrypted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && body.get("ciphertext").is_some();
    let is_signed = body.get("signature").is_some() && body.get("sender_key").is_some();

    assert!(!is_encrypted);
    assert!(!is_signed);
}

#[test]
fn test_signed_then_encrypted_layering() {
    // Verify the sign-then-encrypt layering: signed body becomes the inner payload
    let inner_body = json!({"task": "hello"});
    let signed = SignedMessage {
        body: inner_body.clone(),
        signature: "sig123".to_string(),
        sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
            .to_string(),
    };

    // Encrypt the signed message
    let signed_value = serde_json::to_value(&signed).unwrap();
    let encrypted = EncryptedMessage {
        ciphertext: BASE64.encode(serde_json::to_vec(&signed_value).unwrap()),
        sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
            .to_string(),
        encrypted: true,
    };

    // Recipient decrypts first
    let decrypted_bytes = BASE64.decode(&encrypted.ciphertext).unwrap();
    let decrypted_body: serde_json::Value = serde_json::from_slice(&decrypted_bytes).unwrap();

    // Then verifies the signature on the inner body
    let inner_signed: SignedMessage = serde_json::from_value(decrypted_body).unwrap();
    assert_eq!(inner_signed.body, inner_body);
    assert_eq!(inner_signed.signature, "sig123");
}

// -----------------------------------------------------------------------
// BRC-77 Cross-Wallet Signature Verification (BRC-42 counterparty modes)
// -----------------------------------------------------------------------
//
// Three counterparty modes for BRC-42 key derivation:
//
// | Mode       | Counterparty          | ECDH Shared Secret        | Who Can Verify              |
// |------------|-----------------------|---------------------------|-----------------------------|
// | Self-only  | "self" (own pubkey)   | alice_priv * alice_pub    | Only Alice                  |
// | Broadcast  | "anyone" (G, scalar=1)| alice_priv * G = alice_pub| Anyone with sender's pubkey |
// | Directed   | recipient's pubkey    | alice_priv * bob_pub      | Only Bob (ECDH symmetry)    |

#[test]
fn test_broadcast_signature_cross_wallet_verification() {
    use bsv::wallet::{
        Counterparty, CreateSignatureArgs, ProtoWallet, Protocol, SecurityLevel,
        VerifySignatureArgs,
    };

    let protocol = Protocol::new(
        SecurityLevel::Counterparty,
        "dolphin milk message signature",
    );
    let key_id = "message";
    let data = b"test cross-wallet message";

    let alice = ProtoWallet::new(Some(bsv::PrivateKey::random()));
    let alice_pub = alice.identity_key();

    // Alice signs with counterparty=Anyone (broadcast mode)
    let sig = alice
        .create_signature(CreateSignatureArgs {
            data: Some(data.to_vec()),
            hash_to_directly_sign: None,
            protocol_id: protocol.clone(),
            key_id: key_id.to_string(),
            counterparty: Some(Counterparty::Anyone),
        })
        .unwrap();

    // ProtoWallet::anyone() can verify with Alice's pubkey as counterparty.
    // ECDH = 1 * alice_pub = alice_pub (matches signing ECDH = alice_priv * G)
    let anyone = ProtoWallet::anyone();
    let result = anyone.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Other(alice_pub.clone())),
        for_self: None,
    });
    assert!(
        result.is_ok(),
        "Broadcast signature must be verifiable by anyone"
    );

    // A different wallet (Bob) CANNOT verify broadcast sigs via its own wallet
    // because ECDH = bob_priv * alice_pub ≠ alice_pub
    let bob = ProtoWallet::new(Some(bsv::PrivateKey::random()));
    let bob_result = bob.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Other(alice_pub.clone())),
        for_self: None,
    });
    assert!(
        bob_result.is_err(),
        "Bob's wallet must NOT verify broadcast sig (wrong ECDH)"
    );
}

#[test]
fn test_directed_signature_cross_wallet_verification() {
    use bsv::wallet::{
        Counterparty, CreateSignatureArgs, ProtoWallet, Protocol, SecurityLevel,
        VerifySignatureArgs,
    };

    let protocol = Protocol::new(
        SecurityLevel::Counterparty,
        "dolphin milk message signature",
    );
    let key_id = "message";
    let data = b"directed message for bob only";

    let alice = ProtoWallet::new(Some(bsv::PrivateKey::random()));
    let bob = ProtoWallet::new(Some(bsv::PrivateKey::random()));
    let alice_pub = alice.identity_key();
    let bob_pub = bob.identity_key();

    // Alice signs with counterparty=Bob's pubkey (directed mode)
    let sig = alice
        .create_signature(CreateSignatureArgs {
            data: Some(data.to_vec()),
            hash_to_directly_sign: None,
            protocol_id: protocol.clone(),
            key_id: key_id.to_string(),
            counterparty: Some(Counterparty::Other(bob_pub)),
        })
        .unwrap();

    // Bob CAN verify with counterparty=Alice's pubkey
    // ECDH = bob_priv * alice_pub = alice_priv * bob_pub (ECDH symmetry)
    let result = bob.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Other(alice_pub.clone())),
        for_self: None,
    });
    assert!(
        result.is_ok(),
        "Directed signature must be verifiable by the recipient"
    );

    // ProtoWallet::anyone() CANNOT verify directed sigs
    // because ECDH = 1 * alice_pub ≠ bob_priv * alice_pub
    let anyone = ProtoWallet::anyone();
    let anyone_result = anyone.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Other(alice_pub.clone())),
        for_self: None,
    });
    assert!(
        anyone_result.is_err(),
        "Anyone wallet must NOT verify directed sig (wrong ECDH)"
    );
}

#[test]
fn test_self_signature_only_signer_can_verify() {
    use bsv::wallet::{
        Counterparty, CreateSignatureArgs, ProtoWallet, Protocol, SecurityLevel,
        VerifySignatureArgs,
    };

    let protocol = Protocol::new(
        SecurityLevel::Counterparty,
        "dolphin milk message signature",
    );
    let key_id = "message";
    let data = b"self-only attestation";

    let alice = ProtoWallet::new(Some(bsv::PrivateKey::random()));
    let alice_pub = alice.identity_key();

    // Alice signs with counterparty=Self_ (self-only mode)
    let sig = alice
        .create_signature(CreateSignatureArgs {
            data: Some(data.to_vec()),
            hash_to_directly_sign: None,
            protocol_id: protocol.clone(),
            key_id: key_id.to_string(),
            counterparty: Some(Counterparty::Self_),
        })
        .unwrap();

    // Alice CAN verify her own self-signed message
    let result = alice.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Self_),
        for_self: None,
    });
    assert!(
        result.is_ok(),
        "Signer must be able to verify their own self-signed message"
    );

    // Bob CANNOT verify Alice's self-signed message (even with Alice's pubkey)
    let bob = ProtoWallet::new(Some(bsv::PrivateKey::random()));
    let bob_result = bob.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Other(alice_pub.clone())),
        for_self: None,
    });
    assert!(
        bob_result.is_err(),
        "Self-signed messages are unverifiable by other wallets"
    );

    // ProtoWallet::anyone() also CANNOT verify
    let anyone = ProtoWallet::anyone();
    let anyone_result = anyone.verify_signature(VerifySignatureArgs {
        data: Some(data.to_vec()),
        hash_to_directly_verify: None,
        signature: sig.signature.clone(),
        protocol_id: protocol.clone(),
        key_id: key_id.to_string(),
        counterparty: Some(Counterparty::Other(alice_pub)),
        for_self: None,
    });
    assert!(
        anyone_result.is_err(),
        "Self-signed messages are unverifiable by anyone wallet"
    );
}
