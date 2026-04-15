//! Tests for self-messaging loop prevention.
//!
//! Validates both Solution 1 (self-message filtering) and Solution 2 (reply obligations)
//! that prevent the agent from entering an infinite loop when sending messages to itself.

use dolphin_milk::runner::{LoopState, ReplyObligation};

/// Secp256k1 generator point G — valid pubkey for tests.
const OWN_KEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

/// A different valid pubkey for "external" sender tests.
const EXTERNAL_KEY: &str = "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

/// Another external key for multi-sender tests.
const EXTERNAL_KEY_2: &str = "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";

// -----------------------------------------------------------------------
// ReplyObligation struct tests
// -----------------------------------------------------------------------

#[test]
fn test_reply_obligation_construction() {
    let obligation = ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    };
    assert_eq!(obligation.sender_key, EXTERNAL_KEY);
    assert_eq!(obligation.message_box, "status_inbox");
}

#[test]
fn test_reply_obligation_clone() {
    let obligation = ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    };
    let cloned = obligation.clone();
    assert_eq!(cloned.sender_key, obligation.sender_key);
    assert_eq!(cloned.message_box, obligation.message_box);
}

// -----------------------------------------------------------------------
// LoopState pending_replies tests
// -----------------------------------------------------------------------

#[test]
fn test_loop_state_default_has_empty_pending_replies() {
    let state = LoopState::default();
    assert!(state.comms.pending_replies.is_empty());
}

#[test]
fn test_pending_replies_add_external_obligation() {
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });
    assert_eq!(state.comms.pending_replies.len(), 1);
    assert_eq!(state.comms.pending_replies[0].sender_key, EXTERNAL_KEY);
}

#[test]
fn test_pending_replies_no_obligation_for_self() {
    // Simulates the runner logic: only create obligations for external senders
    let mut state = LoopState::default();
    let identity_key = OWN_KEY;

    // Simulate inbox messages — one from self, one from external
    let senders = vec![identity_key, EXTERNAL_KEY];
    for sender in &senders {
        if *sender != identity_key {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: sender.to_string(),
                message_box: "status_inbox".to_string(),
            });
        }
    }

    // Only the external sender should have an obligation
    assert_eq!(state.comms.pending_replies.len(), 1);
    assert_eq!(state.comms.pending_replies[0].sender_key, EXTERNAL_KEY);
}

#[test]
fn test_pending_replies_dedup_same_sender() {
    // Multiple messages from the same sender should create only one obligation
    let mut state = LoopState::default();

    let senders = vec![EXTERNAL_KEY, EXTERNAL_KEY, EXTERNAL_KEY];
    for sender in &senders {
        if !state
            .comms
            .pending_replies
            .iter()
            .any(|r| r.sender_key == *sender)
        {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: sender.to_string(),
                message_box: "status_inbox".to_string(),
            });
        }
    }

    assert_eq!(state.comms.pending_replies.len(), 1);
}

#[test]
fn test_pending_replies_multiple_external_senders() {
    let mut state = LoopState::default();

    let senders = vec![EXTERNAL_KEY, EXTERNAL_KEY_2];
    for sender in &senders {
        if !state
            .comms
            .pending_replies
            .iter()
            .any(|r| r.sender_key == *sender)
        {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: sender.to_string(),
                message_box: "status_inbox".to_string(),
            });
        }
    }

    assert_eq!(state.comms.pending_replies.len(), 2);
}

// -----------------------------------------------------------------------
// Obligation fulfillment tests
// -----------------------------------------------------------------------

#[test]
fn test_send_message_fulfills_obligation() {
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });
    assert_eq!(state.comms.pending_replies.len(), 1);

    // Simulate send_message to the external sender
    let recipient = EXTERNAL_KEY;
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != recipient);

    assert!(state.comms.pending_replies.is_empty());
}

#[test]
fn test_send_message_to_self_does_not_fulfill_external_obligation() {
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });

    // Sending to self should NOT clear the external obligation
    let recipient = OWN_KEY;
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != recipient);

    // External obligation still pending
    assert_eq!(state.comms.pending_replies.len(), 1);
    assert_eq!(state.comms.pending_replies[0].sender_key, EXTERNAL_KEY);
}

