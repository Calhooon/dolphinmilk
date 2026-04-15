//! Tests for multi-source config precedence (#277).
//!
//! Covers: default config, single-file loading, multi-file deep merge,
//! env var overrides, array concatenation/dedup, ConfigSources tracking,
//! and backward compatibility with existing load_config().
//!
//! IMPORTANT: Environment variables are process-global. Tests that set
//! HOME or DOLPHIN_MILK_* vars can race when run in parallel. To avoid
//! flakiness, most tests here:
//!   - Use fields not affected by common env var overrides (e.g., wallet.origin)
//!   - Test merge logic via TOML parsing rather than full load_config_with_precedence
//!   - Avoid depending on HOME for user-config layer tests
//!
//! Tests that must set HOME are clearly marked and accept the inherent race risk.

use std::io::Write;
use std::sync::Mutex;
use tempfile::TempDir;

use dolphin_milk::config::{load_config, load_config_with_precedence, DmConfig};

/// Mutex to serialize tests that modify HOME. Not a perfect solution since
/// other test files can also modify HOME, but it prevents intra-file races.
static HOME_MUTEX: Mutex<()> = Mutex::new(());

// ===========================================================================
// Helper: write a TOML file at a given path
// ===========================================================================

fn write_toml(dir: &std::path::Path, filename: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

/// Run a closure with HOME set to a specific directory. Serializes via HOME_MUTEX.
fn with_home<F, R>(home_dir: &std::path::Path, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _lock = HOME_MUTEX.lock().unwrap();
    let saved_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", home_dir.to_str().unwrap());
    let result = f();
    match saved_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    result
}

// ===========================================================================
// 1. Default config when no files exist
// ===========================================================================

#[test]
fn test_precedence_default_config_no_files() {
    let dir = TempDir::new().unwrap();
    let bogus = dir.path().join("nonexistent.toml");

    // Use empty HOME so user config is not loaded
    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&bogus)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://localhost");
    assert_eq!(config.wallet.timeout, 120);
    assert_eq!(config.budget.max_per_task, 20_000_000);
    assert!(sources.files_loaded.is_empty());
}

// ===========================================================================
// 2. Single file (instance config) loads correctly
// ===========================================================================

#[test]
fn test_precedence_single_instance_config() {
    let dir = TempDir::new().unwrap();
    let instance = write_toml(
        dir.path(),
        "dolphin-milk.toml",
        r#"
[wallet]
origin = "http://instance-origin"
timeout = 45

[budget]
max_per_task = 5000000
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://instance-origin");
    assert_eq!(config.wallet.timeout, 45);
    assert_eq!(config.budget.max_per_task, 5_000_000);
    assert_eq!(config.budget.max_per_hour, 50_000_000);
    assert_eq!(sources.files_loaded.len(), 1);
    assert_eq!(sources.files_loaded[0], instance);
}

// ===========================================================================
// 3. User config merges with defaults (direct TOML parsing, no env vars)
// ===========================================================================

#[test]
fn test_precedence_user_config_merges_with_defaults() {
    let user_toml = r#"
[logging]
level = "DEBUG"

[budget]
max_per_day = 999999
"#;
    let config: DmConfig = toml::from_str(user_toml).unwrap();

    assert_eq!(config.logging.level, "DEBUG");
    assert_eq!(config.budget.max_per_day, 999_999);
    assert_eq!(config.wallet.origin, "http://localhost");
}

// ===========================================================================
// 4. Project config overrides user config
// ===========================================================================

#[test]
fn test_precedence_project_overrides_user() {
    let dir = TempDir::new().unwrap();

    let fake_home = dir.path().join("home");
    write_toml(
        &fake_home,
        ".dolphin-milk/config.toml",
        r#"
[logging]
level = "DEBUG"

[wallet]
origin = "http://user-config"
"#,
    );

    let workspace = dir.path().join("project");
    write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        r#"
[logging]
level = "TRACE"
"#,
    );

    let bogus = dir.path().join("nonexistent.toml");

    let (config, sources) = with_home(&fake_home, || {
        load_config_with_precedence(Some(&workspace), Some(&bogus)).unwrap()
    });

    assert_eq!(config.logging.level, "TRACE");
    assert_eq!(config.wallet.origin, "http://user-config");
    assert_eq!(sources.files_loaded.len(), 2);
}

