//! Tests for TaskRegistry lifecycle — spawn, running, completed, cleanup.

use dolphin_milk::orchestration::registry::TaskRegistry;
use dolphin_milk::orchestration::types::{AgentInfo, AgentStatus};
use dolphin_milk::types::TaskId;

fn make_info(id: &str, task: &str, status: AgentStatus) -> AgentInfo {
    AgentInfo {
        task_id: TaskId::new(id),
        task: task.into(),
        status,
        budget_sats: 10000,
        created_at: "2026-04-01T00:00:00Z".into(),
    }
}

#[test]
fn test_register_and_get() {
    let mut reg = TaskRegistry::new();
    let info = make_info(
        "sub-001",
        "Research fees",
        AgentStatus::Running {
            iteration: 0,
            sats_spent: 0,
        },
    );
    reg.register(info);
    assert_eq!(reg.total_count(), 1);
    assert!(reg.contains(&TaskId::new("sub-001")));
}

#[test]
fn test_get_status() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-002",
        "Analyze costs",
        AgentStatus::Running {
            iteration: 3,
            sats_spent: 1500,
        },
    ));

    let status = reg.get_status(&TaskId::new("sub-002")).unwrap();
    match status {
        AgentStatus::Running {
            iteration,
            sats_spent,
        } => {
            assert_eq!(*iteration, 3);
            assert_eq!(*sats_spent, 1500);
        }
        _ => panic!("expected Running"),
    }
}

#[test]
fn test_get_status_not_found() {
    let reg = TaskRegistry::new();
    assert!(reg.get_status(&TaskId::new("nonexistent")).is_none());
}

#[test]
fn test_update_status_to_completed() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-003",
        "task",
        AgentStatus::Running {
            iteration: 0,
            sats_spent: 0,
        },
    ));

    let updated = reg.update_status(
        &TaskId::new("sub-003"),
        AgentStatus::Completed {
            result: "Done!".into(),
            sats_spent: 5000,
            iterations: 10,
        },
    );
    assert!(updated);

    let status = reg.get_status(&TaskId::new("sub-003")).unwrap();
    assert!(status.is_terminal());
}

#[test]
fn test_update_status_not_found() {
    let mut reg = TaskRegistry::new();
    let updated = reg.update_status(&TaskId::new("ghost"), AgentStatus::Killed);
    assert!(!updated);
}

#[test]
fn test_list_all() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-a",
        "task a",
        AgentStatus::Running {
            iteration: 1,
            sats_spent: 500,
        },
    ));
    reg.register(make_info(
        "sub-b",
        "task b",
        AgentStatus::Completed {
            result: "ok".into(),
            sats_spent: 3000,
            iterations: 5,
        },
    ));

    let all = reg.list_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_list_running() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-run",
        "running task",
        AgentStatus::Running {
            iteration: 2,
            sats_spent: 800,
        },
    ));
    reg.register(make_info(
        "sub-done",
        "done task",
        AgentStatus::Completed {
            result: "ok".into(),
            sats_spent: 2000,
            iterations: 5,
        },
    ));
    reg.register(make_info("sub-dead", "dead task", AgentStatus::Killed));

    assert_eq!(reg.list_running().len(), 1);
    assert_eq!(reg.list_running()[0].task_id.as_str(), "sub-run");
}

#[test]
fn test_list_completed() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-run",
        "running",
        AgentStatus::Running {
            iteration: 0,
            sats_spent: 0,
        },
    ));
    reg.register(make_info(
        "sub-done",
        "done",
        AgentStatus::Completed {
            result: "ok".into(),
            sats_spent: 2000,
            iterations: 5,
        },
    ));
    reg.register(make_info(
        "sub-fail",
        "failed",
        AgentStatus::Failed {
            error: "oops".into(),
            sats_spent: 500,
        },
    ));

    assert_eq!(reg.list_completed().len(), 2);
}

#[test]
fn test_remove_if_terminal_completed() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-x",
        "completed task",
        AgentStatus::Completed {
            result: "ok".into(),
            sats_spent: 1000,
            iterations: 3,
        },
    ));

    let removed = reg.remove_if_terminal(&TaskId::new("sub-x"));
    assert!(removed.is_some());
    assert_eq!(reg.total_count(), 0);
}

#[test]
fn test_remove_if_terminal_running_not_removed() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-y",
        "running task",
        AgentStatus::Running {
            iteration: 5,
            sats_spent: 2000,
        },
    ));

    let removed = reg.remove_if_terminal(&TaskId::new("sub-y"));
    assert!(removed.is_none());
    assert_eq!(reg.total_count(), 1);
}

#[test]
fn test_cleanup_terminal() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "sub-1",
        "running",
        AgentStatus::Running {
            iteration: 0,
            sats_spent: 0,
        },
    ));
    reg.register(make_info(
        "sub-2",
        "done",
        AgentStatus::Completed {
            result: "ok".into(),
            sats_spent: 1000,
            iterations: 2,
        },
    ));
    reg.register(make_info("sub-3", "killed", AgentStatus::Killed));
    reg.register(make_info(
        "sub-4",
        "failed",
        AgentStatus::Failed {
            error: "err".into(),
            sats_spent: 300,
        },
    ));

    let cleaned = reg.cleanup_terminal();
    assert_eq!(cleaned.len(), 3); // completed, killed, failed
    assert_eq!(reg.total_count(), 1); // only running remains
    assert!(reg.contains(&TaskId::new("sub-1")));
}

#[test]
fn test_running_count() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "r1",
        "task",
        AgentStatus::Running {
            iteration: 0,
            sats_spent: 0,
        },
    ));
    reg.register(make_info(
        "r2",
        "task",
        AgentStatus::Running {
            iteration: 0,
            sats_spent: 0,
        },
    ));
    reg.register(make_info("d1", "task", AgentStatus::Killed));

    assert_eq!(reg.running_count(), 2);
    assert_eq!(reg.total_count(), 3);
}

#[test]
fn test_total_sats_spent() {
    let mut reg = TaskRegistry::new();
    reg.register(make_info(
        "a",
        "task",
        AgentStatus::Running {
            iteration: 3,
            sats_spent: 1000,
        },
    ));
    reg.register(make_info(
        "b",
        "task",
        AgentStatus::Completed {
            result: "ok".into(),
            sats_spent: 5000,
            iterations: 10,
        },
    ));
    reg.register(make_info("c", "task", AgentStatus::Killed));

    assert_eq!(reg.total_sats_spent(), 6000); // 1000 + 5000 + 0
}

#[test]
fn test_default_empty() {
    let reg = TaskRegistry::default();
    assert_eq!(reg.total_count(), 0);
    assert_eq!(reg.running_count(), 0);
    assert_eq!(reg.total_sats_spent(), 0);
}
