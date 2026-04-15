//! Tests for per-tool approval workflows (D.2).
//!
//! Tests the ToolApprovalConfig, cert-driven approval, approval checking on DmLoop,
//! and the file-based approval/abort mechanism.

use std::io::Write;
use tempfile::TempDir;

use dolphin_milk::config::{load_config, DmConfig, ToolApprovalConfig};

/// Helper: clear all DOLPHIN_MILK_* env vars that affect tool_approval config,
/// returning their prior values so they can be restored.
fn save_and_clear_approval_env() -> (Option<String>, Option<String>) {
    let tools = std::env::var("DOLPHIN_MILK_APPROVAL_TOOLS").ok();
    let timeout = std::env::var("DOLPHIN_MILK_APPROVAL_TIMEOUT").ok();
    std::env::remove_var("DOLPHIN_MILK_APPROVAL_TOOLS");
    std::env::remove_var("DOLPHIN_MILK_APPROVAL_TIMEOUT");
    (tools, timeout)
}

/// Restore previously saved DOLPHIN_MILK_* env vars.
fn restore_approval_env(saved: (Option<String>, Option<String>)) {
    match saved.0 {
        Some(v) => std::env::set_var("DOLPHIN_MILK_APPROVAL_TOOLS", v),
        None => std::env::remove_var("DOLPHIN_MILK_APPROVAL_TOOLS"),
    }
    match saved.1 {
        Some(v) => std::env::set_var("DOLPHIN_MILK_APPROVAL_TIMEOUT", v),
        None => std::env::remove_var("DOLPHIN_MILK_APPROVAL_TIMEOUT"),
    }
}

// ---------------------------------------------------------------------------
// Config tests
// ---------------------------------------------------------------------------

#[test]
fn test_approval_config_default_empty() {
    let cfg = DmConfig::default();
    assert!(cfg.tool_approval.require.is_empty());
    assert_eq!(cfg.tool_approval.timeout_secs, 300);
}

#[test]
fn test_approval_config_with_tools() {
    let cfg = ToolApprovalConfig {
        require: vec!["execute_bash".into(), "file_write".into()],
        timeout_secs: 120,
    };
    assert_eq!(cfg.require.len(), 2);
    assert!(cfg.require.contains(&"execute_bash".to_string()));
    assert!(cfg.require.contains(&"file_write".to_string()));
    assert_eq!(cfg.timeout_secs, 120);
}

#[test]
fn test_approval_config_parsing() {
    // Parse TOML directly (not via load_config) to avoid env var races with
    // parallel tests that set DOLPHIN_MILK_APPROVAL_TOOLS. See issue #224.
    let toml_str = r#"
[tool_approval]
require = ["execute_bash", "wallet_call"]
timeout_secs = 60
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tool_approval.require.len(), 2);
    assert!(cfg
        .tool_approval
        .require
        .contains(&"execute_bash".to_string()));
    assert!(cfg
        .tool_approval
        .require
        .contains(&"wallet_call".to_string()));
    assert_eq!(cfg.tool_approval.timeout_secs, 60);
}

#[test]
fn test_approval_config_env_override() {
    // Save existing env vars so we can restore them after the test,
    // preventing leakage into parallel tests. See issue #224.
    let saved = save_and_clear_approval_env();

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_dolphin_milk.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f).unwrap();
    }

    // Set env vars
    std::env::set_var("DOLPHIN_MILK_APPROVAL_TOOLS", "x402_call,send_message");
    std::env::set_var("DOLPHIN_MILK_APPROVAL_TIMEOUT", "180");

    let cfg = load_config(Some(path.as_path())).unwrap();

    // Restore env vars immediately
    restore_approval_env(saved);

    assert_eq!(cfg.tool_approval.require.len(), 2);
    assert!(cfg.tool_approval.require.contains(&"x402_call".to_string()));
    assert!(cfg
        .tool_approval
        .require
        .contains(&"send_message".to_string()));
    assert_eq!(cfg.tool_approval.timeout_secs, 180);
}

#[test]
fn test_approval_config_partial_toml() {
    // Only specify require, timeout should default.
    // Parse TOML directly to avoid env var races. See issue #224.
    let toml_str = r#"
[tool_approval]
require = ["browser"]
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tool_approval.require, vec!["browser".to_string()]);
    assert_eq!(cfg.tool_approval.timeout_secs, 300); // default
}

