//! Tests for tools — registry and sandbox tools.
//! Mirrors Python tests/test_tools.py (14 tests).

use serde_json::json;
use std::collections::HashSet;
use tempfile::TempDir;

use dolphin_milk::tools::registry::{ToolDef, ToolRegistry, ALWAYS_ON_TOOLS};
use dolphin_milk::tools::sandbox::{all_sandbox_tools, is_excluded_path, SEARCH_EXCLUDED_DIRS};
use dolphin_milk::tools::wallet_tools::all_wallet_tools;

// -- TestToolRegistry --

#[test]
fn test_register_and_get() {
    let mut reg = ToolRegistry::new();
    let tool = ToolDef {
        name: "test_tool".to_string(),
        description: "A test tool".to_string(),
        parameters: json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    };
    reg.register(tool);
    assert!(reg.get("test_tool").is_some());
    assert_eq!(reg.tool_count(), 1);
}

#[tokio::test]
async fn test_execute() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "echo".to_string(),
        description: "echo".to_string(),
        parameters: json!({"type": "object", "properties": {}}),
        execute: Box::new(|p| {
            Box::pin(async move {
                let msg = p.get("msg").and_then(|v| v.as_str()).unwrap_or("");
                format!("echoed: {msg}")
            })
        }),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    let result = reg
        .execute("echo", json!({"msg": "hello"}), None)
        .await
        .unwrap();
    assert_eq!(result, "echoed: hello");
}

#[tokio::test]
async fn test_execute_unknown_tool() {
    let reg = ToolRegistry::new();
    let result = reg.execute("nonexistent", json!({}), None).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown tool"), "got: {msg}");
}

#[tokio::test]
async fn test_allowlist() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "allowed".to_string(),
        description: "".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    reg.register(ToolDef {
        name: "blocked".to_string(),
        description: "".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    reg.set_allowlist(HashSet::from(["allowed".to_string()]));
    assert!(reg.is_allowed("allowed"));
    assert!(!reg.is_allowed("blocked"));

    let result = reg.execute("blocked", json!({}), None).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not allowed"), "got: {msg}");
}

#[test]
fn test_list_descriptions() {
    let mut reg = ToolRegistry::new();
    for tool in all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000) {
        reg.register(tool);
    }
    let descs = reg.list_descriptions();
    assert_eq!(descs.len(), 7);
    let names: HashSet<String> = descs.iter().map(|d| d.name.clone()).collect();
    assert!(names.contains("execute_bash"));
    assert!(names.contains("file_read"));
}

#[test]
fn test_to_openai_tools() {
    let mut reg = ToolRegistry::new();
    for tool in all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000) {
        reg.register(tool);
    }
    let openai_tools = reg.to_openai_tools();
    assert_eq!(openai_tools.len(), 7);
    assert!(openai_tools.iter().all(|t| t["type"] == "function"));
    assert!(openai_tools.iter().all(|t| t.get("function").is_some()));
}

#[test]
fn test_tool_names() {
    let mut reg = ToolRegistry::new();
    for tool in all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000) {
        reg.register(tool);
    }
    let names = reg.tool_names();
    assert_eq!(names.len(), 7);
}

// -- TestSandboxTools --

#[tokio::test]
async fn test_execute_bash_echo() {
    let dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let bash = tools.iter().find(|t| t.name == "execute_bash").unwrap();
    let result = (bash.execute)(json!({"command": "echo hello"})).await;
    assert!(result.contains("hello"), "got: {result}");
}

#[tokio::test]
async fn test_execute_bash_exit_code() {
    let dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let bash = tools.iter().find(|t| t.name == "execute_bash").unwrap();
    let result = (bash.execute)(json!({"command": "exit 1"})).await;
    assert!(result.contains("exit code 1"), "got: {result}");
}

#[tokio::test]
async fn test_execute_bash_timeout() {
    let dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let bash = tools.iter().find(|t| t.name == "execute_bash").unwrap();
    let result = (bash.execute)(json!({"command": "sleep 10", "timeout": 1})).await;
    assert!(result.contains("timed out"), "got: {result}");
}

#[tokio::test]
async fn test_execute_bash_empty() {
    let dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let bash = tools.iter().find(|t| t.name == "execute_bash").unwrap();
    let result = (bash.execute)(json!({})).await;
    assert!(result.contains("Error"), "got: {result}");
}

