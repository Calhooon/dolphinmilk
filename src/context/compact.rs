//! Structured compaction — Tier 2 compaction with 9-section prompt,
//! CompactionBoundary persistence, circuit breaker, and duplicate detection.
//!
//! Tier 1 (microcompact) lives in `manager.rs` — zero LLM cost cleanup.
//! This module handles the more expensive LLM-driven compaction when
//! microcompact alone isn't enough.

use std::collections::HashMap;
use std::ops::Range;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// CompactionBoundary — serializable compaction state
// ---------------------------------------------------------------------------

/// Boundary marker produced after a compaction pass.
///
/// Captures the summary, metadata, and cross-cutting concerns that must
/// survive compaction (discovered tools, proof references, chain continuity).
/// Used by Wave 2 (#271) for tool discovery persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionBoundary {
    /// LLM-generated summary of compacted messages.
    pub summary: String,
    /// Index range of messages that were compacted (in the original history).
    pub compacted_range: Range<usize>,
    /// Key context items preserved across the boundary.
    pub preserved_context: Vec<String>,
    /// Deferred tool names that were discovered before compaction.
    /// These survive compaction so the agent retains tool awareness.
    pub discovered_tools: Vec<String>,
    /// SHA-256 hash of the last message before compaction (BRC-60 chain continuity).
    pub last_message_hash: String,
    /// BRC-18 proof transaction IDs that were referenced in compacted messages.
    pub proof_references: Vec<String>,
    /// Approximate tokens saved by the compaction.
    pub token_savings: usize,
    /// Satoshis saved by reducing context size (fewer tokens = cheaper LLM calls).
    pub sats_saved: u64,
    /// When the compaction occurred.
    pub timestamp: DateTime<Utc>,
}

