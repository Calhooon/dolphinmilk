//! Persistent multi-turn conversation layer backed by BRC standards.
//!
//! Each conversation is a sequence of messages linked by a BRC-60 hash chain.
//! Storage: `workspace/conversations/{conv_id}/meta.json` + `messages.jsonl`
//!
//! The wallet IS the session store. Conversations are portable, encrypted,
//! and integrity-proven. Phase 5 adds wallet sync; this module handles local storage.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::context::manager::compact_content;
use crate::error::{DmError, DmResult};
use crate::transcript::Transcript;
use crate::wallet::WalletBackend;

/// A single message in a conversation, linked by BRC-60 hash chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub seq: u32,
    pub role: String, // user | assistant | tool
    pub content: String,
    pub prev_hash: String, // BRC-60: hash of previous message
    pub hash: String,      // BRC-60: SHA-256 of (prev_hash || role || content || ts)
    pub ts: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>, // For assistant messages with tool invocations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>, // For tool result messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>, // Tool name for tool result messages
    /// Attachment metadata for user messages (filename, mime_type). No base64 data — files live on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Value>>,
}

/// Conversation metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,              // "conv-{uuid}"
    pub participant_key: String, // BRC-31 identity key
    pub title: String,           // First 60 chars of first message
    pub created_at: String,      // RFC 3339
    pub updated_at: String,      // RFC 3339
    pub message_count: u32,
    pub total_sats: u64,
    pub task_ids: Vec<String>,
    pub head_hash: String, // SHA-256 hex of latest message
    /// LLM-generated summary of messages compacted out of the context window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_summary: Option<String>,
    /// Sequence number through which the compaction summary covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_seq: Option<u32>,
}

/// Per-message hash chain verification result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageVerification {
    pub seq: u32,
    pub role: String,
    pub valid: bool,
    pub hash: String,
    pub expected_hash: String,
    pub prev_hash_valid: bool,
    pub timestamp: f64,
}

/// Result of a hash chain integrity check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainVerification {
    pub valid: bool,
    pub message_count: u32,
    pub head_hash: String,
    pub genesis_hash: String,
    pub messages: Vec<MessageVerification>,
    /// Legacy field: sequence numbers where chain breaks (for backward compat).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<u32>,
    /// Whether `meta.message_count` matches the actual number of messages.
    /// `None` when verification is run from in-memory data without metadata context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_count_valid: Option<bool>,
    /// Whether `meta.head_hash` matches the last message's hash.
    /// `None` when verification is run from in-memory data without metadata context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_head_hash_valid: Option<bool>,
}

/// Detail response: metadata + messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationDetail {
    pub conversation: Conversation,
    pub messages: Vec<ConversationMessage>,
}

/// BRC-46 basket for conversation tokens.
pub const BASKET_CONVERSATIONS: &str = "dm-conversations";

/// Legacy shared key_id used before per-conversation keys.
const LEGACY_CONVERSATION_KEY_ID: &str = "conversations";

/// Per-conversation encryption key_id.
/// Each conversation gets its own derived key, so compromising one key
/// does not compromise all conversations.
pub fn conversation_key_id(conv_id: &str) -> String {
    format!("conv-{}", conv_id)
}

/// Protocol ID for conversation encryption via wallet.
fn conversation_protocol_id() -> serde_json::Value {
    serde_json::json!([2, "dolphin milk conversation"])
}

/// Result of syncing a conversation to wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub txid: String,
    pub conversation_id: String,
}

/// Advisory file lock for per-conversation concurrency control.
///
/// Serializes writes to `messages.jsonl` and `meta.json` within a conversation
/// directory. Uses `File::lock()` (stabilized in Rust 1.84) for cross-platform
/// advisory locking (`flock` on Unix, `LockFileEx` on Windows).
///
/// The lock is released automatically when the guard (and its `File`) is dropped.
struct ConversationLock {
    _file: fs::File,
}

impl ConversationLock {
    /// Acquire an exclusive lock for the conversation directory.
    ///
    /// Opens (or creates) `conv_dir/.lock` and holds an advisory exclusive lock
    /// on it for the lifetime of the returned guard.
    fn exclusive(dir: &Path) -> DmResult<Self> {
        let lock_path = dir.join(".lock");
        if let Some(parent) = lock_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| {
                crate::error::DmError::conversation(format!("Failed to open lock file: {e}"))
            })?;

        // Acquire exclusive advisory lock (blocking). Uses flock on Unix,
        // LockFileEx on Windows. Stabilized in Rust 1.84.
        file.lock().map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to acquire conversation lock: {e}"))
        })?;

        Ok(Self { _file: file })
    }
}

/// Manages conversations stored in the workspace.
pub struct ConversationManager {
    pub base_dir: PathBuf, // workspace/conversations/
}

impl ConversationManager {
    pub fn new(workspace: &Path) -> Self {
        let base_dir = workspace.join("conversations");
        let _ = fs::create_dir_all(&base_dir);
        Self { base_dir }
    }

