//! Context window manager — token tracking and file-based offloading.
//!
//! Manages the message list sent to the LLM. Key responsibilities:
//!   - Track approximate token usage per message
//!   - Truncate old history when approaching context limit
//!   - Offload large results to files (paper's 8x token reduction insight)
//!   - Maintain system prompt + recent history + tool results
//!   - Intelligent token analysis and budget-aware compaction

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Rough token estimation: ~4 chars per token for English text.
const CHARS_PER_TOKEN: usize = 4;

/// Default context window budget (tokens).
pub const DEFAULT_MAX_TOKENS: usize = 128_000;

/// When a single message exceeds this, offload it to a file.
const OFFLOAD_THRESHOLD_TOKENS: usize = 2_000;

/// Default maximum content length (chars) for any message in history.
/// Used by `compact_content()` (the convenience wrapper) and as the floor
/// for the dynamic `max_message_chars()` method on ContextManager.
const DEFAULT_MAX_MESSAGE_CHARS: usize = 8_000;

/// Default: keep at least this many recent messages regardless of token pressure.
const DEFAULT_MIN_RECENT_MESSAGES: usize = 8;

/// Default: maximum conversation turns to keep before hard-trimming.
const DEFAULT_MAX_HISTORY_TURNS: usize = 40;

/// Rough token estimate from character count.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() / CHARS_PER_TOKEN).max(1)
}

/// Estimate tokens for a single OpenAI-format message.
pub fn estimate_message_tokens(msg: &Value) -> usize {
    let mut tokens = 4; // overhead per message
    if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
        tokens += estimate_tokens(content);
    }
    if let Some(tc) = msg.get("tool_calls") {
        if let Ok(s) = serde_json::to_string(tc) {
            tokens += estimate_tokens(&s);
        }
    }
    tokens
}

/// Default compaction threshold — token utilization ratio that triggers auto-compaction.
pub const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8;

// ---------------------------------------------------------------------------
// Token analysis and budget-aware compaction
// ---------------------------------------------------------------------------

/// Breakdown of token usage across context window categories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBreakdown {
    /// Tokens used by the system prompt.
    pub system_prompt_tokens: usize,
    /// Tokens used by tool definitions.
    pub tool_definitions_tokens: usize,
    /// Tokens used by conversation history.
    pub conversation_tokens: usize,
    /// Tokens used by memory summaries.
    pub memory_tokens: usize,
    /// Tokens used by skill sections.
    pub skill_tokens: usize,
    /// Total tokens across all categories.
    pub total_tokens: usize,
    /// Context window limit in tokens.
    pub limit_tokens: usize,
    /// Utilization as a percentage (0.0–100.0).
    pub utilization_pct: f64,
}

/// Record of a compaction event for audit/debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEvent {
    /// Tokens before compaction.
    pub tokens_before: usize,
    /// Tokens after compaction.
    pub tokens_after: usize,
    /// Number of messages dropped during compaction.
    pub messages_dropped: usize,
    /// Timestamp of the compaction event (RFC 3339).
    pub timestamp: String,
}

/// Stateless analyzer for token budgets and compaction decisions.
pub struct TokenAnalyzer;

impl TokenAnalyzer {
    /// Analyze the token breakdown of the current context window.
    ///
    /// Each category is estimated independently from its source text.
    /// `limit_tokens` is the total context window size (before output reservation).
    pub fn analyze(
        system_prompt: &str,
        tool_defs: &str,
        history: &[Value],
        memory_summary: &str,
        skills_section: &str,
        limit_tokens: usize,
    ) -> TokenBreakdown {
        let system_prompt_tokens = estimate_tokens(system_prompt);
        let tool_definitions_tokens = estimate_tokens(tool_defs);
        let conversation_tokens: usize = history.iter().map(estimate_message_tokens).sum();
        let memory_tokens = estimate_tokens(memory_summary);
        let skill_tokens = estimate_tokens(skills_section);
        let total_tokens = system_prompt_tokens
            + tool_definitions_tokens
            + conversation_tokens
            + memory_tokens
            + skill_tokens;
        let utilization_pct = if limit_tokens > 0 {
            (total_tokens as f64 / limit_tokens as f64) * 100.0
        } else {
            0.0
        };
        TokenBreakdown {
            system_prompt_tokens,
            tool_definitions_tokens,
            conversation_tokens,
            memory_tokens,
            skill_tokens,
            total_tokens,
            limit_tokens,
            utilization_pct,
        }
    }

