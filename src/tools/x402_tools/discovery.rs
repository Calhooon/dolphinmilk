//! x402 service discovery — browse registry and inspect manifests.
//!
//! `discover_services` lists available agents, `discover_endpoints` fetches
//! manifest details for a specific service. Provider tips are auto-appended.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::x402::{discovery, registry};

// ---------------------------------------------------------------------------
// discover_services
// ---------------------------------------------------------------------------

pub(crate) async fn discover_services_impl(
    params: Value,
    _wallet_url: String,
    registry_url: String,
) -> String {
    let category_filter = params
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    match registry::list_agents_from(&registry_url, None).await {
        Ok(agents) => {
            let filtered: Vec<Value> = agents
                .iter()
                .filter(|a| {
                    if category_filter.is_empty() {
                        return true;
                    }
                    a.name.to_lowercase().contains(&category_filter)
                        || a.tagline.to_lowercase().contains(&category_filter)
                        || a.capabilities
                            .iter()
                            .any(|c| c.to_lowercase().contains(&category_filter))
                        || a.extra
                            .get("category")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_lowercase().contains(&category_filter))
                            .unwrap_or(false)
                        || a.extra
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_lowercase().contains(&category_filter))
                            .unwrap_or(false)
                })
                .map(|a| {
                    let category = a
                        .extra
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let description = a
                        .extra
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    json!({
                        "name": a.name,
                        "display_name": a.display_name,
                        "url": a.url,
                        "tagline": a.tagline,
                        "capabilities": a.capabilities,
                        "category": category,
                        "description": description,
                    })
                })
                .collect();

            serde_json::to_string_pretty(&filtered)
                .unwrap_or_else(|_| "Error: serialization failed".to_string())
        }
        Err(e) => format!("Error: {e}"),
    }
}

// ---------------------------------------------------------------------------
// discover_endpoints
// ---------------------------------------------------------------------------

pub(crate) async fn discover_endpoints_impl(
    params: Value,
    _wallet_url: String,
    failures: Option<Arc<Mutex<HashMap<String, u32>>>>,
) -> String {
    let agent = params.get("agent").and_then(|v| v.as_str()).unwrap_or("");

    if agent.is_empty() {
        return "Error: agent is required (name like 'banana' or full URL)".to_string();
    }

    let manifest_url = match registry::resolve_x402_info(agent, None).await {
        Ok(url) => url,
        Err(e) => return format!("Error resolving agent: {e}"),
    };

    let summary = match discovery::fetch_manifest_from_url(&manifest_url).await {
        Ok(manifest) => discovery::format_manifest_summary(&manifest),
        Err(e) => return format!("Error fetching manifest: {e}"),
    };

    // Append provider tips if available
    let provider_name = match agent.find('/') {
        Some(idx) => &agent[..idx],
        None => agent,
    }
    .to_lowercase();

    // Reset circuit breaker for this provider on successful discovery.
    if let Some(ref failures) = failures {
        let mut counts = failures.lock().unwrap();
        let prefix_slash = format!("{}/", provider_name);
        let prefix_colon = format!("{}:", provider_name);
        counts.retain(|key, _| {
            let lower = key.to_lowercase();
            !lower.starts_with(&prefix_slash) && !lower.starts_with(&prefix_colon)
        });
    }

    let provider_tips = load_provider_tips(&provider_name);

    if let Some(tips) = provider_tips {
        format!("{summary}\n---\nProvider Tips ({provider_name}):\n{tips}")
    } else {
        summary
    }
}

// ---------------------------------------------------------------------------
// Provider tips helpers
// ---------------------------------------------------------------------------

/// Load provider tips from `skills/x402/providers/{name}.md`.
///
/// Checks project-root `skills/` first, then workspace `working/skills/`.
/// Skips files starting with `_` (templates, etc.).
pub(crate) fn load_provider_tips(name: &str) -> Option<String> {
    if name.starts_with('_') {
        return None;
    }

    let candidates = [
        std::path::PathBuf::from(format!("skills/x402/providers/{name}.md")),
        std::path::PathBuf::from(format!("working/skills/x402/providers/{name}.md")),
    ];

    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            if !content.trim().is_empty() {
                return Some(content);
            }
        }
    }

    None
}

/// Extract the `## Key Constraints` section from provider tips.
pub(crate) fn load_key_constraints(name: &str) -> Option<String> {
    let tips = load_provider_tips(name)?;
    extract_section(&tips, "Key Constraints")
}