// ===========================================================================
// 5. Env vars override everything
// ===========================================================================

#[test]
fn test_precedence_env_overrides_all() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "dolphin-milk.toml",
        r#"
[logging]
level = "WARN"
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let saved = std::env::var("DOLPHIN_MILK_LOG_LEVEL").ok();
    std::env::set_var("DOLPHIN_MILK_LOG_LEVEL", "TRACE");

    let (config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&instance)).unwrap()
    });

    match saved {
        Some(v) => std::env::set_var("DOLPHIN_MILK_LOG_LEVEL", v),
        None => std::env::remove_var("DOLPHIN_MILK_LOG_LEVEL"),
    }

    assert_eq!(config.logging.level, "TRACE");
    assert!(sources
        .env_overrides
        .contains(&"DOLPHIN_MILK_LOG_LEVEL".to_string()));
}

// ===========================================================================
// 6. Deep merge: nested section fields merge correctly
// ===========================================================================

#[test]
fn test_precedence_deep_merge_nested_sections() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "instance.toml",
        r#"
[wallet]
origin = "http://instance-origin"
timeout = 60

[budget]
max_per_task = 1000000
"#,
    );

    let workspace = dir.path().join("project");
    write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        r#"
[wallet]
timeout = 120
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, _sources) = with_home(&empty_home, || {
        load_config_with_precedence(Some(&workspace), Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://instance-origin");
    assert_eq!(config.wallet.timeout, 120);
    assert_eq!(config.budget.max_per_task, 1_000_000);
}

// ===========================================================================
// 7. Array fields: concatenate and deduplicate
// ===========================================================================

#[test]
fn test_precedence_array_concatenate_dedup() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "instance.toml",
        r#"
[tool_approval]
require = ["execute_bash", "file_write"]
"#,
    );

    let workspace = dir.path().join("project");
    write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        r#"
[tool_approval]
require = ["file_write", "web_fetch"]
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, _sources) = with_home(&empty_home, || {
        load_config_with_precedence(Some(&workspace), Some(&instance)).unwrap()
    });

    assert!(config
        .tool_approval
        .require
        .contains(&"execute_bash".to_string()));
    assert!(config
        .tool_approval
        .require
        .contains(&"file_write".to_string()));
    assert!(config
        .tool_approval
        .require
        .contains(&"web_fetch".to_string()));
    assert_eq!(
        config
            .tool_approval
            .require
            .iter()
            .filter(|t| t.as_str() == "file_write")
            .count(),
        1,
        "file_write should appear exactly once (deduplication)"
    );
}

// ===========================================================================
// 8. Missing project dir is not an error
// ===========================================================================

#[test]
fn test_precedence_missing_project_dir_ok() {
    let dir = TempDir::new().unwrap();
    let bogus_instance = dir.path().join("nonexistent.toml");

    let workspace = dir.path().join("no_project_config");
    std::fs::create_dir_all(&workspace).unwrap();

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let result = with_home(&empty_home, || {
        load_config_with_precedence(Some(&workspace), Some(&bogus_instance))
    });
    assert!(
        result.is_ok(),
        "missing project config dir should not be an error"
    );
}

// ===========================================================================
// 9. ConfigSources tracks loaded files
// ===========================================================================

#[test]
fn test_config_sources_tracks_files() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "instance.toml",
        "[wallet]\norigin = \"http://track\"",
    );

    let workspace = dir.path().join("project");
    let project_file = write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        "[budget]\nmax_per_task = 100",
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (_config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(Some(&workspace), Some(&instance)).unwrap()
    });

    assert_eq!(sources.files_loaded.len(), 2);
    assert_eq!(sources.files_loaded[0], instance);
    assert_eq!(sources.files_loaded[1], project_file);
}

