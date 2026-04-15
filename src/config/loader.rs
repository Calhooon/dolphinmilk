//! Configuration loading: TOML file + DOLPHIN_MILK_* environment variable overrides.

use std::path::Path;

use crate::error::DmError;

use super::schema::DmConfig;

/// Read an environment variable with DOLPHIN_MILK_ prefix.
pub(super) fn env_var(name: &str) -> Result<String, std::env::VarError> {
    let dm_name = format!("DOLPHIN_MILK_{name}");
    std::env::var(&dm_name)
}

/// Load configuration from TOML file, then apply env overrides.
///
/// Delegates to [`super::precedence::load_config_with_precedence`] which
/// merges multiple config sources. When called with an explicit `config_path`,
/// that path is used as the instance config. User (`~/.dolphin-milk/config.toml`)
/// and project (`.dolphin-milk/config.toml`) configs are also loaded if they exist.
///
/// If no project or user configs are present on disk, behavior is identical
/// to the original single-file loading.
///
/// Config file search order for instance config:
/// 1. `config_path` argument (if provided)
/// 2. `$DOLPHIN_MILK_DATA_DIR/dolphin-milk.toml`
/// 3. `./dolphin-milk.toml`
/// 4. Built-in defaults
pub fn load_config(config_path: Option<&Path>) -> Result<DmConfig, DmError> {
    let (config, _sources) = super::precedence::load_config_with_precedence(None, config_path)?;
    Ok(config)
}

