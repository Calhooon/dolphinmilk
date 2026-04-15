//! Tests for skill-local state — config.json loading and persistent data directory.

use std::path::Path;
use tempfile::TempDir;

use dolphin_milk::skills::loader::{load_skills_from_dir, parse_skill_content};
use dolphin_milk::skills::{SkillRegistry, SkillState};

// -- Helpers --

/// Create a test skill directory with SKILL.md and optional config.json.
fn create_test_skill(dir: &Path, name: &str, config: Option<&str>) {
    let skill_dir = dir.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!(
            "---\nname: {}\ndescription: test skill\nauto_activate: false\n---\nTest content for {}.",
            name, name
        ),
    )
    .unwrap();
    if let Some(config_json) = config {
        std::fs::write(skill_dir.join("config.json"), config_json).unwrap();
    }
}

// -- SkillState defaults --

#[test]
fn test_skill_state_default_no_config() {
    // parse_skill_content creates a SkillState with no config
    let content = "---\nname: my-skill\n---\nInstructions.";
    let skill = parse_skill_content(content, "test.md").unwrap();
    assert!(
        skill.state.config.is_none(),
        "parse_skill_content should produce None config"
    );
}

#[test]
fn test_skill_state_missing_config_is_ok() {
    // A skill directory without config.json should load fine with config = None
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "no-config", None);

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "no-config");
    assert!(
        skills[0].state.config.is_none(),
        "skill without config.json should have config = None"
    );
}

// -- config.json loading --

#[test]
fn test_skill_state_loads_config_json() {
    let dir = TempDir::new().unwrap();
    create_test_skill(
        dir.path(),
        "with-config",
        Some(r#"{"key": "value", "count": 42}"#),
    );

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    let config = skills[0]
        .state
        .config
        .as_ref()
        .expect("config should be Some");
    assert_eq!(config["key"], "value");
    assert_eq!(config["count"], 42);
}

#[test]
fn test_skill_state_invalid_json_returns_none() {
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "bad-json", Some("not valid json {{{"));

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "bad-json");
    // Invalid JSON is warned but skill still loads with config = None
    assert!(
        skills[0].state.config.is_none(),
        "invalid JSON should result in config = None"
    );
}

#[test]
fn test_skill_state_handles_nested_json_config() {
    let dir = TempDir::new().unwrap();
    let nested_config = r#"{
        "model": "gpt-5",
        "parameters": {
            "temperature": 0.7,
            "max_tokens": 1000
        },
        "tags": ["fast", "cheap"],
        "enabled": true
    }"#;
    create_test_skill(dir.path(), "nested", Some(nested_config));

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    let config = skills[0]
        .state
        .config
        .as_ref()
        .expect("config should be Some");
    assert_eq!(config["model"], "gpt-5");
    assert_eq!(config["parameters"]["temperature"], 0.7);
    assert_eq!(config["parameters"]["max_tokens"], 1000);
    assert_eq!(config["tags"][0], "fast");
    assert_eq!(config["tags"][1], "cheap");
    assert_eq!(config["enabled"], true);
}

#[test]
fn test_skill_state_config_is_readonly() {
    // SkillState has no write method for config — config is pub but read-only
    // by convention. This test verifies it's loaded once and not mutated.
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "readonly", Some(r#"{"version": 1}"#));

    let skills = load_skills_from_dir(dir.path()).unwrap();
    let config = skills[0].state.config.as_ref().unwrap();
    assert_eq!(config["version"], 1);

    // No write_config method exists on SkillState — config is loaded once.
    // The struct only has read_data/write_data for the data directory.
    // Verify the config file on disk hasn't changed:
    let on_disk: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("readonly").join("config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(on_disk["version"], 1);
}

// -- Data directory path convention --

#[test]
fn test_skill_state_data_dir_path_convention() {
    // parse_skill_content sets data_dir to working/skills/{name}/
    let content = "---\nname: my-skill\n---\nInstructions.";
    let skill = parse_skill_content(content, "test.md").unwrap();
    let expected = std::path::PathBuf::from("working")
        .join("skills")
        .join("my-skill");
    assert_eq!(skill.state.data_dir, expected);
}

#[test]
fn test_skill_state_data_dir_created_on_write() {
    // Data directory should NOT exist before write_data is called
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("test-skill");

    let state = SkillState::new(None, data_dir.clone());

    // Before write: directory does not exist
    assert!(
        !data_dir.exists(),
        "data dir should not exist before first write"
    );

    // After write: directory exists and file is written
    let success = state.write_data("output.txt", "hello world");
    assert!(success, "write_data should succeed");
    assert!(data_dir.exists(), "data dir should be created on write");
    assert!(
        data_dir.join("output.txt").exists(),
        "file should exist after write"
    );
}

// -- Read/write data operations --

#[test]
fn test_skill_state_write_then_read_data() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("test-skill");
    let state = SkillState::new(None, data_dir);

    // Write a file
    assert!(state.write_data("log.txt", "entry 1\nentry 2\n"));

    // Read it back
    let content = state.read_data("log.txt");
    assert!(content.is_some());
    assert_eq!(content.unwrap(), "entry 1\nentry 2\n");
}

#[test]
fn test_skill_state_read_nonexistent_returns_none() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("ghost");
    let state = SkillState::new(None, data_dir);

    // Reading a file that doesn't exist should return None
    let result = state.read_data("does-not-exist.txt");
    assert!(
        result.is_none(),
        "reading nonexistent file should return None"
    );
}

