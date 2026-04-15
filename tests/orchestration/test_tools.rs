//! Tests for orchestration tools — spawn_agent, check_agent, list_agents, kill_agent.
//!
//! Uses a mock spawner to test tool parameter parsing and error handling
//! without running actual child agents.

use std::sync::Arc;
use tokio::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use dolphin_milk::error::DmError;
use dolphin_milk::orchestration::types::{AgentInfo, AgentStatus, SpawnConfig};
use dolphin_milk::orchestration::AgentSpawner;
use dolphin_milk::tools::orchestration_tools::all_orchestration_tools;
use dolphin_milk::tools::registry::{ToolDef, ToolRegistry, ALWAYS_ON_TOOLS};
use dolphin_milk::types::TaskId;

/// Mock spawner that tracks calls without running actual agents.
struct MockSpawner {
    /// Track last spawn config for assertions.
    last_spawn: Mutex<Option<SpawnConfig>>,
    /// Pre-configured status responses.
    statuses: Mutex<std::collections::HashMap<String, AgentStatus>>,
    /// Pre-configured agent list.
    agents: Mutex<Vec<AgentInfo>>,
}

impl MockSpawner {
    fn new() -> Self {
        Self {
            last_spawn: Mutex::new(None),
            statuses: Mutex::new(std::collections::HashMap::new()),
            agents: Mutex::new(Vec::new()),
        }
    }

    async fn set_status(&self, id: &str, status: AgentStatus) {
        self.statuses.lock().await.insert(id.to_string(), status);
    }

    async fn set_agents(&self, agents: Vec<AgentInfo>) {
        *self.agents.lock().await = agents;
    }
}

#[async_trait]
impl AgentSpawner for MockSpawner {
    async fn spawn(&self, config: SpawnConfig) -> Result<TaskId, DmError> {
        config
            .validate()
            .map_err(|e| DmError::tool(format!("invalid spawn config: {e}")))?;
        *self.last_spawn.lock().await = Some(config);
        Ok(TaskId::new("sub-mock-001"))
    }

    async fn check(&self, task_id: &TaskId) -> Result<AgentStatus, DmError> {
        self.statuses
            .lock()
            .await
            .get(task_id.as_str())
            .cloned()
            .ok_or_else(|| DmError::tool(format!("unknown agent: {task_id}")))
    }

    async fn list(&self) -> Result<Vec<AgentInfo>, DmError> {
        Ok(self.agents.lock().await.clone())
    }

    async fn kill(&self, task_id: &TaskId) -> Result<(), DmError> {
        if self.statuses.lock().await.contains_key(task_id.as_str()) {
            Ok(())
        } else {
            Err(DmError::tool(format!("unknown agent: {task_id}")))
        }
    }
}

fn make_spawner() -> Arc<Mutex<dyn AgentSpawner>> {
    Arc::new(Mutex::new(MockSpawner::new()))
}

fn make_spawner_with_mock() -> (Arc<Mutex<dyn AgentSpawner>>, Arc<MockSpawner>) {
    let mock = Arc::new(MockSpawner::new());
    let spawner: Arc<Mutex<dyn AgentSpawner>> =
        Arc::new(Mutex::new(MockSpawnerWrapper(mock.clone())));
    (spawner, mock)
}

/// Wrapper to use Arc<MockSpawner> as AgentSpawner.
struct MockSpawnerWrapper(Arc<MockSpawner>);

#[async_trait]
impl AgentSpawner for MockSpawnerWrapper {
    async fn spawn(&self, config: SpawnConfig) -> Result<TaskId, DmError> {
        self.0.spawn(config).await
    }
    async fn check(&self, task_id: &TaskId) -> Result<AgentStatus, DmError> {
        self.0.check(task_id).await
    }
    async fn list(&self) -> Result<Vec<AgentInfo>, DmError> {
        self.0.list().await
    }
    async fn kill(&self, task_id: &TaskId) -> Result<(), DmError> {
        self.0.kill(task_id).await
    }
}

// -- Tool registration --

#[test]
fn test_tool_count() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    assert_eq!(tools.len(), 4);
}

#[test]
fn test_tool_names() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"spawn_agent"));
    assert!(names.contains(&"check_agent"));
    assert!(names.contains(&"list_agents"));
    assert!(names.contains(&"kill_agent"));
}

#[test]
fn test_tools_are_discoverable_not_always_on() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    for tool in &tools {
        assert!(
            !ALWAYS_ON_TOOLS.contains(&tool.name.as_str()),
            "{} should not be in ALWAYS_ON_TOOLS",
            tool.name
        );
    }
}

#[test]
fn test_tool_categories() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    for tool in &tools {
        assert_eq!(tool.category, "orchestration", "tool: {}", tool.name);
    }
}

// -- spawn_agent tool --

#[tokio::test]
async fn test_spawn_agent_success() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let spawn_tool = tools.iter().find(|t| t.name == "spawn_agent").unwrap();

    let result = (spawn_tool.execute)(json!({
        "task": "Research BSV fees",
        "budget_sats": 10000
    }))
    .await;

    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "spawned");
    assert!(parsed["task_id"].as_str().is_some());
}

