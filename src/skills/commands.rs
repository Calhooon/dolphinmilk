//! Quick Commands — markdown files in `commands/` directories that inject prompts into agent tasks.
//!
//! Users write a markdown file, drop it in `commands/`, and get a reusable command.
//! Commands are loaded from three locations (merged, higher priority wins):
//!
//! - `~/.dolphin-milk/commands/*.md` (user-level, highest priority)
//! - `{workspace}/.dolphin-milk/commands/*.md` (project-level)
//! - `{workspace}/commands/*.md` (workspace-level, lowest priority)
//!
//! Each file has optional YAML frontmatter (`name`, `description`) plus a markdown body.
//! `$ARGUMENTS` in the body is substituted with the arguments passed at invocation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the command was loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandSource {
    /// User-level: `~/.dolphin-milk/commands/`
    User,
    /// Project-level: `{workspace}/.dolphin-milk/commands/`
    Project,
    /// Workspace-level: `{workspace}/commands/`
    Workspace,
}

/// A loaded command definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// Command name (from frontmatter or derived from filename).
    pub name: String,
    /// Human-readable description (from frontmatter, or empty).
    pub description: String,
    /// The markdown body (may contain `$ARGUMENTS` placeholders).
    pub body: String,
    /// Path to the source file.
    pub path: PathBuf,
    /// Where this command was loaded from.
    pub source: CommandSource,
}

/// Load commands from user-level, project-level, and workspace-level directories.
///
/// Priority (highest wins): user > project > workspace. When commands share the
/// same name (case-insensitive), higher-priority sources win.
///
/// Returns commands sorted by name.
pub fn load_commands(workspace: &Path) -> Vec<Command> {
    let user_dir = user_commands_dir();
    load_commands_with_user_dir(workspace, user_dir.as_deref())
}

/// Load commands with an explicit user commands directory.
///
/// This is the testable core — `load_commands` delegates here with the
/// auto-detected `~/.dolphin-milk/commands/` path.
///
/// Load order (lowest priority first, highest priority last):
/// 1. Workspace: `{workspace}/commands/`
/// 2. Project: `{workspace}/.dolphin-milk/commands/`
/// 3. User: `~/.dolphin-milk/commands/`
pub fn load_commands_with_user_dir(workspace: &Path, user_dir: Option<&Path>) -> Vec<Command> {
    let mut commands = Vec::new();

    // Load workspace-level commands first (lowest priority)
    let workspace_dir = workspace.join("commands");
    if workspace_dir.is_dir() {
        load_commands_from_dir(&workspace_dir, CommandSource::Workspace, &mut commands);
    }

    // Load project-level commands (middle priority)
    let project_dir = workspace.join(".dolphin-milk").join("commands");
    if project_dir.is_dir() {
        load_commands_from_dir(&project_dir, CommandSource::Project, &mut commands);
    }

    // Load user-level commands (highest priority)
    if let Some(dir) = user_dir {
        if dir.is_dir() {
            load_commands_from_dir(dir, CommandSource::User, &mut commands);
        }
    }

    // Deduplicate: higher-priority commands override lower-priority with the same name
    let mut seen = std::collections::HashMap::new();
    let mut deduped = Vec::new();
    // Process in reverse so highest priority (added last) wins
    for cmd in commands.into_iter().rev() {
        let key = cmd.name.to_lowercase();
        if seen.contains_key(&key) {
            continue;
        }
        seen.insert(key, true);
        deduped.push(cmd);
    }
    deduped.reverse();

    // Sort by name
    deduped.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    deduped
}

/// Find a command by name (case-insensitive).
pub fn find_command<'a>(commands: &'a [Command], name: &str) -> Option<&'a Command> {
    let lower = name.to_lowercase();
    commands.iter().find(|c| c.name.to_lowercase() == lower)
}

/// Render a command by substituting `$ARGUMENTS` with the provided arguments.
///
/// If `arguments` is `None`, `$ARGUMENTS` is left as-is in the body.
pub fn render_command(command: &Command, arguments: Option<&str>) -> String {
    match arguments {
        Some(args) => command.body.replace("$ARGUMENTS", args),
        None => command.body.clone(),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────

/// Return the user-level commands directory: `~/.dolphin-milk/commands/`.
fn user_commands_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".dolphin-milk").join("commands"))
}