impl CompactionBoundary {
    /// Create a new compaction boundary.
    pub fn new(summary: String, compacted_range: Range<usize>, token_savings: usize) -> Self {
        Self {
            summary,
            compacted_range,
            preserved_context: Vec::new(),
            discovered_tools: Vec::new(),
            last_message_hash: String::new(),
            proof_references: Vec::new(),
            token_savings,
            sats_saved: 0,
            timestamp: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// 9-section structured compaction prompt
// ---------------------------------------------------------------------------

/// The structured compaction prompt that replaces the old 5-category summary.
///
/// Uses `<analysis>` scratchpad (stripped from output) and `<summary>` tags.
/// The 9 sections ensure comprehensive context preservation.
pub const COMPACTION_PROMPT: &str = concat!(
    "You are a context compaction engine. The following conversation history is about to be ",
    "dropped from the context window. Your job is to produce a structured summary that ",
    "preserves all critical information.\n\n",
    "First, analyze the conversation in an <analysis> block (this will be stripped from the output).\n",
    "Then produce a <summary> block with EXACTLY these 9 sections:\n\n",
    "<analysis>\n",
    "Think through what information is critical to preserve...\n",
    "</analysis>\n\n",
    "<summary>\n",
    "## 1. Primary Request\n",
    "What the user originally asked for.\n\n",
    "## 2. Key Technical Concepts\n",
    "Domain terms, patterns, architecture decisions mentioned.\n\n",
    "## 3. Files/Code\n",
    "Important files, functions, line numbers referenced.\n\n",
    "## 4. Errors/Fixes\n",
    "What broke and how it was fixed.\n\n",
    "## 5. Problem Solving\n",
    "Key decisions and reasoning.\n\n",
    "## 6. All User Messages\n",
    "Verbatim user requirements (quoted).\n\n",
    "## 7. Pending Tasks\n",
    "What's left to do.\n\n",
    "## 8. Current Work\n",
    "What was being worked on when compacted.\n\n",
    "## 9. Optional Next Step\n",
    "What should happen next.\n",
    "</summary>\n\n",
    "IMPORTANT: Keep each section concise. Omit sections that have no relevant content ",
    "(write \"None\" for the section). Preserve exact file paths, function names, and ",
    "numerical values. Quote user messages verbatim when possible.",
);

/// Section names for the 9-section compaction prompt.
pub const COMPACTION_SECTIONS: [&str; 9] = [
    "Primary Request",
    "Key Technical Concepts",
    "Files/Code",
    "Errors/Fixes",
    "Problem Solving",
    "All User Messages",
    "Pending Tasks",
    "Current Work",
    "Optional Next Step",
];

/// Extract the `<summary>` block from a compaction LLM response,
/// stripping the `<analysis>` scratchpad.
pub fn extract_summary(response: &str) -> String {
    // Try to extract content between <summary> tags
    if let Some(start) = response.find("<summary>") {
        let after_tag = start + "<summary>".len();
        if let Some(end) = response[after_tag..].find("</summary>") {
            return response[after_tag..after_tag + end].trim().to_string();
        }
        // No closing tag — take everything after <summary>
        return response[after_tag..].trim().to_string();
    }
    // No <summary> tags — return the whole response minus any <analysis> block
    let mut result = response.to_string();
    if let Some(start) = result.find("<analysis>") {
        if let Some(end) = result.find("</analysis>") {
            let end_tag = end + "</analysis>".len();
            result = format!("{}{}", &result[..start], &result[end_tag..])
                .trim()
                .to_string();
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Circuit breaker for LLM compaction
// ---------------------------------------------------------------------------

/// Tracks consecutive LLM compaction failures.
///
/// After `MAX_FAILURES` consecutive failures, falls back to microcompact-only
/// mode. Counter resets on any successful compaction.
pub struct CompactionCircuitBreaker {
    consecutive_failures: u32,
    last_failure_reason: Option<String>,
}

/// Maximum consecutive compaction failures before fallback.
const MAX_FAILURES: u32 = 3;

impl CompactionCircuitBreaker {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            last_failure_reason: None,
        }
    }

    /// Record a successful compaction. Resets the failure counter.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.last_failure_reason = None;
    }

    /// Record a failed compaction attempt.
    pub fn record_failure(&mut self, reason: &str) {
        self.consecutive_failures += 1;
        self.last_failure_reason = Some(reason.to_string());
        tracing::warn!(
            "Compaction failure #{}: {}",
            self.consecutive_failures,
            reason
        );
    }

    /// Whether we should skip LLM compaction and fall back to microcompact only.
    pub fn should_fallback(&self) -> bool {
        self.consecutive_failures >= MAX_FAILURES
    }

    /// Number of consecutive failures.
    pub fn failure_count(&self) -> u32 {
        self.consecutive_failures
    }

    /// Last failure reason, if any.
    pub fn last_failure_reason(&self) -> Option<&str> {
        self.last_failure_reason.as_deref()
    }
}

impl Default for CompactionCircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Duplicate content detection
// ---------------------------------------------------------------------------

/// Detect duplicate content in messages before compaction.
///
/// Hashes the first 500 chars of each message's content using SHA-256.
/// Returns indices of messages to keep (deduplicating repeated content).
pub fn dedup_messages(messages: &[serde_json::Value]) -> Vec<usize> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut keep = Vec::with_capacity(messages.len());

    for (i, msg) in messages.iter().enumerate() {
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");

        // Only dedup within the same role (don't dedup user asking same question
        // if assistant answered differently)
        let hash_input = format!("{}:{}", role, &content[..content.len().min(500)]);
        let hash = content_hash_short(&hash_input);

        if let Some(&first_idx) = seen.get(&hash) {
            tracing::debug!(
                "Duplicate content detected at index {} (first seen at {})",
                i,
                first_idx
            );
        } else {
            seen.insert(hash, i);
            keep.push(i);
        }
    }

    keep
}

/// SHA-256 hash of the first 500 chars of content, returned as hex string.
fn content_hash_short(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute SHA-256 hash of a message's content for BRC-60 chain continuity.
pub fn message_content_hash(msg: &serde_json::Value) -> String {
    let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
    content_hash_short(&format!("{}:{}", role, content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_boundary_serde_roundtrip() {
        let boundary = CompactionBoundary {
            summary: "Test summary".to_string(),
            compacted_range: 0..10,
            preserved_context: vec!["ctx1".to_string()],
            discovered_tools: vec!["browser".to_string(), "generate_image".to_string()],
            last_message_hash: "abc123".to_string(),
            proof_references: vec!["txid1".to_string()],
            token_savings: 5000,
            sats_saved: 200,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&boundary).unwrap();
        let restored: CompactionBoundary = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.summary, "Test summary");
        assert_eq!(restored.compacted_range, 0..10);
        assert_eq!(restored.discovered_tools.len(), 2);
        assert_eq!(restored.token_savings, 5000);
        assert_eq!(restored.sats_saved, 200);
    }

    #[test]
    fn test_compaction_boundary_new() {
        let b = CompactionBoundary::new("summary".into(), 5..15, 3000);
        assert_eq!(b.summary, "summary");
        assert_eq!(b.compacted_range, 5..15);
        assert_eq!(b.token_savings, 3000);
        assert!(b.discovered_tools.is_empty());
        assert!(b.proof_references.is_empty());
    }

    #[test]
    fn test_extract_summary_with_tags() {
        let response = "<analysis>thinking...</analysis>\n<summary>\n## 1. Primary Request\nBuild a feature.\n</summary>";
        let summary = extract_summary(response);
        assert!(summary.contains("Primary Request"));
        assert!(!summary.contains("thinking"));
    }

    #[test]
    fn test_extract_summary_no_closing_tag() {
        let response = "<summary>\n## 1. Primary Request\nBuild a feature.";
        let summary = extract_summary(response);
        assert!(summary.contains("Primary Request"));
    }

    #[test]
    fn test_extract_summary_no_tags() {
        let response = "Just a plain summary with no tags.";
        let summary = extract_summary(response);
        assert_eq!(summary, response);
    }

    #[test]
    fn test_extract_summary_strips_analysis() {
        let response = "Before <analysis>scratchpad</analysis> After";
        let summary = extract_summary(response);
        assert!(summary.contains("Before"));
        assert!(summary.contains("After"));
        assert!(!summary.contains("scratchpad"));
    }

    #[test]
    fn test_circuit_breaker_initial_state() {
        let cb = CompactionCircuitBreaker::new();
        assert_eq!(cb.failure_count(), 0);
        assert!(!cb.should_fallback());
        assert!(cb.last_failure_reason().is_none());
    }

    #[test]
    fn test_circuit_breaker_below_threshold() {
        let mut cb = CompactionCircuitBreaker::new();
        cb.record_failure("error 1");
        cb.record_failure("error 2");
        assert_eq!(cb.failure_count(), 2);
        assert!(!cb.should_fallback());
        assert_eq!(cb.last_failure_reason(), Some("error 2"));
    }

    #[test]
    fn test_circuit_breaker_triggers_fallback() {
        let mut cb = CompactionCircuitBreaker::new();
        cb.record_failure("error 1");
        cb.record_failure("error 2");
        cb.record_failure("error 3");
        assert_eq!(cb.failure_count(), 3);
        assert!(cb.should_fallback());
    }

    #[test]
    fn test_circuit_breaker_reset_on_success() {
        let mut cb = CompactionCircuitBreaker::new();
        cb.record_failure("error 1");
        cb.record_failure("error 2");
        cb.record_success();
        assert_eq!(cb.failure_count(), 0);
        assert!(!cb.should_fallback());
        assert!(cb.last_failure_reason().is_none());
    }

    #[test]
    fn test_circuit_breaker_beyond_threshold() {
        let mut cb = CompactionCircuitBreaker::new();
        for i in 0..5 {
            cb.record_failure(&format!("error {i}"));
        }
        assert_eq!(cb.failure_count(), 5);
        assert!(cb.should_fallback());
    }

    #[test]
    fn test_dedup_messages_no_duplicates() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi there"}),
            serde_json::json!({"role": "user", "content": "how are you"}),
        ];
        let keep = dedup_messages(&messages);
        assert_eq!(keep, vec![0, 1, 2]);
    }

    #[test]
    fn test_dedup_messages_with_duplicates() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi"}),
            serde_json::json!({"role": "user", "content": "hello"}), // duplicate
            serde_json::json!({"role": "assistant", "content": "different response"}),
        ];
        let keep = dedup_messages(&messages);
        assert_eq!(keep, vec![0, 1, 3]); // index 2 is deduped
    }

