# src/config/
> TOML configuration with environment overrides and hot reload.

## Overview

Three-tier config precedence (highest wins): `WORM_*` env vars → `worm.toml` → built-in defaults. Split across four files: schema definitions, loading logic, filesystem watcher, and hot-reload coordination.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 83 | Re-exports all 21 types + `reload_safe_fields()` impl on `WormConfig` |
| `schema.rs` | 599 | 21 config structs with serde derives and `Default` impls (17 sections + 3 sub-structs + `WormConfig` root) + data directory resolution helpers |
| `loader.rs` | 399 | `load_config()` with `WORM_DATA_DIR`-aware search + `apply_env()` with 74 env overrides |
| `watcher.rs` | 75 | `ConfigWatcher` — `notify` crate fs watcher with 500ms debounce |

## Configuration Structs

`WormConfig` is the root struct with 17 section structs plus `data_dir: Option<String>` (root directory for all worm state, default `~/.bsv-worm`):

| Struct | Key Fields | Defaults |
|--------|-----------|----------|
| `WalletConfig` | `url`, `origin`, `timeout` | `localhost:3322`, `localhost`, `30` |
| `BudgetConfig` | `max_per_task`, `max_per_hour`, `max_per_day`, `max_per_week`, `max_per_month`, `max_lifetime`, `low_power_threshold`, `staging_threshold`, `enforcement` | 20M, 50M, 200M, 100M, 500M, 0 (unlimited), 50K, 500K sats, `strict` |
| `LlmConfig` | `default_provider`, `default_model`, `max_tokens`, `max_tokens_reasoning`, `context_window`, `max_history_turns`, `min_recent_messages`, `compaction_enabled`, `compaction_model`, `cache_ttl_secs`, `cache_max_entries`, `thinking_budget`, `reasoning_effort` | `openai-agent`, `gpt-5-mini`, `16384`, `16384`, `128000`, `40`, `8`, `true`, `None`, `3600`, `1000`, `None`, `None` |
| `LoggingConfig` | `level`, `format` | `INFO`, `json` |
| `MemoryConfig` | `base_dir`, `maintenance_enabled`, `maintenance_interval_secs`, `stale_session_days` | `memory`, `false`, `3600`, `30` |
| `HeartbeatConfig` | `enabled`, `inbox_poll_secs`, `max_concurrent_tasks`, `active_hours`, `checklist_file`, `reflection_*`, `checklist_cooldown_secs` | `true`, `60`, `3`, no restriction, `HEARTBEAT.md`, reflection off, `600` |
| `ParentConfig` | `identity_key`, `wallet_url` | empty (dev mode), `localhost:3321` |
| `McpConfig` | `enabled`, `transport`, `port`, `wallet_mcp_command` | `false`, `stdio`, `3001`, `bsv-wallet-mcp` |
| `ServerConfig` | `openai_compat_enabled` | `false` |
| `CertificateConfig` | `agent_name`, `certificate_type` | `worm-agent`, `agent-authorization` |
| `BrowserConfig` | `enabled`, `chrome_path`, `headless`, `page_load_timeout`, `default_wait_ms`, `max_pages`, `viewport_width`, `viewport_height` | `true`, `""` (auto-fetch), `true`, `30`, `1000`, `3`, `1280`, `720` |
| `LifecycleConfig` | `budget_token_max_age_hours`, `checkpoint_max_age_hours`, `auto_sweep_enabled`, `sweep_interval_minutes`, `verify_interval`, `consistency_check_interval`, `basket_monitoring_interval_secs` | `168` (7d), `720` (30d), `true`, `60`, `10`, `5`, `3600` (1h) |
| `ComplianceConfig` | `enabled`, `default_retention_days`, `regulations`, `financial_classification`, `communication_classification`, `worm_mode` | `false`, `2555` (7yr SEC 17a-4), `[]`, `financial`, `communication`, `false` |
| `X402Config` | `registry_url`, `registry_cache_ttl_secs`, `rate_limits` | `https://x402agency.com/.well-known/agents`, `300` (5 min), disabled |
| `RatesConfig` | `refresh_interval_secs`, `margin_percent`, `stale_threshold_secs` | `60`, `0.0`, `300` (5 min) |
| `ModerationConfig` | `enabled`, `pii_mode`, `profanity_mode`, `custom_block_patterns`, `custom_flag_patterns`, `custom_block_keywords` | `false`, `off`, `off`, `[]`, `[]`, `[]` |
| `ToolApprovalConfig` | `require`, `timeout_secs` | `[]`, `300` (5 min) |

