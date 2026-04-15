//! MessageBox tools — send and receive P2P messages.
//!
//! Two tools for the agent:
//!   - `send_message`: send a message to another agent's inbox
//!   - `check_inbox`: check for incoming messages

use std::sync::Arc;

use serde_json::{json, Value};

use crate::certificates::CertificateManager;
use crate::messagebox::client::MessageBoxClient;
use crate::messagebox::types::*;
use crate::tools::registry::ToolDef;
use crate::wallet::{HttpWalletClient, WalletBackend};

async fn send_message_impl(
    params: Value,
    wallet_url: String,
    client: Arc<MessageBoxClient>,
) -> String {
    let recipient = params
        .get("recipient")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message_box = params
        .get("message_box")
        .and_then(|v| v.as_str())
        .unwrap_or(BOX_TASK_INBOX);
    let raw_body = params.get("body").cloned().unwrap_or(Value::Null);

    if recipient.is_empty() {
        return "Error: recipient is required".to_string();
    }
    if recipient.len() != 66 {
        return "Error: recipient must be a 66-char hex compressed pubkey".to_string();
    }
    if raw_body.is_null() {
        return "Error: 'body' parameter is required. Provide a JSON object, e.g. \
                {\"message\": \"your text here\"}"
            .to_string();
    }

    // Accept string bodies by auto-wrapping into {"message": "..."}
    let body = if let Some(s) = raw_body.as_str() {
        json!({ "message": s })
    } else {
        raw_body
    };

    let sign = params
        .get("sign")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let encrypt = params
        .get("encrypt")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let prove_identity = params
        .get("prove_identity")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Wrap in structured envelope if turn-taking params present
    let conversation_ref = params.get("conversation_ref").and_then(|v| v.as_str());
    let mut send_body = if let Some(conv_ref) = conversation_ref {
        let turn = params.get("turn").and_then(|v| v.as_u64()).unwrap_or(0);
        let max_turns = params
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);
        let done = params
            .get("done")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        json!({
            "type": "agent_message",
            "conversation_ref": conv_ref,
            "turn": turn,
            "max_turns": max_turns,
            "done": done,
            "body": body,
        })
    } else {
        body
    };

    // Optionally attach a BRC-52 certificate proof to the message
    if prove_identity {
        let cert_wallet: std::sync::Arc<dyn WalletBackend> =
            std::sync::Arc::new(HttpWalletClient::new(&wallet_url, "http://localhost", 30));
        let cert_mgr = CertificateManager::new(cert_wallet);
        match cert_mgr
            .prove_authorization(&["name", "capabilities"])
            .await
        {
            Ok(proof) => {
                if let Some(obj) = send_body.as_object_mut() {
                    obj.insert("certificate_proof".to_string(), proof);
                } else {
                    send_body = json!({
                        "message": send_body,
                        "certificate_proof": proof,
                    });
                }
            }
            Err(e) => {
                tracing::warn!("Failed to attach certificate proof (non-fatal): {e}");
            }
        }
    }

    // Dispatch based on sign/encrypt flags
    let result = match (sign, encrypt) {
        (true, true) => {
            client
                .send_signed_encrypted_message(recipient, message_box, &send_body)
                .await
        }
        (true, false) => {
            client
                .send_signed_message(recipient, message_box, &send_body)
                .await
        }
        (false, true) => {
            client
                .send_encrypted_message(recipient, message_box, &send_body)
                .await
        }
        (false, false) => {
            client
                .send_message(recipient, message_box, &send_body)
                .await
        }
    };

    match result {
        Ok(result) => serde_json::to_string(&result)
            .unwrap_or_else(|_| "Error: serialization failed".to_string()),
        Err(e) => format!("Error: {e}"),
    }
}

