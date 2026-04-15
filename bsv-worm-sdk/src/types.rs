//! Shared types used across tool, skill, and provider interfaces.

use serde::{Deserialize, Serialize};

/// Message in a conversation history, used by providers.
///
/// Follows the OpenAI chat completions message format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message author: "system", "user", "assistant", or "tool".
    pub role: String,
    /// Text content of the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Tool calls requested by the assistant (OpenAI function calling format).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<serde_json::Value>,
    /// Tool call ID (present when role is "tool").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// Create a tool result message.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Description of a tool for registration and prompt generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescription {
    /// Unique tool name (e.g. "weather_lookup").
    pub name: String,
    /// Human-readable description for the LLM system prompt.
    pub description: String,
    /// Tool category (e.g. "sandbox", "wallet", "x402").
    pub category: String,
}

/// Metadata about a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// Plugin name.
    pub name: String,
    /// Plugin version (semver).
    pub version: String,
    /// Short description.
    pub description: String,
    /// Plugin author.
    pub author: String,
}

impl PluginMetadata {
    /// Create new plugin metadata.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
            author: author.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_constructors() {
        let sys = Message::system("You are helpful.");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content.as_deref(), Some("You are helpful."));

        let user = Message::user("Hello");
        assert_eq!(user.role, "user");
        assert_eq!(user.content.as_deref(), Some("Hello"));

        let asst = Message::assistant("Hi there");
        assert_eq!(asst.role, "assistant");
        assert_eq!(asst.content.as_deref(), Some("Hi there"));

        let tool = Message::tool_result("call_123", "result data");
        assert_eq!(tool.role, "tool");
        assert_eq!(tool.tool_call_id.as_deref(), Some("call_123"));
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("test");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "test");
        // Empty tool_calls should be skipped
        assert!(json.get("tool_calls").is_none());
        // None tool_call_id should be skipped
        assert!(json.get("tool_call_id").is_none());
    }

    #[test]
    fn test_plugin_metadata() {
        let meta = PluginMetadata::new("test-plugin", "0.1.0", "A test", "tester");
        assert_eq!(meta.name, "test-plugin");
        assert_eq!(meta.version, "0.1.0");
    }

    #[test]
    fn test_tool_description() {
        let desc = ToolDescription {
            name: "my_tool".into(),
            description: "Does things".into(),
            category: "sandbox".into(),
        };
        let json = serde_json::to_string(&desc).unwrap();
        assert!(json.contains("my_tool"));
    }
}