### Sub-structs

| Struct | Parent | Key Fields | Defaults |
|--------|--------|-----------|----------|
| `ActiveHours` | `HeartbeatConfig` | `start`, `end`, `timezone` | all `None` (always active) |
| `RateLimitConfig` | `X402Config` | `enabled`, `default_rpm`, `per_service` | `false`, `0`, `{}` |
| `ServiceRateLimit` | `RateLimitConfig` | `requests_per_minute` | — |

## Data Directory Resolution

`WormConfig` provides 5 helper methods for deriving paths from `data_dir`:

| Method | Returns | Default |
|--------|---------|---------|
| `resolved_data_dir()` | Root state directory (tilde-expanded) | `~/.bsv-worm` |
| `workspace_dir()` | `{data_dir}/workspace` | `~/.bsv-worm/workspace` |
| `memory_dir()` | Memory storage (honors explicit `memory.base_dir` overrides) | `{workspace}/memory` |
| `config_path()` | Config file location | `{data_dir}/worm.toml` |
| `skills_dir()` | Skills directory | `{data_dir}/skills` |

`memory_dir()` has backward-compat logic: if `memory.base_dir` is set to something other than the old default (`/testbed/memory`) or new default (`memory`), the explicit path is honored. Otherwise it derives from `workspace_dir()`.

## Environment Variables

74 `WORM_*` env overrides applied after TOML parsing.

