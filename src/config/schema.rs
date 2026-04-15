//! Config struct definitions with defaults and serde derives.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WalletConfig {
    pub url: String,
    pub origin: String,
    pub timeout: u64,
    /// SQLite database path for the embedded wallet. Default: `{data_dir}/wallet.db`
    pub db_path: Option<String>,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:3322".into(),
            origin: "http://localhost".into(),
            timeout: 120,
            db_path: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BudgetConfig {
    pub max_per_task: u64,
    pub max_per_hour: u64,
    pub max_per_day: u64,
    /// Rolling 7-day budget limit in satoshis. Default: 100M. 0 = unlimited.
    pub max_per_week: u64,
    /// Rolling 30-day budget limit in satoshis. Default: 500M. 0 = unlimited.
    pub max_per_month: u64,
    /// Lifetime cumulative budget limit in satoshis. Default: 0 (unlimited).
    pub max_lifetime: u64,
    pub low_power_threshold: u64,
    /// Satoshi threshold above which payments require manual approval (staging).
    /// Default: 500,000 (500K sats). Set to 0 to disable staging.
    pub staging_threshold: u64,
    /// Budget enforcement mode: "strict" (default) blocks on limit, "advisory" warns but continues.
    pub enforcement: String,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_per_task: 20_000_000,
            max_per_hour: 50_000_000,
            max_per_day: 200_000_000,
            max_per_week: 100_000_000,
            max_per_month: 500_000_000,
            max_lifetime: 0,
            low_power_threshold: 50_000,
            staging_threshold: 500_000,
            enforcement: "strict".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// LLM provider: "openai-agent" or "claude-chat". Auto-detected from model name if unset.
    pub default_provider: String,
    /// Model name for inference (e.g., "gpt-5-mini", "claude-sonnet-4-6").
    pub default_model: String,
    /// Max tokens for standard models.
    pub max_tokens: u32,
    /// Max tokens for reasoning models (o-series, gpt-5.x, gpt-4.1).
    pub max_tokens_reasoning: u32,
    /// Context window size in tokens. Used for proportional tool result capping.
    pub context_window: usize,
    /// Maximum conversation turns to keep.
    pub max_history_turns: usize,
    /// Minimum messages guaranteed during truncation.
    pub min_recent_messages: usize,
    /// Enable two-phase compaction (memory flush + summary) when history exceeds max_history_turns.
    pub compaction_enabled: bool,
    /// Model override for compaction LLM calls. None = use default_model.
    pub compaction_model: Option<String>,
    /// Response cache TTL in seconds. 0 = disabled. Default: 3600 (1 hour).
    pub cache_ttl_secs: u64,
    /// Maximum number of cached responses. Default: 1000.
    pub cache_max_entries: usize,
    /// Token utilization threshold (0.0–1.0) at which auto-compaction triggers.
    /// Default: 0.8 (80%). Budget-aware adjustment may lower this dynamically.
    pub compaction_threshold: f64,
    /// Extended thinking budget for Claude models (tokens). None = disabled.
    /// When set, Claude will use extended thinking with this token budget.
    pub thinking_budget: Option<u32>,
    /// Reasoning effort for OpenAI models. None = API default.
    /// Valid values: "low", "medium", "high".
    pub reasoning_effort: Option<String>,
    /// Stall detection timeout in seconds. None = default 120s.
    /// If the LLM does not respond within this timeout, the request is aborted.
    pub stall_timeout_secs: Option<u64>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            default_provider: "openai-agent".into(),
            default_model: std::env::var("DOLPHIN_MILK_LLM_MODEL")
                .unwrap_or_else(|_| "gpt-5-mini".into()),
            max_tokens: 16384,
            max_tokens_reasoning: 16384,
            context_window: 128_000,
            max_history_turns: 40,
            min_recent_messages: 8,
            compaction_enabled: true,
            compaction_model: None,
            compaction_threshold: 0.8,
            cache_ttl_secs: 3600,
            cache_max_entries: 1000,
            thinking_budget: None,
            reasoning_effort: None,
            stall_timeout_secs: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "INFO".into(),
            format: "json".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub base_dir: String,
    /// Enable periodic memory maintenance via heartbeat. Default: false (off).
    pub maintenance_enabled: bool,
    /// Interval in seconds between maintenance runs. Default: 3600 (1 hour).
    pub maintenance_interval_secs: u64,
    /// Number of days after which session/execution memories are considered stale. Default: 30.
    /// Knowledge entries are NEVER flagged as stale.
    pub stale_session_days: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            base_dir: "memory".into(),
            maintenance_enabled: false,
            maintenance_interval_secs: 3600,
            stale_session_days: 30,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HeartbeatConfig {
    pub enabled: bool,
    pub inbox_poll_secs: u64,
    /// Maximum number of concurrent tasks the scheduler can run. Default 3.
    pub max_concurrent_tasks: usize,
    /// Active hours window — heartbeat skips ticks outside this window.
    #[serde(default)]
    pub active_hours: ActiveHours,
    /// Path to the heartbeat checklist file relative to workspace.
    /// Set to empty string or "none" to disable. Default: "HEARTBEAT.md".
    pub checklist_file: Option<String>,
    /// Enable autonomous reflection tasks. Default: false (disabled to prevent unexpected costs).
    pub reflection_enabled: bool,
    /// Interval in seconds between reflection tasks. Default: 60.
    pub reflection_interval_secs: u64,
    /// Model for reflection tasks. Default: "claude-haiku-4-5-20251001" (cheap model).
    pub reflection_model: String,
    /// Maximum iterations per reflection task. Default: 3.
    pub reflection_max_iterations: u32,
    /// Cooldown in seconds before re-processing identical HEARTBEAT.md content. Default: 600 (10 min).
    pub checklist_cooldown_secs: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inbox_poll_secs: 60,
            max_concurrent_tasks: 3,
            active_hours: ActiveHours::default(),
            checklist_file: Some("HEARTBEAT.md".to_string()),
            reflection_enabled: false,
            reflection_interval_secs: 60,
            reflection_model: "claude-haiku-4-5-20251001".into(),
            reflection_max_iterations: 3,
            checklist_cooldown_secs: 600,
        }
    }
}

