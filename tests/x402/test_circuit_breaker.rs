//! Tests for x402 circuit breaker — state machine, error classification,
//! per-endpoint isolation, and thread safety.

use std::time::Duration;

use dolphin_milk::error::DmError;
use dolphin_milk::x402::circuit_breaker::{
    is_transient_message, CircuitBreaker, CircuitBreakerRegistry, CircuitState,
    DEFAULT_FAILURE_THRESHOLD, DEFAULT_RECOVERY_WINDOW,
};

/// Determine whether a DmError is transient by checking its message.
/// This wraps the crate's `is_transient_message` for backward compat in tests.
fn is_transient_error(err: &DmError) -> bool {
    is_transient_message(&err.to_string())
}

// ---------------------------------------------------------------------------
// Helper: make a ThinkResult-compatible error of different types
// ---------------------------------------------------------------------------

fn payment_err(msg: &str) -> DmError {
    DmError::payment(msg)
}

fn budget_err(msg: &str) -> DmError {
    DmError::budget(msg)
}

// ---------------------------------------------------------------------------
// 1. test_circuit_starts_closed
// ---------------------------------------------------------------------------
#[test]
fn test_circuit_starts_closed() {
    let cb = CircuitBreaker::default();
    assert_eq!(cb.state, CircuitState::Closed);
    assert_eq!(cb.failure_count, 0);
}

// ---------------------------------------------------------------------------
// 2. test_single_failure_stays_closed
// ---------------------------------------------------------------------------
#[test]
fn test_single_failure_stays_closed() {
    let mut cb = CircuitBreaker::default();
    cb.record_failure(true); // one transient failure
    assert_eq!(cb.state, CircuitState::Closed);
    assert_eq!(cb.failure_count, 1);
    assert!(cb.should_allow());
}

// ---------------------------------------------------------------------------
// 3. test_threshold_failures_opens_circuit
// ---------------------------------------------------------------------------
#[test]
fn test_threshold_failures_opens_circuit() {
    let mut cb = CircuitBreaker::default();
    for _ in 0..DEFAULT_FAILURE_THRESHOLD {
        cb.record_failure(true);
    }
    assert!(matches!(cb.state, CircuitState::Open { .. }));
    assert_eq!(cb.failure_count, DEFAULT_FAILURE_THRESHOLD);
}

// ---------------------------------------------------------------------------
// 4. test_open_circuit_rejects_calls
// ---------------------------------------------------------------------------
#[test]
fn test_open_circuit_rejects_calls() {
    let mut cb = CircuitBreaker::new(2, Duration::from_secs(60));
    cb.record_failure(true);
    cb.record_failure(true);
    assert!(matches!(cb.state, CircuitState::Open { .. }));
    assert!(!cb.should_allow()); // should be rejected
}

// ---------------------------------------------------------------------------
// 5. test_recovery_window_transitions_to_half_open
// ---------------------------------------------------------------------------
#[test]
fn test_recovery_window_transitions_to_half_open() {
    let mut cb = CircuitBreaker::new(1, Duration::from_millis(1));
    cb.record_failure(true); // opens circuit
    assert!(matches!(cb.state, CircuitState::Open { .. }));

    // Wait for recovery window (1ms — virtually instant)
    std::thread::sleep(Duration::from_millis(5));

    assert!(cb.should_allow()); // transitions to HalfOpen
    assert_eq!(cb.state, CircuitState::HalfOpen);
}

// ---------------------------------------------------------------------------
// 6. test_half_open_success_closes_circuit
// ---------------------------------------------------------------------------
#[test]
fn test_half_open_success_closes_circuit() {
    let mut cb = CircuitBreaker::new(1, Duration::from_millis(1));
    cb.record_failure(true);
    std::thread::sleep(Duration::from_millis(5));
    cb.should_allow(); // → HalfOpen
    assert_eq!(cb.state, CircuitState::HalfOpen);

    cb.record_success();
    assert_eq!(cb.state, CircuitState::Closed);
    assert_eq!(cb.failure_count, 0);
}

// ---------------------------------------------------------------------------
// 7. test_half_open_failure_reopens_circuit
// ---------------------------------------------------------------------------
#[test]
fn test_half_open_failure_reopens_circuit() {
    let mut cb = CircuitBreaker::new(1, Duration::from_millis(1));
    cb.record_failure(true);
    std::thread::sleep(Duration::from_millis(5));
    cb.should_allow(); // → HalfOpen

    cb.record_failure(true);
    assert!(matches!(cb.state, CircuitState::Open { .. }));
}

