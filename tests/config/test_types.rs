//! Tests for newtype ID wrappers in `src/types.rs`.

use dolphin_milk::types::{ConversationId, ProofTxid, SessionId, TaskId};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Construction & accessors
// ---------------------------------------------------------------------------

#[test]
fn new_from_str_literal() {
    let id = TaskId::new("task-001");
    assert_eq!(id.as_str(), "task-001");
}

#[test]
fn new_from_owned_string() {
    let s = String::from("sess-abc");
    let id = SessionId::new(s);
    assert_eq!(id.as_str(), "sess-abc");
}

#[test]
fn from_string_construction() {
    let id = ConversationId::from_string("conv-xyz".to_string());
    assert_eq!(id.as_str(), "conv-xyz");
}

#[test]
fn into_inner_returns_owned_string() {
    let id = ProofTxid::new("deadbeef");
    let inner: String = id.into_inner();
    assert_eq!(inner, "deadbeef");
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

#[test]
fn display_matches_inner_string() {
    let id = TaskId::new("task-display");
    assert_eq!(format!("{id}"), "task-display");
}

#[test]
fn display_in_format_string() {
    let id = SessionId::new("s-1");
    let msg = format!("session={id}");
    assert_eq!(msg, "session=s-1");
}

// ---------------------------------------------------------------------------
// From conversions
// ---------------------------------------------------------------------------

#[test]
fn from_string_trait() {
    let id: TaskId = String::from("from-string").into();
    assert_eq!(id.as_str(), "from-string");
}

#[test]
fn from_str_trait() {
    let id: SessionId = "from-str".into();
    assert_eq!(id.as_str(), "from-str");
}

// ---------------------------------------------------------------------------
// AsRef<str>
// ---------------------------------------------------------------------------

#[test]
fn as_ref_str() {
    let id = ConversationId::new("conv-ref");
    let s: &str = id.as_ref();
    assert_eq!(s, "conv-ref");
}

// ---------------------------------------------------------------------------
// Deref to &str
// ---------------------------------------------------------------------------

#[test]
fn deref_allows_str_methods() {
    let id = TaskId::new("TASK-UPPER");
    // .to_lowercase() comes from &str via Deref
    assert_eq!(id.to_lowercase(), "task-upper");
}

#[test]
fn deref_starts_with() {
    let id = ProofTxid::new("abcdef1234");
    assert!(id.starts_with("abcdef"));
}

#[test]
fn deref_len() {
    let id = SessionId::new("12345");
    assert_eq!(id.len(), 5);
}

// ---------------------------------------------------------------------------
// Serialize / Deserialize (transparent — plain JSON string)
// ---------------------------------------------------------------------------

#[test]
fn serialize_as_plain_string() {
    let id = TaskId::new("task-ser");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""task-ser""#);
}

#[test]
fn deserialize_from_plain_string() {
    let id: SessionId = serde_json::from_str(r#""sess-de""#).unwrap();
    assert_eq!(id.as_str(), "sess-de");
}

#[test]
fn roundtrip_serialization() {
    let original = ConversationId::new("conv-roundtrip");
    let json = serde_json::to_string(&original).unwrap();
    let restored: ConversationId = serde_json::from_str(&json).unwrap();
    assert_eq!(original, restored);
}

#[test]
fn serialize_in_struct() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        task_id: TaskId,
        proof: ProofTxid,
    }

    let w = Wrapper {
        task_id: TaskId::new("t1"),
        proof: ProofTxid::new("0xdead"),
    };

    let json = serde_json::to_value(&w).unwrap();
    assert_eq!(json["task_id"], "t1");
    assert_eq!(json["proof"], "0xdead");

    let restored: Wrapper = serde_json::from_value(json).unwrap();
    assert_eq!(restored, w);
}

// ---------------------------------------------------------------------------
// Hash & Eq
// ---------------------------------------------------------------------------

#[test]
fn equal_values_are_eq() {
    let a = TaskId::new("same");
    let b = TaskId::new("same");
    assert_eq!(a, b);
}

#[test]
fn different_values_are_ne() {
    let a = TaskId::new("one");
    let b = TaskId::new("two");
    assert_ne!(a, b);
}

#[test]
fn equal_values_same_hash() {
    let a = SessionId::new("hash-test");
    let b = SessionId::new("hash-test");

    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}

#[test]
fn hash_set_deduplicates() {
    let mut set = HashSet::new();
    set.insert(ProofTxid::new("abc"));
    set.insert(ProofTxid::new("abc"));
    set.insert(ProofTxid::new("def"));
    assert_eq!(set.len(), 2);
}

// ---------------------------------------------------------------------------
// Clone
// ---------------------------------------------------------------------------

#[test]
fn clone_is_independent() {
    let a = TaskId::new("original");
    let b = a.clone();
    assert_eq!(a, b);
    // Both are still usable after clone
    assert_eq!(a.as_str(), "original");
    assert_eq!(b.as_str(), "original");
}

// ---------------------------------------------------------------------------
// Type safety — different ID types are NOT interchangeable
// ---------------------------------------------------------------------------

/// This test verifies the *purpose* of the newtypes: compile-time type safety.
/// A function that takes `TaskId` will not accept `SessionId`, `ConversationId`,
/// or `ProofTxid` at compile time.  We verify this indirectly by showing that
/// each type has its own identity via `TypeId`.
#[test]
fn types_are_distinct() {
    use std::any::TypeId;

    let task = TypeId::of::<TaskId>();
    let session = TypeId::of::<SessionId>();
    let conversation = TypeId::of::<ConversationId>();
    let proof = TypeId::of::<ProofTxid>();

    // All four are distinct types
    assert_ne!(task, session);
    assert_ne!(task, conversation);
    assert_ne!(task, proof);
    assert_ne!(session, conversation);
    assert_ne!(session, proof);
    assert_ne!(conversation, proof);
}

/// Verify that a function parameterized on `TaskId` works with `TaskId` values.
/// The compile-time guarantee is that `SessionId` etc. would NOT work here.
#[test]
fn function_accepts_correct_type() {
    fn process_task(id: &TaskId) -> String {
        format!("processing {id}")
    }

    let tid = TaskId::new("t-42");
    assert_eq!(process_task(&tid), "processing t-42");

    // The following would NOT compile (uncomment to verify):
    // let sid = SessionId::new("s-42");
    // process_task(&sid);  // ERROR: expected `&TaskId`, found `&SessionId`
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_string_id() {
    let id = TaskId::new("");
    assert_eq!(id.as_str(), "");
    assert!(id.is_empty());
    assert_eq!(format!("{id}"), "");
}

#[test]
fn unicode_id() {
    let id = ConversationId::new("conv-\u{1F600}-test");
    assert_eq!(id.as_str(), "conv-\u{1F600}-test");
}

#[test]
fn whitespace_preserved() {
    let id = SessionId::new("  spaces  ");
    assert_eq!(id.as_str(), "  spaces  ");
}
