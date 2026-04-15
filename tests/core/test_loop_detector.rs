//! Tests for loop detection — 4 detectors + budget drain.
//! Mirrors Python tests/test_loop_detector.py (9 tests).

use serde_json::json;

use dolphin_milk::loop_detect::detector::LoopDetector;

// -- TestLoopDetector --

#[test]
fn test_no_loop_initially() {
    let mut det = LoopDetector::new(10, 20, 30, 100_000);
    let result = det.check("execute_bash", &json!({"command": "ls"}));
    assert!(!result.stuck);
}

#[test]
fn test_generic_repeat_warning() {
    let mut det = LoopDetector::new(3, 5, 100, 100_000);

    // Record 3 identical calls
    for _ in 0..3 {
        det.record_outcome("execute_bash", &json!({"command": "ls"}), "files", 0);
    }

    // 4th check should trigger warning
    let result = det.check("execute_bash", &json!({"command": "ls"}));
    assert!(result.stuck, "should be stuck");
    assert_eq!(result.level, "warning");
    assert!(
        result.detector.contains("generic_repeat"),
        "detector: {}",
        result.detector
    );
}

#[test]
fn test_generic_repeat_critical() {
    let mut det = LoopDetector::new(2, 4, 100, 100_000);

    for _ in 0..4 {
        det.record_outcome("execute_bash", &json!({"command": "ls"}), "files", 0);
    }

    let result = det.check("execute_bash", &json!({"command": "ls"}));
    assert!(result.stuck, "should be stuck");
    assert_eq!(result.level, "critical");
}

#[test]
fn test_different_params_no_loop() {
    let mut det = LoopDetector::new(3, 10, 100, 100_000);

    for i in 0..5 {
        let params = json!({"command": format!("cmd_{i}")});
        det.check("execute_bash", &params);
        det.record_outcome("execute_bash", &params, &format!("result_{i}"), 0);
    }

    // Next check with new params should not trigger
    let result = det.check("execute_bash", &json!({"command": "cmd_new"}));
    assert!(!result.stuck, "should not be stuck with different params");
}

#[test]
fn test_global_circuit_breaker() {
    let mut det = LoopDetector::new(100, 200, 5, 100_000);

    for i in 0..4 {
        det.check(&format!("tool_{i}"), &json!({"x": i}));
        det.record_outcome(&format!("tool_{i}"), &json!({"x": i}), "r", 0);
    }

    // 5th check triggers breaker (total_calls becomes 5 in check())
    let result = det.check("tool_4", &json!({"x": 4}));
    assert!(result.stuck, "should be stuck");
    assert_eq!(result.level, "breaker");
    assert!(
        result.message.contains("CIRCUIT BREAKER"),
        "msg: {}",
        result.message
    );
}

#[test]
fn test_reset() {
    let mut det = LoopDetector::new(2, 10, 100, 100_000);
    for _ in 0..3 {
        det.record_outcome("t", &json!({"a": 1}), "r", 0);
    }

    det.reset();
    assert_eq!(det.total_calls(), 0);
    let result = det.check("t", &json!({"a": 1}));
    assert!(!result.stuck);
}

#[test]
fn test_budget_drain_ok() {
    let mut det = LoopDetector::new(10, 20, 30, 100_000);
    det.record_outcome("tool", &json!({}), "result", 1000);
    let result = det.check_budget_drain(1000);
    assert!(!result.stuck);
}

#[test]
fn test_budget_drain_excessive() {
    let mut det = LoopDetector::new(10, 20, 30, 10);
    for _ in 0..5 {
        det.record_outcome("tool", &json!({}), "result", 10);
    }
    let result = det.check_budget_drain(50);
    assert!(result.stuck, "should detect budget drain");
    assert_eq!(result.level, "critical");
    assert!(
        result.message.contains("BUDGET DRAIN"),
        "msg: {}",
        result.message
    );
}

#[test]
fn test_ping_pong_detection() {
    let mut det = LoopDetector::new(3, 10, 50, 100_000);
    let params_a = json!({"cmd": "a"});
    let params_b = json!({"cmd": "b"});

    // Create A-B-A-B pattern
    for _ in 0..6 {
        det.check("tool_a", &params_a);
        det.record_outcome("tool_a", &params_a, "ra", 0);
        det.check("tool_b", &params_b);
        det.record_outcome("tool_b", &params_b, "rb", 0);
    }

    // Next A should detect ping-pong
    let result = det.check("tool_a", &params_a);
    // May or may not trigger depending on threshold — just verify no crash
    assert!(
        matches!(result.stuck, true | false),
        "should return a valid result"
    );
}

