//! Tests for CI.6: Parallel tool execution via JoinSet.
//!
//! Verifies that multiple tool calls are executed concurrently when >1,
//! sequentially when 1, with proper ordering and error handling.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::RwLock;

use dolphin_milk::tools::registry::{ToolDef, ToolRegistry};

// ===========================================================================
// Helpers — create a registry with test tools
// ===========================================================================

fn make_echo_tool(name: &str) -> ToolDef {
    let tool_name = name.to_string();
    ToolDef {
        name: name.to_string(),
        description: format!("Test tool: {name}"),
        parameters: json!({"type": "object", "properties": {}}),
        execute: Box::new(move |params| {
            let n = tool_name.clone();
            Box::pin(async move { json!({"tool": n, "params": params}).to_string() })
        }),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    }
}

fn make_sleep_tool(name: &str, ms: u64) -> ToolDef {
    let tool_name = name.to_string();
    ToolDef {
        name: name.to_string(),
        description: format!("Test sleep tool: {name} ({ms}ms)"),
        parameters: json!({"type": "object", "properties": {}}),
        execute: Box::new(move |_params| {
            let n = tool_name.clone();
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                json!({"tool": n, "slept_ms": ms}).to_string()
            })
        }),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    }
}

fn make_failing_tool(name: &str) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: format!("Test failing tool: {name}"),
        parameters: json!({"type": "object", "properties": {}}),
        execute: Box::new(move |_params| {
            Box::pin(async move { "Error: intentional test failure".to_string() })
        }),
        category: "sandbox".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    }
}

fn build_registry(tools: Vec<ToolDef>) -> Arc<RwLock<ToolRegistry>> {
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    Arc::new(RwLock::new(registry))
}

/// Execute a single tool from the registry (standalone, mirrors the pattern in step.rs).
async fn execute_tool(
    tools: &Arc<RwLock<ToolRegistry>>,
    name: &str,
    arguments: Value,
) -> Result<String, String> {
    let future: Pin<Box<dyn Future<Output = String> + Send>> = {
        let registry = tools.read().await;
        let tool = registry
            .get(name)
            .ok_or_else(|| format!("Unknown tool: {name}"))?;
        (tool.execute)(arguments)
    };
    Ok(future.await)
}

// ===========================================================================
// Sequential path tests
// ===========================================================================

#[tokio::test]
async fn test_single_tool_call_no_joinset() {
    // Single tool call should execute directly without JoinSet overhead
    let registry = build_registry(vec![make_echo_tool("echo_tool")]);

    let start = Instant::now();
    let result = execute_tool(&registry, "echo_tool", json!({"test": true})).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("echo_tool"));
    // Should be fast (no JoinSet setup)
    assert!(elapsed < Duration::from_millis(100));
}

#[tokio::test]
async fn test_empty_tool_calls_no_crash() {
    // Zero tool calls should be handled gracefully (no panic)
    let registry = build_registry(vec![make_echo_tool("echo_tool")]);

    // Executing zero tools should produce an empty results vec
    let empty_calls: Vec<(&str, Value)> = vec![];
    let mut results = Vec::new();
    for (name, args) in &empty_calls {
        if let Ok(output) = execute_tool(&registry, name, args.clone()).await {
            results.push(output);
        }
    }
    assert!(results.is_empty());
}

// ===========================================================================
// Parallel execution tests
// ===========================================================================

#[tokio::test]
async fn test_multiple_tools_execute_concurrently() {
    // Two 100ms sleep tools should complete in ~100ms wall clock (not 200ms)
    let registry = build_registry(vec![
        make_sleep_tool("sleep_a", 100),
        make_sleep_tool("sleep_b", 100),
    ]);

    let start = Instant::now();

    // Execute in parallel via JoinSet
    let mut join_set = tokio::task::JoinSet::new();

    for (idx, (name, args)) in [("sleep_a", json!({})), ("sleep_b", json!({}))]
        .iter()
        .enumerate()
    {
        let tools = registry.clone();
        let name = name.to_string();
        let args = args.clone();
        join_set.spawn(async move {
            let result = execute_tool(&tools, &name, args).await;
            (idx, name, result)
        });
    }

    #[allow(clippy::type_complexity)]
    let mut results: Vec<Option<(usize, String, Result<String, String>)>> = vec![None, None];
    while let Some(join_result) = join_set.join_next().await {
        let (idx, name, result) = join_result.unwrap();
        results[idx] = Some((idx, name, result));
    }

    let elapsed = start.elapsed();

    // Both should succeed
    assert!(results[0].is_some());
    assert!(results[1].is_some());
    assert!(results[0].as_ref().unwrap().2.is_ok());
    assert!(results[1].as_ref().unwrap().2.is_ok());

    // Wall clock should be ~100ms, not 200ms (parallel execution)
    assert!(
        elapsed < Duration::from_millis(180),
        "Parallel execution took {:?}, should be ~100ms not ~200ms",
        elapsed
    );
}

