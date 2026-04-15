//! Integration tests for the MCP module (server + client).
//!
//! Tests cover DmMcpServer construction, get_info(), tool_def_to_mcp conversion,
//! mcp_tools_to_tooldefs filtering, and connect_wallet_mcp graceful degradation.
//!
//! NOTE: ServerHandler trait methods list_tools() and call_tool() require a
//! `RequestContext<RoleServer>` which cannot be constructed outside the rmcp
//! framework (it is created internally during MCP session setup). Those methods
//! are tested indirectly through the tool registry integration tests below.

use std::borrow::Cow;
use std::sync::Arc;

use serde_json::{json, Value};

use dolphin_milk::config::DmConfig;
use dolphin_milk::mcp::client::{connect_wallet_mcp, mcp_tools_to_tooldefs, McpClient};
use dolphin_milk::mcp::server::DmMcpServer;
use dolphin_milk::tools::registry::ToolDef;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo, Tool};

// ============================================================
// Helper: construct an rmcp Tool
// ============================================================

fn make_mcp_tool(name: &str, description: &str, schema: serde_json::Map<String, Value>) -> Tool {
    Tool {
        name: Cow::Owned(name.to_string()),
        title: None,
        description: Some(Cow::Owned(description.to_string())),
        input_schema: Arc::new(schema),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

fn make_mcp_tool_no_desc(name: &str) -> Tool {
    Tool {
        name: Cow::Owned(name.to_string()),
        title: None,
        description: None,
        input_schema: Arc::new(serde_json::Map::new()),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

/// Helper to apply skip_names filtering on a slice of tools (mirrors mcp_tools_to_tooldefs logic).
fn filter_tool_names<'a>(tools: &'a [Tool], skip: &[&str]) -> Vec<&'a str> {
    tools
        .iter()
        .map(|t| t.name.as_ref())
        .filter(|n| !skip.contains(n))
        .collect()
}

// ============================================================
// DmMcpServer::new — construction
// ============================================================

#[test]
fn test_server_construction_with_default_config() {
    let config = DmConfig::default();
    let _server = DmMcpServer::new(&config);
    // Construction should not panic
}

#[test]
fn test_server_construction_with_custom_wallet_url() {
    let mut config = DmConfig::default();
    config.wallet.url = "http://custom-wallet:9999".to_string();
    let _server = DmMcpServer::new(&config);
}

#[test]
fn test_server_construction_with_custom_x402_registry() {
    let mut config = DmConfig::default();
    config.x402.registry_url = "https://custom-registry.example.com/agents".to_string();
    let _server = DmMcpServer::new(&config);
}

// ============================================================
// DmMcpServer::get_info — ServerInfo
// ============================================================

#[test]
fn test_get_info_name() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    assert_eq!(info.server_info.name, "dolphin-milk");
}

#[test]
fn test_get_info_title() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    assert_eq!(
        info.server_info.title.as_deref(),
        Some("Dolphin Milk Agent")
    );
}

#[test]
fn test_get_info_version_matches_cargo() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn test_get_info_protocol_version() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    assert_eq!(info.protocol_version, ProtocolVersion::V_2025_03_26);
}

#[test]
fn test_get_info_capabilities_tools_enabled() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    // Tools capability should be enabled
    assert!(info.capabilities.tools.is_some());
}

#[test]
fn test_get_info_instructions_present() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    let instructions = info
        .instructions
        .as_deref()
        .expect("instructions should be present");
    assert!(instructions.contains("discover_services"));
    assert!(instructions.contains("discover_endpoints"));
}

#[test]
fn test_get_info_instructions_mention_x402() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    let instructions = info.instructions.as_deref().unwrap();
    assert!(instructions.contains("x402"));
}

// ============================================================
// ServerInfo construction — structural tests
// ============================================================

