//! Anthropic Claude provider — wraps the Anthropic messages API.
//!
//! Translates bridge requests (OpenAI-compatible format) to Anthropic's
//! native messages API format and parses responses back.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{LlmProvider, LlmRequest, LlmResponse, ProviderError};
use crate::config::ProviderConfig;

/// Anthropic Claude messages API provider.
pub struct ClaudeProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl ClaudeProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Convert OpenAI-format messages to Anthropic format.
    ///
    /// Anthropic's API separates the system message from user/assistant messages.
    /// Returns (system_prompt, messages).
    fn convert_messages(messages: &[super::ChatMessage]) -> (Option<String>, Vec<Value>) {
        let mut system = None;
        let mut converted = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    // Anthropic uses a separate system parameter
                    if let Some(ref content) = msg.content {
                        system = Some(content.clone());
                    }
                }
                "user" | "assistant" => {
                    let mut m = json!({
                        "role": msg.role,
                    });
                    if let Some(ref content) = msg.content {
                        m["content"] = json!(content);
                    }
                    converted.push(m);
                }
                "tool" => {
                    // Tool results in Anthropic format
                    converted.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                            "content": msg.content.as_deref().unwrap_or(""),
                        }]
                    }));
                }
                _ => {
                    // Unknown role — pass through as user message
                    if let Some(ref content) = msg.content {
                        converted.push(json!({
                            "role": "user",
                            "content": content,
                        }));
                    }
                }
            }
        }

        (system, converted)
    }

    /// Convert OpenAI-format tools to Anthropic format.
    fn convert_tools(tools: &[Value]) -> Vec<Value> {
        tools
            .iter()
            .filter_map(|tool| {
                let func = tool.get("function")?;
                Some(json!({
                    "name": func.get("name")?,
                    "description": func.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "input_schema": func.get("parameters").cloned().unwrap_or(json!({"type": "object", "properties": {}})),
                }))
            })
            .collect()
    }

    /// Convert Anthropic tool_use content blocks to OpenAI tool_calls format.
    fn convert_tool_uses(content: &[Value]) -> Vec<Value> {
        content
            .iter()
            .filter_map(|block| {
                if block.get("type")?.as_str()? == "tool_use" {
                    Some(json!({
                        "id": block.get("id")?,
                        "type": "function",
                        "function": {
                            "name": block.get("name")?,
                            "arguments": serde_json::to_string(
                                block.get("input").unwrap_or(&json!({}))
                            ).unwrap_or_default(),
                        }
                    }))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for ClaudeProvider {
    fn name(&self) -> &str {
        "Anthropic"
    }

    fn supports_model(&self, model: &str) -> bool {
        model.starts_with("claude-")
    }

    fn default_model(&self) -> &str {
        &self.config.default_model
    }

    async fn chat(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        let url = format!("{}/messages", self.config.base_url);

        let (system, messages) = Self::convert_messages(&request.messages);

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens,
        });

        if let Some(system_prompt) = system {
            body["system"] = json!(system_prompt);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        if let Some(ref tools) = request.tools {
            if !tools.is_empty() {
                body["tools"] = json!(Self::convert_tools(tools));
            }
        }

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Request(format!("Anthropic request failed: {e}")))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth(
                "Anthropic API key is invalid or expired".to_string(),
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderError::RateLimited(
                "Anthropic rate limit exceeded".to_string(),
            ));
        }
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Request(format!(
                "Anthropic returned HTTP {status}: {body_text}"
            )));
        }

        let response_body: Value = resp.json().await.map_err(|e| {
            ProviderError::InvalidResponse(format!("Failed to parse response: {e}"))
        })?;

        // Parse Anthropic response format
        let content = response_body
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        // Extract text from text blocks
        let text = content
            .iter()
            .filter_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        // Extract tool_use blocks and convert to OpenAI format
        let tool_calls = Self::convert_tool_uses(&content);

        let stop_reason = response_body
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .unwrap_or("end_turn");

        // Map Anthropic stop reasons to OpenAI finish reasons
        let finish_reason = match stop_reason {
            "end_turn" => "stop",
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            other => other,
        }
        .to_string();

        let usage = response_body.get("usage");
        let prompt_tokens = usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let completion_tokens = usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0);

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
            total_tokens: prompt_tokens + completion_tokens,
            finish_reason,
            tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ChatMessage;

    #[test]
    fn test_supports_model() {
        let config = ProviderConfig {
            api_key: "test".to_string(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            default_model: "claude-sonnet-4-20250514".to_string(),
            max_tokens_limit: 4096,
            available_models: vec!["claude-sonnet-4-20250514".to_string()],
        };
        let provider = ClaudeProvider::new(config);

        assert!(provider.supports_model("claude-sonnet-4-20250514"));
        assert!(provider.supports_model("claude-opus-4-20250514"));
        assert!(!provider.supports_model("gpt-4o"));
        assert!(!provider.supports_model("o3-mini"));
    }

    #[test]
    fn test_convert_messages_extracts_system() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: Some("You are helpful.".to_string()),
                name: None,
                tool_calls: vec![],
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                name: None,
                tool_calls: vec![],
                tool_call_id: None,
            },
        ];

        let (system, converted) = ClaudeProvider::convert_messages(&messages);
        assert_eq!(system.unwrap(), "You are helpful.");
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "Hello");
    }

    #[test]
    fn test_convert_messages_no_system() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: Some("Hello".to_string()),
            name: None,
            tool_calls: vec![],
            tool_call_id: None,
        }];

        let (system, converted) = ClaudeProvider::convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(converted.len(), 1);
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }
            }
        })];

        let converted = ClaudeProvider::convert_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["name"], "get_weather");
        assert_eq!(converted[0]["description"], "Get the weather");
        assert!(converted[0]["input_schema"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_convert_tool_uses() {
        let content = vec![
            json!({
                "type": "text",
                "text": "Let me check the weather."
            }),
            json!({
                "type": "tool_use",
                "id": "toolu_123",
                "name": "get_weather",
                "input": {"location": "London"}
            }),
        ];

        let tool_calls = ClaudeProvider::convert_tool_uses(&content);
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "toolu_123");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
    }

    #[test]
    fn test_convert_tool_result_message() {
        let messages = vec![ChatMessage {
            role: "tool".to_string(),
            content: Some("Sunny, 22C".to_string()),
            name: None,
            tool_calls: vec![],
            tool_call_id: Some("toolu_123".to_string()),
        }];

        let (_system, converted) = ClaudeProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        let content = converted[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "toolu_123");
        assert_eq!(content[0]["content"], "Sunny, 22C");
    }
}
