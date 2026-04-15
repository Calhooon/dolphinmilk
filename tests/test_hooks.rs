//! Tests for the lifecycle hook system.
//!
//! Covers registration, priority ordering, blocking, timeout enforcement,
//! command execution, config parsing, and edge cases.

use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Once;

use dolphin_milk::hooks::{
    config::HookConfigEntry,
    events::{HookEvent, HookEventType},
    executor::{is_private_ip, is_private_url, HookHandler, HookResult},
    is_blocked, HookRegistration, HookRegistry,
};

/// Set DOLPHIN_MILK_HOOK_SSRF_BYPASS so that mockito-based HTTP handler tests
/// can reach the local mock server (127.0.0.1). The SSRF guard logic
/// itself is tested via the `is_private_ip` and `is_private_url` unit tests.
static INIT: Once = Once::new();
fn enable_ssrf_bypass() {
    INIT.call_once(|| {
        std::env::set_var("DOLPHIN_MILK_HOOK_SSRF_BYPASS", "1");
    });
}

// =============================================================================
// Registration
// =============================================================================

#[test]
fn test_registry_new_is_empty() {
    let registry = HookRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.list().is_empty());
}

#[test]
fn test_register_and_list() {
    let mut registry = HookRegistry::new();
    registry.register(make_hook("hook-1", HookEventType::TaskStarted, 10));
    registry.register(make_hook("hook-2", HookEventType::TaskCompleted, 20));

    assert_eq!(registry.len(), 2);
    assert!(!registry.is_empty());
    assert_eq!(registry.list()[0].id, "hook-1");
    assert_eq!(registry.list()[1].id, "hook-2");
}

#[test]
fn test_unregister_existing() {
    let mut registry = HookRegistry::new();
    registry.register(make_hook("hook-1", HookEventType::TaskStarted, 10));
    registry.register(make_hook("hook-2", HookEventType::TaskCompleted, 20));

    assert!(registry.unregister("hook-1"));
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.list()[0].id, "hook-2");
}

#[test]
fn test_unregister_nonexistent() {
    let mut registry = HookRegistry::new();
    registry.register(make_hook("hook-1", HookEventType::TaskStarted, 10));

    assert!(!registry.unregister("hook-999"));
    assert_eq!(registry.len(), 1);
}

#[test]
fn test_count_for_event() {
    let mut registry = HookRegistry::new();
    registry.register(make_hook("h1", HookEventType::PreToolExecution, 10));
    registry.register(make_hook("h2", HookEventType::PreToolExecution, 20));
    registry.register(make_hook("h3", HookEventType::TaskStarted, 30));

    assert_eq!(
        registry.count_for_event(&HookEventType::PreToolExecution),
        2
    );
    assert_eq!(registry.count_for_event(&HookEventType::TaskStarted), 1);
    assert_eq!(registry.count_for_event(&HookEventType::TaskFailed), 0);
}

// =============================================================================
// fire() — empty and priority
// =============================================================================

