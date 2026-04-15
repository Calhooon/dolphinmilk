//! MCP (Model Context Protocol) server — expose worm tools to Claude Code/Codex.
//!
//! Bridges the `ToolRegistry` to MCP's JSON-RPC 2.0 protocol over stdio.
//! Tools are served dynamically: static tools appear immediately, and when
//! `discover_endpoints` is called, newly synthesized tools appear via
//! `notifications/tools/list_changed`.

pub mod client;
pub mod server;