| Variable | Config Field | Type |
|----------|-------------|------|
| `WORM_DATA_DIR` | `data_dir` | String (sets `Some`) |
| `WORM_WALLET_URL` | `wallet.url` | String |
| `WORM_WALLET_TIMEOUT` | `wallet.timeout` | u64 |
| `WORM_BUDGET_MAX_PER_TASK` | `budget.max_per_task` | u64 |
| `WORM_BUDGET_MAX_PER_HOUR` | `budget.max_per_hour` | u64 |
| `WORM_BUDGET_MAX_PER_DAY` | `budget.max_per_day` | u64 |
| `WORM_BUDGET_WEEKLY_SATS` | `budget.max_per_week` | u64 |
| `WORM_BUDGET_MONTHLY_SATS` | `budget.max_per_month` | u64 |
| `WORM_BUDGET_LIFETIME_SATS` | `budget.max_lifetime` | u64 |
| `WORM_BUDGET_ENFORCEMENT` | `budget.enforcement` | String |
| `WORM_BUDGET_STAGING_THRESHOLD` | `budget.staging_threshold` | u64 |
| `WORM_LOG_LEVEL` | `logging.level` | String |
| `WORM_LOG_FORMAT` | `logging.format` | String |
| `WORM_LLM_PROVIDER` | `llm.default_provider` | String |
| `WORM_LLM_MODEL` | `llm.default_model` | String |
| `WORM_LLM_MAX_TOKENS` | `llm.max_tokens` | u32 |
| `WORM_LLM_MAX_TOKENS_REASONING` | `llm.max_tokens_reasoning` | u32 |
| `WORM_LLM_CONTEXT_WINDOW` | `llm.context_window` | usize |
| `WORM_LLM_MAX_HISTORY_TURNS` | `llm.max_history_turns` | usize |
| `WORM_LLM_MIN_RECENT_MESSAGES` | `llm.min_recent_messages` | usize |
| `WORM_LLM_COMPACTION_ENABLED` | `llm.compaction_enabled` | bool |
| `WORM_LLM_COMPACTION_MODEL` | `llm.compaction_model` | Option (empty = None) |
| `WORM_LLM_CACHE_TTL_SECS` | `llm.cache_ttl_secs` | u64 |
| `WORM_LLM_CACHE_MAX_ENTRIES` | `llm.cache_max_entries` | usize |
| `WORM_LLM_THINKING_BUDGET` | `llm.thinking_budget` | Option<u32> (sets Some) |
| `WORM_LLM_REASONING_EFFORT` | `llm.reasoning_effort` | Option<String> (sets Some) |
| `WORM_MEMORY_DIR` | `memory.base_dir` | String |
| `WORM_MEMORY_MAINTENANCE_ENABLED` | `memory.maintenance_enabled` | bool |
| `WORM_MEMORY_MAINTENANCE_INTERVAL` | `memory.maintenance_interval_secs` | u64 |
| `WORM_MEMORY_STALE_DAYS` | `memory.stale_session_days` | u64 |
| `WORM_HEARTBEAT_ENABLED` | `heartbeat.enabled` | bool |
| `WORM_HEARTBEAT_POLL_SECS` | `heartbeat.inbox_poll_secs` | u64 |
| `WORM_HEARTBEAT_MAX_CONCURRENT` | `heartbeat.max_concurrent_tasks` | usize |
| `WORM_HEARTBEAT_CHECKLIST_FILE` | `heartbeat.checklist_file` | Option (empty/"none" = None) |
| `WORM_HEARTBEAT_ACTIVE_START` | `heartbeat.active_hours.start` | String (HH:MM) |
| `WORM_HEARTBEAT_ACTIVE_END` | `heartbeat.active_hours.end` | String (HH:MM) |
| `WORM_HEARTBEAT_ACTIVE_TZ` | `heartbeat.active_hours.timezone` | String (IANA) |
| `WORM_REFLECTION_ENABLED` | `heartbeat.reflection_enabled` | bool |
| `WORM_REFLECTION_INTERVAL` | `heartbeat.reflection_interval_secs` | u64 |
| `WORM_REFLECTION_MODEL` | `heartbeat.reflection_model` | String |
| `WORM_REFLECTION_MAX_ITERATIONS` | `heartbeat.reflection_max_iterations` | u32 |
| `WORM_PARENT_KEY` | `parent.identity_key` | String (66-char hex pubkey) |
| `WORM_PARENT_WALLET_URL` | `parent.wallet_url` | String |
| `WORM_OPENAI_COMPAT` | `server.openai_compat_enabled` | bool |
| `WORM_BROWSER_ENABLED` | `browser.enabled` | bool |
| `WORM_BROWSER_CHROME_PATH` | `browser.chrome_path` | String |
| `WORM_BROWSER_HEADLESS` | `browser.headless` | bool |
| `WORM_BROWSER_TIMEOUT` | `browser.page_load_timeout` | u64 |
| `WORM_BROWSER_MAX_PAGES` | `browser.max_pages` | usize |
| `WORM_LIFECYCLE_BUDGET_TOKEN_MAX_AGE_HOURS` | `lifecycle.budget_token_max_age_hours` | u64 |
| `WORM_LIFECYCLE_CHECKPOINT_MAX_AGE_HOURS` | `lifecycle.checkpoint_max_age_hours` | u64 |
| `WORM_LIFECYCLE_AUTO_SWEEP` | `lifecycle.auto_sweep_enabled` | bool |
| `WORM_LIFECYCLE_SWEEP_INTERVAL` | `lifecycle.sweep_interval_minutes` | u64 |
| `WORM_LIFECYCLE_VERIFY_INTERVAL` | `lifecycle.verify_interval` | u32 |
| `WORM_LIFECYCLE_CONSISTENCY_CHECK_INTERVAL` | `lifecycle.consistency_check_interval` | u32 |
| `WORM_LIFECYCLE_BASKET_MONITORING_INTERVAL` | `lifecycle.basket_monitoring_interval_secs` | u64 |
| `WORM_COMPLIANCE_ENABLED` | `compliance.enabled` | bool |
| `WORM_COMPLIANCE_RETENTION_DAYS` | `compliance.default_retention_days` | u64 |
| `WORM_COMPLIANCE_REGULATIONS` | `compliance.regulations` | Comma-separated list |
| `WORM_COMPLIANCE_WORM_MODE` | `compliance.worm_mode` | bool |
| `WORM_MCP_WALLET_COMMAND` | `mcp.wallet_mcp_command` | String |
| `WORM_X402_REGISTRY_URL` | `x402.registry_url` | String |
| `WORM_X402_CACHE_TTL` | `x402.registry_cache_ttl_secs` | u64 |
| `WORM_RATE_LIMIT_ENABLED` | `x402.rate_limits.enabled` | bool |
| `WORM_RATE_LIMIT_DEFAULT_RPM` | `x402.rate_limits.default_rpm` | u32 |
| `WORM_RATE_REFRESH_INTERVAL` | `rates.refresh_interval_secs` | u64 |
| `WORM_RATE_MARGIN_PERCENT` | `rates.margin_percent` | f64 |
| `WORM_RATE_STALE_THRESHOLD` | `rates.stale_threshold_secs` | u64 |
| `WORM_MODERATION_ENABLED` | `moderation.enabled` | bool |
| `WORM_MODERATION_PII_MODE` | `moderation.pii_mode` | String |
| `WORM_MODERATION_PROFANITY_MODE` | `moderation.profanity_mode` | String |
| `WORM_MODERATION_BLOCK_KEYWORDS` | `moderation.custom_block_keywords` | Comma-separated list |
| `WORM_APPROVAL_TOOLS` | `tool_approval.require` | Comma-separated list |
| `WORM_APPROVAL_TIMEOUT` | `tool_approval.timeout_secs` | u64 |