async fn check_inbox_impl(params: Value, client: Arc<MessageBoxClient>) -> String {
    let message_box = params.get("message_box").and_then(|v| v.as_str());
    let decrypt = params
        .get("decrypt")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // If specific box requested, check just that box.
    // Otherwise, poll all inboxes.
    let result = if let Some(box_name) = message_box {
        client.list_messages(box_name).await
    } else {
        client.poll_inboxes().await
    };

    match result {
        Ok(messages) => {
            let mut summaries: Vec<Value> = Vec::with_capacity(messages.len());

            for m in &messages {
                if decrypt {
                    // Process message: auto-decrypt + auto-verify
                    match client.process_received_message(m).await {
                        Ok(processed) => {
                            summaries.push(json!({
                                "messageId": m.message_id,
                                "sender": processed.sender,
                                "messageBox": m.message_box,
                                "body": processed.body,
                                "createdAt": m.created_at,
                                "was_encrypted": processed.was_encrypted,
                                "was_signed": processed.was_signed,
                                "signature_valid": processed.signature_valid,
                            }));
                        }
                        Err(e) => {
                            // If decryption/verification fails, return raw body with error
                            summaries.push(json!({
                                "messageId": m.message_id,
                                "sender": m.sender,
                                "messageBox": m.message_box,
                                "body": m.body,
                                "createdAt": m.created_at,
                                "decrypt_error": format!("{e}"),
                            }));
                        }
                    }
                } else {
                    summaries.push(json!({
                        "messageId": m.message_id,
                        "sender": m.sender,
                        "messageBox": m.message_box,
                        "body": m.body,
                        "createdAt": m.created_at,
                    }));
                }
            }

            serde_json::to_string(&json!({
                "messages": summaries,
                "count": summaries.len(),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string())
        }
        Err(e) => format!("Error: {e}"),
    }
}

async fn set_permission_impl(params: Value, client: Arc<MessageBoxClient>) -> String {
    let message_box = params
        .get("message_box")
        .and_then(|v| v.as_str())
        .unwrap_or(BOX_TASK_INBOX);
    let recipient_fee = params
        .get("recipient_fee")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let sender = params.get("sender").and_then(|v| v.as_str());

    match client
        .set_permission(message_box, recipient_fee, sender)
        .await
    {
        Ok(result) => serde_json::to_string(&json!({
            "success": true,
            "message_box": message_box,
            "recipient_fee": recipient_fee,
            "result": result,
        }))
        .unwrap_or_else(|_| "Error: serialization failed".to_string()),
        Err(e) => format!("Error: {e}"),
    }
}

/// Create all messagebox tool definitions.
///
/// Accepts a shared `MessageBoxClient` to prevent BRC-31 session churn.
/// All tools share the same underlying auth session with messagebox.babbage.systems.
/// `wallet_url` is only used for BRC-52 certificate proofs (prove_identity).
pub fn all_messagebox_tools_shared(
    messagebox: Arc<MessageBoxClient>,
    wallet_url: String,
) -> Vec<ToolDef> {
    let send_client = messagebox.clone();
    let check_client = messagebox.clone();
    let perm_client = messagebox;
    let send_wallet_url = Arc::new(wallet_url);

    vec![
        ToolDef {
            name: "send_message".to_string(),
            description: "Send a message to another agent via BRC-33 MessageBox. \
                Messages are delivered to the recipient's inbox and persist until acknowledged. \
                Supports BRC-77 signing (proves authenticity) and BRC-78 encryption (confidentiality)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "Recipient's identity key (66-char hex compressed pubkey)"
                    },
                    "message_box": {
                        "type": "string",
                        "description": "Target inbox: 'task_inbox', 'status_inbox', 'results_inbox', or 'dolphin_milk_coordination'",
                        "enum": [BOX_TASK_INBOX, BOX_STATUS_INBOX, BOX_RESULTS_INBOX, BOX_DM_COORDINATION]
                    },
                    "body": {
                        "description": "REQUIRED. The message content. Pass a JSON object like {\"message\": \"your text\", \"task\": \"details\"} or a plain string (auto-wrapped to {\"message\": \"...\"}). Must not be omitted."
                    },
                    "sign": {
                        "type": "boolean",
                        "description": "Sign the message with BRC-77 ECDSA (proves authenticity). Default false."
                    },
                    "encrypt": {
                        "type": "boolean",
                        "description": "Encrypt the message with BRC-78 for the recipient (confidentiality). Default false."
                    },
                    "conversation_ref": {
                        "type": "string",
                        "description": "Reference ID for ongoing exchange (enables turn-taking)"
                    },
                    "turn": {
                        "type": "integer",
                        "description": "Current turn number in the exchange (default 0)"
                    },
                    "max_turns": {
                        "type": "integer",
                        "description": "Maximum exchanges before stopping (default 5)"
                    },
                    "done": {
                        "type": "boolean",
                        "description": "Set true to signal end of exchange"
                    },
                    "prove_identity": {
                        "type": "boolean",
                        "description": "Attach a BRC-52 certificate proof to the message (reveals name + capabilities)"
                    }
                },
                "required": ["recipient", "body"]
            }),
            execute: {
                let client = send_client.clone();
                let wurl = send_wallet_url.clone();
                Box::new(move |params| {
                    let c = client.clone();
                    let u = wurl.as_ref().clone();
                    Box::pin(send_message_impl(params, u, c))
                })
            },
            category: "messagebox".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "check_inbox".to_string(),
            description: "Check for incoming messages in your MessageBox inboxes. \
                Returns messages from all boxes (task_inbox, results_inbox, status_inbox, \
                dolphin_milk_coordination) in priority order. Optionally check a specific box. \
                Use decrypt=true to auto-decrypt BRC-78 encrypted messages and verify BRC-77 signatures."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message_box": {
                        "type": "string",
                        "description": "Specific box to check (omit to check all inboxes)",
                        "enum": [BOX_TASK_INBOX, BOX_STATUS_INBOX, BOX_RESULTS_INBOX, BOX_DM_COORDINATION]
                    },
                    "decrypt": {
                        "type": "boolean",
                        "description": "Auto-process messages: decrypt BRC-78 encrypted bodies and verify BRC-77 signatures. Default false."
                    }
                }
            }),
            execute: {
                let client = check_client.clone();
                Box::new(move |params| {
                    let c = client.clone();
                    Box::pin(check_inbox_impl(params, c))
                })
            },
            category: "messagebox".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "set_permission".to_string(),
            description: "Set a fee or block senders for a MessageBox inbox. \
                Set recipient_fee to charge senders (in satoshis) for message delivery, \
                or set it to -1 to block a specific sender. \
                Use this to set up paid inbox spam filters."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message_box": {
                        "type": "string",
                        "description": "Inbox to set permissions for (default: task_inbox)",
                        "enum": [BOX_TASK_INBOX, BOX_STATUS_INBOX, BOX_RESULTS_INBOX, BOX_DM_COORDINATION]
                    },
                    "recipient_fee": {
                        "type": "integer",
                        "description": "Fee in satoshis senders must pay (0 = free, -1 = blocked)"
                    },
                    "sender": {
                        "type": "string",
                        "description": "Specific sender identity key to set fee for (omit for global fee)"
                    }
                },
                "required": ["recipient_fee"]
            }),
            execute: {
                let client = perm_client.clone();
                Box::new(move |params| {
                    let c = client.clone();
                    Box::pin(set_permission_impl(params, c))
                })
            },
            category: "messagebox".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
    ]
}

