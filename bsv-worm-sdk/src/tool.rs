//! Tool trait and associated types for building Dolphin Milk tools.
//!
//! A tool is a callable action the agent can invoke. Each tool has a name,
//! description, JSON schema for parameters, and an async execute method.
//!
//! # Example
//!
//! ```rust
//! use bsv_worm_sdk::tool::{Tool, ToolInput, ToolResult, ToolError};
//! use async_trait::async_trait;
//! use serde_json::json;
//!
//! struct MyTool;
//!
//! #[async_trait]
//! impl Tool for MyTool {
//!     fn name(&self) -> &str { "my_tool" }
//!     fn description(&self) -> &str { "Does something useful" }
//!     fn parameters_schema(&self) -> serde_json::Value {
//!         json!({
//!             "type": "object",
//!             "properties": {
//!                 "input": { "type": "string", "description": "The input" }
//!             },
//!             "required": ["input"]
//!         })
//!     }
//!     fn category(&self) -> &str { "custom" }
//!     async fn execute(&self, input: ToolInput) -> Result<ToolResult, ToolError> {
//!         let value = input.params.get("input")
//!             .and_then(|v| v.as_str())
//!             .unwrap_or("nothing");
//!         Ok(ToolResult::text(format!("Got: {value}")))
//!     }
//! }
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Input passed to a tool's execute method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInput {
    /// The parsed parameters from the LLM's tool call.
    pub params: serde_json::Value,
}

impl ToolInput {
    /// Create a new ToolInput from a JSON value.
    pub fn new(params: serde_json::Value) -> Self {
        Self { params }
    }

    /// Get a string parameter by key.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }

    /// Get an integer parameter by key.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.params.get(key).and_then(|v| v.as_i64())
    }

    /// Get a float parameter by key.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.params.get(key).and_then(|v| v.as_f64())
    }

    /// Get a boolean parameter by key.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.params.get(key).and_then(|v| v.as_bool())
    }
}

/// Result returned by a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// The output text to return to the LLM.
    pub output: String,
    /// Whether the tool execution was successful.
    pub success: bool,
}

impl ToolResult {
    /// Create a successful result with text output.
    pub fn text(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            success: true,
        }
    }

    /// Create a failure result with an error message.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            output: message.into(),
            success: false,
        }
    }
}

/// Errors that can occur during tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Invalid input parameters.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Tool execution failed.
    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    /// Required parameter missing.
    #[error("missing required parameter: {0}")]
    MissingParameter(String),

    /// External service error (network, API, etc.).
    #[error("external error: {0}")]
    External(String),
}

/// Trait for implementing a Dolphin Milk tool.
///
/// Tools are the primary way the agent interacts with the world. Each tool
/// is registered in the ToolRegistry and made available to the LLM via
/// function calling.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name for this tool (e.g. "weather_lookup").
    fn name(&self) -> &str;

    /// Human-readable description shown in the LLM system prompt.
    fn description(&self) -> &str;

    /// JSON Schema describing the tool's parameters.
    ///
    /// Must be a valid JSON Schema object. The LLM uses this to generate
    /// valid tool calls.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Tool category (e.g. "sandbox", "wallet", "x402", "custom").
    ///
    /// Categories are used for capability-based access control.
    fn category(&self) -> &str {
        "custom"
    }

    /// Execute the tool with the given input.
    async fn execute(&self, input: ToolInput) -> Result<ToolResult, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_input_accessors() {
        let input = ToolInput::new(json!({
            "city": "Austin",
            "count": 5,
            "temp": 72.5,
            "verbose": true
        }));

        assert_eq!(input.get_str("city"), Some("Austin"));
        assert_eq!(input.get_i64("count"), Some(5));
        assert_eq!(input.get_f64("temp"), Some(72.5));
        assert_eq!(input.get_bool("verbose"), Some(true));
        assert_eq!(input.get_str("missing"), None);
    }

    #[test]
    fn test_tool_result() {
        let ok = ToolResult::text("success");
        assert!(ok.success);
        assert_eq!(ok.output, "success");

        let err = ToolResult::error("something went wrong");
        assert!(!err.success);
        assert_eq!(err.output, "something went wrong");
    }

    #[test]
    fn test_tool_error_display() {
        let e = ToolError::InvalidInput("bad param".into());
        assert_eq!(e.to_string(), "invalid input: bad param");

        let e = ToolError::MissingParameter("city".into());
        assert_eq!(e.to_string(), "missing required parameter: city");
    }

    #[test]
    fn test_tool_input_serialization() {
        let input = ToolInput::new(json!({"key": "value"}));
        let serialized = serde_json::to_string(&input).unwrap();
        let deserialized: ToolInput = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.get_str("key"), Some("value"));
    }
}
