//! Tests for DOLPHIN-MILK.md layered instruction loader.
//!
//! Covers: file loading, directory walk, YAML frontmatter parsing,
//! git root detection, prompt injection, and priority ordering.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use dolphin_milk::context::instructions::{
    format_instructions, load_instructions, InstructionFile,
};
use dolphin_milk::context::prompt::{build_system_prompt, PromptContext};

fn default_ctx() -> PromptContext {
    PromptContext {
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
    }
}

// ---------------------------------------------------------------------------
// 1. No files found returns empty
// ---------------------------------------------------------------------------

#[test]
fn test_no_instruction_files_returns_empty() {
    let dir = TempDir::new().unwrap();
    let files = load_instructions(dir.path());
    assert!(files.is_empty(), "should return empty when no files exist");
}

// ---------------------------------------------------------------------------
// 2. Load from workspace root DOLPHIN-MILK.md
// ---------------------------------------------------------------------------

#[test]
fn test_load_dolphin_milk_md_from_workspace() {
    let dir = TempDir::new().unwrap();
    let md_path = dir.path().join("DOLPHIN-MILK.md");
    fs::write(&md_path, "Always use gpt-5-mini for cheap tasks.").unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "Always use gpt-5-mini for cheap tasks.");
    assert!(files[0].paths.is_none());
    assert_eq!(files[0].path, md_path);
}

// ---------------------------------------------------------------------------
// 3. Load from .dolphin-milk/INSTRUCTIONS.md
// ---------------------------------------------------------------------------

#[test]
fn test_load_from_dolphin_milk_instructions_dir() {
    let dir = TempDir::new().unwrap();
    let dm_dir = dir.path().join(".dolphin-milk");
    fs::create_dir_all(&dm_dir).unwrap();
    fs::write(dm_dir.join("INSTRUCTIONS.md"), "Use BSV for payments.").unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "Use BSV for payments.");
}

// ---------------------------------------------------------------------------
// 4. Both files loaded with correct priority order
// ---------------------------------------------------------------------------

#[test]
fn test_both_files_priority_order() {
    let dir = TempDir::new().unwrap();

    // .dolphin-milk/INSTRUCTIONS.md (highest priority, first)
    let dm_dir = dir.path().join(".dolphin-milk");
    fs::create_dir_all(&dm_dir).unwrap();
    fs::write(dm_dir.join("INSTRUCTIONS.md"), "Priority 1").unwrap();

    // DOLPHIN-MILK.md in workspace root
    fs::write(dir.path().join("DOLPHIN-MILK.md"), "Priority 2").unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].content, "Priority 1");
    assert_eq!(files[1].content, "Priority 2");
}

// ---------------------------------------------------------------------------
// 5. Walk up directory tree to find files
// ---------------------------------------------------------------------------

#[test]
fn test_walk_up_directory_tree() {
    let root = TempDir::new().unwrap();
    // Create nested directory structure
    let nested = root.path().join("projects").join("myapp");
    fs::create_dir_all(&nested).unwrap();

    // Put DOLPHIN-MILK.md in the root
    fs::write(root.path().join("DOLPHIN-MILK.md"), "Root instructions").unwrap();

    // Also in projects/
    fs::write(
        root.path().join("projects").join("DOLPHIN-MILK.md"),
        "Projects instructions",
    )
    .unwrap();

    let files = load_instructions(&nested);
    // Should find both: projects/ (closest first), then root
    assert!(files.len() >= 2, "should find at least 2 files walking up");
    assert_eq!(files[0].content, "Projects instructions");
    assert_eq!(files[1].content, "Root instructions");
}

// ---------------------------------------------------------------------------
// 6. Git root detection stops the walk
// ---------------------------------------------------------------------------

#[test]
fn test_git_root_stops_walk() {
    let root = TempDir::new().unwrap();
    // Create a .git directory to simulate a git repo
    fs::create_dir_all(root.path().join(".git")).unwrap();

    // Create nested workspace
    let workspace = root.path().join("src").join("module");
    fs::create_dir_all(&workspace).unwrap();

    // Put DOLPHIN-MILK.md at git root
    fs::write(root.path().join("DOLPHIN-MILK.md"), "Git root instructions").unwrap();

    let files = load_instructions(&workspace);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "Git root instructions");
}

// ---------------------------------------------------------------------------
// 7. YAML frontmatter parsing with paths field
// ---------------------------------------------------------------------------

