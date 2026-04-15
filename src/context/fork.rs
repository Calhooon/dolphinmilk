//! Context fork — isolate memory-intensive operations from the main context.
//!
//! When a skill is tagged `memory_intensive: true` in its YAML frontmatter,
//! the runner can fork the current `PromptContext` and conversation messages,
//! run a heavy operation (e.g., full memory dump, large recall) in the clone,
//! and extract results without polluting the main context window.

use serde_json::Value;

use crate::context::prompt::PromptContext;

/// A forked context that isolates memory-intensive operations.
///
/// The fork clones the current `PromptContext` and conversation messages,
/// runs operations on the cloned copy, and provides access to extracted
/// results without modifying the original context.
pub struct ContextFork {
    /// Cloned prompt context (safe to mutate without affecting main).
    pub context: PromptContext,
    /// Cloned conversation messages (OpenAI format).
    messages: Vec<Value>,
    /// Results extracted from the forked operation.
    results: Vec<ForkResult>,
}

/// A result extracted from a forked context operation.
#[derive(Debug, Clone)]
pub struct ForkResult {
    /// Label describing the operation that produced this result.
    pub label: String,
    /// The extracted content (summary, data, etc.).
    pub content: String,
}

impl ContextFork {
    /// Create a new fork from an existing `PromptContext` and message history.
    ///
    /// Both are deep-cloned so the fork is fully independent of the original.
    pub fn new(context: &PromptContext, messages: &[Value]) -> Self {
        Self {
            context: context.clone(),
            messages: messages.to_vec(),
            results: Vec::new(),
        }
    }

    /// Get the forked messages (read-only access).
    pub fn messages(&self) -> &[Value] {
        &self.messages
    }

    /// Append a message to the forked context (does not affect original).
    pub fn push_message(&mut self, message: Value) {
        self.messages.push(message);
    }

    /// Replace the memory summary in the forked context with a larger one.
    ///
    /// This is the typical use case: inject a full memory dump into the fork
    /// without bloating the main context.
    pub fn inject_memory_summary(&mut self, summary: String) {
        self.context.memory_summary = summary;
    }

    /// Record an extracted result from the forked operation.
    pub fn record_result(&mut self, label: impl Into<String>, content: impl Into<String>) {
        self.results.push(ForkResult {
            label: label.into(),
            content: content.into(),
        });
    }

    /// Get all extracted results.
    pub fn results(&self) -> &[ForkResult] {
        &self.results
    }

    /// Consume the fork and return extracted results.
    pub fn into_results(self) -> Vec<ForkResult> {
        self.results
    }

    /// Extract a compact summary of all results (for injection back into main context).
    ///
    /// Concatenates all result contents with labels, suitable for embedding
    /// as a single system message in the main context.
    pub fn summary(&self) -> String {
        if self.results.is_empty() {
            return String::new();
        }
        self.results
            .iter()
            .map(|r| format!("[{}] {}", r.label, r.content))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Number of messages in the forked context.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

/// Check if a skill's YAML frontmatter indicates it is memory-intensive.
///
/// Looks for `memory_intensive: true` in the raw YAML string.
pub fn is_memory_intensive_skill(frontmatter_yaml: &str) -> bool {
    // Parse YAML and check the flag
    if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter_yaml) {
        yaml.get("memory_intensive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::prompt::PromptContext;

    fn test_ctx() -> PromptContext {
        PromptContext {
            identity_key: "test-key".to_string(),
            balance_sats: 1000,
            model: "test-model".to_string(),
            tools: vec![],
            memory_summary: "short summary".to_string(),
            available_files: vec![],
            task: "test task".to_string(),
            budget_remaining: 500,
            low_power: false,
            inbox_count: 0,
            skills_section: String::new(),
            workspace_path: String::new(),
            certificate_info: None,
            has_external_messages: false,
            identity_soul: None,
            auto_recall_ids: vec![],
            basket_health: std::collections::HashMap::new(),
            spendable_output_count: 0,
            instructions: String::new(),
            env_snapshot: None,
            working_memory_section: String::new(),
        }
    }

    #[test]
    fn test_fork_preserves_original() {
        let ctx = test_ctx();
        let messages = vec![serde_json::json!({"role": "user", "content": "hello"})];
        let mut fork = ContextFork::new(&ctx, &messages);

        // Mutate the fork
        fork.context.memory_summary = "HUGE DUMP".to_string();
        fork.push_message(serde_json::json!({"role": "system", "content": "extra"}));

        // Original unchanged
        assert_eq!(ctx.memory_summary, "short summary");
        assert_eq!(messages.len(), 1);

        // Fork changed
        assert_eq!(fork.context.memory_summary, "HUGE DUMP");
        assert_eq!(fork.message_count(), 2);
    }

    #[test]
    fn test_fork_results() {
        let ctx = test_ctx();
        let mut fork = ContextFork::new(&ctx, &[]);

        fork.record_result("memory_dump", "Found 42 relevant entries about BSV");
        fork.record_result("analysis", "Key themes: transactions, fees, proofs");

        assert_eq!(fork.results().len(), 2);
        assert_eq!(fork.results()[0].label, "memory_dump");
        assert!(fork.summary().contains("memory_dump"));
        assert!(fork.summary().contains("Key themes"));
    }

    #[test]
    fn test_fork_empty_summary() {
        let ctx = test_ctx();
        let fork = ContextFork::new(&ctx, &[]);
        assert_eq!(fork.summary(), "");
    }

    #[test]
    fn test_inject_memory_summary() {
        let ctx = test_ctx();
        let mut fork = ContextFork::new(&ctx, &[]);

        let big_summary = "entry1\nentry2\nentry3\n".repeat(100);
        fork.inject_memory_summary(big_summary.clone());

        assert_eq!(fork.context.memory_summary, big_summary);
        // Original unchanged
        assert_eq!(ctx.memory_summary, "short summary");
    }

    #[test]
    fn test_into_results() {
        let ctx = test_ctx();
        let mut fork = ContextFork::new(&ctx, &[]);
        fork.record_result("test", "data");
        let results = fork.into_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "data");
    }

    #[test]
    fn test_is_memory_intensive_skill_true() {
        let yaml = "name: deep-recall\nmemory_intensive: true\nauto_activate: false";
        assert!(is_memory_intensive_skill(yaml));
    }

    #[test]
    fn test_is_memory_intensive_skill_false() {
        let yaml = "name: simple\nauto_activate: true";
        assert!(!is_memory_intensive_skill(yaml));
    }

    #[test]
    fn test_is_memory_intensive_skill_invalid_yaml() {
        let yaml = "not valid yaml: [[[";
        assert!(!is_memory_intensive_skill(yaml));
    }
}
