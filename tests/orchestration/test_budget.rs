//! Tests for BudgetPool — budget carving from parent to child.

use dolphin_milk::orchestration::budget::{shared_budget_pool, BudgetPool};
use dolphin_milk::types::TaskId;

#[test]
fn test_new_pool() {
    let pool = BudgetPool::new(100_000);
    assert_eq!(pool.pool_limit(), 100_000);
    assert_eq!(pool.pool_remaining(), 100_000);
    assert_eq!(pool.total_allocated(), 0);
    assert_eq!(pool.allocation_count(), 0);
}

#[test]
fn test_allocate_success() {
    let mut pool = BudgetPool::new(100_000);
    let id = TaskId::new("sub-001");
    assert!(pool.allocate(&id, 30_000).is_ok());
    assert_eq!(pool.total_allocated(), 30_000);
    assert_eq!(pool.pool_remaining(), 70_000);
    assert_eq!(pool.allocation_count(), 1);
}

#[test]
fn test_allocate_multiple() {
    let mut pool = BudgetPool::new(100_000);
    assert!(pool.allocate(&TaskId::new("a"), 30_000).is_ok());
    assert!(pool.allocate(&TaskId::new("b"), 40_000).is_ok());
    assert_eq!(pool.total_allocated(), 70_000);
    assert_eq!(pool.pool_remaining(), 30_000);
    assert_eq!(pool.allocation_count(), 2);
}

#[test]
fn test_allocate_exceeds_pool() {
    let mut pool = BudgetPool::new(50_000);
    assert!(pool.allocate(&TaskId::new("a"), 30_000).is_ok());

    let err = pool.allocate(&TaskId::new("b"), 30_000);
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("budget pool exhausted"), "got: {msg}");
}

#[test]
fn test_allocate_zero_rejected() {
    let mut pool = BudgetPool::new(100_000);
    let err = pool.allocate(&TaskId::new("a"), 0);
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("cannot allocate 0 sats"), "got: {msg}");
}

#[test]
fn test_record_spending() {
    let mut pool = BudgetPool::new(100_000);
    let id = TaskId::new("sub-001");
    pool.allocate(&id, 50_000).unwrap();
    pool.record_spending(&id, 10_000);
    pool.record_spending(&id, 5_000);

    assert_eq!(pool.child_spent(&id), 15_000);
    assert_eq!(pool.child_remaining(&id), 35_000);
}

#[test]
fn test_return_unspent() {
    let mut pool = BudgetPool::new(100_000);
    let id = TaskId::new("sub-001");
    pool.allocate(&id, 50_000).unwrap();
    pool.record_spending(&id, 20_000);

    let unspent = pool.return_unspent(&id);
    assert_eq!(unspent, 30_000);
    // Pool should have 30k more available now
    assert_eq!(pool.pool_remaining(), 80_000);
}

#[test]
fn test_return_unspent_fully_spent() {
    let mut pool = BudgetPool::new(100_000);
    let id = TaskId::new("sub-001");
    pool.allocate(&id, 50_000).unwrap();
    pool.record_spending(&id, 50_000);

    let unspent = pool.return_unspent(&id);
    assert_eq!(unspent, 0);
    assert_eq!(pool.pool_remaining(), 50_000);
}

#[test]
fn test_child_remaining_for_unknown() {
    let pool = BudgetPool::new(100_000);
    assert_eq!(pool.child_remaining(&TaskId::new("ghost")), 0);
}

#[test]
fn test_child_spent_for_unknown() {
    let pool = BudgetPool::new(100_000);
    assert_eq!(pool.child_spent(&TaskId::new("ghost")), 0);
}

#[test]
fn test_budget_recycle_after_return() {
    let mut pool = BudgetPool::new(100_000);

    // First child uses 30k, spends 10k, returns 20k
    let id1 = TaskId::new("sub-001");
    pool.allocate(&id1, 30_000).unwrap();
    pool.record_spending(&id1, 10_000);
    pool.return_unspent(&id1);

    // Now 90k should be available (100k - 10k spent by first child)
    assert_eq!(pool.pool_remaining(), 90_000);

    // Second child can allocate up to 90k
    let id2 = TaskId::new("sub-002");
    assert!(pool.allocate(&id2, 90_000).is_ok());
}

#[test]
fn test_exact_pool_allocation() {
    let mut pool = BudgetPool::new(100_000);
    assert!(pool.allocate(&TaskId::new("a"), 100_000).is_ok());
    assert_eq!(pool.pool_remaining(), 0);

    // Can't allocate even 1 more sat
    let err = pool.allocate(&TaskId::new("b"), 1);
    assert!(err.is_err());
}

#[tokio::test]
async fn test_shared_budget_pool() {
    let pool = shared_budget_pool(100_000);
    let id = TaskId::new("sub-001");

    {
        let mut p = pool.lock().await;
        p.allocate(&id, 25_000).unwrap();
    }

    {
        let p = pool.lock().await;
        assert_eq!(p.pool_remaining(), 75_000);
    }
}
