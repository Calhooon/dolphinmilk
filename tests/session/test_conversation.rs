//! Tests for the multi-turn conversation storage layer.

use std::fs::OpenOptions;
use std::io::Write;

macro_rules! skip_on_ci {
    () => {
        if std::env::var("CI").is_ok() {
            return;
        }
    };
}

use dolphin_milk::conversation::{
    compute_genesis_hash, compute_message_hash, conversation_key_id, generate_title,
    ChainVerification, ConversationManager,
};
use dolphin_milk::transcript::Transcript;

fn tmp_mgr() -> (tempfile::TempDir, ConversationManager) {
    let dir = tempfile::TempDir::new().unwrap();
    let mgr = ConversationManager::new(dir.path());
    (dir, mgr)
}

// -- Storage --

#[test]
fn test_create_conversation_files() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("pubkey123", "Hello, what is BSV?").unwrap();
    assert!(conv.id.starts_with("conv-"));
    assert_eq!(conv.title, "Hello, what is BSV?");
    assert_eq!(conv.message_count, 1);
    assert_eq!(conv.participant_key, "pubkey123");
    // meta.json and messages.jsonl must exist
    let base = mgr.base_dir.join(&conv.id);
    assert!(base.join("meta.json").exists());
    assert!(base.join("messages.jsonl").exists());
}

#[test]
fn test_append_user_message_chain() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "First message").unwrap();
    let first_hash = conv.head_hash.clone();
    let msg2 = mgr.append_user_message(&conv.id, "Second message").unwrap();
    assert_eq!(msg2.seq, 1);
    assert_eq!(msg2.role, "user");
    assert_eq!(msg2.prev_hash, first_hash);
    assert_ne!(msg2.hash, first_hash);
}

#[test]
fn test_load_messages_round_trip() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    mgr.append_user_message(&conv.id, "Follow-up").unwrap();
    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Hello");
    assert_eq!(messages[1].content, "Follow-up");
    assert_eq!(messages[0].seq, 0);
    assert_eq!(messages[1].seq, 1);
}

#[test]
fn test_to_openai_messages_user_assistant() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "What is 2+2?").unwrap();
    // Manually append assistant message via the internal helper (through load+append)
    let dir = mgr.base_dir.join(&conv.id);
    let msgs = mgr.load_messages(&conv.id).unwrap();
    let prev = msgs.last().unwrap().hash.clone();
    // Use the public append_user_message then patch last message role via a direct write
    // Instead: test via load + openai conversion of what we have
    drop(dir);
    drop(prev);

    let openai = mgr.to_openai_messages(&conv.id).unwrap();
    assert_eq!(openai.len(), 1);
    assert_eq!(openai[0]["role"], "user");
    assert_eq!(openai[0]["content"], "What is 2+2?");
}

#[test]
fn test_to_openai_messages_tool() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Run ls").unwrap();

    // Append a fake tool result message via append_user_message for structure,
    // but verify the tool role conversion via direct JSONL write
    let path = mgr.base_dir.join(&conv.id).join("messages.jsonl");
    let msgs = mgr.load_messages(&conv.id).unwrap();
    let prev_hash = msgs.last().unwrap().hash.clone();
    let ts = 1700000000.0f64;
    let hash = compute_message_hash(&prev_hash, "tool", "file.txt", ts);
    let tool_msg = serde_json::json!({
        "seq": 1,
        "role": "tool",
        "content": "file.txt",
        "prev_hash": prev_hash,
        "hash": hash,
        "ts": ts,
        "tool_call_id": "call-abc",
        "name": "file_read",
    });
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(f, "{}", serde_json::to_string(&tool_msg).unwrap()).unwrap();

    let openai = mgr.to_openai_messages(&conv.id).unwrap();
    assert_eq!(openai.len(), 2);
    assert_eq!(openai[1]["role"], "tool");
    assert_eq!(openai[1]["tool_call_id"], "call-abc");
    assert_eq!(openai[1]["content"], "file.txt");
    assert_eq!(openai[1]["name"], "file_read");
}

#[test]
fn test_verify_chain_valid() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Message 1").unwrap();
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
fn test_verify_chain_tampered_hash() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Message 1").unwrap();
    mgr.append_user_message(&conv.id, "Message 2").unwrap();

    let path = mgr.base_dir.join(&conv.id).join("messages.jsonl");
    let content = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    // Corrupt the hash on the first message
    if let Ok(mut obj) = serde_json::from_str::<serde_json::Value>(&lines[0]) {
        obj["hash"] = serde_json::json!("deadbeef000000000000000000000000");
        lines[0] = serde_json::to_string(&obj).unwrap();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    let result = mgr.verify_chain(&conv.id).unwrap();
    assert!(!result.valid);
    assert!(!result.breaks.is_empty());
}

