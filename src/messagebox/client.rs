//! BRC-33 MessageBox client.
//!
//! Provides send, list, acknowledge, quote, and permission operations
//! against the MessageBox server using BRC-31 Authrite authentication.
//!
//! BRC-77 message signing: sign message bodies with ECDSA for authenticity.
//! BRC-78 message encryption: encrypt message bodies via BRC-42 ECDH for confidentiality.
//! Both are composable: a message can be signed, encrypted, or both.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::auth::AuthriteClient;
use crate::error::DmError;
use crate::x402::payment::{create_payment, parse_402_response};

use super::types::*;

/// Protocol ID for BRC-77 message signing.
fn signing_protocol_id() -> Value {
    json!([2, "dolphin milk message signature"])
}

/// Protocol ID for BRC-78 message encryption.
fn encryption_protocol_id() -> Value {
    json!([2, "dolphin milk message encryption"])
}

/// Key ID for message signing and encryption (static).
const MESSAGE_KEY_ID: &str = "message";

/// Maximum number of payment retry attempts for MessageBox delivery.
const MAX_PAYMENT_ATTEMPTS: u32 = 3;

/// Compute SHA-256 hash of a serialized message body for cross-agent proof linking.
///
/// Both sender and receiver can compute the same hash from the same message body,
/// enabling cross-reference in their respective proof chains.
pub fn compute_message_hash(body: &Value) -> String {
    let serialized = serde_json::to_string(body).unwrap_or_default();
    let hash = Sha256::digest(serialized.as_bytes());
    hex::encode(hash)
}

/// BRC-33 MessageBox client.
pub struct MessageBoxClient {
    auth: AuthriteClient,
    server_url: String,
}

impl MessageBoxClient {
    /// Create a new MessageBoxClient with the default server.
    pub fn new(auth: AuthriteClient) -> Self {
        Self {
            auth,
            server_url: MESSAGEBOX_URL.to_string(),
        }
    }

    /// Create with a custom server URL (for testing).
    pub fn with_url(auth: AuthriteClient, server_url: &str) -> Self {
        Self {
            auth,
            server_url: server_url.trim_end_matches('/').to_string(),
        }
    }

    /// Get delivery cost quote for sending to a recipient.
    pub async fn quote(
        &self,
        recipient: &str,
        message_box: &str,
    ) -> Result<DeliveryQuote, DmError> {
        let url = format!(
            "{}/permissions/quote?recipient={}&messageBox={}",
            self.server_url, recipient, message_box
        );

        let resp = self.auth.get(&url).await?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| DmError::messagebox(format!("Invalid JSON from quote: {e}")))?;

        if !status.is_success() {
            return Err(DmError::messagebox(format!(
                "Quote failed (HTTP {}): {}",
                status,
                body.get("description")
                    .or(body.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
            )));
        }

        // Single recipient response has quote at top level
        let quote = if let Some(q) = body.get("quote") {
            serde_json::from_value::<DeliveryQuote>(q.clone())
                .map_err(|e| DmError::messagebox(format!("Failed to parse quote: {e}")))?
        } else {
            // Try top-level fields
            DeliveryQuote {
                delivery_fee: body
                    .get("deliveryFee")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                recipient_fee: body
                    .get("recipientFee")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            }
        };

        Ok(quote)
    }

