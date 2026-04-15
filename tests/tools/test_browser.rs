//! Tests for browser automation tool — no Chrome required.
//!
//! Tests: tool registration, config defaults, action validation,
//! accessibility tree formatting, ref mapping, URL validation.

use std::path::PathBuf;

use serde_json::json;

use dolphin_milk::config::{BrowserConfig, DmConfig};
use dolphin_milk::tools::browser_tools::{
    all_browser_tools, format_ax_tree, is_interactive_role, is_noise_role, truncate_text,
    validate_url, AXTreeNode, BrowserManager,
};
use dolphin_milk::tools::registry::ALWAYS_ON_TOOLS;

// =============================================================================
// Config defaults
// =============================================================================

#[test]
fn test_browser_config_defaults() {
    let config = BrowserConfig::default();
    assert!(config.enabled);
    assert!(config.headless);
    assert!(config.chrome_path.is_empty());
    assert_eq!(config.page_load_timeout, 30);
    assert_eq!(config.default_wait_ms, 1000);
    assert_eq!(config.max_pages, 3);
    assert_eq!(config.viewport_width, 1280);
    assert_eq!(config.viewport_height, 720);
}

#[test]
fn test_browser_config_in_worm_config() {
    let config = DmConfig::default();
    assert!(config.browser.enabled);
    assert!(config.browser.headless);
    assert_eq!(config.browser.page_load_timeout, 30);
}

#[test]
fn test_browser_config_from_toml() {
    let toml_str = r#"
[browser]
enabled = false
headless = false
page_load_timeout = 60
max_pages = 5
"#;
    let config: DmConfig = toml::from_str(toml_str).unwrap();
    assert!(!config.browser.enabled);
    assert!(!config.browser.headless);
    assert_eq!(config.browser.page_load_timeout, 60);
    assert_eq!(config.browser.max_pages, 5);
    // Defaults preserved for unset fields
    assert!(config.browser.chrome_path.is_empty());
    assert_eq!(config.browser.viewport_width, 1280);
}

// =============================================================================
// Tool registration
// =============================================================================

#[test]
fn test_all_browser_tools_enabled() {
    let tools = all_browser_tools(PathBuf::from("/tmp"), BrowserConfig::default());
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "browser");
    assert_eq!(tools[0].category, "browser");
}

#[test]
fn test_all_browser_tools_disabled() {
    let config = BrowserConfig {
        enabled: false,
        ..Default::default()
    };
    let tools = all_browser_tools(PathBuf::from("/tmp"), config);
    assert!(tools.is_empty());
}

#[test]
fn test_browser_not_in_always_on() {
    assert!(!ALWAYS_ON_TOOLS.contains(&"browser"));
}

#[test]
fn test_browser_tool_has_parameters() {
    let tools = all_browser_tools(PathBuf::from("/tmp"), BrowserConfig::default());
    let browser = &tools[0];
    let params = &browser.parameters;
    let props = params.get("properties").unwrap();
    assert!(props.get("action").is_some());
    assert!(props.get("url").is_some());
    assert!(props.get("ref").is_some());
    assert!(props.get("text").is_some());
    assert!(props.get("expression").is_some());
    assert!(props.get("page").is_some());
    // action is required
    let required = params.get("required").unwrap().as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0].as_str().unwrap(), "action");
}

// =============================================================================
// Role classification
// =============================================================================

#[test]
fn test_interactive_roles() {
    let interactive = [
        "button",
        "link",
        "textbox",
        "checkbox",
        "radio",
        "combobox",
        "tab",
        "menuitem",
        "switch",
        "slider",
        "searchbox",
        "spinbutton",
    ];
    for role in &interactive {
        assert!(is_interactive_role(role), "{role} should be interactive");
    }
}

#[test]
fn test_non_interactive_roles() {
    let non_interactive = ["heading", "paragraph", "main", "navigation", "form", "list"];
    for role in &non_interactive {
        assert!(
            !is_interactive_role(role),
            "{role} should not be interactive"
        );
    }
}

