//! Task registry — lifecycle tracking for child agents.
//!
//! Tracks active children, collects results, and handles cleanup when
//! children complete or are killed.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::types::TaskId;

use super::types::{AgentInfo, AgentStatus};

/// Registry tracking all spawned child agents.
#[derive(Debug)]
pub struct TaskRegistry {
    /// All tracked agents, keyed by task ID.
    agents: HashMap<String, AgentInfo>,
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Register a new child agent.
    pub fn register(&mut self, info: AgentInfo) {
        self.agents.insert(info.task_id.as_str().to_string(), info);
    }

    /// Update the status of a child agent.
    ///
    /// Returns `true` if the agent was found and updated, `false` otherwise.
    pub fn update_status(&mut self, task_id: &TaskId, status: AgentStatus) -> bool {
        if let Some(info) = self.agents.get_mut(task_id.as_str()) {
            info.status = status;
            true
        } else {
            false
        }
    }

    /// Get the status of a child agent.
    pub fn get_status(&self, task_id: &TaskId) -> Option<&AgentStatus> {
        self.agents.get(task_id.as_str()).map(|info| &info.status)
    }

    /// Get full info for a child agent.
    pub fn get_info(&self, task_id: &TaskId) -> Option<&AgentInfo> {
        self.agents.get(task_id.as_str())
    }

    /// List all tracked agents.
    pub fn list_all(&self) -> Vec<&AgentInfo> {
        self.agents.values().collect()
    }

    /// List only running agents.
    pub fn list_running(&self) -> Vec<&AgentInfo> {
        self.agents
            .values()
            .filter(|info| !info.status.is_terminal())
            .collect()
    }

    /// List only completed/failed/killed agents.
    pub fn list_completed(&self) -> Vec<&AgentInfo> {
        self.agents
            .values()
            .filter(|info| info.status.is_terminal())
            .collect()
    }

    /// Remove a terminal agent from the registry (cleanup).
    ///
    /// Only removes agents in terminal states (completed, failed, killed).
    /// Returns the removed agent info, or `None` if not found or still running.
    pub fn remove_if_terminal(&mut self, task_id: &TaskId) -> Option<AgentInfo> {
        if let Some(info) = self.agents.get(task_id.as_str()) {
            if info.status.is_terminal() {
                return self.agents.remove(task_id.as_str());
            }
        }
        None
    }

    /// Clean up all terminal agents, returning them.
    pub fn cleanup_terminal(&mut self) -> Vec<AgentInfo> {
        let terminal_ids: Vec<String> = self
            .agents
            .iter()
            .filter(|(_, info)| info.status.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();

        terminal_ids
            .into_iter()
            .filter_map(|id| self.agents.remove(&id))
            .collect()
    }

    /// Total number of tracked agents.
    pub fn total_count(&self) -> usize {
        self.agents.len()
    }

    /// Number of currently running agents.
    pub fn running_count(&self) -> usize {
        self.agents
            .values()
            .filter(|info| !info.status.is_terminal())
            .count()
    }

    /// Check if a task ID is tracked.
    pub fn contains(&self, task_id: &TaskId) -> bool {
        self.agents.contains_key(task_id.as_str())
    }

    /// Get total sats spent across all agents.
    pub fn total_sats_spent(&self) -> u64 {
        self.agents
            .values()
            .map(|info| info.status.sats_spent())
            .sum()
    }
}

/// Thread-safe handle to a `TaskRegistry`.
pub type SharedTaskRegistry = Arc<Mutex<TaskRegistry>>;

/// Create a new shared task registry.
pub fn shared_task_registry() -> SharedTaskRegistry {
    Arc::new(Mutex::new(TaskRegistry::new()))
}