// ===========================================================================
// 10. ConfigSources tracks env overrides
// ===========================================================================

#[test]
fn test_config_sources_tracks_env_overrides() {
    let dir = TempDir::new().unwrap();
    let bogus = dir.path().join("nonexistent.toml");

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let saved = std::env::var("DOLPHIN_MILK_BROWSER_HEADLESS").ok();
    std::env::set_var("DOLPHIN_MILK_BROWSER_HEADLESS", "false");

    let (_config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&bogus)).unwrap()
    });

    match saved {
        Some(v) => std::env::set_var("DOLPHIN_MILK_BROWSER_HEADLESS", v),
        None => std::env::remove_var("DOLPHIN_MILK_BROWSER_HEADLESS"),
    }

    assert!(
        sources
            .env_overrides
            .contains(&"DOLPHIN_MILK_BROWSER_HEADLESS".to_string()),
        "ConfigSources should include DOLPHIN_MILK_BROWSER_HEADLESS in env_overrides"
    );
}

// ===========================================================================
// 11. Backward compat: existing load_config behavior unchanged
// ===========================================================================

#[test]
fn test_backward_compat_load_config_still_works() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[wallet]
origin = "http://compat-origin"
timeout = 99
"#
        )
        .unwrap();
    }

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let cfg = with_home(&empty_home, || load_config(Some(path.as_path())).unwrap());

    assert_eq!(cfg.wallet.origin, "http://compat-origin");
    assert_eq!(cfg.wallet.timeout, 99);
    assert_eq!(cfg.budget.max_per_task, 20_000_000);
}

#[test]
fn test_backward_compat_load_config_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("does_not_exist.toml");

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let cfg = with_home(&empty_home, || load_config(Some(path.as_path())).unwrap());
    assert_eq!(cfg.wallet.timeout, 120);
}

#[test]
fn test_backward_compat_load_config_invalid_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "this is not valid toml [[[").unwrap();

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let result = with_home(&empty_home, || load_config(Some(path.as_path())));
    assert!(result.is_err());
}

// ===========================================================================
// 12. Three-layer merge: instance < user < project
// ===========================================================================

#[test]
fn test_precedence_three_layer_merge() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "instance.toml",
        r#"
[wallet]
origin = "http://instance-origin"

[budget]
staging_threshold = 1000

[logging]
format = "text"
"#,
    );

    let fake_home = dir.path().join("home");
    write_toml(
        &fake_home,
        ".dolphin-milk/config.toml",
        r#"
[budget]
staging_threshold = 2000

[heartbeat]
enabled = false
"#,
    );

    let workspace = dir.path().join("project");
    write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        r#"
