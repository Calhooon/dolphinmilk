//! Tests for project-level agent configuration (.dolphin-milk/) detection
//! and integration with commands and skills systems.
//!
//! Covers:
//! - `detect_project()` finding/not-finding `.dolphin-milk/`
//! - `ProjectConfig` reporting available components
//! - Commands loaded from `.dolphin-milk/commands/`
//! - Command priority: user > project > workspace
//! - Skills loaded from `.dolphin-milk/skills/`
//! - Empty and full `.dolphin-milk/` directories

use dolphin_milk::config::project::{detect_project, PROJECT_DIR_NAME};
use dolphin_milk::skills::commands::{load_commands_with_user_dir, CommandSource};
use dolphin_milk::skills::SkillRegistry;
use tempfile::TempDir;

// ── detect_project ───────────────────────────────────────────────────

#[test]
fn test_detect_project_returns_none_when_no_dir() {
    let dir = TempDir::new().unwrap();
    assert!(detect_project(dir.path()).is_none());
}

#[test]
fn test_detect_project_returns_none_for_file_not_dir() {
    let dir = TempDir::new().unwrap();
    // Create .dolphin-milk as a file, not a directory
    std::fs::write(dir.path().join(PROJECT_DIR_NAME), "not a directory").unwrap();
    assert!(detect_project(dir.path()).is_none());
}

#[test]
fn test_detect_project_finds_dolphin_milk_dir() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(PROJECT_DIR_NAME)).unwrap();

    let config = detect_project(dir.path());
    assert!(config.is_some());
    let config = config.unwrap();
    assert!(config.root.ends_with(PROJECT_DIR_NAME));
}

#[test]
fn test_detect_project_empty_dir_reports_no_components() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(PROJECT_DIR_NAME)).unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(!config.has_config);
    assert!(!config.has_instructions);
    assert!(!config.has_commands);
    assert!(!config.has_skills);
    assert_eq!(config.component_count(), 0);
}

// ── ProjectConfig component detection ────────────────────────────────

#[test]
fn test_project_config_detects_config_toml() {
    let dir = TempDir::new().unwrap();
    let dm = dir.path().join(PROJECT_DIR_NAME);
    std::fs::create_dir_all(&dm).unwrap();
    std::fs::write(dm.join("config.toml"), "[budget]\nmax_per_task = 5000").unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(config.has_config);
    assert!(!config.has_instructions);
    assert!(!config.has_commands);
    assert!(!config.has_skills);
    assert_eq!(config.component_count(), 1);
}

#[test]
fn test_project_config_detects_instructions() {
    let dir = TempDir::new().unwrap();
    let dm = dir.path().join(PROJECT_DIR_NAME);
    std::fs::create_dir_all(&dm).unwrap();
    std::fs::write(dm.join("INSTRUCTIONS.md"), "# Project Instructions").unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(!config.has_config);
    assert!(config.has_instructions);
    assert_eq!(config.component_count(), 1);
}

#[test]
fn test_project_config_detects_commands_dir() {
    let dir = TempDir::new().unwrap();
    let dm = dir.path().join(PROJECT_DIR_NAME);
    std::fs::create_dir_all(dm.join("commands")).unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(config.has_commands);
    assert!(!config.has_skills);
    assert_eq!(config.component_count(), 1);
}

#[test]
fn test_project_config_detects_skills_dir() {
    let dir = TempDir::new().unwrap();
    let dm = dir.path().join(PROJECT_DIR_NAME);
    std::fs::create_dir_all(dm.join("skills")).unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(!config.has_commands);
    assert!(config.has_skills);
    assert_eq!(config.component_count(), 1);
}

#[test]
fn test_project_config_full_dir_all_components() {
    let dir = TempDir::new().unwrap();
    let dm = dir.path().join(PROJECT_DIR_NAME);
    std::fs::create_dir_all(dm.join("commands")).unwrap();
    std::fs::create_dir_all(dm.join("skills")).unwrap();
    std::fs::write(dm.join("config.toml"), "").unwrap();
    std::fs::write(dm.join("INSTRUCTIONS.md"), "# Instructions").unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(config.has_config);
    assert!(config.has_instructions);
    assert!(config.has_commands);
    assert!(config.has_skills);
    assert_eq!(config.component_count(), 4);
}

