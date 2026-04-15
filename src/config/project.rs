//! Project detection — scans for `.dolphin-milk/` directory in a workspace.
//!
//! When the agent runs in a workspace containing a `.dolphin-milk/` directory,
//! it adapts to the project's conventions. This module provides detection and
//! inventory of available project-level configuration components.
//!
//! Components detected:
//! - `config.toml` — project-level config (loaded by `config/precedence.rs`)
//! - `INSTRUCTIONS.md` — project instructions (loaded by `context/instructions.rs`)
//! - `commands/` — project commands (loaded by `skills/commands.rs`)
//! - `skills/` — project skills (loaded by `skills/mod.rs`)

use std::path::{Path, PathBuf};

/// Inventory of available project-level configuration components.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// Root of the `.dolphin-milk/` directory (absolute or relative).
    pub root: PathBuf,
    /// Whether `.dolphin-milk/config.toml` exists.
    pub has_config: bool,
    /// Whether `.dolphin-milk/INSTRUCTIONS.md` exists.
    pub has_instructions: bool,
    /// Whether `.dolphin-milk/commands/` directory exists.
    pub has_commands: bool,
    /// Whether `.dolphin-milk/skills/` directory exists.
    pub has_skills: bool,
}

/// The project configuration directory name.
pub const PROJECT_DIR_NAME: &str = ".dolphin-milk";

/// Detect a `.dolphin-milk/` directory in the given workspace.
///
/// Returns `Some(ProjectConfig)` if the directory exists, with flags indicating
/// which components are available. Returns `None` if the directory does not exist.
pub fn detect_project(workspace: &Path) -> Option<ProjectConfig> {
    let root = workspace.join(PROJECT_DIR_NAME);
    if !root.is_dir() {
        return None;
    }

    Some(ProjectConfig {
        has_config: root.join("config.toml").is_file(),
        has_instructions: root.join("INSTRUCTIONS.md").is_file(),
        has_commands: root.join("commands").is_dir(),
        has_skills: root.join("skills").is_dir(),
        root,
    })
}

impl ProjectConfig {
    /// Path to the project commands directory: `.dolphin-milk/commands/`.
    pub fn commands_dir(&self) -> PathBuf {
        self.root.join("commands")
    }

    /// Path to the project skills directory: `.dolphin-milk/skills/`.
    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// Count how many components are available.
    pub fn component_count(&self) -> usize {
        let mut count = 0;
        if self.has_config {
            count += 1;
        }
        if self.has_instructions {
            count += 1;
        }
        if self.has_commands {
            count += 1;
        }
        if self.has_skills {
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_project_no_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(detect_project(dir.path()).is_none());
    }

    #[test]
    fn test_detect_project_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(PROJECT_DIR_NAME)).unwrap();

        let config = detect_project(dir.path()).unwrap();
        assert!(!config.has_config);
        assert!(!config.has_instructions);
        assert!(!config.has_commands);
        assert!(!config.has_skills);
        assert_eq!(config.component_count(), 0);
    }

    #[test]
    fn test_detect_project_with_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let dm = dir.path().join(PROJECT_DIR_NAME);
        std::fs::create_dir_all(&dm).unwrap();
        std::fs::write(dm.join("config.toml"), "[budget]\nmax_per_task = 100").unwrap();

        let config = detect_project(dir.path()).unwrap();
        assert!(config.has_config);
        assert!(!config.has_instructions);
        assert!(!config.has_commands);
        assert!(!config.has_skills);
        assert_eq!(config.component_count(), 1);
    }

    #[test]
    fn test_detect_project_full() {
        let dir = tempfile::TempDir::new().unwrap();
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
}