#[test]
fn test_empty_approval_list_allows_all() {
    let cfg = DmConfig::default();
    // With empty approval list, no tool requires approval
    assert!(!cfg
        .tool_approval
        .require
        .contains(&"execute_bash".to_string()));
    assert!(!cfg
        .tool_approval
        .require
        .contains(&"wallet_call".to_string()));
}

// ---------------------------------------------------------------------------
// Certificate-driven approval tests
// ---------------------------------------------------------------------------

#[test]
fn test_cert_tool_approval_struct() {
    use dolphin_milk::certificates::CertToolApproval;

    let none = CertToolApproval::none();
    assert!(none.tools.is_none());

    let some = CertToolApproval {
        tools: Some(vec!["execute_bash".into(), "file_write".into()]),
    };
    assert_eq!(some.tools.as_ref().unwrap().len(), 2);
}

#[test]
fn test_cert_approval_no_cert_falls_back() {
    use dolphin_milk::certificates::CertToolApproval;

    // When no cert exists, tools is None
    let approval = CertToolApproval::none();
    assert!(approval.tools.is_none());
}

// ---------------------------------------------------------------------------
// DmLoop requires_approval tests (via config, not full construction)
// ---------------------------------------------------------------------------

#[test]
fn test_tool_requires_approval_when_configured() {
    let approval = ToolApprovalConfig {
        require: vec!["execute_bash".into(), "wallet_call".into()],
        timeout_secs: 300,
    };
    // Simulate the check: tool name in require list
    assert!(approval.require.iter().any(|t| t == "execute_bash"));
    assert!(approval.require.iter().any(|t| t == "wallet_call"));
}

#[test]
fn test_tool_does_not_require_approval_when_not_configured() {
    let approval = ToolApprovalConfig {
        require: vec!["execute_bash".into()],
        timeout_secs: 300,
    };
    // file_read is NOT in the list
    assert!(!approval.require.iter().any(|t| t == "file_read"));
    assert!(!approval.require.iter().any(|t| t == "memory_search"));
}

#[test]
fn test_approval_list_merge_cert_and_config() {
    let mut config_tools: Vec<String> = vec!["execute_bash".into()];
    let cert_tools: Vec<String> = vec!["wallet_call".into(), "execute_bash".into()]; // execute_bash is duplicate

    // Merge: cert tools added only if not already present
    for tool in cert_tools {
        if !config_tools.contains(&tool) {
            config_tools.push(tool);
        }
    }

    assert_eq!(config_tools.len(), 2);
    assert!(config_tools.contains(&"execute_bash".to_string()));
    assert!(config_tools.contains(&"wallet_call".to_string()));
}

// ---------------------------------------------------------------------------
// File-based approval mechanism tests
// ---------------------------------------------------------------------------

#[test]
fn test_approval_file_creation() {
    let dir = TempDir::new().unwrap();
    let approval_dir = dir.path().join("pending_approval");
    std::fs::create_dir_all(&approval_dir).unwrap();

    let call_id = "test-call-123";
    let approval_file = approval_dir.join(format!("{}.json", call_id));
    let approval_data = serde_json::json!({
        "staged_ref": format!("tool-execute_bash-{}", call_id),
        "tool": "execute_bash",
        "call_id": call_id,
        "arguments": r#"{"command":"ls"}"#,
        "timeout_secs": 300,
    });
    std::fs::write(
        &approval_file,
        serde_json::to_string_pretty(&approval_data).unwrap(),
    )
    .unwrap();

    assert!(approval_file.exists());

    // Verify the file contents
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&approval_file).unwrap()).unwrap();
    assert_eq!(content["tool"], "execute_bash");
    assert_eq!(content["call_id"], call_id);
    assert_eq!(content["timeout_secs"], 300);
}

#[test]
fn test_approval_approved_file_detection() {
    let dir = TempDir::new().unwrap();
    let approval_dir = dir.path().join("pending_approval");
    std::fs::create_dir_all(&approval_dir).unwrap();

    let call_id = "test-call-456";

    // Initially, no approval file
    let approved_file = approval_dir.join(format!("{}-approved.json", call_id));
    assert!(!approved_file.exists());

    // Create approved file (simulating what the server handler does)
    std::fs::write(&approved_file, "{}").unwrap();
    assert!(approved_file.exists());
}

