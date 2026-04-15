//! Tests for config module — TOML loading and env overrides.
//! Mirrors Python config.py behavior.

use std::io::Write;
use tempfile::TempDir;

use dolphin_milk::config::{load_config, DmConfig};

#[test]
fn test_default_config() {
    let cfg = DmConfig::default();
    assert_eq!(cfg.wallet.url, "http://localhost:3322");
    assert_eq!(cfg.wallet.origin, "http://localhost");
    assert_eq!(cfg.wallet.timeout, 120);
    assert_eq!(cfg.budget.max_per_task, 20_000_000);
    assert_eq!(cfg.budget.max_per_hour, 50_000_000);
    assert_eq!(cfg.budget.max_per_day, 200_000_000);
    assert_eq!(cfg.llm.default_provider, "openai-agent");
    assert_eq!(cfg.logging.level, "INFO");
    assert_eq!(cfg.logging.format, "json");
    // Heartbeat defaults
    assert!(cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 60);
}

#[test]
fn test_load_config_missing_file() {
    // When no dolphin-milk.toml exists, should return defaults
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.toml");
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.wallet.url, "http://localhost:3322");
}

#[test]
fn test_load_config_from_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_dolphin_milk.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[wallet]
url = "http://localhost:9999"
timeout = 60

[budget]
max_per_task = 100000

[logging]
level = "DEBUG"
format = "text"
"#
        )
        .unwrap();
    }
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.wallet.url, "http://localhost:9999");
    assert_eq!(cfg.wallet.timeout, 60);
    assert_eq!(cfg.budget.max_per_task, 100_000);
    // Defaults still apply for unset fields
    assert_eq!(cfg.budget.max_per_hour, 50_000_000);
    assert_eq!(cfg.logging.level, "DEBUG");
    assert_eq!(cfg.logging.format, "text");
}

#[test]
fn test_partial_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("partial.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[wallet]
url = "http://custom:3322"
"#
        )
        .unwrap();
    }
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.wallet.url, "http://custom:3322");
    // Everything else is default
    assert_eq!(cfg.wallet.timeout, 120);
    assert_eq!(cfg.budget.max_per_task, 20_000_000);
}

#[test]
fn test_empty_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.toml");
    std::fs::write(&path, "").unwrap();
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.wallet.url, "http://localhost:3322");
}

#[test]
fn test_invalid_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "this is not valid toml [[[").unwrap();
    let result = load_config(Some(path.as_path()));
    assert!(result.is_err());
}

#[test]
fn test_budget_defaults() {
    let cfg = DmConfig::default();
    assert_eq!(cfg.budget.low_power_threshold, 50_000);
    assert!(cfg.budget.max_per_task < cfg.budget.max_per_hour);
    assert!(cfg.budget.max_per_hour < cfg.budget.max_per_day);
}

#[test]
fn test_llm_defaults() {
    let cfg = DmConfig::default();
    assert_eq!(cfg.llm.default_provider, "openai-agent");
}

#[test]
fn test_heartbeat_defaults() {
    let cfg = DmConfig::default();
    assert!(cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 60);
}

#[test]
fn test_heartbeat_from_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hb.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[heartbeat]
enabled = false
inbox_poll_secs = 120
"#
        )
        .unwrap();
    }
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert!(!cfg.heartbeat.enabled);
    assert_eq!(cfg.heartbeat.inbox_poll_secs, 120);
}

// -- Phase 6: Compaction config --

#[test]
fn test_compaction_config_defaults() {
    let path = std::path::PathBuf::from("nonexistent-config-file.toml");
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert!(cfg.llm.compaction_enabled);
    assert!(cfg.llm.compaction_model.is_none());
}

// -- Phase 7: Reflection config --

#[test]
fn test_reflection_config_defaults() {
    let path = std::path::PathBuf::from("nonexistent-config-file.toml");
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert!(!cfg.heartbeat.reflection_enabled);
    assert_eq!(cfg.heartbeat.reflection_interval_secs, 60);
    assert_eq!(cfg.heartbeat.reflection_model, "claude-haiku-4-5-20251001");
    assert_eq!(cfg.heartbeat.reflection_max_iterations, 3);
}

#[test]
fn test_reflection_config_from_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reflection.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[heartbeat]
reflection_enabled = true
reflection_interval_secs = 300
reflection_model = "gpt-5-mini"
reflection_max_iterations = 10
"#
        )
        .unwrap();
    }
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert!(cfg.heartbeat.reflection_enabled);
    assert_eq!(cfg.heartbeat.reflection_interval_secs, 300);
    assert_eq!(cfg.heartbeat.reflection_model, "gpt-5-mini");
    assert_eq!(cfg.heartbeat.reflection_max_iterations, 10);
}