/// Load all `.md` files from a directory into the commands vec.
fn load_commands_from_dir(dir: &Path, source: CommandSource, commands: &mut Vec<Command>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to read commands directory {}: {e}", dir.display());
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Only process .md files
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "md" {
            continue;
        }

        match parse_command_file(&path, source.clone()) {
            Ok(cmd) => commands.push(cmd),
            Err(e) => {
                tracing::warn!("Failed to parse command file {}: {e}", path.display());
            }
        }
    }
}

/// Parse a single command markdown file.
///
/// Supports optional YAML frontmatter with `name` and `description`.
/// If no frontmatter, name is derived from the filename (without `.md`).
fn parse_command_file(path: &Path, source: CommandSource) -> Result<Command, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    parse_command_content(&content, path, source)
}

/// Parse command content from a string, used by `parse_command_file` and tests.
pub fn parse_command_content(
    content: &str,
    path: &Path,
    source: CommandSource,
) -> Result<Command, String> {
    let trimmed = content.trim_start();

    let (name, description, body) = if let Some(after_first) = trimmed.strip_prefix("---") {
        // Has frontmatter — parse it
        let end = after_first
            .find("\n---")
            .ok_or("No closing --- for frontmatter")?;

        let frontmatter_str = after_first[..end].trim();
        let body_str = after_first[end + 4..].trim().to_string();

        let yaml: serde_yaml::Value = serde_yaml::from_str(frontmatter_str)
            .map_err(|e| format!("Invalid YAML frontmatter: {e}"))?;

        let name = yaml
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| name_from_path(path));

        let description = yaml
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        (name, description, body_str)
    } else {
        // No frontmatter — derive name from filename
        let name = name_from_path(path);
        let body = trimmed.trim().to_string();
        (name, String::new(), body)
    };

    Ok(Command {
        name,
        description,
        body,
        path: path.to_path_buf(),
        source,
    })
}

/// Derive a command name from a file path: `review.md` -> `review`.
fn name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_frontmatter() {
        let content = "---\nname: review\ndescription: Code review\n---\nReview the code.";
        let cmd = parse_command_content(content, Path::new("review.md"), CommandSource::Workspace)
            .unwrap();
        assert_eq!(cmd.name, "review");
        assert_eq!(cmd.description, "Code review");
        assert_eq!(cmd.body, "Review the code.");
    }

    #[test]
    fn test_parse_without_frontmatter() {
        let content = "Just do the thing.\n\nWith $ARGUMENTS.";
        let cmd =
            parse_command_content(content, Path::new("deploy.md"), CommandSource::User).unwrap();
        assert_eq!(cmd.name, "deploy");
        assert_eq!(cmd.description, "");
        assert_eq!(cmd.body, "Just do the thing.\n\nWith $ARGUMENTS.");
    }

    #[test]
    fn test_name_from_filename_when_no_frontmatter_name() {
        let content = "---\ndescription: No name field\n---\nBody.";
        let cmd = parse_command_content(content, Path::new("my-cmd.md"), CommandSource::Workspace)
            .unwrap();
        assert_eq!(cmd.name, "my-cmd");
    }

    #[test]
    fn test_render_with_arguments() {
        let cmd = Command {
            name: "review".into(),
            description: "".into(),
            body: "Review $ARGUMENTS carefully.".into(),
            path: PathBuf::from("review.md"),
            source: CommandSource::Workspace,
        };
        let rendered = render_command(&cmd, Some("the wallet module"));
        assert_eq!(rendered, "Review the wallet module carefully.");
    }

    #[test]
    fn test_render_without_arguments() {
        let cmd = Command {
            name: "review".into(),
            description: "".into(),
            body: "Review $ARGUMENTS carefully.".into(),
            path: PathBuf::from("review.md"),
            source: CommandSource::Workspace,
        };
        let rendered = render_command(&cmd, None);
        assert_eq!(rendered, "Review $ARGUMENTS carefully.");
    }

    #[test]
    fn test_find_command_case_insensitive() {
        let commands = vec![Command {
            name: "Review".into(),
            description: "".into(),
            body: "body".into(),
            path: PathBuf::from("Review.md"),
            source: CommandSource::Workspace,
        }];
        assert!(find_command(&commands, "review").is_some());
        assert!(find_command(&commands, "REVIEW").is_some());
        assert!(find_command(&commands, "Review").is_some());
        assert!(find_command(&commands, "nonexistent").is_none());
    }
}