/// Apply DOLPHIN_MILK_* environment variable overrides.
pub(super) fn apply_env(config: &mut DmConfig) -> Result<(), DmError> {
    // Data directory — root for all state
    if let Ok(val) = env_var("DATA_DIR") {
        config.data_dir = Some(val);
    }
    if let Ok(v) = env_var("WALLET_URL") {
        config.wallet.url = v;
    }
    if let Ok(v) = env_var("WALLET_TIMEOUT") {
        config.wallet.timeout = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_WALLET_TIMEOUT: {v}")))?;
    }
    if let Ok(v) = env_var("BUDGET_MAX_PER_TASK") {
        config.budget.max_per_task = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_BUDGET_MAX_PER_TASK: {v}"))
        })?;
    }
    if let Ok(v) = env_var("BUDGET_MAX_PER_HOUR") {
        config.budget.max_per_hour = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_BUDGET_MAX_PER_HOUR: {v}"))
        })?;
    }
    if let Ok(v) = env_var("BUDGET_MAX_PER_DAY") {
        config.budget.max_per_day = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_BUDGET_MAX_PER_DAY: {v}"))
        })?;
    }
    if let Ok(v) = env_var("BUDGET_WEEKLY_SATS") {
        config.budget.max_per_week = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_BUDGET_WEEKLY_SATS: {v}"))
        })?;
    }
    if let Ok(v) = env_var("BUDGET_MONTHLY_SATS") {
        config.budget.max_per_month = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_BUDGET_MONTHLY_SATS: {v}"))
        })?;
    }
    if let Ok(v) = env_var("BUDGET_LIFETIME_SATS") {
        config.budget.max_lifetime = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_BUDGET_LIFETIME_SATS: {v}"))
        })?;
    }
    if let Ok(v) = env_var("BUDGET_ENFORCEMENT") {
        config.budget.enforcement = v;
    }
    if let Ok(v) = env_var("BUDGET_STAGING_THRESHOLD") {
        config.budget.staging_threshold = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_BUDGET_STAGING_THRESHOLD: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LOG_LEVEL") {
        config.logging.level = v;
    }
    if let Ok(v) = env_var("LOG_FORMAT") {
        config.logging.format = v;
    }
    if let Ok(v) = env_var("LLM_PROVIDER") {
        config.llm.default_provider = v;
    }
    if let Ok(v) = env_var("MEMORY_DIR") {
        config.memory.base_dir = v;
    }
    if let Ok(v) = env_var("MEMORY_MAINTENANCE_ENABLED") {
        config.memory.maintenance_enabled = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_MEMORY_MAINTENANCE_ENABLED: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("MEMORY_MAINTENANCE_INTERVAL") {
        config.memory.maintenance_interval_secs = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_MEMORY_MAINTENANCE_INTERVAL: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("MEMORY_STALE_DAYS") {
        config.memory.stale_session_days = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_MEMORY_STALE_DAYS: {v}")))?;
    }
    if let Ok(v) = env_var("LLM_MODEL") {
        config.llm.default_model = v;
    }
    if let Ok(v) = env_var("LLM_MAX_TOKENS") {
        config.llm.max_tokens = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_LLM_MAX_TOKENS: {v}")))?;
    }
    if let Ok(v) = env_var("LLM_MAX_TOKENS_REASONING") {
        config.llm.max_tokens_reasoning = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LLM_MAX_TOKENS_REASONING: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LLM_CONTEXT_WINDOW") {
        config.llm.context_window = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LLM_CONTEXT_WINDOW: {v}"))
        })?;
    }
    if let Ok(v) = env_var("LLM_MAX_HISTORY_TURNS") {
        config.llm.max_history_turns = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LLM_MAX_HISTORY_TURNS: {v}"))
        })?;
    }
    if let Ok(v) = env_var("LLM_MIN_RECENT_MESSAGES") {
        config.llm.min_recent_messages = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LLM_MIN_RECENT_MESSAGES: {v}"))
        })?;
    }
    if let Ok(v) = env_var("LLM_COMPACTION_ENABLED") {
        config.llm.compaction_enabled = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LLM_COMPACTION_ENABLED: {v}"))
        })?;
    }
    if let Ok(v) = env_var("LLM_COMPACTION_MODEL") {
        config.llm.compaction_model = if v.is_empty() { None } else { Some(v) };
    }
    if let Ok(v) = env_var("LLM_COMPACTION_THRESHOLD") {
        config.llm.compaction_threshold = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LLM_COMPACTION_THRESHOLD: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LLM_CACHE_TTL_SECS") {
        config.llm.cache_ttl_secs = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LLM_CACHE_TTL_SECS: {v}"))
        })?;
    }
    if let Ok(v) = env_var("LLM_CACHE_MAX_ENTRIES") {
        config.llm.cache_max_entries = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LLM_CACHE_MAX_ENTRIES: {v}"))
        })?;
    }
    if let Ok(val) = env_var("LLM_THINKING_BUDGET") {
        if let Ok(n) = val.parse::<u32>() {
            config.llm.thinking_budget = Some(n);
        }
    }
    if let Ok(val) = env_var("LLM_REASONING_EFFORT") {
        config.llm.reasoning_effort = Some(val);
    }
    if let Ok(val) = env_var("LLM_STALL_TIMEOUT") {
        if let Ok(n) = val.parse::<u64>() {
            config.llm.stall_timeout_secs = Some(n);
        }
    }
    if let Ok(v) = env_var("HEARTBEAT_ENABLED") {
        config.heartbeat.enabled = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_HEARTBEAT_ENABLED: {v}")))?;
    }
    if let Ok(v) = env_var("HEARTBEAT_POLL_SECS") {
        config.heartbeat.inbox_poll_secs = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_HEARTBEAT_POLL_SECS: {v}"))
        })?;
    }
    if let Ok(v) = env_var("HEARTBEAT_MAX_CONCURRENT") {
        config.heartbeat.max_concurrent_tasks = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_HEARTBEAT_MAX_CONCURRENT: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("HEARTBEAT_CHECKLIST_FILE") {
        config.heartbeat.checklist_file = if v.is_empty() || v == "none" {
            None
        } else {
            Some(v)
        };
    }
    if let Ok(v) = env_var("HEARTBEAT_ACTIVE_START") {
        config.heartbeat.active_hours.start = Some(v);
    }
    if let Ok(v) = env_var("HEARTBEAT_ACTIVE_END") {
        config.heartbeat.active_hours.end = Some(v);
    }
    if let Ok(v) = env_var("HEARTBEAT_ACTIVE_TZ") {
        config.heartbeat.active_hours.timezone = Some(v);
    }
    if let Ok(v) = env_var("REFLECTION_ENABLED") {
        config.heartbeat.reflection_enabled = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_REFLECTION_ENABLED: {v}"))
        })?;
    }
    if let Ok(v) = env_var("REFLECTION_INTERVAL") {
        config.heartbeat.reflection_interval_secs = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_REFLECTION_INTERVAL: {v}"))
        })?;
    }
    if let Ok(v) = env_var("REFLECTION_MODEL") {
        config.heartbeat.reflection_model = v;
    }
    if let Ok(v) = env_var("REFLECTION_MAX_ITERATIONS") {
        config.heartbeat.reflection_max_iterations = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_REFLECTION_MAX_ITERATIONS: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("PARENT_KEY") {
        config.parent.identity_key = v;
    }
    if let Ok(v) = env_var("PARENT_WALLET_URL") {
        config.parent.wallet_url = v;
    }
    if let Ok(v) = env_var("TRUSTED_CERTIFIERS") {
        config.parent.trusted_certifiers = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(v) = env_var("OPENAI_COMPAT") {
        config.server.openai_compat_enabled = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_OPENAI_COMPAT: {v}")))?;
    }
    if let Ok(v) = env_var("BROWSER_ENABLED") {
        config.browser.enabled = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_BROWSER_ENABLED: {v}")))?;
    }
    if let Ok(v) = env_var("BROWSER_CHROME_PATH") {
        config.browser.chrome_path = v;
    }
    if let Ok(v) = env_var("BROWSER_HEADLESS") {
        config.browser.headless = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_BROWSER_HEADLESS: {v}")))?;
    }
    if let Ok(v) = env_var("BROWSER_TIMEOUT") {
        config.browser.page_load_timeout = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_BROWSER_TIMEOUT: {v}")))?;
    }
    if let Ok(v) = env_var("BROWSER_MAX_PAGES") {
        config.browser.max_pages = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_BROWSER_MAX_PAGES: {v}")))?;
    }
    // Lifecycle / UTXO sweep
    if let Ok(v) = env_var("LIFECYCLE_BUDGET_TOKEN_MAX_AGE_HOURS") {
        config.lifecycle.budget_token_max_age_hours = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LIFECYCLE_BUDGET_TOKEN_MAX_AGE_HOURS: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LIFECYCLE_CHECKPOINT_MAX_AGE_HOURS") {
        config.lifecycle.checkpoint_max_age_hours = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LIFECYCLE_CHECKPOINT_MAX_AGE_HOURS: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LIFECYCLE_AUTO_SWEEP") {
        config.lifecycle.auto_sweep_enabled = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_LIFECYCLE_AUTO_SWEEP: {v}"))
        })?;
    }
    if let Ok(v) = env_var("LIFECYCLE_SWEEP_INTERVAL") {
        config.lifecycle.sweep_interval_minutes = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LIFECYCLE_SWEEP_INTERVAL: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LIFECYCLE_VERIFY_INTERVAL") {
        config.lifecycle.verify_interval = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LIFECYCLE_VERIFY_INTERVAL: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LIFECYCLE_CONSISTENCY_CHECK_INTERVAL") {
        config.lifecycle.consistency_check_interval = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LIFECYCLE_CONSISTENCY_CHECK_INTERVAL: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("LIFECYCLE_BASKET_MONITORING_INTERVAL") {
        config.lifecycle.basket_monitoring_interval_secs = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_LIFECYCLE_BASKET_MONITORING_INTERVAL: {v}"
            ))
        })?;
    }
    // Compliance
    if let Ok(v) = env_var("COMPLIANCE_ENABLED") {
        config.compliance.enabled = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_COMPLIANCE_ENABLED: {v}"))
        })?;
    }
    if let Ok(v) = env_var("COMPLIANCE_RETENTION_DAYS") {
        config.compliance.default_retention_days = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_COMPLIANCE_RETENTION_DAYS: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("COMPLIANCE_REGULATIONS") {
        config.compliance.regulations = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(v) = env_var("COMPLIANCE_WORM_MODE") {
        config.compliance.worm_mode = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_COMPLIANCE_WORM_MODE: {v}"))
        })?;
    }
    // MCP
    if let Ok(v) = env_var("MCP_WALLET_COMMAND") {
        config.mcp.wallet_mcp_command = v;
    }
    // x402
    if let Ok(v) = env_var("X402_REGISTRY_URL") {
        config.x402.registry_url = v;
    }
    if let Ok(v) = env_var("X402_CACHE_TTL") {
        config.x402.registry_cache_ttl_secs = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_X402_CACHE_TTL: {v}")))?;
    }
    // x402 rate limits
    if let Ok(v) = env_var("RATE_LIMIT_ENABLED") {
        config.x402.rate_limits.enabled = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_RATE_LIMIT_ENABLED: {v}"))
        })?;
    }
    if let Ok(v) = env_var("RATE_LIMIT_DEFAULT_RPM") {
        config.x402.rate_limits.default_rpm = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_RATE_LIMIT_DEFAULT_RPM: {v}"))
        })?;
    }
    // Rates
    if let Ok(v) = env_var("RATE_REFRESH_INTERVAL") {
        config.rates.refresh_interval_secs = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_RATE_REFRESH_INTERVAL: {v}"))
        })?;
    }
    if let Ok(v) = env_var("RATE_MARGIN_PERCENT") {
        config.rates.margin_percent = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_RATE_MARGIN_PERCENT: {v}"))
        })?;
    }
    if let Ok(v) = env_var("RATE_STALE_THRESHOLD") {
        config.rates.stale_threshold_secs = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_RATE_STALE_THRESHOLD: {v}"))
        })?;
    }
    // Moderation
    if let Ok(v) = env_var("MODERATION_ENABLED") {
        config.moderation.enabled = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_MODERATION_ENABLED: {v}"))
        })?;
    }
    if let Ok(v) = env_var("MODERATION_PII_MODE") {
        config.moderation.pii_mode = v;
    }
    if let Ok(v) = env_var("MODERATION_PROFANITY_MODE") {
        config.moderation.profanity_mode = v;
    }
    if let Ok(v) = env_var("MODERATION_BLOCK_KEYWORDS") {
        config.moderation.custom_block_keywords = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    // Tool approval
    if let Ok(v) = env_var("APPROVAL_TOOLS") {
        config.tool_approval.require = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(v) = env_var("APPROVAL_TIMEOUT") {
        config.tool_approval.timeout_secs = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_APPROVAL_TIMEOUT: {v}")))?;
    }

    // Agent name (used for overlay registration + certificates)
    if let Ok(v) = env_var("AGENT_NAME") {
        config.certificates.agent_name = v;
    }
    // Per-role BRC-52 capabilities for the boot-time cert acquisition. Empty
    // string is treated as "use default" (None) so cluster.js / production
    // launchers can opt-in cleanly.
    if let Ok(v) = env_var("CERT_CAPABILITIES") {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            config.certificates.capabilities = None;
        } else {
            config.certificates.capabilities = Some(trimmed.to_string());
        }
    }

    // Overlay
    if let Ok(v) = env_var("OVERLAY_ENABLED") {
        config.overlay.enabled = v
            .parse()
            .map_err(|_| DmError::config(format!("Invalid DOLPHIN_MILK_OVERLAY_ENABLED: {v}")))?;
    }
    if let Ok(v) = env_var("OVERLAY_SUBMIT_URL") {
        config.overlay.submit_url = v;
    }
    if let Ok(v) = env_var("OVERLAY_ENDPOINT") {
        config.overlay.endpoint = v;
    }

    // [messagebox] — BRC-33 peer messaging relay
    if let Ok(v) = env_var("MESSAGEBOX_URL") {
        config.messagebox.url = v;
    }
    if let Ok(v) = env_var("MESSAGEBOX_IDENTITY_KEY") {
        config.messagebox.server_identity_key = v;
    }

    // [tools] — deferral configuration
    if let Ok(v) = env_var("TOOLS_DEFERRAL_MODE") {
        config.tools.deferral_mode = v;
    }
    if let Ok(v) = env_var("TOOLS_AUTO_THRESHOLD") {
        config.tools.auto_threshold_pct = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_TOOLS_AUTO_THRESHOLD: {v}"))
        })?;
    }

    // [trust] — Phase 3 cross-agent delegation root certifier list and verifier knobs.
    // See `docs/DELEGATION-DESIGN.md` §8.
    if let Ok(v) = env_var("TRUST_CERTIFIERS") {
        config.trust.certifiers = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(v) = env_var("TRUST_MAX_CHAIN_DEPTH") {
        config.trust.max_chain_depth = v.parse().map_err(|_| {
            DmError::config(format!("Invalid DOLPHIN_MILK_TRUST_MAX_CHAIN_DEPTH: {v}"))
        })?;
    }
    if let Ok(v) = env_var("TRUST_VERIFIER_CACHE_TTL_SECS") {
        config.trust.verifier_cache_ttl_secs = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_TRUST_VERIFIER_CACHE_TTL_SECS: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("TRUST_VERIFIER_CACHE_NEGATIVE_TTL_SECS") {
        config.trust.verifier_cache_negative_ttl_secs = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_TRUST_VERIFIER_CACHE_NEGATIVE_TTL_SECS: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("TRUST_REQUIRE_DELEGATION_FOR_EXTERNAL") {
        config.trust.require_delegation_for_external = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_TRUST_REQUIRE_DELEGATION_FOR_EXTERNAL: {v}"
            ))
        })?;
    }
    if let Ok(v) = env_var("TRUST_REVOCATION_RECHECK_INTERVAL_SECS") {
        config.trust.revocation_recheck_interval_secs = v.parse().map_err(|_| {
            DmError::config(format!(
                "Invalid DOLPHIN_MILK_TRUST_REVOCATION_RECHECK_INTERVAL_SECS: {v}"
            ))
        })?;
    }

    // Auto-insert parent identity key at the front of `trust.certifiers`
    // (deduped). Makes "parent is trusted by default" the natural behavior
    // without requiring duplicate config across [parent] and [trust]. Operators
    // can still add 3rd-party certifiers (DAOs, industry groups) to the list.
    auto_insert_parent_certifier(config);

    Ok(())
}

/// Insert `parent.identity_key` at the front of `trust.certifiers` if set and
/// not already present. Runs at the end of `apply_env()` so it picks up both
/// the TOML file value and any DOLPHIN_MILK_PARENT_KEY env override.
pub(super) fn auto_insert_parent_certifier(config: &mut DmConfig) {
    let parent_key = config.parent.identity_key.trim();
    if parent_key.is_empty() {
        return;
    }
    if config.trust.certifiers.iter().any(|c| c == parent_key) {
        return;
    }
    config.trust.certifiers.insert(0, parent_key.to_string());
}