// ---------------------------------------------------------------------------
// 8. test_client_errors_dont_trip_breaker
// ---------------------------------------------------------------------------
#[test]
fn test_client_errors_dont_trip_breaker() {
    let err_400 = payment_err("LLM call failed: HTTP 400 BAD_REQUEST: invalid model");
    assert!(!is_transient_error(&err_400));

    let err_401 = payment_err("LLM call failed: HTTP 401 UNAUTHORIZED: bad key");
    assert!(!is_transient_error(&err_401));

    let err_403 = payment_err("LLM call failed: HTTP 403 FORBIDDEN: access denied");
    assert!(!is_transient_error(&err_403));

    let err_404 = payment_err("LLM call failed: HTTP 404 NOT_FOUND: endpoint gone");
    assert!(!is_transient_error(&err_404));

    // 400-level errors should NOT open the circuit
    let mut cb = CircuitBreaker::new(1, Duration::from_secs(30));
    cb.record_failure(false); // non-transient
    assert_eq!(cb.state, CircuitState::Closed);
    assert_eq!(cb.failure_count, 0);
}

// ---------------------------------------------------------------------------
// 9. test_server_errors_trip_breaker
// ---------------------------------------------------------------------------
#[test]
fn test_server_errors_trip_breaker() {
    let err_500 = payment_err("LLM call failed: HTTP 500 INTERNAL_SERVER_ERROR: upstream crash");
    assert!(is_transient_error(&err_500));

    let err_502 = payment_err("LLM call failed: HTTP 502 BAD_GATEWAY: proxy down");
    assert!(is_transient_error(&err_502));

    let err_503 = payment_err("LLM call failed: HTTP 503 SERVICE_UNAVAILABLE: overloaded");
    assert!(is_transient_error(&err_503));

    let err_504 = payment_err("LLM call failed: HTTP 504 GATEWAY_TIMEOUT");
    assert!(is_transient_error(&err_504));
}

// ---------------------------------------------------------------------------
// 10. test_timeout_errors_trip_breaker
// ---------------------------------------------------------------------------
#[test]
fn test_timeout_errors_trip_breaker() {
    let err_timeout = payment_err("Request failed: timeout waiting for response");
    assert!(is_transient_error(&err_timeout));

    let err_conn = payment_err("Request failed: connection refused");
    assert!(is_transient_error(&err_conn));

    let err_reset = payment_err("Paid request failed: connection reset by peer");
    assert!(is_transient_error(&err_reset));

    let err_dns = payment_err("Request failed: dns resolution failed");
    assert!(is_transient_error(&err_dns));
}

// ---------------------------------------------------------------------------
// 11. test_per_endpoint_isolation
// ---------------------------------------------------------------------------
#[test]
fn test_per_endpoint_isolation() {
    let registry = CircuitBreakerRegistry::with_config(2, Duration::from_secs(60));

    let url_a = "https://openai-chat.x402agency.com/chat";
    let url_b = "https://claude-chat.x402agency.com/chat";

    // Trip circuit for url_a
    let err = payment_err("HTTP 500 INTERNAL_SERVER_ERROR");
    registry.record_failure_msg(url_a, &err.to_string());
    registry.record_failure_msg(url_a, &err.to_string());

    // url_a should be open, url_b should still be closed
    assert!(!registry.should_allow(url_a));
    assert!(registry.should_allow(url_b));
}

// ---------------------------------------------------------------------------
// 12. test_success_resets_failure_count
// ---------------------------------------------------------------------------
#[test]
fn test_success_resets_failure_count() {
    let mut cb = CircuitBreaker::new(5, Duration::from_secs(30));
    cb.record_failure(true);
    cb.record_failure(true);
    cb.record_failure(true);
    assert_eq!(cb.failure_count, 3);

    cb.record_success();
    assert_eq!(cb.failure_count, 0);
    assert_eq!(cb.state, CircuitState::Closed);
}

// ---------------------------------------------------------------------------
// 13. test_configurable_threshold
// ---------------------------------------------------------------------------
#[test]
fn test_configurable_threshold() {
    let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));
    cb.record_failure(true);
    cb.record_failure(true);
    assert_eq!(cb.state, CircuitState::Closed); // 2 < 3

    cb.record_failure(true);
    assert!(matches!(cb.state, CircuitState::Open { .. })); // 3 = threshold
}

// ---------------------------------------------------------------------------
// 14. test_configurable_recovery_window
// ---------------------------------------------------------------------------
#[test]
fn test_configurable_recovery_window() {
    // Short recovery window (1ms)
    let mut cb = CircuitBreaker::new(1, Duration::from_millis(1));
    cb.record_failure(true);
    assert!(matches!(cb.state, CircuitState::Open { .. }));

    std::thread::sleep(Duration::from_millis(5));
    assert!(cb.should_allow()); // recovery elapsed → HalfOpen

    // Long recovery window (10s) — should NOT transition yet
    let mut cb2 = CircuitBreaker::new(1, Duration::from_secs(10));
    cb2.record_failure(true);
    assert!(matches!(cb2.state, CircuitState::Open { .. }));
    assert!(!cb2.should_allow()); // 10s not elapsed
}