#[tokio::test]
async fn test_file_read_write() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.txt");
    let path_str = path.to_str().unwrap();

    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let write_tool = tools.iter().find(|t| t.name == "file_write").unwrap();
    let read_tool = tools.iter().find(|t| t.name == "file_read").unwrap();

    (write_tool.execute)(json!({"path": path_str, "content": "hello world"})).await;
    let result = (read_tool.execute)(json!({"path": path_str})).await;
    assert!(result.contains("hello world"), "got: {result}");
}

#[tokio::test]
async fn test_file_read_not_found() {
    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let read_tool = tools.iter().find(|t| t.name == "file_read").unwrap();
    let result = (read_tool.execute)(json!({"path": "/nonexistent/file.txt"})).await;
    assert!(result.contains("not found"), "got: {result}");
}

#[tokio::test]
async fn test_file_read_offset_limit() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.txt");
    let path_str = path.to_str().unwrap();

    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let write_tool = tools.iter().find(|t| t.name == "file_write").unwrap();
    let read_tool = tools.iter().find(|t| t.name == "file_read").unwrap();

    (write_tool.execute)(json!({"path": path_str, "content": "line0\nline1\nline2\nline3"})).await;
    let result = (read_tool.execute)(json!({"path": path_str, "offset": 1, "limit": 2})).await;
    assert!(result.contains("line1"), "got: {result}");
    assert!(result.contains("line2"), "got: {result}");
    assert!(
        !result.contains("line0"),
        "should not contain line0, got: {result}"
    );
}

#[tokio::test]
async fn test_file_search_glob() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("test.py"), "print('hello')").unwrap();
    std::fs::write(dir.path().join("test.txt"), "world").unwrap();

    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let search_tool = tools.iter().find(|t| t.name == "file_search").unwrap();
    let result = (search_tool.execute)(
        json!({"pattern": "*.py", "directory": dir.path().to_str().unwrap()}),
    )
    .await;
    assert!(result.contains("test.py"), "got: {result}");
}

#[test]
fn test_all_sandbox_tools_registered() {
    assert_eq!(
        all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000).len(),
        7
    );
}

// -- web_fetch (now in sandbox) --

#[tokio::test]
async fn test_web_fetch_validates_url() {
    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let web_fetch = tools.iter().find(|t| t.name == "web_fetch").unwrap();
    let result = (web_fetch.execute)(json!({})).await;
    assert!(result.starts_with("Error"), "got: {result}");
    assert!(result.contains("url"), "got: {result}");
}

#[tokio::test]
async fn test_web_fetch_validates_empty_url() {
    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let web_fetch = tools.iter().find(|t| t.name == "web_fetch").unwrap();
    let result = (web_fetch.execute)(json!({"url": ""})).await;
    assert!(result.starts_with("Error"), "got: {result}");
    assert!(result.contains("url"), "got: {result}");
}

#[tokio::test]
async fn test_web_fetch_validates_url_scheme() {
    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let web_fetch = tools.iter().find(|t| t.name == "web_fetch").unwrap();
    let result = (web_fetch.execute)(json!({"url": "ftp://example.com"})).await;
    assert!(result.starts_with("Error"), "got: {result}");
    assert!(result.contains("http"), "got: {result}");
}

#[test]
fn test_web_fetch_is_sandbox_category() {
    let tools = all_sandbox_tools(std::path::PathBuf::from("test-workspace"), 128_000);
    let web_fetch = tools.iter().find(|t| t.name == "web_fetch").unwrap();
    assert_eq!(web_fetch.category, "sandbox");
}

// -- file_read size guard --

#[tokio::test]
async fn test_file_read_rejects_oversized_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("huge.bin");
    // Create a file larger than max_file_read * 4 for a small context window.
    // With context_window=1000: max_tool_result = 1000*0.3*4 = 1200, gate = 1200*4 = 4800
    let content = "x".repeat(5000);
    std::fs::write(&path, &content).unwrap();

    let tools = all_sandbox_tools(dir.path().to_path_buf(), 1000);
    let read_tool = tools.iter().find(|t| t.name == "file_read").unwrap();
    let result = (read_tool.execute)(json!({"path": path.to_str().unwrap()})).await;
    assert!(
        result.contains("too large"),
        "should reject oversized file, got: {result}"
    );
    assert!(
        result.contains("MB"),
        "should show size in MB, got: {result}"
    );
}