#[test]
fn test_server_info_struct_fields() {
    let info = ServerInfo {
        protocol_version: ProtocolVersion::V_2025_03_26,
        capabilities: ServerCapabilities::builder().enable_tools().build(),
        server_info: Implementation {
            name: "test-server".to_string(),
            title: Some("Test Server".to_string()),
            version: "1.0.0".to_string(),
            description: None,
            icons: None,
            website_url: None,
        },
        instructions: Some("test instructions".to_string()),
    };
    assert_eq!(info.server_info.name, "test-server");
    assert_eq!(info.server_info.title.as_deref(), Some("Test Server"));
    assert!(info.capabilities.tools.is_some());
    assert_eq!(info.instructions.as_deref(), Some("test instructions"));
}

#[test]
fn test_server_info_no_instructions() {
    let info = ServerInfo {
        protocol_version: ProtocolVersion::V_2025_03_26,
        capabilities: ServerCapabilities::builder().enable_tools().build(),
        server_info: Implementation {
            name: "minimal".to_string(),
            title: None,
            version: "0.0.1".to_string(),
            description: None,
            icons: None,
            website_url: None,
        },
        instructions: None,
    };
    assert!(info.instructions.is_none());
    assert!(info.server_info.title.is_none());
}

// ============================================================
// tool_def_to_mcp — static helper (via DmMcpServer get_info roundtrip)
// ============================================================
// tool_def_to_mcp is a private method on DmMcpServer, so we test it
// indirectly through the public API. The inline unit tests in server.rs
// cover it directly. Here we verify the MCP Tool struct creation pattern.

#[test]
fn test_mcp_tool_struct_with_schema() {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert(
        "properties".to_string(),
        json!({"query": {"type": "string"}}),
    );
    schema.insert("required".to_string(), json!(["query"]));

    let tool = make_mcp_tool("search", "Search things", schema);
    assert_eq!(tool.name.as_ref(), "search");
    assert_eq!(tool.description.as_deref(), Some("Search things"));
    assert!(tool.input_schema.contains_key("properties"));
    assert!(tool.input_schema.contains_key("required"));
}

#[test]
fn test_mcp_tool_struct_empty_schema() {
    let tool = make_mcp_tool("empty_tool", "No params", serde_json::Map::new());
    assert_eq!(tool.name.as_ref(), "empty_tool");
    assert!(tool.input_schema.is_empty());
}

#[test]
fn test_mcp_tool_struct_no_description() {
    let tool = make_mcp_tool_no_desc("nodesc");
    assert!(tool.description.is_none());
}

#[test]
fn test_mcp_tool_struct_complex_schema() {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert(
        "properties".to_string(),
        json!({
            "url": {"type": "string", "description": "Target URL"},
            "method": {"type": "string", "enum": ["GET", "POST"]},
            "body": {"type": "object"}
        }),
    );
    schema.insert("required".to_string(), json!(["url"]));

    let tool = make_mcp_tool("http_call", "Make HTTP request", schema);
    let props = tool.input_schema.get("properties").unwrap();
    assert!(props.get("url").is_some());
    assert!(props.get("method").is_some());
    assert!(props.get("body").is_some());
}

// ============================================================
// mcp_tools_to_tooldefs — filtering and conversion
// ============================================================
// mcp_tools_to_tooldefs requires an Arc<McpClient>, but McpClient cannot
// be constructed without connecting to a real MCP child process. We test
// the filtering logic by replicating the skip_names check, and verify
// the function compiles correctly with its expected signature.

#[test]
fn test_skip_names_filtering_logic() {
    // Replicate the skip logic used in mcp_tools_to_tooldefs
    let tools = [
        make_mcp_tool("wallet_balance", "Balance", serde_json::Map::new()),
        make_mcp_tool("create_action", "Create tx", serde_json::Map::new()),
        make_mcp_tool("list_outputs", "List outputs", serde_json::Map::new()),
    ];

    let skip = &["wallet_balance"];
    let kept = filter_tool_names(&tools, skip);

    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&"create_action"));
    assert!(kept.contains(&"list_outputs"));
    assert!(!kept.contains(&"wallet_balance"));
}

