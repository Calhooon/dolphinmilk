//! Tests for git worktree isolation.

use dolphin_milk::orchestration::worktree::{validate_slug, IsolationMode};

// ─── validate_slug ────────────────────────────────────────────────────────────

#[test]
fn valid_slug_alphanumeric() {
    assert!(validate_slug("my-agent-1").is_ok());
}

#[test]
fn valid_slug_underscores() {
    assert!(validate_slug("research_agent").is_ok());
}

#[test]
fn valid_slug_single_char() {
    assert!(validate_slug("a").is_ok());
}

#[test]
fn valid_slug_max_length() {
    let slug: String = "a".repeat(64);
    assert!(validate_slug(&slug).is_ok());
}

#[test]
fn invalid_slug_empty() {
    assert!(validate_slug("").is_err());
    assert!(validate_slug("").unwrap_err().contains("empty"));
}

#[test]
fn invalid_slug_too_long() {
    let slug: String = "a".repeat(65);
    assert!(validate_slug(&slug).is_err());
    assert!(validate_slug(&slug).unwrap_err().contains("too long"));
}

#[test]
fn invalid_slug_spaces() {
    assert!(validate_slug("my agent").is_err());
    assert!(validate_slug("my agent")
        .unwrap_err()
        .contains("invalid characters"));
}

#[test]
fn invalid_slug_dots() {
    assert!(validate_slug("my.agent").is_err());
}

#[test]
fn invalid_slug_slashes() {
    assert!(validate_slug("my/agent").is_err());
}

#[test]
fn invalid_slug_special_chars() {
    assert!(validate_slug("agent@1").is_err());
    assert!(validate_slug("agent!").is_err());
    assert!(validate_slug("agent#2").is_err());
}

// ─── IsolationMode ────────────────────────────────────────────────────────────

#[test]
fn isolation_mode_default_is_in_process() {
    assert_eq!(IsolationMode::default(), IsolationMode::InProcess);
}

#[test]
fn isolation_mode_equality() {
    assert_eq!(IsolationMode::InProcess, IsolationMode::InProcess);
    assert_eq!(IsolationMode::Worktree, IsolationMode::Worktree);
    assert_ne!(IsolationMode::InProcess, IsolationMode::Worktree);
}