#[tokio::test]
async fn test_file_read_allows_normal_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("normal.txt");
    std::fs::write(&path, "hello world").unwrap();

    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let read_tool = tools.iter().find(|t| t.name == "file_read").unwrap();
    let result = (read_tool.execute)(json!({"path": path.to_str().unwrap()})).await;
    assert!(
        result.contains("hello world"),
        "should read normal file, got: {result}"
    );
}

// -- file_write workspace resolution (Phase 1) --

#[tokio::test]
async fn test_file_write_workspace_resolution() {
    let dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let write_tool = tools.iter().find(|t| t.name == "file_write").unwrap();

    // Relative path should resolve to workspace directory
    let result =
        (write_tool.execute)(json!({"path": "test-output.txt", "content": "hello workspace"}))
            .await;
    assert!(result.contains("Written"), "got: {result}");
    assert!(
        result.contains(&dir.path().display().to_string()),
        "should contain absolute workspace path, got: {result}"
    );

    // Verify the file actually landed in the workspace
    let file_path = dir.path().join("test-output.txt");
    assert!(file_path.exists(), "file should exist in workspace");
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "hello workspace");
}

#[tokio::test]
async fn test_file_write_absolute_path_unchanged() {
    let dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let write_tool = tools.iter().find(|t| t.name == "file_write").unwrap();

    // Absolute path should NOT be resolved against workspace
    let abs_path = output_dir.path().join("absolute-test.txt");
    let abs_path_str = abs_path.to_str().unwrap();
    let result =
        (write_tool.execute)(json!({"path": abs_path_str, "content": "absolute write"})).await;
    assert!(result.contains("Written"), "got: {result}");
    assert!(
        result.contains(abs_path_str),
        "should contain the absolute path, got: {result}"
    );

    // Verify it's at the absolute path, NOT in workspace
    assert!(abs_path.exists(), "file should exist at absolute path");
    assert!(
        !dir.path().join("absolute-test.txt").exists(),
        "file should NOT be in workspace"
    );
}

#[tokio::test]
async fn test_file_write_nested_relative_path() {
    let dir = TempDir::new().unwrap();
    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let write_tool = tools.iter().find(|t| t.name == "file_write").unwrap();

    // Nested relative path should create subdirectories in workspace
    let result = (write_tool.execute)(
        json!({"path": "subdir/nested/file.txt", "content": "nested content"}),
    )
    .await;
    assert!(result.contains("Written"), "got: {result}");

    let file_path = dir.path().join("subdir/nested/file.txt");
    assert!(file_path.exists(), "nested file should exist in workspace");
}

#[test]
fn test_all_wallet_tools_registered() {
    // Phase 2: consolidated from 14 → 6, then +2 (receive_address, fund_from_tx)
    assert_eq!(
        all_wallet_tools("http://localhost:3322".to_string()).len(),
        8
    );
}

#[test]
fn test_wallet_call_tool_registered() {
    let tools = all_wallet_tools("http://localhost:3322".to_string());
    let wallet_call = tools.iter().find(|t| t.name == "wallet_call");
    assert!(
        wallet_call.is_some(),
        "wallet_call tool should be registered"
    );
    let tool = wallet_call.unwrap();
    assert_eq!(tool.category, "wallet");
    // Verify required endpoint param in schema
    let required = tool.parameters.get("required").unwrap();
    assert!(required.as_array().unwrap().contains(&json!("endpoint")));
}

#[tokio::test]
async fn test_wallet_call_missing_endpoint() {
    let tools = all_wallet_tools("http://localhost:3322".to_string());
    let wallet_call = tools.iter().find(|t| t.name == "wallet_call").unwrap();
    let result = (wallet_call.execute)(json!({})).await;
    assert!(
        result.contains("Error"),
        "should error without endpoint: {result}"
    );
    assert!(
        result.contains("endpoint"),
        "should mention 'endpoint' param"
    );
}

#[tokio::test]
async fn test_wallet_call_accepts_create_action() {
    // Phase 2: createAction is no longer blocked — wallet_call handles all endpoints.
    // This test verifies it attempts the call (will fail due to no wallet, but not "blocked").
    let tools = all_wallet_tools("http://localhost:3322".to_string());
    let wallet_call = tools.iter().find(|t| t.name == "wallet_call").unwrap();
    let result = (wallet_call.execute)(
        json!({"endpoint": "createAction", "params": {"description": "test"}}),
    )
    .await;
    // Should NOT contain "blocked" — it should attempt the call and fail on network
    assert!(
        !result.contains("blocked"),
        "createAction should not be blocked: {result}"
    );
}

