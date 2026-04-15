//! Integration tests for skill hooks (#84) and composition (#106).

use std::io::Write;
use tempfile::TempDir;

use dolphin_milk::skills::composition::{
    detect_cycles, validate_required_deps, DependencyError, SkillDependencies,
};
use dolphin_milk::skills::hooks::HookResult;
use dolphin_milk::skills::loader::parse_skill_content;
use dolphin_milk::skills::{
    PreToolUseHook, Skill, SkillDependencies as SkillDeps, SkillRegistry, SkillState,
};

// ============================================================
// Helper
// ============================================================

fn make_skill(name: &str) -> Skill {
    Skill {
        name: name.to_string(),
        description: String::new(),
        auto_activate: false,
        tools: vec![],
        instructions: String::new(),
        source: "test".to_string(),
        state: SkillState::default(),
        dependencies: SkillDeps::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    }
}

fn make_skill_with_deps(name: &str, requires: Vec<&str>, optional: Vec<&str>) -> Skill {
    Skill {
        name: name.to_string(),
        description: String::new(),
        auto_activate: false,
        tools: vec![],
        instructions: String::new(),
        source: "test".to_string(),
        state: SkillState::default(),
        dependencies: SkillDeps {
            requires: requires.into_iter().map(|s| s.to_string()).collect(),
            optional: optional.into_iter().map(|s| s.to_string()).collect(),
        },
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    }
}

// ============================================================
// Hooks — Registration and firing order (#84)
// ============================================================

#[test]
fn test_registry_hook_registration_and_firing_order() {
    let mut registry = SkillRegistry::new();

    // Register hooks with different priorities
    let fired_order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    // High priority (fires last)
    let order_clone = fired_order.clone();
    registry.register_hook(
        PreToolUseHook {
            skill_name: "high".to_string(),
            tool_pattern: "*".to_string(),
            priority: 10,
        },
        Box::new(move |_, _| {
            order_clone.lock().unwrap().push("high");
            HookResult::Allow
        }),
    );

    // Low priority (fires first)
    let order_clone = fired_order.clone();
    registry.register_hook(
        PreToolUseHook {
            skill_name: "low".to_string(),
            tool_pattern: "*".to_string(),
            priority: -5,
        },
        Box::new(move |_, _| {
            order_clone.lock().unwrap().push("low");
            HookResult::Allow
        }),
    );

    // Medium priority
    let order_clone = fired_order.clone();
    registry.register_hook(
        PreToolUseHook {
            skill_name: "mid".to_string(),
            tool_pattern: "*".to_string(),
            priority: 0,
        },
        Box::new(move |_, _| {
            order_clone.lock().unwrap().push("mid");
            HookResult::Allow
        }),
    );

    // Evaluate
    let result = registry.evaluate_hooks("any_tool", &serde_json::json!({}));
    assert!(matches!(result, HookResult::Allow));

    let order = fired_order.lock().unwrap();
    assert_eq!(&*order, &["low", "mid", "high"]);
}

#[test]
fn test_registry_deny_hook_blocks_execution() {
    let mut registry = SkillRegistry::new();

    registry.register_hook(
        PreToolUseHook {
            skill_name: "security".to_string(),
            tool_pattern: "execute_bash".to_string(),
            priority: 0,
        },
        Box::new(|_, _| HookResult::Deny("bash disabled".to_string())),
    );

    let result = registry.evaluate_hooks("execute_bash", &serde_json::json!({"command": "ls"}));
    match result {
        HookResult::Deny(reason) => assert_eq!(reason, "bash disabled"),
        _ => panic!("Expected Deny"),
    }

    // Non-matching tool should pass
    let result = registry.evaluate_hooks("file_read", &serde_json::json!({}));
    assert!(matches!(result, HookResult::Allow));
}