    /// Adjust compaction threshold downward as budget pressure increases.
    ///
    /// When the agent has spent >80% of its budget, compact more aggressively
    /// to avoid wasting remaining sats on overly long contexts.
    ///
    /// - `base_threshold`: configured threshold (e.g. 0.8)
    /// - `budget_spent_pct`: percentage of task budget spent (0.0–100.0)
    pub fn budget_aware_threshold(base_threshold: f64, budget_spent_pct: f64) -> f64 {
        if budget_spent_pct > 80.0 {
            0.6_f64.min(base_threshold)
        } else if budget_spent_pct > 60.0 {
            0.7_f64.min(base_threshold)
        } else {
            base_threshold
        }
    }

    /// Check whether the current token utilization exceeds the compaction threshold.
    pub fn needs_token_compaction(breakdown: &TokenBreakdown, threshold: f64) -> bool {
        // Convert threshold (0.0–1.0) to percentage (0.0–100.0) for comparison.
        breakdown.utilization_pct > threshold * 100.0
    }

    /// Build a structured summary from messages that are about to be dropped.
    ///
    /// Extracts three categories of durable context:
    /// - **User facts**: key statements from user messages
    /// - **Tool outcomes**: tool names and abbreviated results
    /// - **Decisions**: assistant decisions and reasoning
    ///
    /// Returns a multi-line text summary suitable for injection as a system message.
    pub fn build_summary_header(dropped_messages: &[Value]) -> String {
        if dropped_messages.is_empty() {
            return "[No earlier context to summarize]".to_string();
        }

        let mut user_facts = Vec::new();
        let mut tool_outcomes = Vec::new();
        let mut decisions = Vec::new();

        for msg in dropped_messages {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

            match role {
                "user" => {
                    // Extract first meaningful sentence as a fact.
                    let fact = content.lines().next().unwrap_or("").trim();
                    if !fact.is_empty() && fact.len() > 5 {
                        let truncated = if fact.len() > 200 {
                            format!("{}...", &fact[..200])
                        } else {
                            fact.to_string()
                        };
                        user_facts.push(truncated);
                    }
                }
                "tool" => {
                    let tool_name = msg
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown_tool");
                    let result_preview = if content.len() > 100 {
                        format!("{}...", &content[..100])
                    } else {
                        content.to_string()
                    };
                    if !result_preview.is_empty() {
                        tool_outcomes.push(format!("{tool_name}: {result_preview}"));
                    }
                }
                "assistant" => {
                    // Skip tool-call-only assistant messages.
                    if content.is_empty()
                        || msg
                            .get("tool_calls")
                            .and_then(|v| v.as_array())
                            .is_some_and(|tc| !tc.is_empty() && content.is_empty())
                    {
                        continue;
                    }
                    let decision = content.lines().next().unwrap_or("").trim();
                    if !decision.is_empty() && decision.len() > 5 {
                        let truncated = if decision.len() > 200 {
                            format!("{}...", &decision[..200])
                        } else {
                            decision.to_string()
                        };
                        decisions.push(truncated);
                    }
                }
                _ => {}
            }
        }

        let mut sections = Vec::new();
        if !user_facts.is_empty() {
            sections.push(format!(
                "User facts:\n{}",
                user_facts
                    .iter()
                    .take(5)
                    .map(|f| format!("- {f}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !tool_outcomes.is_empty() {
            sections.push(format!(
                "Tool outcomes:\n{}",
                tool_outcomes
                    .iter()
                    .take(5)
                    .map(|o| format!("- {o}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !decisions.is_empty() {
            sections.push(format!(
                "Decisions:\n{}",
                decisions
                    .iter()
                    .take(5)
                    .map(|d| format!("- {d}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        if sections.is_empty() {
            "[Earlier conversation context was compacted — no extractable facts]".to_string()
        } else {
            format!(
                "[Summary of {} compacted messages]\n\n{}",
                dropped_messages.len(),
                sections.join("\n\n")
            )
        }
    }

    /// Build a re-injection payload after compaction.
    ///
    /// Returns a message list consisting of:
    /// 1. The system prompt (always first)
    /// 2. A summary of dropped messages
    /// 3. The most recent `recent_count` turns from history
    pub fn build_reinjection(
        system_prompt: &str,
        history: &[Value],
        dropped_messages: &[Value],
        recent_count: usize,
    ) -> Vec<Value> {
        let mut result = Vec::new();

        // System prompt always first.
        result.push(serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }));

        // Inject summary of dropped messages.
        let summary = Self::build_summary_header(dropped_messages);
        result.push(serde_json::json!({
            "role": "system",
            "content": summary,
        }));

        // Recent turns from history.
        let start = if history.len() > recent_count {
            history.len() - recent_count
        } else {
            0
        };
        result.extend(history[start..].iter().cloned());

        result
    }
}

/// Regex-like pattern for data URIs: `data:image/...;base64,...`
/// Matches `![...]( data:image/...;base64,... )` and bare `data:image/...;base64,...`
/// Keeps https:// URLs intact.
static DATA_URI_PREFIX: &str = "data:image/";

/// Strip base64 data URIs and truncate if oversized.
/// Uses a fixed 8,000 character limit (suitable for conversation storage).
pub fn compact_content(content: &str) -> String {
    compact_content_with_limit(content, DEFAULT_MAX_MESSAGE_CHARS)
}

/// Strip base64 data URIs and truncate if oversized.
/// `max_chars` controls the truncation threshold.
///
/// 1. Replace `data:image/...;base64,...` URIs with a placeholder.
///    Handles both markdown image syntax `![alt](data:image/...)` and bare URIs.
/// 2. If the result still exceeds `max_chars`, truncate with a summary.
///
/// Returns the original string unchanged if no modifications are needed.
pub fn compact_content_with_limit(content: &str, max_chars: usize) -> String {
    let mut result = content.to_string();

    // Strip data URIs (base64 images embedded inline)
    if result.contains(DATA_URI_PREFIX) {
        // Handle markdown images: ![alt](data:image/...;base64,...)
        let mut output = String::with_capacity(result.len());
        let mut remaining = result.as_str();
        while let Some(start) = remaining.find("![") {
            output.push_str(&remaining[..start]);
            // Find the ](data:image/ pattern
            if let Some(paren_start) = remaining[start..].find("](data:image/") {
                let abs_paren = start + paren_start;
                // Extract alt text
                let alt = &remaining[start + 2..abs_paren];
                // Find closing )
                if let Some(close) = remaining[abs_paren + 2..].find(')') {
                    let abs_close = abs_paren + 2 + close;
                    output.push_str(&format!("[Image: {alt} — base64 data URI removed]"));
                    remaining = &remaining[abs_close + 1..];
                    continue;
                }
            }
            // Not a data URI image, keep the ![
            output.push_str("![");
            remaining = &remaining[start + 2..];
        }
        output.push_str(remaining);
        result = output;

        // Also handle bare data:image/ URIs not in markdown syntax
        let mut output2 = String::with_capacity(result.len());
        let mut remaining2 = result.as_str();
        while let Some(start) = remaining2.find("data:image/") {
            output2.push_str(&remaining2[..start]);
            // Find the end of the data URI (whitespace, quote, paren, or end of string)
            let after = &remaining2[start..];
            let end = after
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == '>')
                .unwrap_or(after.len());
            output2.push_str("[base64 data URI removed]");
            remaining2 = &remaining2[start + end..];
        }
        output2.push_str(remaining2);
        result = output2;
    }

    // If still oversized after stripping data URIs, truncate
    if result.len() > max_chars {
        let original_len = result.len();
        let content_hint = if content.contains("data:image")
            || content.contains("/9j/")
            || content.contains("iVBOR")
            || content.contains("R0lGOD")
        {
            "base64-encoded image"
        } else if content.starts_with('{') || content.starts_with('[') {
            "JSON data"
        } else if content.starts_with("<!") || content.starts_with("<html") {
            "HTML content"
        } else {
            "large output"
        };

        // UTF-8 safe slicing: byte index `keep` may fall inside a multi-byte
        // character (e.g., an emoji or any non-ASCII). `&str[..keep]` panics
        // in that case. Back off to the nearest char boundary ≤ keep so the
        // slice is always valid. Worst case we lose up to 3 bytes off the
        // preview, which is imperceptible compared to an 8K truncation.
        let keep = (max_chars / 2).min(result.len());
        let mut safe_keep = keep;
        while safe_keep > 0 && !result.is_char_boundary(safe_keep) {
            safe_keep -= 1;
        }
        let preview = &result[..safe_keep];
        result = format!(
            "{}\n\n[{} truncated: {} chars → {} chars. If this was a tool result, the full data may be in a workspace file — use file_read to access it.]",
            preview, content_hint, original_len, keep
        );

        tracing::info!(
            "Compacted message: {} chars → {} chars ({})",
            original_len,
            result.len(),
            content_hint,
        );
    }

    result
}

/// Default output token reservation — deducted from context window for LLM responses.
const DEFAULT_OUTPUT_TOKENS: usize = 16_384;

/// Manages the context window for LLM calls.
pub struct ContextManager {
    pub max_tokens: usize,
    pub offload_dir: PathBuf,
    min_recent_messages: usize,
    max_history_turns: usize,
    /// Output tokens reserved for LLM response generation.
    /// Deducted from the context window to compute the input budget.
    output_tokens: usize,
    offloaded_files: Vec<String>,
    offload_counter: u32,
    /// LLM-generated summary of compacted messages, prepended to context when set.
    compaction_summary: Option<String>,
    /// Token utilization threshold (0.0–1.0) above which auto-compaction triggers.
    compaction_threshold: f64,
    /// History of compaction events for diagnostics.
    compaction_events: Vec<CompactionEvent>,
}

impl ContextManager {
    pub fn new(max_tokens: usize, offload_dir: PathBuf) -> Self {
        Self {
            max_tokens,
            offload_dir,
            min_recent_messages: DEFAULT_MIN_RECENT_MESSAGES,
            max_history_turns: DEFAULT_MAX_HISTORY_TURNS,
            output_tokens: DEFAULT_OUTPUT_TOKENS,
            offloaded_files: Vec::new(),
            offload_counter: 0,
            compaction_summary: None,
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
            compaction_events: Vec::new(),
        }
    }

    /// Create with configurable turn limits.
    pub fn with_limits(
        max_tokens: usize,
        offload_dir: PathBuf,
        min_recent_messages: usize,
        max_history_turns: usize,
    ) -> Self {
        Self {
            max_tokens,
            offload_dir,
            min_recent_messages,
            max_history_turns,
            output_tokens: DEFAULT_OUTPUT_TOKENS,
            offloaded_files: Vec::new(),
            offload_counter: 0,
            compaction_summary: None,
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
            compaction_events: Vec::new(),
        }
    }

    /// Create with configurable turn limits and output token reservation.
    pub fn with_output_tokens(
        max_tokens: usize,
        offload_dir: PathBuf,
        min_recent_messages: usize,
        max_history_turns: usize,
        output_tokens: usize,
    ) -> Self {
        Self {
            max_tokens,
            offload_dir,
            min_recent_messages,
            max_history_turns,
            output_tokens,
            offloaded_files: Vec::new(),
            offload_counter: 0,
            compaction_summary: None,
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
            compaction_events: Vec::new(),
        }
    }

    /// Check if history exceeds max_history_turns and needs compaction.
    pub fn needs_compaction(&self, history_len: usize) -> bool {
        history_len > self.max_history_turns
    }

    /// Get the current compaction summary, if any.
    pub fn compaction_summary(&self) -> Option<&str> {
        self.compaction_summary.as_deref()
    }

    /// Set a compaction summary (from LLM-generated content).
    pub fn set_compaction_summary(&mut self, summary: String) {
        self.compaction_summary = Some(summary);
    }

    /// Update the output token reservation.
    ///
    /// Used when discovered model capabilities provide more accurate output limits
    /// than the hardcoded defaults set at construction time.
    pub fn set_output_tokens(&mut self, output_tokens: usize) {
        self.output_tokens = output_tokens;
    }

    /// Return the current output token reservation.
    pub fn output_tokens(&self) -> usize {
        self.output_tokens
    }

    /// Return the max_history_turns setting.
    pub fn max_history_turns(&self) -> usize {
        self.max_history_turns
    }

    /// Dynamic per-message character limit based on context window budget.
    /// Scales with the model's available input tokens: 15% of input budget per message.
    /// Floor of 8,000 chars (prevents overly aggressive compaction on tiny windows).
    /// Ceiling of 400,000 chars (prevents single messages dominating with huge windows).
    pub fn max_message_chars(&self) -> usize {
        let input_budget = self.max_tokens.saturating_sub(self.output_tokens);
        let budget_chars = input_budget * CHARS_PER_TOKEN;
        let dynamic_limit = budget_chars * 15 / 100; // 15% of input budget per message
        dynamic_limit.clamp(8_000, 400_000) // floor 8K, ceiling 400K
    }

    /// Set the compaction threshold (0.0–1.0).
    pub fn set_compaction_threshold(&mut self, threshold: f64) {
        self.compaction_threshold = threshold.clamp(0.0, 1.0);
    }

    /// Return the current compaction threshold.
    pub fn compaction_threshold(&self) -> f64 {
        self.compaction_threshold
    }

    /// Return the history of compaction events.
    pub fn compaction_events(&self) -> &[CompactionEvent] {
        &self.compaction_events
    }

    // =========================================================================
    // Microcompact — Tier 1 zero-LLM-cost cleanup
    // =========================================================================

    /// Tool names whose results can be cleared during microcompaction.
    /// These tools produce large, transient output that doesn't need to persist.
    pub const COMPACTABLE_TOOLS: &'static [&'static str] = &[
        "file_read",
        "file_write",
        "execute_bash",
        "file_search",
        "web_fetch",
        "x402_call",
    ];

    /// Default number of recent results to keep per compactable tool.
    pub const MICROCOMPACT_KEEP_RECENT: usize = 3;

    /// Placeholder text for cleared tool results.
    pub const MICROCOMPACT_PLACEHOLDER: &'static str =
        "[Content cleared — use search_tools or file_read to reload]";

    /// Microcompact: zero-LLM-cost cleanup of old tool results.
    ///
    /// Two policies applied in order:
    /// 1. **Count-based**: When a compactable tool has more than `keep_recent`
    ///    results in the history, clear the oldest ones.
    /// 2. **Time-based**: When there's a large gap (>60 min) since the last
    ///    assistant message, clear all compactable tool results except the
    ///    most recent N.
    ///
    /// Returns the number of messages modified.
    pub fn microcompact(
        &self,
        history: &mut [Value],
        keep_recent: usize,
        time_gap_minutes: Option<u64>,
    ) -> usize {
        let mut modified = 0;

        // Collect indices of compactable tool results, grouped by tool name
        let mut tool_indices: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, msg) in history.iter().enumerate() {
            if msg.get("role").and_then(|v| v.as_str()) != Some("tool") {
                continue;
            }
            // Check if this tool result's corresponding tool_call is from a compactable tool
            let tool_name = Self::extract_tool_name_for_result(msg, history);
            if let Some(name) = tool_name {
                if Self::COMPACTABLE_TOOLS.contains(&name.as_str()) {
                    tool_indices.entry(name).or_default().push(i);
                }
            }
        }

        // Count-based policy: clear oldest when exceeding keep_recent per tool
        for indices in tool_indices.values() {
            if indices.len() > keep_recent {
                let to_clear = indices.len() - keep_recent;
                for &idx in indices.iter().take(to_clear) {
                    if Self::clear_tool_result(&mut history[idx]) {
                        modified += 1;
                    }
                }
            }
        }

        // Time-based policy: if requested, clear old results based on time gap
        if let Some(gap_minutes) = time_gap_minutes {
            if gap_minutes > 60 {
                // Find the last assistant message index
                let last_assistant_idx = history
                    .iter()
                    .rposition(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"));

                if let Some(last_asst) = last_assistant_idx {
                    // Clear all compactable tool results before the last assistant message,
                    // except the most recent keep_recent overall
                    let all_compactable: Vec<usize> = tool_indices
                        .values()
                        .flat_map(|v| v.iter())
                        .copied()
                        .filter(|&idx| idx < last_asst)
                        .collect();

                    let mut sorted = all_compactable;
                    sorted.sort();

                    if sorted.len() > keep_recent {
                        let to_clear = sorted.len() - keep_recent;
                        for &idx in sorted.iter().take(to_clear) {
                            if Self::clear_tool_result(&mut history[idx]) {
                                modified += 1;
                            }
                        }
                    }
                }
            }
        }

        if modified > 0 {
            tracing::info!("Microcompact: cleared {} old tool results", modified);
        }
        modified
    }

    /// Extract the tool name for a tool-result message by finding the matching
    /// assistant tool_calls entry.
    fn extract_tool_name_for_result(tool_msg: &Value, history: &[Value]) -> Option<String> {
        let call_id = tool_msg.get("tool_call_id")?.as_str()?;
        // Also check the "name" field on the tool message itself (some formats include it)
        if let Some(name) = tool_msg.get("name").and_then(|v| v.as_str()) {
            return Some(name.to_string());
        }
        // Search backwards through history for the matching assistant tool_calls
        for msg in history.iter().rev() {
            if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(tc) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                for call in tc {
                    if call.get("id").and_then(|v| v.as_str()) == Some(call_id) {
                        return call
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
        }
        None
    }

    /// Replace a tool result's content with the microcompact placeholder.
    /// Returns true if the content was actually changed.
    fn clear_tool_result(msg: &mut Value) -> bool {
        if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
            if content == Self::MICROCOMPACT_PLACEHOLDER {
                return false; // Already cleared
            }
        }
        msg["content"] = Value::String(Self::MICROCOMPACT_PLACEHOLDER.to_string());
        true
    }

    /// Check token utilization and auto-compact if threshold is exceeded.
    ///
    /// Uses `TokenAnalyzer` to measure utilization, then compacts by dropping
    /// older messages while preserving the most recent `min_recent_messages`.
    /// Returns `Some(CompactionEvent)` if compaction occurred, `None` otherwise.
    ///
    /// `budget_spent_pct` enables budget-aware threshold adjustment (0.0–100.0).
    pub fn check_and_compact(
        &mut self,
        system_prompt: &str,
        history: &mut Vec<Value>,
        tool_defs: &str,
        memory_summary: &str,
        skills_section: &str,
        budget_spent_pct: f64,
    ) -> Option<CompactionEvent> {
        let breakdown = TokenAnalyzer::analyze(
            system_prompt,
            tool_defs,
            history,
            memory_summary,
            skills_section,
            self.max_tokens,
        );

        let effective_threshold =
            TokenAnalyzer::budget_aware_threshold(self.compaction_threshold, budget_spent_pct);

        if !TokenAnalyzer::needs_token_compaction(&breakdown, effective_threshold) {
            return None;
        }

        let tokens_before = breakdown.total_tokens;

        // Target: reduce conversation tokens to fit within threshold.
        // Keep the most recent min_recent_messages.
        let overhead = breakdown.system_prompt_tokens
            + breakdown.tool_definitions_tokens
            + breakdown.memory_tokens
            + breakdown.skill_tokens;
        let target_conversation_tokens =
            ((self.max_tokens as f64 * effective_threshold) as usize).saturating_sub(overhead);

        // Walk backwards from end to find how many messages to keep.
        let mut kept_tokens = 0usize;
        let mut keep_from = history.len();
        for (i, msg) in history.iter().enumerate().rev() {
            let msg_tokens = estimate_message_tokens(msg);
            if kept_tokens + msg_tokens > target_conversation_tokens
                && history.len() - i > self.min_recent_messages
            {
                break;
            }
            kept_tokens += msg_tokens;
            keep_from = i;
        }

        if keep_from == 0 {
            // Nothing to drop.
            return None;
        }

        let dropped = &history[..keep_from];
        let messages_dropped = dropped.len();

        // Build and set a summary from the dropped messages.
        let summary = TokenAnalyzer::build_summary_header(dropped);
        self.compaction_summary = Some(summary);

        // Remove the dropped messages.
        *history = history[keep_from..].to_vec();

        let tokens_after = overhead + history.iter().map(estimate_message_tokens).sum::<usize>();

        let event = CompactionEvent {
            tokens_before,
            tokens_after,
            messages_dropped,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        tracing::info!(
            "Auto-compaction: {} → {} tokens ({} messages dropped, threshold={:.0}%)",
            tokens_before,
            tokens_after,
            messages_dropped,
            effective_threshold * 100.0,
        );

        self.compaction_events.push(event.clone());
        Some(event)
    }

    /// Build the message list, fitting within token budget.
    ///
    /// Enforces two limits:
    /// 1. `max_history_turns` — hard cap on conversation length (trims oldest)
    /// 2. Token budget — fits remaining messages within context window
    ///
    /// The input budget is the context window minus the output token reservation.
    /// This prevents the input from consuming tokens that the LLM needs for its response.
    pub fn build_messages(
        &self,
        system_prompt: &str,
        history: &[Value],
        budget_tokens: Option<usize>,
    ) -> Vec<Value> {
        let budget =
            budget_tokens.unwrap_or_else(|| self.max_tokens.saturating_sub(self.output_tokens));
        let mut messages: Vec<Value> = Vec::new();

        // System prompt always included
        let sys_msg = serde_json::json!({"role": "system", "content": system_prompt});
        let sys_tokens = estimate_message_tokens(&sys_msg);
        messages.push(sys_msg);
        let mut remaining = budget.saturating_sub(sys_tokens);

        if history.is_empty() {
            return messages;
        }

        // Compact oversized tool results BEFORE any token estimation.
        // This catches base64 blobs, huge command output, etc. in both
        // prior_messages (conversation history) and current transcript.
        let max_chars = self.max_message_chars();
        let history: Vec<Value> = Self::compact_large_messages(history, max_chars);
        let history = history.as_slice();

        // Enforce max_history_turns: hard-trim oldest messages first.
        // If a compaction summary exists, prepend it as a system message.
        let dropped_count = history.len().saturating_sub(self.max_history_turns);
        let history = if dropped_count > 0 {
            &history[dropped_count..]
        } else {
            history
        };

        // If we trimmed messages and have a compaction summary, inject it
        if dropped_count > 0 {
            if let Some(ref summary) = self.compaction_summary {
                let summary_msg = serde_json::json!({
                    "role": "system",
                    "content": format!(
                        "[Earlier conversation summary — {} messages compacted]\n\n{}",
                        dropped_count,
                        summary,
                    )
                });
                let summary_tokens = estimate_message_tokens(&summary_msg);
                messages.push(summary_msg);
                remaining = remaining.saturating_sub(summary_tokens);
            }
        }

        // Calculate token cost of each history message
        let msg_tokens: Vec<(&Value, usize)> = history
            .iter()
            .map(|msg| (msg, estimate_message_tokens(msg)))
            .collect();

        let total_history_tokens: usize = msg_tokens.iter().map(|(_, t)| *t).sum();

        if total_history_tokens <= remaining {
            // Everything fits — still sanitize in case prior_messages created orphaned pairs
            messages.extend(history.iter().cloned());
            Self::sanitize_tool_pairs(&mut messages);
            return messages;
        }

        // Need to truncate: keep last min_recent_messages, fill from oldest with what fits
        let split = if msg_tokens.len() > self.min_recent_messages {
            msg_tokens.len() - self.min_recent_messages
        } else {
            0
        };
        let (older, recent) = msg_tokens.split_at(split);

        let recent_tokens: usize = recent.iter().map(|(_, t)| *t).sum();
        if recent_tokens > remaining {
            // Even recent messages don't fit — truncate them
            messages.extend(Self::truncate_messages(
                &recent.iter().map(|(m, _)| (*m).clone()).collect::<Vec<_>>(),
                remaining,
            ));
            return messages;
        }

        remaining -= recent_tokens;

        // Fill from oldest
        let mut kept_older: Vec<Value> = Vec::new();
        let mut dropped_count = 0u32;
        for (msg, tokens) in older {
            if *tokens <= remaining {
                kept_older.push((*msg).clone());
                remaining -= tokens;
            } else {
                dropped_count += 1;
            }
        }

        if dropped_count > 0 {
            kept_older.push(serde_json::json!({
                "role": "system",
                "content": format!("[{dropped_count} earlier messages truncated to fit context window]"),
            }));
        }

        messages.extend(kept_older);
        messages.extend(recent.iter().map(|(m, _)| (*m).clone()));

        // Remove orphaned tool messages after truncation
        Self::sanitize_tool_pairs(&mut messages);

        messages
    }

    /// Remove orphaned tool messages that arise after history truncation.
    ///
    /// Two cases handled:
    /// - Assistant message with `tool_calls` whose results were truncated → remove the assistant msg
    /// - Tool result message whose parent assistant `tool_calls` was truncated → remove the tool msg
    fn sanitize_tool_pairs(messages: &mut Vec<Value>) {
        use std::collections::HashSet;

        // Collect tool_call IDs present in assistant messages
        let mut call_ids_in_assistants: HashSet<String> = HashSet::new();
        for msg in messages.iter() {
            if msg.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                if let Some(tc) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    for call in tc {
                        if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                            call_ids_in_assistants.insert(id.to_string());
                        }
                    }
                }
            }
        }

        // Collect tool_call_ids present in tool result messages
        let mut tool_result_ids: HashSet<String> = HashSet::new();
        for msg in messages.iter() {
            if msg.get("role").and_then(|v| v.as_str()) == Some("tool") {
                if let Some(id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                    tool_result_ids.insert(id.to_string());
                }
            }
        }

        messages.retain(|msg| {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            match role {
                "assistant" => {
                    // If this assistant message has tool_calls, all results must be present
                    if let Some(tc) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                        if tc.is_empty() {
                            return true;
                        }
                        tc.iter().all(|call| {
                            call.get("id")
                                .and_then(|v| v.as_str())
                                .map(|id| tool_result_ids.contains(id))
                                .unwrap_or(false)
                        })
                    } else {
                        true
                    }
                }
                "tool" => {
                    // Tool result must have a matching assistant tool_calls entry
                    msg.get("tool_call_id")
                        .and_then(|v| v.as_str())
                        .map(|id| call_ids_in_assistants.contains(id))
                        .unwrap_or(false)
                }
                _ => true,
            }
        });
    }

    /// Compact oversized messages of ANY role to prevent context blowup.
    ///
    /// Replaces the content of any message exceeding `max_chars`
    /// with a truncated version plus a summary. Also strips data: URIs
    /// (base64 images) from any message regardless of size.
    fn compact_large_messages(history: &[Value], max_chars: usize) -> Vec<Value> {
        history
            .iter()
            .map(|msg| {
                let content = match msg.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => return msg.clone(),
                };
                let compacted = compact_content_with_limit(content, max_chars);
                if compacted == content {
                    return msg.clone();
                }
                let mut new_msg = msg.clone();
                new_msg["content"] = Value::String(compacted);
                new_msg
            })
            .collect()
    }

    fn truncate_messages(messages: &[Value], budget: usize) -> Vec<Value> {
        let mut result = Vec::new();
        let mut remaining = budget;

        for msg in messages {
            let tokens = estimate_message_tokens(msg);
            if tokens <= remaining {
                result.push(msg.clone());
                remaining -= tokens;
            } else {
                // Truncate content
                if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                    let max_chars = (remaining * CHARS_PER_TOKEN).max(100);
                    let truncated = if content.len() > max_chars {
                        format!("{}\n[truncated]", &content[..max_chars])
                    } else {
                        content.to_string()
                    };
                    let mut new_msg = msg.clone();
                    new_msg["content"] = Value::String(truncated);
                    remaining = remaining.saturating_sub(estimate_message_tokens(&new_msg));
                    result.push(new_msg);
                } else {
                    result.push(msg.clone());
                    remaining = remaining.saturating_sub(tokens);
                }
            }
        }

        result
    }

    /// Check if content is large enough to warrant file offloading.
    pub fn should_offload(&self, content: &str) -> bool {
        estimate_tokens(content) > OFFLOAD_THRESHOLD_TOKENS
    }

    /// Write large content to a file and return the filename.
    pub fn offload_to_file(&mut self, content: &str, label: &str) -> String {
        let _ = fs::create_dir_all(&self.offload_dir);
        self.offload_counter += 1;
        let filename = format!("{}_{:04}.txt", label, self.offload_counter);
        let filepath = self.offload_dir.join(&filename);
        let _ = fs::write(&filepath, content);
        let path_str = filepath.to_string_lossy().to_string();
        self.offloaded_files.push(path_str.clone());
        tracing::info!(
            "Offloaded {} tokens to {}",
            estimate_tokens(content),
            filepath.display()
        );
        path_str
    }

    /// List of files created by offloading.
    pub fn offloaded_files(&self) -> &[String] {
        &self.offloaded_files
    }
}

// ---------------------------------------------------------------------------
// Compaction helper functions (standalone — called from runner.rs)
// ---------------------------------------------------------------------------

/// Before compaction, extract durable knowledge from messages about to be dropped
/// and write it to the memory system. This is the safety net — even if the
/// compaction summary is lossy, critical context survives in long-term memory.
///
/// Returns the memory entries written (for logging/audit), or empty vec on failure.
pub async fn pre_compaction_memory_flush(
    auth: &crate::auth::AuthriteClient,
    messages_to_drop: &[Value],
    model: &str,
    config: &crate::config::DmConfig,
    memory_store: &crate::memory::store::MemoryStore,
    conversation_id: &str,
) -> Vec<String> {
    if messages_to_drop.is_empty() {
        return Vec::new();
    }

    // Build a message array for the extraction LLM call
    let history_text = messages_to_drop
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role")?.as_str()?;
            let content = msg.get("content")?.as_str()?;
            Some(format!("[{role}]: {content}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let extraction_prompt = concat!(
        "You are about to lose access to the following conversation history due to context ",
        "window compaction. Extract any critical information that should be preserved in ",
        "long-term memory. Focus on:\n\n",
        "- Decisions made and their reasoning\n",
        "- User preferences or requirements expressed\n",
        "- Technical findings or solutions discovered\n",
        "- Action items or commitments\n",
        "- Key facts or data points referenced\n\n",
        "Format each item as a separate paragraph with a clear topic heading.\n",
        "If nothing is worth preserving, respond with \"NOTHING_TO_STORE\".",
    );

    let messages = vec![
        serde_json::json!({"role": "system", "content": extraction_prompt}),
        serde_json::json!({"role": "user", "content": history_text}),
    ];

    let result = match crate::think::think(auth, &messages, model, 1024, Some(0.0), config).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Pre-compaction memory flush LLM call failed: {e}");
            return Vec::new();
        }
    };

    let text = result.text.trim();
    if text == "NOTHING_TO_STORE" || text.is_empty() {
        return Vec::new();
    }

    // Split into paragraphs and store each as a memory entry
    let mut stored = Vec::new();
    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        let title = paragraph.lines().next().unwrap_or("Compaction flush");

        let content = format!("# {title}\n\n{paragraph}");
        let entry = crate::memory::store::MemoryEntry::new(
            crate::memory::store::MemoryCategory::Session,
            content,
            vec!["compaction".to_string(), conversation_id.to_string()],
            format!("compaction-{conversation_id}"),
        );

        match memory_store.store(&entry) {
            Ok(path) => stored.push(path.to_string_lossy().to_string()),
            Err(e) => tracing::warn!("Failed to store compaction memory entry: {e}"),
        }
    }

    stored
}

/// Generate a compaction summary of messages that are about to be dropped.
///
/// Returns the summary text, or None if the LLM call fails.
pub async fn generate_compaction_summary(
    auth: &crate::auth::AuthriteClient,
    messages_to_drop: &[Value],
    model: &str,
    config: &crate::config::DmConfig,
) -> Option<String> {
    if messages_to_drop.is_empty() {
        return None;
    }

    let history_text = messages_to_drop
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role")?.as_str()?;
            let content = msg.get("content")?.as_str()?;
            Some(format!("[{role}]: {content}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let summarization_prompt = concat!(
        "Summarize the following conversation history into key points. ",
        "Focus on: decisions made, problems identified, approaches tried, ",
        "user preferences expressed, and any commitments or next steps. ",
        "Be concise but preserve all actionable context.",
    );

    let messages = vec![
        serde_json::json!({"role": "system", "content": summarization_prompt}),
        serde_json::json!({"role": "user", "content": history_text}),
    ];

    match crate::think::think(auth, &messages, model, 2048, Some(0.0), config).await {
        Ok(r) => {
            let text = r.text.trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Err(e) => {
            tracing::warn!("Compaction summary LLM call failed: {e}");
            None
        }
    }
}