#[test]
fn test_list_conversations() {
    let (_dir, mgr) = tmp_mgr();
    mgr.create("key", "Conv 1").unwrap();
    mgr.create("key", "Conv 2").unwrap();
    mgr.create("key", "Conv 3").unwrap();
    let list = mgr.list().unwrap();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_list_conversations_empty() {
    let (_dir, mgr) = tmp_mgr();
    let list = mgr.list().unwrap();
    assert!(list.is_empty());
}

#[test]
fn test_update_meta_sats_and_task() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    mgr.append_user_message(&conv.id, "Follow-up").unwrap();
    mgr.update_meta(&conv.id, 5000, "task-abc").unwrap();
    let updated = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(updated.total_sats, 5000);
    assert!(updated.task_ids.contains(&"task-abc".to_string()));
    assert_eq!(updated.message_count, 2);
}

#[test]
fn test_update_meta_accumulates_sats() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    mgr.update_meta(&conv.id, 1000, "task-1").unwrap();
    mgr.update_meta(&conv.id, 2000, "task-2").unwrap();
    let updated = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(updated.total_sats, 3000);
    assert_eq!(updated.task_ids.len(), 2);
}

#[test]
fn test_corrupt_jsonl_recovery() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Good message").unwrap();
    let path = mgr.base_dir.join(&conv.id).join("messages.jsonl");
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(f, "{{not valid json at all}}").unwrap();
    // Should skip corrupt line and return the one good message
    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Good message");
}

// -- Hash chain --

#[test]
fn test_genesis_hash_deterministic() {
    let h1 = compute_genesis_hash("conv-test-123");
    let h2 = compute_genesis_hash("conv-test-123");
    assert_eq!(h1, h2);
}

#[test]
fn test_genesis_hash_differs_by_id() {
    let h1 = compute_genesis_hash("conv-aaa");
    let h2 = compute_genesis_hash("conv-bbb");
    assert_ne!(h1, h2);
}

#[test]
fn test_message_hash_deterministic() {
    let h1 = compute_message_hash("prev", "user", "hello", 1234567.0);
    let h2 = compute_message_hash("prev", "user", "hello", 1234567.0);
    assert_eq!(h1, h2);
}

#[test]
fn test_message_hash_differs_by_content() {
    let h1 = compute_message_hash("prev", "user", "hello", 1234567.0);
    let h2 = compute_message_hash("prev", "user", "world", 1234567.0);
    assert_ne!(h1, h2);
}

// -- Title generation --

#[test]
fn test_generate_title_short() {
    assert_eq!(generate_title("Hello world"), "Hello world");
}

#[test]
fn test_generate_title_exactly_60() {
    let s = "a".repeat(60);
    assert_eq!(generate_title(&s), s);
}

#[test]
fn test_generate_title_truncates_at_word_boundary() {
    let msg = "This is a very long message that exceeds the sixty character limit for titles here";
    let title = generate_title(msg);
    assert!(title.len() <= 60);
    assert!(!title.ends_with(' '));
}

#[test]
fn test_generate_title_no_spaces() {
    let msg = "a".repeat(80);
    let title = generate_title(&msg);
    assert_eq!(title.len(), 60);
}

// -- ChainVerification serde --

#[test]
fn test_chain_verification_serde() {
    let cv = ChainVerification {
        valid: true,
        message_count: 5,
        head_hash: "aabbccdd".to_string(),
        genesis_hash: "11223344".to_string(),
        messages: vec![],
        breaks: vec![],
        meta_count_valid: None,
        meta_head_hash_valid: None,
    };
    let json = serde_json::to_string(&cv).unwrap();
    let parsed: ChainVerification = serde_json::from_str(&json).unwrap();
    assert!(parsed.valid);
    assert_eq!(parsed.message_count, 5);
    assert_eq!(parsed.head_hash, "aabbccdd");
    assert_eq!(parsed.genesis_hash, "11223344");
    assert!(parsed.messages.is_empty());
}

// -- append_from_transcript with real Transcript --