## Hot Reload

`ConfigWatcher::start()` watches the parent directory of `worm.toml` using the `notify` crate (handles editor write-then-rename patterns). On change:

1. 500ms debounce window drains duplicate events
2. `load_config()` re-reads file + env overrides
3. New config broadcast via `tokio::sync::watch` channel
4. Consumers call `reload_safe_fields()` to merge safe fields

**Safe fields** (updated at runtime): heartbeat operational params (poll, concurrency, active hours, checklist, reflection), budget limits (all 6 tiers + low_power_threshold + enforcement), LLM settings (model, provider, token limits, context window, history turns, min recent messages), browser config, lifecycle sweep policies, compliance settings, memory maintenance params (enabled, interval, stale days), rates (refresh interval, margin, stale threshold), moderation policy (enabled, PII mode, profanity mode, block/flag patterns, keywords).

**Never updated via hot reload**: `data_dir`, `wallet`, `parent.identity_key`, `logging`, `mcp`, `server`, `certificates`, `tool_approval`, `budget.staging_threshold`, `llm.compaction_enabled`, `llm.compaction_model`, `llm.cache_ttl_secs`, `llm.cache_max_entries`, `llm.thinking_budget`, `llm.reasoning_effort`, `memory.base_dir`, `x402`. These require a restart.

## Loading Flow

```
load_config(path) → search for worm.toml → parse (or defaults) → apply_env() → WormConfig
```

Config file search order:
1. `config_path` argument (if provided)
2. `$WORM_DATA_DIR/worm.toml` (if `WORM_DATA_DIR` env var is set and file exists)
3. `./worm.toml` (backward compat)
4. Built-in defaults (if no file found)

- `load_config()` accepts `Option<&Path>`
- Missing file is not an error — falls back to `WormConfig::default()`
- Parse errors return `WormError::Config`
- Env override parse failures also return `WormError::Config`

## Key Behaviors

