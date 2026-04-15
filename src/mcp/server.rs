//! MCP server handler that bridges `ToolRegistry` to the MCP protocol.
//!
//! Implements `ServerHandler` from the rmcp crate to expose the worm's tools
//! via JSON-RPC 2.0 over stdio.

use std::borrow::Cow;
use std::sync::Arc;

use serde_json::Value;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use rmcp::{ErrorData as McpError, ServiceExt};

use crate::config::DmConfig;
use crate::tools::registry::ToolRegistry;

/// MCP server backed by a shared `ToolRegistry`.
///
/// The tool set includes sandbox, wallet, memory, messagebox, and 3 x402
/// discovery tools.
#[derive(Clone)]
pub struct DmMcpServer {
    tools: Arc<tokio::sync::RwLock<ToolRegistry>>,
}

impl DmMcpServer {
    /// Create a new MCP server with the standard tool set.
    pub fn new(config: &DmConfig) -> Self {
        use crate::tools::memory_tools::all_memory_tools;
        use crate::tools::messagebox_tools::all_messagebox_tools_with_mb_url;
        use crate::tools::sandbox::all_sandbox_tools;
        use crate::tools::wallet_tools::all_wallet_tools;
        use crate::tools::x402_tools::all_x402_tools;

        let wallet_url = config.wallet.url.clone();
        let workspace = std::path::PathBuf::from("working");
        let memory_dir = workspace.join("memory");

        let mut registry = ToolRegistry::new();

        // Register all tools
        for tool in all_sandbox_tools(workspace.clone(), config.llm.context_window) {
            registry.register(tool);
        }
        for tool in all_wallet_tools(wallet_url.clone()) {
            registry.register(tool);
        }
        for tool in all_memory_tools(memory_dir) {
            registry.register(tool);
        }
        for tool in
            all_messagebox_tools_with_mb_url(wallet_url.clone(), config.messagebox.url.clone())
        {
            registry.register(tool);
        }
        for tool in all_x402_tools(wallet_url, config.x402.registry_url.clone()) {
            registry.register(tool);
        }
        for tool in crate::tools::analytics_tools::all_analytics_tools(workspace.clone()) {
            registry.register(tool);
        }
        registry.register(crate::tools::introspect_tools::create_introspect_tool(
            std::sync::Arc::new(workspace.clone()),
        ));
        for tool in
            crate::tools::overlay_tools::all_overlay_tools(config.overlay.submit_url.clone())
        {
            registry.register(tool);
        }

        // Wrap in Arc<RwLock> for shared access
        let tools = Arc::new(tokio::sync::RwLock::new(registry));

        Self { tools }
    }

    /// Convert our `ToolDef` parameters to an MCP `Tool`.
    fn tool_def_to_mcp(name: &str, description: &str, parameters: &Value) -> Tool {
        // Convert serde_json::Value to JsonObject (serde_json::Map<String, Value>)
        let input_schema = match parameters.as_object() {
            Some(obj) => Arc::new(obj.clone()),
            None => Arc::new(serde_json::Map::new()),
        };

        Tool {
            name: Cow::Owned(name.to_string()),
            title: None,
            description: Some(Cow::Owned(description.to_string())),
            input_schema,
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }
}

#[allow(clippy::manual_async_fn)] // Required by ServerHandler trait signature
impl ServerHandler for DmMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "dolphin-milk".to_string(),
                title: Some("Dolphin Milk Agent".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "BSV autonomous agent with x402 micropayments. \
                 Use discover_services to browse available paid services, \
                 then discover_endpoints to enable service-specific tools."
                    .to_string(),
            ),
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move {
            let registry = self.tools.read().await;
            let tools: Vec<Tool> = registry
                .to_openai_tools()
                .iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    let name = func.get("name")?.as_str()?;
                    let description = func.get("description")?.as_str()?;
                    let parameters = func.get("parameters").unwrap_or(&Value::Null);
                    Some(Self::tool_def_to_mcp(name, description, parameters))
                })
                .collect();

            Ok(ListToolsResult {
                tools,
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            let name = request.name.to_string();

            // Convert MCP arguments (Map) to serde_json::Value
            let arguments: Value = match request.arguments {
                Some(map) => Value::Object(map),
                None => Value::Object(serde_json::Map::new()),
            };

            // Get the tool's execute future while holding the read lock,
            // then drop the lock before awaiting (prevents deadlock when
            // discover_endpoints writes back to the registry).
            let future = {
                let registry = self.tools.read().await;
                let tool = registry.get(&name).ok_or_else(|| {
                    McpError::invalid_params(format!("Unknown tool: {name}"), None)
                })?;
                if !registry.is_allowed(&name) {
                    return Err(McpError::invalid_params(
                        format!("Tool not allowed: {name}"),
                        None,
                    ));
                }
                (tool.execute)(arguments)
            };

            let result = future.await;

            // Check if the result looks like an error
            let is_error = result.starts_with("Error:");

            if is_error {
                Ok(CallToolResult::error(vec![Content::text(result)]))
            } else {
                Ok(CallToolResult::success(vec![Content::text(result)]))
            }
        }
    }

    fn get_tool(&self, _name: &str) -> Option<Tool> {
        // Return None to bypass validation — our tools are dynamic
        None
    }
}

/// Start the MCP server on stdio.
pub async fn run_mcp_server(config: &DmConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Redirect tracing to stderr so it doesn't interfere with stdio MCP protocol
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Starting MCP server on stdio...");

    let server = DmMcpServer::new(config);
    let transport = rmcp::transport::io::stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_def_to_mcp() {
        let params = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        });

        let tool = DmMcpServer::tool_def_to_mcp("test_tool", "A test tool", &params);
        assert_eq!(tool.name.as_ref(), "test_tool");
        assert_eq!(tool.description.as_deref(), Some("A test tool"));
        assert!(tool.input_schema.contains_key("properties"));
    }

    #[test]
    fn test_tool_def_to_mcp_empty_params() {
        let tool = DmMcpServer::tool_def_to_mcp("empty", "No params", &Value::Null);
        assert_eq!(tool.name.as_ref(), "empty");
        assert!(tool.input_schema.is_empty());
    }

    #[test]
    fn test_server_info() {
        let info = ServerInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "dolphin-milk".to_string(),
                title: None,
                version: "0.1.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: None,
        };
        assert_eq!(info.server_info.name, "dolphin-milk");
    }
}