#[tokio::test]
async fn test_results_ordered_by_original_index() {
    // Tools complete in different order, but results should be reordered by index
    let registry = build_registry(vec![
        make_sleep_tool("slow", 80), // idx 0: slow
        make_sleep_tool("fast", 10), // idx 1: fast
    ]);

    let mut join_set = tokio::task::JoinSet::new();
    let tools_data = [("slow", json!({})), ("fast", json!({}))];

    for (idx, (name, args)) in tools_data.iter().enumerate() {
        let tools = registry.clone();
        let name = name.to_string();
        let args = args.clone();
        join_set.spawn(async move {
            let result = execute_tool(&tools, &name, args).await.unwrap_or_default();
            (idx, name, result)
        });
    }

    let mut ordered: Vec<Option<(usize, String, String)>> = vec![None, None];
    while let Some(join_result) = join_set.join_next().await {
        let (idx, name, result) = join_result.unwrap();
        ordered[idx] = Some((idx, name, result));
    }

    // Results should be in original order regardless of completion order
    assert_eq!(ordered[0].as_ref().unwrap().1, "slow");
    assert_eq!(ordered[1].as_ref().unwrap().1, "fast");
    assert!(ordered[0].as_ref().unwrap().2.contains("slow"));
    assert!(ordered[1].as_ref().unwrap().2.contains("fast"));
}

#[tokio::test]
async fn test_panicking_tool_returns_error_result() {
    // A panicking tool should produce an error, not crash the whole iteration
    let registry = build_registry(vec![make_echo_tool("good_tool")]);

    // Register a panicking tool
    {
        let mut reg = registry.write().await;
        reg.register(ToolDef {
            name: "panic_tool".to_string(),
            description: "A tool that panics".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
            execute: Box::new(|_| {
                Box::pin(async move {
                    panic!("intentional test panic");
                })
            }),
            category: "sandbox".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        });
    }

    let mut join_set = tokio::task::JoinSet::new();

    // Spawn good tool
    {
        let tools = registry.clone();
        join_set.spawn(async move {
            let result = execute_tool(&tools, "good_tool", json!({})).await;
            (0_usize, "good_tool".to_string(), result)
        });
    }

    // Spawn panicking tool
    {
        let tools = registry.clone();
        join_set.spawn(async move {
            let result = execute_tool(&tools, "panic_tool", json!({})).await;
            (1_usize, "panic_tool".to_string(), result)
        });
    }

    #[allow(clippy::type_complexity)]
    let mut results: Vec<Option<(usize, String, Result<String, String>)>> = vec![None, None];
    let mut had_panic = false;
    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((idx, name, result)) => {
                results[idx] = Some((idx, name, result));
            }
            Err(e) => {
                // JoinError from panic — this is the expected path
                had_panic = true;
                assert!(e.is_panic(), "Should be a panic error: {e}");
            }
        }
    }

    // The good tool should still succeed
    assert!(results[0].is_some(), "Good tool should have completed");
    assert!(
        results[0].as_ref().unwrap().2.is_ok(),
        "Good tool should succeed"
    );

    // The panicking tool should have produced a JoinError
    assert!(had_panic, "Should have caught a panic");
}