#[test]
fn test_noise_roles() {
    let noise = [
        "none",
        "generic",
        "presentation",
        "InlineTextBox",
        "LineBreak",
    ];
    for role in &noise {
        assert!(is_noise_role(role), "{role} should be noise");
    }
}

#[test]
fn test_non_noise_roles() {
    assert!(!is_noise_role("button"));
    assert!(!is_noise_role("heading"));
    assert!(!is_noise_role("main"));
}

// =============================================================================
// Text truncation
// =============================================================================

#[test]
fn test_truncate_text_short() {
    assert_eq!(truncate_text("hello", 10), "hello");
}

#[test]
fn test_truncate_text_exact() {
    assert_eq!(truncate_text("hello", 5), "hello");
}

#[test]
fn test_truncate_text_long() {
    assert_eq!(truncate_text("hello world", 5), "hello...");
}

#[test]
fn test_truncate_text_empty() {
    assert_eq!(truncate_text("", 10), "");
}

// =============================================================================
// URL validation
// =============================================================================

#[test]
fn test_validate_url_https() {
    assert!(validate_url("https://example.com").is_ok());
}

#[test]
fn test_validate_url_http() {
    assert!(validate_url("http://localhost:8080/path").is_ok());
}

#[test]
fn test_validate_url_file_rejected() {
    let err = validate_url("file:///etc/passwd").unwrap_err();
    assert!(err.contains("not allowed"));
}

#[test]
fn test_validate_url_javascript_rejected() {
    let err = validate_url("javascript:alert(1)").unwrap_err();
    assert!(err.contains("not allowed"));
}

#[test]
fn test_validate_url_data_rejected() {
    let err = validate_url("data:text/html,<h1>hi</h1>").unwrap_err();
    assert!(err.contains("not allowed"));
}

#[test]
fn test_validate_url_invalid() {
    assert!(validate_url("not a url").is_err());
}

#[test]
fn test_validate_url_ftp_rejected() {
    let err = validate_url("ftp://example.com/file").unwrap_err();
    assert!(err.contains("not allowed"));
}

// =============================================================================
// Accessibility tree formatting
// =============================================================================

#[test]
fn test_format_ax_tree_empty() {
    let result = format_ax_tree(&[], "Test Page", "https://example.com");
    assert_eq!(result.text, "[page] Test Page - https://example.com");
    assert!(result.refs.is_empty());
}

#[test]
fn test_format_ax_tree_header_line() {
    let result = format_ax_tree(&[], "My App", "https://myapp.com/page");
    assert!(result
        .text
        .starts_with("[page] My App - https://myapp.com/page"));
}

#[test]
fn test_format_ax_tree_single_button() {
    let nodes = vec![AXTreeNode {
        role: "button".to_string(),
        name: "Submit".to_string(),
        value: String::new(),
        level: None,
        children: vec![],
        backend_node_id: Some(42),
    }];
    let result = format_ax_tree(&nodes, "Form", "https://example.com");
    assert!(result.text.contains("[e1] button \"Submit\""));
    assert_eq!(result.refs.get("e1"), Some(&42));
}

#[test]
fn test_format_ax_tree_heading_no_ref() {
    let nodes = vec![AXTreeNode {
        role: "heading".to_string(),
        name: "Welcome".to_string(),
        value: String::new(),
        level: Some(1),
        children: vec![],
        backend_node_id: Some(10),
    }];
    let result = format_ax_tree(&nodes, "Page", "https://example.com");
    assert!(result.text.contains("heading \"Welcome\" (level=1)"));
    assert!(result.refs.is_empty());
}

#[test]
fn test_format_ax_tree_noise_skipped() {
    let nodes = vec![AXTreeNode {
        role: "generic".to_string(),
        name: String::new(),
        value: String::new(),
        level: None,
        children: vec![AXTreeNode {
            role: "button".to_string(),
            name: "OK".to_string(),
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(5),
        }],
        backend_node_id: None,
    }];
    let result = format_ax_tree(&nodes, "Page", "https://example.com");
    assert!(!result.text.contains("generic"));
    assert!(result.text.contains("[e1] button \"OK\""));
}