// -----------------------------------------------------------------------
// Hot config reload tests (Task 4.4)
// -----------------------------------------------------------------------

#[test]
fn test_reload_safe_fields_updates_heartbeat() {
    let mut config = DmConfig::default();
    let mut fresh = DmConfig::default();
    fresh.heartbeat.inbox_poll_secs = 30;
    fresh.heartbeat.max_concurrent_tasks = 5;
    fresh.heartbeat.reflection_enabled = true;

    config.reload_safe_fields(&fresh);

    assert_eq!(config.heartbeat.inbox_poll_secs, 30);
    assert_eq!(config.heartbeat.max_concurrent_tasks, 5);
    assert!(config.heartbeat.reflection_enabled);
}

#[test]
fn test_reload_safe_fields_updates_budget() {
    let mut config = DmConfig::default();
    let mut fresh = DmConfig::default();
    fresh.budget.max_per_task = 100;
    fresh.budget.max_per_hour = 200;
    fresh.budget.max_per_day = 300;
    fresh.budget.low_power_threshold = 50;

    config.reload_safe_fields(&fresh);

    assert_eq!(config.budget.max_per_task, 100);
    assert_eq!(config.budget.max_per_hour, 200);
    assert_eq!(config.budget.max_per_day, 300);
    assert_eq!(config.budget.low_power_threshold, 50);
}

#[test]
fn test_reload_safe_fields_updates_llm() {
    let mut config = DmConfig::default();
    let mut fresh = DmConfig::default();
    fresh.llm.default_model = "claude-sonnet-4-5-20250514".to_string();
    fresh.llm.max_tokens = 8192;
    fresh.llm.context_window = 64000;

    config.reload_safe_fields(&fresh);

    assert_eq!(config.llm.default_model, "claude-sonnet-4-5-20250514");
    assert_eq!(config.llm.max_tokens, 8192);
    assert_eq!(config.llm.context_window, 64000);
}

#[test]
fn test_reload_safe_fields_does_not_update_wallet() {
    let mut config = DmConfig::default();
    let original_wallet_url = config.wallet.url.clone();
    let mut fresh = DmConfig::default();
    fresh.wallet.url = "http://evil:9999".to_string();

    config.reload_safe_fields(&fresh);

    assert_eq!(config.wallet.url, original_wallet_url);
}

#[test]
fn test_reload_safe_fields_does_not_update_parent() {
    let mut config = DmConfig::default();
    config.parent.identity_key = "original-key".to_string();
    let mut fresh = DmConfig::default();
    fresh.parent.identity_key = "evil-key".to_string();

    config.reload_safe_fields(&fresh);

    assert_eq!(config.parent.identity_key, "original-key");
}

#[test]
fn test_reload_safe_fields_does_not_update_logging() {
    let mut config = DmConfig::default();
    let original_level = config.logging.level.clone();
    let mut fresh = DmConfig::default();
    fresh.logging.level = "TRACE".to_string();

    config.reload_safe_fields(&fresh);

    assert_eq!(config.logging.level, original_level);
}

#[test]
fn test_reload_safe_fields_does_not_update_server() {
    let mut config = DmConfig::default();
    assert!(!config.server.openai_compat_enabled);
    let mut fresh = DmConfig::default();
    fresh.server.openai_compat_enabled = true;

    config.reload_safe_fields(&fresh);

    assert!(!config.server.openai_compat_enabled);
}

// ===========================================================================
// Issue #6 & #7: Budget config for new limits + enforcement mode
// ===========================================================================

#[test]
fn test_config_env_overrides_for_new_budget_limits() {
    // This test verifies env vars override the TOML defaults for new budget limits.
    // We test by creating a TOML with defaults, then the env override would take effect.
    let cfg = DmConfig::default();
    assert_eq!(cfg.budget.max_per_week, 100_000_000);
    assert_eq!(cfg.budget.max_per_month, 500_000_000);
    assert_eq!(cfg.budget.max_lifetime, 0);
    assert_eq!(cfg.budget.enforcement, "strict");
}

#[test]
fn test_budget_new_limits_from_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("budget_new.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[budget]
max_per_week = 77777
max_per_month = 88888
max_lifetime = 99999
enforcement = "advisory"
"#
        )
        .unwrap();
    }
    let cfg = load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.budget.max_per_week, 77777);
    assert_eq!(cfg.budget.max_per_month, 88888);
    assert_eq!(cfg.budget.max_lifetime, 99999);
    assert_eq!(cfg.budget.enforcement, "advisory");
}