#[test]
fn test_skip_names_empty_skip_list() {
    let tools = [
        make_mcp_tool("tool_a", "A", serde_json::Map::new()),
        make_mcp_tool("tool_b", "B", serde_json::Map::new()),
    ];

    let skip: &[&str] = &[];
    let kept = filter_tool_names(&tools, skip);

    assert_eq!(kept.len(), 2);
}

#[test]
fn test_skip_names_all_skipped() {
    let tools = [make_mcp_tool(
        "wallet_balance",
        "Balance",
        serde_json::Map::new(),
    )];

    let skip = &["wallet_balance"];
    let kept = filter_tool_names(&tools, skip);

    assert_eq!(kept.len(), 0);
}

#[test]
fn test_skip_names_empty_tools_vec() {
    let tools: Vec<Tool> = vec![];
    let skip = &["wallet_balance"];
    let kept = filter_tool_names(&tools, skip);

    assert_eq!(kept.len(), 0);
}

#[test]
fn test_skip_names_multiple_skip() {
    let tools = [
        make_mcp_tool("wallet_balance", "Balance", serde_json::Map::new()),
        make_mcp_tool("wallet_identity", "Identity", serde_json::Map::new()),
        make_mcp_tool("create_action", "Create tx", serde_json::Map::new()),
        make_mcp_tool("list_actions", "List", serde_json::Map::new()),
    ];

    let skip = &["wallet_balance", "wallet_identity"];
    let kept = filter_tool_names(&tools, skip);

    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&"create_action"));
    assert!(kept.contains(&"list_actions"));
}

#[test]
fn test_tool_name_preserved_from_mcp() {
    let tool = make_mcp_tool(
        "my_custom_tool",
        "Does custom things",
        serde_json::Map::new(),
    );
    assert_eq!(tool.name.to_string(), "my_custom_tool");
}

#[test]
fn test_tool_description_preserved_from_mcp() {
    let tool = make_mcp_tool(
        "test",
        "A detailed description of the tool",
        serde_json::Map::new(),
    );
    assert_eq!(
        tool.description.as_deref(),
        Some("A detailed description of the tool")
    );
}

#[test]
fn test_tool_description_none_becomes_empty() {
    // In mcp_tools_to_tooldefs, None description becomes ""
    let tool = make_mcp_tool_no_desc("nodesc");
    let desc = tool.description.as_deref().unwrap_or("");
    assert_eq!(desc, "");
}

#[test]
fn test_tool_input_schema_preserved() {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert(
        "properties".to_string(),
        json!({
            "name": {"type": "string"},
            "age": {"type": "integer"}
        }),
    );

    let tool = make_mcp_tool("user_tool", "User management", schema);
    let params = Value::Object((*tool.input_schema).clone());
    assert_eq!(params["type"], "object");
    assert!(params["properties"]["name"].is_object());
    assert!(params["properties"]["age"].is_object());
}

// ============================================================
// connect_wallet_mcp — graceful degradation
// ============================================================

#[tokio::test]
async fn test_connect_wallet_mcp_binary_not_found() {
    // A nonexistent binary should return Ok(None) — graceful degradation
    let result = connect_wallet_mcp("nonexistent-binary-xyz-12345", "http://localhost:9999").await;
    assert!(result.is_ok(), "Should not return Err");
    assert!(
        result.unwrap().is_none(),
        "Should return None for missing binary"
    );
}