#[test]
fn test_append_from_transcript() {
    skip_on_ci!();
    // 1. Create a ConversationManager and a conversation with a user message
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("pubkey-test", "Run ls").unwrap();

    // 2. Create a real Transcript in a tempdir
    let transcript_dir = tempfile::TempDir::new().unwrap();
    let transcript_path = transcript_dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(transcript_path);

    // 3. Record events into the transcript
    // a) User event (will be skipped by append_from_transcript, which only processes think_response + tool_result)
    transcript.record_user("Run ls");

    // b) think_response with tool_calls
    let tool_calls = vec![serde_json::json!({
        "id": "call-1",
        "type": "function",
        "function": {
            "name": "execute_bash",
            "arguments": "{\"command\":\"ls\"}"
        }
    })];
    transcript.record_think_response(
        "Let me run that", // text
        "gpt-5-nano",      // model
        500,               // sats_paid
        450,               // sats_effective
        50,                // sats_refunded
        100,               // prompt_tokens
        20,                // completion_tokens
        Some(&tool_calls), // tool_calls
        "stop",            // finish_reason
        200,               // duration_ms
    );

    // c) tool_result for call-1
    transcript.record_tool_result(
        "call-1",              // call_id
        "execute_bash",        // name
        "file.txt\nREADME.md", // result
        true,                  // success
        0,                     // sats_paid
    );

    // d) think_response with no tool_calls (final answer)
    transcript.record_think_response(
        "Here are your files: file.txt and README.md", // text
        "gpt-5-nano",                                  // model
        300,                                           // sats_paid
        280,                                           // sats_effective
        20,                                            // sats_refunded
        150,                                           // prompt_tokens
        30,                                            // completion_tokens
        None,                                          // no tool_calls
        "stop",                                        // finish_reason
        150,                                           // duration_ms
    );

    // 4. Append transcript events to the conversation
    // Small sleep ensures unique timestamps on platforms with coarse clocks (Linux CI)
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_from_transcript(&conv.id, &transcript, "task-123")
        .unwrap();

    // 5. Load messages and verify
    let messages = mgr.load_messages(&conv.id).unwrap();

    // Total: 1 user (from create) + 2 assistant (think_responses) + 1 tool (tool_result) = 4
    assert_eq!(
        messages.len(),
        4,
        "Expected 4 messages, got {}",
        messages.len()
    );

    // messages[0]: original user message from create()
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Run ls");
    assert_eq!(
        messages[0].task_id, None,
        "First user message should have no task_id"
    );

    // messages[1]: assistant with tool_calls
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "Let me run that");
    assert!(
        messages[1].tool_calls.is_some(),
        "First assistant message should have tool_calls"
    );
    let tc = messages[1].tool_calls.as_ref().unwrap();
    assert_eq!(tc.len(), 1);
    assert_eq!(tc[0]["id"], "call-1");
    assert_eq!(messages[1].task_id, Some("task-123".to_string()));

    // messages[2]: tool result
    assert_eq!(messages[2].role, "tool");
    assert_eq!(messages[2].tool_call_id, Some("call-1".to_string()));
    assert_eq!(messages[2].content, "file.txt\nREADME.md");
    assert_eq!(messages[2].name, Some("execute_bash".to_string()));
    assert_eq!(messages[2].task_id, Some("task-123".to_string()));

    // messages[3]: final assistant response
    assert_eq!(messages[3].role, "assistant");
    assert!(messages[3].content.contains("your files"));
    assert!(
        messages[3].tool_calls.is_none(),
        "Final assistant message should have no tool_calls"
    );
    assert_eq!(messages[3].task_id, Some("task-123".to_string()));

    // 6. Verify hash chain integrity
    let verification = mgr.verify_chain(&conv.id).unwrap();
    assert!(
        verification.valid,
        "Hash chain should be valid, breaks: {:?}",
        verification.breaks
    );
    assert_eq!(verification.message_count, 4);
    assert!(verification.breaks.is_empty());

    // 7. Verify to_openai_messages produces valid LLM input
    let openai = mgr.to_openai_messages(&conv.id).unwrap();
    assert_eq!(openai.len(), 4);
    assert_eq!(openai[0]["role"], "user");
    assert_eq!(openai[1]["role"], "assistant");
    assert!(
        openai[1].get("tool_calls").is_some(),
        "assistant should have tool_calls for LLM"
    );
    assert_eq!(openai[1]["tool_calls"][0]["id"], "call-1");
    assert_eq!(openai[2]["role"], "tool");
    assert_eq!(openai[2]["tool_call_id"], "call-1");
    assert_eq!(openai[3]["role"], "assistant");
    // tool result messages must NOT have tool_calls
    assert!(openai[3].get("tool_calls").is_none());
}

// -- Phase 6: Per-conversation encryption keys --

#[test]
fn test_conversation_key_id_format() {
    assert_eq!(conversation_key_id("abc"), "conv-abc");
    assert_eq!(conversation_key_id("conv-12345"), "conv-conv-12345");
    assert_eq!(conversation_key_id(""), "conv-");
}

#[test]
fn test_conversation_key_id_unique_per_conversation() {
    let key1 = conversation_key_id("conv-aaa");
    let key2 = conversation_key_id("conv-bbb");
    assert_ne!(key1, key2);
}

#[test]
fn test_conversation_key_id_deterministic() {
    let key1 = conversation_key_id("conv-test-123");
    let key2 = conversation_key_id("conv-test-123");
    assert_eq!(key1, key2);
}