/// Active hours window for the heartbeat scheduler.
/// When configured, the scheduler skips ticks outside the window.
/// All fields None = always active (no restriction).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ActiveHours {
    /// Start time in HH:MM 24-hour format, e.g., "09:00".
    pub start: Option<String>,
    /// End time in HH:MM 24-hour format, e.g., "22:00".
    pub end: Option<String>,
    /// IANA timezone, e.g., "America/New_York". Defaults to UTC.
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ParentConfig {
    /// 66-char hex compressed pubkey of the parent wallet.
    /// If empty, parent auth is disabled (dev mode).
    pub identity_key: String,
    /// URL of the parent wallet for BRC-52 certificate signing.
    /// Empty = self-signed certs (sovereign mode, no parent needed).
    /// Set to a wallet URL (e.g., "http://localhost:3321") for parent-child delegation.
    pub wallet_url: String,
    /// Identity keys of trusted certifiers beyond our own parent.
    /// Agents whose certs are signed by these certifiers get Vouched trust tier
    /// (lower fees, more access). Default empty.
    /// Env: DOLPHIN_MILK_TRUSTED_CERTIFIERS (comma-separated)
    pub trusted_certifiers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub enabled: bool,
    pub transport: String,
    pub port: u16,
    /// Path or command for the wallet MCP server binary (default: "bsv-wallet-mcp")
    #[serde(default = "default_wallet_mcp_command")]
    pub wallet_mcp_command: String,
}

