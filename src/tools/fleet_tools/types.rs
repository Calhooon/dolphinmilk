//! Fleet management types — agent status, fleet membership, task assignments.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Status of an individual agent in the fleet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", content = "detail")]
pub enum AgentStatus {
    /// Agent is actively executing a task.
    Running,
    /// Agent is alive but has no current task.
    Idle,
    /// Agent encountered an error.
    Error(String),
    /// Agent has been recalled or shut down.
    Stopped,
    /// No heartbeat received within the timeout window.
    Unknown,
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentStatus::Running => write!(f, "Running"),
            AgentStatus::Idle => write!(f, "Idle"),
            AgentStatus::Error(msg) => write!(f, "Error: {msg}"),
            AgentStatus::Stopped => write!(f, "Stopped"),
            AgentStatus::Unknown => write!(f, "Unknown"),
        }
    }
}

/// A single member of the fleet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetMember {
    /// The child agent's identity key (66-char hex compressed pubkey).
    pub agent_id: String,
    /// Human-readable name.
    pub name: String,
    /// Agent template (e.g. "researcher", "writer", "analyst").
    pub template: String,
    /// Current status.
    pub status: AgentStatus,
    /// Description of the current task, if any.
    pub current_task: Option<String>,
    /// Satoshis remaining in the child's budget.
    pub budget_remaining_sats: u64,
    /// Satoshis spent by the child so far.
    pub budget_spent_sats: u64,
    /// Last heartbeat timestamp from the child.
    pub last_heartbeat: DateTime<Utc>,
    /// Endpoint where the child is reachable (e.g. MessageBox identity key).
    pub endpoint: String,
}

/// Aggregate status of the entire fleet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetStatus {
    /// All fleet members.
    pub members: Vec<FleetMember>,
    /// Total budget allocated across all members (satoshis).
    pub total_budget_sats: u64,
    /// Total spent across all members (satoshis).
    pub total_spent_sats: u64,
    /// Number of members currently executing tasks.
    pub active_tasks: usize,
}

/// A task assignment sent to a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignment {
    /// Target agent's identity key.
    pub agent_id: String,
    /// Task description.
    pub task: String,
    /// Priority level.
    pub priority: TaskPriority,
    /// Optional budget limit for this specific task (satoshis).
    pub budget_limit_sats: Option<u64>,
}

/// Task priority levels.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
}

impl fmt::Display for TaskPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskPriority::Low => write!(f, "low"),
            TaskPriority::Normal => write!(f, "normal"),
            TaskPriority::High => write!(f, "high"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_status_display() {
        assert_eq!(AgentStatus::Running.to_string(), "Running");
        assert_eq!(AgentStatus::Idle.to_string(), "Idle");
        assert_eq!(
            AgentStatus::Error("out of memory".into()).to_string(),
            "Error: out of memory"
        );
        assert_eq!(AgentStatus::Stopped.to_string(), "Stopped");
        assert_eq!(AgentStatus::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn test_agent_status_serialization() {
        let running = serde_json::to_string(&AgentStatus::Running).unwrap();
        assert!(running.contains("Running"));

        let error = serde_json::to_string(&AgentStatus::Error("timeout".into())).unwrap();
        assert!(error.contains("Error"));
        assert!(error.contains("timeout"));

        // Round-trip
        let deserialized: AgentStatus = serde_json::from_str(&running).unwrap();
        assert_eq!(deserialized, AgentStatus::Running);

        let deserialized_err: AgentStatus = serde_json::from_str(&error).unwrap();
        assert_eq!(deserialized_err, AgentStatus::Error("timeout".into()));
    }

    #[test]
    fn test_task_priority_default() {
        assert_eq!(TaskPriority::default(), TaskPriority::Normal);
    }

    #[test]
    fn test_task_priority_display() {
        assert_eq!(TaskPriority::Low.to_string(), "low");
        assert_eq!(TaskPriority::Normal.to_string(), "normal");
        assert_eq!(TaskPriority::High.to_string(), "high");
    }

    #[test]
    fn test_task_priority_serialization() {
        let json = serde_json::to_string(&TaskPriority::High).unwrap();
        assert_eq!(json, "\"high\"");
        let back: TaskPriority = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TaskPriority::High);
    }

    #[test]
    fn test_fleet_member_serialization() {
        let member = FleetMember {
            agent_id: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
            name: "researcher-1".to_string(),
            template: "researcher".to_string(),
            status: AgentStatus::Running,
            current_task: Some("Analyze BSV fees".to_string()),
            budget_remaining_sats: 45000,
            budget_spent_sats: 5000,
            last_heartbeat: Utc::now(),
            endpoint: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
        };

        let json = serde_json::to_string(&member).unwrap();
        assert!(json.contains("researcher-1"));
        assert!(json.contains("45000"));

        let back: FleetMember = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "researcher-1");
        assert_eq!(back.budget_remaining_sats, 45000);
    }

    #[test]
    fn test_task_assignment_serialization() {
        let task = TaskAssignment {
            agent_id: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
            task: "Research BSV overlay network".to_string(),
            priority: TaskPriority::High,
            budget_limit_sats: Some(20000),
        };

        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("Research BSV overlay network"));
        assert!(json.contains("high"));
        assert!(json.contains("20000"));

        let back: TaskAssignment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.task, "Research BSV overlay network");
        assert_eq!(back.priority, TaskPriority::High);
        assert_eq!(back.budget_limit_sats, Some(20000));
    }
}