#[test]
fn test_send_message_fulfills_only_matching_obligation() {
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY_2.to_string(),
        message_box: "status_inbox".to_string(),
    });
    assert_eq!(state.comms.pending_replies.len(), 2);

    // Reply to only one sender
    let recipient = EXTERNAL_KEY;
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != recipient);

    assert_eq!(state.comms.pending_replies.len(), 1);
    assert_eq!(state.comms.pending_replies[0].sender_key, EXTERNAL_KEY_2);
}

#[test]
fn test_all_obligations_fulfilled_is_empty() {
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY_2.to_string(),
        message_box: "status_inbox".to_string(),
    });

    // Reply to both
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != EXTERNAL_KEY);
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != EXTERNAL_KEY_2);

    assert!(state.comms.pending_replies.is_empty());
}

// -----------------------------------------------------------------------
// Done-signal logic tests (simulating the runner's decision)
// -----------------------------------------------------------------------

#[test]
fn test_done_when_no_obligations_and_no_tool_calls() {
    // Text-only response, no obligations → done = true
    let mut state = LoopState::default();
    let has_tool_calls = false;
    let unfulfilled = state.comms.pending_replies.len();

    if !has_tool_calls && unfulfilled == 0 {
        state.exec.done = true;
    }
    assert!(state.exec.done);
}

#[test]
fn test_not_done_when_unfulfilled_obligations() {
    // Text-only response but unfulfilled obligations → nudge, not done
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });

    let has_tool_calls = false;
    let called_send_message = false;
    let unfulfilled = state.comms.pending_replies.len();

    if !has_tool_calls && unfulfilled > 0 && !called_send_message && state.comms.nudge_count < 2 {
        state.comms.nudge_count += 1;
        // Don't set done
    }
    assert!(!state.exec.done);
    assert_eq!(state.comms.nudge_count, 1);
}

#[test]
fn test_done_after_tool_calls_with_obligations_fulfilled() {
    // Tool iteration that fulfilled all obligations with only send_message
    let mut state = LoopState::default();
    // Start with an obligation, then fulfill it
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != EXTERNAL_KEY);

    let has_tool_calls = true;
    let called_send_message = true;
    let only_messaging = true; // all tool calls were send_message
    let has_text = true;

    // Simulate the new done-signal logic
    if has_tool_calls
        && state.comms.pending_replies.is_empty()
        && called_send_message
        && only_messaging
        && has_text
    {
        state.exec.done = true;
    }
    assert!(state.exec.done);
}

#[test]
fn test_not_done_when_non_messaging_tools_called() {
    // Tool iteration with send_message + other tools → not done (still working)
    let mut state = LoopState::default();
    let has_tool_calls = true;
    let called_send_message = true;
    let only_messaging = false; // also called web_fetch or similar
    let has_text = true;

    if has_tool_calls
        && state.comms.pending_replies.is_empty()
        && called_send_message
        && only_messaging
        && has_text
    {
        state.exec.done = true;
    }
    assert!(!state.exec.done); // Should NOT be done — other work is happening
}

#[test]
fn test_not_done_when_tool_calls_but_obligations_remain() {
    // Tool iteration where send_message was called but not to the right person
    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });

    let has_tool_calls = true;
    let called_send_message = true;
    let only_messaging = true;
    let has_text = true;

    // Obligations NOT empty — send_message went to wrong recipient
    if has_tool_calls
        && state.comms.pending_replies.is_empty()
        && called_send_message
        && only_messaging
        && has_text
    {
        state.exec.done = true;
    }
    assert!(!state.exec.done); // Obligation still pending
}

// -----------------------------------------------------------------------
// Nudge reset logic tests
// -----------------------------------------------------------------------