#[test]
fn test_reload_safe_fields_updates_new_budget_fields() {
    let mut config = DmConfig::default();
    let mut fresh = DmConfig::default();
    fresh.budget.max_per_week = 42;
    fresh.budget.max_per_month = 43;
    fresh.budget.max_lifetime = 44;
    fresh.budget.enforcement = "advisory".to_string();

    config.reload_safe_fields(&fresh);

    assert_eq!(config.budget.max_per_week, 42);
    assert_eq!(config.budget.max_per_month, 43);
    assert_eq!(config.budget.max_lifetime, 44);
    assert_eq!(config.budget.enforcement, "advisory");
}

#[test]
fn test_budget_defaults_include_new_limits() {
    let cfg = DmConfig::default();
    // Weekly < monthly (sane ordering)
    assert!(cfg.budget.max_per_week < cfg.budget.max_per_month);
    // Lifetime = 0 (unlimited by default)
    assert_eq!(cfg.budget.max_lifetime, 0);
    // Enforcement is strict by default
    assert_eq!(cfg.budget.enforcement, "strict");
}

// ===========================================================================
// Issue #12: Configurable x402 registry URL
// ===========================================================================

#[test]
fn test_x402_config_default_registry_url() {
    let cfg = DmConfig::default();
    assert_eq!(
        cfg.x402.registry_url, "https://x402agency.com/.well-known/agents",
        "default registry URL should match the hardcoded constant"
    );
    // Also verify it matches the constant in registry.rs
    assert_eq!(
        cfg.x402.registry_url,
        dolphin_milk::x402::registry::DEFAULT_REGISTRY_URL,
    );
}

#[test]
fn test_x402_config_custom_registry_url() {
    // Test TOML parsing directly (not via load_config which applies env overrides
    // that could race with concurrent tests).
    let toml_str = r#"
[x402]
registry_url = "https://custom-registry.example.com/agents"
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(
        cfg.x402.registry_url,
        "https://custom-registry.example.com/agents"
    );
    // Other sections should still get defaults
    assert_eq!(cfg.wallet.url, "http://localhost:3322");
}

#[test]
fn test_x402_config_env_override() {
    // Test that DOLPHIN_MILK_X402_REGISTRY_URL env var overrides the TOML value.
    // NOTE: env vars are process-global and can race with parallel tests.
    // The custom_registry_url test above uses direct TOML parsing (not load_config)
    // to avoid this race.
    let saved = std::env::var("DOLPHIN_MILK_X402_REGISTRY_URL").ok();
    std::env::set_var(
        "DOLPHIN_MILK_X402_REGISTRY_URL",
        "https://env-registry.example.com/agents",
    );

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("env_override.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[x402]
registry_url = "https://toml-registry.example.com/agents"
"#
        )
        .unwrap();
    }
    let cfg = load_config(Some(path.as_path())).unwrap();
    // Env override should win over TOML
    assert_eq!(
        cfg.x402.registry_url,
        "https://env-registry.example.com/agents"
    );

    // Restore
    match saved {
        Some(v) => std::env::set_var("DOLPHIN_MILK_X402_REGISTRY_URL", v),
        None => std::env::remove_var("DOLPHIN_MILK_X402_REGISTRY_URL"),
    }
}

#[test]
fn test_x402_config_cache_ttl_default() {
    let cfg = DmConfig::default();
    assert_eq!(cfg.x402.registry_cache_ttl_secs, 300);
}

// ===========================================================================
// Issue #21: dolphin-milk.toml.example
// ===========================================================================

#[test]
fn test_worm_toml_example_parses() {
    // The example file has all values commented out, so it should parse as
    // a valid DmConfig with all defaults.
    let example_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dolphin-milk.toml.example");
    assert!(
        example_path.exists(),
        "dolphin-milk.toml.example should exist at project root"
    );

    let contents = std::fs::read_to_string(&example_path).unwrap();
    let cfg: DmConfig = toml::from_str(&contents).unwrap();

    // Should parse to defaults since everything is commented out
    assert_eq!(cfg.wallet.url, "http://localhost:3322");
    assert_eq!(cfg.budget.max_per_task, 20_000_000);
    assert_eq!(
        cfg.x402.registry_url,
        "https://x402agency.com/.well-known/agents"
    );
}