// -- Phase 2.3: Tool Search Index --

#[test]
fn test_always_on_tools_count() {
    // 17 after Phase 2 of the delegation epic (#329) added `delegate_task`
    // as an always-on tool. Bump this count whenever a new entry joins
    // ALWAYS_ON_TOOLS and document the new essential in the adjacent
    // test_always_on_tools_contains_essentials test.
    assert_eq!(ALWAYS_ON_TOOLS.len(), 17);
}

#[test]
fn test_always_on_tools_contains_essentials() {
    assert!(ALWAYS_ON_TOOLS.contains(&"execute_bash"));
    assert!(ALWAYS_ON_TOOLS.contains(&"memory_search"));
    assert!(ALWAYS_ON_TOOLS.contains(&"wallet_balance"));
    assert!(ALWAYS_ON_TOOLS.contains(&"x402_call"));
    assert!(ALWAYS_ON_TOOLS.contains(&"search_tools"));
    // Phase 2 of delegation epic (#329) — the end-to-end delegation tool
    // must be discoverable by the LLM without a search_tools hop.
    assert!(ALWAYS_ON_TOOLS.contains(&"delegate_task"));
}

#[test]
fn test_always_on_tools_excludes_discoverable() {
    // These should be discoverable via search_tools, not always-on
    assert!(!ALWAYS_ON_TOOLS.contains(&"wallet_encrypt"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"wallet_decrypt"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"send_message"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"check_inbox"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"create_schedule"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"generate_image"));
    assert!(!ALWAYS_ON_TOOLS.contains(&"list_conversations"));
}

#[test]
fn test_list_prompt_tools_filters_to_always_on() {
    let mut reg = ToolRegistry::new();
    // Register one always-on tool
    reg.register(ToolDef {
        name: "execute_bash".to_string(),
        description: "Run commands".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // Register one discoverable tool
    reg.register(ToolDef {
        name: "send_message".to_string(),
        description: "Send a message".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "messagebox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });

    let prompt_tools = reg.list_prompt_tools();
    let all_tools = reg.list_descriptions();

    assert_eq!(prompt_tools.len(), 1);
    assert_eq!(prompt_tools[0].name, "execute_bash");
    assert_eq!(all_tools.len(), 2); // list_descriptions still returns all
}

#[test]
fn test_list_prompt_tools_respects_allowlist() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "execute_bash".to_string(),
        description: "Run commands".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    reg.register(ToolDef {
        name: "file_read".to_string(),
        description: "Read files".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });

    // Both are always-on, but allowlist only permits one
    let mut allowed = std::collections::HashSet::new();
    allowed.insert("execute_bash".to_string());
    reg.set_allowlist(allowed);

    let prompt_tools = reg.list_prompt_tools();
    assert_eq!(prompt_tools.len(), 1);
    assert_eq!(prompt_tools[0].name, "execute_bash");
}

#[test]
fn test_all_tool_summaries_returns_all() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "tool_a".to_string(),
        description: "Description A".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    reg.register(ToolDef {
        name: "tool_b".to_string(),
        description: "Description B".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });

    // Set an allowlist that only allows tool_a
    let mut allowed = std::collections::HashSet::new();
    allowed.insert("tool_a".to_string());
    reg.set_allowlist(allowed);

    // all_tool_summaries ignores allowlist
    let summaries = reg.all_tool_summaries();
    assert_eq!(summaries.len(), 2);
    let names: Vec<&str> = summaries.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"tool_a"));
    assert!(names.contains(&"tool_b"));
}

// -- file_search exclusion tests --

#[test]
fn test_search_excluded_dirs_contains_target() {
    assert!(SEARCH_EXCLUDED_DIRS.contains(&"target"));
    assert!(SEARCH_EXCLUDED_DIRS.contains(&"node_modules"));
    assert!(SEARCH_EXCLUDED_DIRS.contains(&".git"));
}

