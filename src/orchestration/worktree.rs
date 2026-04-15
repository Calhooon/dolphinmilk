//! Git worktree isolation for sub-agents.
//!
//! Provides isolated working copies of the repository for sub-agents
//! via `git worktree`. Each agent gets its own branch and directory.

use std::path::{Path, PathBuf};

/// Result of creating a worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Path to the worktree directory.
    pub path: PathBuf,
    /// Branch name created for this worktree.
    pub branch: String,
    /// Whether the worktree has uncommitted changes.
    pub has_changes: bool,
}

/// Isolation mode for sub-agent execution.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum IsolationMode {
    /// Run in the same workspace (default).
    #[default]
    InProcess,
    /// Run in a separate git worktree.
    Worktree,
}

/// Validate a worktree slug (branch name component).
/// Must be 1-64 chars, alphanumeric + hyphens only.
pub fn validate_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err("Worktree slug cannot be empty".to_string());
    }
    if slug.len() > 64 {
        return Err(format!(
            "Worktree slug too long: {} chars (max 64)",
            slug.len()
        ));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Worktree slug '{}' contains invalid characters (only a-z, 0-9, -, _ allowed)",
            slug
        ));
    }
    Ok(())
}

/// Create a git worktree for isolated agent execution.
///
/// Creates a new branch and worktree at `{base_dir}/worktrees/{slug}`.
/// Returns the worktree info on success.
pub async fn create_worktree(
    repo_dir: &Path,
    base_dir: &Path,
    slug: &str,
) -> Result<WorktreeInfo, String> {
    validate_slug(slug)?;

    let branch = format!("dolphin-milk-agent-{slug}");
    let worktree_path = base_dir.join("worktrees").join(slug);

    // Create parent directory
    std::fs::create_dir_all(worktree_path.parent().unwrap_or(base_dir))
        .map_err(|e| format!("Failed to create worktree parent: {e}"))?;

    // git worktree add -B <branch> <path> HEAD
    let output = tokio::process::Command::new("git")
        .args([
            "worktree",
            "add",
            "-B",
            &branch,
            worktree_path.to_str().unwrap_or(""),
            "HEAD",
        ])
        .current_dir(repo_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to run git worktree: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {stderr}"));
    }

    Ok(WorktreeInfo {
        path: worktree_path,
        branch,
        has_changes: false,
    })
}

/// Check if a worktree has uncommitted changes.
pub async fn has_changes(worktree_path: &Path) -> bool {
    let output = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output()
        .await;

    match output {
        Ok(o) => !o.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Clean up a worktree. If it has no changes, removes it entirely.
/// If it has changes, keeps it and returns the branch info.
pub async fn cleanup_worktree(
    repo_dir: &Path,
    worktree_path: &Path,
) -> Result<Option<WorktreeInfo>, String> {
    let changes = has_changes(worktree_path).await;

    if changes {
        // Keep the worktree — it has uncommitted work
        let branch = get_branch_name(worktree_path).await.unwrap_or_default();
        return Ok(Some(WorktreeInfo {
            path: worktree_path.to_path_buf(),
            branch,
            has_changes: true,
        }));
    }

    // Remove the worktree
    let output = tokio::process::Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap_or(""),
        ])
        .current_dir(repo_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to remove worktree: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("git worktree remove failed: {stderr}");
    }

    Ok(None)
}

/// Get the current branch name for a worktree.
async fn get_branch_name(worktree_path: &Path) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