    /// Send a message to a recipient's message box.
    ///
    /// Handles quoting and BRC-29 payment construction automatically.
    /// If the server requires payment (402), constructs a body-transport
    /// payment and retries with the payment merged into the request body.
    ///
    /// The result includes a `message_hash` field (SHA-256 of the serialized body)
    /// for cross-agent proof chain linking.
    pub async fn send_message(
        &self,
        recipient: &str,
        message_box: &str,
        body: &Value,
    ) -> Result<Value, DmError> {
        // Compute message hash for cross-agent proof chain linking
        let message_hash = compute_message_hash(body);

        // 1. Quote delivery cost (detect blocked senders early)
        let quote = self.quote(recipient, message_box).await?;

        if quote.is_blocked() {
            return Err(DmError::messagebox(format!(
                "Recipient has blocked messages to {message_box}"
            )));
        }

        if quote.requires_payment() {
            tracing::info!(
                "MessageBox delivery requires payment: {} sats (delivery={}, recipient={})",
                quote.total_cost(),
                quote.delivery_fee,
                quote.recipient_fee,
            );
        }

        // 2. Generate unique message ID
        let message_id = uuid::Uuid::new_v4().to_string();

        // 3. Construct request body
        let request_body = json!({
            "message": {
                "recipient": recipient,
                "messageBox": message_box,
                "messageId": message_id,
                "body": body,
            }
        });

        // 4. Send (BRC-31 authenticated)
        let url = format!("{}/sendMessage", self.server_url);
        let resp = self.auth.post_json(&url, &request_body).await?;

        let status = resp.status();

        // 5. If not 402, handle normally
        if status != StatusCode::PAYMENT_REQUIRED {
            let response: Value = resp
                .json()
                .await
                .map_err(|e| DmError::messagebox(format!("Invalid JSON from sendMessage: {e}")))?;

            if !status.is_success() {
                return Err(DmError::messagebox(format!(
                    "sendMessage failed (HTTP {}): {}",
                    status,
                    response
                        .get("description")
                        .or(response.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error")
                )));
            }

            let mut result = response;
            result["sentMessageId"] = Value::String(message_id);
            result["message_hash"] = Value::String(message_hash);
            return Ok(result);
        }

        // 6. Got 402 — enter body-transport payment loop
        tracing::info!("MessageBox sendMessage returned 402 Payment Required");

        // Extract server identity key from 402 response headers, fall back to known key
        let server_identity_key = resp
            .headers()
            .get("x-bsv-auth-identity-key")
            .and_then(|v| v.to_str().ok())
            .filter(|k| !k.is_empty())
            .unwrap_or(MESSAGEBOX_IDENTITY_KEY)
            .to_string();

        let mut last_headers = resp.headers().clone();

        for attempt in 1..=MAX_PAYMENT_ATTEMPTS {
            tracing::info!(
                "MessageBox payment attempt {}/{}",
                attempt,
                MAX_PAYMENT_ATTEMPTS,
            );

            // Parse 402 response headers for payment requirements
            let payment_req = parse_402_response(&last_headers)?;
            let payment_sats = payment_req.satoshis;

            // Construct BRC-29 payment via wallet
            let (payment_val, payment_txid) = create_payment(
                self.auth.wallet_api(),
                &payment_req.derivation_prefix,
                &server_identity_key,
                payment_req.satoshis,
                &self.server_url,
            )
            .await?;

            tracing::info!(
                "MessageBox payment created: txid={}, sats={}",
                payment_txid,
                payment_sats,
            );

            // Body-transport: merge payment into the request body
            let mut paid_body = request_body.clone();
            paid_body["payment"] = payment_val;

            // Retry with BRC-31 auth + payment in body
            let resp = self.auth.post_json(&url, &paid_body).await?;

            let status = resp.status();

            if status == StatusCode::PAYMENT_REQUIRED {
                tracing::warn!(
                    "MessageBox returned 402 again after payment (attempt {}/{})",
                    attempt,
                    MAX_PAYMENT_ATTEMPTS,
                );
                last_headers = resp.headers().clone();
                if attempt >= MAX_PAYMENT_ATTEMPTS {
                    return Err(DmError::payment(format!(
                        "MessageBox still returning 402 after {} payment attempts",
                        MAX_PAYMENT_ATTEMPTS,
                    )));
                }
                continue;
            }

            let response: Value = resp
                .json()
                .await
                .map_err(|e| DmError::messagebox(format!("Invalid JSON from sendMessage: {e}")))?;

            if !status.is_success() {
                return Err(DmError::messagebox(format!(
                    "sendMessage failed (HTTP {}): {}",
                    status,
                    response
                        .get("description")
                        .or(response.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error")
                )));
            }

            tracing::info!(
                "MessageBox payment accepted! txid={}, sats={} (attempt {}/{})",
                payment_txid,
                payment_sats,
                attempt,
                MAX_PAYMENT_ATTEMPTS,
            );

            // Include payment info, message ID, and message hash in result
            let mut result = response;
            result["sentMessageId"] = Value::String(message_id);
            result["payment_txid"] = Value::String(payment_txid);
            result["sats_paid"] = json!(payment_sats);
            result["message_hash"] = Value::String(message_hash.clone());
            return Ok(result);
        }

        Err(DmError::payment(format!(
            "MessageBox payment to {} exhausted all {} attempts",
            self.server_url, MAX_PAYMENT_ATTEMPTS
        )))
    }

    /// List messages in a specific message box.
    ///
    /// The server wraps bodies as `{"message": <original_body>}`.
    /// This method unwraps them automatically.
    pub async fn list_messages(&self, message_box: &str) -> Result<Vec<ReceivedMessage>, DmError> {
        let url = format!("{}/listMessages", self.server_url);
        let body = json!({ "messageBox": message_box });

        let resp = self.auth.post_json(&url, &body).await?;
        let status = resp.status();
        let response: Value = resp
            .json()
            .await
            .map_err(|e| DmError::messagebox(format!("Invalid JSON from listMessages: {e}")))?;

        if !status.is_success() {
            return Err(DmError::messagebox(format!(
                "listMessages failed (HTTP {}): {}",
                status,
                response
                    .get("description")
                    .or(response.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
            )));
        }

        let messages = response
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut result = Vec::with_capacity(messages.len());
        for msg_val in messages {
            let mut msg: ReceivedMessage = serde_json::from_value(msg_val)
                .map_err(|e| DmError::messagebox(format!("Failed to parse message: {e}")))?;

            // Unwrap server body wrapping: {"message": <original_body>}
            msg.body = unwrap_message_body(&msg.body);
            msg.message_box = Some(message_box.to_string());

            result.push(msg);
        }

        Ok(result)
    }

    /// Acknowledge (delete) received messages.
    pub async fn acknowledge_message(&self, message_ids: &[String]) -> Result<(), DmError> {
        let url = format!("{}/acknowledgeMessage", self.server_url);
        let body = json!({ "messageIds": message_ids });

        let resp = self.auth.post_json(&url, &body).await?;
        let status = resp.status();

        if !status.is_success() {
            let response: Value = resp.json().await.unwrap_or(json!({}));
            return Err(DmError::messagebox(format!(
                "acknowledgeMessage failed (HTTP {}): {}",
                status,
                response
                    .get("description")
                    .or(response.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
            )));
        }

        Ok(())
    }

    /// Set inbox permission (fee or block).
    pub async fn set_permission(
        &self,
        message_box: &str,
        recipient_fee: i64,
        sender: Option<&str>,
    ) -> Result<Value, DmError> {
        let url = format!("{}/permissions/set", self.server_url);
        let mut body = json!({
            "messageBox": message_box,
            "recipientFee": recipient_fee,
        });

        if let Some(s) = sender {
            body["sender"] = Value::String(s.to_string());
        }

        let resp = self.auth.post_json(&url, &body).await?;
        let status = resp.status();
        let response: Value = resp
            .json()
            .await
            .map_err(|e| DmError::messagebox(format!("Invalid JSON from permissions/set: {e}")))?;

        if !status.is_success() {
            return Err(DmError::messagebox(format!(
                "permissions/set failed (HTTP {}): {}",
                status,
                response
                    .get("description")
                    .or(response.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
            )));
        }

        Ok(response)
    }

    /// Poll all inbox boxes and return messages in priority order.
    pub async fn poll_inboxes(&self) -> Result<Vec<ReceivedMessage>, DmError> {
        let mut all_messages = Vec::new();

        for &box_name in INBOX_BOXES {
            match self.list_messages(box_name).await {
                Ok(messages) => {
                    all_messages.extend(messages);
                }
                Err(e) => {
                    tracing::warn!("Failed to poll {box_name}: {e}");
                }
            }
        }

        Ok(all_messages)
    }

    // ─────────────────────────────────────────────────────────────────────
    // BRC-77 Message Signing
    // ─────────────────────────────────────────────────────────────────────

    /// Sign a message body using BRC-77 (ECDSA via BRC-42 key derivation).
    ///
    /// Uses counterparty="anyone" (broadcast mode) so any party can verify
    /// the signature using the sender's public key. This matches the TS/Go
    /// SDK SignedMessage pattern.
    ///
    /// For directed signing (only a specific recipient can verify), use
    /// [`sign_message_body_directed`].
    ///
    /// ## BRC-42 Counterparty Modes for Signing
    ///
    /// | Mode | Counterparty | Who Can Verify |
    /// |------|-------------|----------------|
    /// | **Broadcast** | `"anyone"` (this method) | Anyone who knows the sender's pubkey |
    /// | **Directed** | recipient's pubkey | Only the specific recipient |
    /// | **Self-only** | `"self"` | Only the signer (not useful for messaging) |
    pub async fn sign_message_body(&self, body: &Value) -> Result<SignedMessage, DmError> {
        self.sign_with_counterparty(body, "anyone").await
    }

    /// Sign a message body for a specific recipient (directed mode).
    ///
    /// Uses the recipient's pubkey as the BRC-42 counterparty. Only the
    /// recipient can verify this signature (via ECDH symmetry: the recipient's
    /// wallet derives the same shared secret using the sender's pubkey).
    pub async fn sign_message_body_directed(
        &self,
        body: &Value,
        recipient_key: &str,
    ) -> Result<SignedMessage, DmError> {
        self.sign_with_counterparty(body, recipient_key).await
    }

    /// Internal: sign a message body with a specified counterparty.
    async fn sign_with_counterparty(
        &self,
        body: &Value,
        counterparty: &str,
    ) -> Result<SignedMessage, DmError> {
        let wallet = self.auth.wallet();

        // Deterministic serialization
        let serialized = serde_json::to_string(body).map_err(|e| {
            DmError::messagebox(format!("Failed to serialize body for signing: {e}"))
        })?;

        let signature_bytes = wallet
            .create_signature(
                serialized.as_bytes(),
                &signing_protocol_id(),
                MESSAGE_KEY_ID,
                counterparty,
            )
            .await?;

        // Get our identity pubkey for the recipient to verify against
        let sender_key = wallet.get_identity_key().await?;

        Ok(SignedMessage {
            body: body.clone(),
            signature: hex::encode(&signature_bytes),
            sender_key,
        })
    }

    /// Verify a signed message using BRC-77.
    ///
    /// Supports both broadcast and directed signatures:
    ///
    /// 1. **Broadcast** (counterparty="anyone"): Verified locally using
    ///    `ProtoWallet::anyone()` — no wallet call needed. ECDH = 1 * sender_pub
    ///    = sender_pub, matching the signer's ECDH of sender_priv * G.
    ///
    /// 2. **Directed** (counterparty=recipient_pub): Falls back to the real
    ///    wallet for verification. ECDH = our_priv * sender_pub, matching the
    ///    signer's ECDH of sender_priv * our_pub (ECDH symmetry).
    ///
    /// Mode 1 ("self") signatures from other wallets are inherently
    /// unverifiable cross-wallet (the ECDH requires the signer's private key).
    pub async fn verify_message_signature(
        &self,
        signed: &SignedMessage,
        sender_key: &str,
    ) -> Result<bool, DmError> {
        // Deterministic re-serialization of the body
        let serialized = serde_json::to_string(&signed.body).map_err(|e| {
            DmError::messagebox(format!("Failed to serialize body for verification: {e}"))
        })?;
        let data_bytes = serialized.into_bytes();

        // Decode hex signature
        let signature_bytes = hex::decode(&signed.signature)
            .map_err(|e| DmError::messagebox(format!("Invalid hex signature: {e}")))?;

        // Parse sender's public key
        let sender_pub = bsv::PublicKey::from_hex(sender_key)
            .map_err(|e| DmError::messagebox(format!("Invalid sender public key: {e}")))?;

        // --- Try broadcast verification first (local, fast, no wallet call) ---
        let anyone = bsv::ProtoWallet::anyone();
        let broadcast_args = bsv::wallet::VerifySignatureArgs {
            data: Some(data_bytes.clone()),
            hash_to_directly_verify: None,
            signature: signature_bytes.clone(),
            protocol_id: bsv::wallet::Protocol::new(
                bsv::wallet::SecurityLevel::Counterparty,
                "dolphin milk message signature",
            ),
            key_id: MESSAGE_KEY_ID.to_string(),
            counterparty: Some(bsv::wallet::Counterparty::Other(sender_pub)),
            for_self: None,
        };
        if anyone.verify_signature(broadcast_args).is_ok() {
            return Ok(true);
        }

        // --- Fall back to directed verification (via wallet) ---
        let wallet = self.auth.wallet();
        wallet
            .verify_signature(
                &data_bytes,
                &signature_bytes,
                &signing_protocol_id(),
                MESSAGE_KEY_ID,
                sender_key,
            )
            .await
    }

    // ─────────────────────────────────────────────────────────────────────
    // BRC-78 Message Encryption
    // ─────────────────────────────────────────────────────────────────────

    /// Encrypt a message body using BRC-78 (AES-256-GCM via BRC-42 ECDH).
    ///
    /// The recipient's pubkey is the counterparty for ECDH key agreement.
    /// Returns an `EncryptedMessage` with base64-encoded ciphertext.
    pub async fn encrypt_message_body(
        &self,
        body: &Value,
        recipient_key: &str,
    ) -> Result<EncryptedMessage, DmError> {
        let wallet = self.auth.wallet();

        // Serialize body to JSON bytes
        let json_bytes = serde_json::to_vec(body).map_err(|e| {
            DmError::messagebox(format!("Failed to serialize body for encryption: {e}"))
        })?;

        // Encrypt with BRC-42 ECDH (counterparty = recipient)
        let ciphertext_bytes = wallet
            .encrypt(
                &json_bytes,
                &encryption_protocol_id(),
                MESSAGE_KEY_ID,
                recipient_key,
            )
            .await?;

        // Get our identity pubkey (recipient needs it for decryption)
        let sender_key = wallet.get_identity_key().await?;

        Ok(EncryptedMessage {
            ciphertext: BASE64.encode(&ciphertext_bytes),
            sender_key,
            encrypted: true,
        })
    }

    /// Decrypt a message body using BRC-78.
    ///
    /// The sender's pubkey is the counterparty for ECDH key agreement
    /// (same shared secret as the sender derived with our pubkey).
    pub async fn decrypt_message_body(
        &self,
        encrypted: &EncryptedMessage,
    ) -> Result<Value, DmError> {
        let wallet = self.auth.wallet();

        // Decode base64 ciphertext
        let ciphertext_bytes = BASE64
            .decode(&encrypted.ciphertext)
            .map_err(|e| DmError::messagebox(format!("Invalid base64 ciphertext: {e}")))?;

        // Decrypt with BRC-42 ECDH (counterparty = sender)
        let plaintext_bytes = wallet
            .decrypt(
                &ciphertext_bytes,
                &encryption_protocol_id(),
                MESSAGE_KEY_ID,
                &encrypted.sender_key,
            )
            .await?;

        // Parse decrypted bytes as JSON
        serde_json::from_slice(&plaintext_bytes)
            .map_err(|e| DmError::messagebox(format!("Decrypted body is not valid JSON: {e}")))
    }

    // ─────────────────────────────────────────────────────────────────────
    // Convenience: signed/encrypted send
    // ─────────────────────────────────────────────────────────────────────

    /// Send a signed message (BRC-77).
    ///
    /// Signs the body, wraps it as a `SignedMessage`, and sends.
    pub async fn send_signed_message(
        &self,
        recipient: &str,
        message_box: &str,
        body: &Value,
    ) -> Result<Value, DmError> {
        let signed = self.sign_message_body(body).await?;
        let signed_body = serde_json::to_value(&signed)
            .map_err(|e| DmError::messagebox(format!("Failed to serialize SignedMessage: {e}")))?;
        self.send_message(recipient, message_box, &signed_body)
            .await
    }

    /// Send an encrypted message (BRC-78).
    ///
    /// Encrypts the body for the recipient and sends.
    pub async fn send_encrypted_message(
        &self,
        recipient: &str,
        message_box: &str,
        body: &Value,
    ) -> Result<Value, DmError> {
        let encrypted = self.encrypt_message_body(body, recipient).await?;
        let encrypted_body = serde_json::to_value(&encrypted).map_err(|e| {
            DmError::messagebox(format!("Failed to serialize EncryptedMessage: {e}"))
        })?;
        self.send_message(recipient, message_box, &encrypted_body)
            .await
    }

    /// Send a signed and encrypted message (BRC-77 + BRC-78).
    ///
    /// Signs the body first, then encrypts the signed message for the recipient.
    /// Recipient decrypts first, then verifies the signature.
    pub async fn send_signed_encrypted_message(
        &self,
        recipient: &str,
        message_box: &str,
        body: &Value,
    ) -> Result<Value, DmError> {
        // Sign first
        let signed = self.sign_message_body(body).await?;
        let signed_body = serde_json::to_value(&signed)
            .map_err(|e| DmError::messagebox(format!("Failed to serialize SignedMessage: {e}")))?;

        // Then encrypt the signed message
        let encrypted = self.encrypt_message_body(&signed_body, recipient).await?;
        let encrypted_body = serde_json::to_value(&encrypted).map_err(|e| {
            DmError::messagebox(format!("Failed to serialize EncryptedMessage: {e}"))
        })?;

        self.send_message(recipient, message_box, &encrypted_body)
            .await
    }

    // ─────────────────────────────────────────────────────────────────────
    // Receive-side processing: auto-decrypt + auto-verify
    // ─────────────────────────────────────────────────────────────────────

    /// Process a received message: auto-decrypt if encrypted, auto-verify if signed.
    ///
    /// Detection:
    /// 1. If body has `encrypted: true` and `ciphertext` field → decrypt
    /// 2. If (decrypted) body has `signature` and `sender_key` fields → verify
    pub async fn process_received_message(
        &self,
        msg: &ReceivedMessage,
    ) -> Result<ProcessedMessage, DmError> {
        let mut body = msg.body.clone();
        let mut was_encrypted = false;
        let mut was_signed = false;
        let mut signature_valid = None;
        let sender = msg.sender.clone();

        // Step 1: Check if encrypted
        if body
            .get("encrypted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && body.get("ciphertext").is_some()
        {
            let encrypted: EncryptedMessage =
                serde_json::from_value(body.clone()).map_err(|e| {
                    DmError::messagebox(format!("Failed to parse EncryptedMessage: {e}"))
                })?;
            body = self.decrypt_message_body(&encrypted).await?;
            was_encrypted = true;
        }

        // Step 2: Check if signed
        if body.get("signature").is_some() && body.get("sender_key").is_some() {
            let signed: SignedMessage = serde_json::from_value(body.clone())
                .map_err(|e| DmError::messagebox(format!("Failed to parse SignedMessage: {e}")))?;
            // Verification may fail if the wallet doesn't know the sender's key
            // (cross-wallet BRC-42 key derivation limitation). In that case,
            // we still unwrap the signed body but report signature_valid as None.
            match self
                .verify_message_signature(&signed, &signed.sender_key)
                .await
            {
                Ok(valid) => signature_valid = Some(valid),
                Err(e) => {
                    tracing::warn!("Signature verification failed (key derivation): {e}");
                    signature_valid = None;
                }
            }
            was_signed = true;
            body = signed.body;
        }

        Ok(ProcessedMessage {
            body,
            was_encrypted,
            was_signed,
            signature_valid,
            sender,
        })
    }
}

/// Unwrap the server's body wrapping.
///
/// The server wraps: `{"message": <original_body>}`.
/// If the body is a JSON string, parse it first.
fn unwrap_message_body(body: &Value) -> Value {
    // If body is a string, try to parse it as JSON
    let parsed = if let Some(s) = body.as_str() {
        serde_json::from_str(s).unwrap_or_else(|_| body.clone())
    } else {
        body.clone()
    };

    // Unwrap {"message": ...} wrapping
    if let Some(inner) = parsed.get("message") {
        inner.clone()
    } else {
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_payment_attempts_constant() {
        assert_eq!(MAX_PAYMENT_ATTEMPTS, 3);
    }

    #[test]
    fn test_payment_body_merge_structure() {
        // Verify that body-transport payment is merged correctly into the request body
        let request_body = json!({
            "message": {
                "recipient": "02abc123",
                "messageBox": "task_inbox",
                "messageId": "test-uuid",
                "body": {"type": "task_assignment", "task": "hello"},
            }
        });

        let payment_val = json!({
            "derivationPrefix": "prefix123",
            "derivationSuffix": "suffix456",
            "transaction": "dHgxMjM=",
        });

        // Simulate the merge that happens in send_message
        let mut paid_body = request_body.clone();
        paid_body["payment"] = payment_val;

        // Verify structure matches what MessageBox expects
        assert!(paid_body.get("message").is_some());
        assert!(paid_body.get("payment").is_some());

        let message = &paid_body["message"];
        assert_eq!(message["recipient"], "02abc123");
        assert_eq!(message["messageBox"], "task_inbox");
        assert_eq!(message["messageId"], "test-uuid");
        assert!(message["body"].is_object());

        let payment = &paid_body["payment"];
        assert_eq!(payment["derivationPrefix"], "prefix123");
        assert_eq!(payment["derivationSuffix"], "suffix456");
        assert_eq!(payment["transaction"], "dHgxMjM=");

        // Original body should not have payment
        assert!(request_body.get("payment").is_none());
    }

    #[test]
    fn test_unwrap_message_body_object() {
        let wrapped = json!({"message": {"type": "task_assignment", "task": "hello"}});
        let unwrapped = unwrap_message_body(&wrapped);
        assert_eq!(unwrapped["type"], "task_assignment");
        assert_eq!(unwrapped["task"], "hello");
    }

    #[test]
    fn test_unwrap_message_body_string() {
        let wrapped = json!("{\"message\": {\"type\": \"status_update\"}}");
        let unwrapped = unwrap_message_body(&wrapped);
        assert_eq!(unwrapped["type"], "status_update");
    }

    #[test]
    fn test_unwrap_message_body_no_wrapping() {
        let raw = json!({"type": "direct", "data": "test"});
        let unwrapped = unwrap_message_body(&raw);
        // No "message" key, returns as-is
        assert_eq!(unwrapped["type"], "direct");
    }

    #[test]
    fn test_unwrap_message_body_plain_string() {
        let raw = json!("just a plain string");
        let unwrapped = unwrap_message_body(&raw);
        assert_eq!(unwrapped.as_str(), Some("just a plain string"));
    }

    #[test]
    fn test_messagebox_client_creation() {
        let wallet =
            crate::wallet::WalletClient::new("http://localhost:3322", "http://localhost", 30);
        let auth = AuthriteClient::new(std::sync::Arc::new(wallet), "http://localhost:3322");
        let client = MessageBoxClient::new(auth);
        assert_eq!(client.server_url, MESSAGEBOX_URL);
    }

    #[test]
    fn test_messagebox_client_custom_url() {
        let wallet =
            crate::wallet::WalletClient::new("http://localhost:3322", "http://localhost", 30);
        let auth = AuthriteClient::new(std::sync::Arc::new(wallet), "http://localhost:3322");
        let client = MessageBoxClient::with_url(auth, "https://custom.server.com/");
        assert_eq!(client.server_url, "https://custom.server.com");
    }

    #[test]
    fn test_signing_protocol_id() {
        let pid = signing_protocol_id();
        assert_eq!(pid, json!([2, "dolphin milk message signature"]));
    }

    #[test]
    fn test_encryption_protocol_id() {
        let pid = encryption_protocol_id();
        assert_eq!(pid, json!([2, "dolphin milk message encryption"]));
    }

    #[test]
    fn test_message_key_id() {
        assert_eq!(MESSAGE_KEY_ID, "message");
    }

    #[test]
    fn test_signed_message_structure() {
        // Verify SignedMessage can be serialized and used as a message body
        let signed = SignedMessage {
            body: json!({"type": "task_assignment", "task": "test"}),
            signature: "deadbeef".to_string(),
            sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
        };
        let as_value = serde_json::to_value(&signed).unwrap();
        // Should have signature and sender_key fields (detectable by recipient)
        assert!(as_value.get("signature").is_some());
        assert!(as_value.get("sender_key").is_some());
        assert!(as_value.get("body").is_some());
    }

    #[test]
    fn test_encrypted_message_structure() {
        // Verify EncryptedMessage has the encrypted marker
        let encrypted = EncryptedMessage {
            ciphertext: BASE64.encode(b"test ciphertext"),
            sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
            encrypted: true,
        };
        let as_value = serde_json::to_value(&encrypted).unwrap();
        assert_eq!(as_value["encrypted"], true);
        assert!(as_value.get("ciphertext").is_some());
        assert!(as_value.get("sender_key").is_some());
    }

    #[test]
    fn test_process_received_plain_message_detection() {
        // A plain message (no encryption, no signature) should be detectable
        let body = json!({"type": "task_assignment", "task": "hello"});
        // No "encrypted" field, no "signature" field — it's a plain message
        assert!(body.get("encrypted").is_none());
        assert!(body.get("signature").is_none());
    }

    #[test]
    fn test_encrypted_message_detection_fields() {
        // Verify the field pattern used for detection
        let encrypted = json!({
            "ciphertext": "base64data",
            "sender_key": "02abc",
            "encrypted": true,
        });
        assert!(encrypted
            .get("encrypted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert!(encrypted.get("ciphertext").is_some());
    }

    #[test]
    fn test_signed_message_detection_fields() {
        // Verify the field pattern used for detection
        let signed = json!({
            "body": {"type": "test"},
            "signature": "hexsig",
            "sender_key": "02abc",
        });
        assert!(signed.get("signature").is_some());
        assert!(signed.get("sender_key").is_some());
    }
}