#[test]
fn test_is_excluded_path_target() {
    assert!(is_excluded_path("target/release/deps/foo.rlib"));
    assert!(is_excluded_path("./target/debug/build/bar.rs"));
    assert!(is_excluded_path("some/dir/target/nested/file.rs"));
}

#[test]
fn test_is_excluded_path_node_modules() {
    assert!(is_excluded_path("node_modules/foo/index.js"));
    assert!(is_excluded_path("ui/node_modules/@bsv/sdk/lib.js"));
    assert!(is_excluded_path("./node_modules/bar.js"));
}

#[test]
fn test_is_excluded_path_git() {
    assert!(is_excluded_path(".git/objects/ab/cd1234"));
    assert!(is_excluded_path("./.git/HEAD"));
}

#[test]
fn test_is_excluded_path_allows_normal_paths() {
    assert!(!is_excluded_path("src/main.rs"));
    assert!(!is_excluded_path("tests/test_tools.rs"));
    assert!(!is_excluded_path("working/tasks/abc/session.jsonl"));
    assert!(!is_excluded_path("ui/src/app.ts"));
}

#[tokio::test]
async fn test_file_search_excludes_target_dir() {
    let dir = TempDir::new().unwrap();
    // Create files: one in a normal dir, one in "target/"
    let src_dir = dir.path().join("src");
    let target_dir = dir.path().join("target/debug");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();
    std::fs::write(target_dir.join("artifact.rs"), "compiled").unwrap();

    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let search_tool = tools.iter().find(|t| t.name == "file_search").unwrap();
    let result = (search_tool.execute)(json!({
        "pattern": "**/*.rs",
        "directory": dir.path().to_str().unwrap()
    }))
    .await;

    assert!(
        result.contains("main.rs"),
        "should find src/main.rs, got: {result}"
    );
    assert!(
        !result.contains("artifact.rs"),
        "should exclude target/artifact.rs, got: {result}"
    );
}

#[tokio::test]
async fn test_file_search_excludes_node_modules() {
    let dir = TempDir::new().unwrap();
    let src_dir = dir.path().join("ui/src");
    let nm_dir = dir.path().join("node_modules/foo");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&nm_dir).unwrap();
    std::fs::write(src_dir.join("app.ts"), "export default {}").unwrap();
    std::fs::write(nm_dir.join("index.ts"), "module").unwrap();

    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let search_tool = tools.iter().find(|t| t.name == "file_search").unwrap();
    let result = (search_tool.execute)(json!({
        "pattern": "**/*.ts",
        "directory": dir.path().to_str().unwrap()
    }))
    .await;

    assert!(
        result.contains("app.ts"),
        "should find ui/src/app.ts, got: {result}"
    );
    assert!(
        !result.contains("index.ts"),
        "should exclude node_modules/foo/index.ts, got: {result}"
    );
}

#[tokio::test]
async fn test_file_search_content_grep_excludes_target() {
    let dir = TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    let target_dir = dir.path().join("target");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(src_dir.join("lib.rs"), "persistent storage guide").unwrap();
    std::fs::write(target_dir.join("cached.rs"), "persistent storage guide").unwrap();

    let tools = all_sandbox_tools(dir.path().to_path_buf(), 128_000);
    let search_tool = tools.iter().find(|t| t.name == "file_search").unwrap();
    let result = (search_tool.execute)(json!({
        "content_pattern": "persistent storage",
        "directory": dir.path().to_str().unwrap()
    }))
    .await;

    assert!(
        result.contains("lib.rs"),
        "should find src/lib.rs, got: {result}"
    );
    assert!(
        !result.contains("cached.rs"),
        "should exclude target/cached.rs, got: {result}"
    );
}

// -- Phase 10.2: Capability enforcement tests --

use dolphin_milk::tools::registry::required_capability;

#[test]
fn test_required_capability_sandbox() {
    assert_eq!(required_capability("sandbox"), "tools");
}

#[test]
fn test_required_capability_system() {
    assert_eq!(required_capability("system"), "tools");
}

#[test]
fn test_required_capability_wallet() {
    assert_eq!(required_capability("wallet"), "wallet");
}

#[test]
fn test_required_capability_messagebox() {
    assert_eq!(required_capability("messagebox"), "messaging");
}

#[test]
fn test_required_capability_conversation() {
    assert_eq!(required_capability("conversation"), "messaging");
}

#[test]
fn test_required_capability_x402() {
    assert_eq!(required_capability("x402"), "tools");
}

