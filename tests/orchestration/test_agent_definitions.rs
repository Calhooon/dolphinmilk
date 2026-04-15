//! Tests for agent definition parsing and loading from .md files.

use dolphin_milk::orchestration::agents::{
    agent_available, load_agents_from_dir, parse_agent_content, AgentDefinition,
};
use std::path::PathBuf;

const VALID_AGENT_MD: &str = r#"---
name: researcher
description: Research agent for finding information
model: gpt-5-mini
capabilities:
  - tools
  - memory
max_iterations: 30
budget_fraction: 0.2
allowed_tools:
  - web_fetch
  - memory_store
  - memory_search
---

You are a research agent. Your job is to find and summarize information.
Always cite your sources.
"#;

#[test]
fn parse_valid_agent() {
    let def = parse_agent_content(VALID_AGENT_MD, &PathBuf::from("test.md")).unwrap();
    assert_eq!(def.name, "researcher");
    assert_eq!(def.description, "Research agent for finding information");
    assert_eq!(def.model.as_deref(), Some("gpt-5-mini"));
    assert_eq!(def.capabilities, vec!["tools", "memory"]);
    assert_eq!(def.max_iterations, Some(30));
    assert!((def.budget_fraction - 0.2).abs() < f64::EPSILON);
    assert_eq!(
        def.allowed_tools,
        vec!["web_fetch", "memory_store", "memory_search"]
    );
    assert!(def.instructions.contains("research agent"));
}

#[test]
fn parse_minimal_agent() {
    let md = "---\nname: minimal\n---\nDo things.\n";
    let def = parse_agent_content(md, &PathBuf::from("min.md")).unwrap();
    assert_eq!(def.name, "minimal");
    assert!(def.description.is_empty());
    assert!(def.model.is_none());
    assert!(def.capabilities.is_empty());
    assert!(def.max_iterations.is_none());
    assert!((def.budget_fraction - 0.1).abs() < f64::EPSILON); // default
    assert!(def.allowed_tools.is_empty());
    assert_eq!(def.instructions, "Do things.");
}

