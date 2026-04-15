//! Tests for skills module — SKILL.md parsing, registry, and directory loading.

use std::io::Write;
use tempfile::TempDir;

use dolphin_milk::skills::loader::{load_skills_from_dir, parse_skill_content};
use dolphin_milk::skills::{Skill, SkillDependencies, SkillRegistry, SkillState};

// -- parse_skill_content --

#[test]
fn test_parse_skill_content_valid() {
    let content = r#"---
name: messaging
description: Cross-wallet communication via BRC-33 MessageBox
auto_activate: true
tools: [send_message, check_inbox]
---
# Messaging Skill

When you receive inbox messages, ALWAYS respond using the `send_message` tool.
"#;
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert_eq!(skill.name, "messaging");
    assert_eq!(
        skill.description,
        "Cross-wallet communication via BRC-33 MessageBox"
    );
    assert!(skill.auto_activate);
    assert_eq!(skill.tools, vec!["send_message", "check_inbox"]);
    assert!(skill.instructions.contains("Messaging Skill"));
    assert!(skill.instructions.contains("send_message"));
    assert_eq!(skill.source, "test.md");
}

#[test]
fn test_parse_skill_content_no_frontmatter() {
    let content = "# No frontmatter\nJust body content.";
    let result = parse_skill_content(content, "test.md");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("No YAML frontmatter"), "error was: {err}");
}

#[test]
fn test_parse_skill_content_missing_name() {
    let content = "---\ndescription: no name here\nauto_activate: true\n---\nBody";
    let result = parse_skill_content(content, "test.md");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("Missing 'name'"), "error was: {err}");
}

#[test]
fn test_parse_skill_content_minimal() {
    // Only name is required — description, auto_activate, tools are optional
    let content = "---\nname: minimal\n---\nJust instructions.";
    let skill = parse_skill_content(content, "minimal.md").unwrap();
    assert_eq!(skill.name, "minimal");
    assert_eq!(skill.description, "");
    assert!(!skill.auto_activate);
    assert!(skill.tools.is_empty());
    assert_eq!(skill.instructions, "Just instructions.");
}

#[test]
fn test_parse_skill_content_no_closing_delimiter() {
    let content = "---\nname: broken\nNo closing delimiter here.";
    let result = parse_skill_content(content, "test.md");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("No closing ---"), "error was: {err}");
}

#[test]
fn test_parse_skill_content_empty_body() {
    let content = "---\nname: empty-body\n---\n";
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert_eq!(skill.name, "empty-body");
    assert_eq!(skill.instructions, "");
}

#[test]
fn test_parse_skill_content_multiline_body() {
    let content =
        "---\nname: multi\n---\n# Title\n\nParagraph one.\n\nParagraph two.\n\n- List item";
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert!(skill.instructions.contains("# Title"));
    assert!(skill.instructions.contains("Paragraph one."));
    assert!(skill.instructions.contains("Paragraph two."));
    assert!(skill.instructions.contains("- List item"));
}

// -- split_frontmatter (tested via parse_skill_content and loader internal tests) --

#[test]
fn test_split_frontmatter_preserves_body_newlines() {
    let content = "---\nname: test\n---\nLine 1\nLine 2\nLine 3";
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert_eq!(skill.instructions, "Line 1\nLine 2\nLine 3");
}

// -- SkillRegistry --

#[test]
fn test_skill_registry_new() {
    let registry = SkillRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.all().is_empty());
}

#[test]
fn test_skill_registry_register_and_find() {
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "messaging".to_string(),
        description: "Send messages".to_string(),
        auto_activate: true,
        tools: vec!["send_message".to_string()],
        instructions: "Use send_message.".to_string(),
        source: "test.md".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });
    assert_eq!(registry.len(), 1);
    assert!(!registry.is_empty());

    let found = registry.find("messaging");
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "messaging");
}

#[test]
fn test_skill_registry_find_case_insensitive() {
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "Messaging".to_string(),
        description: "".to_string(),
        auto_activate: false,
        tools: vec![],
        instructions: "".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });

    assert!(registry.find("messaging").is_some());
    assert!(registry.find("MESSAGING").is_some());
    assert!(registry.find("Messaging").is_some());
    assert!(registry.find("nonexistent").is_none());
}

