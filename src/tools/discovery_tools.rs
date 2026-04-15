//! Peer discovery tools — find and verify other BSV agents.
//!
//! Two discoverable tools (NOT always-on):
//!   - `discover_agent`: Find agents by identity key or attributes
//!   - `verify_agent`: Check an agent's certificate status

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::discovery::PeerDiscovery;
use crate::tools::registry::ToolDef;
use crate::wallet::WalletClient;

async fn discover_agent_impl(params: Value, wallet_url: String) -> String {
    let identity_key = params.get("identity_key").and_then(|v| v.as_str());
    let attributes = params.get("attributes").and_then(|v| v.as_object());
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

    if identity_key.is_none() && attributes.is_none() {
        return "Error: at least one of 'identity_key' or 'attributes' is required".to_string();
    }

    let wallet = WalletClient::new(&wallet_url, "http://localhost", 30);
    let discovery = PeerDiscovery::new(wallet);

    if let Some(key) = identity_key {
        if key.len() != 66 {
            return "Error: identity_key must be a 66-char hex compressed pubkey".to_string();
        }
        match discovery.discover_by_key(key, limit).await {
            Ok(peers) => serde_json::to_string_pretty(&json!({
                "query": "identity_key",
                "identity_key": key,
                "results": peers,
                "count": peers.len(),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string()),
            Err(e) => format!("Error: {e}"),
        }
    } else if let Some(attrs) = attributes {
        let attr_map: HashMap<String, String> = attrs
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        if attr_map.is_empty() {
            return "Error: attributes must contain at least one key-value pair".to_string();
        }
        match discovery.discover_by_attributes(&attr_map, limit).await {
            Ok(peers) => serde_json::to_string_pretty(&json!({
                "query": "attributes",
                "attributes": attr_map,
                "results": peers,
                "count": peers.len(),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string()),
            Err(e) => format!("Error: {e}"),
        }
    } else {
        "Error: unexpected parameter state".to_string()
    }
}

async fn verify_agent_impl(params: Value, wallet_url: String) -> String {
    let identity_key = params
        .get("identity_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if identity_key.is_empty() {
        return "Error: identity_key is required".to_string();
    }
    if identity_key.len() != 66 {
        return "Error: identity_key must be a 66-char hex compressed pubkey".to_string();
    }

    let wallet = WalletClient::new(&wallet_url, "http://localhost", 30);
    let discovery = PeerDiscovery::new(wallet);

    match discovery.verify_peer(identity_key).await {
        Ok(verification) => serde_json::to_string_pretty(&verification)
            .unwrap_or_else(|_| "Error: serialization failed".to_string()),
        Err(e) => format!("Error: {e}"),
    }
}

/// Create all peer discovery tool definitions.
///
/// These are discoverable via `search_tools`, NOT always-on.
pub fn all_discovery_tools(wallet_url: String) -> Vec<ToolDef> {
    let discover_url = Arc::new(wallet_url.clone());
    let verify_url = Arc::new(wallet_url);

    vec![
        ToolDef {
            name: "discover_agent".to_string(),
            description: "Discover other BSV agents by identity key or certificate attributes \
                (BRC-56). Use to find agents for delegation, verify sender identity, or discover \
                agents with specific capabilities."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "identity_key": {
                        "type": "string",
                        "description": "Agent's identity key (66-char hex compressed pubkey). Search by specific agent."
                    },
                    "attributes": {
                        "type": "object",
                        "description": "Certificate attributes to search by (e.g. {\"name\": \"AgentName\", \"capabilities\": \"tools\"})"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default 10)"
                    }
                }
            }),
            execute: {
                let url = Arc::clone(&discover_url);
                Box::new(move |params| {
                    let u = url.as_ref().clone();
                    Box::pin(discover_agent_impl(params, u))
                })
            },
            category: "discovery".to_string(),
            cleanup: None,
            deferred: true,
            always_load: false,
            search_hint: Some("Find agents by identity key or attributes (BRC-56)".to_string()),
        },
        ToolDef {
            name: "verify_agent".to_string(),
            description: "Verify another agent's identity by checking their BRC-52 certificates \
                (BRC-56). Returns certificate count, types, certifiers, and whether any are \
                parent-signed (operator-authorized)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "identity_key": {
                        "type": "string",
                        "description": "Agent's identity key to verify (66-char hex compressed pubkey)"
                    }
                },
                "required": ["identity_key"]
            }),
            execute: {
                let url = Arc::clone(&verify_url);
                Box::new(move |params| {
                    let u = url.as_ref().clone();
                    Box::pin(verify_agent_impl(params, u))
                })
            },
            category: "discovery".to_string(),
            cleanup: None,
            deferred: true,
            always_load: false,
            search_hint: Some("Verify an agent's BRC-52 certificates".to_string()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_discovery_tools_creates_two_tools() {
        let tools = all_discovery_tools("http://localhost:3322".into());
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"discover_agent"));
        assert!(names.contains(&"verify_agent"));
    }

    #[test]
    fn test_discovery_tools_are_not_always_on() {
        use crate::tools::registry::ALWAYS_ON_TOOLS;
        assert!(!ALWAYS_ON_TOOLS.contains(&"discover_agent"));
        assert!(!ALWAYS_ON_TOOLS.contains(&"verify_agent"));
    }

    #[test]
    fn test_discovery_tools_category() {
        let tools = all_discovery_tools("http://localhost:3322".into());
        for tool in &tools {
            assert_eq!(tool.category, "discovery");
        }
    }

    #[tokio::test]
    async fn test_discover_agent_requires_key_or_attributes() {
        let result = discover_agent_impl(json!({}), "http://localhost:3322".into()).await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("identity_key"));
    }

    #[tokio::test]
    async fn test_discover_agent_validates_key_length() {
        let result = discover_agent_impl(
            json!({"identity_key": "short"}),
            "http://localhost:3322".into(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("66-char"));
    }

    #[tokio::test]
    async fn test_discover_agent_validates_empty_attributes() {
        let result =
            discover_agent_impl(json!({"attributes": {}}), "http://localhost:3322".into()).await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("at least one"));
    }

    #[tokio::test]
    async fn test_verify_agent_requires_key() {
        let result = verify_agent_impl(json!({}), "http://localhost:3322".into()).await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("identity_key"));
    }

    #[tokio::test]
    async fn test_verify_agent_validates_key_length() {
        let result = verify_agent_impl(
            json!({"identity_key": "too_short"}),
            "http://localhost:3322".into(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("66-char"));
    }
}
