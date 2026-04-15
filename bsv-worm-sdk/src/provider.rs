//! Provider trait and associated types for building LLM providers.
//!
//! A provider wraps an LLM backend (OpenAI, Claude, local model, etc.) and
//! exposes a uniform `think` interface. Providers handle authentication,
//! request formatting, and response parsing.
//!
//! # Example
//!
//! ```rust
//! use bsv_worm_sdk::provider::{Provider, ProviderError, ThinkRequest, ThinkResult};
//! use bsv_worm_sdk::types::Message;
//! use async_trait::async_trait;
//!
//! struct MockProvider;
//!
//! #[async_trait]
//! impl Provider for MockProvider {
//!     fn name(&self) -> &str { "mock" }
//!     fn models(&self) -> Vec<String> { vec!["mock-v1".into()] }
//!     async fn think(&self, request: ThinkRequest) -> Result<ThinkResult, ProviderError> {
//!         Ok(ThinkResult {
//!             text: format!("Echo: {}", request.messages.last()
//!                 .and_then(|m| m.content.as_deref())
//!                 .unwrap_or("")),
//!             model: request.model,
//!             prompt_tokens: 10,
//!             completion_tokens: 5,
//!             total_tokens: 15,
//!             finish_reason: "stop".into(),
//!             duration_ms: 1,
//!             tool_calls: Vec::new(),
//!         })
//!     }
//! }
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::Message;

/// Request to an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkRequest {
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Model identifier (e.g. "gpt-5-mini", "claude-sonnet-4-6").
    pub model: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature (0.0 - 2.0). None uses the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Tool definitions in OpenAI function calling format.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<serde_json::Value>,
}

impl ThinkRequest {
    /// Create a simple request with messages and a model.
    pub fn new(messages: Vec<Message>, model: impl Into<String>) -> Self {
        Self {
            messages,
            model: model.into(),
            max_tokens: 4096,
            temperature: None,
            tools: Vec::new(),
        }
    }

    /// Set the maximum tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Set the temperature.
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set tool definitions.
    pub fn with_tools(mut self, tools: Vec<serde_json::Value>) -> Self {
        self.tools = tools;
        self
    }
}

/// Result returned by a provider's think method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkResult {
    /// The generated text response.
    pub text: String,
    /// The model that was actually used.
    pub model: String,
    /// Number of tokens in the prompt.
    pub prompt_tokens: u64,
    /// Number of tokens generated.
    pub completion_tokens: u64,
    /// Total tokens used.
    pub total_tokens: u64,
    /// Reason the generation stopped (e.g. "stop", "tool_calls", "length").
    pub finish_reason: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Tool calls requested by the LLM (OpenAI function calling format).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<serde_json::Value>,
}

/// Errors that can occur during provider operations.
#[derive(Debug, Error)]
pub enum ProviderError {
    /// Authentication failed.
    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// Request to the LLM backend failed.
    #[error("request failed: {0}")]
    RequestFailed(String),

    /// Response parsing failed.
    #[error("invalid response: {0}")]
    InvalidResponse(String),

    /// Model not supported by this provider.
    #[error("unsupported model: {0}")]
    UnsupportedModel(String),

    /// Rate limit exceeded.
    #[error("rate limited: {0}")]
    RateLimited(String),
}

/// Trait for implementing an LLM provider.
///
/// Providers wrap LLM backends and present a uniform interface. The agent
/// loop calls `think()` to generate responses, and the provider handles
/// all backend-specific details (auth, format conversion, error handling).
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name (e.g. "openai", "claude", "local").
    fn name(&self) -> &str;

    /// List of model identifiers this provider supports.
    fn models(&self) -> Vec<String>;

    /// Check if a specific model is supported.
    fn supports_model(&self, model: &str) -> bool {
        self.models().iter().any(|m| m == model)
    }

    /// Send a think request and return the result.
    async fn think(&self, request: ThinkRequest) -> Result<ThinkResult, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_think_request_builder() {
        let req = ThinkRequest::new(
            vec![Message::user("Hello")],
            "gpt-5-mini",
        )
        .with_max_tokens(2048)
        .with_temperature(0.7);

        assert_eq!(req.model, "gpt-5-mini");
        assert_eq!(req.max_tokens, 2048);
        assert_eq!(req.temperature, Some(0.7));
        assert_eq!(req.messages.len(), 1);
        assert!(req.tools.is_empty());
    }

    #[test]
    fn test_think_result_serialization() {
        let result = ThinkResult {
            text: "Hello!".into(),
            model: "test-v1".into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            finish_reason: "stop".into(),
            duration_ms: 42,
            tool_calls: Vec::new(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Hello!"));
        assert!(json.contains("test-v1"));

        // tool_calls should be skipped when empty
        assert!(!json.contains("tool_calls"));
    }

    #[test]
    fn test_provider_error_display() {
        let e = ProviderError::AuthFailed("bad token".into());
        assert_eq!(e.to_string(), "authentication failed: bad token");

        let e = ProviderError::UnsupportedModel("gpt-99".into());
        assert_eq!(e.to_string(), "unsupported model: gpt-99");

        let e = ProviderError::RateLimited("retry after 60s".into());
        assert_eq!(e.to_string(), "rate limited: retry after 60s");
    }

    #[test]
    fn test_think_request_with_tools() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": { "type": "object", "properties": {} }
            }
        })];

        let req = ThinkRequest::new(vec![Message::user("weather?")], "test")
            .with_tools(tools);

        assert_eq!(req.tools.len(), 1);
    }
}