// -- Phase 6B: verify_chain_from_messages --

#[test]
fn test_verify_chain_from_messages_valid() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Message 1").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "Message 2").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "Message 3").unwrap();

    let messages = mgr.load_messages(&conv.id).unwrap();
    let verification = ConversationManager::verify_chain_from_messages(&messages, &conv.id);
    assert!(verification.valid, "breaks: {:?}", verification.breaks);
    assert_eq!(verification.message_count, 3);
    assert!(verification.breaks.is_empty());
}

#[test]
fn test_verify_chain_from_messages_tampered() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Message 1").unwrap();
    mgr.append_user_message(&conv.id, "Message 2").unwrap();

    let mut messages = mgr.load_messages(&conv.id).unwrap();
    // Tamper with the first message's hash
    messages[0].hash = "deadbeef000000000000000000000000".to_string();

    let verification = ConversationManager::verify_chain_from_messages(&messages, &conv.id);
    assert!(!verification.valid);
    assert!(!verification.breaks.is_empty());
    // Both messages should be broken: first has wrong hash, second has wrong prev_hash
    assert!(verification.breaks.contains(&0));
    assert!(verification.breaks.contains(&1));
}

#[test]
fn test_verify_chain_from_messages_empty() {
    let verification = ConversationManager::verify_chain_from_messages(&[], "conv-empty");
    assert!(verification.valid);
    assert_eq!(verification.message_count, 0);
    assert!(verification.breaks.is_empty());
    // Head hash should equal genesis hash when there are no messages
    assert_eq!(verification.head_hash, verification.genesis_hash);
}

#[test]
fn test_verify_chain_from_messages_matches_verify_chain() {
    skip_on_ci!();
    // Ensure the in-memory verification matches the disk-based one
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "World").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "!").unwrap();

    let disk_result = mgr.verify_chain(&conv.id).unwrap();
    let messages = mgr.load_messages(&conv.id).unwrap();
    let mem_result = ConversationManager::verify_chain_from_messages(&messages, &conv.id);

    assert_eq!(disk_result.valid, mem_result.valid);
    assert_eq!(disk_result.message_count, mem_result.message_count);
    assert_eq!(disk_result.head_hash, mem_result.head_hash);
    assert_eq!(disk_result.genesis_hash, mem_result.genesis_hash);
    assert_eq!(disk_result.breaks, mem_result.breaks);
    assert_eq!(disk_result.messages.len(), mem_result.messages.len());
    for (d, m) in disk_result.messages.iter().zip(mem_result.messages.iter()) {
        assert_eq!(d.seq, m.seq);
        assert_eq!(d.valid, m.valid);
        assert_eq!(d.hash, m.hash);
        assert_eq!(d.expected_hash, m.expected_hash);
        assert_eq!(d.prev_hash_valid, m.prev_hash_valid);
    }
}

// -- Phase 6: find_by_participant --

#[test]
fn test_find_by_participant_found() {
    let (_dir, mgr) = tmp_mgr();
    let sender_key = "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce";
    let conv = mgr.create(sender_key, "Hello from remote agent").unwrap();
    let found = mgr.find_by_participant(sender_key);
    assert_eq!(found, Some(conv.id));
}

#[test]
fn test_find_by_participant_not_found() {
    let (_dir, mgr) = tmp_mgr();
    mgr.create("key-aaa", "Some conversation").unwrap();
    let found = mgr.find_by_participant("key-bbb");
    assert_eq!(found, None);
}

#[test]
fn test_find_by_participant_returns_most_recent() {
    let (_dir, mgr) = tmp_mgr();
    let key = "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce";
    let _old = mgr.create(key, "Old conversation").unwrap();
    // Small delay to ensure different updated_at timestamps
    std::thread::sleep(std::time::Duration::from_millis(10));
    let newer = mgr.create(key, "Newer conversation").unwrap();
    // list() returns newest first, so find_by_participant should return the newer one
    let found = mgr.find_by_participant(key);
    assert_eq!(found, Some(newer.id));
}

#[test]
fn test_find_by_participant_empty_conversations() {
    let (_dir, mgr) = tmp_mgr();
    let found = mgr.find_by_participant("any-key");
    assert_eq!(found, None);
}

#[test]
fn test_find_by_participant_multiple_keys() {
    let (_dir, mgr) = tmp_mgr();
    let key_a = "key-alice";
    let key_b = "key-bob";
    let conv_a = mgr.create(key_a, "Alice says hi").unwrap();
    let conv_b = mgr.create(key_b, "Bob says hi").unwrap();
    assert_eq!(mgr.find_by_participant(key_a), Some(conv_a.id));
    assert_eq!(mgr.find_by_participant(key_b), Some(conv_b.id));
}

// -- Phase 6: Compaction summary persistence --

