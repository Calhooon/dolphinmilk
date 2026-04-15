//! Conversation introspection tools — let the agent inspect its own conversation history.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::conversation::ConversationManager;
use crate::tools::registry::ToolDef;

/// Max chars of message content returned before truncation.
const MAX_CONTENT_LEN: usize = 500;

pub fn all_conversation_tools(workspace: PathBuf) -> Vec<ToolDef> {
    let ws1 = workspace.clone();
    let ws2 = workspace;

    vec![
        ToolDef {
            name: "list_conversations".to_string(),
            description:
                "List recent conversations with titles, message counts, and cost summaries"
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max conversations to return (default 10)" }
                }
            }),
            execute: Box::new(move |params: Value| {
                let ws = ws1.clone();
                Box::pin(async move {
                    let mgr = ConversationManager::new(&ws);
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                    let convs = match mgr.list() {
                        Ok(c) => c,
                        Err(e) => return format!("Error listing conversations: {e}"),
                    };
                    let results: Vec<Value> = convs
                        .into_iter()
                        .take(limit)
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "title": c.title,
                                "message_count": c.message_count,
                                "total_sats": c.total_sats,
                                "updated_at": c.updated_at,
                                "task_ids": c.task_ids,
                            })
                        })
                        .collect();
                    json!({ "conversations": results, "count": results.len() }).to_string()
                })
            }),
            category: "conversation".to_string(),
            cleanup: None,
            deferred: true,
            always_load: false,
            search_hint: Some("List recent conversations with costs".to_string()),
        },
        ToolDef {
            name: "read_conversation".to_string(),
            description: "Read messages from a specific conversation".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation_id": { "type": "string", "description": "Conversation ID (e.g., conv-abc123)" },
                    "limit": { "type": "integer", "description": "Max messages, most recent first (default 20)" }
                },
                "required": ["conversation_id"]
            }),
            execute: Box::new(move |params: Value| {
                let ws = ws2.clone();
                Box::pin(async move {
                    let conv_id = match params.get("conversation_id").and_then(|v| v.as_str()) {
                        Some(id) => id.to_string(),
                        None => return "Error: conversation_id is required".to_string(),
                    };
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

                    let mgr = ConversationManager::new(&ws);
                    let messages = match mgr.load_messages(&conv_id) {
                        Ok(m) => m,
                        Err(e) => return format!("Error loading conversation {conv_id}: {e}"),
                    };

                    if messages.is_empty() {
                        return json!({ "messages": [], "count": 0 }).to_string();
                    }

                    // Take last `limit` messages (most recent), preserve chronological order
                    let start = messages.len().saturating_sub(limit);
                    let recent: Vec<Value> = messages[start..]
                        .iter()
                        .map(|m| {
                            let content = if m.content.len() > MAX_CONTENT_LEN {
                                let safe_end = m.content.floor_char_boundary(MAX_CONTENT_LEN);
                                format!("{}...[truncated]", &m.content[..safe_end])
                            } else {
                                m.content.clone()
                            };
                            json!({
                                "seq": m.seq,
                                "role": m.role,
                                "content": content,
                                "task_id": m.task_id,
                                "ts": m.ts,
                            })
                        })
                        .collect();
                    json!({ "messages": recent, "count": recent.len() }).to_string()
                })
            }),
            category: "conversation".to_string(),
            cleanup: None,
            deferred: true,
            always_load: false,
            search_hint: Some("Read messages from a conversation".to_string()),
        },
    ]
}