fn default_wallet_mcp_command() -> String {
    "bsv-wallet-mcp".into()
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: "stdio".into(),
            port: 3001,
            wallet_mcp_command: default_wallet_mcp_command(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CertificateConfig {
    /// Agent name for the authorization certificate.
    pub agent_name: String,
    /// Certificate type. Default: "agent-authorization".
    pub certificate_type: String,
    /// Comma-separated capabilities list for the boot-time BRC-52 authorization
    /// certificate (e.g., `"web_fetch,execute_bash,scraping"`). When set, the
    /// agent acquires a parent-signed cert with exactly these capabilities at
    /// boot. If an existing parent-signed cert does not match this set, it is
    /// revoked (spending its revocation UTXO) and relinquished, and a fresh cert
    /// is issued. When unset, the hardcoded default capability set is used
    /// (preserves legacy behavior).
    ///
    /// Override via `DOLPHIN_MILK_CERT_CAPABILITIES` env var. Primary use: test
    /// harnesses (`tests/multi-worm/lib/cluster.js`) and per-role DolphinSense
    /// production agents that each need role-specific capabilities for overlay
    /// `findByCapability` discovery.
    pub capabilities: Option<String>,
}

impl Default for CertificateConfig {
    fn default() -> Self {
        Self {
            agent_name: "dolphin-milk-agent".into(),
            certificate_type: "agent-authorization".into(),
            capabilities: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ServerConfig {
    /// Enable OpenAI-compatible `/v1/chat/completions` endpoint.
    pub openai_compat_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// Enable browser automation tool. Default: true.
    pub enabled: bool,
    /// Path to Chrome/Chromium binary. Empty = auto-fetch via chromiumoxide fetcher.
    pub chrome_path: String,
    /// Run Chrome in headless mode. Default: true.
    pub headless: bool,
    /// Page load timeout in seconds. Default: 30.
    pub page_load_timeout: u64,
    /// Default wait after navigation in milliseconds. Default: 1000.
    pub default_wait_ms: u64,
    /// Maximum number of open pages. Default: 3.
    pub max_pages: usize,
    /// Viewport width in pixels. Default: 1280.
    pub viewport_width: u32,
    /// Viewport height in pixels. Default: 720.
    pub viewport_height: u32,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            chrome_path: String::new(),
            headless: true,
            page_load_timeout: 30,
            default_wait_ms: 1000,
            max_pages: 3,
            viewport_width: 1280,
            viewport_height: 720,
        }
    }
}

/// UTXO lifecycle management — sweep policies for stale state tokens.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LifecycleConfig {
    /// Max age in hours for BudgetAllocation tokens (completed tasks). Default: 168 (7 days).
    pub budget_token_max_age_hours: u64,
    /// Max age in hours for Checkpoint tokens. Default: 720 (30 days).
    pub checkpoint_max_age_hours: u64,
    /// Whether to auto-sweep on heartbeat tick. Default: true.
    pub auto_sweep_enabled: bool,
    /// Sweep interval in minutes. Default: 60.
    pub sweep_interval_minutes: u64,
    /// How often (in iterations) to auto-verify in-memory state against on-chain proofs.
    /// 0 = disabled. Default: 10.
    pub verify_interval: u32,
    /// How often (in iterations) to check BRC-48 token consistency against LoopState.
    /// 0 = disabled. Default: 5.
    pub consistency_check_interval: u32,
    /// Interval in seconds between basket monitoring log messages. Default: 3600 (1 hour).
    /// 0 = disabled. Logs basket counts for capacity planning.
    pub basket_monitoring_interval_secs: u64,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            budget_token_max_age_hours: 168, // 7 days
            checkpoint_max_age_hours: 720,   // 30 days
            auto_sweep_enabled: true,
            sweep_interval_minutes: 60,
            verify_interval: 10,
            consistency_check_interval: 5,
            basket_monitoring_interval_secs: 3600,
        }
    }
}

/// x402 service registry configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct X402Config {
    /// Registry URL for discovering x402 services.
    /// Default: "https://x402agency.com/.well-known/agents".
    pub registry_url: String,
    /// Cache TTL in seconds for the registry response. Default: 300 (5 minutes).
    pub registry_cache_ttl_secs: u64,
    /// Per-service rate limiting configuration.
    #[serde(default)]
    pub rate_limits: RateLimitConfig,
}

impl Default for X402Config {
    fn default() -> Self {
        Self {
            registry_url: "https://x402agency.com/.well-known/agents".into(),
            registry_cache_ttl_secs: 300,
            rate_limits: RateLimitConfig::default(),
        }
    }
}

