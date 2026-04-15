//! Tests for CI.7: Tool intent nudging.
//!
//! Verifies that `signals_tool_intent()` detects when the LLM describes
//! tool actions without actually calling them, so a nudge can be injected.

use dolphin_milk::runner::{signals_tool_intent, MAX_INTENT_NUDGES, TOOL_INTENT_NUDGE};

// ===========================================================================
// Detection tests
// ===========================================================================

#[test]
fn test_detects_intent_phrase_without_tool_call() {
    // "let me search for that" should be detected as tool intent
    assert!(signals_tool_intent(
        "Let me search for that in the memory store."
    ));
    assert!(signals_tool_intent(
        "I'll search the documents to find what you need."
    ));
    assert!(signals_tool_intent(
        "Let me check the wallet balance first."
    ));
}

#[test]
fn test_no_nudge_on_normal_response() {
    // Plain text without intent phrases should NOT trigger
    assert!(!signals_tool_intent(
        "The payment protocol uses BRC-31 authentication."
    ));
    assert!(!signals_tool_intent(
        "Here is the information you requested."
    ));
    assert!(!signals_tool_intent("The wallet balance is 50000 sats."));
    assert!(!signals_tool_intent("I found three relevant results."));
}

#[test]
fn test_multiple_intent_patterns() {
    // Various intent patterns should all be detected
    assert!(signals_tool_intent(
        "I'll now run the command to check the status."
    ));
    assert!(signals_tool_intent(
        "Let me check the configuration for errors."
    ));
    assert!(signals_tool_intent(
        "I will search for matching entries in memory."
    ));
    assert!(signals_tool_intent("Let me look up the payment details."));
    assert!(signals_tool_intent("I'll fetch the latest exchange rates."));
    assert!(signals_tool_intent(
        "Let me find the relevant memory entries."
    ));
    assert!(signals_tool_intent(
        "I'm going to search for that information."
    ));
    assert!(signals_tool_intent("Let me run the diagnostic command."));
    assert!(signals_tool_intent("I will now execute the bash command."));
    assert!(signals_tool_intent("Let me look into the error logs."));
    assert!(signals_tool_intent("I'll look for matching documents."));
    assert!(signals_tool_intent(
        "Let me retrieve the conversation history."
    ));
    assert!(signals_tool_intent("I'm going to check the budget status."));
}

#[test]
fn test_exclusion_phrases_not_detected() {
    // Conversational phrases should NOT be detected as tool intent
    assert!(!signals_tool_intent(
        "Let me explain how the payment protocol works."
    ));
    assert!(!signals_tool_intent(
        "Let me know if you need more information."
    ));
    assert!(!signals_tool_intent(
        "Let me think about the best approach."
    ));
    assert!(!signals_tool_intent(
        "Let me summarize what we've discussed."
    ));
    assert!(!signals_tool_intent(
        "Let me clarify the authentication flow."
    ));
    assert!(!signals_tool_intent("Let me describe the architecture."));
    assert!(!signals_tool_intent(
        "Let me help you understand the codebase."
    ));
    assert!(!signals_tool_intent("Let me provide some context."));
    assert!(!signals_tool_intent(
        "Let me suggest an alternative approach."
    ));
    assert!(!signals_tool_intent("Let me elaborate on the design."));
    assert!(!signals_tool_intent("Let me walk you through the process."));
}

#[test]
fn test_case_insensitive_detection() {
    // Detection should be case-insensitive
    assert!(signals_tool_intent("LET ME SEARCH FOR THAT"));
    assert!(signals_tool_intent("let me search for that"));
    assert!(signals_tool_intent("Let Me Search For That"));
    assert!(signals_tool_intent("I'LL CHECK THE STATUS"));
    assert!(signals_tool_intent("i'll check the status"));
}

#[test]
fn test_code_blocks_excluded() {
    // Intent phrases inside code blocks should NOT trigger
    let with_code = "Here's an example:\n```\nLet me search for that\n```\nThat's how it works.";
    assert!(!signals_tool_intent(with_code));
}

#[test]
fn test_empty_and_whitespace() {
    assert!(!signals_tool_intent(""));
    assert!(!signals_tool_intent("   "));
    assert!(!signals_tool_intent("\n\n\n"));
}

// ===========================================================================
// Constants tests
// ===========================================================================

#[test]
fn test_max_nudges_respected() {
    // MAX_INTENT_NUDGES should be 2 (not a magic number)
    assert_eq!(MAX_INTENT_NUDGES, 2);
}

#[test]
fn test_nudge_message_is_descriptive() {
    // The nudge message should instruct the LLM to make actual tool calls
    assert!(TOOL_INTENT_NUDGE.contains("tool call"));
    assert!(TOOL_INTENT_NUDGE.contains("describing"));
    assert!(!TOOL_INTENT_NUDGE.is_empty());
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn test_intent_with_tool_name_mentioned() {
    // Even mentioning a tool name should detect intent if pattern matches
    assert!(signals_tool_intent(
        "Let me search using memory_search to find relevant entries."
    ));
    assert!(signals_tool_intent(
        "I'll execute the execute_bash tool to check the status."
    ));
}

#[test]
fn test_no_false_positive_on_past_tense() {
    // Past tense should not trigger (LLM is describing what it already did)
    assert!(!signals_tool_intent(
        "I searched for the payment details and found three results."
    ));
    assert!(!signals_tool_intent(
        "I checked the wallet balance and it shows 50000 sats."
    ));
}

#[test]
fn test_multiline_detection() {
    // Intent phrase on a non-first line should still be detected
    let multiline =
        "Here's what I found so far.\n\nLet me search for more details about the BRC-31 protocol.";
    assert!(signals_tool_intent(multiline));
}

#[test]
fn test_intent_at_end_of_long_response() {
    let long_text = format!(
        "This is a detailed analysis of the payment protocol.\n{}\nLet me search for additional information.",
        "More analysis content. ".repeat(50)
    );
    assert!(signals_tool_intent(&long_text));
}