#[test]
fn test_registry_modify_hook_changes_input() {
    let mut registry = SkillRegistry::new();

    registry.register_hook(
        PreToolUseHook {
            skill_name: "fee-injector".to_string(),
            tool_pattern: "wallet_*".to_string(),
            priority: 0,
        },
        Box::new(|_, input| {
            let mut modified = input.clone();
            if let Some(obj) = modified.as_object_mut() {
                obj.insert("fee_injected".to_string(), serde_json::json!(true));
            }
            HookResult::Modify(modified)
        }),
    );

    let input = serde_json::json!({"method": "createAction"});
    let result = registry.evaluate_hooks("wallet_call", &input);
    match result {
        HookResult::Modify(new_input) => {
            assert_eq!(new_input["method"], "createAction");
            assert_eq!(new_input["fee_injected"], true);
        }
        _ => panic!("Expected Modify"),
    }
}

#[test]
fn test_registry_remove_hooks_for_skill() {
    let mut registry = SkillRegistry::new();

    registry.register_hook(
        PreToolUseHook {
            skill_name: "keep".to_string(),
            tool_pattern: "*".to_string(),
            priority: 0,
        },
        Box::new(|_, _| HookResult::Allow),
    );
    registry.register_hook(
        PreToolUseHook {
            skill_name: "remove-me".to_string(),
            tool_pattern: "*".to_string(),
            priority: 0,
        },
        Box::new(|_, _| HookResult::Deny("should be gone".to_string())),
    );

    assert_eq!(registry.hooks().len(), 2);
    registry.remove_hooks_for_skill("remove-me");
    assert_eq!(registry.hooks().len(), 1);

    // Removed hook should no longer fire
    let result = registry.evaluate_hooks("any_tool", &serde_json::json!({}));
    assert!(matches!(result, HookResult::Allow));
}

#[test]
fn test_deny_stops_further_hooks() {
    let mut registry = SkillRegistry::new();
    let was_called = std::sync::Arc::new(std::sync::Mutex::new(false));

    // Priority 0: deny
    registry.register_hook(
        PreToolUseHook {
            skill_name: "blocker".to_string(),
            tool_pattern: "*".to_string(),
            priority: 0,
        },
        Box::new(|_, _| HookResult::Deny("blocked".to_string())),
    );

    // Priority 1: should never fire
    let called = was_called.clone();
    registry.register_hook(
        PreToolUseHook {
            skill_name: "after-blocker".to_string(),
            tool_pattern: "*".to_string(),
            priority: 1,
        },
        Box::new(move |_, _| {
            *called.lock().unwrap() = true;
            HookResult::Allow
        }),
    );

    let result = registry.evaluate_hooks("any_tool", &serde_json::json!({}));
    assert!(matches!(result, HookResult::Deny(_)));
    assert!(
        !*was_called.lock().unwrap(),
        "Hook after Deny should not fire"
    );
}

// ============================================================
// Composition — Dependency resolution (#106)
// ============================================================

#[test]
fn test_parse_skill_with_dependencies() {
    let content = r#"---
name: advanced-research
description: Research with dependencies
requires: [web-search, memory]
optional: [summarization]
---
# Advanced Research
Uses web search and memory skills.
"#;
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert_eq!(skill.name, "advanced-research");
    assert_eq!(skill.dependencies.requires, vec!["web-search", "memory"]);
    assert_eq!(skill.dependencies.optional, vec!["summarization"]);
}

#[test]
fn test_parse_skill_without_dependencies() {
    let content = "---\nname: standalone\n---\nNo deps.";
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert!(skill.dependencies.requires.is_empty());
    assert!(skill.dependencies.optional.is_empty());
    assert!(!skill.dependencies.has_dependencies());
}

#[test]
fn test_dependency_resolution_happy_path() {
    let deps = SkillDependencies {
        requires: vec!["web-search".to_string(), "memory".to_string()],
        optional: vec!["summarization".to_string()],
    };
    let available: std::collections::HashSet<String> = ["web-search", "memory", "summarization"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let errors = validate_required_deps("research", &deps, &available);
    assert!(errors.is_empty());
}

#[test]
fn test_missing_required_dependency_error() {
    let deps = SkillDependencies {
        requires: vec!["web-search".to_string(), "nonexistent".to_string()],
        optional: vec![],
    };
    let available: std::collections::HashSet<String> =
        ["web-search"].iter().map(|s| s.to_string()).collect();

    let errors = validate_required_deps("research", &deps, &available);
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        DependencyError::MissingRequired { skill, missing } => {
            assert_eq!(skill, "research");
            assert_eq!(missing, "nonexistent");
        }
        _ => panic!("Expected MissingRequired"),
    }
}