/// Per-service rate limiting configuration for x402 services.
///
/// When enabled, a token bucket rate limiter throttles requests to each service.
/// Default: disabled (backward compatible — no rate limiting).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RateLimitConfig {
    /// Enable rate limiting globally. Default: false (no rate limiting).
    pub enabled: bool,
    /// Default requests per minute for all services. 0 = unlimited.
    pub default_rpm: u32,
    /// Per-service rate limit overrides, keyed by base URL.
    pub per_service: HashMap<String, ServiceRateLimit>,
}

/// Rate limit configuration for a single x402 service.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceRateLimit {
    /// Requests per minute allowed for this service.
    pub requests_per_minute: u32,
}

/// BSV/USD exchange rate configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RatesConfig {
    /// Refresh interval in seconds. Default: 60.
    pub refresh_interval_secs: u64,
    /// Margin percentage applied on top of the spot rate (for JIT proxy use). Default: 0.0.
    pub margin_percent: f64,
    /// Number of seconds after which a cached rate is considered stale. Default: 300 (5 minutes).
    pub stale_threshold_secs: u64,
}

impl Default for RatesConfig {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 60,
            margin_percent: 0.0,
            stale_threshold_secs: 300,
        }
    }
}

/// Compliance mode — regulatory tags, retention policies, WORM enforcement.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ComplianceConfig {
    /// Whether compliance mode is enabled. Default: false (opt-in).
    pub enabled: bool,
    /// Default retention period in days. Default: 2555 (7 years, SEC 17a-4).
    pub default_retention_days: u64,
    /// Regulatory frameworks to tag on proofs. Default: empty.
    pub regulations: Vec<String>,
    /// Classification for financial operations. Default: "financial".
    pub financial_classification: String,
    /// Classification for communication operations. Default: "communication".
    pub communication_classification: String,
    /// WORM mode: prevent deletion of proofs and transcripts. Default: false.
    pub worm_mode: bool,
}

impl Default for ComplianceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_retention_days: 2555, // 7 years (SEC 17a-4)
            regulations: Vec::new(),
            financial_classification: "financial".into(),
            communication_classification: "communication".into(),
            worm_mode: false,
        }
    }
}

/// Content moderation policy — PII detection, keyword blocking, regex filtering.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModerationConfig {
    /// Master switch for moderation. Default: false (disabled, zero overhead).
    pub enabled: bool,
    /// PII detection mode: "block", "flag", "off". Default: "off".
    pub pii_mode: String,
    /// Profanity detection mode: "block", "flag", "off". Default: "off".
    pub profanity_mode: String,
    /// Regex patterns that block content when matched.
    pub custom_block_patterns: Vec<String>,
    /// Regex patterns that flag content when matched (logged, execution continues).
    pub custom_flag_patterns: Vec<String>,
    /// Exact keywords that block content when matched (case-insensitive).
    pub custom_block_keywords: Vec<String>,
}

impl Default for ModerationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pii_mode: "off".into(),
            profanity_mode: "off".into(),
            custom_block_patterns: Vec::new(),
            custom_flag_patterns: Vec::new(),
            custom_block_keywords: Vec::new(),
        }
    }
}

/// Per-tool approval workflow configuration.
///
/// When a tool name is listed in `require`, the runner blocks execution
/// of that tool until manual approval is received (via `/staged/{ref}/approve`).
/// If no approval arrives within `timeout_secs`, the call is auto-aborted.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ToolApprovalConfig {
    /// Tool names that require manual approval before execution.
    pub require: Vec<String>,
    /// Seconds to wait for approval before auto-aborting. Default: 300 (5 minutes).
    pub timeout_secs: u64,
}

impl Default for ToolApprovalConfig {
    fn default() -> Self {
        Self {
            require: Vec::new(),
            timeout_secs: 300,
        }
    }
}