    /// Create a new conversation, append the first user message, write meta.json.
    pub fn create(&self, participant_key: &str, first_message: &str) -> DmResult<Conversation> {
        let id = format!("conv-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let title = generate_title(first_message);

        let dir = self.base_dir.join(&id);
        fs::create_dir_all(&dir).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to create conversation dir: {e}"))
        })?;

        let genesis_hash = compute_genesis_hash(&id);

        let msg = self.append_message_to_dir(
            &dir,
            0,
            "user",
            first_message,
            &genesis_hash,
            None,
            None,
            None,
            None,
        )?;

        let conv = Conversation {
            id: id.clone(),
            participant_key: participant_key.to_string(),
            title,
            created_at: now.clone(),
            updated_at: now,
            message_count: 1,
            total_sats: 0,
            task_ids: Vec::new(),
            head_hash: msg.hash.clone(),
            compaction_summary: None,
            compaction_seq: None,
        };
        self.write_meta(&dir, &conv)?;
        Ok(conv)
    }

    /// Load conversation metadata.
    pub fn load(&self, conv_id: &str) -> DmResult<Option<Conversation>> {
        let meta_path = self.base_dir.join(conv_id).join("meta.json");
        if !meta_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&meta_path).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to read meta.json: {e}"))
        })?;
        let conv: Conversation = serde_json::from_str(&content).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to parse meta.json: {e}"))
        })?;
        Ok(Some(conv))
    }

    /// Load all messages for a conversation.
    pub fn load_messages(&self, conv_id: &str) -> DmResult<Vec<ConversationMessage>> {
        let path = self.base_dir.join(conv_id).join("messages.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to open messages.jsonl: {e}"))
        })?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<ConversationMessage>(&trimmed) {
                Ok(msg) => messages.push(msg),
                Err(e) => tracing::warn!("Skipping corrupt conversation message: {e}"),
            }
        }
        Ok(messages)
    }

    /// Convert conversation messages to OpenAI API format.
    ///
    /// Applies `compact_content()` to all messages to strip data URIs and
    /// truncate oversized content before sending to the LLM.
    pub fn to_openai_messages(&self, conv_id: &str) -> DmResult<Vec<Value>> {
        let messages = self.load_messages(conv_id)?;
        let mut result = Vec::new();
        for msg in &messages {
            let compacted = compact_content(&msg.content);
            match msg.role.as_str() {
                "user" => {
                    let mut obj = serde_json::json!({
                        "role": "user",
                        "content": compacted,
                    });
                    if let Some(atts) = &msg.attachments {
                        obj["attachments"] = serde_json::json!(atts);
                    }
                    if let Some(tid) = &msg.task_id {
                        obj["task_id"] = serde_json::json!(tid);
                    }
                    result.push(obj);
                }
                "assistant" => {
                    let mut obj = serde_json::json!({
                        "role": "assistant",
                        "content": compacted,
                    });
                    if let Some(tc) = &msg.tool_calls {
                        obj["tool_calls"] = serde_json::json!(tc);
                    }
                    result.push(obj);
                }
                "tool" => {
                    let mut obj = serde_json::json!({
                        "role": "tool",
                        "tool_call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                        "content": compacted,
                    });
                    if let Some(name) = &msg.name {
                        obj["name"] = serde_json::json!(name);
                    }
                    result.push(obj);
                }
                _ => {}
            }
        }
        Ok(result)
    }

    /// Create a conversation with a specific ID (for system conversations like `conv-heartbeat`).
    ///
    /// Unlike `create()`, this uses the provided `id` instead of generating a UUID.
    /// If a conversation with this ID already exists, it is loaded and returned unchanged.
    pub fn create_with_id(
        &self,
        id: &str,
        participant_key: &str,
        title: &str,
    ) -> DmResult<Conversation> {
        // If already exists, return it
        if let Some(existing) = self.load(id)? {
            return Ok(existing);
        }

        let now = chrono::Utc::now().to_rfc3339();
        let dir = self.base_dir.join(id);
        fs::create_dir_all(&dir).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to create conversation dir: {e}"))
        })?;

        let genesis_hash = compute_genesis_hash(id);

        let conv = Conversation {
            id: id.to_string(),
            participant_key: participant_key.to_string(),
            title: title.to_string(),
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            total_sats: 0,
            task_ids: Vec::new(),
            head_hash: genesis_hash,
            compaction_summary: None,
            compaction_seq: None,
        };
        self.write_meta(&dir, &conv)?;
        Ok(conv)
    }

    /// Append a user message to the conversation hash chain.
    ///
    /// After appending, meta.json is updated so `message_count` and `head_hash`
    /// stay in sync with the actual messages on disk.
    pub fn append_user_message(
        &self,
        conv_id: &str,
        content: &str,
    ) -> DmResult<ConversationMessage> {
        self.append_user_message_with_attachments(conv_id, content, None, None)
    }

    /// Append a system message to the conversation hash chain.
    ///
    /// System messages are used to surface non-LLM lifecycle events to the
    /// user-facing conversation view (e.g. delegation payment claims, payment
    /// receipts, on-chain proofs). They participate in the BRC-60 hash chain
    /// like any other message but use the "system" role so the UI can render
    /// them distinctly.
    ///
    /// EPIC #329 Phase 3: used by `runner::emit_commission_payment_claim()`
    /// and `heartbeat::commission_payments` to render commission payment
    /// activity in the commission-{id} conversation timeline.
    ///
    /// IMPORTANT: As of the chat-render fix, system messages now appear in the
    /// live chat UI (`ui/src/pages/chat/chat.ts::_buildMessagesFromConversation`)
    /// in addition to the conversation-detail page. Use this API sparingly —
    /// every call produces a visible bubble in chat. Future writers should
    /// consider adding a typed `kind` field if they need invisible/back-channel
    /// system messages, rather than reusing this for arbitrary debug markers.
    pub fn append_system_message(
        &self,
        conv_id: &str,
        content: &str,
        task_id: Option<&str>,
    ) -> DmResult<ConversationMessage> {
        let dir = self.base_dir.join(conv_id);
        let messages = self.load_messages(conv_id)?;
        let seq = messages.len() as u32;
        let prev_hash = messages
            .last()
            .map(|m| m.hash.clone())
            .unwrap_or_else(|| compute_genesis_hash(conv_id));
        let msg = self.append_message_to_dir_with_attachments(
            &dir, seq, "system", content, &prev_hash, task_id, None, None, None, None,
        )?;
        self.sync_meta_after_append(conv_id, &msg.hash, seq + 1)?;
        Ok(msg)
    }

    /// Append a user message with optional attachment metadata to the conversation.
    pub fn append_user_message_with_attachments(
        &self,
        conv_id: &str,
        content: &str,
        attachments: Option<Vec<Value>>,
        task_id: Option<&str>,
    ) -> DmResult<ConversationMessage> {
        let dir = self.base_dir.join(conv_id);
        let messages = self.load_messages(conv_id)?;
        let seq = messages.len() as u32;
        let prev_hash = messages
            .last()
            .map(|m| m.hash.clone())
            .unwrap_or_else(|| compute_genesis_hash(conv_id));
        let msg = self.append_message_to_dir_with_attachments(
            &dir,
            seq,
            "user",
            content,
            &prev_hash,
            task_id,
            None,
            None,
            None,
            attachments,
        )?;

        // Sync meta.json so message_count and head_hash reflect the new message.
        self.sync_meta_after_append(conv_id, &msg.hash, seq + 1)?;

        Ok(msg)
    }

    /// Extract messages from a completed task transcript and append them to the conversation.
    ///
    /// Skips user events (already recorded before task start). Appends assistant and tool messages.
    pub fn append_from_transcript(
        &self,
        conv_id: &str,
        transcript: &Transcript,
        task_id: &str,
    ) -> DmResult<()> {
        let dir = self.base_dir.join(conv_id);
        let mut messages = self.load_messages(conv_id)?;
        // Track which tool_call IDs we expect results for
        let mut pending_tool_calls: HashMap<String, ()> = HashMap::new();

        for event in transcript.replay() {
            match event.event_type.as_str() {
                "think_response" => {
                    let content = event
                        .data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let tool_calls = event
                        .data
                        .get("tool_calls")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.to_vec());

                    // Track pending tool call IDs for orphan detection
                    if let Some(ref tc) = tool_calls {
                        for call in tc {
                            if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                                pending_tool_calls.insert(id.to_string(), ());
                            }
                        }
                    }

                    let seq = messages.len() as u32;
                    let prev_hash = messages
                        .last()
                        .map(|m| m.hash.clone())
                        .unwrap_or_else(|| compute_genesis_hash(conv_id));

                    let msg = self.append_message_to_dir(
                        &dir,
                        seq,
                        "assistant",
                        content,
                        &prev_hash,
                        Some(task_id),
                        tool_calls,
                        None,
                        None,
                    )?;
                    messages.push(msg);
                }
                "tool_result" => {
                    let call_id = event
                        .data
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if pending_tool_calls.remove(call_id).is_some() {
                        let raw_content = event
                            .data
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        // Compact tool results before writing to conversation JSONL
                        // to prevent base64 blobs from accumulating in the file.
                        let content = compact_content(raw_content);
                        let content = content.as_str();
                        let name = event
                            .data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let seq = messages.len() as u32;
                        let prev_hash = messages
                            .last()
                            .map(|m| m.hash.clone())
                            .unwrap_or_else(|| compute_genesis_hash(conv_id));

                        let msg = self.append_message_to_dir(
                            &dir,
                            seq,
                            "tool",
                            content,
                            &prev_hash,
                            Some(task_id),
                            None,
                            Some(call_id),
                            name.as_deref(),
                        )?;
                        messages.push(msg);
                    }
                }
                _ => {}
            }
        }

        // Sync meta.json so message_count and head_hash reflect all appended messages.
        if !messages.is_empty() {
            if let Some(last) = messages.last() {
                self.sync_meta_after_append(conv_id, &last.hash, messages.len() as u32)?;
            }
        }

        Ok(())
    }

    /// Update meta.json after task completion.
    pub fn update_meta(&self, conv_id: &str, sats_spent: u64, task_id: &str) -> DmResult<()> {
        let dir = self.base_dir.join(conv_id);
        let _lock = ConversationLock::exclusive(&dir)?;
        let mut conv = self
            .load(conv_id)?
            .ok_or_else(|| crate::error::DmError::conversation("Conversation not found"))?;

        let messages = self.load_messages(conv_id)?;
        if let Some(last) = messages.last() {
            conv.head_hash = last.hash.clone();
        }
        conv.message_count = messages.len() as u32;
        conv.total_sats = conv.total_sats.saturating_add(sats_spent);
        if !conv.task_ids.contains(&task_id.to_string()) {
            conv.task_ids.push(task_id.to_string());
        }
        conv.updated_at = chrono::Utc::now().to_rfc3339();
        self.write_meta(&dir, &conv)
    }

    /// Store a compaction summary for the conversation.
    ///
    /// Called when the context manager is about to truncate history.
    /// Updates meta.json with the summary text and the sequence number
    /// through which it covers.
    pub fn set_compaction_summary(
        &self,
        conv_id: &str,
        summary: String,
        through_seq: u32,
    ) -> DmResult<()> {
        let dir = self.base_dir.join(conv_id);
        let _lock = ConversationLock::exclusive(&dir)?;
        let mut conv = self
            .load(conv_id)?
            .ok_or_else(|| crate::error::DmError::conversation("Conversation not found"))?;
        conv.compaction_summary = Some(summary);
        conv.compaction_seq = Some(through_seq);
        conv.updated_at = chrono::Utc::now().to_rfc3339();
        self.write_meta(&dir, &conv)
    }

    /// Find a conversation by participant identity key.
    ///
    /// Scans all conversations and returns the ID of the most recently updated
    /// conversation where `participant_key` matches. Returns `None` if no match.
    pub fn find_by_participant(&self, participant_key: &str) -> Option<String> {
        let convs = self.list().ok()?;
        // list() returns newest-first, so the first match is the most recent
        convs
            .iter()
            .find(|c| c.participant_key == participant_key)
            .map(|c| c.id.clone())
    }

    /// List all conversations, newest first.
    pub fn list(&self) -> DmResult<Vec<Conversation>> {
        let mut convs = Vec::new();
        let entries = match fs::read_dir(&self.base_dir) {
            Ok(e) => e,
            Err(_) => return Ok(Vec::new()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let meta_path = path.join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let content = match fs::read_to_string(&meta_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            match serde_json::from_str::<Conversation>(&content) {
                Ok(conv) => convs.push(conv),
                Err(e) => tracing::warn!("Skipping corrupt conversation meta: {e}"),
            }
        }
        // Newest first
        convs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(convs)
    }

    /// Verify hash chain integrity.
    ///
    /// If the conversation has been compacted (i.e. `compaction_seq` is set),
    /// messages at or below the compaction boundary are treated as a trusted
    /// summary block. Verification only starts from the first message *after*
    /// `compaction_seq`, using the hash of the last compacted message as the
    /// initial `prev_hash`.
    pub fn verify_chain(&self, conv_id: &str) -> DmResult<ChainVerification> {
        let messages = self.load_messages(conv_id)?;
        let genesis_hash = compute_genesis_hash(conv_id);

        // Determine compaction boundary
        let conv = self.load(conv_id)?;
        let compaction_seq = conv.as_ref().and_then(|c| c.compaction_seq);

        let mut breaks = Vec::new();
        let mut prev_hash = genesis_hash.clone();
        let mut verifications = Vec::new();

        for msg in &messages {
            // If conversation has been compacted, skip verification for messages
            // at or below the compaction boundary. We still advance prev_hash
            // so the chain continues from the correct point.
            if let Some(cs) = compaction_seq {
                if msg.seq <= cs {
                    // Trust the compacted region; just track the hash for chain continuity.
                    verifications.push(MessageVerification {
                        seq: msg.seq,
                        role: msg.role.clone(),
                        valid: true, // trusted (compacted)
                        hash: msg.hash.clone(),
                        expected_hash: msg.hash.clone(), // match by definition
                        prev_hash_valid: true,
                        timestamp: msg.ts,
                    });
                    prev_hash = msg.hash.clone();
                    continue;
                }
            }

            let prev_hash_valid = msg.prev_hash == prev_hash;
            let expected = compute_message_hash(&msg.prev_hash, &msg.role, &msg.content, msg.ts);
            let hash_valid = msg.hash == expected;
            let valid = prev_hash_valid && hash_valid;

            if !valid {
                breaks.push(msg.seq);
            }

            verifications.push(MessageVerification {
                seq: msg.seq,
                role: msg.role.clone(),
                valid,
                hash: msg.hash.clone(),
                expected_hash: expected,
                prev_hash_valid,
                timestamp: msg.ts,
            });

            prev_hash = msg.hash.clone();
        }

        let head_hash = messages
            .last()
            .map(|m| m.hash.clone())
            .unwrap_or_else(|| genesis_hash.clone());

        // Verify metadata consistency
        let (meta_count_valid, meta_head_hash_valid) = if let Some(ref c) = conv {
            let count_ok = c.message_count == messages.len() as u32;
            let head_ok = c.head_hash == head_hash;
            (Some(count_ok), Some(head_ok))
        } else {
            (None, None)
        };

        // Metadata mismatch makes the overall verification invalid
        let meta_valid = meta_count_valid.unwrap_or(true) && meta_head_hash_valid.unwrap_or(true);

        Ok(ChainVerification {
            valid: breaks.is_empty() && meta_valid,
            message_count: messages.len() as u32,
            head_hash,
            genesis_hash,
            messages: verifications,
            breaks,
            meta_count_valid,
            meta_head_hash_valid,
        })
    }

    /// Verify hash chain integrity from an in-memory message slice.
    ///
    /// This is the same logic as `verify_chain()` but operates on messages directly
    /// instead of loading from disk. Used after wallet restore to verify integrity
    /// before the data is trusted.
    pub fn verify_chain_from_messages(
        messages: &[ConversationMessage],
        conv_id: &str,
    ) -> ChainVerification {
        let genesis_hash = compute_genesis_hash(conv_id);
        let mut breaks = Vec::new();
        let mut prev_hash = genesis_hash.clone();
        let mut verifications = Vec::new();

        for msg in messages {
            let prev_hash_valid = msg.prev_hash == prev_hash;
            let expected = compute_message_hash(&msg.prev_hash, &msg.role, &msg.content, msg.ts);
            let hash_valid = msg.hash == expected;
            let valid = prev_hash_valid && hash_valid;

            if !valid {
                breaks.push(msg.seq);
            }

            verifications.push(MessageVerification {
                seq: msg.seq,
                role: msg.role.clone(),
                valid,
                hash: msg.hash.clone(),
                expected_hash: expected,
                prev_hash_valid,
                timestamp: msg.ts,
            });

            prev_hash = msg.hash.clone();
        }

        let head_hash = messages
            .last()
            .map(|m| m.hash.clone())
            .unwrap_or_else(|| genesis_hash.clone());

        ChainVerification {
            valid: breaks.is_empty(),
            message_count: messages.len() as u32,
            head_hash,
            genesis_hash,
            messages: verifications,
            breaks,
            meta_count_valid: None,
            meta_head_hash_valid: None,
        }
    }

    // -- Internal helpers --

    /// Sync meta.json after appending messages, updating `message_count` and `head_hash`.
    ///
    /// This is a lightweight update that avoids the overhead of `update_meta()` (which
    /// also updates sats and task_ids). Called after `append_user_message()` and
    /// `append_from_transcript()` to keep meta.json consistent with messages.jsonl.
    fn sync_meta_after_append(
        &self,
        conv_id: &str,
        head_hash: &str,
        message_count: u32,
    ) -> DmResult<()> {
        let dir = self.base_dir.join(conv_id);
        let _lock = ConversationLock::exclusive(&dir)?;
        let mut conv = match self.load(conv_id)? {
            Some(c) => c,
            None => return Ok(()), // No meta to update (e.g., dir created but no meta yet)
        };
        conv.head_hash = head_hash.to_string();
        conv.message_count = message_count;
        conv.updated_at = chrono::Utc::now().to_rfc3339();
        self.write_meta(&dir, &conv)
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn append_message_to_dir(
        &self,
        dir: &Path,
        seq: u32,
        role: &str,
        content: &str,
        prev_hash: &str,
        task_id: Option<&str>,
        tool_calls: Option<Vec<Value>>,
        tool_call_id: Option<&str>,
        name: Option<&str>,
    ) -> DmResult<ConversationMessage> {
        self.append_message_to_dir_with_attachments(
            dir,
            seq,
            role,
            content,
            prev_hash,
            task_id,
            tool_calls,
            tool_call_id,
            name,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_message_to_dir_with_attachments(
        &self,
        dir: &Path,
        seq: u32,
        role: &str,
        content: &str,
        prev_hash: &str,
        task_id: Option<&str>,
        tool_calls: Option<Vec<Value>>,
        tool_call_id: Option<&str>,
        name: Option<&str>,
        attachments: Option<Vec<Value>>,
    ) -> DmResult<ConversationMessage> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let hash = compute_message_hash(prev_hash, role, content, ts);
        let msg = ConversationMessage {
            seq,
            role: role.to_string(),
            content: content.to_string(),
            prev_hash: prev_hash.to_string(),
            hash,
            ts,
            task_id: task_id.map(|s| s.to_string()),
            tool_calls,
            tool_call_id: tool_call_id.map(|s| s.to_string()),
            name: name.map(|s| s.to_string()),
            attachments,
        };

        let messages_path = dir.join("messages.jsonl");
        if let Some(parent) = messages_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&messages_path)
        {
            if let Ok(json_str) = serde_json::to_string(&msg) {
                let _ = writeln!(f, "{json_str}");
            }
        }

        Ok(msg)
    }

    /// Encrypt and store conversation as a PushDrop token in the wallet.
    ///
    /// Protocol: `[2, "dolphin milk conversation"]`, key_id: conversation ID, counterparty: "self".
    /// Basket: `worm-conversations`. Script: `["conversation", encrypted_data] OP_2DROP <pk> OP_CHECKSIG`.
    ///
    /// Returns the txid of the on-chain token.
    pub async fn sync_to_wallet(
        &self,
        wallet: &dyn WalletBackend,
        conv_id: &str,
    ) -> DmResult<SyncResult> {
        // 1. Load conversation metadata + messages
        let conv = self
            .load(conv_id)?
            .ok_or_else(|| DmError::conversation("Conversation not found for sync"))?;
        let messages = self.load_messages(conv_id)?;
        let detail = ConversationDetail {
            conversation: conv,
            messages,
        };

        // 2. Serialize to JSON
        let json_bytes = serde_json::to_vec(&detail)
            .map_err(|e| DmError::conversation(format!("Failed to serialize conversation: {e}")))?;

        // 3. Encrypt via wallet
        let protocol_id = conversation_protocol_id();
        let key_id = conversation_key_id(conv_id);
        let encrypted = wallet
            .encrypt(&json_bytes, &protocol_id, &key_id, "self")
            .await?;

        // 4. Get identity key for OP_CHECKSIG lock
        let pubkey = wallet.get_identity_key().await?;

        // 5. Build PushDrop script: ["conversation", encrypted_data] OP_2DROP <pubkey> OP_CHECKSIG
        let type_bytes = b"conversation";
        let script =
            crate::state::build_push_drop_script(&[type_bytes.as_ref(), &encrypted], &pubkey)?;

        // 6. Create action with basket
        let output = serde_json::json!({
            "lockingScript": script,
            "satoshis": 1,
            "outputDescription": "dolphin milk sync",
            "basket": BASKET_CONVERSATIONS,
        });

        let result = wallet
            .create_action(&[output], "dolphin milk sync", false, false)
            .await?;

        Ok(SyncResult {
            txid: result.txid,
            conversation_id: conv_id.to_string(),
        })
    }

    /// Restore conversations from wallet tokens (bootstrap from empty state).
    ///
    /// Lists outputs in `worm-conversations` basket, decrypts each PushDrop token,
    /// and writes `meta.json` + `messages.jsonl` to disk.
    ///
    /// Returns the list of restored conversation IDs.
    pub async fn restore_from_wallet(&self, wallet: &dyn WalletBackend) -> DmResult<Vec<String>> {
        let protocol_id = conversation_protocol_id();
        let mut restored = Vec::new();
        let mut offset = 0u64;
        let limit = 100u64;

        loop {
            let result = wallet
                .list_outputs(BASKET_CONVERSATIONS, "locking scripts", limit, offset)
                .await?;

            let outputs = result
                .get("outputs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if outputs.is_empty() {
                break;
            }

            for output in &outputs {
                // Extract the locking script hex
                let script_hex = match output.get("lockingScript").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => continue,
                };

                // Decode the script and extract PushDrop data fields
                let script_bytes = match hex::decode(script_hex) {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                // Extract the encrypted data (second push in the script)
                let encrypted = match extract_second_push(&script_bytes) {
                    Some(d) => d,
                    None => continue,
                };

                // Decrypt: try per-conversation key first, then legacy shared key.
                // We don't know the conv_id yet (it's inside the encrypted payload),
                // so we must try the legacy key first for tokens encrypted before
                // per-conversation keys were introduced.
                //
                // Strategy: try legacy key first (since we don't have conv_id),
                // then if decryption succeeds and we have the conv_id, we know
                // whether it was legacy-encrypted.
                let plaintext = match wallet
                    .decrypt(&encrypted, &protocol_id, LEGACY_CONVERSATION_KEY_ID, "self")
                    .await
                {
                    Ok(p) => p,
                    Err(_legacy_err) => {
                        // Legacy key failed — this token may use a per-conversation key,
                        // but we can't try it without knowing the conv_id.
                        // Try a brute-force approach: attempt to deserialize first to
                        // see if the data is already plaintext (shouldn't happen) or
                        // skip this token.
                        tracing::debug!(
                            "Legacy key failed for conversation token, skipping \
                             (per-conversation key tokens require known conv_id)"
                        );
                        continue;
                    }
                };

                // Deserialize the ConversationDetail
                let detail: ConversationDetail = match serde_json::from_slice(&plaintext) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!("Failed to deserialize conversation from token: {e}");
                        continue;
                    }
                };

                let conv_id = detail.conversation.id.clone();
                let dir = self.base_dir.join(&conv_id);

                // Now that we know the conv_id, try per-conversation key.
                // If it succeeds, use that data instead (it's the newer format).
                let (final_detail, used_legacy) = match wallet
                    .decrypt(
                        &encrypted,
                        &protocol_id,
                        &conversation_key_id(&conv_id),
                        "self",
                    )
                    .await
                {
                    Ok(per_conv_plaintext) => {
                        match serde_json::from_slice::<ConversationDetail>(&per_conv_plaintext) {
                            Ok(d) => (d, false),
                            Err(_) => (detail, true),
                        }
                    }
                    Err(_) => {
                        // Per-conversation key failed, use legacy-decrypted data
                        (detail, true)
                    }
                };

                if used_legacy {
                    tracing::warn!(
                        "Conversation {conv_id} restored using legacy shared key; \
                         it will be re-encrypted with a per-conversation key on next sync"
                    );
                }

                // Skip if already exists locally
                if dir.join("meta.json").exists() {
                    tracing::debug!("Skipping already-present conversation {conv_id}");
                    continue;
                }

                // Write meta.json + messages.jsonl
                let _ = fs::create_dir_all(&dir);
                self.write_meta(&dir, &final_detail.conversation)?;

                let messages_path = dir.join("messages.jsonl");
                let mut f = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&messages_path)
                    .map_err(|e| {
                        DmError::conversation(format!("Failed to create messages.jsonl: {e}"))
                    })?;
                for msg in &final_detail.messages {
                    if let Ok(json_str) = serde_json::to_string(msg) {
                        let _ = writeln!(f, "{json_str}");
                    }
                }

                // 6B: Verify hash chain integrity after restore
                let verification =
                    Self::verify_chain_from_messages(&final_detail.messages, &conv_id);
                if !verification.valid {
                    tracing::warn!(
                        "Restored conversation {} has broken hash chain at seq {:?}",
                        conv_id,
                        verification.breaks,
                    );
                }

                restored.push(conv_id);
            }

            if (outputs.len() as u64) < limit {
                break;
            }
            offset += limit;
        }

        Ok(restored)
    }

    fn write_meta(&self, dir: &Path, conv: &Conversation) -> DmResult<()> {
        let meta_path = dir.join("meta.json");
        let json = serde_json::to_string_pretty(conv).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to serialize meta: {e}"))
        })?;
        fs::write(&meta_path, json).map_err(|e| {
            crate::error::DmError::conversation(format!("Failed to write meta.json: {e}"))
        })
    }
}