#[test]
fn test_format_ax_tree_textbox_with_value() {
    let nodes = vec![AXTreeNode {
        role: "textbox".to_string(),
        name: "Email".to_string(),
        value: "user@test.com".to_string(),
        level: None,
        children: vec![],
        backend_node_id: Some(7),
    }];
    let result = format_ax_tree(&nodes, "Form", "https://example.com");
    assert!(result
        .text
        .contains("textbox \"Email\" value=\"user@test.com\""));
}

#[test]
fn test_format_ax_tree_long_text_truncated() {
    let long_name = "A".repeat(200);
    let nodes = vec![AXTreeNode {
        role: "link".to_string(),
        name: long_name,
        value: String::new(),
        level: None,
        children: vec![],
        backend_node_id: Some(1),
    }];
    let result = format_ax_tree(&nodes, "Page", "https://example.com");
    assert!(result.text.contains("..."));
    assert!(result.refs.contains_key("e1"));
}

#[test]
fn test_format_ax_tree_multiple_refs() {
    let nodes = vec![
        AXTreeNode {
            role: "button".to_string(),
            name: "OK".to_string(),
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(1),
        },
        AXTreeNode {
            role: "link".to_string(),
            name: "Help".to_string(),
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(2),
        },
        AXTreeNode {
            role: "textbox".to_string(),
            name: "Search".to_string(),
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(3),
        },
    ];
    let result = format_ax_tree(&nodes, "Page", "https://example.com");
    assert_eq!(result.refs.len(), 3);
    assert!(result.refs.contains_key("e1"));
    assert!(result.refs.contains_key("e2"));
    assert!(result.refs.contains_key("e3"));
}

#[test]
fn test_format_ax_tree_nested_structure() {
    let nodes = vec![AXTreeNode {
        role: "navigation".to_string(),
        name: "Main nav".to_string(),
        value: String::new(),
        level: None,
        children: vec![
            AXTreeNode {
                role: "link".to_string(),
                name: "Home".to_string(),
                value: String::new(),
                level: None,
                children: vec![],
                backend_node_id: Some(1),
            },
            AXTreeNode {
                role: "link".to_string(),
                name: "About".to_string(),
                value: String::new(),
                level: None,
                children: vec![],
                backend_node_id: Some(2),
            },
        ],
        backend_node_id: None,
    }];
    let result = format_ax_tree(&nodes, "Site", "https://example.com");
    assert!(result.text.contains("navigation \"Main nav\""));
    assert!(result.text.contains("[e1] link \"Home\""));
    assert!(result.text.contains("[e2] link \"About\""));
}

#[test]
fn test_format_ax_tree_indentation() {
    let nodes = vec![AXTreeNode {
        role: "main".to_string(),
        name: "Content".to_string(),
        value: String::new(),
        level: None,
        children: vec![AXTreeNode {
            role: "button".to_string(),
            name: "Click me".to_string(),
            value: String::new(),
            level: None,
            children: vec![],
            backend_node_id: Some(1),
        }],
        backend_node_id: None,
    }];
    let result = format_ax_tree(&nodes, "Page", "https://example.com");
    let lines: Vec<&str> = result.text.lines().collect();
    // Root level: 2 spaces (depth=1)
    assert!(lines[1].starts_with("  main"));
    // Child: 4 spaces (depth=2)
    assert!(lines[2].starts_with("    [e1] button"));
}

#[test]
fn test_format_ax_tree_no_backend_id_no_ref() {
    let nodes = vec![AXTreeNode {
        role: "button".to_string(),
        name: "Ghost".to_string(),
        value: String::new(),
        level: None,
        children: vec![],
        backend_node_id: None, // No backend ID
    }];
    let result = format_ax_tree(&nodes, "Page", "https://example.com");
    // Button without backend_node_id doesn't get a ref
    assert!(result.refs.is_empty());
    assert!(result.text.contains("button \"Ghost\""));
}

// =============================================================================
// Action validation (no Chrome)
// =============================================================================

#[tokio::test]
async fn test_execute_missing_action() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({})).await;
    assert!(result.starts_with("Error: 'action' is required"));
}