#[tokio::test]
async fn test_spawn_agent_missing_task() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let spawn_tool = tools.iter().find(|t| t.name == "spawn_agent").unwrap();

    let result = (spawn_tool.execute)(json!({"budget_sats": 10000})).await;
    assert!(result.contains("Error"), "got: {result}");
    assert!(result.contains("task"), "got: {result}");
}

#[tokio::test]
async fn test_spawn_agent_missing_budget() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let spawn_tool = tools.iter().find(|t| t.name == "spawn_agent").unwrap();

    let result = (spawn_tool.execute)(json!({"task": "Do something"})).await;
    assert!(result.contains("Error"), "got: {result}");
    assert!(result.contains("budget_sats"), "got: {result}");
}

// -- check_agent tool --

#[tokio::test]
async fn test_check_agent_running() {
    let (spawner, mock) = make_spawner_with_mock();
    mock.set_status(
        "sub-001",
        AgentStatus::Running {
            iteration: 5,
            sats_spent: 2500,
        },
    )
    .await;

    let tools = all_orchestration_tools(spawner);
    let check_tool = tools.iter().find(|t| t.name == "check_agent").unwrap();

    let result = (check_tool.execute)(json!({"task_id": "sub-001"})).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "running");
    assert_eq!(parsed["iteration"], 5);
}

#[tokio::test]
async fn test_check_agent_not_found() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let check_tool = tools.iter().find(|t| t.name == "check_agent").unwrap();

    let result = (check_tool.execute)(json!({"task_id": "nonexistent"})).await;
    assert!(result.contains("Error"), "got: {result}");
}

#[tokio::test]
async fn test_check_agent_missing_task_id() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let check_tool = tools.iter().find(|t| t.name == "check_agent").unwrap();

    let result = (check_tool.execute)(json!({})).await;
    assert!(result.contains("Error"), "got: {result}");
    assert!(result.contains("task_id"), "got: {result}");
}

// -- list_agents tool --

#[tokio::test]
async fn test_list_agents_empty() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let list_tool = tools.iter().find(|t| t.name == "list_agents").unwrap();

    let result = (list_tool.execute)(json!({})).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["count"], 0);
    assert!(parsed["agents"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_list_agents_with_entries() {
    let (spawner, mock) = make_spawner_with_mock();
    mock.set_agents(vec![
        AgentInfo {
            task_id: TaskId::new("sub-a"),
            task: "task a".into(),
            status: AgentStatus::Running {
                iteration: 3,
                sats_spent: 1000,
            },
            budget_sats: 20000,
            created_at: "2026-04-01T00:00:00Z".into(),
        },
        AgentInfo {
            task_id: TaskId::new("sub-b"),
            task: "task b".into(),
            status: AgentStatus::Completed {
                result: "done".into(),
                sats_spent: 5000,
                iterations: 10,
            },
            budget_sats: 30000,
            created_at: "2026-04-01T00:01:00Z".into(),
        },
    ])
    .await;

    let tools = all_orchestration_tools(spawner);
    let list_tool = tools.iter().find(|t| t.name == "list_agents").unwrap();

    let result = (list_tool.execute)(json!({})).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["count"], 2);
}

// -- kill_agent tool --

#[tokio::test]
async fn test_kill_agent_success() {
    let (spawner, mock) = make_spawner_with_mock();
    mock.set_status(
        "sub-001",
        AgentStatus::Running {
            iteration: 5,
            sats_spent: 2000,
        },
    )
    .await;

    let tools = all_orchestration_tools(spawner);
    let kill_tool = tools.iter().find(|t| t.name == "kill_agent").unwrap();

    let result = (kill_tool.execute)(json!({"task_id": "sub-001"})).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "killed");
}

#[tokio::test]
async fn test_kill_agent_not_found() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let kill_tool = tools.iter().find(|t| t.name == "kill_agent").unwrap();

    let result = (kill_tool.execute)(json!({"task_id": "ghost"})).await;
    assert!(result.contains("Error"), "got: {result}");
}

#[tokio::test]
async fn test_kill_agent_missing_task_id() {
    let spawner = make_spawner();
    let tools = all_orchestration_tools(spawner);
    let kill_tool = tools.iter().find(|t| t.name == "kill_agent").unwrap();

    let result = (kill_tool.execute)(json!({})).await;
    assert!(result.contains("Error"), "got: {result}");
    assert!(result.contains("task_id"), "got: {result}");
}

// -- Tool restriction enforcement --

#[test]
fn test_tool_restriction_via_registry() {
    use std::collections::HashSet;

    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "web_fetch".to_string(),
        description: "fetch".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".into() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    reg.register(ToolDef {
        name: "execute_bash".to_string(),
        description: "bash".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".into() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });

    // Restrict to only web_fetch
    let allowed: HashSet<String> = vec!["web_fetch".to_string()].into_iter().collect();
    reg.set_allowlist(allowed);

    assert!(reg.is_allowed("web_fetch"));
    assert!(!reg.is_allowed("execute_bash"));
}