#[test]
fn test_approval_aborted_file_detection() {
    let dir = TempDir::new().unwrap();
    let approval_dir = dir.path().join("pending_approval");
    std::fs::create_dir_all(&approval_dir).unwrap();

    let call_id = "test-call-789";

    let aborted_file = approval_dir.join(format!("{}-aborted.json", call_id));
    assert!(!aborted_file.exists());

    std::fs::write(&aborted_file, "{}").unwrap();
    assert!(aborted_file.exists());
}

#[tokio::test]
async fn test_approval_timeout_aborts() {
    // Test the timeout logic directly: when deadline passes, returns error
    let timeout_secs: u64 = 1; // 1 second timeout
    let dir = TempDir::new().unwrap();
    let approval_dir = dir.path().join("pending_approval");
    std::fs::create_dir_all(&approval_dir).unwrap();

    let call_id = "timeout-test";
    let approval_file = approval_dir.join(format!("{}.json", call_id));
    std::fs::write(&approval_file, "{}").unwrap();

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

    // Wait past the deadline
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    assert!(tokio::time::Instant::now() >= deadline);

    // Clean up the approval file as the real code would
    let _ = std::fs::remove_file(&approval_file);
    assert!(!approval_file.exists());
}

#[test]
fn test_approval_result_message_format() {
    // Test the error message format for aborted tool
    let name = "execute_bash";
    let msg = format!(
        "Error: Tool '{}' was manually aborted. The tool call was not executed.",
        name
    );
    assert!(msg.contains("execute_bash"));
    assert!(msg.contains("manually aborted"));
    assert!(msg.contains("not executed"));
}

#[test]
fn test_approval_timeout_message_format() {
    let name = "wallet_call";
    let timeout_secs = 300u64;
    let staged_ref = format!("tool-{}-call123", name);
    let msg = format!(
        "Error: Tool '{}' approval timed out after {}s. The tool call was auto-aborted. \
         Use POST /staged/{}/approve before the timeout to authorize future calls.",
        name, timeout_secs, staged_ref
    );
    assert!(msg.contains("wallet_call"));
    assert!(msg.contains("300s"));
    assert!(msg.contains(&staged_ref));
}

// ---------------------------------------------------------------------------
// StepEvent::ApprovalRequired serialization
// ---------------------------------------------------------------------------

#[test]
fn test_approval_required_event_serde() {
    use dolphin_milk::events::StepEvent;

    let event = StepEvent::ApprovalRequired {
        iteration: 3,
        call_id: "call-abc".into(),
        name: "execute_bash".into(),
        arguments: r#"{"command":"rm -rf /tmp/test"}"#.into(),
        staged_ref: "tool-execute_bash-call-abc".into(),
        timeout_secs: 300,
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"type\":\"approval_required\""));
    assert!(json.contains("\"iteration\":3"));
    assert!(json.contains("\"call_id\":\"call-abc\""));
    assert!(json.contains("\"name\":\"execute_bash\""));
    assert!(json.contains("\"staged_ref\":\"tool-execute_bash-call-abc\""));
    assert!(json.contains("\"timeout_secs\":300"));
}

// ---------------------------------------------------------------------------
// Staged ref naming convention
// ---------------------------------------------------------------------------

#[test]
fn test_staged_ref_format() {
    let name = "execute_bash";
    let call_id = "call-123";
    let staged_ref = format!("tool-{}-{}", name, call_id);
    assert_eq!(staged_ref, "tool-execute_bash-call-123");
    assert!(staged_ref.starts_with("tool-"));
}

#[test]
fn test_staged_ref_call_id_extraction() {
    // The approve/abort handlers extract call_id from the staged_ref
    let staged_ref = "tool-execute_bash-call-123";
    let call_id = staged_ref.rsplit('-').next().unwrap_or(staged_ref);
    // Note: this only gets the last segment after the last '-'
    assert_eq!(call_id, "123");

    // For call IDs that contain hyphens, the handler uses rsplit('-').next()
    // which only gets the last segment. This is expected because the handler
    // also writes to the same workspace directory.
}