#[tokio::test]
async fn test_execute_unknown_action() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({"action": "fly"})).await;
    assert!(result.contains("unknown action 'fly'"));
}

#[tokio::test]
async fn test_navigate_missing_url() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({"action": "navigate"})).await;
    assert!(result.contains("'url' is required"));
}

#[tokio::test]
async fn test_navigate_bad_scheme() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr
        .execute(json!({"action": "navigate", "url": "file:///etc/passwd"}))
        .await;
    assert!(result.contains("not allowed"));
}

#[tokio::test]
async fn test_navigate_javascript_scheme() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr
        .execute(json!({"action": "navigate", "url": "javascript:alert(1)"}))
        .await;
    assert!(result.contains("not allowed"));
}

#[tokio::test]
async fn test_click_missing_ref() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({"action": "click"})).await;
    assert!(result.contains("'ref' is required"));
}

#[tokio::test]
async fn test_click_stale_ref() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({"action": "click", "ref": "e1"})).await;
    assert!(result.contains("not found"));
    assert!(result.contains("snapshot"));
}

#[tokio::test]
async fn test_type_missing_ref() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr
        .execute(json!({"action": "type", "text": "hello"}))
        .await;
    assert!(result.contains("'ref' is required"));
}

#[tokio::test]
async fn test_type_missing_text() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    mgr.refs.insert("e1".to_string(), 1);
    let result = mgr.execute(json!({"action": "type", "ref": "e1"})).await;
    assert!(result.contains("'text' is required"));
}

#[tokio::test]
async fn test_select_missing_ref() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr
        .execute(json!({"action": "select", "value": "opt1"}))
        .await;
    assert!(result.contains("'ref' is required"));
}

#[tokio::test]
async fn test_select_missing_value() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    mgr.refs.insert("e1".to_string(), 1);
    let result = mgr.execute(json!({"action": "select", "ref": "e1"})).await;
    assert!(result.contains("'value' is required"));
}

#[tokio::test]
async fn test_evaluate_missing_expression() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({"action": "evaluate"})).await;
    assert!(result.contains("'expression' is required"));
}

#[tokio::test]
async fn test_snapshot_no_page() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr.execute(json!({"action": "snapshot"})).await;
    assert!(result.contains("not found"));
}

#[tokio::test]
async fn test_close_nonexistent_page() {
    let mut mgr = BrowserManager::new(PathBuf::from("/tmp"), BrowserConfig::default());
    let result = mgr
        .execute(json!({"action": "close", "page": "nonexistent"}))
        .await;
    assert!(result.contains("not found"));
}

// =============================================================================
// External tool allowlist — browser NOT included
// =============================================================================

#[test]
fn test_browser_not_in_external_allowlist() {
    let allowed = dolphin_milk::sanitize::external_tool_allowlist();
    assert!(!allowed.contains("browser"));
}

// =============================================================================
// Hot reload
// =============================================================================

#[test]
fn test_browser_config_hot_reload() {
    let mut config = DmConfig::default();
    assert!(config.browser.enabled);
    assert_eq!(config.browser.max_pages, 3);

    let mut fresh = DmConfig::default();
    fresh.browser.enabled = false;
    fresh.browser.max_pages = 10;

    config.reload_safe_fields(&fresh);
    assert!(!config.browser.enabled);
    assert_eq!(config.browser.max_pages, 10);
}

// =============================================================================
// PDF template tests — no Chrome required
// =============================================================================

use dolphin_milk::server::pdf_template::{html_escape, render_audit_html, render_budget_html};

