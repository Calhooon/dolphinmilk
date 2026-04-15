//! Agent spawner — trait and local implementation for child agent lifecycle.
//!
//! The `AgentSpawner` trait abstracts child agent creation so the system can
//! be extended with remote spawners (e.g., fleet delegation) in the future.
//! `LocalAgentSpawner` implements in-process child agents via `tokio::spawn`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;

use crate::config::DmConfig;
use crate::error::DmError;
use crate::types::TaskId;

use super::budget::SharedBudgetPool;
use super::registry::SharedTaskRegistry;
use super::types::{AgentInfo, AgentStatus, SpawnConfig};

/// Trait for spawning and managing child agents.
///
/// Implementations may spawn agents in-process (local), as separate processes,
/// or on remote machines (fleet).
#[async_trait]
pub trait AgentSpawner: Send + Sync {
    /// Spawn a new child agent with the given configuration.
    async fn spawn(&self, config: SpawnConfig) -> Result<TaskId, DmError>;

    /// Check the status of a child agent.
    async fn check(&self, task_id: &TaskId) -> Result<AgentStatus, DmError>;

    /// List all child agents.
    async fn list(&self) -> Result<Vec<AgentInfo>, DmError>;

    /// Kill a running child agent.
    async fn kill(&self, task_id: &TaskId) -> Result<(), DmError>;
}

/// In-process child agent spawner.
///
/// Spawns child agents as tokio tasks within the same process. Each child
/// gets its own workspace, budget tracker, and optionally restricted tool set.
pub struct LocalAgentSpawner {
    /// Parent agent configuration (cloned and modified for children).
    config: DmConfig,
    /// Shared task registry for lifecycle tracking.
    registry: SharedTaskRegistry,
    /// Shared budget pool for allocation and return.
    budget_pool: SharedBudgetPool,
    /// Parent workspace root for creating child workspaces.
    workspace_root: PathBuf,
    /// Global memory directory shared across parent and children.
    memory_dir: PathBuf,
    /// Cancel signals for running children: task_id -> cancel flag.
    cancel_flags: Arc<Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
}

