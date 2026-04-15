# src/mcp/
> MCP (Model Context Protocol) server and client — exposes worm tools to external AI clients and bridges external MCP servers (e.g. wallet) into the agent's tool registry.

## Overview

Two complementary halves:

1. **Server** (`server.rs`): Bridges the agent's `ToolRegistry` to the MCP protocol using the `rmcp` crate. External clients like Claude Code or Codex connect via `bsv-worm mcp` and get access to all registered tools (sandbox, wallet, memory, messagebox, x402) through standard MCP `tools/list` and `tools/call` requests. The tool set is dynamic: `discover_endpoints` can synthesize new tools at runtime.

2. **Client** (`client.rs`): Connects to an external MCP server as a child process via stdio, fetches its tool definitions, and converts them to worm `ToolDef`s so the agent can call them natively. Primary use case: bridging `bsv-wallet-mcp` to give the agent typed wallet tools without `wallet_call` indirection.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 10 | Module doc comment, re-exports `server` and `client` |
| `server.rs` | 263 | `WormMcpServer` struct, `ServerHandler` impl, `run_mcp_server()` entry point, 3 unit tests |
| `client.rs` | 283 | `McpClient` struct, `mcp_tools_to_tooldefs()`, `connect_wallet_mcp()`, 2 unit tests |

## Key Exports

| Export | Source | Description |
|--------|--------|-------------|
| `WormMcpServer` | `server.rs` | MCP server struct wrapping `Arc<RwLock<ToolRegistry>>`. Implements `rmcp::ServerHandler`. |
| `run_mcp_server(config)` | `server.rs` | Entry point: creates server, binds stdio transport, blocks until client disconnects. |
| `McpClient` | `client.rs` | MCP client connected to a child process via stdio. Methods: `connect()`, `list_tools()`, `call_tool()`. |
| `mcp_tools_to_tooldefs(client, tools, skip_names)` | `client.rs` | Converts MCP `Tool` definitions to worm `ToolDef`s with closures that proxy calls through the shared `Arc<McpClient>`. |
| `connect_wallet_mcp(command, wallet_url)` | `client.rs` | Convenience: spawns wallet MCP binary, connects, fetches tools, returns `Option<(Arc<McpClient>, Vec<ToolDef>)>`. Gracefully degrades to `None` if binary not found or connection fails. |

## Server (`server.rs`)

### `WormMcpServer`

#### Construction (`new`)

Registers all tool categories into a fresh `ToolRegistry`:

1. **Sandbox tools** — `execute_bash`, `file_read`, `file_write`, `file_search`, `web_fetch`
2. **Wallet tools** — `wallet_balance`, `wallet_identity`, `wallet_encrypt`, `wallet_decrypt`, `wallet_call`
3. **Memory tools** — `memory_store`, `memory_search`
4. **MessageBox tools** — `send_message`, `check_inbox`
5. **x402 tools** — `discover_services`, `discover_endpoints`, `x402_call`, `generate_image`, `upload_to_nanostore`
6. **Analytics tools** — cost analytics tools registered via `all_analytics_tools(workspace)`
7. **Introspect tool** — `introspect` tool for self-querying proofs, costs, and tasks

Workspace hardcoded to `working/`. Wallet URL, context window, and x402 registry URL read from `WormConfig`.

#### `ServerHandler` trait impl

| Method | Behavior |
|--------|----------|
| `get_info()` | Returns `ServerInfo` with name `"bsv-worm"`, title `"BSV Worm Agent"`, dynamic version via `CARGO_PKG_VERSION`, protocol version `2025-03-26`, tools capability enabled. `Implementation` fields `description`, `icons`, `website_url` are `None`. Includes instructions string guiding clients toward `discover_services` → `discover_endpoints` workflow. |
| `list_tools()` | Reads registry, converts each `ToolDef` to MCP `Tool` via `to_openai_tools()` intermediate format. Returns all registered tools (no pagination). |
| `call_tool()` | Looks up tool by name, checks allowlist, executes. **Lock discipline**: acquires read lock to get the execute future, drops lock before awaiting — prevents deadlock when `discover_endpoints` writes back to the registry. Results prefixed with `"Error:"` are returned as `CallToolResult::error`. |
| `get_tool()` | Returns `None` unconditionally — bypasses rmcp's built-in tool validation since tools are dynamic. |

#### `tool_def_to_mcp`

Static helper converting `(name, description, parameters)` to rmcp's `Tool` struct. Parameters (`serde_json::Value`) are converted to `JsonObject`; null/non-object params produce an empty schema. All optional `Tool` fields (`title`, `output_schema`, `annotations`, `execution`, `icons`, `meta`) are set to `None`.

### `run_mcp_server`

1. Redirects tracing to stderr (protects stdio protocol stream)
2. Creates `WormMcpServer` from config
3. Binds `rmcp::transport::io::stdio()` transport
4. Calls `server.serve(transport)` then `service.waiting()` (blocks until disconnect)

## Client (`client.rs`)

### `McpClient`

Holds a `RunningService<RoleClient, ()>` and its `Peer<RoleClient>` handle. The `()` implements `ClientHandler` (do-nothing handler — no server-initiated requests expected).

#### `connect(command, env_vars)`

Spawns the MCP server binary as a child process via `TokioChildProcess`, performs the MCP handshake, and returns the connected client. Environment variables (e.g. `WALLET_URL`, `WALLET_ORIGIN`) are passed to the child.