#[test]
fn test_skill_registry_auto_activated() {
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "active".to_string(),
        description: "".to_string(),
        auto_activate: true,
        tools: vec![],
        instructions: "Active skill.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });
    registry.register(Skill {
        name: "inactive".to_string(),
        description: "".to_string(),
        auto_activate: false,
        tools: vec![],
        instructions: "Inactive skill.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });

    let active = registry.auto_activated();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "active");
}

#[test]
fn test_skill_registry_format_for_prompt() {
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "messaging".to_string(),
        description: "Cross-wallet communication".to_string(),
        auto_activate: true,
        tools: vec!["send_message".to_string()],
        instructions: "Always reply with send_message.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });
    registry.register(Skill {
        name: "images".to_string(),
        description: "".to_string(),
        auto_activate: false, // not auto-activated
        tools: vec![],
        instructions: "Generate images.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });

    // inbox_count=1: messaging skill should appear
    let prompt = registry.format_for_prompt(1);
    assert!(prompt.contains("## Active Skills"), "prompt was: {prompt}");
    assert!(prompt.contains("### messaging"), "prompt was: {prompt}");
    assert!(
        prompt.contains("Cross-wallet communication"),
        "prompt was: {prompt}"
    );
    assert!(
        prompt.contains("Always reply with send_message."),
        "prompt was: {prompt}"
    );
    // Inactive skill should NOT appear
    assert!(!prompt.contains("### images"), "prompt was: {prompt}");
    assert!(!prompt.contains("Generate images"), "prompt was: {prompt}");
}

#[test]
fn test_skill_registry_format_empty() {
    let registry = SkillRegistry::new();
    let prompt = registry.format_for_prompt(0);
    assert!(prompt.is_empty());
}

#[test]
fn test_skill_registry_format_no_auto_activated() {
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "inactive".to_string(),
        description: "".to_string(),
        auto_activate: false,
        tools: vec![],
        instructions: "Not shown.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });
    let prompt = registry.format_for_prompt(0);
    assert!(
        prompt.is_empty(),
        "prompt should be empty when no auto-activated skills"
    );
}

// -- load_skills_from_dir --

#[test]
fn test_load_skills_from_dir() {
    let dir = TempDir::new().unwrap();

    // Create a SKILL.md in a subdirectory
    let skill_dir = dir.path().join("messaging");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    let mut f = std::fs::File::create(&skill_path).unwrap();
    writeln!(
        f,
        "---\nname: messaging\ndescription: Send messages\nauto_activate: true\ntools: [send_message]\n---\n# Messaging\nReply with send_message."
    )
    .unwrap();

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "messaging");
    assert!(skills[0].auto_activate);
    assert_eq!(skills[0].tools, vec!["send_message"]);
}

#[test]
fn test_load_skills_from_nonexistent_dir() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("does_not_exist");
    let skills = load_skills_from_dir(&missing).unwrap();
    assert!(skills.is_empty());
}

#[test]
fn test_load_skills_recursive() {
    let dir = TempDir::new().unwrap();

    // Create nested SKILL.md files
    let skill_a = dir.path().join("category-a").join("sub");
    std::fs::create_dir_all(&skill_a).unwrap();
    {
        let mut f = std::fs::File::create(skill_a.join("SKILL.md")).unwrap();
        writeln!(f, "---\nname: alpha\n---\nAlpha instructions.").unwrap();
    }

    let skill_b = dir.path().join("category-b");
    std::fs::create_dir_all(&skill_b).unwrap();
    {
        let mut f = std::fs::File::create(skill_b.join("SKILL.md")).unwrap();
        writeln!(
            f,
            "---\nname: beta\nauto_activate: true\n---\nBeta instructions."
        )
        .unwrap();
    }

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 2);

    // Skills are sorted by name
    assert_eq!(skills[0].name, "alpha");
    assert_eq!(skills[1].name, "beta");
    assert!(skills[1].auto_activate);
}

#[test]
fn test_load_skills_ignores_non_skill_files() {
    let dir = TempDir::new().unwrap();

    // Create a regular markdown file (not SKILL.md)
    std::fs::write(dir.path().join("README.md"), "# Not a skill").unwrap();

    // Create a SKILL.md
    let skill_dir = dir.path().join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    {
        let mut f = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
        writeln!(f, "---\nname: real-skill\n---\nInstructions.").unwrap();
    }

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "real-skill");
}

#[test]
fn test_load_skills_skips_invalid_files() {
    let dir = TempDir::new().unwrap();

    // Create a SKILL.md with broken frontmatter
    let bad_dir = dir.path().join("bad-skill");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(bad_dir.join("SKILL.md"), "No frontmatter here.").unwrap();

    // Create a valid SKILL.md
    let good_dir = dir.path().join("good-skill");
    std::fs::create_dir_all(&good_dir).unwrap();
    {
        let mut f = std::fs::File::create(good_dir.join("SKILL.md")).unwrap();
        writeln!(f, "---\nname: good\n---\nGood instructions.").unwrap();
    }

    let skills = load_skills_from_dir(dir.path()).unwrap();
    // Only the valid one loads; invalid one is silently skipped
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "good");
}