#[test]
fn test_required_capability_memory() {
    assert_eq!(required_capability("memory"), "memory");
}

#[test]
fn test_required_capability_schedule() {
    assert_eq!(required_capability("schedule"), "schedule");
}

#[test]
fn test_required_capability_browser() {
    assert_eq!(required_capability("browser"), "tools");
}

#[test]
fn test_required_capability_unknown_defaults_to_tools() {
    assert_eq!(required_capability("unknown_category"), "tools");
    assert_eq!(required_capability(""), "tools");
}

#[tokio::test]
async fn test_execute_with_matching_capability() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "my_tool".to_string(),
        description: "test".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // "tools" capability matches sandbox category
    let caps = vec!["tools".to_string()];
    let result = reg.execute("my_tool", json!({}), Some(&caps)).await;
    assert!(result.is_ok(), "should succeed with matching capability");
    assert_eq!(result.unwrap(), "ok");
}

#[tokio::test]
async fn test_execute_with_missing_capability() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "send_message".to_string(),
        description: "test".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "messagebox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // "tools,llm" does NOT include "messaging"
    let caps = vec!["tools".to_string(), "llm".to_string()];
    let result = reg.execute("send_message", json!({}), Some(&caps)).await;
    assert!(result.is_err(), "should fail with missing capability");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Insufficient capability"), "got: {err}");
    assert!(
        err.contains("messaging"),
        "should mention required capability: {err}"
    );
}

#[tokio::test]
async fn test_execute_none_capabilities_skips_check() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "secret_tool".to_string(),
        description: "test".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "x402".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // None = no capability check (backward compat)
    let result = reg.execute("secret_tool", json!({}), None).await;
    assert!(result.is_ok(), "None capabilities should skip check");
}

#[tokio::test]
async fn test_execute_all_capability_bypasses() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "wallet_balance".to_string(),
        description: "test".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "wallet".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // "all" should bypass capability checks for any category
    let caps = vec!["all".to_string()];
    let result = reg.execute("wallet_balance", json!({}), Some(&caps)).await;
    assert!(
        result.is_ok(),
        "\"all\" capability should bypass all checks"
    );
}

#[tokio::test]
async fn test_execute_tools_llm_cannot_call_send_message() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "send_message".to_string(),
        description: "Send a message".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "sent".to_string() })),
        category: "messagebox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    let caps = vec!["tools".to_string(), "llm".to_string()];
    let result = reg.execute("send_message", json!({}), Some(&caps)).await;
    assert!(
        result.is_err(),
        "tools+llm should not allow messagebox tools"
    );
}

#[tokio::test]
async fn test_execute_multiple_capabilities() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "memory_search".to_string(),
        description: "search".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "found".to_string() })),
        category: "memory".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    reg.register(ToolDef {
        name: "x402_call".to_string(),
        description: "call".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "called".to_string() })),
        category: "x402".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    let caps = vec!["memory".to_string(), "tools".to_string()];

    // Both should succeed (x402 category now requires "tools" capability)
    let r1 = reg.execute("memory_search", json!({}), Some(&caps)).await;
    assert!(r1.is_ok(), "memory should be allowed");
    let r2 = reg.execute("x402_call", json!({}), Some(&caps)).await;
    assert!(r2.is_ok(), "x402 should be allowed with 'tools' capability");
}

#[tokio::test]
async fn test_execute_capability_check_after_allowlist() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "execute_bash".to_string(),
        description: "run commands".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // Tool is allowed by allowlist but blocked by capabilities
    let caps = vec!["memory".to_string()]; // only memory, no "tools"
    let result = reg.execute("execute_bash", json!({}), Some(&caps)).await;
    assert!(
        result.is_err(),
        "should fail: capability check blocks even if allowlist passes"
    );
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Insufficient capability"), "got: {err}");
}

#[tokio::test]
async fn test_execute_empty_capabilities_blocks_all() {
    let mut reg = ToolRegistry::new();
    reg.register(ToolDef {
        name: "my_tool".to_string(),
        description: "test".to_string(),
        parameters: json!({}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });
    // Empty capability list = nothing is authorized
    let caps: Vec<String> = vec![];
    let result = reg.execute("my_tool", json!({}), Some(&caps)).await;
    assert!(result.is_err(), "empty capabilities should block all tools");
}