    #[test]
    fn test_dedup_messages_same_content_different_role() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hello"}), // same content, different role
        ];
        let keep = dedup_messages(&messages);
        assert_eq!(keep, vec![0, 1]); // both kept because different roles
    }

    #[test]
    fn test_dedup_messages_empty() {
        let keep = dedup_messages(&[]);
        assert!(keep.is_empty());
    }

    #[test]
    fn test_dedup_messages_tool_results() {
        let messages = vec![
            serde_json::json!({"role": "tool", "content": "file contents here"}),
            serde_json::json!({"role": "tool", "content": "file contents here"}), // duplicate tool result
            serde_json::json!({"role": "tool", "content": "different result"}),
        ];
        let keep = dedup_messages(&messages);
        assert_eq!(keep, vec![0, 2]);
    }

    #[test]
    fn test_message_content_hash_deterministic() {
        let msg = serde_json::json!({"role": "user", "content": "test"});
        let h1 = message_content_hash(&msg);
        let h2 = message_content_hash(&msg);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn test_message_content_hash_different_content() {
        let msg1 = serde_json::json!({"role": "user", "content": "test1"});
        let msg2 = serde_json::json!({"role": "user", "content": "test2"});
        assert_ne!(message_content_hash(&msg1), message_content_hash(&msg2));
    }

    #[test]
    fn test_compaction_sections_count() {
        assert_eq!(COMPACTION_SECTIONS.len(), 9);
    }

    #[test]
    fn test_compaction_prompt_contains_all_sections() {
        for section in &COMPACTION_SECTIONS {
            assert!(
                COMPACTION_PROMPT.contains(section),
                "Compaction prompt missing section: {section}"
            );
        }
    }
}