// -- SkillRegistry::load_from_dir --

#[test]
fn test_registry_load_from_dir() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("test-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    {
        let mut f = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
        writeln!(f, "---\nname: test\nauto_activate: true\n---\nTest skill.").unwrap();
    }

    let registry = SkillRegistry::load_from_dir(dir.path());
    assert_eq!(registry.len(), 1);
    assert!(registry.find("test").is_some());
    assert_eq!(registry.auto_activated().len(), 1);
}

// -- Integration: skills in system prompt --

#[test]
fn test_skills_section_in_system_prompt() {
    use dolphin_milk::context::prompt::{build_system_prompt, PromptContext, ToolDesc};

    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "messaging".to_string(),
        description: "BRC-33 communication".to_string(),
        auto_activate: true,
        tools: vec!["send_message".to_string()],
        instructions: "Always use send_message for replies.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });

    let ctx = PromptContext {
        identity_key: String::new(),
        balance_sats: 0,
        model: String::new(),
        tools: vec![ToolDesc {
            name: "send_message".to_string(),
            description: "Send a message".to_string(),
            category: "messagebox".to_string(),
            deferred: false,
            hint: None,
        }],
        memory_summary: String::new(),
        available_files: Vec::new(),
        task: String::new(),
        budget_remaining: 0,
        low_power: false,
        inbox_count: 1,
        skills_section: registry.format_for_prompt(1),
        workspace_path: String::new(),
        certificate_info: None,
        has_external_messages: false,
        identity_soul: None,
        auto_recall_ids: vec![],
        basket_health: std::collections::HashMap::new(),
        spendable_output_count: 0,
        instructions: String::new(),
        env_snapshot: None,
        working_memory_section: String::new(),
    };

    let prompt = build_system_prompt(&ctx);
    assert!(
        prompt.contains("# Skills"),
        "prompt should contain Skills section"
    );
    assert!(
        prompt.contains("messaging"),
        "prompt should contain skill name"
    );
    assert!(
        prompt.contains("Always use send_message"),
        "prompt should contain skill instructions"
    );
}