impl LocalAgentSpawner {
    /// Create a new local spawner.
    ///
    /// - `config`: Parent agent configuration (children inherit and modify).
    /// - `registry`: Shared task registry for tracking child lifecycle.
    /// - `budget_pool`: Shared budget pool for allocation.
    /// - `workspace_root`: Directory under which child workspaces are created.
    /// - `memory_dir`: Global memory directory shared with children.
    pub fn new(
        config: DmConfig,
        registry: SharedTaskRegistry,
        budget_pool: SharedBudgetPool,
        workspace_root: PathBuf,
        memory_dir: PathBuf,
    ) -> Self {
        Self {
            config,
            registry,
            budget_pool,
            workspace_root,
            memory_dir,
            cancel_flags: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Build a child-specific DmConfig from the parent config and spawn config.
    fn build_child_config(&self, spawn_config: &SpawnConfig) -> DmConfig {
        let mut child_config = self.config.clone();

        // Override budget: child gets a carved-out budget
        child_config.budget.max_per_task = spawn_config.budget_sats;

        // Override model if specified
        if let Some(ref model) = spawn_config.model {
            child_config.llm.default_model = model.clone();
        }

        child_config
    }
}

#[async_trait]
impl AgentSpawner for LocalAgentSpawner {
    async fn spawn(&self, config: SpawnConfig) -> Result<TaskId, DmError> {
        // Validate the spawn config
        config
            .validate()
            .map_err(|e| DmError::tool(format!("invalid spawn config: {e}")))?;

        // Generate a unique task ID for the child
        let task_id = TaskId::new(format!("sub-{}", uuid::Uuid::new_v4()));

        // Allocate budget from the pool
        {
            let mut pool = self.budget_pool.lock().await;
            pool.allocate(&task_id, config.budget_sats)?;
        }

        // Create child workspace
        let child_workspace = self.workspace_root.join("tasks").join(task_id.as_str());
        let _ = std::fs::create_dir_all(&child_workspace);

        // Register the child as running
        let info = AgentInfo {
            task_id: task_id.clone(),
            task: config.task.clone(),
            status: AgentStatus::Running {
                iteration: 0,
                sats_spent: 0,
            },
            budget_sats: config.budget_sats,
            created_at: Utc::now().to_rfc3339(),
        };
        {
            let mut reg = self.registry.lock().await;
            reg.register(info);
        }

        // Create cancel flag for this child
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut flags = self.cancel_flags.lock().await;
            flags.insert(task_id.as_str().to_string(), cancel.clone());
        }

        // Build child configuration
        let child_config = self.build_child_config(&config);

        // Determine max iterations
        let max_iterations = config.max_iterations.unwrap_or(50);

        // Build tool allowlist
        let allowed_tools = config.allowed_tools.clone();

        // Clone references for the spawned task
        let registry = self.registry.clone();
        let budget_pool = self.budget_pool.clone();
        let cancel_flags = self.cancel_flags.clone();
        let task_id_clone = task_id.clone();
        let task_description = config.task.clone();
        let memory_dir = self.memory_dir.clone();

        // Spawn the child agent as a tokio task
        tokio::spawn(async move {
            let result = run_child_agent(
                child_config,
                child_workspace,
                memory_dir,
                &task_description,
                max_iterations,
                allowed_tools,
                cancel.clone(),
            )
            .await;

            // Update registry with final status
            let final_status = match result {
                Ok((result_text, sats_spent, iterations)) => AgentStatus::Completed {
                    result: result_text,
                    sats_spent,
                    iterations,
                },
                Err(e) => AgentStatus::Failed {
                    error: e.to_string(),
                    sats_spent: 0,
                },
            };

            // Record spending and return unspent budget
            if let AgentStatus::Completed { sats_spent, .. }
            | AgentStatus::Failed { sats_spent, .. } = &final_status
            {
                let mut pool = budget_pool.lock().await;
                pool.record_spending(&task_id_clone, *sats_spent);
                pool.return_unspent(&task_id_clone);
            }

            // Update registry
            {
                let mut reg = registry.lock().await;
                reg.update_status(&task_id_clone, final_status);
            }

            // Clean up cancel flag
            {
                let mut flags = cancel_flags.lock().await;
                flags.remove(task_id_clone.as_str());
            }
        });

        Ok(task_id)
    }

    async fn check(&self, task_id: &TaskId) -> Result<AgentStatus, DmError> {
        let reg = self.registry.lock().await;
        reg.get_status(task_id)
            .cloned()
            .ok_or_else(|| DmError::tool(format!("unknown agent: {task_id}")))
    }

    async fn list(&self) -> Result<Vec<AgentInfo>, DmError> {
        let reg = self.registry.lock().await;
        Ok(reg.list_all().into_iter().cloned().collect())
    }

    async fn kill(&self, task_id: &TaskId) -> Result<(), DmError> {
        // Set the cancel flag
        {
            let flags = self.cancel_flags.lock().await;
            if let Some(flag) = flags.get(task_id.as_str()) {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                // Check if it's already terminal
                let reg = self.registry.lock().await;
                if let Some(status) = reg.get_status(task_id) {
                    if status.is_terminal() {
                        return Err(DmError::tool(format!(
                            "agent {} is already in terminal state",
                            task_id
                        )));
                    }
                }
                return Err(DmError::tool(format!("unknown agent: {task_id}")));
            }
        }

        // Update status to Killed
        {
            let mut reg = self.registry.lock().await;
            reg.update_status(task_id, AgentStatus::Killed);
        }

        // Return unspent budget
        {
            let mut pool = self.budget_pool.lock().await;
            pool.return_unspent(task_id);
        }

        // Clean up cancel flag
        {
            let mut flags = self.cancel_flags.lock().await;
            flags.remove(task_id.as_str());
        }

        Ok(())
    }
}

/// Run a child agent loop.
///
/// This creates a `DmLoop` with the child's configuration and runs it.
/// Tool restrictions are applied via the tool registry's allowlist.
///
/// Returns `(result_text, sats_spent, iterations)` on success.
async fn run_child_agent(
    config: DmConfig,
    workspace: PathBuf,
    memory_dir: PathBuf,
    task: &str,
    max_iterations: u32,
    allowed_tools: Option<Vec<String>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(String, u64, u32), DmError> {
    use crate::runner::create_loop;

    let wallet: std::sync::Arc<dyn crate::wallet::WalletBackend + Send + Sync> =
        std::sync::Arc::new(crate::wallet::HttpWalletClient::from_config(&config.wallet));
    let mut agent_loop = create_loop(config, workspace, Some(memory_dir), None, wallet);

    // Apply tool restrictions if specified
    if let Some(tools) = allowed_tools {
        let allowset: HashSet<String> = tools.into_iter().collect();
        let mut reg = agent_loop.tools.write().await;
        reg.set_allowlist(allowset);
    }

    // Run the agent loop
    agent_loop.run(task, max_iterations, None, cancel).await;

    // Extract results
    let sats_spent = agent_loop.budget_tracker.task_sats();
    let iterations = agent_loop.state.exec.iteration;
    let result = if agent_loop.state.exec.result.is_empty() {
        "Child agent completed without text result".to_string()
    } else {
        agent_loop.state.exec.result.clone()
    };

    Ok((result, sats_spent, iterations))
}