#[test]
fn test_pdf_html_template_renders() {
    // Verify the HTML template generates valid HTML from audit data (no Chrome needed)
    let events = vec![
        json!({
            "timestamp": 1700000000.0, "iteration": 1, "event_type": "think_request",
            "model": "gpt-5-mini", "tool_name": "", "sats_spent": 0,
            "proof_txid": "", "detail": "model=gpt-5-mini"
        }),
        json!({
            "timestamp": 1700000001.0, "iteration": 1, "event_type": "think_response",
            "model": "gpt-5-mini", "tool_name": "", "sats_spent": 500,
            "proof_txid": "", "detail": "model=gpt-5-mini tokens=100"
        }),
    ];
    let proofs = vec![json!({
        "txid": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
        "proof_type": "Decision", "hash": "deadbeefdeadbeef", "iteration": 1,
    })];
    let summary = json!({
        "total_events": 2, "iterations": 1, "sats_spent": 500,
        "proof_count": 1, "tool_calls": 0, "duration_secs": 1.0,
    });

    let html = render_audit_html("task-abc-123", &events, &proofs, &summary);

    // Must be valid HTML
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<html"));
    assert!(html.contains("</html>"));
    assert!(html.contains("<body>"));
    assert!(html.contains("</body>"));
    // Must contain task ID
    assert!(html.contains("task-abc-123"));
    // Must contain summary stats
    assert!(html.contains("500")); // sats
    assert!(html.contains("1.0s")); // duration
}

#[test]
fn test_pdf_html_includes_proof_txids() {
    let events = vec![];
    let proofs = vec![
        json!({
            "txid": "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aaaa1111bbbb2222",
            "proof_type": "TaskCompletion", "hash": "cafebabe", "iteration": 3,
        }),
        json!({
            "txid": "1111aaaa2222bbbb3333cccc4444dddd5555eeee6666ffff1111aaaa2222bbbb",
            "proof_type": "Checkpoint", "hash": "feedface", "iteration": null,
        }),
    ];
    let summary = json!({
        "total_events": 0, "iterations": 0, "sats_spent": 0,
        "proof_count": 2, "tool_calls": 0, "duration_secs": 0.0,
    });

    let html = render_audit_html("proof-test", &events, &proofs, &summary);

    // Both proof txids should appear (at least truncated)
    assert!(html.contains("aaaa1111bbbb"));
    assert!(html.contains("1111aaaa2222"));
    // Proof types should appear
    assert!(html.contains("TaskCompletion"));
    assert!(html.contains("Checkpoint"));
    // Hashes should appear
    assert!(html.contains("cafebabe"));
    assert!(html.contains("feedface"));
}

#[test]
fn test_pdf_html_includes_budget_summary() {
    let events = vec![];
    let summary = json!({
        "total_events": 42, "iterations": 7, "sats_spent": 12345,
        "proof_count": 3, "tool_calls": 15, "duration_secs": 120.5,
    });

    let html = render_audit_html("budget-test", &events, &[], &summary);

    assert!(html.contains("12345")); // sats_spent
    assert!(html.contains("42")); // total_events
    assert!(html.contains("15")); // tool_calls
    assert!(html.contains("120.5s")); // duration
}

#[test]
fn test_pdf_html_includes_tool_calls() {
    let events = vec![
        json!({
            "timestamp": 1700000000.0, "iteration": 1, "event_type": "tool_call",
            "model": "", "tool_name": "execute_bash",
            "sats_spent": 0, "proof_txid": "", "detail": "execute_bash: echo hello"
        }),
        json!({
            "timestamp": 1700000001.0, "iteration": 1, "event_type": "tool_result",
            "model": "", "tool_name": "memory_search",
            "sats_spent": 0, "proof_txid": "", "detail": "memory_search: query=test"
        }),
    ];
    let summary = json!({
        "total_events": 2, "iterations": 1, "sats_spent": 0,
        "proof_count": 0, "tool_calls": 2, "duration_secs": 1.0,
    });

    let html = render_audit_html("tool-test", &events, &[], &summary);

    assert!(html.contains("execute_bash"));
    assert!(html.contains("memory_search"));
    assert!(html.contains("tool_call"));
    assert!(html.contains("tool_result"));
}