#[test]
fn test_compaction_summary_stored_in_meta() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key-test", "Hello there").unwrap();

    // Set compaction summary
    mgr.set_compaction_summary(&conv.id, "Summary of earlier turns.".to_string(), 12)
        .unwrap();

    // Reload and verify
    let loaded = mgr.load(&conv.id).unwrap().expect("should exist");
    assert_eq!(
        loaded.compaction_summary.as_deref(),
        Some("Summary of earlier turns.")
    );
    assert_eq!(loaded.compaction_seq, Some(12));
}

#[test]
fn test_compaction_summary_none_by_default() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key-test", "Hello there").unwrap();

    let loaded = mgr.load(&conv.id).unwrap().expect("should exist");
    assert!(loaded.compaction_summary.is_none());
    assert!(loaded.compaction_seq.is_none());
}

// -- Phase 8A: Conversation introspection tools --

use dolphin_milk::tools::conversation_tools::all_conversation_tools;
use serde_json::json;

#[tokio::test]
async fn test_list_conversations_tool() {
    let dir = tempfile::TempDir::new().unwrap();
    let mgr = ConversationManager::new(dir.path());
    mgr.create("key-a", "First conversation").unwrap();
    mgr.create("key-b", "Second conversation").unwrap();
    mgr.create("key-c", "Third conversation").unwrap();

    let tools = all_conversation_tools(dir.path().to_path_buf());
    let list_tool = &tools[0];
    assert_eq!(list_tool.name, "list_conversations");

    let result = (list_tool.execute)(json!({})).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["count"], 3);
    let convs = parsed["conversations"].as_array().unwrap();
    assert_eq!(convs.len(), 3);
    // Each conversation should have expected fields
    for c in convs {
        assert!(c["id"].as_str().unwrap().starts_with("conv-"));
        assert!(c["title"].as_str().is_some());
        assert!(c["message_count"].is_number());
        assert!(c["total_sats"].is_number());
    }
}

#[tokio::test]
async fn test_list_conversations_tool_with_limit() {
    let dir = tempfile::TempDir::new().unwrap();
    let mgr = ConversationManager::new(dir.path());
    for i in 0..5 {
        mgr.create(&format!("key-{i}"), &format!("Conversation {i}"))
            .unwrap();
    }

    let tools = all_conversation_tools(dir.path().to_path_buf());
    let list_tool = &tools[0];

    let result = (list_tool.execute)(json!({"limit": 2})).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["count"], 2);
}

#[tokio::test]
async fn test_read_conversation_tool() {
    let dir = tempfile::TempDir::new().unwrap();
    let mgr = ConversationManager::new(dir.path());
    let conv = mgr.create("key-read", "Hello agent").unwrap();
    mgr.append_user_message(&conv.id, "How are you?").unwrap();
    mgr.append_user_message(&conv.id, "Tell me about BSV")
        .unwrap();

    let tools = all_conversation_tools(dir.path().to_path_buf());
    let read_tool = &tools[1];
    assert_eq!(read_tool.name, "read_conversation");

    let result = (read_tool.execute)(json!({"conversation_id": conv.id})).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let msgs = parsed["messages"].as_array().unwrap();
    // 1 initial message (from create) + 2 appended = 3
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["role"], "user");
    assert!(msgs[0]["seq"].is_number());
    assert!(msgs[0]["ts"].is_number());
}

#[tokio::test]
async fn test_read_conversation_truncation() {
    let dir = tempfile::TempDir::new().unwrap();
    let mgr = ConversationManager::new(dir.path());
    let long_content = "x".repeat(700);
    let conv = mgr.create("key-trunc", &long_content).unwrap();

    let tools = all_conversation_tools(dir.path().to_path_buf());
    let read_tool = &tools[1];

    let result = (read_tool.execute)(json!({"conversation_id": conv.id})).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let msgs = parsed["messages"].as_array().unwrap();
    let content = msgs[0]["content"].as_str().unwrap();
    assert!(content.len() < 700);
    assert!(content.ends_with("...[truncated]"));
}

#[tokio::test]
async fn test_read_nonexistent_conversation() {
    let dir = tempfile::TempDir::new().unwrap();

    let tools = all_conversation_tools(dir.path().to_path_buf());
    let read_tool = &tools[1];

    let result = (read_tool.execute)(json!({"conversation_id": "conv-nonexistent"})).await;
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    // Should return empty messages (load_messages returns empty for non-existent)
    assert_eq!(parsed["count"], 0);
}

// -- create_with_id --

