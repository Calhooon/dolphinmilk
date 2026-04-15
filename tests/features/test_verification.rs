//! Tests for the verification skill and verify_output tool.

use dolphin_milk::skills::loader::parse_skill_content;
use dolphin_milk::tools::verification_tools::all_verification_tools;

// -- Skill file parses correctly --

#[test]
fn test_verification_skill_file_parses() {
    let content = std::fs::read_to_string("skills/verification/SKILL.md")
        .expect("skills/verification/SKILL.md should exist");

    let skill = parse_skill_content(&content, "skills/verification/SKILL.md")
        .expect("Skill should parse successfully");

    assert_eq!(skill.name, "verification");
    assert!(
        !skill.auto_activate,
        "verification should NOT auto-activate"
    );
    assert!(
        skill.tools.contains(&"verify_output".to_string()),
        "Should reference verify_output tool"
    );
    assert!(!skill.description.is_empty(), "Should have a description");
    assert!(!skill.instructions.is_empty(), "Should have instructions");
}

#[test]
fn test_verification_skill_instructions_content() {
    let content = std::fs::read_to_string("skills/verification/SKILL.md").unwrap();
    let skill = parse_skill_content(&content, "skills/verification/SKILL.md").unwrap();

    assert!(
        skill.instructions.contains("Verification"),
        "Should mention verification"
    );
    assert!(
        skill.instructions.contains("Compare"),
        "Should mention comparison"
    );
    assert!(
        skill.instructions.contains("discrepan"),
        "Should mention discrepancies"
    );
}

// -- verify_output tool registration --

#[test]
fn test_verify_output_tool_registered() {
    let tools = all_verification_tools();
    assert_eq!(tools.len(), 1, "Should have exactly one verification tool");
    assert_eq!(tools[0].name, "verify_output");
    assert_eq!(tools[0].category, "verification");
    assert!(tools[0].cleanup.is_none());
}

#[test]
fn test_verify_output_tool_has_parameters() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let params = &tool.parameters;
    assert_eq!(params["type"], "object");

    let properties = params.get("properties").unwrap();
    assert!(properties.get("original_tool").is_some());
    assert!(properties.get("original_params").is_some());
    assert!(properties.get("original_output").is_some());

    let required = params.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&serde_json::json!("original_tool")));
    assert!(required.contains(&serde_json::json!("original_output")));
}

// -- verify_output tool compares results --

#[tokio::test]
async fn test_verify_output_valid_json_output() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_tool": "memory_search",
        "original_params": {"query": "BSV"},
        "original_output": r#"{"results": ["item1", "item2"], "count": 2}"#
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["tool"], "memory_search");

    let analysis = &parsed["analysis"];
    assert!(analysis["all_passed"].as_bool().unwrap());

    let checks = analysis["checks"].as_array().unwrap();
    assert!(!checks.is_empty());

    // Should have json_validity check that passed
    let json_check = checks.iter().find(|c| c["check"] == "json_validity");
    assert!(json_check.is_some());
    assert!(json_check.unwrap()["passed"].as_bool().unwrap());
}

#[tokio::test]
async fn test_verify_output_error_in_output() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_tool": "execute_bash",
        "original_params": {"command": "ls /nonexistent"},
        "original_output": "Error: No such file or directory"
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let analysis = &parsed["analysis"];
    assert!(!analysis["all_passed"].as_bool().unwrap());

    let checks = analysis["checks"].as_array().unwrap();
    let error_check = checks
        .iter()
        .find(|c| c["check"] == "error_detection")
        .unwrap();
    assert!(!error_check["passed"].as_bool().unwrap());
}

#[tokio::test]
async fn test_verify_output_empty_output() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_tool": "test_tool",
        "original_output": "   "
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let analysis = &parsed["analysis"];
    assert!(!analysis["all_passed"].as_bool().unwrap());
}

#[tokio::test]
async fn test_verify_output_missing_tool_param() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_output": "some output"
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed.get("error").is_some());
}

#[tokio::test]
async fn test_verify_output_missing_output_param() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_tool": "test"
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed.get("error").is_some());
}

#[tokio::test]
async fn test_verify_output_memory_search_specific_check() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_tool": "memory_search",
        "original_params": {"query": "BSV"},
        "original_output": r#"{"results": ["entry"], "found": 1}"#
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let checks = parsed["analysis"]["checks"].as_array().unwrap();

    // Should have search_has_results check
    let search_check = checks.iter().find(|c| c["check"] == "search_has_results");
    assert!(search_check.is_some());
    assert!(search_check.unwrap()["passed"].as_bool().unwrap());
}

#[tokio::test]
async fn test_verify_output_includes_verification_note() {
    let tools = all_verification_tools();
    let tool = &tools[0];

    let result = (tool.execute)(serde_json::json!({
        "original_tool": "wallet_balance",
        "original_output": r#"{"balance": 50000}"#
    }))
    .await;

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let note = parsed["verification_note"].as_str().unwrap();
    assert!(note.contains("wallet_balance"));
    assert!(note.contains("re-run"));
}