/// Extract a named `## Section` from markdown text.
pub(crate) fn extract_section(text: &str, section_name: &str) -> Option<String> {
    let header = format!("## {section_name}");
    let start = text.find(&header)?;
    let content_start = start + header.len();
    let rest = &text[content_start..];
    let end = rest
        .find("\n## ")
        .map(|i| content_start + i)
        .unwrap_or(text.len());
    let section = text[content_start..end].trim();
    if section.is_empty() {
        None
    } else {
        Some(section.to_string())
    }
}

/// A single parameter validation rule parsed from provider tips.
#[derive(Debug)]
pub(crate) struct ValidationRule {
    pub field: String,
    pub operator: String,
    pub value: String,
    pub message: String,
}

/// Parse `## Validation Rules` section from provider tips.
pub(crate) fn load_validation_rules(name: &str) -> Vec<ValidationRule> {
    let tips = match load_provider_tips(name) {
        Some(t) => t,
        None => return vec![],
    };

    let section = match extract_section(&tips, "Validation Rules") {
        Some(s) => s,
        None => return vec![],
    };

    let mut rules = Vec::new();
    for line in section.lines() {
        let line = line.trim().trim_start_matches('-').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() != 2 {
            continue;
        }
        let condition = parts[0].trim();
        let message = parts[1].trim().to_string();

        let tokens: Vec<&str> = condition.split_whitespace().collect();
        if tokens.len() == 2 && tokens[1] == "required" {
            rules.push(ValidationRule {
                field: tokens[0].to_string(),
                operator: "required".to_string(),
                value: String::new(),
                message,
            });
        } else if tokens.len() >= 3 {
            rules.push(ValidationRule {
                field: tokens[0].to_string(),
                operator: tokens[1].to_string(),
                value: tokens[2..].join(" "),
                message,
            });
        }
    }
    rules
}