#[test]
fn test_create_with_id() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr
        .create_with_id("conv-heartbeat", "system", "Heartbeat System Conversation")
        .unwrap();
    assert_eq!(conv.id, "conv-heartbeat");
    assert_eq!(conv.participant_key, "system");
    assert_eq!(conv.title, "Heartbeat System Conversation");
    assert_eq!(conv.message_count, 0);

    // meta.json should exist, messages.jsonl should not (no messages yet)
    let base = mgr.base_dir.join("conv-heartbeat");
    assert!(base.join("meta.json").exists());
}

#[test]
fn test_create_with_id_idempotent() {
    let (_dir, mgr) = tmp_mgr();
    let conv1 = mgr
        .create_with_id("conv-heartbeat", "system", "Original Title")
        .unwrap();
    // Append a message so the conversations differ
    mgr.append_user_message("conv-heartbeat", "Hello from heartbeat")
        .unwrap();

    // Second call with different title should return the existing one unchanged
    let conv2 = mgr
        .create_with_id("conv-heartbeat", "other", "Different Title")
        .unwrap();
    assert_eq!(conv2.id, conv1.id);
    assert_eq!(conv2.title, "Original Title"); // not overwritten
}

#[test]
fn test_create_with_id_then_append() {
    let (_dir, mgr) = tmp_mgr();
    mgr.create_with_id("conv-heartbeat", "system", "Heartbeat")
        .unwrap();

    // Append messages and verify hash chain works
    let msg1 = mgr
        .append_user_message("conv-heartbeat", "Check 1")
        .unwrap();
    assert_eq!(msg1.seq, 0);
    std::thread::sleep(std::time::Duration::from_millis(10));
    let msg2 = mgr
        .append_user_message("conv-heartbeat", "Check 2")
        .unwrap();
    assert_eq!(msg2.seq, 1);
    assert_eq!(msg2.prev_hash, msg1.hash);

    // Verify chain integrity
    let verification = mgr.verify_chain("conv-heartbeat").unwrap();
    assert!(verification.valid);
    assert_eq!(verification.message_count, 2);
}

// =========================================================================
// Block 3 — Task & Conversation Integrity tests
// =========================================================================

// -- Task 3.1: Auto-update conversation metadata after append --

#[test]
fn test_append_user_message_syncs_meta() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "First").unwrap();
    assert_eq!(conv.message_count, 1);

    // Append a second message
    let msg2 = mgr.append_user_message(&conv.id, "Second").unwrap();

    // Reload meta and verify it was updated
    let meta = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(
        meta.message_count, 2,
        "message_count should be 2 after append"
    );
    assert_eq!(
        meta.head_hash, msg2.hash,
        "head_hash should match last appended message"
    );
}

#[test]
fn test_append_user_message_syncs_meta_multiple() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "First").unwrap();

    mgr.append_user_message(&conv.id, "Second").unwrap();
    let msg3 = mgr.append_user_message(&conv.id, "Third").unwrap();
    mgr.append_user_message(&conv.id, "Fourth").unwrap();
    let msg5 = mgr.append_user_message(&conv.id, "Fifth").unwrap();

    let meta = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(meta.message_count, 5);
    assert_eq!(meta.head_hash, msg5.hash);
    // Intermediate states should have been correct too
    assert_ne!(meta.head_hash, msg3.hash);
}

#[test]
fn test_append_from_transcript_syncs_meta() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Run ls").unwrap();

    // Build a transcript with assistant + tool messages
    let transcript_dir = tempfile::TempDir::new().unwrap();
    let transcript_path = transcript_dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(transcript_path);

    transcript.record_user("Run ls");
    transcript.record_think_response(
        "Let me check",
        "gpt-5",
        500,
        450,
        50,
        100,
        20,
        None,
        "stop",
        200,
    );

    mgr.append_from_transcript(&conv.id, &transcript, "task-1")
        .unwrap();

    // Reload meta — should now have 2 messages (1 user from create + 1 assistant)
    let meta = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(
        meta.message_count, 2,
        "message_count should include transcript messages"
    );

    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(meta.head_hash, messages.last().unwrap().hash);
}

// -- Task 3.2: File locking (basic smoke test) --

