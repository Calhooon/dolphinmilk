//! Upstream LLM provider implementations.
//!
//! Each provider module wraps a specific LLM API (OpenAI, Anthropic) and
//! translates between the bridge's internal request/response format and the
//! provider's native API format.

pub mod claude;
pub mod openai;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::ProviderConfig;

/// A chat message in the bridge's internal format (OpenAI-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Request to an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
}

/// Response from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// The generated text content.
    pub text: String,
    /// The model that was actually used.
    pub model: String,
    /// Number of prompt (input) tokens.
    pub prompt_tokens: u64,
    /// Number of completion (output) tokens.
    pub completion_tokens: u64,
    /// Total tokens used.
    pub total_tokens: u64,
    /// Why the model stopped generating (e.g., "stop", "length", "tool_calls").
    pub finish_reason: String,
    /// Tool calls requested by the model (empty if none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<Value>,
}

/// Error from an LLM provider.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Provider request failed: {0}")]
    Request(String),
    #[error("Provider returned invalid response: {0}")]
    InvalidResponse(String),
    #[error("Provider authentication failed: {0}")]
    Auth(String),
    #[error("Provider rate limited: {0}")]
    RateLimited(String),
    #[error("Model not available: {0}")]
    ModelNotAvailable(String),
}

/// Trait for upstream LLM providers.
///
/// Each provider implements this trait to handle the specifics of its API
/// format, authentication, and response parsing.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Human-readable name of the provider (e.g., "OpenAI", "Anthropic").
    fn name(&self) -> &str;

    /// Check if this provider can serve the given model.
    fn supports_model(&self, model: &str) -> bool;

    /// Get the default model for this provider.
    fn default_model(&self) -> &str;

    /// Send a chat completion request to the provider.
    async fn chat(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError>;
}

/// Resolve which provider should handle a given model name.
pub fn resolve_provider<'a>(
    model: &str,
    providers: &'a [Box<dyn LlmProvider>],
) -> Option<&'a dyn LlmProvider> {
    providers
        .iter()
        .find(|p| p.supports_model(model))
        .map(|p| p.as_ref())
}

/// Create all configured providers from the bridge config.
pub fn create_providers(
    openai_config: Option<&ProviderConfig>,
    claude_config: Option<&ProviderConfig>,
) -> Vec<Box<dyn LlmProvider>> {
    let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();

    if let Some(config) = openai_config {
        providers.push(Box::new(openai::OpenAiProvider::new(config.clone())));
    }

    if let Some(config) = claude_config {
        providers.push(Box::new(claude::ClaudeProvider::new(config.clone())));
    }

    providers
}
