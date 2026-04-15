//! Tests for the Quick Commands system (src/skills/commands.rs).
//!
//! Covers loading, parsing, overriding, rendering, and edge cases.
//! Uses `load_commands_with_user_dir` to avoid modifying the process-wide
//! HOME environment variable (which would race across parallel tests).

use std::path::Path;

use dolphin_milk::skills::commands::{
    find_command, load_commands_with_user_dir, parse_command_content, render_command, Command,
    CommandSource,
};
use tempfile::TempDir;

// ── Loading from workspace commands/ ───────────────────────────────

#[test]
fn test_load_from_workspace_commands_dir() {
    let dir = TempDir::new().unwrap();
    let commands_dir = dir.path().join("commands");
    std::fs::create_dir_all(&commands_dir).unwrap();
    std::fs::write(
        commands_dir.join("review.md"),
        "---\nname: review\ndescription: Code review\n---\nReview the code.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(dir.path(), None);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "review");
    assert_eq!(commands[0].description, "Code review");
    assert_eq!(commands[0].body, "Review the code.");
    assert_eq!(commands[0].source, CommandSource::Workspace);
}

// ── Loading from user commands/ ────────────────────────────────────

#[test]
fn test_load_from_user_commands_dir() {
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();
    std::fs::write(
        user_cmds.join("deploy.md"),
        "---\nname: deploy\ndescription: Deploy service\n---\nDeploy $ARGUMENTS.",
    )
    .unwrap();

    let workspace = TempDir::new().unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "deploy");
    assert_eq!(commands[0].source, CommandSource::User);
}

// ── User commands override workspace commands ─────────────────────

#[test]
fn test_user_overrides_workspace_same_name() {
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();
    std::fs::write(
        user_cmds.join("review.md"),
        "---\nname: review\ndescription: User review\n---\nUser body.",
    )
    .unwrap();

    let workspace = TempDir::new().unwrap();
    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(
        ws_cmds.join("review.md"),
        "---\nname: review\ndescription: Workspace review\n---\nWorkspace body.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));

    // Should have only one "review" -- the user version
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "review");
    assert_eq!(commands[0].description, "User review");
    assert_eq!(commands[0].source, CommandSource::User);
}

// ── YAML frontmatter parsing ──────────────────────────────────────

#[test]
fn test_yaml_frontmatter_parsing() {
    let content = "---\nname: test-cmd\ndescription: A test command\n---\nDo the thing.";
    let cmd =
        parse_command_content(content, Path::new("test-cmd.md"), CommandSource::Workspace).unwrap();
    assert_eq!(cmd.name, "test-cmd");
    assert_eq!(cmd.description, "A test command");
    assert_eq!(cmd.body, "Do the thing.");
}

// ── Name derived from filename when no frontmatter ────────────────

#[test]
fn test_name_from_filename_no_frontmatter() {
    let content = "Just do the thing.\nWith multiple lines.";
    let cmd = parse_command_content(content, Path::new("my-action.md"), CommandSource::Workspace)
        .unwrap();
    assert_eq!(cmd.name, "my-action");
    assert_eq!(cmd.description, "");
    assert_eq!(cmd.body, "Just do the thing.\nWith multiple lines.");
}

// ── Name derived from filename when frontmatter has no name ───────

#[test]
fn test_name_from_filename_when_frontmatter_has_no_name() {
    let content = "---\ndescription: Has description but no name\n---\nBody text.";
    let cmd =
        parse_command_content(content, Path::new("fallback.md"), CommandSource::User).unwrap();
    assert_eq!(cmd.name, "fallback");
    assert_eq!(cmd.description, "Has description but no name");
}

// ── $ARGUMENTS substitution ───────────────────────────────────────

#[test]
fn test_arguments_substitution() {
    let cmd = Command {
        name: "review".into(),
        description: "".into(),
        body: "Review the following: $ARGUMENTS\n\nFocus on security.".into(),
        path: "review.md".into(),
        source: CommandSource::Workspace,
    };
    let rendered = render_command(&cmd, Some("wallet module"));
    assert_eq!(
        rendered,
        "Review the following: wallet module\n\nFocus on security."
    );
}