#[tokio::test]
async fn test_fire_empty_registry_returns_empty() {
    let mut registry = HookRegistry::new();
    let results = registry
        .fire(HookEvent::TaskStarted {
            task_id: "t1".into(),
        })
        .await;
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_fire_no_matching_hooks_returns_empty() {
    let mut registry = HookRegistry::new();
    registry.register(make_hook("h1", HookEventType::TaskCompleted, 10));

    let results = registry
        .fire(HookEvent::TaskStarted {
            task_id: "t1".into(),
        })
        .await;
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_fire_priority_ordering() {
    // Register hooks with priorities 30, 10, 20
    // They should fire in order: 10, 20, 30
    let mut registry = HookRegistry::new();
    registry.register(make_echo_hook(
        "h-30",
        HookEventType::PreToolExecution,
        30,
        "echo 30",
    ));
    registry.register(make_echo_hook(
        "h-10",
        HookEventType::PreToolExecution,
        10,
        "echo 10",
    ));
    registry.register(make_echo_hook(
        "h-20",
        HookEventType::PreToolExecution,
        20,
        "echo 20",
    ));

    let results = registry
        .fire(HookEvent::PreToolExecution {
            tool_name: "test".into(),
            parameters: json!({}),
        })
        .await;

    assert_eq!(results.len(), 3);
    // All should be Allow (exit 0)
    for r in &results {
        assert_eq!(*r, HookResult::Allow);
    }
}

// =============================================================================
// Blocking hooks
// =============================================================================

#[tokio::test]
async fn test_blocking_hook_prevents_subsequent() {
    let mut registry = HookRegistry::new();

    // First hook (priority 1): blocking, exits 1 (block)
    registry.register(HookRegistration {
        id: "blocker".into(),
        event_type: HookEventType::PreToolExecution,
        handler: HookHandler::Command {
            cmd: "echo 'blocked by policy'; exit 1".into(),
            timeout_ms: 5000,
        },
        priority: 1,
        blocking: true,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    // Second hook (priority 2): should never fire
    registry.register(HookRegistration {
        id: "after-blocker".into(),
        event_type: HookEventType::PreToolExecution,
        handler: HookHandler::Command {
            cmd: "echo 'should not run'".into(),
            timeout_ms: 5000,
        },
        priority: 2,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    let results = registry
        .fire(HookEvent::PreToolExecution {
            tool_name: "test".into(),
            parameters: json!({}),
        })
        .await;

    // Only one result — the blocker prevented the second hook
    assert_eq!(results.len(), 1);
    assert!(matches!(&results[0], HookResult::Block(reason) if reason == "blocked by policy"));
}

#[tokio::test]
async fn test_non_blocking_failure_continues() {
    let mut registry = HookRegistry::new();

    // Non-blocking hook that exits 1 (which would block if blocking=true)
    registry.register(HookRegistration {
        id: "non-blocker".into(),
        event_type: HookEventType::PreToolExecution,
        handler: HookHandler::Command {
            cmd: "echo 'warn'; exit 1".into(),
            timeout_ms: 5000,
        },
        priority: 1,
        blocking: false, // Non-blocking!
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    // Second hook should still fire
    registry.register(HookRegistration {
        id: "after".into(),
        event_type: HookEventType::PreToolExecution,
        handler: HookHandler::Command {
            cmd: "echo 'ok'".into(),
            timeout_ms: 5000,
        },
        priority: 2,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    let results = registry
        .fire(HookEvent::PreToolExecution {
            tool_name: "test".into(),
            parameters: json!({}),
        })
        .await;

    // Both fired — non-blocking Block doesn't stop chain
    assert_eq!(results.len(), 2);
    assert!(matches!(&results[0], HookResult::Block(_)));
    assert_eq!(results[1], HookResult::Allow);
}

// =============================================================================
// Command execution
// =============================================================================

#[tokio::test]
async fn test_command_exit_0_allows() {
    let handler = HookHandler::Command {
        cmd: "exit 0".into(),
        timeout_ms: 5000,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert_eq!(result, HookResult::Allow);
}

#[tokio::test]
async fn test_command_exit_1_blocks() {
    let handler = HookHandler::Command {
        cmd: "echo 'denied'; exit 1".into(),
        timeout_ms: 5000,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert!(matches!(result, HookResult::Block(reason) if reason == "denied"));
}

#[tokio::test]
async fn test_command_exit_2_modifies() {
    let handler = HookHandler::Command {
        cmd: r#"echo '{"modified": true}'; exit 2"#.into(),
        timeout_ms: 5000,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    match result {
        HookResult::Modify(v) => assert_eq!(v, json!({"modified": true})),
        other => panic!("Expected Modify, got {:?}", other),
    }
}

#[tokio::test]
async fn test_command_captures_output() {
    let handler = HookHandler::Command {
        cmd: "echo 'hello from hook'".into(),
        timeout_ms: 5000,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    // Exit 0 means Allow, but the command ran successfully
    assert_eq!(result, HookResult::Allow);
}

#[tokio::test]
async fn test_command_template_substitution() {
    let handler = HookHandler::Command {
        cmd: "echo '{tool_name}'".into(),
        timeout_ms: 5000,
    };
    let event = HookEvent::PreToolExecution {
        tool_name: "file_read".into(),
        parameters: json!({}),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    // The command ran with substituted tool_name, exit 0 = Allow
    assert_eq!(result, HookResult::Allow);
}

// =============================================================================
// Timeout enforcement
// =============================================================================

#[tokio::test]
async fn test_command_timeout_enforcement() {
    let handler = HookHandler::Command {
        cmd: "sleep 10".into(),
        timeout_ms: 200, // 200ms timeout — much shorter than sleep 10
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let start = std::time::Instant::now();
    let result = handler
        .execute(&event, std::time::Duration::from_millis(200))
        .await;
    let elapsed = start.elapsed();

    // Should complete within ~1 second (well under the 10s sleep)
    assert!(elapsed < std::time::Duration::from_secs(2));
    // Timeout is fail-open
    assert_eq!(result, HookResult::Allow);
}

// =============================================================================
// Disabled hooks
// =============================================================================

#[tokio::test]
async fn test_disabled_hooks_are_skipped() {
    let mut registry = HookRegistry::new();

    let mut hook = make_echo_hook("disabled", HookEventType::TaskStarted, 10, "exit 1");
    hook.enabled = false;
    registry.register(hook);

    let results = registry
        .fire(HookEvent::TaskStarted {
            task_id: "t1".into(),
        })
        .await;

    assert!(results.is_empty());
}

// =============================================================================
// Stub handlers
// =============================================================================

#[tokio::test]
async fn test_prompt_handler_returns_allow() {
    let handler = HookHandler::Prompt {
        template: "some template".into(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert_eq!(result, HookResult::Allow);
}

// =============================================================================
// SSRF guard — is_private_ip
// =============================================================================

#[test]
fn test_is_private_ip_loopback_v4() {
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    assert!(is_private_ip(&ip));
    let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 255, 255, 255));
    assert!(is_private_ip(&ip2));
}

#[test]
fn test_is_private_ip_10_range() {
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    assert!(is_private_ip(&ip));
    let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 255, 255, 255));
    assert!(is_private_ip(&ip2));
}

#[test]
fn test_is_private_ip_172_range() {
    // 172.16.0.0/12 — 172.16.x.x through 172.31.x.x
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1));
    assert!(is_private_ip(&ip));
    let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255));
    assert!(is_private_ip(&ip2));
    // 172.15.x.x is NOT private
    let ip3: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 15, 0, 1));
    assert!(!is_private_ip(&ip3));
    // 172.32.x.x is NOT private
    let ip4: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 32, 0, 1));
    assert!(!is_private_ip(&ip4));
}

#[test]
fn test_is_private_ip_192_168_range() {
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1));
    assert!(is_private_ip(&ip));
    let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 255, 255));
    assert!(is_private_ip(&ip2));
    // 192.169.x.x is NOT private
    let ip3: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 169, 0, 1));
    assert!(!is_private_ip(&ip3));
}

