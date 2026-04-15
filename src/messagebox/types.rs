//! MessageBox message body schemas.
//!
//! Defines the typed message structures exchanged between worms
//! via BRC-33 MessageBox.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default MessageBox server URL. Configurable via [`MessageBoxConfig`].
///
/// Points at the rust-message-box CF Worker deployment. The previous
/// babbage.systems instance was retired. Override via config or the
/// `DOLPHIN_MILK_MESSAGEBOX_URL` env var.
pub const MESSAGEBOX_URL: &str = "https://rust-message-box.dev-a3e.workers.dev";

/// Default MessageBox server identity key (matches [`MESSAGEBOX_URL`]).
/// Used as a fallback when 402 responses omit the auth identity header.
/// Override via [`MessageBoxConfig::server_identity_key`].
pub const MESSAGEBOX_IDENTITY_KEY: &str =
    "02d7c923b39a464c7d029a45975476f39406bc3430ac487742cbd97e0d428343b9";

/// Worm inbox box names.
pub const BOX_TASK_INBOX: &str = "task_inbox";
pub const BOX_STATUS_INBOX: &str = "status_inbox";
pub const BOX_RESULTS_INBOX: &str = "results_inbox";
pub const BOX_DM_COORDINATION: &str = "dolphin_milk_coordination";

/// Default fee for task_inbox (economic spam filter).
pub const TASK_INBOX_FEE: i64 = 100;

/// All inbox boxes in priority order.
pub const INBOX_BOXES: &[&str] = &[
    BOX_TASK_INBOX,
    BOX_RESULTS_INBOX,
    BOX_STATUS_INBOX,
    BOX_DM_COORDINATION,
];

/// Task assignment — sent to a worm's task_inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignment {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub version: u32,
    pub task_id: String,
    pub task: String,
    pub budget_sats: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline_utc: Option<String>,
    pub response_box: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_box: Option<String>,
    pub requester_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

impl TaskAssignment {
    pub fn new(task_id: &str, task: &str, budget_sats: u64, requester_key: &str) -> Self {
        Self {
            msg_type: "task_assignment".into(),
            version: 1,
            task_id: task_id.into(),
            task: task.into(),
            budget_sats,
            deadline_utc: None,
            response_box: BOX_RESULTS_INBOX.into(),
            status_box: Some(BOX_STATUS_INBOX.into()),
            requester_key: requester_key.into(),
            priority: Some("normal".into()),
            context: None,
        }
    }
}

/// Status update — sent to requester's status_inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusUpdate {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub version: u32,
    pub task_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sats_spent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_utc: Option<String>,
}

impl StatusUpdate {
    pub fn new(task_id: &str, status: &str) -> Self {
        Self {
            msg_type: "status_update".into(),
            version: 1,
            task_id: task_id.into(),
            status: status.into(),
            progress_pct: None,
            sats_spent: None,
            current_step: None,
            timestamp_utc: None,
        }
    }
}

/// Task result — sent to requester's results_inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub version: u32,
    pub task_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proofs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_utc: Option<String>,
}

impl TaskResult {
    pub fn completed(task_id: &str, result: Value) -> Self {
        Self {
            msg_type: "task_result".into(),
            version: 1,
            task_id: task_id.into(),
            status: "completed".into(),
            result: Some(result),
            cost: None,
            proofs: None,
            failure_reason: None,
            failure_detail: None,
            timestamp_utc: None,
        }
    }

    pub fn failed(task_id: &str, reason: &str, detail: &str) -> Self {
        Self {
            msg_type: "task_result".into(),
            version: 1,
            task_id: task_id.into(),
            status: "failed".into(),
            result: None,
            cost: None,
            proofs: None,
            failure_reason: Some(reason.into()),
            failure_detail: Some(detail.into()),
            timestamp_utc: None,
        }
    }
}

/// Coordination signal — sent to another worm's dolphin_milk_coordination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationSignal {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub version: u32,
    pub signal: String,
    pub sender_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_budget_sats: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_load: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_tasks: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_utc: Option<String>,
}

impl CoordinationSignal {
    pub fn heartbeat(sender_key: &str) -> Self {
        Self {
            msg_type: "coordination".into(),
            version: 1,
            signal: "heartbeat".into(),
            sender_key: sender_key.into(),
            capabilities: None,
            available_budget_sats: None,
            current_load: None,
            max_concurrent_tasks: None,
            timestamp_utc: None,
        }
    }
}