#[test]
fn test_optional_dependency_missing_is_ok() {
    let deps = SkillDependencies {
        requires: vec![],
        optional: vec!["not-installed".to_string()],
    };
    let available: std::collections::HashSet<String> = std::collections::HashSet::new();

    let errors = validate_required_deps("my-skill", &deps, &available);
    assert!(
        errors.is_empty(),
        "Missing optional deps should not cause errors"
    );
}

#[test]
fn test_cycle_detection_error() {
    let mut graph = std::collections::HashMap::new();
    graph.insert("a".to_string(), vec!["b".to_string()]);
    graph.insert("b".to_string(), vec!["c".to_string()]);
    graph.insert("c".to_string(), vec!["a".to_string()]);

    let err = detect_cycles(&graph).unwrap_err();
    match err {
        DependencyError::Cycle { path } => {
            // Path should contain the cycle
            assert!(path.len() >= 3, "Cycle path too short: {:?}", path);
        }
        _ => panic!("Expected Cycle"),
    }
}

#[test]
fn test_no_cycle_diamond_shape() {
    let mut graph = std::collections::HashMap::new();
    graph.insert("a".to_string(), vec!["b".to_string(), "c".to_string()]);
    graph.insert("b".to_string(), vec!["d".to_string()]);
    graph.insert("c".to_string(), vec!["d".to_string()]);
    graph.insert("d".to_string(), vec![]);

    assert!(detect_cycles(&graph).is_ok());
}

#[test]
fn test_activation_with_deps_auto_activates_required() {
    let mut registry = SkillRegistry::new();

    // Register skills with dependency chain: research -> web-search -> memory
    registry.register(make_skill_with_deps("research", vec!["web-search"], vec![]));
    registry.register(make_skill_with_deps("web-search", vec!["memory"], vec![]));
    registry.register(make_skill("memory"));

    let order = registry.activate_with_deps("research");

    // Should be in dependency-first order
    assert_eq!(order, vec!["memory", "web-search", "research"]);

    // All should have telemetry recorded
    let tel = registry.telemetry();
    assert!(tel.activations.contains_key("memory"));
    assert!(tel.activations.contains_key("web-search"));
    assert!(tel.activations.contains_key("research"));
}

#[test]
fn test_activation_standalone_skill() {
    let mut registry = SkillRegistry::new();
    registry.register(make_skill("standalone"));

    let order = registry.activate_with_deps("standalone");
    assert_eq!(order, vec!["standalone"]);
}

#[test]
fn test_load_skills_with_deps_from_dir() {
    let dir = TempDir::new().unwrap();

    // Create a skill with dependencies
    let skill_dir = dir.path().join("research");
    std::fs::create_dir_all(&skill_dir).unwrap();
    {
        let mut f = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
        writeln!(
            f,
            "---\nname: research\nrequires: [web-search]\noptional: [cache]\n---\nResearch instructions."
        ).unwrap();
    }

    // Create the required dependency
    let dep_dir = dir.path().join("web-search");
    std::fs::create_dir_all(&dep_dir).unwrap();
    {
        let mut f = std::fs::File::create(dep_dir.join("SKILL.md")).unwrap();
        writeln!(f, "---\nname: web-search\n---\nWeb search instructions.").unwrap();
    }

    let registry = SkillRegistry::load_from_dir(dir.path());
    assert_eq!(registry.len(), 2);

    let research = registry.find("research").unwrap();
    assert_eq!(research.dependencies.requires, vec!["web-search"]);
    assert_eq!(research.dependencies.optional, vec!["cache"]);

    let web_search = registry.find("web-search").unwrap();
    assert!(web_search.dependencies.requires.is_empty());
}

#[test]
fn test_validate_dependencies_logs_missing() {
    // This tests that validate_dependencies doesn't panic on missing deps.
    // (It logs warnings but continues.)
    let mut registry = SkillRegistry::new();
    registry.register(make_skill_with_deps(
        "research",
        vec!["nonexistent"],
        vec![],
    ));

    // Should not panic
    registry.validate_dependencies();
}