/// Backward-compatible constructor that creates its own MessageBoxClient
/// pointing at the default MessageBox URL from `MessageBoxConfig::default()`.
/// Prefer `all_messagebox_tools_shared()` when a shared client is available.
pub fn all_messagebox_tools(wallet_url: String) -> Vec<ToolDef> {
    all_messagebox_tools_with_mb_url(wallet_url, crate::config::MessageBoxConfig::default().url)
}

/// Create messagebox tools with an explicit MessageBox server URL.
/// Use this when the caller has access to the agent's config but not a
/// shared MessageBoxClient.
pub fn all_messagebox_tools_with_mb_url(
    wallet_url: String,
    messagebox_url: String,
) -> Vec<ToolDef> {
    let wallet: std::sync::Arc<dyn WalletBackend + Send + Sync> =
        std::sync::Arc::new(HttpWalletClient::new(&wallet_url, "http://localhost", 30));
    let auth = crate::auth::AuthriteClient::new(wallet, &wallet_url);
    let client = Arc::new(MessageBoxClient::with_url(auth, &messagebox_url));
    all_messagebox_tools_shared(client, wallet_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> Arc<MessageBoxClient> {
        let wallet: std::sync::Arc<dyn WalletBackend + Send + Sync> = std::sync::Arc::new(
            HttpWalletClient::new("http://localhost:3322", "http://localhost", 30),
        );
        let auth = crate::auth::AuthriteClient::new(wallet, "http://localhost:3322");
        Arc::new(MessageBoxClient::new(auth))
    }

    #[test]
    fn test_all_messagebox_tools_creates_three_tools() {
        let tools = all_messagebox_tools("http://localhost:3322".into());
        assert_eq!(tools.len(), 3);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"send_message"));
        assert!(names.contains(&"check_inbox"));
        assert!(names.contains(&"set_permission"));
    }

    #[tokio::test]
    async fn test_send_message_validates_recipient() {
        let result = send_message_impl(
            json!({"recipient": "", "body": {"test": true}}),
            "http://localhost:3322".into(),
            test_client(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("recipient"));
    }

    #[tokio::test]
    async fn test_send_message_validates_recipient_length() {
        let result = send_message_impl(
            json!({"recipient": "short", "body": {"test": true}}),
            "http://localhost:3322".into(),
            test_client(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("66-char"));
    }

    #[tokio::test]
    async fn test_send_message_validates_body() {
        let result = send_message_impl(
            json!({"recipient": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"}),
            "http://localhost:3322".into(),
            test_client(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("body"));
        // Error message should guide the LLM on how to fix the call
        assert!(
            result.contains("JSON object"),
            "Error should include example format"
        );
    }

    #[tokio::test]
    async fn test_send_message_string_body_auto_wraps() {
        // String bodies should be auto-wrapped into {"message": "..."}
        // This test verifies the wrapping happens (will fail at network call,
        // but we can verify the body transformation by checking it doesn't
        // fail with "body is required")
        let result = send_message_impl(
            json!({
                "recipient": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                "body": "Hello, this is a plain text message"
            }),
            "http://localhost:3322".into(),
            test_client(),
        )
        .await;
        // Should NOT fail with body validation error — it should proceed
        // to the network call (which will fail since no wallet is running)
        assert!(
            !result.contains("body' parameter is required"),
            "String body should be accepted, not rejected"
        );
    }

    #[tokio::test]
    async fn test_send_message_object_body_still_works() {
        // Object bodies must continue to work unchanged
        let result = send_message_impl(
            json!({
                "recipient": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                "body": {"message": "hello", "task": "test"}
            }),
            "http://localhost:3322".into(),
            test_client(),
        )
        .await;
        assert!(
            !result.contains("body' parameter is required"),
            "Object body should be accepted"
        );
    }

    #[test]
    fn test_send_message_schema_accepts_body_types() {
        let tools = all_messagebox_tools("http://localhost:3322".into());
        let send = tools.iter().find(|t| t.name == "send_message").unwrap();

        let body_schema = &send.parameters["properties"]["body"];
        // Schema should NOT restrict to "type": "object" only
        assert!(
            body_schema.get("type").is_none(),
            "body schema should not restrict type to allow both string and object"
        );
        // Description should mention REQUIRED
        let desc = body_schema["description"].as_str().unwrap();
        assert!(
            desc.contains("REQUIRED"),
            "body description should emphasize it's required"
        );
    }
}
