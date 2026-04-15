//! Fleet management tools — status aggregation and fleet coordination.
//!
//! One discoverable tool (NOT always-on):
//!   - `fleet_status`: aggregate fleet member status

pub mod status;
pub mod types;

use serde_json::{json, Value};

use crate::tools::registry::ToolDef;

use self::status::{aggregate_fleet_status, format_fleet_status};
use self::types::{AgentStatus, FleetMember};

/// Parse fleet member data from the tool parameters.
///
/// Expects a JSON array of member objects under the `members` key. Each member
/// object should contain: agent_id, name, template, status, current_task,
/// budget_remaining_sats, budget_spent_sats, last_heartbeat, endpoint.
///
/// If no `members` key is provided, returns an empty fleet status (useful for
/// checking that the fleet system is operational).
fn parse_members_from_params(params: &Value) -> Vec<FleetMember> {
    let members_arr = match params.get("members").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    members_arr
        .iter()
        .filter_map(|m| {
            let agent_id = m.get("agent_id")?.as_str()?.to_string();
            let name = m
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed")
                .to_string();
            let template = m
                .get("template")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let status = match m.get("status").and_then(|v| v.as_str()) {
                Some("Running") => AgentStatus::Running,
                Some("Idle") => AgentStatus::Idle,
                Some("Stopped") => AgentStatus::Stopped,
                Some("Unknown") => AgentStatus::Unknown,
                Some(other) if other.starts_with("Error") => {
                    let detail = other.strip_prefix("Error: ").unwrap_or(other);
                    AgentStatus::Error(detail.to_string())
                }
                _ => AgentStatus::Unknown,
            };
            let current_task = m
                .get("current_task")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let budget_remaining_sats = m
                .get("budget_remaining_sats")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let budget_spent_sats = m
                .get("budget_spent_sats")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let last_heartbeat = m
                .get("last_heartbeat")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok())
                .unwrap_or_else(chrono::Utc::now);
            let endpoint = m
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            Some(FleetMember {
                agent_id,
                name,
                template,
                status,
                current_task,
                budget_remaining_sats,
                budget_spent_sats,
                last_heartbeat,
                endpoint,
            })
        })
        .collect()
}

async fn fleet_status_impl(params: Value) -> String {
    let members = parse_members_from_params(&params);
    let now = chrono::Utc::now();
    let status = aggregate_fleet_status(&members, now);
    let display = format_fleet_status(&status);

    // Return both structured JSON and human-readable text
    match serde_json::to_string(&json!({
        "fleet_status": status,
        "display": display,
    })) {
        Ok(json) => json,
        Err(_) => "Error: serialization failed".to_string(),
    }
}