/// Trust root configuration for cross-agent delegation (Phase 3 of EPIC #329).
///
/// Controls which identity keys this agent accepts as the root certifier of
/// inbound delegation cert chains, plus operational knobs for the delegation
/// verifier (cache TTLs, max chain depth, revocation re-check cadence). See
/// `docs/DELEGATION-DESIGN.md` §8.
///
/// ```toml
/// [trust]
/// certifiers = [
///     "03abc...parent_wallet_key",
///     "02def...industry_certifier_key",
/// ]
/// max_chain_depth = 5
/// verifier_cache_ttl_secs = 300
/// verifier_cache_negative_ttl_secs = 30
/// require_delegation_for_external = false
/// revocation_recheck_interval_secs = 60
/// ```
///
/// **Hot-reload exclusion:** `certifiers` is NOT hot-reloadable. Changing
/// the trust root requires a restart. This is a security property — the
/// trust root must never silently expand at runtime. Other fields (TTLs,
/// depth, recheck interval) ARE hot-reloadable.
///
/// Env overrides:
/// - `DOLPHIN_MILK_TRUST_CERTIFIERS` (comma-separated 66-hex pubkeys)
/// - `DOLPHIN_MILK_TRUST_MAX_CHAIN_DEPTH`
/// - `DOLPHIN_MILK_TRUST_VERIFIER_CACHE_TTL_SECS`
/// - `DOLPHIN_MILK_TRUST_VERIFIER_CACHE_NEGATIVE_TTL_SECS`
/// - `DOLPHIN_MILK_TRUST_REQUIRE_DELEGATION_FOR_EXTERNAL`
/// - `DOLPHIN_MILK_TRUST_REVOCATION_RECHECK_INTERVAL_SECS`
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TrustConfig {
    /// Identity keys (66-hex compressed pubkeys) accepted as root certifiers
    /// for inbound delegation cert chains. `parent.identity_key` is auto-inserted
    /// at the front of this list at boot when set (see
    /// `loader::auto_insert_parent_certifier`).
    pub certifiers: Vec<String>,
    /// Maximum cert chain depth for multi-hop re-delegation. Hard cap regardless
    /// of cert claims. Default: 5.
    pub max_chain_depth: u8,
    /// Positive verifier cache TTL (seconds). Actual TTL clamps to
    /// `min(cert.expires_at - now, this)`. Default: 300 (5 min).
    pub verifier_cache_ttl_secs: u64,
    /// Negative verifier cache TTL (seconds). How long to remember a verification
    /// failure before re-checking. Default: 30s.
    pub verifier_cache_negative_ttl_secs: u64,
    /// If true, require a valid delegation cert for ALL external-origin tasks.
    /// If false (default for backwards compat), tasks without a cert fall back
    /// to `EXTERNAL_TOOL_ALLOWLIST`. DolphinSense production sets this to `true`.
    pub require_delegation_for_external: bool,
    /// Heartbeat revocation re-check interval for in-flight delegated tasks (seconds).
    /// Default: 60s.
    pub revocation_recheck_interval_secs: u64,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            certifiers: Vec::new(),
            max_chain_depth: 5,
            verifier_cache_ttl_secs: 300,
            verifier_cache_negative_ttl_secs: 30,
            require_delegation_for_external: false,
            revocation_recheck_interval_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct DmConfig {
    /// Root directory for all state. Default: ~/.dolphin-milk
    /// Env: DOLPHIN_MILK_DATA_DIR
    pub data_dir: Option<String>,
    pub wallet: WalletConfig,
    pub budget: BudgetConfig,
    pub llm: LlmConfig,
    pub logging: LoggingConfig,
    pub memory: MemoryConfig,
    pub heartbeat: HeartbeatConfig,
    pub parent: ParentConfig,
    pub mcp: McpConfig,
    pub server: ServerConfig,
    pub certificates: CertificateConfig,
    pub browser: BrowserConfig,
    pub lifecycle: LifecycleConfig,
    pub compliance: ComplianceConfig,
    pub x402: X402Config,
    pub rates: RatesConfig,
    pub moderation: ModerationConfig,
    pub tool_approval: ToolApprovalConfig,
    pub tools: ToolDeferralConfig,
    pub overlay: OverlayConfig,
    pub messagebox: MessageBoxConfig,
    /// Trust root for cross-agent delegation cert verification (Phase 3 of #329).
    pub trust: TrustConfig,
    /// Lifecycle hooks — observe, modify, or block agent behavior.
    /// Defined as `[[hooks]]` array entries in dolphin-milk.toml.
    #[serde(default)]
    pub hooks: Vec<crate::hooks::HookConfigEntry>,
}

/// Overlay service integration — agent registration and lookup.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    /// Enable overlay integration (check-then-register on startup). Default: false.
    pub enabled: bool,
    /// Overlay service URL for /submit and /lookup.
    pub submit_url: String,
    /// This agent's public endpoint URL (advertised to other agents).
    pub endpoint: String,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            submit_url: "https://rust-overlay.dev-a3e.workers.dev".into(),
            endpoint: String::new(),
        }
    }
}