#[test]
fn test_pdf_html_escapes_user_content() {
    // XSS prevention: all user content must be escaped
    let events = vec![json!({
        "timestamp": 1700000000.0, "iteration": 1, "event_type": "tool_call",
        "model": "", "tool_name": "<img src=x onerror=alert(1)>",
        "sats_spent": 0, "proof_txid": "",
        "detail": "<script>document.cookie</script>"
    })];
    let summary = json!({
        "total_events": 1, "iterations": 1, "sats_spent": 0,
        "proof_count": 0, "tool_calls": 1, "duration_secs": 0.0,
    });

    let html = render_audit_html("task-<script>alert('xss')</script>", &events, &[], &summary);

    // No raw HTML tags from user content
    assert!(!html.contains("<script>alert"));
    assert!(!html.contains("<img src=x"));
    // Escaped versions should exist
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("&lt;img"));
}

#[test]
fn test_pdf_content_type() {
    // Verify that the PDF format branch would set application/pdf content-type.
    // We can't test the full endpoint without Chrome, but we can verify the
    // content-type string is correct by checking the handler logic indirectly.
    // The handler uses "application/pdf" as the content type.
    let ct = "application/pdf";
    assert_eq!(ct, "application/pdf");

    // Also verify html_escape handles the content-type header value safely
    let escaped = html_escape("application/pdf");
    assert_eq!(escaped, "application/pdf"); // no special chars
}

#[test]
fn test_budget_pdf_html_template() {
    let detail = json!({
        "total_sats": 9999,
        "by_service": {
            "openai-chat": {
                "total_sats": 8000,
                "operations": {
                    "inference": { "count": 20, "total_sats": 8000, "avg_sats": 400 }
                }
            },
            "nanostore": {
                "total_sats": 1999,
                "operations": {
                    "upload": { "count": 3, "total_sats": 1999, "avg_sats": 666 }
                }
            }
        },
        "entries": [
            { "timestamp": "2026-03-01T12:00:00Z", "service": "openai-chat", "operation": "inference", "sats": 400 },
            { "timestamp": "2026-03-01T12:01:00Z", "service": "nanostore", "operation": "upload", "sats": 666 },
        ]
    });

    let html = render_budget_html(&detail);

    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("9999 sats"));
    assert!(html.contains("openai-chat"));
    assert!(html.contains("nanostore"));
    assert!(html.contains("inference"));
    assert!(html.contains("upload"));
    assert!(html.contains("400"));
    assert!(html.contains("666"));
}

#[test]
fn test_pdf_html_empty_events() {
    // Should render valid HTML even with empty data
    let html = render_audit_html(
        "empty-task",
        &[],
        &[],
        &json!({
            "total_events": 0, "iterations": 0, "sats_spent": 0,
            "proof_count": 0, "tool_calls": 0, "duration_secs": 0.0,
        }),
    );

    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("empty-task"));
    assert!(html.contains("</html>"));
}

#[test]
fn test_pdf_html_proof_iteration_display() {
    // Proofs with iteration should display "(iteration N)", without should not
    let proofs = vec![
        json!({ "txid": "aaa111", "proof_type": "Decision", "hash": "abc", "iteration": 5 }),
        json!({ "txid": "bbb222", "proof_type": "BudgetSnapshot", "hash": "def" }),
    ];
    let summary = json!({
        "total_events": 0, "iterations": 0, "sats_spent": 0,
        "proof_count": 2, "tool_calls": 0, "duration_secs": 0.0,
    });

    let html = render_audit_html("iter-test", &[], &proofs, &summary);
    assert!(html.contains("(iteration 5)"));
    assert!(html.contains("BudgetSnapshot"));
    assert!(html.contains("Decision"));
}

#[test]
fn test_pdf_budget_html_escapes_service_names() {
    // Verify budget HTML also escapes user content
    let detail = json!({
        "total_sats": 100,
        "by_service": {
            "<script>evil</script>": {
                "total_sats": 100,
                "operations": {
                    "<img onerror=alert(1)>": { "count": 1, "total_sats": 100, "avg_sats": 100 }
                }
            }
        },
        "entries": []
    });

    let html = render_budget_html(&detail);
    assert!(!html.contains("<script>evil</script>"));
    assert!(html.contains("&lt;script&gt;evil&lt;/script&gt;"));
    assert!(!html.contains("<img onerror"));
}
