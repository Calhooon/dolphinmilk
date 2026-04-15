//! Configuration loading: multi-source TOML + DOLPHIN_MILK_* environment variable overrides.
//!
//! Priority (highest wins):
//!   1. DOLPHIN_MILK_* environment variables (e.g., DOLPHIN_MILK_WALLET_URL)
//!   2. `.dolphin-milk/config.toml` in workspace (project config)
//!   3. `~/.dolphin-milk/config.toml` (user config)
//!   4. `dolphin-milk.toml` in working directory (instance config)
//!   5. Built-in defaults
//!
//! Use [`load_config`] for backward-compatible single-file loading (delegates to
//! the precedence system internally). Use [`load_config_with_precedence`] when you
//! need workspace-aware multi-source config with source tracking.

mod loader;
pub mod precedence;
pub mod project;
mod schema;
mod watcher;

// Re-export everything so callers continue to use `crate::config::*` unchanged.
pub use loader::load_config;
pub use precedence::{load_config_with_precedence, ConfigSources};
pub use schema::{
    ActiveHours, BrowserConfig, BudgetConfig, CertificateConfig, ComplianceConfig, DmConfig,
    HeartbeatConfig, LifecycleConfig, LlmConfig, LoggingConfig, McpConfig, MemoryConfig,
    MessageBoxConfig, ModerationConfig, OverlayConfig, ParentConfig, RateLimitConfig, RatesConfig,
    ServerConfig, ServiceRateLimit, ToolApprovalConfig, ToolDeferralConfig, TrustConfig,
    WalletConfig, X402Config,
};
pub use watcher::ConfigWatcher;

// ---------------------------------------------------------------------------
// Hot config reload — impl on DmConfig
// ---------------------------------------------------------------------------

impl DmConfig {
    /// Reload safe operational fields from a freshly-loaded config.
    ///
    /// Only updates fields that can change at runtime without security implications.
    /// Wallet, parent identity, and logging config are NEVER updated via hot reload.
    pub fn reload_safe_fields(&mut self, fresh: &DmConfig) {
        // Heartbeat operational parameters
        self.heartbeat.inbox_poll_secs = fresh.heartbeat.inbox_poll_secs;
        self.heartbeat.max_concurrent_tasks = fresh.heartbeat.max_concurrent_tasks;
        self.heartbeat.active_hours = fresh.heartbeat.active_hours.clone();
        self.heartbeat.checklist_file = fresh.heartbeat.checklist_file.clone();
        self.heartbeat.reflection_enabled = fresh.heartbeat.reflection_enabled;
        self.heartbeat.reflection_interval_secs = fresh.heartbeat.reflection_interval_secs;
        self.heartbeat.reflection_model = fresh.heartbeat.reflection_model.clone();
        self.heartbeat.reflection_max_iterations = fresh.heartbeat.reflection_max_iterations;

        // Budget limits
        self.budget.max_per_task = fresh.budget.max_per_task;
        self.budget.max_per_hour = fresh.budget.max_per_hour;
        self.budget.max_per_day = fresh.budget.max_per_day;
        self.budget.max_per_week = fresh.budget.max_per_week;
        self.budget.max_per_month = fresh.budget.max_per_month;
        self.budget.max_lifetime = fresh.budget.max_lifetime;
        self.budget.low_power_threshold = fresh.budget.low_power_threshold;
        self.budget.enforcement = fresh.budget.enforcement.clone();

        // LLM operational settings
        self.llm.default_model = fresh.llm.default_model.clone();
        self.llm.default_provider = fresh.llm.default_provider.clone();
        self.llm.max_tokens = fresh.llm.max_tokens;
        self.llm.max_tokens_reasoning = fresh.llm.max_tokens_reasoning;
        self.llm.context_window = fresh.llm.context_window;
        self.llm.max_history_turns = fresh.llm.max_history_turns;
        self.llm.min_recent_messages = fresh.llm.min_recent_messages;

        // Browser operational settings (safe: only affects future browser launches)
        self.browser = fresh.browser.clone();

        // Lifecycle sweep policies (safe: only affects future sweeps)
        self.lifecycle = fresh.lifecycle.clone();

        // Compliance operational settings (safe: only affects future proofs/sweeps)
        self.compliance = fresh.compliance.clone();

        // Memory maintenance settings (safe: only affects future maintenance runs)
        self.memory.maintenance_enabled = fresh.memory.maintenance_enabled;
        self.memory.maintenance_interval_secs = fresh.memory.maintenance_interval_secs;
        self.memory.stale_session_days = fresh.memory.stale_session_days;

        // Rates operational settings (safe: only affects future rate refreshes)
        self.rates = fresh.rates.clone();

        // Moderation policy (safe: only affects future moderation checks)
        self.moderation = fresh.moderation.clone();

        // Trust config — operational fields ONLY. `certifiers` is deliberately
        // excluded from hot-reload as a security property: changing the trust
        // root requires a restart so it can never silently expand at runtime.
        self.trust.max_chain_depth = fresh.trust.max_chain_depth;
        self.trust.verifier_cache_ttl_secs = fresh.trust.verifier_cache_ttl_secs;
        self.trust.verifier_cache_negative_ttl_secs = fresh.trust.verifier_cache_negative_ttl_secs;
        self.trust.require_delegation_for_external = fresh.trust.require_delegation_for_external;
        self.trust.revocation_recheck_interval_secs = fresh.trust.revocation_recheck_interval_secs;

        // NOT updated: wallet, parent.identity_key, logging, mcp, server, certificates,
        // trust.certifiers (security: trust root changes require restart).
    }
}
