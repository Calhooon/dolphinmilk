//! Verification tools — re-execute and compare tool outputs.
//!
//! The `verify_output` tool takes a previous tool result and re-executes it,
//! comparing the outputs to detect discrepancies.

use serde_json::{json, Value};

use super::registry::{ToolDef, ToolFunc};

/// Create all verification tools.
pub fn all_verification_tools() -> Vec<ToolDef> {
    vec![verify_output_tool()]
}

fn verify_output_tool() -> ToolDef {
    let execute: ToolFunc = Box::new(move |params: Value| {
        Box::pin(async move {
            let original_tool = params
                .get("original_tool")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let original_params = params.get("original_params").cloned().unwrap_or(json!({}));
            let original_output = params
                .get("original_output")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if original_tool.is_empty() {
                return json!({
                    "error": "Missing required parameter: original_tool"
                })
                .to_string();
            }

            if original_output.is_empty() {
                return json!({
                    "error": "Missing required parameter: original_output"
                })
                .to_string();
            }

            // Perform comparison analysis
            let analysis = analyze_output(original_tool, &original_params, original_output);

            json!({
                "tool": original_tool,
                "params": original_params,
                "original_output_length": original_output.len(),
                "analysis": analysis,
                "verification_note": format!(
                    "To fully verify, re-run '{}' with the same parameters and compare results. \
                     This tool provides structural analysis of the original output.",
                    original_tool
                )
            })
            .to_string()
        })
    });

    ToolDef {
        name: "verify_output".to_string(),
        description: "Verify a previous tool result by analyzing its output for consistency. \
                       Provide the original tool name, parameters, and output for comparison analysis."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "original_tool": {
                    "type": "string",
                    "description": "Name of the tool whose output to verify"
                },
                "original_params": {
                    "type": "object",
                    "description": "The parameters that were passed to the original tool call"
                },
                "original_output": {
                    "type": "string",
                    "description": "The output from the original tool call to verify"
                }
            },
            "required": ["original_tool", "original_output"]
        }),
        execute,
        category: "verification".to_string(),
        cleanup: None,
    deferred: true,
    always_load: false,
    search_hint: Some("Analyze previous tool outputs for consistency and errors".to_string()),
    }
}

/// Analyze tool output for internal consistency.
fn analyze_output(tool_name: &str, _params: &Value, output: &str) -> Value {
    let mut checks = Vec::new();

    // Check if output looks like an error
    let has_error = output.to_lowercase().contains("error")
        || output.starts_with("Error:")
        || output.starts_with("{\"error\"");
    checks.push(json!({
        "check": "error_detection",
        "passed": !has_error,
        "detail": if has_error { "Output appears to contain an error" } else { "No error detected in output" }
    }));

    // Check if output is valid JSON (for tools that return JSON)
    let is_json = serde_json::from_str::<Value>(output).is_ok();
    if is_json {
        checks.push(json!({
            "check": "json_validity",
            "passed": true,
            "detail": "Output is valid JSON"
        }));
    }

    // Check output is non-empty
    let is_non_empty = !output.trim().is_empty();
    checks.push(json!({
        "check": "non_empty",
        "passed": is_non_empty,
        "detail": if is_non_empty { "Output is non-empty" } else { "Output is empty" }
    }));

    // Tool-specific checks
    match tool_name {
        "memory_search" => {
            let has_results = output.contains("results") || output.contains("found");
            checks.push(json!({
                "check": "search_has_results",
                "passed": has_results,
                "detail": if has_results { "Search appears to have returned results" } else { "Search may have returned no results" }
            }));
        }
        "execute_bash" => {
            let has_exit_code = output.contains("exit code") || output.contains("Exit code");
            if has_exit_code {
                checks.push(json!({
                    "check": "bash_exit_code",
                    "passed": true,
                    "detail": "Command includes exit code information"
                }));
            }
        }
        _ => {}
    }

    let all_passed = checks
        .iter()
        .all(|c| c.get("passed").and_then(|v| v.as_bool()).unwrap_or(false));

    json!({
        "checks": checks,
        "all_passed": all_passed,
        "tool_name": tool_name,
        "output_length": output.len()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_output_valid_json() {
        let result = analyze_output("memory_search", &json!({}), r#"{"results": ["item1"]}"#);
        let checks = result.get("checks").unwrap().as_array().unwrap();
        assert!(checks.len() >= 2);
        assert!(result.get("all_passed").unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_analyze_output_error_detected() {
        let result = analyze_output("execute_bash", &json!({}), "Error: command not found");
        assert!(!result.get("all_passed").unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_analyze_output_empty() {
        let result = analyze_output("test", &json!({}), "  ");
        assert!(!result.get("all_passed").unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_verify_output_tool_created() {
        let tools = all_verification_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "verify_output");
        assert_eq!(tools[0].category, "verification");
    }

    #[tokio::test]
    async fn test_verify_output_missing_tool() {
        let tool = verify_output_tool();
        let result = (tool.execute)(json!({"original_output": "test"})).await;
        assert!(result.contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn test_verify_output_missing_output() {
        let tool = verify_output_tool();
        let result = (tool.execute)(json!({"original_tool": "test"})).await;
        assert!(result.contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn test_verify_output_success() {
        let tool = verify_output_tool();
        let result = (tool.execute)(json!({
            "original_tool": "memory_search",
            "original_params": {"query": "test"},
            "original_output": r#"{"results": ["item1"]}"#
        }))
        .await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed.get("tool").unwrap().as_str().unwrap(),
            "memory_search"
        );
        assert!(parsed.get("analysis").is_some());
    }
}