#[test]
fn parse_missing_frontmatter() {
    let md = "Just some markdown without frontmatter.";
    let result = parse_agent_content(md, &PathBuf::from("bad.md"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("missing YAML frontmatter"));
}

#[test]
fn parse_missing_closing_delimiter() {
    let md = "---\nname: broken\nNo closing delimiter\n";
    let result = parse_agent_content(md, &PathBuf::from("broken.md"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("missing closing ---"));
}

#[test]
fn parse_empty_name() {
    let md = "---\nname: \"\"\n---\nBody\n";
    let result = parse_agent_content(md, &PathBuf::from("empty.md"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty name"));
}

#[test]
fn parse_invalid_budget_fraction_too_high() {
    let md = "---\nname: expensive\nbudget_fraction: 1.5\n---\nBody\n";
    let result = parse_agent_content(md, &PathBuf::from("expensive.md"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("out of range"));
}

#[test]
fn parse_invalid_budget_fraction_negative() {
    let md = "---\nname: cheap\nbudget_fraction: -0.1\n---\nBody\n";
    let result = parse_agent_content(md, &PathBuf::from("cheap.md"));
    assert!(result.is_err());
}

#[test]
fn parse_invalid_yaml() {
    let md = "---\nname: [invalid yaml\n---\nBody\n";
    let result = parse_agent_content(md, &PathBuf::from("invalid.md"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid YAML"));
}

#[test]
fn load_from_nonexistent_dir() {
    let agents = load_agents_from_dir(&PathBuf::from("/nonexistent/agents"));
    assert!(agents.is_empty());
}

#[test]
fn load_from_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let agents = load_agents_from_dir(dir.path());
    assert!(agents.is_empty());
}

#[test]
fn load_from_dir_with_agent() {
    let dir = tempfile::tempdir().unwrap();
    let agent_path = dir.path().join("researcher.md");
    std::fs::write(&agent_path, VALID_AGENT_MD).unwrap();

    let agents = load_agents_from_dir(dir.path());
    assert_eq!(agents.len(), 1);
    assert!(agents.contains_key("researcher"));
}

#[test]
fn load_skips_non_md_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("readme.txt"), "not an agent").unwrap();
    std::fs::write(dir.path().join("agent.md"), VALID_AGENT_MD).unwrap();

    let agents = load_agents_from_dir(dir.path());
    assert_eq!(agents.len(), 1);
}

#[test]
fn load_skips_invalid_agents() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("good.md"), VALID_AGENT_MD).unwrap();
    std::fs::write(dir.path().join("bad.md"), "no frontmatter").unwrap();

    let agents = load_agents_from_dir(dir.path());
    assert_eq!(agents.len(), 1);
}

#[test]
fn load_later_overrides_earlier() {
    let dir = tempfile::tempdir().unwrap();
    // Both files define "researcher" — the last one loaded wins (HashMap::extend)
    let md1 = "---\nname: researcher\ndescription: First\n---\nFirst\n";
    let md2 = "---\nname: researcher\ndescription: Second\n---\nSecond\n";
    std::fs::write(dir.path().join("a_first.md"), md1).unwrap();
    std::fs::write(dir.path().join("b_second.md"), md2).unwrap();

    let agents = load_agents_from_dir(dir.path());
    assert_eq!(agents.len(), 1);
    // HashMap iteration order isn't guaranteed, but one will overwrite the other
    let agent = agents.get("researcher").unwrap();
    // Either description is valid — the key point is there's only one
    assert!(agent.description == "First" || agent.description == "Second");
}

// ─── agent_available ──────────────────────────────────────────────────────────

#[test]
fn agent_available_no_requirements() {
    let agent = AgentDefinition {
        name: "test".to_string(),
        description: String::new(),
        model: None,
        capabilities: vec![],
        max_iterations: None,
        budget_fraction: 0.1,
        allowed_tools: vec![],
        instructions: String::new(),
        source: PathBuf::new(),
    };
    assert!(agent_available(&agent, &["tools".to_string()]));
}

#[test]
fn agent_available_parent_has_all() {
    let agent = AgentDefinition {
        name: "test".to_string(),
        description: String::new(),
        model: None,
        capabilities: vec!["tools".to_string(), "wallet".to_string()],
        max_iterations: None,
        budget_fraction: 0.1,
        allowed_tools: vec![],
        instructions: String::new(),
        source: PathBuf::new(),
    };
    assert!(agent_available(&agent, &["all".to_string()]));
}

#[test]
fn agent_available_parent_has_subset() {
    let agent = AgentDefinition {
        name: "test".to_string(),
        description: String::new(),
        model: None,
        capabilities: vec!["tools".to_string(), "wallet".to_string()],
        max_iterations: None,
        budget_fraction: 0.1,
        allowed_tools: vec![],
        instructions: String::new(),
        source: PathBuf::new(),
    };
    assert!(agent_available(
        &agent,
        &[
            "tools".to_string(),
            "wallet".to_string(),
            "memory".to_string()
        ]
    ));
}

#[test]
fn agent_unavailable_missing_capability() {
    let agent = AgentDefinition {
        name: "test".to_string(),
        description: String::new(),
        model: None,
        capabilities: vec!["tools".to_string(), "wallet".to_string()],
        max_iterations: None,
        budget_fraction: 0.1,
        allowed_tools: vec![],
        instructions: String::new(),
        source: PathBuf::new(),
    };
    assert!(!agent_available(&agent, &["tools".to_string()]));
}

#[test]
fn agent_unavailable_empty_parent_caps() {
    let agent = AgentDefinition {
        name: "test".to_string(),
        description: String::new(),
        model: None,
        capabilities: vec!["tools".to_string()],
        max_iterations: None,
        budget_fraction: 0.1,
        allowed_tools: vec![],
        instructions: String::new(),
        source: PathBuf::new(),
    };
    assert!(!agent_available(&agent, &[]));
}