/// A message received from the MessageBox server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedMessage {
    #[serde(rename = "messageId")]
    pub message_id: String,
    pub body: Value,
    pub sender: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// The box this message was retrieved from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_box: Option<String>,
}

/// Quote response from /permissions/quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryQuote {
    #[serde(rename = "deliveryFee")]
    pub delivery_fee: i64,
    #[serde(rename = "recipientFee")]
    pub recipient_fee: i64,
}

impl DeliveryQuote {
    /// Total cost in satoshis (0 means free).
    pub fn total_cost(&self) -> i64 {
        let delivery = self.delivery_fee.max(0);
        let recipient = self.recipient_fee.max(0);
        delivery + recipient
    }

    /// Whether any payment is required.
    pub fn requires_payment(&self) -> bool {
        self.total_cost() > 0
    }

    /// Whether the recipient has blocked the sender.
    pub fn is_blocked(&self) -> bool {
        self.recipient_fee == -1
    }
}

/// BRC-77 signed message — body + ECDSA signature + sender identity.
///
/// The body is signed deterministically (canonical JSON serialization).
/// The recipient uses the sender's pubkey to verify the signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedMessage {
    /// Original message body.
    pub body: Value,
    /// Hex-encoded ECDSA signature of the canonical JSON body.
    pub signature: String,
    /// Sender's identity pubkey (66-char hex compressed).
    pub sender_key: String,
}

/// BRC-78 encrypted message — ciphertext + sender identity.
///
/// The body is encrypted via BRC-42 ECDH key agreement
/// between sender and recipient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedMessage {
    /// Base64-encoded encrypted body.
    pub ciphertext: String,
    /// Sender's identity pubkey (needed for decryption via ECDH).
    pub sender_key: String,
    /// Marker flag — always true.
    pub encrypted: bool,
}