#[test]
fn test_project_config_helper_paths() {
    let dir = TempDir::new().unwrap();
    let dm = dir.path().join(PROJECT_DIR_NAME);
    std::fs::create_dir_all(&dm).unwrap();

    let config = detect_project(dir.path()).unwrap();
    assert!(config.commands_dir().ends_with(".dolphin-milk/commands"));
    assert!(config.skills_dir().ends_with(".dolphin-milk/skills"));
}

// ── Commands from .dolphin-milk/commands/ ────────────────────────────

#[test]
fn test_commands_loaded_from_project_dir() {
    let workspace = TempDir::new().unwrap();
    let project_cmds = workspace.path().join(".dolphin-milk").join("commands");
    std::fs::create_dir_all(&project_cmds).unwrap();
    std::fs::write(
        project_cmds.join("lint.md"),
        "---\nname: lint\ndescription: Run linter\n---\nRun cargo clippy.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), None);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "lint");
    assert_eq!(commands[0].description, "Run linter");
    assert_eq!(commands[0].source, CommandSource::Project);
}

#[test]
fn test_project_commands_override_workspace_commands() {
    let workspace = TempDir::new().unwrap();

    // Workspace-level command
    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(
        ws_cmds.join("review.md"),
        "---\nname: review\ndescription: Workspace review\n---\nWorkspace body.",
    )
    .unwrap();

    // Project-level command with same name (should override)
    let project_cmds = workspace.path().join(".dolphin-milk").join("commands");
    std::fs::create_dir_all(&project_cmds).unwrap();
    std::fs::write(
        project_cmds.join("review.md"),
        "---\nname: review\ndescription: Project review\n---\nProject body.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), None);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "review");
    assert_eq!(commands[0].description, "Project review");
    assert_eq!(commands[0].source, CommandSource::Project);
}

#[test]
fn test_user_commands_override_project_commands() {
    let workspace = TempDir::new().unwrap();
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();

    // Project-level command
    let project_cmds = workspace.path().join(".dolphin-milk").join("commands");
    std::fs::create_dir_all(&project_cmds).unwrap();
    std::fs::write(
        project_cmds.join("deploy.md"),
        "---\nname: deploy\ndescription: Project deploy\n---\nProject body.",
    )
    .unwrap();

    // User-level command with same name (should override project)
    std::fs::write(
        user_cmds.join("deploy.md"),
        "---\nname: deploy\ndescription: User deploy\n---\nUser body.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "deploy");
    assert_eq!(commands[0].description, "User deploy");
    assert_eq!(commands[0].source, CommandSource::User);
}

#[test]
fn test_three_tier_merge_different_names() {
    let workspace = TempDir::new().unwrap();
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();

    // Workspace command
    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(
        ws_cmds.join("ws-cmd.md"),
        "---\nname: ws-cmd\n---\nWorkspace.",
    )
    .unwrap();

    // Project command
    let project_cmds = workspace.path().join(".dolphin-milk").join("commands");
    std::fs::create_dir_all(&project_cmds).unwrap();
    std::fs::write(
        project_cmds.join("proj-cmd.md"),
        "---\nname: proj-cmd\n---\nProject.",
    )
    .unwrap();

    // User command
    std::fs::write(
        user_cmds.join("user-cmd.md"),
        "---\nname: user-cmd\n---\nUser.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));
    assert_eq!(commands.len(), 3);

    let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"ws-cmd"));
    assert!(names.contains(&"proj-cmd"));
    assert!(names.contains(&"user-cmd"));

    // Verify each has the correct source
    let ws = commands.iter().find(|c| c.name == "ws-cmd").unwrap();
    assert_eq!(ws.source, CommandSource::Workspace);
    let proj = commands.iter().find(|c| c.name == "proj-cmd").unwrap();
    assert_eq!(proj.source, CommandSource::Project);
    let user = commands.iter().find(|c| c.name == "user-cmd").unwrap();
    assert_eq!(user.source, CommandSource::User);
}

#[test]
fn test_three_tier_override_chain() {
    // All three tiers have the same command name — user should win
    let workspace = TempDir::new().unwrap();
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();

    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(
        ws_cmds.join("test.md"),
        "---\nname: test\ndescription: Workspace\n---\nWS.",
    )
    .unwrap();

    let project_cmds = workspace.path().join(".dolphin-milk").join("commands");
    std::fs::create_dir_all(&project_cmds).unwrap();
    std::fs::write(
        project_cmds.join("test.md"),
        "---\nname: test\ndescription: Project\n---\nProj.",
    )
    .unwrap();

    std::fs::write(
        user_cmds.join("test.md"),
        "---\nname: test\ndescription: User\n---\nUser.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].description, "User");
    assert_eq!(commands[0].source, CommandSource::User);
}

// ── Skills from .dolphin-milk/skills/ ────────────────────────────────

#[test]
fn test_skills_loaded_from_project_dir() {
    let dir = TempDir::new().unwrap();
    let skills_dir = dir
        .path()
        .join(".dolphin-milk")
        .join("skills")
        .join("my-skill");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(
        skills_dir.join("SKILL.md"),
        "---\nname: my-project-skill\ndescription: A project skill\nauto_activate: false\n---\nDo project things.",
    )
    .unwrap();

    let project_skills_dir = dir.path().join(".dolphin-milk").join("skills");
    let registry = SkillRegistry::load_from_dir(&project_skills_dir);
    assert_eq!(registry.len(), 1);

    let skill = registry.find("my-project-skill").unwrap();
    assert_eq!(skill.name, "my-project-skill");
    assert_eq!(skill.description, "A project skill");
    assert_eq!(skill.instructions, "Do project things.");
}

#[test]
fn test_project_skills_additive_to_global() {
    let global_dir = TempDir::new().unwrap();
    let global_skill = global_dir.path().join("global-skill");
    std::fs::create_dir_all(&global_skill).unwrap();
    std::fs::write(
        global_skill.join("SKILL.md"),
        "---\nname: global\ndescription: Global skill\n---\nGlobal instructions.",
    )
    .unwrap();

    // Load global skills first
    let mut registry = SkillRegistry::load_from_dir(global_dir.path());
    assert_eq!(registry.len(), 1);

    // Create project skills
    let project_dir = TempDir::new().unwrap();
    let project_skill = project_dir.path().join("project-skill");
    std::fs::create_dir_all(&project_skill).unwrap();
    std::fs::write(
        project_skill.join("SKILL.md"),
        "---\nname: project\ndescription: Project skill\n---\nProject instructions.",
    )
    .unwrap();

    // Add project skills
    registry.load_additional_from_dir(project_dir.path());
    assert_eq!(registry.len(), 2);

    // Both should be findable
    assert!(registry.find("global").is_some());
    assert!(registry.find("project").is_some());
}

#[test]
fn test_project_skills_empty_dir_is_noop() {
    let dir = TempDir::new().unwrap();
    let skills_dir = dir.path().join(".dolphin-milk").join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    let mut registry = SkillRegistry::new();
    registry.load_additional_from_dir(&skills_dir);
    assert_eq!(registry.len(), 0);
}

#[test]
fn test_project_skills_nonexistent_dir_is_noop() {
    let dir = TempDir::new().unwrap();
    let skills_dir = dir.path().join(".dolphin-milk").join("skills");
    // Don't create the directory

    let mut registry = SkillRegistry::new();
    registry.load_additional_from_dir(&skills_dir);
    assert_eq!(registry.len(), 0);
}

// ── Integration: detect + use ────────────────────────────────────────

#[test]
fn test_detect_then_load_commands() {
    let workspace = TempDir::new().unwrap();
    let dm = workspace.path().join(PROJECT_DIR_NAME);
    let cmds = dm.join("commands");
    std::fs::create_dir_all(&cmds).unwrap();
    std::fs::write(
        cmds.join("build.md"),
        "---\nname: build\ndescription: Build project\n---\ncargo build",
    )
    .unwrap();

    // Detection says commands are available
    let config = detect_project(workspace.path()).unwrap();
    assert!(config.has_commands);

    // Actually loading confirms it works
    let commands = load_commands_with_user_dir(workspace.path(), None);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "build");
    assert_eq!(commands[0].source, CommandSource::Project);
}

#[test]
fn test_detect_then_load_skills() {
    let workspace = TempDir::new().unwrap();
    let dm = workspace.path().join(PROJECT_DIR_NAME);
    let skills = dm.join("skills").join("custom");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(
        skills.join("SKILL.md"),
        "---\nname: custom\ndescription: Custom\nauto_activate: true\n---\nCustom instructions.",
    )
    .unwrap();

    // Detection says skills are available
    let config = detect_project(workspace.path()).unwrap();
    assert!(config.has_skills);

    // Actually loading confirms it works
    let registry = SkillRegistry::load_from_dir(&config.skills_dir());
    assert_eq!(registry.len(), 1);
    let skill = registry.find("custom").unwrap();
    assert!(skill.auto_activate);
}