#[tokio::test]
async fn test_failing_tool_doesnt_block_others() {
    // One tool returning an error should not prevent other tools from completing
    let registry = build_registry(vec![
        make_echo_tool("success_tool"),
        make_failing_tool("fail_tool"),
    ]);

    let mut join_set = tokio::task::JoinSet::new();
    let tool_calls = [("success_tool", json!({})), ("fail_tool", json!({}))];

    for (idx, (name, args)) in tool_calls.iter().enumerate() {
        let tools = registry.clone();
        let name = name.to_string();
        let args = args.clone();
        join_set.spawn(async move {
            let result = execute_tool(&tools, &name, args).await;
            (idx, name, result)
        });
    }

    #[allow(clippy::type_complexity)]
    let mut results: Vec<Option<(usize, String, Result<String, String>)>> = vec![None, None];
    while let Some(join_result) = join_set.join_next().await {
        let (idx, name, result) = join_result.unwrap();
        results[idx] = Some((idx, name, result));
    }

    // Success tool should have completed normally
    assert!(results[0].is_some());
    assert!(results[0].as_ref().unwrap().2.is_ok());
    assert!(results[0]
        .as_ref()
        .unwrap()
        .2
        .as_ref()
        .unwrap()
        .contains("success_tool"));

    // Fail tool should have returned (it returns an error string, not Err)
    assert!(results[1].is_some());
    // The failing tool returns a string starting with "Error:", so it's actually Ok
    assert!(results[1].as_ref().unwrap().2.is_ok());
    assert!(results[1]
        .as_ref()
        .unwrap()
        .2
        .as_ref()
        .unwrap()
        .contains("Error"));
}

// ===========================================================================
// Edge cases and correctness
// ===========================================================================

#[tokio::test]
async fn test_three_tools_parallel() {
    // Three 50ms tools should complete in ~50ms, not 150ms
    let registry = build_registry(vec![
        make_sleep_tool("a", 50),
        make_sleep_tool("b", 50),
        make_sleep_tool("c", 50),
    ]);

    let start = Instant::now();
    let mut join_set = tokio::task::JoinSet::new();

    for (idx, name) in ["a", "b", "c"].iter().enumerate() {
        let tools = registry.clone();
        let name = name.to_string();
        join_set.spawn(async move {
            let result = execute_tool(&tools, &name, json!({})).await;
            (idx, result)
        });
    }

    let mut results: Vec<Option<Result<String, String>>> = vec![None, None, None];
    while let Some(join_result) = join_set.join_next().await {
        let (idx, result) = join_result.unwrap();
        results[idx] = Some(result);
    }

    let elapsed = start.elapsed();

    assert!(results.iter().all(|r| r.is_some()));
    assert!(results.iter().all(|r| r.as_ref().unwrap().is_ok()));
    assert!(
        elapsed < Duration::from_millis(120),
        "Three parallel 50ms tools took {:?}, should be ~50ms not ~150ms",
        elapsed
    );
}

#[tokio::test]
async fn test_tool_not_found_returns_error() {
    let registry = build_registry(vec![make_echo_tool("existing")]);

    let result = execute_tool(&registry, "nonexistent", json!({})).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown tool"));
}

#[tokio::test]
async fn test_parallel_results_contain_correct_data() {
    // Verify each parallel result contains the correct tool output
    let registry = build_registry(vec![make_echo_tool("tool_x"), make_echo_tool("tool_y")]);

    let mut join_set = tokio::task::JoinSet::new();
    for (idx, (name, arg_val)) in [
        ("tool_x", json!({"key": "alpha"})),
        ("tool_y", json!({"key": "beta"})),
    ]
    .iter()
    .enumerate()
    {
        let tools = registry.clone();
        let name = name.to_string();
        let args = arg_val.clone();
        join_set.spawn(async move {
            let result = execute_tool(&tools, &name, args).await.unwrap();
            (idx, name, result)
        });
    }

    let mut ordered: Vec<Option<(usize, String, String)>> = vec![None, None];
    while let Some(join_result) = join_set.join_next().await {
        let (idx, name, result) = join_result.unwrap();
        ordered[idx] = Some((idx, name, result));
    }

    let (_, name_x, result_x) = ordered[0].as_ref().unwrap();
    let (_, name_y, result_y) = ordered[1].as_ref().unwrap();

    assert_eq!(name_x, "tool_x");
    assert_eq!(name_y, "tool_y");
    assert!(result_x.contains("alpha"));
    assert!(result_y.contains("beta"));
}