/// Validate request parameters against provider-specific rules.
pub(crate) fn validate_parameters(rules: &[ValidationRule], params: &Value) -> Option<String> {
    let obj = params.as_object()?;

    for rule in rules {
        match rule.operator.as_str() {
            "required" if !obj.contains_key(&rule.field) => {
                return Some(rule.message.clone());
            }
            ">=" => {
                if let Some(val) = obj.get(&rule.field).and_then(|v| v.as_f64()) {
                    if let Ok(threshold) = rule.value.parse::<f64>() {
                        if val < threshold {
                            return Some(rule.message.clone());
                        }
                    }
                }
            }
            ">" => {
                if let Some(val) = obj.get(&rule.field).and_then(|v| v.as_f64()) {
                    if let Ok(threshold) = rule.value.parse::<f64>() {
                        if val <= threshold {
                            return Some(rule.message.clone());
                        }
                    }
                }
            }
            "<=" => {
                if let Some(val) = obj.get(&rule.field).and_then(|v| v.as_f64()) {
                    if let Ok(threshold) = rule.value.parse::<f64>() {
                        if val > threshold {
                            return Some(rule.message.clone());
                        }
                    }
                }
            }
            "<" => {
                if let Some(val) = obj.get(&rule.field).and_then(|v| v.as_f64()) {
                    if let Ok(threshold) = rule.value.parse::<f64>() {
                        if val >= threshold {
                            return Some(rule.message.clone());
                        }
                    }
                }
            }
            "in" => {
                if let Some(val) = obj.get(&rule.field).and_then(|v| v.as_str()) {
                    let allowed: Vec<&str> = rule
                        .value
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .split(',')
                        .map(|s| s.trim())
                        .collect();
                    if !allowed.contains(&val) {
                        return Some(rule.message.clone());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Scan `skills/x402/providers/` for `.md` files to get known provider names.
fn known_provider_names() -> Vec<String> {
    let mut names = Vec::new();
    let dirs = [
        std::path::Path::new("skills/x402/providers"),
        std::path::Path::new("working/skills/x402/providers"),
    ];

    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                if name.ends_with(".md") && !name.starts_with('_') {
                    let provider = name.trim_end_matches(".md").to_string();
                    if !names.contains(&provider) {
                        names.push(provider);
                    }
                }
            }
        }
    }
    names
}

/// Extract provider name from a service identifier.
pub(crate) fn extract_provider_name(service: &str) -> String {
    if service.starts_with("http://") || service.starts_with("https://") {
        let without_scheme = service.split("://").nth(1).unwrap_or(service);
        let host = without_scheme.split('/').next().unwrap_or(without_scheme);
        let known = known_provider_names();
        for name in &known {
            if host.contains(name) {
                return name.to_string();
            }
        }
        host.to_string()
    } else {
        match service.find('/') {
            Some(idx) => service[..idx].to_lowercase(),
            None => service.to_lowercase(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discover_endpoints_validates_agent() {
        let result = discover_endpoints_impl(json!({}), "http://localhost:3322".into(), None).await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("agent"));
    }

    #[tokio::test]
    async fn test_discover_endpoints_validates_empty_agent() {
        let result =
            discover_endpoints_impl(json!({"agent": ""}), "http://localhost:3322".into(), None)
                .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("agent"));
    }

    #[test]
    fn test_discover_endpoints_resets_circuit_breaker() {
        let failures: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut counts = failures.lock().unwrap();
            counts.insert("polymirror:POST".to_string(), 3);
            counts.insert("polymirror/leaderboard:GET".to_string(), 1);
            counts.insert("polymirror/leaderboard:POST".to_string(), 2);
            counts.insert("banana/generate:POST".to_string(), 2);
        }

        {
            let provider_name = "polymirror";
            let prefix_slash = format!("{}/", provider_name);
            let prefix_colon = format!("{}:", provider_name);
            let mut counts = failures.lock().unwrap();
            counts.retain(|key, _| {
                let lower = key.to_lowercase();
                !lower.starts_with(&prefix_slash) && !lower.starts_with(&prefix_colon)
            });
        }

        let counts = failures.lock().unwrap();
        assert!(!counts.contains_key("polymirror:POST"));
        assert!(!counts.contains_key("polymirror/leaderboard:GET"));
        assert!(!counts.contains_key("polymirror/leaderboard:POST"));
        assert_eq!(counts.get("banana/generate:POST"), Some(&2));
    }

    #[test]
    fn test_load_provider_tips_banana() {
        let tips = load_provider_tips("banana");
        assert!(tips.is_some(), "banana.md provider tips should exist");
        let content = tips.unwrap();
        assert!(
            content.contains("Image Generation"),
            "banana tips should mention image generation"
        );
        assert!(
            content.contains("prediction"),
            "banana tips should mention prediction ID"
        );
    }

    #[test]
    fn test_load_provider_tips_nonexistent() {
        let tips = load_provider_tips("nonexistent-service-xyz");
        assert!(tips.is_none(), "should return None for unknown provider");
    }

    #[test]
    fn test_load_provider_tips_case_handling() {
        let tips = load_provider_tips("BANANA");
        let _ = tips;
    }

    #[test]
    fn test_load_provider_tips_all_known_providers() {
        let providers = [
            "banana",
            "nanostore",
            "x-research",
            "whisper",
            "veo",
            "kling",
            "1sat",
            "openai",
        ];
        for name in &providers {
            let tips = load_provider_tips(name);
            assert!(tips.is_some(), "provider tips should exist for '{name}'");
            assert!(
                !tips.as_ref().unwrap().trim().is_empty(),
                "provider tips for '{name}' should not be empty"
            );
        }
    }

    #[test]
    fn test_load_provider_tips_skips_underscore_prefix() {
        let tips = load_provider_tips("_TEMPLATE");
        assert!(tips.is_none(), "_TEMPLATE should be skipped");
    }

    #[test]
    fn test_extract_provider_name_shorthand() {
        assert_eq!(extract_provider_name("nanostore/upload"), "nanostore");
        assert_eq!(extract_provider_name("banana/generate"), "banana");
        assert_eq!(extract_provider_name("x-research/search"), "x-research");
    }

    #[test]
    fn test_extract_provider_name_bare() {
        assert_eq!(extract_provider_name("nanostore"), "nanostore");
        assert_eq!(extract_provider_name("BANANA"), "banana");
    }

    #[test]
    fn test_extract_provider_name_url() {
        assert_eq!(
            extract_provider_name("https://nanostore.babbage.systems/upload"),
            "nanostore"
        );
        assert_eq!(
            extract_provider_name("https://nano-banana-pro.x402agency.com/generate"),
            "banana"
        );
    }

    #[test]
    fn test_extract_provider_name_unknown_url() {
        assert_eq!(
            extract_provider_name("https://example.com/api"),
            "example.com"
        );
    }

    #[test]
    fn test_extract_provider_name_dynamic() {
        let names = known_provider_names();
        assert!(names.contains(&"banana".to_string()), "should find banana");
        assert!(
            names.contains(&"nanostore".to_string()),
            "should find nanostore"
        );
        assert!(
            !names.iter().any(|n| n.starts_with('_')),
            "should skip _-prefixed files"
        );
    }

    #[test]
    fn test_load_key_constraints_with_section() {
        let constraints = load_key_constraints("nanostore");
        assert!(
            constraints.is_some(),
            "nanostore should have key constraints"
        );
        let text = constraints.unwrap();
        assert!(
            text.contains("retentionPeriod"),
            "should mention retentionPeriod"
        );
        assert!(text.contains("180"), "should mention minimum 180");
        assert!(text.contains("fileSize"), "should mention fileSize");
    }

    #[test]
    fn test_load_key_constraints_nonexistent() {
        let constraints = load_key_constraints("nonexistent-provider-xyz");
        assert!(
            constraints.is_none(),
            "nonexistent provider should return None"
        );
    }

    #[test]
    fn test_load_key_constraints_all_providers_have_section() {
        let providers = [
            "banana",
            "nanostore",
            "x-research",
            "whisper",
            "veo",
            "kling",
            "1sat",
            "openai",
        ];
        for name in &providers {
            let constraints = load_key_constraints(name);
            assert!(
                constraints.is_some(),
                "provider '{name}' should have key constraints"
            );
            assert!(
                !constraints.as_ref().unwrap().is_empty(),
                "key constraints for '{name}' should not be empty"
            );
        }
    }

    #[test]
    fn test_extract_section() {
        let text = "# Title\n\n## Key Constraints\n- rule 1\n- rule 2\n\n## Next Section\nstuff";
        let section = extract_section(text, "Key Constraints");
        assert!(section.is_some());
        let content = section.unwrap();
        assert!(content.contains("rule 1"));
        assert!(content.contains("rule 2"));
        assert!(!content.contains("Next Section"));
    }

    #[test]
    fn test_extract_section_at_end() {
        let text = "# Title\n\n## Validation Rules\n- field >= 10 | msg";
        let section = extract_section(text, "Validation Rules");
        assert!(section.is_some());
        assert!(section.unwrap().contains("field >= 10"));
    }

    #[test]
    fn test_load_validation_rules_nanostore() {
        let rules = load_validation_rules("nanostore");
        assert!(!rules.is_empty(), "nanostore should have validation rules");

        let retention_rule = rules
            .iter()
            .find(|r| r.field == "retentionPeriod" && r.operator == ">=");
        assert!(
            retention_rule.is_some(),
            "should have retentionPeriod >= rule"
        );
        assert_eq!(retention_rule.unwrap().value, "180");

        let size_rule = rules
            .iter()
            .find(|r| r.field == "fileSize" && r.operator == "required");
        assert!(size_rule.is_some(), "should have fileSize required rule");
    }

    #[test]
    fn test_validate_parameters_gte() {
        let rules = vec![ValidationRule {
            field: "retentionPeriod".to_string(),
            operator: ">=".to_string(),
            value: "180".to_string(),
            message: "retentionPeriod must be >= 180".to_string(),
        }];

        let params = json!({"retentionPeriod": 60, "fileSize": 100});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_some(), "should fail validation");
        assert!(result.unwrap().contains("180"));

        let params = json!({"retentionPeriod": 180, "fileSize": 100});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_none(), "should pass validation");

        let params = json!({"retentionPeriod": 525600, "fileSize": 100});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_none(), "should pass validation");
    }

    #[test]
    fn test_validate_parameters_in() {
        let rules = vec![ValidationRule {
            field: "resolution".to_string(),
            operator: "in".to_string(),
            value: "[1K,2K,4K]".to_string(),
            message: "resolution must be 1K, 2K, or 4K".to_string(),
        }];

        let params = json!({"resolution": "1024x1024"});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_some(), "should fail for invalid resolution");

        let params = json!({"resolution": "1K"});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_none(), "should pass for valid resolution");
    }

    #[test]
    fn test_validate_parameters_required() {
        let rules = vec![ValidationRule {
            field: "prompt".to_string(),
            operator: "required".to_string(),
            value: String::new(),
            message: "prompt is required".to_string(),
        }];

        let params = json!({"resolution": "1K"});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_some(), "should fail when prompt is missing");

        let params = json!({"prompt": "a cat", "resolution": "1K"});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_none(), "should pass when prompt is present");
    }

    #[test]
    fn test_validate_parameters_passes_empty_rules() {
        let rules: Vec<ValidationRule> = vec![];
        let params = json!({"anything": "goes"});
        let result = validate_parameters(&rules, &params);
        assert!(result.is_none(), "empty rules should always pass");
    }

    #[test]
    fn test_validate_parameters_lte() {
        let rules = vec![ValidationRule {
            field: "pages".to_string(),
            operator: "<=".to_string(),
            value: "5".to_string(),
            message: "pages must be at most 5".to_string(),
        }];

        let params = json!({"pages": 10});
        assert!(
            validate_parameters(&rules, &params).is_some(),
            "10 > 5 should fail"
        );

        let params = json!({"pages": 5});
        assert!(
            validate_parameters(&rules, &params).is_none(),
            "5 <= 5 should pass"
        );

        let params = json!({"pages": 3});
        assert!(
            validate_parameters(&rules, &params).is_none(),
            "3 <= 5 should pass"
        );
    }
}