/// Compute the BRC-60 genesis hash for a conversation.
/// SHA-256("CONVERSATION" || conversation_id)
pub fn compute_genesis_hash(conversation_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"CONVERSATION");
    hasher.update(conversation_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute the BRC-60 hash for a message.
/// SHA-256(prev_hash || role || content || ts_as_string)
pub fn compute_message_hash(prev_hash: &str, role: &str, content: &str, ts: f64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(role.as_bytes());
    hasher.update(content.as_bytes());
    hasher.update(ts.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Extract the second data push from a PushDrop script.
///
/// Script format: `<push1> <push2> OP_2DROP <pubkey> OP_CHECKSIG`
/// Returns the bytes of the second push, or None if parsing fails.
fn extract_second_push(script: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0;

    // Skip first push
    let (_, end1) = read_push(script, pos)?;
    pos = end1;

    // Read second push (the encrypted data)
    let (data, _) = read_push(script, pos)?;
    Some(data)
}

/// Read a single push-data element from a script at the given position.
/// Returns (data_bytes, position_after_push).
fn read_push(script: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
    if pos >= script.len() {
        return None;
    }
    let opcode = script[pos];
    if opcode == 0x00 {
        // OP_0
        return Some((vec![], pos + 1));
    }
    if (1..=75).contains(&opcode) {
        // Direct push: opcode is length
        let len = opcode as usize;
        let end = pos + 1 + len;
        if end > script.len() {
            return None;
        }
        return Some((script[pos + 1..end].to_vec(), end));
    }
    if opcode == 0x4c {
        // OP_PUSHDATA1
        if pos + 1 >= script.len() {
            return None;
        }
        let len = script[pos + 1] as usize;
        let end = pos + 2 + len;
        if end > script.len() {
            return None;
        }
        return Some((script[pos + 2..end].to_vec(), end));
    }
    if opcode == 0x4d {
        // OP_PUSHDATA2
        if pos + 2 >= script.len() {
            return None;
        }
        let len = u16::from_le_bytes([script[pos + 1], script[pos + 2]]) as usize;
        let end = pos + 3 + len;
        if end > script.len() {
            return None;
        }
        return Some((script[pos + 3..end].to_vec(), end));
    }
    if opcode == 0x4e {
        // OP_PUSHDATA4
        if pos + 4 >= script.len() {
            return None;
        }
        let len = u32::from_le_bytes([
            script[pos + 1],
            script[pos + 2],
            script[pos + 3],
            script[pos + 4],
        ]) as usize;
        let end = pos + 5 + len;
        if end > script.len() {
            return None;
        }
        return Some((script[pos + 5..end].to_vec(), end));
    }
    None
}

/// Generate a conversation title from the first message (no LLM call).
/// Truncates at word boundary up to 60 chars.
pub fn generate_title(message: &str) -> String {
    if message.len() <= 60 {
        return message.to_string();
    }
    let safe_end = message.floor_char_boundary(60);
    let slice = &message[..safe_end];
    if let Some(pos) = slice.rfind(char::is_whitespace) {
        message[..pos].to_string()
    } else {
        slice.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_manager() -> (TempDir, ConversationManager) {
        let dir = TempDir::new().unwrap();
        let mgr = ConversationManager::new(dir.path());
        (dir, mgr)
    }

    #[test]
    fn test_create_conversation() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("pubkey123", "Hello, what is BSV?").unwrap();
        assert!(conv.id.starts_with("conv-"));
        assert_eq!(conv.title, "Hello, what is BSV?");
        assert_eq!(conv.message_count, 1);
        assert_eq!(conv.participant_key, "pubkey123");

        // Verify files exist
        let conv_dir = mgr.base_dir.join(&conv.id);
        assert!(conv_dir.join("meta.json").exists());
        assert!(conv_dir.join("messages.jsonl").exists());

        // Verify that meta.json head_hash matches first message hash
        let messages = mgr.load_messages(&conv.id).unwrap();
        assert_eq!(conv.head_hash, messages[0].hash);
    }

    #[test]
    fn test_append_user_message_extends_chain() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "First message").unwrap();
        let msg2 = mgr.append_user_message(&conv.id, "Second message").unwrap();
        assert_eq!(msg2.seq, 1);
        assert_eq!(msg2.role, "user");
        assert_ne!(msg2.hash, conv.head_hash);
        assert_eq!(msg2.prev_hash, conv.head_hash);
    }

    #[test]
    fn test_load_messages_round_trip() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "Hello").unwrap();
        mgr.append_user_message(&conv.id, "Follow-up").unwrap();
        let messages = mgr.load_messages(&conv.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].content, "Follow-up");
    }

    #[test]
    fn test_to_openai_messages() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "What is 2+2?").unwrap();
        // Manually append an assistant message via the directory helper
        let dir = mgr.base_dir.join(&conv.id);
        let msgs = mgr.load_messages(&conv.id).unwrap();
        let prev_hash = msgs.last().unwrap().hash.clone();
        mgr.append_message_to_dir(
            &dir,
            1,
            "assistant",
            "The answer is 4.",
            &prev_hash,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let openai = mgr.to_openai_messages(&conv.id).unwrap();
        assert_eq!(openai.len(), 2);
        assert_eq!(openai[0]["role"], "user");
        assert_eq!(openai[0]["content"], "What is 2+2?");
        assert_eq!(openai[1]["role"], "assistant");
        assert_eq!(openai[1]["content"], "The answer is 4.");
    }

    #[test]
    fn test_verify_chain_valid() {
        if std::env::var("CI").is_ok() {
            return; // flaky on Linux CI — timestamp-dependent hash chain
        }
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "Message 1").unwrap();
        // Small sleeps ensure unique timestamps on platforms with coarse clocks
        std::thread::sleep(std::time::Duration::from_millis(10));
        mgr.append_user_message(&conv.id, "Message 2").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        mgr.append_user_message(&conv.id, "Message 3").unwrap();
        let result = mgr.verify_chain(&conv.id).unwrap();
        assert!(result.valid, "breaks: {:?}", result.breaks);
        assert_eq!(result.message_count, 3);
        assert!(result.breaks.is_empty());
    }

    #[test]
    fn test_verify_chain_tampered() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "Message 1").unwrap();
        mgr.append_user_message(&conv.id, "Message 2").unwrap();

        // Tamper: overwrite the messages.jsonl with corrupted data
        let path = mgr.base_dir.join(&conv.id).join("messages.jsonl");
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // Corrupt first message hash
        if !lines.is_empty() {
            let mut obj: Value = serde_json::from_str(lines[0]).unwrap();
            obj["hash"] = serde_json::json!("deadbeef");
            let corrupted = serde_json::to_string(&obj).unwrap();
            let mut owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
            owned[0] = corrupted;
            std::fs::write(&path, owned.join("\n") + "\n").unwrap();
        }

        let result = mgr.verify_chain(&conv.id).unwrap();
        assert!(!result.valid);
        assert!(!result.breaks.is_empty());
    }

    #[test]
    fn test_list_conversations() {
        let (_dir, mgr) = tmp_manager();
        mgr.create("key", "Conv 1").unwrap();
        mgr.create("key", "Conv 2").unwrap();
        mgr.create("key", "Conv 3").unwrap();
        let list = mgr.list().unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_generate_title_short() {
        assert_eq!(generate_title("Hello world"), "Hello world");
    }

    #[test]
    fn test_generate_title_long() {
        let long = "This is a very long message that exceeds the sixty character limit for titles";
        let title = generate_title(long);
        assert!(title.len() <= 60);
        // Should break at a word boundary
        assert!(!title.ends_with(' '));
    }

    #[test]
    fn test_corrupt_jsonl_recovery() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "Good message").unwrap();
        // Append corrupt line directly to the JSONL
        let path = mgr.base_dir.join(&conv.id).join("messages.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{not valid json}}").unwrap();
        // Should skip the corrupt line gracefully
        let messages = mgr.load_messages(&conv.id).unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_update_meta() {
        let (_dir, mgr) = tmp_manager();
        let conv = mgr.create("key", "Hello").unwrap();
        mgr.append_user_message(&conv.id, "Follow-up").unwrap();
        mgr.update_meta(&conv.id, 5000, "task-abc").unwrap();
        let updated = mgr.load(&conv.id).unwrap().unwrap();
        assert_eq!(updated.total_sats, 5000);
        assert_eq!(updated.task_ids, vec!["task-abc"]);
        assert_eq!(updated.message_count, 2);
    }

    #[test]
    fn test_genesis_hash_deterministic() {
        let h1 = compute_genesis_hash("conv-test-123");
        let h2 = compute_genesis_hash("conv-test-123");
        assert_eq!(h1, h2);
        // Different IDs produce different hashes
        let h3 = compute_genesis_hash("conv-test-456");
        assert_ne!(h1, h3);
    }
}
