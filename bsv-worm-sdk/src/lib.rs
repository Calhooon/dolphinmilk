//! Dolphin Milk SDK — public API for building plugins.
//!
//! This crate provides the traits and types needed to build Dolphin Milk plugins:
//!
//! - **Tools** — callable actions the agent can invoke (e.g. weather lookup, file search)
//! - **Skills** — behavioral modules that customize agent instructions
//! - **Providers** — LLM backends that implement the think interface
//!
//! # Getting Started
//!
//! Add the SDK as a dependency:
//!
//! ```toml
//! [dependencies]
//! bsv-worm-sdk = { path = "../bsv-worm-sdk" }
//! ```
//!
//! Then implement one of the plugin traits:
//!
//! ```rust
//! use bsv_worm_sdk::prelude::*;
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
//!         json!({ "type": "object", "properties": {} })
//!     }
//!     async fn execute(&self, input: ToolInput) -> Result<ToolResult, ToolError> {
//!         Ok(ToolResult::text("done"))
//!     }
//! }
//! ```

pub mod provider;
pub mod skill;
pub mod tool;
pub mod types;

/// Prelude module — import everything you need with `use bsv_worm_sdk::prelude::*`.
pub mod prelude {
    pub use crate::provider::{Provider, ProviderError, ThinkRequest, ThinkResult};
    pub use crate::skill::{Skill, SkillConfig, SkillError};
    pub use crate::tool::{Tool, ToolError, ToolInput, ToolResult};
    pub use crate::types::{Message, PluginMetadata, ToolDescription};
}

// Re-export key traits at the crate root for convenience.
pub use provider::{Provider, ThinkRequest, ThinkResult};
pub use skill::Skill;
pub use tool::Tool;