// ---------------------------------------------------------------------------
// 15. test_concurrent_access_safe
// ---------------------------------------------------------------------------
#[test]
fn test_concurrent_access_safe() {
    use std::sync::Arc;
    use std::thread;

    let registry = Arc::new(CircuitBreakerRegistry::with_config(
        100,
        Duration::from_secs(30),
    ));
    let url = "https://openai-chat.x402agency.com/chat";

    let mut handles = Vec::new();
    for _ in 0..10 {
        let reg = Arc::clone(&registry);
        let u = url.to_string();
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                reg.should_allow(&u);
                let err = payment_err("HTTP 500 INTERNAL_SERVER_ERROR");
                reg.record_failure_msg(&u, &err.to_string());
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Should not panic or deadlock — state is consistent
    let state = registry.get_state(url);
    // After 500 transient failures with threshold 100, circuit must be open
    assert!(matches!(state, CircuitState::Open { .. }));
}

// ---------------------------------------------------------------------------
// 16. test_rate_limit_429_is_transient
// ---------------------------------------------------------------------------
#[test]
fn test_rate_limit_429_is_transient() {
    let err = payment_err("LLM call failed: HTTP 429 TOO_MANY_REQUESTS: rate limit exceeded");
    assert!(is_transient_error(&err));

    let err2 = payment_err("Rate limit reached for model gpt-5");
    assert!(is_transient_error(&err2));
}

// ---------------------------------------------------------------------------
// 17. test_non_payment_errors_not_transient
// ---------------------------------------------------------------------------
#[test]
fn test_non_payment_errors_not_transient() {
    assert!(!is_transient_error(&budget_err("exceeded daily limit")));
    assert!(!is_transient_error(&DmError::config("bad toml")));
    assert!(!is_transient_error(&DmError::loop_err("stuck in loop")));
    assert!(!is_transient_error(&DmError::tool("file not found")));
    assert!(!is_transient_error(&DmError::memory("index corrupt")));
}

// ---------------------------------------------------------------------------
// 18. test_err_service_failed_is_transient
// ---------------------------------------------------------------------------
#[test]
fn test_err_service_failed_is_transient() {
    let err =
        payment_err("LLM call failed: HTTP 500 ERR_SERVICE_FAILED_REFUND_ISSUED: upstream timeout");
    assert!(is_transient_error(&err));
}

// ---------------------------------------------------------------------------
// 19. test_registry_base_url_extraction
// ---------------------------------------------------------------------------
#[test]
fn test_registry_base_url_extraction() {
    assert_eq!(
        CircuitBreakerRegistry::base_url("https://openai-chat.x402agency.com/chat"),
        "https://openai-chat.x402agency.com"
    );
    assert_eq!(
        CircuitBreakerRegistry::base_url("https://claude-chat.x402agency.com/chat"),
        "https://claude-chat.x402agency.com"
    );
    assert_eq!(
        CircuitBreakerRegistry::base_url("http://localhost:8080/v1/completions"),
        "http://localhost"
    );
}

// ---------------------------------------------------------------------------
// 20. test_default_constants
// ---------------------------------------------------------------------------
#[test]
fn test_default_constants() {
    assert_eq!(DEFAULT_FAILURE_THRESHOLD, 5);
    assert_eq!(DEFAULT_RECOVERY_WINDOW, Duration::from_secs(30));

    let cb = CircuitBreaker::default();
    assert_eq!(cb.threshold, 5);
    assert_eq!(cb.recovery_window, Duration::from_secs(30));
}

// ---------------------------------------------------------------------------
// 21. test_registry_get_state_unknown_endpoint
// ---------------------------------------------------------------------------
#[test]
fn test_registry_get_state_unknown_endpoint() {
    let registry = CircuitBreakerRegistry::new();
    // Unknown endpoint should return Closed
    assert_eq!(
        registry.get_state("https://unknown.example.com/api"),
        CircuitState::Closed
    );
}

// ---------------------------------------------------------------------------
// 22. test_mixed_transient_and_client_failures
// ---------------------------------------------------------------------------
#[test]
fn test_mixed_transient_and_client_failures() {
    let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));

    // Non-transient failures don't count
    cb.record_failure(false);
    cb.record_failure(false);
    cb.record_failure(false);
    assert_eq!(cb.state, CircuitState::Closed);
    assert_eq!(cb.failure_count, 0);

    // Now transient failures count
    cb.record_failure(true);
    cb.record_failure(true);
    assert_eq!(cb.failure_count, 2);
    assert_eq!(cb.state, CircuitState::Closed);

    cb.record_failure(true);
    assert!(matches!(cb.state, CircuitState::Open { .. }));
}

// ---------------------------------------------------------------------------
// 23. test_wallet_connection_error_is_transient
// ---------------------------------------------------------------------------
#[test]
fn test_wallet_connection_error_is_transient() {
    let err = DmError::wallet("connection refused to localhost:3322");
    assert!(is_transient_error(&err));
}

// ---------------------------------------------------------------------------
// 24. test_auth_network_error_is_transient
// ---------------------------------------------------------------------------
#[test]
fn test_auth_network_error_is_transient() {
    let err = DmError::auth("timeout during BRC-31 handshake");
    assert!(is_transient_error(&err));

    // Auth logic errors are NOT transient
    let err2 = DmError::auth("invalid signature");
    assert!(!is_transient_error(&err2));
}
