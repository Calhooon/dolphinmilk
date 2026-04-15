//! DOLPHIN-MILK.md layered instruction loader.
//!
//! Scans for `DOLPHIN-MILK.md` files starting from the workspace directory and
//! walking up to the git root. Also checks `.dolphin-milk/INSTRUCTIONS.md` in
//! the workspace. Files with optional YAML frontmatter can include a `paths:`
//! field with glob patterns for conditional activation.

use std::path::{Path, PathBuf};

/// A single instruction file loaded from disk.
#[derive(Debug, Clone)]
pub struct InstructionFile {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// The instruction content (frontmatter stripped).
    pub content: String,
    /// Optional glob patterns for conditional activation.
    pub paths: Option<Vec<String>>,
}

/// Find the git root by walking up from `start` looking for a `.git` directory
/// or file (worktrees use a `.git` file).
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Parse optional YAML frontmatter delimited by `---` lines.
///
/// Returns `(paths, body)` where `paths` is extracted from the YAML `paths:`
/// key and `body` is the remaining content after the closing `---`.
fn parse_frontmatter(raw: &str) -> (Option<Vec<String>>, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (None, raw.to_string());
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    // Skip the rest of the first --- line (could be "---\n" or "---\r\n")
    let after_first = after_first.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_first.find("\n---") {
        let yaml_block = &after_first[..end_pos];
        // Body starts after the closing --- line
        let rest = &after_first[end_pos + 4..]; // skip "\n---"
        let body = rest.trim_start_matches(['\r', '\n']);

        // Parse YAML for `paths` field
        let paths = parse_paths_from_yaml(yaml_block);
        (paths, body.to_string())
    } else {
        // No closing ---, treat entire content as body (not valid frontmatter)
        (None, raw.to_string())
    }
}

/// Extract `paths:` list from a YAML block.
///
/// Uses `serde_yaml` for robust parsing.
fn parse_paths_from_yaml(yaml: &str) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Frontmatter {
        paths: Option<Vec<String>>,
    }

    let parsed: Frontmatter = serde_yaml::from_str(yaml).ok()?;
    parsed.paths
}

/// Load all instruction files starting from `workspace` and walking up to the
/// git root.
///
/// Files are returned in priority order: deepest (closest to workspace) first,
/// shallowest (closest to git root) last. The `.dolphin-milk/INSTRUCTIONS.md`
/// file in the workspace directory is checked first (highest priority).
pub fn load_instructions(workspace: &Path) -> Vec<InstructionFile> {
    let git_root = find_git_root(workspace);
    let mut files = Vec::new();

    // 1. Check .dolphin-milk/INSTRUCTIONS.md in workspace (highest priority)
    let dm_instructions = workspace.join(".dolphin-milk").join("INSTRUCTIONS.md");
    if let Some(entry) = try_load_instruction_file(&dm_instructions) {
        files.push(entry);
    }

    // 2. Walk from workspace up to git root, collecting DOLPHIN-MILK.md files
    let mut current = workspace.to_path_buf();
    let stop_at = git_root.as_deref();

    loop {
        let candidate = current.join("DOLPHIN-MILK.md");
        if let Some(entry) = try_load_instruction_file(&candidate) {
            files.push(entry);
        }

        // Stop if we've reached the git root
        if let Some(root) = stop_at {
            if current == root {
                break;
            }
        }

        // Move up one directory
        if !current.pop() {
            break;
        }

        // Don't go above git root
        if let Some(root) = stop_at {
            if !current.starts_with(root) && current != root {
                break;
            }
        }
    }

    files
}

/// Try to read and parse a single instruction file.
fn try_load_instruction_file(path: &Path) -> Option<InstructionFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (paths, content) = parse_frontmatter(&raw);
    Some(InstructionFile {
        path: path.to_path_buf(),
        content,
        paths,
    })
}

/// Combine loaded instruction files into a single string for prompt injection.
///
/// Files are joined with headers showing their source path.
pub fn format_instructions(files: &[InstructionFile]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for f in files {
        let header = format!("<!-- from {} -->", f.path.display());
        parts.push(format!("{}\n{}", header, f.content));
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_none() {
        let (paths, body) = parse_frontmatter("Hello world");
        assert!(paths.is_none());
        assert_eq!(body, "Hello world");
    }

    #[test]
    fn test_parse_frontmatter_with_paths() {
        let raw = "---\npaths:\n  - \"src/**/*.rs\"\n  - \"tests/**\"\n---\nBody here";
        let (paths, body) = parse_frontmatter(raw);
        assert_eq!(paths.unwrap(), vec!["src/**/*.rs", "tests/**"]);
        assert_eq!(body, "Body here");
    }

    #[test]
    fn test_parse_frontmatter_no_closing() {
        let raw = "---\npaths:\n  - foo\nNo closing delimiter";
        let (paths, body) = parse_frontmatter(raw);
        assert!(paths.is_none());
        assert_eq!(body, raw);
    }

    #[test]
    fn test_format_instructions_empty() {
        assert_eq!(format_instructions(&[]), "");
    }

    #[test]
    fn test_format_instructions_single() {
        let files = vec![InstructionFile {
            path: PathBuf::from("/tmp/DOLPHIN-MILK.md"),
            content: "Do X".to_string(),
            paths: None,
        }];
        let result = format_instructions(&files);
        assert!(result.contains("/tmp/DOLPHIN-MILK.md"));
        assert!(result.contains("Do X"));
    }
}