#[test]
fn test_messaging_skill_excluded_when_no_inbox() {
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "messaging".to_string(),
        description: "BRC-33 communication".to_string(),
        auto_activate: true,
        tools: vec!["send_message".to_string()],
        instructions: "Always use send_message for replies.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });
    registry.register(Skill {
        name: "x402".to_string(),
        description: "x402 services".to_string(),
        auto_activate: true,
        tools: vec!["x402_call".to_string()],
        instructions: "Use x402_call for paid services.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });

    // inbox_count=0: messaging should be excluded, x402 should remain
    let prompt = registry.format_for_prompt(0);
    assert!(
        !prompt.contains("### messaging"),
        "messaging should be excluded when inbox_count=0: {prompt}"
    );
    assert!(
        prompt.contains("### x402"),
        "x402 should still appear: {prompt}"
    );

    // inbox_count=1: both should appear
    let prompt = registry.format_for_prompt(1);
    assert!(
        prompt.contains("### messaging"),
        "messaging should appear when inbox_count=1: {prompt}"
    );
    assert!(
        prompt.contains("### x402"),
        "x402 should still appear: {prompt}"
    );
}

#[test]
fn test_messaging_only_skill_empty_when_no_inbox() {
    // If messaging is the only auto-activated skill and inbox_count=0,
    // the entire prompt should be empty
    let mut registry = SkillRegistry::new();
    registry.register(Skill {
        name: "messaging".to_string(),
        description: "BRC-33 communication".to_string(),
        auto_activate: true,
        tools: vec![],
        instructions: "Reply instructions.".to_string(),
        source: "".to_string(),
        state: SkillState::default(),
        dependencies: SkillDependencies::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    });

    let prompt = registry.format_for_prompt(0);
    assert!(
        prompt.is_empty(),
        "should be empty when messaging is only skill and no inbox: {prompt}"
    );

    let prompt = registry.format_for_prompt(1);
    assert!(
        !prompt.is_empty(),
        "should have content when inbox_count > 0"
    );
}

#[test]
fn test_no_skills_section_when_empty() {
    use dolphin_milk::context::prompt::{build_system_prompt, PromptContext};

    let ctx = PromptContext {
        identity_key: String::new(),
        balance_sats: 0,
        model: String::new(),
        tools: Vec::new(),
        memory_summary: String::new(),
        available_files: Vec::new(),
        task: String::new(),
        budget_remaining: 0,
        low_power: false,
        inbox_count: 0,
        skills_section: String::new(),
        workspace_path: String::new(),
        certificate_info: None,
        has_external_messages: false,
        identity_soul: None,
        auto_recall_ids: vec![],
        basket_health: std::collections::HashMap::new(),
        spendable_output_count: 0,
        instructions: String::new(),
        env_snapshot: None,
        working_memory_section: String::new(),
    };

    let prompt = build_system_prompt(&ctx);
    assert!(
        !prompt.contains("# Skills"),
        "empty skills should not add Skills section"
    );
}

// ─── Wave 5: Expanded frontmatter parsing ──────────────────────────────

#[test]
fn test_parse_expanded_frontmatter() {
    let md = r#"---
name: advanced-skill
description: A skill with all new fields
auto_activate: false
paths:
  - "*.rs"
  - "src/**/*.ts"
context: fork
model: claude-haiku-4-5
arguments:
  - query
  - limit
allowed_tools:
  - web_fetch
  - memory_search
when_to_use: When the user asks about code analysis
version: "1.2.0"
user_invocable: true
argument_hint: "<query> [limit]"
---
# Advanced Skill
Do advanced things with $query (limit $limit).
"#;
    let skill = parse_skill_content(md, "test").unwrap();
    assert_eq!(skill.name, "advanced-skill");
    assert_eq!(skill.paths, vec!["*.rs", "src/**/*.ts"]);
    assert_eq!(skill.context, "fork");
    assert_eq!(skill.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(skill.arguments, vec!["query", "limit"]);
    assert_eq!(skill.allowed_tools, vec!["web_fetch", "memory_search"]);
    assert_eq!(
        skill.when_to_use.as_deref(),
        Some("When the user asks about code analysis")
    );
    assert_eq!(skill.version.as_deref(), Some("1.2.0"));
    assert!(skill.user_invocable);
    assert_eq!(skill.argument_hint.as_deref(), Some("<query> [limit]"));
}

#[test]
fn test_parse_backward_compatible_minimal() {
    // Existing 6-field SKILL.md files still load without the new fields
    let md =
        "---\nname: legacy\ndescription: Old-style skill\nauto_activate: true\n---\nDo stuff.\n";
    let skill = parse_skill_content(md, "test").unwrap();
    assert_eq!(skill.name, "legacy");
    assert!(skill.auto_activate);
    assert!(skill.paths.is_empty());
    assert_eq!(skill.context, "normal");
    assert!(skill.model.is_none());
    assert!(skill.arguments.is_empty());
    assert!(skill.allowed_tools.is_empty());
    assert!(skill.when_to_use.is_none());
    assert!(skill.version.is_none());
    assert!(!skill.user_invocable);
    assert!(skill.argument_hint.is_none());
}

// ─── Wave 5: Argument substitution ─────────────────────────────────────

use dolphin_milk::skills::loader::substitute_arguments;

#[test]
fn test_substitute_arguments_full() {
    let instructions = "Search for $ARGUMENTS";
    let result = substitute_arguments(instructions, "bitcoin fees", &[]);
    assert_eq!(result, "Search for bitcoin fees");
}

#[test]
fn test_substitute_arguments_positional() {
    let instructions = "Query: $0, Limit: $1";
    let result = substitute_arguments(instructions, "rust 10", &[]);
    assert_eq!(result, "Query: rust, Limit: 10");
}

#[test]
fn test_substitute_arguments_indexed() {
    let instructions = "First: $ARGUMENTS[0], Second: $ARGUMENTS[1]";
    let result = substitute_arguments(instructions, "hello world", &[]);
    assert_eq!(result, "First: hello, Second: world");
}

#[test]
fn test_substitute_arguments_named() {
    let instructions = "Search for $query with limit $limit";
    let result = substitute_arguments(
        instructions,
        "bitcoin 5",
        &["query".to_string(), "limit".to_string()],
    );
    assert_eq!(result, "Search for bitcoin with limit 5");
}

#[test]
fn test_substitute_arguments_empty() {
    let instructions = "No args: $ARGUMENTS";
    let result = substitute_arguments(instructions, "", &[]);
    assert_eq!(result, "No args: ");
}

#[test]
fn test_substitute_arguments_missing_named() {
    let instructions = "Query: $query, Missing: $extra";
    let result = substitute_arguments(
        instructions,
        "bitcoin",
        &["query".to_string(), "extra".to_string()],
    );
    assert_eq!(result, "Query: bitcoin, Missing: ");
}
