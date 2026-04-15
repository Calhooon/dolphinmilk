//! Tests for auto-escalation detection.

use dolphin_milk::runner::escalation::{
    EscalationDetector, EscalationEvent, EscalationReason, EscalationResolution, EscalationStatus,
};

// -- Tool loop detection --

#[test]
fn test_loop_detection_triggers_at_threshold() {
    let mut det = EscalationDetector::new();
    let args = r#"{"query": "test"}"#;

    assert!(det.record_tool_call("memory_search", args).is_none());
    assert!(det.record_tool_call("memory_search", args).is_none());

    let reason = det.record_tool_call("memory_search", args);
    assert!(reason.is_some());

    match reason.unwrap() {
        EscalationReason::ToolLoop {
            tool_name,
            call_count,
        } => {
            assert_eq!(tool_name, "memory_search");
            assert_eq!(call_count, 3);
        }
        _ => panic!("Expected ToolLoop"),
    }
}

#[test]
fn test_different_params_no_loop() {
    let mut det = EscalationDetector::new();
    assert!(det.record_tool_call("search", r#"{"q":"a"}"#).is_none());
    assert!(det.record_tool_call("search", r#"{"q":"b"}"#).is_none());
    assert!(det.record_tool_call("search", r#"{"q":"c"}"#).is_none());
    // No loop because params differ
}

#[test]
fn test_different_tools_no_loop() {
    let mut det = EscalationDetector::new();
    let args = r#"{"x": 1}"#;
    assert!(det.record_tool_call("tool_a", args).is_none());
    assert!(det.record_tool_call("tool_b", args).is_none());
    assert!(det.record_tool_call("tool_c", args).is_none());
    // No loop because tool names differ
}

#[test]
fn test_loop_continues_counting() {
    let mut det = EscalationDetector::new();
    let args = "same";

    for _ in 0..5 {
        det.record_tool_call("x", args);
    }

    // After mark_triggered is NOT called, further calls still detect
    // But once triggered, they don't:
    let mut det2 = EscalationDetector::new();
    det2.record_tool_call("x", args);
    det2.record_tool_call("x", args);
    let r = det2.record_tool_call("x", args);
    assert!(r.is_some());
    det2.mark_triggered();
    assert!(det2.record_tool_call("x", args).is_none());
}

// -- Budget threshold --

#[test]
fn test_budget_threshold_triggers() {
    let det = EscalationDetector::new();
    let reason = det.check_budget(8000, 10000);
    assert!(reason.is_some());
    match reason.unwrap() {
        EscalationReason::BudgetThreshold {
            spent,
            limit,
            percent,
        } => {
            assert_eq!(spent, 8000);
            assert_eq!(limit, 10000);
            assert!((percent - 80.0).abs() < f64::EPSILON);
        }
        _ => panic!("Expected BudgetThreshold"),
    }
}

#[test]
fn test_budget_below_threshold_ok() {
    let det = EscalationDetector::new();
    assert!(det.check_budget(7999, 10000).is_none());
}

#[test]
fn test_budget_above_threshold() {
    let det = EscalationDetector::new();
    let reason = det.check_budget(9500, 10000);
    assert!(reason.is_some());
    match reason.unwrap() {
        EscalationReason::BudgetThreshold { percent, .. } => {
            assert!(percent > 80.0);
        }
        _ => panic!("Expected BudgetThreshold"),
    }
}

#[test]
fn test_budget_zero_limit_no_escalation() {
    let det = EscalationDetector::new();
    assert!(det.check_budget(500, 0).is_none());
}

// -- Error count threshold --

#[test]
fn test_error_count_triggers() {
    let mut det = EscalationDetector::new();
    assert!(det.record_error().is_none()); // 1
    assert!(det.record_error().is_none()); // 2
    assert!(det.record_error().is_none()); // 3
    let reason = det.record_error(); // 4 > threshold of 3
    assert!(reason.is_some());
    match reason.unwrap() {
        EscalationReason::ErrorThreshold { error_count } => {
            assert_eq!(error_count, 4);
        }
        _ => panic!("Expected ErrorThreshold"),
    }
}

#[test]
fn test_error_count_below_threshold() {
    let mut det = EscalationDetector::new();
    assert!(det.record_error().is_none());
    assert!(det.record_error().is_none());
    assert!(det.record_error().is_none());
    assert_eq!(det.error_count(), 3);
}

// -- Explicit uncertainty --

#[test]
fn test_explicit_uncertainty_triggers() {
    let det = EscalationDetector::new();
    let reason = det.check_text("I'm stuck and can't figure this out");
    assert!(reason.is_some());
    match reason.unwrap() {
        EscalationReason::ExplicitUncertainty { trigger_phrase } => {
            assert_eq!(trigger_phrase, "i'm stuck");
        }
        _ => panic!("Expected ExplicitUncertainty"),
    }
}

#[test]
fn test_need_help_detected() {
    let det = EscalationDetector::new();
    let reason = det.check_text("I need help with this complex task");
    assert!(reason.is_some());
}

#[test]
fn test_escalate_to_human_detected() {
    let det = EscalationDetector::new();
    let reason = det.check_text("I think we should escalate to human for this");
    assert!(reason.is_some());
}

#[test]
fn test_normal_text_no_escalation() {
    let det = EscalationDetector::new();
    assert!(det.check_text("I found the answer to be 42").is_none());
    assert!(det.check_text("The task is complete").is_none());
    assert!(det.check_text("Here are the results").is_none());
}

#[test]
fn test_uncertainty_case_insensitive() {
    let det = EscalationDetector::new();
    assert!(det.check_text("I NEED HELP").is_some());
    assert!(det.check_text("I'M STUCK").is_some());
}

// -- Escalation pauses task --

#[test]
fn test_triggered_prevents_further_escalation() {
    let mut det = EscalationDetector::new();
    det.mark_triggered();

    assert!(det.record_tool_call("x", "y").is_none());
    assert!(det.record_error().is_none());
    assert!(det.check_budget(9999, 10000).is_none());
    assert!(det.check_text("i'm stuck").is_none());
}

#[test]
fn test_reset_clears_state() {
    let mut det = EscalationDetector::new();
    det.record_error();
    det.record_error();
    det.mark_triggered();

    det.reset();

    assert!(!det.is_triggered());
    assert_eq!(det.error_count(), 0);
    // Should be able to trigger again after reset
    assert!(det.check_text("i need help").is_some());
}

// -- Event building --

#[test]
fn test_build_event_tool_loop() {
    let reason = EscalationReason::ToolLoop {
        tool_name: "memory_search".to_string(),
        call_count: 5,
    };
    let event = EscalationDetector::build_event(reason, 7);

    assert_eq!(event.iteration, 7);
    assert!(event.message.contains("memory_search"));
    assert!(event.message.contains("5 times"));
    assert!(event.should_pause);
    assert!(!event.timestamp.is_empty());
}

#[test]
fn test_build_event_budget() {
    let reason = EscalationReason::BudgetThreshold {
        spent: 9000,
        limit: 10000,
        percent: 90.0,
    };
    let event = EscalationDetector::build_event(reason, 3);
    assert!(event.message.contains("90.0%"));
}

#[test]
fn test_build_event_error_threshold() {
    let reason = EscalationReason::ErrorThreshold { error_count: 5 };
    let event = EscalationDetector::build_event(reason, 2);
    assert!(event.message.contains("5 errors"));
}

#[test]
fn test_build_event_uncertainty() {
    let reason = EscalationReason::ExplicitUncertainty {
        trigger_phrase: "i'm stuck".to_string(),
    };
    let event = EscalationDetector::build_event(reason, 1);
    assert!(event.message.contains("i'm stuck"));
}

// -- Serde roundtrips --

#[test]
fn test_escalation_reason_serde_roundtrip() {
    let reasons = vec![
        EscalationReason::ToolLoop {
            tool_name: "test".to_string(),
            call_count: 3,
        },
        EscalationReason::BudgetThreshold {
            spent: 8000,
            limit: 10000,
            percent: 80.0,
        },
        EscalationReason::ErrorThreshold { error_count: 4 },
        EscalationReason::ExplicitUncertainty {
            trigger_phrase: "help".to_string(),
        },
    ];

    for reason in reasons {
        let json = serde_json::to_string(&reason).unwrap();
        let parsed: EscalationReason = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reason);
    }
}

#[test]
fn test_escalation_event_serde() {
    let event = EscalationEvent {
        reason: EscalationReason::ErrorThreshold { error_count: 5 },
        iteration: 3,
        message: "Too many errors".to_string(),
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        should_pause: true,
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: EscalationEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.iteration, 3);
    assert!(parsed.should_pause);
    assert_eq!(parsed.message, "Too many errors");
}

#[test]
fn test_escalation_resolution_serde() {
    let resolution = EscalationResolution {
        guidance: "Try approach B instead".to_string(),
        resolved_by: "operator-key".to_string(),
        timestamp: "2026-03-23T12:00:00Z".to_string(),
    };

    let json = serde_json::to_string(&resolution).unwrap();
    let parsed: EscalationResolution = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.guidance, "Try approach B instead");
    assert_eq!(parsed.resolved_by, "operator-key");
}

#[test]
fn test_escalation_status_default() {
    let status = EscalationStatus::default();
    assert!(!status.escalated);
    assert!(status.event.is_none());
    assert!(status.resolution.is_none());
}

#[test]
fn test_escalation_status_serde() {
    let status = EscalationStatus {
        escalated: true,
        event: Some(EscalationEvent {
            reason: EscalationReason::ErrorThreshold { error_count: 4 },
            iteration: 2,
            message: "test".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            should_pause: true,
        }),
        resolution: None,
    };

    let json = serde_json::to_string(&status).unwrap();
    let parsed: EscalationStatus = serde_json::from_str(&json).unwrap();
    assert!(parsed.escalated);
    assert!(parsed.event.is_some());
    assert!(parsed.resolution.is_none());
}