#### `list_tools()`

Calls `peer.list_all_tools()` to fetch all tool definitions from the remote server.

#### `call_tool(name, args)`

Calls a tool by name with JSON arguments. Passes `task: None` and `meta: None` in the request params. Extracts text content from the response's content blocks (joining with newlines). If `is_error` is set on the result, prefixes with `"Error:"` to match worm convention.

### `mcp_tools_to_tooldefs(client, tools, skip_names)`

Converts a `Vec<Tool>` from an MCP server into worm `ToolDef` structs:

- Each `ToolDef.execute` closure captures a clone of `Arc<McpClient>` and proxies `call_tool()` through it.
- `skip_names` filters out tools that collide with worm built-ins (e.g. `"wallet_balance"`).
- All generated tools get `category: "wallet"` and `cleanup: None`.
- `input_schema` from the MCP tool is used directly as `parameters`.

### `connect_wallet_mcp(command, wallet_url)`

High-level convenience for the wallet MCP bridge:

1. Checks if the binary exists via `which` — returns `Ok(None)` if not found (graceful degradation).
2. Calls `McpClient::connect()` with `WALLET_URL` and `WALLET_ORIGIN` env vars.
3. Fetches tools via `list_tools()`.
4. Converts to `ToolDef`s via `mcp_tools_to_tooldefs()`, skipping `"wallet_balance"`.
5. Returns `Some((Arc<McpClient>, Vec<ToolDef>))` on success, `None` on any failure.

All failures log a warning and fall back to `None` — the worm continues with `wallet_call` as the fallback.

## Usage

```bash
# Launch as MCP server (stdio transport)
cargo run -- mcp

# Or with custom config
WORM_WALLET_URL=http://localhost:3322 cargo run -- mcp
```

Clients connect via stdin/stdout using MCP JSON-RPC 2.0. The `bsv-worm mcp` subcommand is dispatched from `main.rs` → `cli.rs`.

The client side is used programmatically — `connect_wallet_mcp()` is called during agent initialization to bridge wallet MCP tools into the tool registry.

## Decisions

- **Graceful degradation for client**: `connect_wallet_mcp()` never fails the agent startup. Missing binary, connection errors, and tool listing failures all return `Ok(None)` with a log warning. The agent falls back to `wallet_call` for wallet operations.
- **Skip list for collision avoidance**: `wallet_balance` is skipped because the worm has its own built-in version with cached balance display. Other wallet tools (e.g. `create_action`, `list_outputs`) come through without collision.
- **Category hardcoded to "wallet"**: All MCP-bridged tools get the `"wallet"` category since the primary (and currently only) use case is wallet tool bridging.
- **`()` as ClientHandler**: The client doesn't need to handle server-initiated requests (notifications, sampling), so the unit type's no-op implementation suffices.
- **Arc<McpClient> shared across closures**: Each `ToolDef.execute` closure needs to call back to the MCP server. `Arc` sharing avoids the cost of multiple connections and ensures a single child process per MCP server.

## Gotchas

- **Lock discipline in server**: `call_tool()` acquires a read lock to get the execute future, then drops the lock before `.await`. This is critical — `discover_endpoints` tool writes back to the registry, which would deadlock if the read lock were held across the await point.
- **`get_tool()` returns None**: This bypasses rmcp's built-in schema validation. Without this, dynamically added tools (from `discover_endpoints`) would fail validation since rmcp wouldn't know about them.
- **Client child process lifetime**: The `_service` field on `McpClient` keeps the child process alive. Dropping the `McpClient` terminates the child.
- **Environment variable injection**: `connect_wallet_mcp` passes `WALLET_URL` and `WALLET_ORIGIN` to the child process. The wallet MCP binary expects these to know which wallet to connect to.
- **Text extraction from content blocks**: `call_tool()` joins multiple text content blocks with newlines. Non-text content blocks (images, etc.) are silently ignored.

## Dependencies

- **rmcp** — MCP protocol implementation (`ServerHandler`, `ClientHandler`, `ServiceExt`, `Peer`, `TokioChildProcess`, transport)
- **tokio::sync::RwLock** — Shared mutable access to `ToolRegistry` (server side)
- **tokio::process::Command** — Spawning child MCP server processes (client side)
- **crate::tools::registry::ToolRegistry** / **ToolDef** — Tool storage and definition types
- **crate::config::WormConfig** — Wallet URL, context window, and other config
- **crate::error::WormError** — Error type for client operations

## Tests

| File | Tests | Coverage |
|------|-------|----------|
| `server.rs` | 3 | `tool_def_to_mcp` conversion, empty params handling, `ServerInfo` construction |
| `client.rs` | 2 | Tool name extraction from MCP `Tool` struct, skip-list collision filtering |

All tests are unit tests that don't require a running MCP server or wallet.

## Related

- `src/tools/registry.rs` — `ToolRegistry` that backs the MCP server and receives client-bridged tools
- `src/tools/` — All tool definitions registered at startup
- `src/tools/wallet_tools.rs` — Built-in wallet tools (including `wallet_call` fallback)
- `src/cli.rs` — `Mcp` subcommand definition
- `src/main.rs` — `run_mcp_server()` call site
- Parent: [`src/CLAUDE.md`](../CLAUDE.md)