- **Data directory**: `data_dir` (default `~/.bsv-worm`) is the root for all worm state. Supports `~` tilde expansion. All derived paths (workspace, memory, config, skills) branch from this root. Set via `WORM_DATA_DIR` env var or `data_dir` in TOML.
- **Dev mode**: `parent.identity_key` empty → BRC-31 auth disabled on server
- **LLM model from env**: `LlmConfig::default()` reads `WORM_LLM_MODEL` at struct construction time (before `apply_env`), so the env var works even without a TOML file. Default: `gpt-5-mini`.
- **Extended thinking**: `thinking_budget` (Option<u32>, default `None`) sets Claude's extended thinking token budget. When set, Claude uses extended thinking with this budget.
- **Reasoning effort**: `reasoning_effort` (Option<String>, default `None`) sets OpenAI's reasoning effort level. Valid values: `"low"`, `"medium"`, `"high"`.
- **Active hours**: All `None` = always active. Both `start` and `end` required for windowing. IANA timezone (defaults to UTC).
- **Reflection defaults to off**: `reflection_enabled: false` prevents unexpected LLM costs
- **Budget in satoshis**: All budget values are in sats. Six tiers: per-task (20M), per-hour (50M), per-day (200M), per-week (100M), per-month (500M), lifetime (0 = unlimited). `low_power_threshold` (50K sats) triggers conservative agent behavior.
- **Budget enforcement**: `enforcement` field controls behavior on limit hit — `"strict"` (default) blocks the operation, `"advisory"` warns but continues.
- **Staging threshold**: `staging_threshold` (default 500K sats) determines when payments require manual approval. Set to 0 to disable staging.
- **LLM response cache**: `cache_ttl_secs` (default 3600 = 1 hour) and `cache_max_entries` (default 1000) configure the in-memory LRU response cache in `x402/cache.rs`. Set TTL to 0 to disable caching.
- **Memory maintenance**: `maintenance_enabled` (default false), `maintenance_interval_secs` (default 3600), `stale_session_days` (default 30). When enabled, heartbeat runs periodic cleanup of stale session/execution memories. Knowledge entries are never flagged as stale.
- **Checklist cooldown**: `checklist_cooldown_secs` (default 600) prevents re-processing identical `HEARTBEAT.md` content within the cooldown window
- **Browser auto-fetch**: Empty `chrome_path` triggers chromiumoxide's built-in Chrome fetcher
- **Lifecycle sweep**: `LifecycleConfig` controls UTXO cleanup — stale BudgetAllocation tokens (>7d) and Checkpoints (>30d) are swept on heartbeat tick when `auto_sweep_enabled`
- **Lifecycle verification**: `verify_interval` (default 10) controls how often (in iterations) the runner auto-verifies in-memory state against on-chain proofs. `consistency_check_interval` (default 5) controls how often BRC-48 token consistency is checked against LoopState. 0 = disabled for either.
- **Basket monitoring**: `basket_monitoring_interval_secs` (default 3600 = 1 hour) controls periodic basket count logging for capacity planning. 0 = disabled.
- **Compliance opt-in**: `ComplianceConfig.enabled` defaults to `false`. When enabled, proofs get regulatory tags. `worm_mode` prevents deletion of proofs/transcripts. `regulations` is a comma-separated list of regulatory frameworks (e.g., `SEC-17a-4,SOX`). Retention default is 2555 days (7 years per SEC 17a-4).
- **x402 service registry**: `X402Config.registry_url` points to the agent registry for service discovery (default `x402agency.com`). `registry_cache_ttl_secs` (default 300 = 5 min) controls disk cache TTL for registry responses.
- **x402 rate limiting**: `RateLimitConfig` (under `[x402.rate_limits]`) enables token-bucket rate limiting per x402 service. `default_rpm` sets global rate; `per_service` map overrides by base URL. Disabled by default.
- **BSV/USD rates**: `RatesConfig` controls exchange rate refresh (`refresh_interval_secs` default 60s), proxy margin (`margin_percent` default 0%), and staleness TTL (`stale_threshold_secs` default 300s).
- **Content moderation**: `ModerationConfig` provides PII detection, profanity filtering, custom regex block/flag patterns, and keyword blocking. All off by default (`enabled: false`). Modes: `"block"` (reject), `"flag"` (log + continue), `"off"`.
- **Tool approval workflow**: `ToolApprovalConfig` lists tool names requiring manual approval before execution. Pending calls auto-abort after `timeout_secs` (default 300s). Empty `require` list = no approval needed.
- **Wallet MCP command**: `McpConfig.wallet_mcp_command` (default `bsv-wallet-mcp`) configures the path or command for the wallet MCP server binary, overridable via `WORM_MCP_WALLET_COMMAND`

## Related

- Parent: [`src/CLAUDE.md`](../CLAUDE.md)
- Consumer: [`src/server/CLAUDE.md`](../server/CLAUDE.md) — `AppState` holds `Arc<RwLock<WormConfig>>` + watch receiver
- Consumer: [`src/heartbeat/CLAUDE.md`](../heartbeat/CLAUDE.md) — scheduler reads config via watch channel
- Errors: `src/error.rs` — `WormError::Config` variant
