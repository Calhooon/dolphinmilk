//! Sub-agent orchestration — spawn, track, and manage child agents.
//!
//! This module enables the parent agent to spawn child agents with carved-out
//! budgets and restricted tool sets. Each child runs its own agent loop
//! (OBSERVE-THINK-ACT-RECORD-BUDGET CHECK) as a tokio task.
//!
//! ## Components
//!
//! - **types** — `SpawnConfig`, `AgentStatus`, `AgentInfo` data structures
//! - **spawner** — `AgentSpawner` trait + `LocalAgentSpawner` implementation
//! - **registry** — `TaskRegistry` for lifecycle tracking
//! - **budget** — `BudgetPool` for parent-to-child budget carving

pub mod agents;
pub mod budget;
pub mod registry;
pub mod spawner;
pub mod types;
pub mod worktree;

// Re-exports for convenience
pub use agents::{agent_available, load_all_agents, parse_agent_content, AgentDefinition};
pub use budget::{shared_budget_pool, BudgetPool, SharedBudgetPool};
pub use registry::{shared_task_registry, SharedTaskRegistry, TaskRegistry};
pub use spawner::{AgentSpawner, LocalAgentSpawner};
pub use types::{AgentInfo, AgentStatus, SpawnConfig};
pub use worktree::{IsolationMode, WorktreeInfo};