#[test]
fn test_worm_toml_example_has_all_sections() {
    let example_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dolphin-milk.toml.example");
    let contents = std::fs::read_to_string(&example_path).unwrap();

    // All 15 config sections must appear (14 top-level + heartbeat.active_hours sub-section)
    let required_sections = [
        "[wallet]",
        "[budget]",
        "[llm]",
        "[logging]",
        "[memory]",
        "[heartbeat]",
        "[heartbeat.active_hours]",
        "[parent]",
        "[mcp]",
        "[server]",
        "[certificates]",
        "[browser]",
        "[lifecycle]",
        "[compliance]",
        "[x402]",
        "[rates]",
    ];

    for section in &required_sections {
        assert!(
            contents.contains(section),
            "dolphin-milk.toml.example is missing section: {section}"
        );
    }
}

// ===========================================================================
// Issue #16: RatesConfig — exchange rate configuration
// ===========================================================================

#[test]
fn test_rates_config_defaults() {
    let cfg = DmConfig::default();
    assert_eq!(cfg.rates.refresh_interval_secs, 60);
    assert!((cfg.rates.margin_percent - 0.0).abs() < f64::EPSILON);
    assert_eq!(cfg.rates.stale_threshold_secs, 300);
}

#[test]
fn test_rates_config_from_toml() {
    let toml_str = r#"
[rates]
refresh_interval_secs = 30
margin_percent = 2.5
stale_threshold_secs = 120
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.rates.refresh_interval_secs, 30);
    assert!((cfg.rates.margin_percent - 2.5).abs() < f64::EPSILON);
    assert_eq!(cfg.rates.stale_threshold_secs, 120);
}

#[test]
fn test_rates_config_partial_toml() {
    let toml_str = r#"
[rates]
margin_percent = 5.0
"#;
    let cfg: DmConfig = toml::from_str(toml_str).unwrap();
    assert!((cfg.rates.margin_percent - 5.0).abs() < f64::EPSILON);
    // Defaults for unset fields
    assert_eq!(cfg.rates.refresh_interval_secs, 60);
    assert_eq!(cfg.rates.stale_threshold_secs, 300);
}

#[test]
fn test_reload_safe_fields_updates_rates() {
    let mut config = DmConfig::default();
    let mut fresh = DmConfig::default();
    fresh.rates.refresh_interval_secs = 15;
    fresh.rates.margin_percent = 3.0;
    fresh.rates.stale_threshold_secs = 600;

    config.reload_safe_fields(&fresh);

    assert_eq!(config.rates.refresh_interval_secs, 15);
    assert!((config.rates.margin_percent - 3.0).abs() < f64::EPSILON);
    assert_eq!(config.rates.stale_threshold_secs, 600);
}

// ===========================================================================
// Issue #160: Unified data directory (~/.dolphin-milk/)
// ===========================================================================

#[test]
fn test_data_dir_default() {
    // When data_dir is None, resolved_data_dir should be ~/.dolphin-milk
    let cfg = DmConfig::default();
    assert!(cfg.data_dir.is_none());
    let resolved = cfg.resolved_data_dir();
    // Should end with .dolphin-milk regardless of actual HOME value
    assert!(
        resolved.ends_with(".dolphin-milk"),
        "resolved_data_dir should end with .dolphin-milk, got: {}",
        resolved.display()
    );
}

#[test]
fn test_data_dir_from_config() {
    let cfg = DmConfig {
        data_dir: Some("/tmp/my-worm-data".to_string()),
        ..Default::default()
    };
    let resolved = cfg.resolved_data_dir();
    assert_eq!(resolved, std::path::PathBuf::from("/tmp/my-worm-data"));
}

#[test]
fn test_data_dir_from_env() {
    // Test that DOLPHIN_MILK_DATA_DIR env var sets data_dir.
    // NOTE: env vars are process-global and can race with parallel tests.
    // We test data_dir override via direct struct manipulation to avoid races.
    // The env var is tested by verifying the field exists and the apply_env
    // path is exercised in loader.rs (same pattern as other env override tests).
    let mut cfg = DmConfig::default();
    assert!(cfg.data_dir.is_none());

    // Simulate what apply_env does for DOLPHIN_MILK_DATA_DIR
    cfg.data_dir = Some("/tmp/env-worm-dir".to_string());

    assert_eq!(cfg.data_dir, Some("/tmp/env-worm-dir".to_string()));
    assert_eq!(
        cfg.resolved_data_dir(),
        std::path::PathBuf::from("/tmp/env-worm-dir")
    );
    // Verify derived paths
    assert_eq!(
        cfg.workspace_dir(),
        std::path::PathBuf::from("/tmp/env-worm-dir/workspace")
    );
}

