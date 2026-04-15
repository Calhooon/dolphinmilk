//! Tests for SpawnConfig validation.

use dolphin_milk::orchestration::types::{AgentStatus, SpawnConfig};

// -- SpawnConfig validation --

#[test]
fn test_valid_config_minimal() {
    let config = SpawnConfig {
        task: "Research BSV fees".into(),
        budget_sats: 10000,
        allowed_tools: None,
        max_iterations: None,
        model: None,
    };
    assert!(config.validate().is_ok());
}

#[test]
fn test_valid_config_full() {
    let config = SpawnConfig {
        task: "Analyze costs".into(),
        budget_sats: 50000,
        allowed_tools: Some(vec!["web_fetch".into(), "memory_search".into()]),
        max_iterations: Some(10),
        model: Some("gpt-5-mini".into()),
    };
    assert!(config.validate().is_ok());
}

#[test]
fn test_empty_task_rejected() {
    let config = SpawnConfig {
        task: "".into(),
        budget_sats: 10000,
        allowed_tools: None,
        max_iterations: None,
        model: None,
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("task must not be empty"), "got: {err}");
}

#[test]
fn test_whitespace_task_rejected() {
    let config = SpawnConfig {
        task: "   ".into(),
        budget_sats: 10000,
        allowed_tools: None,
        max_iterations: None,
        model: None,
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("task must not be empty"), "got: {err}");
}

#[test]
fn test_zero_budget_rejected() {
    let config = SpawnConfig {
        task: "Do something".into(),
        budget_sats: 0,
        allowed_tools: None,
        max_iterations: None,
        model: None,
    };
    let err = config.validate().unwrap_err();
    assert!(
        err.contains("budget_sats must be greater than 0"),
        "got: {err}"
    );
}

#[test]
fn test_empty_allowed_tools_rejected() {
    let config = SpawnConfig {
        task: "Do something".into(),
        budget_sats: 10000,
        allowed_tools: Some(vec![]),
        max_iterations: None,
        model: None,
    };
    let err = config.validate().unwrap_err();
    assert!(
        err.contains("allowed_tools must not be empty"),
        "got: {err}"
    );
}

#[test]
fn test_empty_string_in_allowed_tools_rejected() {
    let config = SpawnConfig {
        task: "Do something".into(),
        budget_sats: 10000,
        allowed_tools: Some(vec!["web_fetch".into(), "".into()]),
        max_iterations: None,
        model: None,
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("must not contain empty strings"), "got: {err}");
}

#[test]
fn test_zero_max_iterations_rejected() {
    let config = SpawnConfig {
        task: "Do something".into(),
        budget_sats: 10000,
        allowed_tools: None,
        max_iterations: Some(0),
        model: None,
    };
    let err = config.validate().unwrap_err();
    assert!(
        err.contains("max_iterations must be greater than 0"),
        "got: {err}"
    );
}

#[test]
fn test_config_serde_roundtrip() {
    let config = SpawnConfig {
        task: "Research BSV".into(),
        budget_sats: 25000,
        allowed_tools: Some(vec!["web_fetch".into()]),
        max_iterations: Some(15),
        model: Some("claude-haiku-4-5-20251001".into()),
    };
    let json = serde_json::to_string(&config).unwrap();
    let parsed: SpawnConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.task, "Research BSV");
    assert_eq!(parsed.budget_sats, 25000);
    assert_eq!(parsed.allowed_tools.as_ref().unwrap().len(), 1);
    assert_eq!(parsed.max_iterations, Some(15));
    assert_eq!(parsed.model.as_deref(), Some("claude-haiku-4-5-20251001"));
}

// -- AgentStatus --

#[test]
fn test_running_is_not_terminal() {
    let status = AgentStatus::Running {
        iteration: 3,
        sats_spent: 1500,
    };
    assert!(!status.is_terminal());
    assert_eq!(status.sats_spent(), 1500);
}

#[test]
fn test_completed_is_terminal() {
    let status = AgentStatus::Completed {
        result: "Done!".into(),
        sats_spent: 5000,
        iterations: 10,
    };
    assert!(status.is_terminal());
    assert_eq!(status.sats_spent(), 5000);
}

#[test]
fn test_failed_is_terminal() {
    let status = AgentStatus::Failed {
        error: "budget exceeded".into(),
        sats_spent: 3000,
    };
    assert!(status.is_terminal());
    assert_eq!(status.sats_spent(), 3000);
}

#[test]
fn test_killed_is_terminal() {
    let status = AgentStatus::Killed;
    assert!(status.is_terminal());
    assert_eq!(status.sats_spent(), 0);
}

#[test]
fn test_agent_status_serde_running() {
    let status = AgentStatus::Running {
        iteration: 5,
        sats_spent: 2000,
    };
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("\"status\":\"running\""), "got: {json}");
    let parsed: AgentStatus = serde_json::from_str(&json).unwrap();
    match parsed {
        AgentStatus::Running {
            iteration,
            sats_spent,
        } => {
            assert_eq!(iteration, 5);
            assert_eq!(sats_spent, 2000);
        }
        _ => panic!("expected Running"),
    }
}

#[test]
fn test_agent_status_serde_completed() {
    let status = AgentStatus::Completed {
        result: "Summary here".into(),
        sats_spent: 8000,
        iterations: 20,
    };
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("\"status\":\"completed\""), "got: {json}");
    let parsed: AgentStatus = serde_json::from_str(&json).unwrap();
    match parsed {
        AgentStatus::Completed {
            result,
            sats_spent,
            iterations,
        } => {
            assert_eq!(result, "Summary here");
            assert_eq!(sats_spent, 8000);
            assert_eq!(iterations, 20);
        }
        _ => panic!("expected Completed"),
    }
}