#[test]
fn test_skill_state_data_dir_persists_across_loads() {
    // Simulate two separate loads: data written by first load is visible in second
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("persistent");

    // First "session": write data
    let state1 = SkillState::new(None, data_dir.clone());
    assert!(state1.write_data("counter.txt", "42"));

    // Second "session": create new SkillState pointing at same data_dir
    let state2 = SkillState::new(None, data_dir);
    let content = state2.read_data("counter.txt");
    assert_eq!(content, Some("42".to_string()));
}

// -- Skill isolation --

#[test]
fn test_skill_state_isolation_between_skills() {
    let dir = TempDir::new().unwrap();
    let data_dir_a = dir.path().join("skills").join("skill-a");
    let data_dir_b = dir.path().join("skills").join("skill-b");

    let state_a = SkillState::new(None, data_dir_a.clone());
    let state_b = SkillState::new(None, data_dir_b.clone());

    // Write to skill A
    assert!(state_a.write_data("secret.txt", "A's secret"));

    // Skill B cannot see A's file
    assert!(
        state_b.read_data("secret.txt").is_none(),
        "skill B should not see skill A's data"
    );

    // Write to skill B
    assert!(state_b.write_data("secret.txt", "B's secret"));

    // Each reads its own
    assert_eq!(
        state_a.read_data("secret.txt"),
        Some("A's secret".to_string())
    );
    assert_eq!(
        state_b.read_data("secret.txt"),
        Some("B's secret".to_string())
    );

    // Directories are separate
    assert_ne!(data_dir_a, data_dir_b);
}

// -- Registry integration --

#[test]
fn test_skill_registry_loads_skills_with_state() {
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "alpha", Some(r#"{"mode": "fast"}"#));
    create_test_skill(dir.path(), "beta", None);

    let registry = SkillRegistry::load_from_dir(dir.path());
    assert_eq!(registry.len(), 2);

    let alpha = registry.find("alpha").expect("alpha should exist");
    assert!(alpha.state.config.is_some());
    assert_eq!(alpha.state.config.as_ref().unwrap()["mode"], "fast");

    let beta = registry.find("beta").expect("beta should exist");
    assert!(beta.state.config.is_none());
}

#[test]
fn test_skill_with_config_json_has_config() {
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "configured", Some(r#"{"timeout": 30}"#));

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert!(skills[0].state.config.is_some());
    assert_eq!(skills[0].state.config.as_ref().unwrap()["timeout"], 30);
}

#[test]
fn test_skill_without_config_json_has_none() {
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "bare", None);

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert!(skills[0].state.config.is_none());
}

// -- Edge cases --

#[test]
fn test_skill_state_write_overwrites_existing() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("overwrite");
    let state = SkillState::new(None, data_dir);

    assert!(state.write_data("value.txt", "first"));
    assert_eq!(state.read_data("value.txt"), Some("first".to_string()));

    assert!(state.write_data("value.txt", "second"));
    assert_eq!(state.read_data("value.txt"), Some("second".to_string()));
}

#[test]
fn test_skill_state_write_empty_content() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("empty");
    let state = SkillState::new(None, data_dir);

    assert!(state.write_data("empty.txt", ""));
    assert_eq!(state.read_data("empty.txt"), Some(String::new()));
}

#[test]
fn test_skill_state_multiple_files() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("skills").join("multi");
    let state = SkillState::new(None, data_dir);

    assert!(state.write_data("a.txt", "alpha"));
    assert!(state.write_data("b.txt", "beta"));
    assert!(state.write_data("c.json", r#"{"key": "gamma"}"#));

    assert_eq!(state.read_data("a.txt"), Some("alpha".to_string()));
    assert_eq!(state.read_data("b.txt"), Some("beta".to_string()));
    assert_eq!(
        state.read_data("c.json"),
        Some(r#"{"key": "gamma"}"#.to_string())
    );
}

#[test]
fn test_skill_state_clone_preserves_config() {
    let config = serde_json::json!({"version": 2, "features": ["a", "b"]});
    let state = SkillState::new(Some(config.clone()), std::path::PathBuf::from("/tmp/test"));
    let cloned = state.clone();
    assert_eq!(cloned.config, Some(config));
    assert_eq!(cloned.data_dir, state.data_dir);
}

#[test]
fn test_skill_state_default() {
    let state = SkillState::default();
    assert!(state.config.is_none());
    assert_eq!(state.data_dir, std::path::PathBuf::new());
}

#[test]
fn test_skill_state_config_empty_json_object() {
    // An empty JSON object {} is valid config, not None
    let dir = TempDir::new().unwrap();
    create_test_skill(dir.path(), "empty-config", Some("{}"));

    let skills = load_skills_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 1);
    let config = skills[0]
        .state
        .config
        .as_ref()
        .expect("empty object is valid config");
    assert!(config.is_object());
    assert_eq!(config.as_object().unwrap().len(), 0);
}