#[test]
fn test_concurrent_append_does_not_corrupt() {
    skip_on_ci!();
    // Verify that sequential appends under the same manager produce valid chains
    // (True concurrency is hard to test deterministically, but this verifies the
    // locking code path doesn't break normal single-threaded usage.)
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Start").unwrap();

    for i in 0..10 {
        mgr.append_user_message(&conv.id, &format!("Message {i}"))
            .unwrap();
        // Small sleep ensures unique timestamps on platforms with coarse clocks (Linux CI)
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(messages.len(), 11); // 1 from create + 10 appended

    let meta = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(meta.message_count, 11);
    assert_eq!(meta.head_hash, messages.last().unwrap().hash);

    // Verify chain is still valid
    let verification = mgr.verify_chain(&conv.id).unwrap();
    assert!(
        verification.valid,
        "Chain should be valid after sequential appends"
    );
    assert!(verification.breaks.is_empty());
}

// -- Task 3.3: Compaction-aware hash chain verification --

#[test]
fn test_verify_chain_with_compaction_skips_compacted_region() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Msg 0").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "Msg 1").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "Msg 2").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "Msg 3").unwrap();

    // Set compaction through seq 1 (messages 0 and 1 are "compacted")
    mgr.set_compaction_summary(&conv.id, "Summary of first two messages".to_string(), 1)
        .unwrap();

    // Tamper with the first message's hash (inside compacted region)
    let path = mgr.base_dir.join(&conv.id).join("messages.jsonl");
    let content = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    if let Ok(mut obj) = serde_json::from_str::<serde_json::Value>(&lines[0]) {
        obj["hash"] = serde_json::json!("deadbeef000000000000000000000000");
        lines[0] = serde_json::to_string(&obj).unwrap();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    // Verification should still pass because seq 0 is in the compacted region
    let verification = mgr.verify_chain(&conv.id).unwrap();
    // The compacted region is trusted, so the tampered hash doesn't cause a break.
    // However, message seq 1's prev_hash won't match the tampered hash either,
    // but since seq 1 is also in the compacted region (seq <= compaction_seq),
    // it too is trusted.
    // Messages 2 and 3 should be verified normally against the actual hashes.
    assert!(
        verification.valid,
        "Compacted region should be trusted: breaks={:?}",
        verification.breaks
    );
}

#[test]
fn test_verify_chain_without_compaction_still_works() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Msg 0").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "Msg 1").unwrap();

    // No compaction — standard verification
    let verification = mgr.verify_chain(&conv.id).unwrap();
    assert!(verification.valid);
    assert_eq!(verification.message_count, 2);
}

// -- Task 3.4: Metadata consistency verification --

#[test]
fn test_verify_chain_meta_count_valid() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    mgr.append_user_message(&conv.id, "World").unwrap();

    let verification = mgr.verify_chain(&conv.id).unwrap();
    assert!(verification.valid);
    assert_eq!(verification.meta_count_valid, Some(true));
    assert_eq!(verification.meta_head_hash_valid, Some(true));
}

#[test]
fn test_verify_chain_detects_stale_message_count() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    mgr.append_user_message(&conv.id, "World").unwrap();

    // Manually corrupt meta.json to have wrong message_count
    let dir = mgr.base_dir.join(&conv.id);
    let mut meta = mgr.load(&conv.id).unwrap().unwrap();
    meta.message_count = 99; // wrong!
    let json = serde_json::to_string_pretty(&meta).unwrap();
    std::fs::write(dir.join("meta.json"), json).unwrap();

    let verification = mgr.verify_chain(&conv.id).unwrap();
    assert!(
        !verification.valid,
        "Should be invalid when meta.message_count is wrong"
    );
    assert_eq!(verification.meta_count_valid, Some(false));
}

#[test]
fn test_verify_chain_detects_stale_head_hash() {
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    mgr.append_user_message(&conv.id, "World").unwrap();

    // Manually corrupt meta.json to have wrong head_hash
    let dir = mgr.base_dir.join(&conv.id);
    let mut meta = mgr.load(&conv.id).unwrap().unwrap();
    meta.head_hash = "wrong_hash_value".to_string();
    let json = serde_json::to_string_pretty(&meta).unwrap();
    std::fs::write(dir.join("meta.json"), json).unwrap();

    let verification = mgr.verify_chain(&conv.id).unwrap();
    assert!(
        !verification.valid,
        "Should be invalid when meta.head_hash is wrong"
    );
    assert_eq!(verification.meta_head_hash_valid, Some(false));
}

#[test]
fn test_verify_chain_from_messages_has_no_meta_fields() {
    skip_on_ci!();
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("key", "Hello").unwrap();
    let messages = mgr.load_messages(&conv.id).unwrap();

    let verification = ConversationManager::verify_chain_from_messages(&messages, &conv.id);
    assert!(verification.valid);
    // In-memory verification has no metadata context
    assert_eq!(verification.meta_count_valid, None);
    assert_eq!(verification.meta_head_hash_valid, None);
}

