//! Echo Provider — example bsv-worm provider plugin.
//!
//! Demonstrates how to implement the `Provider` trait from `bsv-worm-sdk`.
//! Echoes the last user message back as the LLM response. Useful for testing
//! the agent loop without incurring real LLM costs.

use async_trait::async_trait;
use bsv_worm_sdk::provider::{Provider, ProviderError, ThinkRequest, ThinkResult};
use std::time::Instant;

/// Model name for the echo provider.
pub const ECHO_MODEL: &str = "echo-v1";

/// A mock LLM provider that echoes input.
///
/// Extracts the last user message from the conversation and returns it
/// as the assistant response. Useful for testing pipelines without
/// calling a real LLM.
pub struct EchoProvider {
    /// Optional prefix added to echoed responses.
    prefix: String,
}

impl EchoProvider {
    /// Create a new EchoProvider with the default prefix.
    pub fn new() -> Self {
        Self {
            prefix: "Echo: ".into(),
        }
    }

    /// Create an EchoProvider with a custom prefix.
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    /// Extract the last user message from the conversation.
    fn last_user_message(request: &ThinkRequest) -> Option<String> {
        request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.clone())
    }
}

impl Default for EchoProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn models(&self) -> Vec<String> {
        vec![ECHO_MODEL.into()]
    }

    async fn think(&self, request: ThinkRequest) -> Result<ThinkResult, ProviderError> {
        if !self.supports_model(&request.model) {
            return Err(ProviderError::UnsupportedModel(request.model.clone()));
        }

        let t0 = Instant::now();

        let user_text = Self::last_user_message(&request).unwrap_or_default();

        if user_text.is_empty() {
            return Err(ProviderError::RequestFailed(
                "no user message found in conversation".into(),
            ));
        }

        let response_text = format!("{}{}", self.prefix, user_text);

        // Approximate token counts (1 token ~ 4 chars).
        let prompt_tokens = request
            .messages
            .iter()
            .map(|m| m.content.as_deref().unwrap_or("").len() as u64 / 4 + 1)
            .sum::<u64>();
        let completion_tokens = response_text.len() as u64 / 4 + 1;

        Ok(ThinkResult {
            text: response_text,
            model: ECHO_MODEL.into(),
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            finish_reason: "stop".into(),
            duration_ms: t0.elapsed().as_millis() as u64,
            tool_calls: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bsv_worm_sdk::types::Message;

    #[tokio::test]
    async fn test_echo_basic() {
        let provider = EchoProvider::new();
        let request = ThinkRequest::new(vec![Message::user("Hello, world!")], ECHO_MODEL);

        let result = provider.think(request).await.unwrap();

        assert_eq!(result.text, "Echo: Hello, world!");
        assert_eq!(result.model, ECHO_MODEL);
        assert_eq!(result.finish_reason, "stop");
        assert!(result.total_tokens > 0);
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_echo_custom_prefix() {
        let provider = EchoProvider::with_prefix("Reply: ");
        let request = ThinkRequest::new(vec![Message::user("test message")], ECHO_MODEL);

        let result = provider.think(request).await.unwrap();
        assert_eq!(result.text, "Reply: test message");
    }

    #[tokio::test]
    async fn test_echo_multi_message() {
        let provider = EchoProvider::new();
        let messages = vec![
            Message::system("You are helpful."),
            Message::user("First message"),
            Message::assistant("Got it."),
            Message::user("Second message"),
        ];
        let request = ThinkRequest::new(messages, ECHO_MODEL);

        let result = provider.think(request).await.unwrap();
        // Should echo the LAST user message
        assert_eq!(result.text, "Echo: Second message");
    }

    #[tokio::test]
    async fn test_echo_unsupported_model() {
        let provider = EchoProvider::new();
        let request = ThinkRequest::new(vec![Message::user("test")], "gpt-99");

        let result = provider.think(request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::UnsupportedModel(model) => assert_eq!(model, "gpt-99"),
            other => panic!("Expected UnsupportedModel, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_echo_no_user_message() {
        let provider = EchoProvider::new();
        let request = ThinkRequest::new(vec![Message::system("system only")], ECHO_MODEL);

        let result = provider.think(request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::RequestFailed(msg) => assert!(msg.contains("no user message")),
            other => panic!("Expected RequestFailed, got: {other}"),
        }
    }

    #[test]
    fn test_provider_metadata() {
        let provider = EchoProvider::new();
        assert_eq!(provider.name(), "echo");
        assert_eq!(provider.models(), vec!["echo-v1".to_string()]);
        assert!(provider.supports_model(ECHO_MODEL));
        assert!(!provider.supports_model("gpt-4"));
    }
}
