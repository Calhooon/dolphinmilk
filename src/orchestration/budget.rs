//! Budget carving — allocate sats from parent to child, return unspent.
//!
//! The parent agent carves a portion of its budget for each child. The child
//! has a dedicated `BudgetTracker` capped at the carved amount. When the child
//! finishes, unspent sats are returned to the parent's available pool.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::DmError;
use crate::types::TaskId;

/// Tracks budget allocations from parent to children.
///
/// Thread-safe via `Arc<Mutex<>>` so it can be shared across spawned tasks.
#[derive(Debug)]
pub struct BudgetPool {
    /// Total sats the parent has allocated for sub-agent spawning.
    total_allocated: u64,
    /// Per-child allocations: task_id -> allocated sats.
    allocations: HashMap<String, u64>,
    /// Per-child spending: task_id -> sats actually spent.
    spent: HashMap<String, u64>,
    /// Maximum sats the pool can allocate in total.
    pool_limit: u64,
}

impl BudgetPool {
    /// Create a new budget pool with a maximum allocation limit.
    pub fn new(pool_limit: u64) -> Self {
        Self {
            total_allocated: 0,
            allocations: HashMap::new(),
            spent: HashMap::new(),
            pool_limit,
        }
    }

    /// Allocate sats for a child agent.
    ///
    /// Returns `Ok(())` if the allocation fits within the pool limit.
    /// Returns `Err` if the requested amount would exceed the pool.
    pub fn allocate(&mut self, task_id: &TaskId, sats: u64) -> Result<(), DmError> {
        if sats == 0 {
            return Err(DmError::budget("cannot allocate 0 sats"));
        }

        let new_total = self.total_allocated.saturating_add(sats);
        if new_total > self.pool_limit {
            return Err(DmError::budget(format!(
                "budget pool exhausted: requested {} sats but only {} available (allocated {}/{})",
                sats,
                self.pool_limit.saturating_sub(self.total_allocated),
                self.total_allocated,
                self.pool_limit,
            )));
        }

        self.total_allocated = new_total;
        self.allocations.insert(task_id.as_str().to_string(), sats);
        self.spent.insert(task_id.as_str().to_string(), 0);
        Ok(())
    }

    /// Record spending by a child agent.
    pub fn record_spending(&mut self, task_id: &TaskId, sats: u64) {
        let entry = self.spent.entry(task_id.as_str().to_string()).or_insert(0);
        *entry = entry.saturating_add(sats);
    }

    /// Return unspent sats from a completed child back to the pool.
    ///
    /// This decreases `total_allocated` by the unspent amount, making those
    /// sats available for future child allocations.
    pub fn return_unspent(&mut self, task_id: &TaskId) -> u64 {
        let allocated = self.allocations.get(task_id.as_str()).copied().unwrap_or(0);
        let spent = self.spent.get(task_id.as_str()).copied().unwrap_or(0);
        let unspent = allocated.saturating_sub(spent);

        // Return unspent to the pool
        self.total_allocated = self.total_allocated.saturating_sub(unspent);

        unspent
    }

    /// Get the remaining budget for a specific child.
    pub fn child_remaining(&self, task_id: &TaskId) -> u64 {
        let allocated = self.allocations.get(task_id.as_str()).copied().unwrap_or(0);
        let spent = self.spent.get(task_id.as_str()).copied().unwrap_or(0);
        allocated.saturating_sub(spent)
    }

    /// Get the total unallocated sats remaining in the pool.
    pub fn pool_remaining(&self) -> u64 {
        self.pool_limit.saturating_sub(self.total_allocated)
    }

    /// Get the pool limit.
    pub fn pool_limit(&self) -> u64 {
        self.pool_limit
    }

    /// Get total allocated sats across all children.
    pub fn total_allocated(&self) -> u64 {
        self.total_allocated
    }

    /// Get the number of active allocations.
    pub fn allocation_count(&self) -> usize {
        self.allocations.len()
    }

    /// Get the sats spent by a specific child.
    pub fn child_spent(&self, task_id: &TaskId) -> u64 {
        self.spent.get(task_id.as_str()).copied().unwrap_or(0)
    }
}

/// Thread-safe handle to a `BudgetPool`.
pub type SharedBudgetPool = Arc<Mutex<BudgetPool>>;

/// Create a new shared budget pool.
pub fn shared_budget_pool(pool_limit: u64) -> SharedBudgetPool {
    Arc::new(Mutex::new(BudgetPool::new(pool_limit)))
}