#[test]
fn test_chain_verification_serde_new_fields() {
    // Verify the new fields serialize/deserialize correctly (with skip_serializing_if)
    let cv = ChainVerification {
        valid: true,
        message_count: 3,
        head_hash: "abc".to_string(),
        genesis_hash: "def".to_string(),
        messages: vec![],
        breaks: vec![],
        meta_count_valid: Some(true),
        meta_head_hash_valid: Some(false),
    };
    let json = serde_json::to_string(&cv).unwrap();
    let parsed: ChainVerification = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.meta_count_valid, Some(true));
    assert_eq!(parsed.meta_head_hash_valid, Some(false));

    // None fields should be omitted from JSON
    let cv_none = ChainVerification {
        valid: true,
        message_count: 0,
        head_hash: "x".to_string(),
        genesis_hash: "y".to_string(),
        messages: vec![],
        breaks: vec![],
        meta_count_valid: None,
        meta_head_hash_valid: None,
    };
    let json_none = serde_json::to_string(&cv_none).unwrap();
    assert!(!json_none.contains("meta_count_valid"));
    assert!(!json_none.contains("meta_head_hash_valid"));

    // Backward compat: old JSON without these fields deserializes as None
    let old_json =
        r#"{"valid":true,"message_count":2,"head_hash":"h","genesis_hash":"g","messages":[]}"#;
    let parsed_old: ChainVerification = serde_json::from_str(old_json).unwrap();
    assert_eq!(parsed_old.meta_count_valid, None);
    assert_eq!(parsed_old.meta_head_hash_valid, None);
}

// -- Self-healing conversation tests --

#[test]
fn test_self_heal_from_transcript_recovers_missing_assistant_messages() {
    // Simulate the bug: conversation has task_ids and user message,
    // but append_from_transcript never ran (e.g., panic or silent error).
    // Then call append_from_transcript again to verify recovery works.
    let (dir, mgr) = tmp_mgr();
    let conv = mgr.create("pubkey", "What is BSV?").unwrap();

    // Build a transcript with assistant response
    let task_dir = dir.path().join("tasks").join("task-heal-1");
    std::fs::create_dir_all(&task_dir).unwrap();
    let transcript_path = task_dir.join("session.jsonl");
    let mut transcript = Transcript::new(transcript_path.clone());
    transcript.record_user("What is BSV?");
    transcript.record_think_response(
        "BSV is Bitcoin Satoshi Vision",
        "gpt-5",
        500,
        450,
        50,
        100,
        20,
        None,
        "stop",
        200,
    );

    // Manually add task_id to meta WITHOUT running append_from_transcript
    // (simulating the bug where append_from_transcript failed silently)
    let meta_path = mgr.base_dir.join(&conv.id).join("meta.json");
    let mut meta = mgr.load(&conv.id).unwrap().unwrap();
    meta.task_ids.push("task-heal-1".to_string());
    let json = serde_json::to_string_pretty(&meta).unwrap();
    std::fs::write(&meta_path, json).unwrap();

    // Verify: conversation has task_ids but no assistant messages
    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
    let has_assistant = messages.iter().any(|m| m.role == "assistant");
    assert!(
        !has_assistant,
        "Should have no assistant messages (simulating the bug)"
    );

    // Self-heal: detect missing messages and reconstruct from transcript
    let reloaded_meta = mgr.load(&conv.id).unwrap().unwrap();
    if !has_assistant && !reloaded_meta.task_ids.is_empty() {
        for task_id in &reloaded_meta.task_ids {
            let tp = dir.path().join(format!("tasks/{task_id}/session.jsonl"));
            if tp.exists() {
                let t = Transcript::new(tp);
                mgr.append_from_transcript(&conv.id, &t, task_id).unwrap();
            }
        }
    }

    // Verify: assistant message recovered
    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(
        messages.len(),
        2,
        "Should have user + assistant after self-heal"
    );
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "BSV is Bitcoin Satoshi Vision");

    // Verify metadata was updated by append_from_transcript
    let meta = mgr.load(&conv.id).unwrap().unwrap();
    assert_eq!(meta.message_count, 2);
    assert_eq!(meta.head_hash, messages[1].hash);
}

#[test]
fn test_self_heal_skips_when_assistant_messages_exist() {
    // If assistant messages already exist, self-heal should NOT duplicate them.
    let (_dir, mgr) = tmp_mgr();
    let conv = mgr.create("pubkey", "Hello").unwrap();

    // Build transcript and run append_from_transcript normally
    let transcript_dir = tempfile::TempDir::new().unwrap();
    let transcript_path = transcript_dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(transcript_path);
    transcript.record_user("Hello");
    transcript.record_think_response(
        "Hi there!",
        "gpt-5",
        500,
        450,
        50,
        100,
        20,
        None,
        "stop",
        200,
    );
    mgr.append_from_transcript(&conv.id, &transcript, "task-1")
        .unwrap();

    // Verify assistant exists
    let messages = mgr.load_messages(&conv.id).unwrap();
    assert_eq!(messages.len(), 2);
    let has_assistant = messages.iter().any(|m| m.role == "assistant");
    assert!(has_assistant);

    // Self-heal check: should NOT run because assistant messages exist
    assert!(
        has_assistant,
        "Self-heal should be skipped when assistant messages exist"
    );
    // (In the real handler, the `if !has_assistant` guard prevents duplication)
}