// ── $ARGUMENTS with no arguments provided ─────────────────────────

#[test]
fn test_arguments_no_args_provided() {
    let cmd = Command {
        name: "review".into(),
        description: "".into(),
        body: "Review $ARGUMENTS carefully.".into(),
        path: "review.md".into(),
        source: CommandSource::Workspace,
    };
    let rendered = render_command(&cmd, None);
    assert_eq!(rendered, "Review $ARGUMENTS carefully.");
}

// ── Multiple $ARGUMENTS occurrences ───────────────────────────────

#[test]
fn test_multiple_arguments_substitution() {
    let cmd = Command {
        name: "analyze".into(),
        description: "".into(),
        body: "Analyze $ARGUMENTS. Summary of $ARGUMENTS.".into(),
        path: "analyze.md".into(),
        source: CommandSource::Workspace,
    };
    let rendered = render_command(&cmd, Some("the code"));
    assert_eq!(rendered, "Analyze the code. Summary of the code.");
}

// ── find_command case-insensitive ─────────────────────────────────

#[test]
fn test_find_command_case_insensitive() {
    let commands = vec![
        Command {
            name: "Review".into(),
            description: "review desc".into(),
            body: "body".into(),
            path: "Review.md".into(),
            source: CommandSource::Workspace,
        },
        Command {
            name: "deploy".into(),
            description: "deploy desc".into(),
            body: "body".into(),
            path: "deploy.md".into(),
            source: CommandSource::User,
        },
    ];

    assert!(find_command(&commands, "review").is_some());
    assert!(find_command(&commands, "REVIEW").is_some());
    assert!(find_command(&commands, "Review").is_some());
    assert!(find_command(&commands, "Deploy").is_some());
    assert!(find_command(&commands, "nonexistent").is_none());
}

// ── render_command output ─────────────────────────────────────────

#[test]
fn test_render_command_preserves_formatting() {
    let cmd = Command {
        name: "format".into(),
        description: "".into(),
        body: "# Header\n\n- Item 1\n- Item 2\n\n$ARGUMENTS".into(),
        path: "format.md".into(),
        source: CommandSource::Workspace,
    };
    let rendered = render_command(&cmd, Some("extra context"));
    assert_eq!(rendered, "# Header\n\n- Item 1\n- Item 2\n\nextra context");
}

// ── Empty commands directory ──────────────────────────────────────

#[test]
fn test_empty_commands_directory() {
    let dir = TempDir::new().unwrap();
    let commands_dir = dir.path().join("commands");
    std::fs::create_dir_all(&commands_dir).unwrap();

    let commands = load_commands_with_user_dir(dir.path(), None);
    assert!(commands.is_empty());
}

// ── No commands directory at all ──────────────────────────────────

#[test]
fn test_no_commands_directory() {
    let dir = TempDir::new().unwrap();
    // No commands/ directory exists

    let commands = load_commands_with_user_dir(dir.path(), None);
    assert!(commands.is_empty());
}

// ── Non-.md files ignored ─────────────────────────────────────────