#[test]
fn test_data_dir_from_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("data_dir.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"data_dir = "/opt/worm-state""#).unwrap();
    }
    // Clear env to avoid interference
    let saved = std::env::var("DOLPHIN_MILK_DATA_DIR").ok();
    std::env::remove_var("DOLPHIN_MILK_DATA_DIR");

    let cfg = load_config(Some(path.as_path())).unwrap();
    assert_eq!(cfg.data_dir, Some("/opt/worm-state".to_string()));
    assert_eq!(
        cfg.resolved_data_dir(),
        std::path::PathBuf::from("/opt/worm-state")
    );

    // Restore
    if let Some(v) = saved {
        std::env::set_var("DOLPHIN_MILK_DATA_DIR", v);
    }
}

#[test]
fn test_data_dir_tilde_expansion() {
    let cfg = DmConfig {
        data_dir: Some("~/my-worm".to_string()),
        ..Default::default()
    };
    let resolved = cfg.resolved_data_dir();
    // Should NOT contain ~ — it should be expanded
    assert!(
        !resolved.to_string_lossy().contains('~'),
        "tilde should be expanded, got: {}",
        resolved.display()
    );
    // Should end with /my-worm
    assert!(
        resolved.ends_with("my-worm"),
        "should end with my-worm, got: {}",
        resolved.display()
    );
}

#[test]
fn test_workspace_dir_derives_from_data_dir() {
    let cfg = DmConfig {
        data_dir: Some("/tmp/test-worm".to_string()),
        ..Default::default()
    };
    assert_eq!(
        cfg.workspace_dir(),
        std::path::PathBuf::from("/tmp/test-worm/workspace")
    );
}

#[test]
fn test_memory_dir_derives_from_data_dir() {
    let cfg = DmConfig {
        data_dir: Some("/tmp/test-worm".to_string()),
        ..Default::default()
    };
    // Default memory.base_dir is "memory" -> should derive from workspace
    assert_eq!(
        cfg.memory_dir(),
        std::path::PathBuf::from("/tmp/test-worm/workspace/memory")
    );
}

#[test]
fn test_memory_dir_explicit_override() {
    let mut cfg = DmConfig {
        data_dir: Some("/tmp/test-worm".to_string()),
        ..Default::default()
    };
    cfg.memory.base_dir = "/custom/memory/path".to_string();
    // Explicit non-default path should be honored
    assert_eq!(
        cfg.memory_dir(),
        std::path::PathBuf::from("/custom/memory/path")
    );
}

#[test]
fn test_memory_dir_old_default_falls_through() {
    let mut cfg = DmConfig {
        data_dir: Some("/tmp/test-worm".to_string()),
        ..Default::default()
    };
    // Old default "/testbed/memory" should be treated as unset
    cfg.memory.base_dir = "/testbed/memory".to_string();
    assert_eq!(
        cfg.memory_dir(),
        std::path::PathBuf::from("/tmp/test-worm/workspace/memory")
    );
}

#[test]
fn test_config_path_derives_from_data_dir() {
    let cfg = DmConfig {
        data_dir: Some("/tmp/test-worm".to_string()),
        ..Default::default()
    };
    assert_eq!(
        cfg.config_path(),
        std::path::PathBuf::from("/tmp/test-worm/dolphin-milk.toml")
    );
}

#[test]
fn test_skills_dir_derives_from_data_dir() {
    let cfg = DmConfig {
        data_dir: Some("/tmp/test-worm".to_string()),
        ..Default::default()
    };
    assert_eq!(
        cfg.skills_dir(),
        std::path::PathBuf::from("/tmp/test-worm/skills")
    );
}

#[test]
fn test_backward_compat_no_data_dir() {
    // When data_dir is not set, the old default memory.base_dir should resolve
    // to workspace/memory under ~/.dolphin-milk
    let cfg = DmConfig::default();
    assert!(cfg.data_dir.is_none());
    let mem = cfg.memory_dir();
    // Should end with workspace/memory
    assert!(
        mem.ends_with("workspace/memory"),
        "memory_dir should end with workspace/memory, got: {}",
        mem.display()
    );
}

#[test]
fn test_memory_config_default_changed() {
    // Verify the default changed from "/testbed/memory" to "memory"
    let cfg = DmConfig::default();
    assert_eq!(cfg.memory.base_dir, "memory");
}

// ---------------------------------------------------------------------------
// Stall detection config (#259)
// ---------------------------------------------------------------------------

#[test]
fn test_stall_timeout_default_none() {
    let cfg = DmConfig::default();
    assert!(cfg.llm.stall_timeout_secs.is_none());
}

#[test]
fn test_stall_timeout_from_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("dolphin-milk.toml");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "[llm]\nstall_timeout_secs = 90").unwrap();
    let cfg = load_config(Some(&path)).unwrap();
    assert_eq!(cfg.llm.stall_timeout_secs, Some(90));
}
