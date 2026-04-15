//! Tests for error module — DmError hierarchy.
//! Ensures all error variants can be constructed and formatted.

use dolphin_milk::error::{DmError, DmResult, ErrorContext};
use serde_json::json;

#[test]
fn test_wallet_error() {
    let e = DmError::wallet("connection failed");
    assert!(e.to_string().contains("connection failed"));
    assert!(e.context().is_empty());
}

#[test]
fn test_wallet_error_with_context() {
    let mut ctx = ErrorContext::new();
    ctx.insert("url".to_string(), json!("http://localhost:3322"));
    let e = DmError::wallet_with("timeout", ctx);
    assert!(e.to_string().contains("timeout"));
    assert!(e.context().contains_key("url"));
}

#[test]
fn test_payment_error() {
    let e = DmError::payment("402 not handled");
    assert!(e.to_string().contains("402 not handled"));
}

#[test]
fn test_payment_error_with_context() {
    let mut ctx = ErrorContext::new();
    ctx.insert("satoshis".to_string(), json!(5000));
    let e = DmError::payment_with("insufficient funds", ctx);
    assert!(e.to_string().contains("insufficient funds"));
    assert!(e.context().contains_key("satoshis"));
}

#[test]
fn test_tool_error() {
    let e = DmError::tool("unknown tool: foo");
    assert!(e.to_string().contains("unknown tool: foo"));
}

#[test]
fn test_budget_error() {
    let e = DmError::budget("exceeded 50000 sats");
    assert!(e.to_string().contains("exceeded 50000 sats"));
}

#[test]
fn test_config_error() {
    let e = DmError::config("invalid TOML");
    assert!(e.to_string().contains("invalid TOML"));
}

#[test]
fn test_loop_error() {
    let e = DmError::loop_err("circuit breaker tripped");
    assert!(e.to_string().contains("circuit breaker tripped"));
}

#[test]
fn test_error_display_format() {
    let e = DmError::wallet("test");
    assert_eq!(format!("{e}"), "wallet error: test");

    let e = DmError::payment("test");
    assert_eq!(format!("{e}"), "payment error: test");

    let e = DmError::tool("test");
    assert_eq!(format!("{e}"), "tool error: test");

    let e = DmError::budget("test");
    assert_eq!(format!("{e}"), "budget error: test");

    let e = DmError::config("test");
    assert_eq!(format!("{e}"), "config error: test");

    let e = DmError::loop_err("test");
    assert_eq!(format!("{e}"), "loop error: test");
}

#[test]
fn test_worm_result_type() {
    let ok: DmResult<i32> = Ok(42);
    assert!(ok.is_ok());

    let err: DmResult<i32> = Err(DmError::budget("out of money"));
    assert!(err.is_err());
}

#[test]
fn test_error_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    // DmError should be Send + Sync for async usage
    // This is a compile-time check
    assert_send_sync::<DmError>();
}

#[test]
fn test_error_matches() {
    let e = DmError::budget("test");
    match &e {
        DmError::Budget { message, .. } => assert_eq!(message, "test"),
        _ => panic!("wrong variant"),
    }
}
