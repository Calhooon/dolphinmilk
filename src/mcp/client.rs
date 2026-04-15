//! MCP client — connects to an external MCP server (e.g. bsv-wallet-mcp) as a
//! child process, fetches typed tool definitions, and proxies tool calls.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::Value;
use tokio::process::Command;

use crate::error::DmError;
use crate::tools::registry::ToolDef;

/// MCP client connected to a child process via stdio.
pub struct McpClient {
    _service: RunningService<RoleClient, ()>,
    peer: Peer<RoleClient>,
}

impl McpClient {
    /// Spawn an MCP server as a child process and connect via stdio.
    ///
    /// `command` is the binary name/path (e.g. "bsv-wallet-mcp").
    /// `env_vars` are extra environment variables to set on the child.
    pub async fn connect(command: &str, env_vars: &[(&str, &str)]) -> Result<Self, DmError> {
        let mut cmd = Command::new(command);
        for (k, v) in env_vars {
            cmd.env(k, v);
        }

        let transport = TokioChildProcess::new(cmd)
            .map_err(|e| DmError::tool(format!("Failed to spawn MCP server '{command}': {e}")))?;

        // () implements ClientHandler (do-nothing handler)
        let service = ()
            .serve(transport)
            .await
            .map_err(|e| DmError::tool(format!("MCP handshake failed: {e}")))?;

        let peer = service.peer().clone();
        tracing::info!("Connected to MCP server: {command}");
        Ok(Self {
            _service: service,
            peer,
        })
    }

    /// Fetch all tool definitions from the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, DmError> {
        self.peer
            .list_all_tools()
            .await
            .map_err(|e| DmError::tool(format!("Failed to list MCP tools: {e}")))
    }

    /// Call a tool by name with the given JSON arguments.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String, DmError> {
        let arguments = match args {
            Value::Object(map) => Some(map),
            _ => None,
        };

        let result: CallToolResult = self
            .peer
            .call_tool(CallToolRequestParams {
                name: Cow::Owned(name.to_string()),
                arguments,
                meta: None,
                task: None,
            })
            .await
            .map_err(|e| DmError::tool(format!("MCP call_tool '{name}' failed: {e}")))?;

        // Extract text from content blocks
        let mut text = String::new();
        for content in result.content {
            if let Some(t) = content.as_text() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
        }

        if result.is_error.unwrap_or(false) && !text.starts_with("Error:") {
            text = format!("Error: {text}");
        }

        Ok(text)
    }
}

/// Convert MCP tool definitions to worm ToolDefs and return them.
///
/// The `client` is shared via `Arc` so each tool closure can call it.
/// `skip_names` lists tool names to NOT register (e.g. "wallet_balance" to
/// avoid colliding with the worm's built-in version).
pub fn mcp_tools_to_tooldefs(
    client: Arc<McpClient>,
    tools: Vec<Tool>,
    skip_names: &[&str],
) -> Vec<ToolDef> {
    let mut defs = Vec::new();

    for tool in tools {
        let name = tool.name.to_string();
        if skip_names.contains(&name.as_str()) {
            tracing::debug!("Skipping MCP tool (collision): {name}");
            continue;
        }

        let description = tool.description.as_deref().unwrap_or("").to_string();
        let hint = crate::tools::registry::derive_hint(&description);

        // The input_schema is already JSON Schema from schemars
        let parameters = Value::Object((*tool.input_schema).clone());

        let client = client.clone();
        let tool_name = name.clone();

        defs.push(ToolDef {
            name,
            description,
            parameters,
            execute: Box::new(move |params| {
                let client = client.clone();
                let tool_name = tool_name.clone();
                Box::pin(async move {
                    match client.call_tool(&tool_name, params).await {
                        Ok(result) => result,
                        Err(e) => format!("Error: {e}"),
                    }
                })
            }),
            category: "wallet".to_string(),
            cleanup: None,
            deferred: true,
            always_load: false,
            search_hint: Some(hint),
        });
    }

    defs
}

/// Connect to the wallet MCP server and return registered tool definitions.
///
/// Returns `Ok(None)` if the MCP binary is not found or connection fails
/// (graceful degradation — the worm continues with `wallet_call` fallback).
pub async fn connect_wallet_mcp(
    command: &str,
    wallet_url: &str,
) -> Result<Option<(Arc<McpClient>, Vec<ToolDef>)>, DmError> {
    // Check if the binary exists
    let which = tokio::process::Command::new("which")
        .arg(command)
        .output()
        .await;

    match which {
        Ok(output) if !output.status.success() => {
            tracing::info!(
                "MCP wallet binary '{command}' not found in PATH — using wallet_call fallback"
            );
            return Ok(None);
        }
        Err(e) => {
            tracing::warn!(
                "Failed to check for MCP binary '{command}': {e} — using wallet_call fallback"
            );
            return Ok(None);
        }
        Ok(_) => {}
    }

    let client = match McpClient::connect(
        command,
        &[
            ("WALLET_URL", wallet_url),
            ("WALLET_ORIGIN", "http://localhost"),
        ],
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to connect to wallet MCP server: {e} — using wallet_call fallback"
            );
            return Ok(None);
        }
    };

    let tools = match client.list_tools().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to list wallet MCP tools: {e} — using wallet_call fallback");
            return Ok(None);
        }
    };

    tracing::info!("Wallet MCP: {} tools available", tools.len());

    let client = Arc::new(client);

    // Skip tools that collide with worm built-ins
    let skip = &["wallet_balance"];
    let defs = mcp_tools_to_tooldefs(client.clone(), tools, skip);

    Ok(Some((client, defs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_tool_name_extraction() {
        let tool = Tool {
            name: Cow::Owned("create_action".to_string()),
            title: None,
            description: Some(Cow::Owned("Create a BSV transaction".to_string())),
            input_schema: Arc::new({
                let mut map = serde_json::Map::new();
                map.insert(
                    "properties".to_string(),
                    serde_json::json!({"description": {"type": "string"}}),
                );
                map.insert("required".to_string(), serde_json::json!(["description"]));
                map
            }),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        };

        assert_eq!(tool.name.as_ref(), "create_action");
        assert!(tool.description.as_deref().unwrap().contains("BSV"));
        assert!(tool.input_schema.contains_key("properties"));
    }

    #[test]
    fn test_skip_colliding_tools() {
        // Verify skip logic without needing a real MCP connection
        let tools = [
            Tool {
                name: Cow::Borrowed("wallet_balance"),
                title: None,
                description: Some(Cow::Borrowed("Balance")),
                input_schema: Arc::new(serde_json::Map::new()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("create_action"),
                title: None,
                description: Some(Cow::Borrowed("Create tx")),
                input_schema: Arc::new(serde_json::Map::new()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ];

        // Can't create a real McpClient without a process, but we can test
        // the filtering logic by checking the names
        let skip = &["wallet_balance"];
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t.name.as_ref())
            .filter(|n| !skip.contains(n))
            .collect();

        assert_eq!(names, vec!["create_action"]);
    }
}