#[test]
fn test_yaml_frontmatter_with_paths() {
    let dir = TempDir::new().unwrap();
    let content =
        "---\npaths:\n  - \"src/**/*.rs\"\n  - \"tests/**\"\n---\nRust-specific instructions.";
    fs::write(dir.path().join("DOLPHIN-MILK.md"), content).unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "Rust-specific instructions.");
    let paths = files[0].paths.as_ref().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], "src/**/*.rs");
    assert_eq!(paths[1], "tests/**");
}

// ---------------------------------------------------------------------------
// 8. YAML frontmatter with no paths field
// ---------------------------------------------------------------------------

#[test]
fn test_yaml_frontmatter_without_paths() {
    let dir = TempDir::new().unwrap();
    let content = "---\nauthor: John\n---\nGeneral instructions.";
    fs::write(dir.path().join("DOLPHIN-MILK.md"), content).unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "General instructions.");
    assert!(files[0].paths.is_none());
}

// ---------------------------------------------------------------------------
// 9. No frontmatter (plain markdown)
// ---------------------------------------------------------------------------

#[test]
fn test_no_frontmatter() {
    let dir = TempDir::new().unwrap();
    let content = "# Project Instructions\n\nJust plain markdown.";
    fs::write(dir.path().join("DOLPHIN-MILK.md"), content).unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, content);
    assert!(files[0].paths.is_none());
}

// ---------------------------------------------------------------------------
// 10. Invalid frontmatter (no closing ---) treated as plain content
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_frontmatter_no_closing() {
    let dir = TempDir::new().unwrap();
    let content = "---\npaths:\n  - foo\nThis has no closing delimiter";
    fs::write(dir.path().join("DOLPHIN-MILK.md"), content).unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    // Entire content is treated as body since frontmatter is invalid
    assert_eq!(files[0].content, content);
    assert!(files[0].paths.is_none());
}

// ---------------------------------------------------------------------------
// 11. Content injected into system prompt
// ---------------------------------------------------------------------------

#[test]
fn test_instructions_in_system_prompt() {
    let mut ctx = default_ctx();
    ctx.instructions = "Always respond in haiku format.".to_string();

    let prompt = build_system_prompt(&ctx);
    assert!(
        prompt.contains("# Project Instructions"),
        "should have instructions header"
    );
    assert!(
        prompt.contains("Always respond in haiku format."),
        "should include instruction content"
    );
}

// ---------------------------------------------------------------------------
// 12. Empty instructions not in prompt
// ---------------------------------------------------------------------------

#[test]
fn test_empty_instructions_not_in_prompt() {
    let ctx = default_ctx();
    let prompt = build_system_prompt(&ctx);
    assert!(
        !prompt.contains("# Project Instructions"),
        "should NOT have instructions header when empty"
    );
}

// ---------------------------------------------------------------------------
// 13. format_instructions combines multiple files
// ---------------------------------------------------------------------------

#[test]
fn test_format_instructions_multiple_files() {
    let files = vec![
        InstructionFile {
            path: PathBuf::from("/project/.dolphin-milk/INSTRUCTIONS.md"),
            content: "High priority rule.".to_string(),
            paths: None,
        },
        InstructionFile {
            path: PathBuf::from("/project/DOLPHIN-MILK.md"),
            content: "General rule.".to_string(),
            paths: Some(vec!["src/**".to_string()]),
        },
    ];
    let result = format_instructions(&files);
    assert!(result.contains("High priority rule."));
    assert!(result.contains("General rule."));
    assert!(result.contains("/project/.dolphin-milk/INSTRUCTIONS.md"));
    assert!(result.contains("/project/DOLPHIN-MILK.md"));
    // High priority file should appear first
    let pos1 = result.find("High priority rule.").unwrap();
    let pos2 = result.find("General rule.").unwrap();
    assert!(pos1 < pos2, "higher priority file should come first");
}

// ---------------------------------------------------------------------------
// 14. InstructionFile struct fields
// ---------------------------------------------------------------------------

#[test]
fn test_instruction_file_struct() {
    let f = InstructionFile {
        path: PathBuf::from("/tmp/test/DOLPHIN-MILK.md"),
        content: "Test content".to_string(),
        paths: Some(vec!["*.rs".to_string()]),
    };
    assert_eq!(f.path, PathBuf::from("/tmp/test/DOLPHIN-MILK.md"));
    assert_eq!(f.content, "Test content");
    assert_eq!(f.paths.as_ref().unwrap(), &vec!["*.rs".to_string()]);
}

// ---------------------------------------------------------------------------
// 15. Instructions section positioned after Identity in prompt
// ---------------------------------------------------------------------------