#[test]
fn test_non_md_files_ignored() {
    let dir = TempDir::new().unwrap();
    let commands_dir = dir.path().join("commands");
    std::fs::create_dir_all(&commands_dir).unwrap();

    // Create various non-.md files
    std::fs::write(commands_dir.join("notes.txt"), "some notes").unwrap();
    std::fs::write(commands_dir.join("config.json"), "{}").unwrap();
    std::fs::write(commands_dir.join("README"), "readme").unwrap();
    std::fs::write(
        commands_dir.join("valid.md"),
        "---\nname: valid\n---\nBody.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(dir.path(), None);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "valid");
}

// ── Command with no frontmatter ───────────────────────────────────

#[test]
fn test_command_with_no_frontmatter() {
    let dir = TempDir::new().unwrap();
    let commands_dir = dir.path().join("commands");
    std::fs::create_dir_all(&commands_dir).unwrap();
    std::fs::write(
        commands_dir.join("simple.md"),
        "Just a plain markdown body.\n\nNo frontmatter here.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(dir.path(), None);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "simple");
    assert_eq!(commands[0].description, "");
    assert!(commands[0].body.starts_with("Just a plain markdown body."));
}

// ── Multiple commands loaded and sorted ───────────────────────────

#[test]
fn test_multiple_commands_sorted() {
    let dir = TempDir::new().unwrap();
    let commands_dir = dir.path().join("commands");
    std::fs::create_dir_all(&commands_dir).unwrap();
    std::fs::write(
        commands_dir.join("zebra.md"),
        "---\nname: zebra\n---\nZebra body.",
    )
    .unwrap();
    std::fs::write(
        commands_dir.join("alpha.md"),
        "---\nname: alpha\n---\nAlpha body.",
    )
    .unwrap();
    std::fs::write(
        commands_dir.join("middle.md"),
        "---\nname: middle\n---\nMiddle body.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(dir.path(), None);
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].name, "alpha");
    assert_eq!(commands[1].name, "middle");
    assert_eq!(commands[2].name, "zebra");
}

// ── Both sources merge (different names) ──────────────────────────

#[test]
fn test_merge_user_and_workspace_different_names() {
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();
    std::fs::write(
        user_cmds.join("user-cmd.md"),
        "---\nname: user-cmd\n---\nUser body.",
    )
    .unwrap();

    let workspace = TempDir::new().unwrap();
    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(
        ws_cmds.join("ws-cmd.md"),
        "---\nname: ws-cmd\n---\nWorkspace body.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));

    // Should have both commands
    assert_eq!(commands.len(), 2);
    let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"user-cmd"));
    assert!(names.contains(&"ws-cmd"));
}

// ── Command with empty body ───────────────────────────────────────

#[test]
fn test_command_with_empty_body() {
    let content = "---\nname: empty\ndescription: Empty body command\n---\n";
    let cmd =
        parse_command_content(content, Path::new("empty.md"), CommandSource::Workspace).unwrap();
    assert_eq!(cmd.name, "empty");
    assert_eq!(cmd.body, "");
}

// ── Arguments substitution with empty string ──────────────────────

#[test]
fn test_arguments_substitution_empty_string() {
    let cmd = Command {
        name: "test".into(),
        description: "".into(),
        body: "Do $ARGUMENTS now.".into(),
        path: "test.md".into(),
        source: CommandSource::Workspace,
    };
    let rendered = render_command(&cmd, Some(""));
    assert_eq!(rendered, "Do  now.");
}

// ── find_command with empty slice ─────────────────────────────────

#[test]
fn test_find_command_empty_list() {
    let commands: Vec<Command> = vec![];
    assert!(find_command(&commands, "anything").is_none());
}

// ── User non-existent dir is graceful ─────────────────────────────

#[test]
fn test_nonexistent_user_dir_graceful() {
    let workspace = TempDir::new().unwrap();
    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(ws_cmds.join("cmd.md"), "---\nname: cmd\n---\nBody.").unwrap();

    let nonexistent = Path::new("/tmp/nonexistent-commands-dir-for-test-12345");
    let commands = load_commands_with_user_dir(workspace.path(), Some(nonexistent));
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "cmd");
}

// ── Case-insensitive override ─────────────────────────────────────

#[test]
fn test_case_insensitive_override() {
    let user_dir = TempDir::new().unwrap();
    let user_cmds = user_dir.path().join("commands");
    std::fs::create_dir_all(&user_cmds).unwrap();
    std::fs::write(
        user_cmds.join("REVIEW.md"),
        "---\nname: REVIEW\n---\nUser uppercase.",
    )
    .unwrap();

    let workspace = TempDir::new().unwrap();
    let ws_cmds = workspace.path().join("commands");
    std::fs::create_dir_all(&ws_cmds).unwrap();
    std::fs::write(
        ws_cmds.join("review.md"),
        "---\nname: review\n---\nWorkspace lowercase.",
    )
    .unwrap();

    let commands = load_commands_with_user_dir(workspace.path(), Some(&user_cmds));

    // Should deduplicate case-insensitively, user wins
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].source, CommandSource::User);
}
