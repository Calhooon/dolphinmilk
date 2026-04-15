//! Orchestration tools — spawn, check, list, and kill sub-agents.
//!
//! Four discoverable tools (NOT always-on) for sub-agent orchestration:
//! - `spawn_agent` — Create a child agent with budget and tool restrictions
//! - `check_agent` — Get the current status of a child agent
//! - `list_agents` — List all child agents with their statuses
//! - `kill_agent` — Terminate a running child agent

use std::sync::Arc;
use tokio::sync::Mutex;

use serde_json::{json, Value};

use crate::orchestration::{AgentSpawner, SpawnConfig};
use crate::tools::registry::ToolDef;
use crate::types::TaskId;

/// Create all orchestration tools.
///
/// The spawner is shared via `Arc<Mutex<>>` so stateless tool closures can
/// access it. The spawner itself holds references to the task registry and
/// budget pool.
pub fn all_orchestration_tools(spawner: Arc<Mutex<dyn AgentSpawner>>) -> Vec<ToolDef> {
    vec![
        // spawn_agent — create a new child agent
        {
            let spawner = spawner.clone();
            ToolDef {
                name: "spawn_agent".to_string(),
                description: "Spawn a sub-agent to work on a task in parallel. \
                    The child agent gets a carved-out budget and optionally restricted \
                    tool set. Returns the child's task_id for status checking."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "Task description for the child agent to execute"
                        },
                        "budget_sats": {
                            "type": "integer",
                            "description": "Budget to allocate to the child agent in satoshis"
                        },
                        "allowed_tools": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Optional list of tool names the child can use. Omit for all parent tools."
                        },
                        "max_iterations": {
                            "type": "integer",
                            "description": "Maximum iterations for the child (default: 50)"
                        },
                        "model": {
                            "type": "string",
                            "description": "LLM model override for the child agent"
                        }
                    },
                    "required": ["task", "budget_sats"]
                }),
                execute: Box::new(move |params| {
                    let spawner = spawner.clone();
                    Box::pin(async move { spawn_agent(params, &spawner).await })
                }),
                category: "orchestration".to_string(),
                cleanup: None,
                deferred: true,
                always_load: false,
                search_hint: Some(
                    "Spawn a sub-agent with budget and tool restrictions".to_string(),
                ),
            }
        },
        // check_agent — get status of a child agent
        {
            let spawner = spawner.clone();
            ToolDef {
                name: "check_agent".to_string(),
                description: "Check the current status of a spawned sub-agent. \
                    Returns status (running/completed/failed/killed), iterations, \
                    sats spent, and result text if completed."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "Task ID of the child agent (returned by spawn_agent)"
                        }
                    },
                    "required": ["task_id"]
                }),
                execute: Box::new(move |params| {
                    let spawner = spawner.clone();
                    Box::pin(async move { check_agent(params, &spawner).await })
                }),
                category: "orchestration".to_string(),
                cleanup: None,
                deferred: true,
                always_load: false,
                search_hint: Some("Check sub-agent status".to_string()),
            }
        },
        // list_agents — list all child agents
        {
            let spawner = spawner.clone();
            ToolDef {
                name: "list_agents".to_string(),
                description: "List all spawned sub-agents with their current statuses, \
                    budgets, and task descriptions."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
                execute: Box::new(move |params| {
                    let spawner = spawner.clone();
                    Box::pin(async move { list_agents(params, &spawner).await })
                }),
                category: "orchestration".to_string(),
                cleanup: None,
                deferred: true,
                always_load: false,
                search_hint: Some("List running sub-agents".to_string()),
            }
        },
        // kill_agent — terminate a running child
        {
            let spawner = spawner.clone();
            ToolDef {
                name: "kill_agent".to_string(),
                description: "Kill a running sub-agent. Unspent budget is returned \
                    to the parent's pool. Cannot kill agents that have already completed."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "Task ID of the child agent to kill"
                        }
                    },
                    "required": ["task_id"]
                }),
                execute: Box::new(move |params| {
                    let spawner = spawner.clone();
                    Box::pin(async move { kill_agent(params, &spawner).await })
                }),
                category: "orchestration".to_string(),
                cleanup: None,
                deferred: true,
                always_load: false,
                search_hint: Some("Kill a running sub-agent".to_string()),
            }
        },
    ]
}

async fn spawn_agent(params: Value, spawner: &Arc<Mutex<dyn AgentSpawner>>) -> String {
    let task = match params.get("task").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return "Error: 'task' is required".to_string(),
    };

    let budget_sats = match params.get("budget_sats").and_then(|v| v.as_u64()) {
        Some(b) => b,
        None => {
            return "Error: 'budget_sats' is required and must be a positive integer".to_string()
        }
    };

    let allowed_tools = params
        .get("allowed_tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

    let max_iterations = params
        .get("max_iterations")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let model = params
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let config = SpawnConfig {
        task,
        budget_sats,
        allowed_tools,
        max_iterations,
        model,
    };

    let spawner = spawner.lock().await;
    match spawner.spawn(config).await {
        Ok(task_id) => {
            serde_json::to_string_pretty(&json!({
                "task_id": task_id.as_str(),
                "status": "spawned",
                "message": format!("Sub-agent spawned with task ID: {task_id}. Use check_agent to monitor progress."),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".into())
        }
        Err(e) => format!("Error: {e}"),
    }
}

async fn check_agent(params: Value, spawner: &Arc<Mutex<dyn AgentSpawner>>) -> String {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => TaskId::new(id),
        None => return "Error: 'task_id' is required".to_string(),
    };

    let spawner = spawner.lock().await;
    match spawner.check(&task_id).await {
        Ok(status) => serde_json::to_string_pretty(&status)
            .unwrap_or_else(|_| "Error: serialization failed".into()),
        Err(e) => format!("Error: {e}"),
    }
}

async fn list_agents(_params: Value, spawner: &Arc<Mutex<dyn AgentSpawner>>) -> String {
    let spawner = spawner.lock().await;
    match spawner.list().await {
        Ok(agents) => {
            let summary = json!({
                "count": agents.len(),
                "agents": agents,
            });
            serde_json::to_string_pretty(&summary)
                .unwrap_or_else(|_| "Error: serialization failed".into())
        }
        Err(e) => format!("Error: {e}"),
    }
}

async fn kill_agent(params: Value, spawner: &Arc<Mutex<dyn AgentSpawner>>) -> String {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => TaskId::new(id),
        None => return "Error: 'task_id' is required".to_string(),
    };

    let spawner = spawner.lock().await;
    match spawner.kill(&task_id).await {
        Ok(()) => {
            serde_json::to_string_pretty(&json!({
                "task_id": task_id.as_str(),
                "status": "killed",
                "message": format!("Sub-agent {} has been killed. Unspent budget returned to pool.", task_id),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".into())
        }
        Err(e) => format!("Error: {e}"),
    }
}