#[test]
fn test_is_private_ip_link_local() {
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1));
    assert!(is_private_ip(&ip));
    let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(169, 254, 255, 255));
    assert!(is_private_ip(&ip2));
}

#[test]
fn test_is_private_ip_public_address() {
    let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
    assert!(!is_private_ip(&ip));
    let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
    assert!(!is_private_ip(&ip2));
}

#[test]
fn test_is_private_ip_ipv6_loopback() {
    let ip: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);
    assert!(is_private_ip(&ip));
}

#[test]
fn test_is_private_ip_ipv6_public() {
    let ip: IpAddr = IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888));
    assert!(!is_private_ip(&ip));
}

// =============================================================================
// SSRF guard — is_private_url
// =============================================================================

#[test]
fn test_is_private_url_localhost() {
    assert!(is_private_url("http://localhost/hook"));
    assert!(is_private_url("http://localhost:8080/hook"));
    assert!(is_private_url("http://LOCALHOST/hook"));
}

#[test]
fn test_is_private_url_loopback_ip() {
    assert!(is_private_url("http://127.0.0.1/hook"));
    assert!(is_private_url("http://127.0.0.1:3000/hook"));
}

#[test]
fn test_is_private_url_private_ranges() {
    assert!(is_private_url("http://10.0.0.1/hook"));
    assert!(is_private_url("http://172.16.5.10/hook"));
    assert!(is_private_url("http://192.168.1.1/hook"));
    assert!(is_private_url("http://169.254.169.254/latest/metadata")); // AWS metadata
}

#[test]
fn test_is_private_url_ipv6_loopback() {
    assert!(is_private_url("http://[::1]/hook"));
    assert!(is_private_url("http://[::1]:8080/hook"));
}

#[test]
fn test_is_private_url_invalid_url() {
    assert!(is_private_url("not-a-url"));
    assert!(is_private_url(""));
}