/// MessageBox server configuration — BRC-33 peer-to-peer messaging relay.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MessageBoxConfig {
    /// MessageBox server URL. Default: new rust-message-box CF Worker.
    /// Env: DOLPHIN_MILK_MESSAGEBOX_URL
    pub url: String,
    /// MessageBox server identity key (66-char hex compressed pubkey).
    /// Used as fallback when the 402 response doesn't include the auth header.
    /// Env: DOLPHIN_MILK_MESSAGEBOX_IDENTITY_KEY
    pub server_identity_key: String,
}

impl Default for MessageBoxConfig {
    fn default() -> Self {
        Self {
            url: "https://rust-message-box.dev-a3e.workers.dev".into(),
            server_identity_key:
                "02d7c923b39a464c7d029a45975476f39406bc3430ac487742cbd97e0d428343b9".into(),
        }
    }
}

/// Tool schema deferral configuration.
///
/// Controls whether tool schemas are sent in full to the LLM or replaced
/// with lightweight hints to save tokens.
///
/// ```toml
/// [tools]
/// deferral_mode = "auto"   # "always", "auto", "never"
/// auto_threshold_pct = 50  # defer when tool schemas > N% of context
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ToolDeferralConfig {
    /// Deferral mode: "always" | "auto" | "never". Default: "auto".
    pub deferral_mode: String,
    /// For "auto" mode: defer when tool schemas exceed this % of context window. Default: 50.
    pub auto_threshold_pct: f64,
}

impl Default for ToolDeferralConfig {
    fn default() -> Self {
        Self {
            deferral_mode: "auto".to_string(),
            auto_threshold_pct: 50.0,
        }
    }
}

impl DmConfig {
    /// Resolve the data directory path.
    ///
    /// Priority:
    /// 1. `data_dir` config field (supports ~ tilde expansion)
    /// 2. `~/.dolphin-milk` (default)
    pub fn resolved_data_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.data_dir {
            let expanded = expand_tilde(dir);
            PathBuf::from(expanded)
        } else {
            home_dir_or_dot().join(".dolphin-milk")
        }
    }

    /// Workspace directory, derived from data_dir.
    pub fn workspace_dir(&self) -> PathBuf {
        self.resolved_data_dir().join("workspace")
    }

    /// Memory directory, derived from data_dir unless explicitly overridden.
    ///
    /// If `memory.base_dir` has been explicitly set to a non-default value,
    /// that value is honored for backward compatibility. Otherwise, the
    /// memory directory is derived from the workspace.
    pub fn memory_dir(&self) -> PathBuf {
        let base = &self.memory.base_dir;
        // Honor explicit overrides — anything that isn't the old default
        // ("/testbed/memory") or the new relative default ("memory").
        if base != "/testbed/memory" && base != "memory" {
            PathBuf::from(base)
        } else {
            self.workspace_dir().join("memory")
        }
    }

    /// Config file path within the data directory.
    pub fn config_path(&self) -> PathBuf {
        self.resolved_data_dir().join("dolphin-milk.toml")
    }

    /// Skills directory path within the data directory.
    pub fn skills_dir(&self) -> PathBuf {
        self.resolved_data_dir().join("skills")
    }
}

/// Expand leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        if path == "~" {
            home
        } else {
            format!("{}{}", home, &path[1..])
        }
    } else {
        path.to_string()
    }
}

/// Return the user's home directory, or `.` as a fallback.
fn home_dir_or_dot() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
