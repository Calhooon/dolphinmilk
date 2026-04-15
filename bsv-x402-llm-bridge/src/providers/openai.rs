//! OpenAI provider — wraps the OpenAI chat completions API.
//!
//! Translates bridge requests to OpenAI's native format and parses responses.
//! Supports both standard models (gpt-4o, gpt-4o-mini) and reasoning models
//! (o1, o3, gpt-5) which use `max_completion_tokens` instead of `max_tokens`.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{LlmProvider, LlmRequest, LlmResponse, ProviderError};
use crate::config::ProviderConfig;

/// OpenAI chat completions provider.
pub struct OpenAiProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Check if a model is a reasoning model that needs special handling.
    fn is_reasoning_model(model: &str) -> bool {
        model.starts_with("o1")
            || model.starts_with("o3")
            || model.starts_with("o4")
            || model.starts_with("gpt-5")
            || model.starts_with("gpt-4.1")
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "OpenAI"
    }

    fn supports_model(&self, model: &str) -> bool {
        // OpenAI handles all non-Claude models
        !model.starts_with("claude-")
    }

    fn default_model(&self) -> &str {
        &self.config.default_model
    }

    async fn chat(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let is_reasoning = Self::is_reasoning_model(&request.model);

        // Build request body
        let mut body = json!({
            "model": request.model,
            "messages": request.messages,
        });

        // Reasoning models use max_completion_tokens; standard use max_tokens
        if is_reasoning {
            body["max_completion_tokens"] = json!(request.max_tokens);
        } else {
            body["max_tokens"] = json!(request.max_tokens);
            if let Some(temp) = request.temperature {
                body["temperature"] = json!(temp);
            }
        }

        if let Some(ref tools) = request.tools {
            if !tools.is_empty() {
                body["tools"] = json!(tools);
            }
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Request(format!("OpenAI request failed: {e}")))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth(
                "OpenAI API key is invalid or expired".to_string(),
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderError::RateLimited(
                "OpenAI rate limit exceeded".to_string(),
            ));
        }
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Request(format!(
                "OpenAI returned HTTP {status}: {body_text}"
            )));
        }

        let response_body: Value = resp.json().await.map_err(|e| {
            ProviderError::InvalidResponse(format!("Failed to parse response: {e}"))
        })?;

        // Parse the response
        let choice = response_body
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| ProviderError::InvalidResponse("No choices in response".to_string()))?;

        let message = choice
            .get("message")
            .ok_or_else(|| ProviderError::InvalidResponse("No message in choice".to_string()))?;

        let text = message
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let finish_reason = choice
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop")
            .to_string();

        let tool_calls = message
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let usage = response_body.get("usage");
        let prompt_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let completion_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let total_tokens = usage
            .and_then(|u| u.get("total_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(prompt_tokens + completion_tokens);

        let model = response_body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&request.model)
            .to_string();

        Ok(LlmResponse {
            text,
            model,
            prompt_tokens,
            completion_tokens,
            total_tokens,
            finish_reason,
            tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_model_detection() {
        assert!(OpenAiProvider::is_reasoning_model("o1"));
        assert!(OpenAiProvider::is_reasoning_model("o3-mini"));
        assert!(OpenAiProvider::is_reasoning_model("o4-mini"));
        assert!(OpenAiProvider::is_reasoning_model("gpt-5-mini"));
        assert!(OpenAiProvider::is_reasoning_model("gpt-4.1"));
        assert!(!OpenAiProvider::is_reasoning_model("gpt-4o"));
        assert!(!OpenAiProvider::is_reasoning_model("gpt-4o-mini"));
        assert!(!OpenAiProvider::is_reasoning_model(
            "claude-sonnet-4-20250514"
        ));
    }

    #[test]
    fn test_supports_model() {
        let config = ProviderConfig {
            api_key: "test".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            default_model: "gpt-4o-mini".to_string(),
            max_tokens_limit: 4096,
            available_models: vec!["gpt-4o".to_string()],
        };
        let provider = OpenAiProvider::new(config);

        assert!(provider.supports_model("gpt-4o"));
        assert!(provider.supports_model("gpt-4o-mini"));
        assert!(provider.supports_model("o3-mini"));
        assert!(!provider.supports_model("claude-sonnet-4-20250514"));
    }
}