#[test]
fn test_is_private_url_no_host() {
    assert!(is_private_url("file:///etc/passwd"));
}

// =============================================================================
// HTTP hook handler — SSRF blocking
// =============================================================================

#[tokio::test]
async fn test_http_ssrf_blocks_localhost() {
    let handler = HookHandler::Http {
        url: "http://localhost:9999/hook".into(),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    // SSRF blocked → fail-open → Allow
    assert_eq!(result, HookResult::Allow);
}

#[tokio::test]
async fn test_http_ssrf_blocks_private_ip() {
    let handler = HookHandler::Http {
        url: "http://192.168.1.1/hook".into(),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert_eq!(result, HookResult::Allow);
}

#[tokio::test]
async fn test_http_ssrf_blocks_loopback() {
    let handler = HookHandler::Http {
        url: "http://127.0.0.1:3322/hook".into(),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert_eq!(result, HookResult::Allow);
}

// =============================================================================
// HTTP hook handler — mockito integration tests
// =============================================================================

#[tokio::test]
async fn test_http_handler_allow_on_2xx() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_body(r#"{"status": "ok"}"#)
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_block_action() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"action": "block", "reason": "policy violation"}"#)
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::PreToolExecution {
        tool_name: "dangerous_tool".into(),
        parameters: json!({}),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert!(matches!(
        result,
        HookResult::Block(reason) if reason == "policy violation"
    ));
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_block_action_default_reason() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"action": "block"}"#)
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert!(matches!(
        result,
        HookResult::Block(reason) if reason == "blocked by webhook"
    ));
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_modify_action() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"action": "modify", "data": {"tool_name": "safe_tool", "sanitized": true}}"#)
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::PreToolExecution {
        tool_name: "test".into(),
        parameters: json!({}),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    match result {
        HookResult::Modify(data) => {
            assert_eq!(data["tool_name"], "safe_tool");
            assert_eq!(data["sanitized"], true);
        }
        other => panic!("Expected Modify, got {:?}", other),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_modify_without_data_allows() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"action": "modify"}"#)
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    // modify without data → Allow
    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_4xx_fail_open() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(403)
        .with_body("forbidden")
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_5xx_fail_open() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(500)
        .with_body("internal error")
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_network_error_fail_open() {
    enable_ssrf_bypass();
    // Connect to a port where nothing is listening
    let handler = HookHandler::Http {
        url: "http://203.0.113.1:1/hook".into(), // TEST-NET, nothing there
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(2))
        .await;

    assert_eq!(result, HookResult::Allow);
}

#[tokio::test]
async fn test_http_handler_sends_event_json() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .match_body(mockito::Matcher::JsonString(
            r#"{"type":"task_started","task_id":"event-payload-test"}"#.into(),
        ))
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "event-payload-test".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_custom_headers() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .match_header("X-Custom-Header", "custom-value")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let mut headers = std::collections::HashMap::new();
    headers.insert("X-Custom-Header".into(), "custom-value".into());

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_env_var_interpolation() {
    enable_ssrf_bypass();
    // Set a test environment variable
    std::env::set_var("DOLPHIN_MILK_TEST_HOOK_TOKEN", "secret123");

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .match_header("Authorization", "Bearer secret123")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let mut headers = std::collections::HashMap::new();
    headers.insert(
        "Authorization".into(),
        "Bearer $DOLPHIN_MILK_TEST_HOOK_TOKEN".into(),
    );

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;

    std::env::remove_var("DOLPHIN_MILK_TEST_HOOK_TOKEN");
}

#[tokio::test]
async fn test_http_handler_respects_method() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("PUT", "/hook")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "PUT".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_non_json_response_allows() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_body("OK")
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    // Non-JSON body → Allow
    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_http_handler_unknown_action_allows() {
    enable_ssrf_bypass();
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/hook")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"action": "allow"}"#)
        .create_async()
        .await;

    let handler = HookHandler::Http {
        url: format!("{}/hook", server.url()),
        method: "POST".into(),
        headers: std::collections::HashMap::new(),
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;

    assert_eq!(result, HookResult::Allow);
    mock.assert_async().await;
}

#[tokio::test]
async fn test_agent_handler_returns_allow() {
    let handler = HookHandler::Agent {
        task_description: "handle event".into(),
        budget_sats: 1000,
    };
    let event = HookEvent::TaskStarted {
        task_id: "t1".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert_eq!(result, HookResult::Allow);
}

// =============================================================================
// Config parsing
// =============================================================================

#[test]
fn test_config_parsing_valid_entry() {
    let toml_str = r#"
        [[hooks]]
        event = "pre_tool_execution"
        priority = 5
        blocking = true
        timeout_ms = 3000
        [hooks.handler]
        type = "command"
        cmd = "echo test"
    "#;

    #[derive(serde::Deserialize)]
    struct Wrapper {
        hooks: Vec<HookConfigEntry>,
    }

    let parsed: Wrapper = toml::from_str(toml_str).unwrap();
    assert_eq!(parsed.hooks.len(), 1);

    let reg = parsed.hooks[0].clone().into_registration(0).unwrap();
    assert_eq!(reg.event_type, HookEventType::PreToolExecution);
    assert_eq!(reg.priority, 5);
    assert!(reg.blocking);
    assert_eq!(reg.timeout_ms, 3000);
}

#[test]
fn test_config_parsing_invalid_event_type() {
    let entry = HookConfigEntry {
        event: "nonexistent_event".into(),
        handler: HookHandler::Command {
            cmd: "echo test".into(),
            timeout_ms: 5000,
        },
        priority: 10,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        id: None,
        matcher: None,
        once: false,
    };

    let result = entry.into_registration(0);
    assert!(result.is_err());
}

#[test]
fn test_config_parsing_defaults() {
    let toml_str = r#"
        [[hooks]]
        event = "task_started"
        [hooks.handler]
        type = "command"
        cmd = "echo test"
    "#;

    #[derive(serde::Deserialize)]
    struct Wrapper {
        hooks: Vec<HookConfigEntry>,
    }

    let parsed: Wrapper = toml::from_str(toml_str).unwrap();
    let reg = parsed.hooks[0].clone().into_registration(0).unwrap();

    assert_eq!(reg.priority, 100); // default
    assert!(!reg.blocking); // default false
    assert!(reg.enabled); // default true
    assert_eq!(reg.timeout_ms, 5000); // default
}

#[test]
fn test_config_parse_multiple_hooks() {
    let toml_str = r#"
        [[hooks]]
        event = "task_started"
        [hooks.handler]
        type = "command"
        cmd = "echo start"

        [[hooks]]
        event = "task_completed"
        priority = 1
        [hooks.handler]
        type = "command"
        cmd = "echo done"
    "#;

    #[derive(serde::Deserialize)]
    struct Wrapper {
        hooks: Vec<HookConfigEntry>,
    }

    let parsed: Wrapper = toml::from_str(toml_str).unwrap();
    assert_eq!(parsed.hooks.len(), 2);

    let registrations = dolphin_milk::hooks::parse_hook_configs(&parsed.hooks);
    assert_eq!(registrations.len(), 2);
}

// =============================================================================
// Event types
// =============================================================================

#[test]
fn test_hook_event_type_roundtrip() {
    let types = vec![
        HookEventType::PreToolExecution,
        HookEventType::PostToolExecution,
        HookEventType::ToolError,
        HookEventType::PermissionRequest,
        HookEventType::PermissionDenied,
        HookEventType::BudgetThreshold,
        HookEventType::SessionStart,
        HookEventType::SessionEnd,
        HookEventType::ConversationTurn,
        HookEventType::ContextCompaction,
        HookEventType::PreCompact,
        HookEventType::PostCompact,
        HookEventType::ContextOverflow,
        HookEventType::SystemPromptUpdate,
        HookEventType::TaskCreated,
        HookEventType::TaskStarted,
        HookEventType::TaskCompleted,
        HookEventType::TaskFailed,
        HookEventType::IterationStart,
        HookEventType::IterationEnd,
        HookEventType::ConfigReloaded,
        HookEventType::ConfigError,
    ];

    for t in &types {
        let s = t.to_string();
        let parsed: HookEventType = s.parse().unwrap();
        assert_eq!(*t, parsed);
    }
}

#[test]
fn test_hook_event_discriminant() {
    let event = HookEvent::PreToolExecution {
        tool_name: "test".into(),
        parameters: json!({}),
    };
    assert_eq!(event.event_type(), HookEventType::PreToolExecution);

    let event = HookEvent::TaskFailed {
        task_id: "t1".into(),
        error: "boom".into(),
    };
    assert_eq!(event.event_type(), HookEventType::TaskFailed);
}

// =============================================================================
// is_blocked helper
// =============================================================================

#[test]
fn test_is_blocked_with_block() {
    let results = vec![HookResult::Allow, HookResult::Block("reason".into())];
    assert_eq!(is_blocked(&results), Some("reason"));
}

#[test]
fn test_is_blocked_without_block() {
    let results = vec![HookResult::Allow, HookResult::Allow];
    assert_eq!(is_blocked(&results), None);
}

#[test]
fn test_is_blocked_empty() {
    let results: Vec<HookResult> = vec![];
    assert_eq!(is_blocked(&results), None);
}

// =============================================================================
// Multiple hooks on same event fire in priority order
// =============================================================================

#[tokio::test]
async fn test_multiple_hooks_same_event_priority_order() {
    let mut registry = HookRegistry::new();

    // Hook with priority 50
    registry.register(HookRegistration {
        id: "p50".into(),
        event_type: HookEventType::IterationStart,
        handler: HookHandler::Command {
            cmd: "echo p50".into(),
            timeout_ms: 5000,
        },
        priority: 50,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    // Hook with priority 10 (should fire first)
    registry.register(HookRegistration {
        id: "p10".into(),
        event_type: HookEventType::IterationStart,
        handler: HookHandler::Command {
            cmd: "echo p10".into(),
            timeout_ms: 5000,
        },
        priority: 10,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    // Hook with priority 30
    registry.register(HookRegistration {
        id: "p30".into(),
        event_type: HookEventType::IterationStart,
        handler: HookHandler::Command {
            cmd: "echo p30".into(),
            timeout_ms: 5000,
        },
        priority: 30,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    });

    let results = registry
        .fire(HookEvent::IterationStart {
            task_id: "t1".into(),
            iteration: 1,
        })
        .await;

    // All three fired (all return Allow for exit 0)
    assert_eq!(results.len(), 3);
}

// =============================================================================
// from_config
// =============================================================================

#[test]
fn test_from_config_skips_invalid() {
    let entries = vec![
        HookConfigEntry {
            event: "task_started".into(),
            handler: HookHandler::Command {
                cmd: "echo ok".into(),
                timeout_ms: 5000,
            },
            priority: 10,
            blocking: false,
            enabled: true,
            timeout_ms: 5000,
            id: None,
            matcher: None,
            once: false,
        },
        HookConfigEntry {
            event: "invalid_event".into(),
            handler: HookHandler::Command {
                cmd: "echo bad".into(),
                timeout_ms: 5000,
            },
            priority: 20,
            blocking: false,
            enabled: true,
            timeout_ms: 5000,
            id: None,
            matcher: None,
            once: false,
        },
    ];

    let registry = HookRegistry::from_config(&entries);
    // Only the valid one should be registered
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.list()[0].event_type, HookEventType::TaskStarted);
}

// =============================================================================
// HOOK_EVENT environment variable
// =============================================================================

#[tokio::test]
async fn test_command_receives_hook_event_env() {
    // The command checks that HOOK_EVENT env var is set and contains the task_id
    let handler = HookHandler::Command {
        cmd: r#"echo "$HOOK_EVENT" | grep -q '"task_id":"env-test"' && exit 0 || exit 3"#.into(),
        timeout_ms: 5000,
    };
    let event = HookEvent::TaskStarted {
        task_id: "env-test".into(),
    };
    let result = handler
        .execute(&event, std::time::Duration::from_secs(5))
        .await;
    assert_eq!(result, HookResult::Allow);
}

// =============================================================================
// Helpers
// =============================================================================

fn make_hook(id: &str, event_type: HookEventType, priority: i32) -> HookRegistration {
    HookRegistration {
        id: id.into(),
        event_type,
        handler: HookHandler::Command {
            cmd: "true".into(),
            timeout_ms: 5000,
        },
        priority,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    }
}

fn make_echo_hook(
    id: &str,
    event_type: HookEventType,
    priority: i32,
    cmd: &str,
) -> HookRegistration {
    HookRegistration {
        id: id.into(),
        event_type,
        handler: HookHandler::Command {
            cmd: cmd.into(),
            timeout_ms: 5000,
        },
        priority,
        blocking: false,
        enabled: true,
        timeout_ms: 5000,
        matcher: None,
        once: false,
    }
}
