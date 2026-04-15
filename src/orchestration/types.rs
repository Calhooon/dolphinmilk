//! Sub-agent orchestration types — spawn config, status, and info.

use serde::{Deserialize, Serialize};

use crate::types::TaskId;

/// Configuration for spawning a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnConfig {
    /// Task description for the child agent to execute.
    pub task: String,
    /// Budget allocated to the child agent in satoshis.
    pub budget_sats: u64,
    /// Optional tool allowlist. `None` means all parent tools are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    /// Maximum iterations for the child agent loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
    /// LLM model override for the child agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl SpawnConfig {
    /// Validate the spawn configuration.
    ///
    /// Returns `Ok(())` if valid, or an error message describing the problem.
    pub fn validate(&self) -> Result<(), String> {
        if self.task.trim().is_empty() {
            return Err("task must not be empty".into());
        }
        if self.budget_sats == 0 {
            return Err("budget_sats must be greater than 0".into());
        }
        if let Some(ref tools) = self.allowed_tools {
            if tools.is_empty() {
                return Err("allowed_tools must not be empty when specified".into());
            }
            for tool in tools {
                if tool.trim().is_empty() {
                    return Err("allowed_tools must not contain empty strings".into());
                }
            }
        }
        if let Some(max) = self.max_iterations {
            if max == 0 {
                return Err("max_iterations must be greater than 0".into());
            }
        }
        Ok(())
    }
}

/// Current status of a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum AgentStatus {
    /// Agent is currently running.
    #[serde(rename = "running")]
    Running { iteration: u32, sats_spent: u64 },
    /// Agent completed successfully.
    #[serde(rename = "completed")]
    Completed {
        result: String,
        sats_spent: u64,
        iterations: u32,
    },
    /// Agent failed with an error.
    #[serde(rename = "failed")]
    Failed { error: String, sats_spent: u64 },
    /// Agent was killed by the parent.
    #[serde(rename = "killed")]
    Killed,
}

impl AgentStatus {
    /// Returns true if the agent has finished (completed, failed, or killed).
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running { .. })
    }

    /// Returns the sats spent, or 0 for killed agents.
    pub fn sats_spent(&self) -> u64 {
        match self {
            Self::Running { sats_spent, .. } => *sats_spent,
            Self::Completed { sats_spent, .. } => *sats_spent,
            Self::Failed { sats_spent, .. } => *sats_spent,
            Self::Killed => 0,
        }
    }
}

/// Information about a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Unique task ID for the child agent.
    pub task_id: TaskId,
    /// Task description.
    pub task: String,
    /// Current status.
    pub status: AgentStatus,
    /// Budget allocated in satoshis.
    pub budget_sats: u64,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}