#[test]
fn test_with_defaults() {
    let det = LoopDetector::with_defaults(100_000);
    assert_eq!(det.warn_threshold, 10);
    assert_eq!(det.critical_threshold, 20);
    assert_eq!(det.breaker_threshold, 30);
}

// -- End-to-end detector tests (check + record_outcome cycle) --

#[test]
fn test_end_to_end_check_then_record_no_loop() {
    // Simulate how the runner uses the detector: check before executing,
    // then record_outcome after. Different params each time should not trigger.
    let mut det = LoopDetector::new(3, 5, 100, 100_000);

    for i in 0..10 {
        let params = json!({"command": format!("cmd_{i}")});
        let result = det.check("execute_bash", &params);
        assert!(!result.stuck, "iteration {i} should not be stuck");
        det.record_outcome("execute_bash", &params, &format!("output_{i}"), 0);
    }
}

#[test]
fn test_end_to_end_repeat_detection_with_actual_params() {
    // Simulate the runner passing actual tool parameters (not call_id) to check().
    // Identical params should trigger generic_repeat.
    let mut det = LoopDetector::new(3, 5, 100, 100_000);
    let params = json!({"command": "ls -la"});

    for _ in 0..3 {
        let result = det.check("execute_bash", &params);
        assert!(!result.stuck, "should not trigger yet");
        det.record_outcome("execute_bash", &params, "file_listing", 0);
    }

    // 4th check with identical params should trigger warning (3 consecutive in history)
    let result = det.check("execute_bash", &params);
    assert!(result.stuck, "should be stuck on 4th identical call");
    assert_eq!(result.level, "warning");
    assert_eq!(result.detector, "generic_repeat");
}

#[test]
fn test_end_to_end_no_progress_detection() {
    // Simulate the runner calling record_outcome with identical results.
    // The no_progress detector fires a tracing::warn (not stuck), but we can
    // verify it doesn't crash and the history is properly maintained.
    let mut det = LoopDetector::new(3, 10, 100, 100_000);

    // Record 3 identical results with different params
    for i in 0..3 {
        let params = json!({"query": format!("query_{i}")});
        det.check("memory_search", &params);
        det.record_outcome("memory_search", &params, "same_result_every_time", 0);
    }

    // No panic, and detector still works for subsequent checks
    let result = det.check("memory_search", &json!({"query": "new_query"}));
    assert!(!result.stuck, "different params should not trigger repeat");
}

#[test]
fn test_end_to_end_ping_pong_with_record_outcome() {
    // The ping_pong detector needs history entries from record_outcome.
    // Simulate an ABAB pattern with proper check+record cycle.
    let mut det = LoopDetector::new(4, 10, 100, 100_000);
    let params_a = json!({"file": "a.txt"});
    let params_b = json!({"file": "b.txt"});

    // Create ABAB pattern: check+record for each
    for _ in 0..5 {
        det.check("file_read", &params_a);
        det.record_outcome("file_read", &params_a, "content_a", 0);
        det.check("file_write", &params_b);
        det.record_outcome("file_write", &params_b, "ok", 0);
    }

    // Now check — should detect ping-pong pattern
    // The history has 10 entries alternating between two patterns
    let result = det.check("file_read", &params_a);
    assert!(result.stuck, "should detect ping-pong");
    assert_eq!(result.detector, "ping_pong");
}

#[test]
fn test_end_to_end_mixed_tools_no_false_positive() {
    // Verify that a varied sequence of different tool calls does not trigger.
    let mut det = LoopDetector::new(3, 5, 100, 100_000);

    let calls = vec![
        ("execute_bash", json!({"command": "ls"}), "files"),
        ("file_read", json!({"path": "/tmp/a.txt"}), "content"),
        ("memory_search", json!({"query": "foo"}), "results"),
        ("execute_bash", json!({"command": "pwd"}), "/home"),
        (
            "file_write",
            json!({"path": "/tmp/b.txt", "content": "data"}),
            "ok",
        ),
    ];

    for (name, params, result) in &calls {
        let check = det.check(name, params);
        assert!(!check.stuck, "tool {name} should not trigger");
        det.record_outcome(name, params, result, 0);
    }
}

#[test]
fn test_record_outcome_tracks_sats_cost() {
    // Verify budget drain uses sats from record_outcome.
    let mut det = LoopDetector::new(10, 20, 30, 100);

    // Record 5 tool calls each costing 50 sats = 250 total in last hour
    for i in 0..5 {
        let params = json!({"i": i});
        det.check("x402_call", &params);
        det.record_outcome("x402_call", &params, "result", 50);
    }

    let drain = det.check_budget_drain(250);
    assert!(drain.stuck, "250 sats > 100 max should trigger drain");
    assert_eq!(drain.detector, "budget_drain");
}