/// Result of processing a received message (decrypt + verify).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedMessage {
    /// The unwrapped message body (decrypted and/or signature-stripped).
    pub body: Value,
    /// Whether the message was encrypted.
    pub was_encrypted: bool,
    /// Whether the message was signed.
    pub was_signed: bool,
    /// Signature verification result (None if not signed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_valid: Option<bool>,
    /// Sender's identity pubkey.
    pub sender: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(
            MESSAGEBOX_URL,
            "https://rust-message-box.dev-a3e.workers.dev"
        );
        assert_eq!(MESSAGEBOX_IDENTITY_KEY.len(), 66);
        assert!(
            MESSAGEBOX_IDENTITY_KEY.starts_with("02") || MESSAGEBOX_IDENTITY_KEY.starts_with("03")
        );
        assert_eq!(INBOX_BOXES.len(), 4);
        assert_eq!(TASK_INBOX_FEE, 100);
    }

    #[test]
    fn test_task_assignment_serde() {
        let task = TaskAssignment::new(
            "test-uuid",
            "Research BSV agents",
            50000,
            "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
        );
        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("\"type\":\"task_assignment\""));
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"budget_sats\":50000"));

        let back: TaskAssignment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.task_id, "test-uuid");
        assert_eq!(back.task, "Research BSV agents");
        assert_eq!(back.budget_sats, 50000);
    }

    #[test]
    fn test_status_update_serde() {
        let status = StatusUpdate::new("task-123", "in_progress");
        let json = serde_json::to_string(&status).unwrap();
        let back: StatusUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.task_id, "task-123");
        assert_eq!(back.status, "in_progress");
        assert_eq!(back.msg_type, "status_update");
    }

    #[test]
    fn test_task_result_completed() {
        let result = TaskResult::completed("task-456", serde_json::json!({"summary": "done"}));
        let json = serde_json::to_string(&result).unwrap();
        let back: TaskResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, "completed");
        assert!(back.result.is_some());
        assert!(back.failure_reason.is_none());
    }

    #[test]
    fn test_task_result_failed() {
        let result = TaskResult::failed("task-789", "budget_exhausted", "Ran out of sats");
        let json = serde_json::to_string(&result).unwrap();
        let back: TaskResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, "failed");
        assert_eq!(back.failure_reason.as_deref(), Some("budget_exhausted"));
        assert!(back.result.is_none());
    }

    #[test]
    fn test_coordination_signal_serde() {
        let signal = CoordinationSignal::heartbeat(
            "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
        );
        let json = serde_json::to_string(&signal).unwrap();
        let back: CoordinationSignal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.signal, "heartbeat");
        assert_eq!(back.msg_type, "coordination");
    }

    #[test]
    fn test_delivery_quote_free() {
        let quote = DeliveryQuote {
            delivery_fee: 0,
            recipient_fee: 0,
        };
        assert_eq!(quote.total_cost(), 0);
        assert!(!quote.requires_payment());
        assert!(!quote.is_blocked());
    }

    #[test]
    fn test_delivery_quote_paid() {
        let quote = DeliveryQuote {
            delivery_fee: 10,
            recipient_fee: 100,
        };
        assert_eq!(quote.total_cost(), 110);
        assert!(quote.requires_payment());
        assert!(!quote.is_blocked());
    }

    #[test]
    fn test_delivery_quote_blocked() {
        let quote = DeliveryQuote {
            delivery_fee: 0,
            recipient_fee: -1,
        };
        assert!(quote.is_blocked());
        assert_eq!(quote.total_cost(), 0); // blocked, not payable
    }

    #[test]
    fn test_received_message_serde() {
        let json = serde_json::json!({
            "messageId": "msg-001",
            "body": {"message": {"type": "status_update"}},
            "sender": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
            "createdAt": "2026-02-23T15:30:00Z",
        });
        let msg: ReceivedMessage = serde_json::from_value(json).unwrap();
        assert_eq!(msg.message_id, "msg-001");
        assert_eq!(msg.sender.len(), 66);
    }

    #[test]
    fn test_signed_message_serde() {
        let signed = SignedMessage {
            body: serde_json::json!({"type": "task_assignment", "task": "hello"}),
            signature: "abcdef1234567890".to_string(),
            sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
        };
        let json = serde_json::to_string(&signed).unwrap();
        let back: SignedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.signature, "abcdef1234567890");
        assert_eq!(back.sender_key.len(), 66);
        assert_eq!(back.body["type"], "task_assignment");
    }

    #[test]
    fn test_encrypted_message_serde() {
        let encrypted = EncryptedMessage {
            ciphertext: "dGVzdCBjaXBoZXJ0ZXh0".to_string(),
            sender_key: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
            encrypted: true,
        };
        let json = serde_json::to_string(&encrypted).unwrap();
        let back: EncryptedMessage = serde_json::from_str(&json).unwrap();
        assert!(back.encrypted);
        assert_eq!(back.ciphertext, "dGVzdCBjaXBoZXJ0ZXh0");
        assert_eq!(back.sender_key.len(), 66);
    }

    #[test]
    fn test_encrypted_message_always_has_encrypted_true() {
        let encrypted = EncryptedMessage {
            ciphertext: "data".to_string(),
            sender_key: "02abc".to_string(),
            encrypted: true,
        };
        assert!(encrypted.encrypted);
    }

    #[test]
    fn test_processed_message_serde() {
        let processed = ProcessedMessage {
            body: serde_json::json!({"task": "hello"}),
            was_encrypted: true,
            was_signed: true,
            signature_valid: Some(true),
            sender: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
        };
        let json = serde_json::to_string(&processed).unwrap();
        let back: ProcessedMessage = serde_json::from_str(&json).unwrap();
        assert!(back.was_encrypted);
        assert!(back.was_signed);
        assert_eq!(back.signature_valid, Some(true));
    }

    #[test]
    fn test_processed_message_omits_signature_valid_when_none() {
        let processed = ProcessedMessage {
            body: serde_json::json!({"data": "plain"}),
            was_encrypted: false,
            was_signed: false,
            signature_valid: None,
            sender: "02abc".to_string(),
        };
        let json = serde_json::to_string(&processed).unwrap();
        assert!(!json.contains("signature_valid"));
    }

    #[test]
    fn test_signed_message_body_preserved() {
        let body = serde_json::json!({
            "complex": {"nested": [1, 2, 3]},
            "text": "hello world"
        });
        let signed = SignedMessage {
            body: body.clone(),
            signature: "sig".to_string(),
            sender_key: "key".to_string(),
        };
        assert_eq!(signed.body, body);
    }
}