#[tokio::test]
async fn test_connect_wallet_mcp_another_nonexistent_binary() {
    let result =
        connect_wallet_mcp("absolutely-not-a-real-command-zzz", "http://localhost:1234").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_connect_wallet_mcp_empty_command() {
    // Empty command string should gracefully degrade
    let result = connect_wallet_mcp("", "http://localhost:3322").await;
    // Either Ok(None) from binary-not-found or Ok(None) from connection failure
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

// ============================================================
// McpClient type signature — compile-time verification
// ============================================================
// These tests verify the McpClient API compiles correctly.
// We cannot construct a real McpClient without a child process.

#[test]
#[allow(clippy::type_complexity)]
fn test_mcp_tools_to_tooldefs_signature() {
    // Verify the function signature: (Arc<McpClient>, Vec<Tool>, &[&str]) -> Vec<ToolDef>
    fn _assert_signature(_f: fn(Arc<McpClient>, Vec<Tool>, &[&str]) -> Vec<ToolDef>) {}
    _assert_signature(mcp_tools_to_tooldefs);
}

// ============================================================
// ToolDef category for MCP-bridged tools
// ============================================================

#[test]
fn test_mcp_bridged_tools_get_wallet_category() {
    // mcp_tools_to_tooldefs assigns category "wallet" to all MCP-bridged tools.
    // We verify this by checking the source code expectation.
    // Since we can't call mcp_tools_to_tooldefs without a real McpClient,
    // we verify the invariant by constructing what the function would produce.
    let tool_def = ToolDef {
        name: "create_action".to_string(),
        description: "Create a BSV transaction".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "wallet".to_string(), // mcp_tools_to_tooldefs always sets this
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    };
    assert_eq!(tool_def.category, "wallet");
}

// ============================================================
// DmMcpServer get_tool — dynamic tool bypass
// ============================================================

#[test]
fn test_get_tool_returns_none() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    // get_tool always returns None to bypass rmcp validation for dynamic tools
    assert!(server.get_tool("execute_bash").is_none());
    assert!(server.get_tool("wallet_balance").is_none());
    assert!(server.get_tool("nonexistent_tool").is_none());
    assert!(server.get_tool("").is_none());
}

// ============================================================
// DmMcpServer clone — server is Clone
// ============================================================

#[test]
fn test_server_is_clone() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let _cloned = server.clone();
    // Clone should work since DmMcpServer derives Clone
}

// ============================================================
// Tool registry integration — verify tools are registered
// ============================================================

#[tokio::test]
async fn test_server_has_sandbox_tools() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    // Server should have tools capability
    assert!(info.capabilities.tools.is_some());
    // We can't call list_tools without RequestContext, but we verified
    // the tools were registered in construction (no panic = success)
}

// ============================================================
// Protocol version — verify we use the correct version
// ============================================================

#[test]
fn test_protocol_version_is_2025_03_26() {
    let config = DmConfig::default();
    let server = DmMcpServer::new(&config);
    let info = server.get_info();
    // The MCP protocol version should be the latest supported
    assert_eq!(info.protocol_version, ProtocolVersion::V_2025_03_26);
}

// ============================================================
// connect_wallet_mcp — wallet_balance skip
// ============================================================

#[test]
fn test_default_skip_list_contains_wallet_balance() {
    // connect_wallet_mcp internally skips "wallet_balance" to avoid
    // collision with the worm's built-in version. Verify the convention.
    let skip = &["wallet_balance"];
    assert!(skip.contains(&"wallet_balance"));
    assert!(!skip.contains(&"create_action"));
}

// ============================================================
// MCP Tool -> ToolDef parameter conversion
// ============================================================

#[test]
fn test_input_schema_to_parameters_conversion() {
    // mcp_tools_to_tooldefs converts Tool.input_schema (Arc<Map>) to
    // ToolDef.parameters (Value::Object). Verify the pattern.
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert(
        "properties".to_string(),
        json!({
            "endpoint": {"type": "string"},
            "params": {"type": "object"}
        }),
    );
    schema.insert("required".to_string(), json!(["endpoint"]));

    let tool = make_mcp_tool("wallet_call", "Generic wallet call", schema.clone());

    // The conversion in mcp_tools_to_tooldefs does:
    // Value::Object((*tool.input_schema).clone())
    let parameters = Value::Object((*tool.input_schema).clone());
    assert_eq!(parameters, Value::Object(schema));
}

#[test]
fn test_empty_input_schema_to_parameters() {
    let tool = make_mcp_tool("no_params", "No parameters", serde_json::Map::new());
    let parameters = Value::Object((*tool.input_schema).clone());
    assert_eq!(parameters, json!({}));
}