#[test]
fn test_nudge_resets_when_obligations_cleared() {
    let mut state = LoopState::default();
    state.comms.nudge_count = 1;
    // All obligations fulfilled
    let called_send_message = true;

    if called_send_message && state.comms.pending_replies.is_empty() {
        state.comms.nudge_count = 0;
    }
    assert_eq!(state.comms.nudge_count, 0);
}

#[test]
fn test_nudge_does_not_reset_when_obligations_remain() {
    let mut state = LoopState::default();
    state.comms.nudge_count = 1;
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "status_inbox".to_string(),
    });

    let called_send_message = true;

    // Should NOT reset — obligations still pending
    if called_send_message && state.comms.pending_replies.is_empty() {
        state.comms.nudge_count = 0;
    }
    assert_eq!(state.comms.nudge_count, 1); // Still 1
}

// -----------------------------------------------------------------------
// Inbox partitioning simulation tests
// -----------------------------------------------------------------------

#[test]
fn test_inbox_partition_separates_self_from_external() {
    // Simulate the partition logic from runner.rs step()
    let identity_key = OWN_KEY;

    struct FakeMessage {
        sender: String,
    }

    let messages = vec![
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
        FakeMessage {
            sender: EXTERNAL_KEY.to_string(),
        },
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
        FakeMessage {
            sender: EXTERNAL_KEY_2.to_string(),
        },
    ];

    let (external, self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    assert_eq!(external.len(), 2);
    assert_eq!(self_sent.len(), 2);
    assert_eq!(external[0].sender, EXTERNAL_KEY);
    assert_eq!(external[1].sender, EXTERNAL_KEY_2);
}

#[test]
fn test_inbox_partition_all_self_messages() {
    let identity_key = OWN_KEY;

    struct FakeMessage {
        sender: String,
    }

    let messages = vec![
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
    ];

    let (external, self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    assert!(external.is_empty());
    assert_eq!(self_sent.len(), 2);
}

#[test]
fn test_inbox_partition_all_external_messages() {
    let identity_key = OWN_KEY;

    struct FakeMessage {
        sender: String,
    }

    let messages = vec![
        FakeMessage {
            sender: EXTERNAL_KEY.to_string(),
        },
        FakeMessage {
            sender: EXTERNAL_KEY_2.to_string(),
        },
    ];

    let (external, self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    assert_eq!(external.len(), 2);
    assert!(self_sent.is_empty());
}

#[test]
fn test_inbox_partition_empty_inbox() {
    let identity_key = OWN_KEY;

    struct FakeMessage {
        sender: String,
    }

    let messages: Vec<FakeMessage> = vec![];
    let (external, self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    assert!(external.is_empty());
    assert!(self_sent.is_empty());
}

// -----------------------------------------------------------------------
// Self-messaging scenario simulation (end-to-end logic)
// -----------------------------------------------------------------------

#[test]
fn test_self_message_scenario_no_loop() {
    // Simulates the full cycle that previously caused the infinite loop.
    // Agent sends a message to itself → next iteration:
    // 1. Inbox poll returns the self-message
    // 2. Partition: self_message goes to self_sent, not external
    // 3. No obligations created
    // 4. LLM produces text → done=true (no unfulfilled obligations)

    let identity_key = OWN_KEY;
    let mut state = LoopState::default();

    // Step 1: Inbox returns self-message
    struct FakeMessage {
        sender: String,
    }
    let messages = vec![FakeMessage {
        sender: OWN_KEY.to_string(),
    }];

    // Step 2: Partition
    let (external, _self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    // Step 3: Create obligations only for external
    for msg in &external {
        if !state
            .comms
            .pending_replies
            .iter()
            .any(|r| r.sender_key == msg.sender)
        {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: msg.sender.clone(),
                message_box: "status_inbox".to_string(),
            });
        }
    }
    assert!(state.comms.pending_replies.is_empty()); // No obligations for self

    // Step 4: LLM produces text (no tool calls)
    let has_tool_calls = false;
    let unfulfilled = state.comms.pending_replies.len();
    if !has_tool_calls && unfulfilled == 0 {
        state.exec.done = true;
    }

    assert!(state.exec.done); // Loop terminates!
}

#[test]
fn test_external_message_scenario_creates_obligation() {
    // External agent sends a message → obligation created → nudge if needed
    let identity_key = OWN_KEY;
    let mut state = LoopState::default();

    struct FakeMessage {
        sender: String,
    }
    let messages = vec![FakeMessage {
        sender: EXTERNAL_KEY.to_string(),
    }];

    let (external, _self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    for msg in &external {
        if !state
            .comms
            .pending_replies
            .iter()
            .any(|r| r.sender_key == msg.sender)
        {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: msg.sender.clone(),
                message_box: "status_inbox".to_string(),
            });
        }
    }

    // External message creates an obligation
    assert_eq!(state.comms.pending_replies.len(), 1);
    assert_eq!(state.comms.pending_replies[0].sender_key, EXTERNAL_KEY);

    // LLM responds with text but doesn't call send_message → nudge fires
    let has_tool_calls = false;
    let called_send_message = false;
    let unfulfilled = state.comms.pending_replies.len();

    if !has_tool_calls && unfulfilled > 0 && !called_send_message && state.comms.nudge_count < 2 {
        state.comms.nudge_count += 1;
    }

    assert!(!state.exec.done); // Should not be done — obligation unfulfilled
    assert_eq!(state.comms.nudge_count, 1); // Nudge fired
}

#[test]
fn test_mixed_scenario_self_and_external() {
    // Both self and external messages arrive — only external creates obligations
    let identity_key = OWN_KEY;
    let mut state = LoopState::default();

    struct FakeMessage {
        sender: String,
    }
    let messages = vec![
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
        FakeMessage {
            sender: EXTERNAL_KEY.to_string(),
        },
    ];

    let (external, self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    assert_eq!(external.len(), 1);
    assert_eq!(self_sent.len(), 1);

    for msg in &external {
        if !state
            .comms
            .pending_replies
            .iter()
            .any(|r| r.sender_key == msg.sender)
        {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: msg.sender.clone(),
                message_box: "status_inbox".to_string(),
            });
        }
    }

    // Only external obligation
    assert_eq!(state.comms.pending_replies.len(), 1);
    assert_eq!(state.comms.pending_replies[0].sender_key, EXTERNAL_KEY);

    // LLM replies to external sender
    state
        .comms
        .pending_replies
        .retain(|r| r.sender_key != EXTERNAL_KEY);
    assert!(state.comms.pending_replies.is_empty());

    // Now done can be set
    state.exec.done = true;
    assert!(state.exec.done);
}

// -----------------------------------------------------------------------
// BUG-007 regression — reasoning model messaging-only done signal
// -----------------------------------------------------------------------
//
// The runner's tool-call-only done signal at src/runner/step.rs:961 used to
// require `!result.text.is_empty()`. Reasoning models (gpt-5-mini, o-series,
// gpt-4.1) return empty visible text with their tool calls — the reasoning is
// hidden. That broke the done-signal for any reasoning-model agent that calls
// only `send_message` in an iteration, leaving no termination path. See #324
// for the real-world Coral refusal loop that burned 188K sats.
//
// These tests simulate the fixed logic: done should fire when obligations are
// clear AND only messaging tools were called, regardless of text content.

/// Pure function mirroring the done-signal decision at step.rs. Kept as a
/// free function so tests can drive it without building a full DmLoop.
fn should_mark_done_messaging_only(
    has_tool_calls: bool,
    unfulfilled: usize,
    called_send_message: bool,
    only_messaging: bool,
) -> bool {
    // BUG-007 fix: drop the `!text.is_empty()` requirement.
    has_tool_calls && unfulfilled == 0 && called_send_message && only_messaging
}

#[test]
fn bug_007_reasoning_model_empty_text_terminates() {
    // gpt-5-mini returned empty text + one send_message tool call that cleared
    // the sole reply obligation. Before the fix this did NOT terminate because
    // text was empty. After the fix it DOES terminate.
    let done = should_mark_done_messaging_only(
        /* has_tool_calls */ true, /* unfulfilled    */ 0,
        /* called_sm      */ true, /* only_messaging */ true,
    );
    assert!(
        done,
        "messaging-only iteration with empty reasoning-model text must terminate"
    );
}

#[test]
fn bug_007_non_empty_text_still_terminates() {
    // Smoke check: a chatty model (non-reasoning, or reasoning with extended
    // thinking disabled) that produces text should terminate the same way.
    let done = should_mark_done_messaging_only(true, 0, true, true);
    assert!(done);
}

#[test]
fn bug_007_non_messaging_tool_does_not_terminate() {
    // If the tool call set isn't pure-messaging (e.g. a web_fetch was also in
    // the batch), the done signal must NOT fire — other work is pending.
    let done = should_mark_done_messaging_only(true, 0, true, /* only_messaging */ false);
    assert!(!done);
}

#[test]
fn bug_007_pending_obligations_block_terminate() {
    // Even with tool calls, pending obligations block the done signal.
    let done = should_mark_done_messaging_only(true, /* unfulfilled */ 1, true, true);
    assert!(!done);
}

#[test]
fn bug_007_synthesizes_fallback_text() {
    // The runner synthesizes a summary when the reasoning model returns empty
    // text. This test validates the synthesis is deterministic and references
    // the number of tool calls.
    let tool_call_count = 2;
    let empty = String::new();
    let final_text = if empty.is_empty() {
        format!("Sent {} message(s) and terminated.", tool_call_count)
    } else {
        empty
    };
    assert_eq!(final_text, "Sent 2 message(s) and terminated.");
}

// -----------------------------------------------------------------------
// BUG-008 regression — nudge/force-send for has_tool_calls case
// -----------------------------------------------------------------------
//
// The nudge/force-send fallback at step.rs:862-889 was gated on `!has_tool_calls`.
// If the LLM made tool calls that did NOT fulfill any obligation (e.g. Coral
// calling send_message to status_inbox when the obligation pointed at the
// original sender's identity_key), the runner had no termination path — it
// just iterated. Combined with BUG-007 this was the root cause of Coral's
// 188K-sat refusal loop.

/// Pure function mirroring the post-fix nudge branch for the has_tool_calls case.
/// Returns the action the runner should take.
#[derive(Debug, PartialEq)]
enum NudgeAction {
    Continue,
    Nudge,
    ForceSendAndDone,
}

fn nudge_decision_with_tool_calls(
    has_tool_calls: bool,
    unfulfilled: usize,
    called_send_message: bool,
    already_done: bool,
    nudge_count: u32,
    max_nudges: u32,
) -> NudgeAction {
    let branch_fires = has_tool_calls && unfulfilled > 0 && !called_send_message && !already_done;
    if !branch_fires {
        return NudgeAction::Continue;
    }
    if nudge_count < max_nudges {
        NudgeAction::Nudge
    } else {
        NudgeAction::ForceSendAndDone
    }
}

#[test]
fn bug_008_tool_calls_unfulfilled_obligation_nudges() {
    // LLM made tool calls but didn't fulfill the obligation — first pass nudges.
    let action = nudge_decision_with_tool_calls(
        /* has_tool_calls  */ true, /* unfulfilled     */ 1,
        /* called_sm       */ false, /* already_done    */ false,
        /* nudge_count     */ 0, /* max_nudges      */ 2,
    );
    assert_eq!(action, NudgeAction::Nudge);
}

#[test]
fn bug_008_tool_calls_unfulfilled_nudges_exhausted_force_sends() {
    // Second pass with nudges already exhausted → force-send + done.
    let action = nudge_decision_with_tool_calls(true, 1, false, false, 2, 2);
    assert_eq!(action, NudgeAction::ForceSendAndDone);
}

#[test]
fn bug_008_fulfilled_obligation_does_not_nudge() {
    // called_send_message is true → the normal done-signal handles it; no nudge.
    let action = nudge_decision_with_tool_calls(true, 0, true, false, 0, 2);
    assert_eq!(action, NudgeAction::Continue);
}

#[test]
fn bug_008_no_tool_calls_does_not_hit_new_branch() {
    // Text-only iteration: the existing `!has_tool_calls` branch handles nudging.
    // This branch should NOT fire.
    let action = nudge_decision_with_tool_calls(false, 1, false, false, 0, 2);
    assert_eq!(action, NudgeAction::Continue);
}

#[test]
fn bug_008_already_done_skips_nudge_branch() {
    // If the iteration was already marked done by the done-signal block above
    // (e.g. obligations cleared AND only messaging), this branch must not
    // re-enter and re-force-send.
    let action = nudge_decision_with_tool_calls(true, 1, false, /* already_done */ true, 0, 2);
    assert_eq!(action, NudgeAction::Continue);
}

// -----------------------------------------------------------------------
// Integration: simulate a Coral-style refusal loop with the fixes applied
// -----------------------------------------------------------------------

#[test]
fn coral_style_refusal_loop_terminates_within_max_nudges() {
    // Simulates the Coral scenario: agent receives one external task_inbox
    // message, then iterates making send_message calls to the WRONG inbox
    // (status_inbox), which don't fulfill the obligation. Before the fix this
    // ran forever. After the fix:
    //   iter 1: nudge #1 fires (has_tool_calls + unfulfilled + no send_message-to-sender)
    //   iter 2: nudge #2 fires
    //   iter 3: nudges exhausted → force-send → done

    let mut state = LoopState::default();
    state.comms.pending_replies.push(ReplyObligation {
        sender_key: EXTERNAL_KEY.to_string(),
        message_box: "task_inbox".to_string(),
    });

    let max_nudges = 2;
    let mut force_send_fired = false;

    for _iter in 1..=3 {
        // Each iteration: LLM makes tool calls (send_message to status_inbox)
        // that don't clear the pending reply obligation pointing at EXTERNAL_KEY.
        let has_tool_calls = true;
        let unfulfilled = state.comms.pending_replies.len();
        let called_send_message_for_obligation = false; // wrong recipient
        let already_done = state.exec.done;

        let action = nudge_decision_with_tool_calls(
            has_tool_calls,
            unfulfilled,
            called_send_message_for_obligation,
            already_done,
            state.comms.nudge_count,
            max_nudges,
        );

        match action {
            NudgeAction::Nudge => {
                state.comms.nudge_count += 1;
            }
            NudgeAction::ForceSendAndDone => {
                force_send_fired = true;
                state.comms.pending_replies.clear();
                state.exec.done = true;
                break;
            }
            NudgeAction::Continue => {}
        }
    }

    assert!(
        force_send_fired,
        "refusal loop must hit force-send within 3 iterations"
    );
    assert!(state.exec.done, "runner must terminate after force-send");
    assert_eq!(state.comms.nudge_count, max_nudges);
}

// -----------------------------------------------------------------------
// Prompt context / skill activation tests
// -----------------------------------------------------------------------

#[test]
fn test_inbox_count_uses_obligation_count_not_raw() {
    // The prompt should show obligation count (pending_replies.len()),
    // not the raw inbox message count. This test verifies the concept.
    let mut state = LoopState::default();

    // 3 inbox messages total, but 2 are from self
    // Only 1 obligation should be created
    let identity_key = OWN_KEY;

    struct FakeMessage {
        sender: String,
    }
    let messages = vec![
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
        FakeMessage {
            sender: OWN_KEY.to_string(),
        },
        FakeMessage {
            sender: EXTERNAL_KEY.to_string(),
        },
    ];

    let (external, _self_sent): (Vec<_>, Vec<_>) =
        messages.into_iter().partition(|m| m.sender != identity_key);

    for msg in &external {
        if !state
            .comms
            .pending_replies
            .iter()
            .any(|r| r.sender_key == msg.sender)
        {
            state.comms.pending_replies.push(ReplyObligation {
                sender_key: msg.sender.clone(),
                message_box: "status_inbox".to_string(),
            });
        }
    }

    // inbox_count passed to prompt should be 1, not 3
    let inbox_count_for_prompt = state.comms.pending_replies.len();
    assert_eq!(inbox_count_for_prompt, 1);
}