/// Create all fleet management tool definitions.
///
/// These are discoverable via `search_tools`, NOT always-on.
pub fn all_fleet_tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "fleet_status".to_string(),
        description: "Aggregate fleet member status into a summary. Pass an array of member \
            objects to get fleet-wide health, budget totals, and active task count. \
            Members with stale heartbeats (>5 min) are marked Unknown."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "members": {
                    "type": "array",
                    "description": "Array of fleet member objects. Each member: { agent_id, name, template, status, current_task, budget_remaining_sats, budget_spent_sats, last_heartbeat, endpoint }",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent_id": {
                                "type": "string",
                                "description": "Agent identity key (66-char hex pubkey)"
                            },
                            "name": {
                                "type": "string",
                                "description": "Human-readable agent name"
                            },
                            "template": {
                                "type": "string",
                                "description": "Agent template (e.g. researcher, writer)"
                            },
                            "status": {
                                "type": "string",
                                "description": "Agent status: Running, Idle, Error: <msg>, Stopped, Unknown"
                            },
                            "current_task": {
                                "type": "string",
                                "description": "Current task description (null if none)"
                            },
                            "budget_remaining_sats": {
                                "type": "integer",
                                "description": "Remaining budget in satoshis"
                            },
                            "budget_spent_sats": {
                                "type": "integer",
                                "description": "Budget spent in satoshis"
                            },
                            "last_heartbeat": {
                                "type": "string",
                                "description": "ISO-8601 timestamp of last heartbeat"
                            },
                            "endpoint": {
                                "type": "string",
                                "description": "Agent endpoint or identity key"
                            }
                        },
                        "required": ["agent_id"]
                    }
                }
            }
        }),
        execute: Box::new(move |params| Box::pin(fleet_status_impl(params))),
        category: "fleet".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("Multi-agent fleet status aggregation".to_string()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_fleet_tools_creates_one_tool() {
        let tools = all_fleet_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "fleet_status");
    }

    #[test]
    fn test_fleet_tools_category() {
        let tools = all_fleet_tools();
        for tool in &tools {
            assert_eq!(tool.category, "fleet");
        }
    }

    #[test]
    fn test_fleet_tools_are_not_always_on() {
        use crate::tools::registry::ALWAYS_ON_TOOLS;
        assert!(!ALWAYS_ON_TOOLS.contains(&"fleet_status"));
    }

    #[tokio::test]
    async fn test_fleet_status_empty_params() {
        let result = fleet_status_impl(json!({})).await;
        assert!(result.contains("fleet_status"));
        assert!(result.contains("\"active_tasks\":0"));
        assert!(result.contains("No fleet members"));
    }

    #[tokio::test]
    async fn test_fleet_status_with_members() {
        let now = chrono::Utc::now().to_rfc3339();
        let result = fleet_status_impl(json!({
            "members": [
                {
                    "agent_id": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                    "name": "researcher-1",
                    "template": "researcher",
                    "status": "Running",
                    "current_task": "Analyzing fees",
                    "budget_remaining_sats": 45000,
                    "budget_spent_sats": 5000,
                    "last_heartbeat": now,
                    "endpoint": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                }
            ]
        }))
        .await;

        assert!(result.contains("researcher-1"));
        assert!(result.contains("\"active_tasks\":1"));
        assert!(result.contains("45000"));
    }

    #[tokio::test]
    async fn test_fleet_status_mixed_states() {
        let now = chrono::Utc::now().to_rfc3339();
        let result = fleet_status_impl(json!({
            "members": [
                {
                    "agent_id": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                    "name": "runner",
                    "status": "Running",
                    "current_task": "Task A",
                    "budget_remaining_sats": 40000,
                    "budget_spent_sats": 10000,
                    "last_heartbeat": now
                },
                {
                    "agent_id": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
                    "name": "idler",
                    "status": "Idle",
                    "budget_remaining_sats": 50000,
                    "budget_spent_sats": 0,
                    "last_heartbeat": now
                },
                {
                    "agent_id": "03a34b99f22c790c4e36b2b3c2c35a36db06226e41c692fc82b8b56ac1c540c5bd",
                    "name": "broken",
                    "status": "Error: connection refused",
                    "budget_remaining_sats": 20000,
                    "budget_spent_sats": 30000,
                    "last_heartbeat": now
                }
            ]
        }))
        .await;

        assert!(result.contains("runner"));
        assert!(result.contains("idler"));
        assert!(result.contains("broken"));
        assert!(result.contains("\"active_tasks\":1"));
    }

    #[test]
    fn test_parse_members_no_members_key() {
        let params = json!({"other": "value"});
        let members = parse_members_from_params(&params);
        assert!(members.is_empty());
    }

    #[test]
    fn test_parse_members_empty_array() {
        let params = json!({"members": []});
        let members = parse_members_from_params(&params);
        assert!(members.is_empty());
    }

    #[test]
    fn test_parse_members_missing_agent_id_skipped() {
        let params = json!({
            "members": [
                {"name": "no-id"}
            ]
        });
        let members = parse_members_from_params(&params);
        assert!(members.is_empty());
    }

    #[test]
    fn test_parse_members_defaults() {
        let params = json!({
            "members": [
                {"agent_id": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"}
            ]
        });
        let members = parse_members_from_params(&params);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "unnamed");
        assert_eq!(members[0].template, "default");
        assert_eq!(members[0].status, AgentStatus::Unknown);
        assert_eq!(members[0].budget_remaining_sats, 0);
        assert_eq!(members[0].budget_spent_sats, 0);
    }

    #[test]
    fn test_parse_members_error_status() {
        let params = json!({
            "members": [
                {
                    "agent_id": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                    "status": "Error: timeout"
                }
            ]
        });
        let members = parse_members_from_params(&params);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].status, AgentStatus::Error("timeout".to_string()));
    }
}