[logging]
format = "compact"
"#,
    );

    let (config, sources) = with_home(&fake_home, || {
        load_config_with_precedence(Some(&workspace), Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://instance-origin");
    assert_eq!(config.budget.staging_threshold, 2000);
    assert_eq!(config.logging.format, "compact");
    assert!(!config.heartbeat.enabled);
    assert_eq!(sources.files_loaded.len(), 3);
}

// ===========================================================================
// 13. Empty config files are valid
// ===========================================================================

#[test]
fn test_precedence_empty_config_files_ok() {
    let dir = TempDir::new().unwrap();
    let instance = write_toml(dir.path(), "empty.toml", "");

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.timeout, 120);
    assert_eq!(sources.files_loaded.len(), 1);
}

// ===========================================================================
// 14. Invalid config file returns error
// ===========================================================================

#[test]
fn test_precedence_invalid_config_file_returns_error() {
    let dir = TempDir::new().unwrap();
    let bad = write_toml(dir.path(), "bad.toml", "this is not valid toml [[[");

    let result = load_config_with_precedence(None, Some(&bad));
    assert!(result.is_err());
}

// ===========================================================================
// 15. Workspace is None: project config skipped gracefully
// ===========================================================================

#[test]
fn test_precedence_no_workspace_skips_project() {
    let dir = TempDir::new().unwrap();
    let instance = write_toml(
        dir.path(),
        "dolphin-milk.toml",
        "[wallet]\norigin = \"http://instance-only\"",
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://instance-only");
    assert!(sources.files_loaded.contains(&instance));
}

// ===========================================================================
// 16. Instance config with partial sections: other sections get defaults
// ===========================================================================

#[test]
fn test_precedence_partial_sections_get_defaults() {
    let dir = TempDir::new().unwrap();
    let instance = write_toml(
        dir.path(),
        "partial.toml",
        r#"
[wallet]
origin = "http://custom-origin"
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, _) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://custom-origin");
    assert_eq!(config.wallet.timeout, 120);
    assert_eq!(config.budget.max_per_task, 20_000_000);
    assert!(config.heartbeat.enabled);
}

// ===========================================================================
// 17. User config only (no instance, no project)
// ===========================================================================

#[test]
fn test_precedence_user_config_only() {
    let dir = TempDir::new().unwrap();

    let fake_home = dir.path().join("home");
    write_toml(
        &fake_home,
        ".dolphin-milk/config.toml",
        r#"
[llm]
max_tokens = 8192
context_window = 64000
"#,
    );

    let bogus = dir.path().join("nonexistent.toml");

    let (config, sources) = with_home(&fake_home, || {
        load_config_with_precedence(None, Some(&bogus)).unwrap()
    });

    assert_eq!(config.llm.max_tokens, 8192);
    assert_eq!(config.llm.context_window, 64000);
    assert_eq!(sources.files_loaded.len(), 1);
}

// ===========================================================================
// 18. ConfigSources env_overrides is sorted
// ===========================================================================

#[test]
fn test_config_sources_env_overrides_sorted() {
    let dir = TempDir::new().unwrap();
    let bogus = dir.path().join("nonexistent.toml");

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (_config, sources) = with_home(&empty_home, || {
        load_config_with_precedence(None, Some(&bogus)).unwrap()
    });

    let mut sorted = sources.env_overrides.clone();
    sorted.sort();
    assert_eq!(
        sources.env_overrides, sorted,
        "env_overrides should be sorted"
    );
}

// ===========================================================================
// 19. Deep merge: compliance regulations array from multiple sources
// ===========================================================================

#[test]
fn test_precedence_compliance_regulations_merge() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "instance.toml",
        r#"
[compliance]
regulations = ["SEC-17a-4"]
"#,
    );

    let workspace = dir.path().join("project");
    write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        r#"
[compliance]
regulations = ["SOX", "SEC-17a-4"]
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, _sources) = with_home(&empty_home, || {
        load_config_with_precedence(Some(&workspace), Some(&instance)).unwrap()
    });

    assert!(config
        .compliance
        .regulations
        .contains(&"SEC-17a-4".to_string()));
    assert!(config.compliance.regulations.contains(&"SOX".to_string()));
    assert_eq!(
        config
            .compliance
            .regulations
            .iter()
            .filter(|r| r.as_str() == "SEC-17a-4")
            .count(),
        1
    );
}

// ===========================================================================
// 20. Instance + project: project adds new section not in instance
// ===========================================================================

#[test]
fn test_precedence_project_adds_new_section() {
    let dir = TempDir::new().unwrap();

    let instance = write_toml(
        dir.path(),
        "instance.toml",
        r#"
[wallet]
origin = "http://instance-origin"
"#,
    );

    let workspace = dir.path().join("project");
    write_toml(
        &workspace,
        ".dolphin-milk/config.toml",
        r#"
[browser]
page_load_timeout = 99
max_pages = 10
"#,
    );

    let empty_home = dir.path().join("empty");
    std::fs::create_dir_all(&empty_home).unwrap();

    let (config, _sources) = with_home(&empty_home, || {
        load_config_with_precedence(Some(&workspace), Some(&instance)).unwrap()
    });

    assert_eq!(config.wallet.origin, "http://instance-origin");
    assert_eq!(config.browser.page_load_timeout, 99);
    assert_eq!(config.browser.max_pages, 10);
}