#[test]
fn test_instructions_after_identity_in_prompt() {
    let mut ctx = default_ctx();
    ctx.instructions = "Custom project instructions here.".to_string();

    let prompt = build_system_prompt(&ctx);

    let identity_pos = prompt.find("# Identity").unwrap();
    let instructions_pos = prompt.find("# Project Instructions").unwrap();
    let environment_pos = prompt.find("# Environment").unwrap();

    assert!(
        instructions_pos > identity_pos,
        "instructions should come after identity"
    );
    assert!(
        instructions_pos < environment_pos,
        "instructions should come before environment"
    );
}

// ---------------------------------------------------------------------------
// 16. Multiline content preserved
// ---------------------------------------------------------------------------

#[test]
fn test_multiline_content_preserved() {
    let dir = TempDir::new().unwrap();
    let content =
        "---\npaths:\n  - \"**/*.rs\"\n---\n# Rules\n\n1. Be safe\n2. Be fast\n3. Be correct";
    fs::write(dir.path().join("DOLPHIN-MILK.md"), content).unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert!(files[0].content.contains("# Rules"));
    assert!(files[0].content.contains("1. Be safe"));
    assert!(files[0].content.contains("3. Be correct"));
}

// ---------------------------------------------------------------------------
// 17. Empty file produces empty content
// ---------------------------------------------------------------------------

#[test]
fn test_empty_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("DOLPHIN-MILK.md"), "").unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "");
}

// ---------------------------------------------------------------------------
// 18. .dolphin-milk/INSTRUCTIONS.md takes priority over DOLPHIN-MILK.md
// ---------------------------------------------------------------------------

#[test]
fn test_dot_dir_instructions_before_root() {
    let dir = TempDir::new().unwrap();

    let dm_dir = dir.path().join(".dolphin-milk");
    fs::create_dir_all(&dm_dir).unwrap();
    fs::write(dm_dir.join("INSTRUCTIONS.md"), "from .dolphin-milk dir").unwrap();
    fs::write(dir.path().join("DOLPHIN-MILK.md"), "from root").unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 2);
    // .dolphin-milk/INSTRUCTIONS.md should be first (highest priority)
    assert!(files[0].path.ends_with("INSTRUCTIONS.md"));
    assert!(files[1].path.ends_with("DOLPHIN-MILK.md"));
}

// ---------------------------------------------------------------------------
// 19. Git root in nested workspace
// ---------------------------------------------------------------------------

#[test]
fn test_git_root_found_above_workspace() {
    let root = TempDir::new().unwrap();
    // Simulate git repo at root
    fs::create_dir_all(root.path().join(".git")).unwrap();
    fs::write(root.path().join("DOLPHIN-MILK.md"), "Root level").unwrap();

    // Deep nested workspace
    let deep = root.path().join("a").join("b").join("c");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("DOLPHIN-MILK.md"), "Deep level").unwrap();

    let files = load_instructions(&deep);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].content, "Deep level");
    assert_eq!(files[1].content, "Root level");
}

// ---------------------------------------------------------------------------
// 20. format_instructions returns empty string for empty input
// ---------------------------------------------------------------------------

#[test]
fn test_format_instructions_empty_vec() {
    let result = format_instructions(&[]);
    assert_eq!(result, "");
}

// ---------------------------------------------------------------------------
// 21. Clone works on InstructionFile
// ---------------------------------------------------------------------------

#[test]
fn test_instruction_file_clone() {
    let f = InstructionFile {
        path: PathBuf::from("/tmp/test.md"),
        content: "Original".to_string(),
        paths: Some(vec!["*.rs".to_string()]),
    };
    let cloned = f.clone();
    assert_eq!(cloned.path, f.path);
    assert_eq!(cloned.content, f.content);
    assert_eq!(cloned.paths, f.paths);
}

// ---------------------------------------------------------------------------
// 22. Nonexistent workspace path returns empty
// ---------------------------------------------------------------------------

#[test]
fn test_nonexistent_workspace() {
    let files = load_instructions(&PathBuf::from("/nonexistent/path/to/workspace"));
    assert!(files.is_empty());
}

// ---------------------------------------------------------------------------
// 23. YAML frontmatter with extra fields is tolerated
// ---------------------------------------------------------------------------

#[test]
fn test_frontmatter_extra_fields_tolerated() {
    let dir = TempDir::new().unwrap();
    let content = "---\npaths:\n  - \"*.rs\"\nauthor: John\nversion: 2\n---\nBody text.";
    fs::write(dir.path().join("DOLPHIN-MILK.md"), content).unwrap();

    let files = load_instructions(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].content, "Body text.");
    let paths = files[0].paths.as_ref().unwrap();
    assert_eq!(paths, &vec!["*.rs"]);
}
