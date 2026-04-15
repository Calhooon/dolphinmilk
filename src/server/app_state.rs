//! Application state construction, router building, and server lifecycle.
//!
//! Extracted from `server/mod.rs` to keep that file focused on module
//! declarations and re-exports.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{header, HeaderName, Method};
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::{broadcast, Mutex, Notify, RwLock, Semaphore};
use tower_http::cors::CorsLayer;
#[cfg(not(feature = "embed-ui"))]
use tower_http::services::ServeDir;

use crate::auth::server::Brc31SessionStore;
use bsv::wallet::substrates::HttpWalletJson;
use bsv::wallet::GetPublicKeyArgs;

use crate::budget::BudgetTracker;
use crate::config::DmConfig;
use crate::events::StepEvent;
use crate::heartbeat::Scheduler;
use crate::x402::rate_limit::RateLimiterRegistry;

/// Task tracking: in-memory task registry, cancel flags, conversation locks.
pub struct TaskManager {
    /// In-memory task info, keyed by task ID.
    pub tasks: Mutex<HashMap<String, super::TaskInfo>>,
    /// Cancel flags per task — set to true to stop a running task.
    pub cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Maps task_id -> session_id (conversation_id) for the events endpoint.
    pub task_sessions: Mutex<HashMap<String, String>>,
    /// Per-conversation semaphores (1 permit each) — tasks queue behind each other on the same conversation.
    pub conversation_semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// Number of currently running tasks — used by the scheduler for concurrency limiting.
    pub active_task_count: Arc<AtomicUsize>,
    /// Idempotency map for POST /chat requests keyed by "<requester>:<client_command_id>".
    pub(crate) chat_command_dedupe: Mutex<HashMap<String, ChatCommandDedupeEntry>>,
}

/// BRC-31 auth state: sessions, server identity, wallet for signing.
pub struct AuthState {
    /// BRC-31 server sessions, keyed by server nonce.
    pub brc31_sessions: Mutex<Brc31SessionStore>,
    /// Server identity key (fetched from wallet at startup). Empty if wallet unreachable.
    pub server_identity_key: String,
    /// SDK HttpWalletJson for BRC-31 signing (delegates to wallet service).
    pub wallet: HttpWalletJson,
}

/// Scheduler channels: heartbeat triggers, system events, tick tracking.
pub struct SchedulerChannels {
    /// Sender side of the heartbeat trigger channel. `None` when heartbeat is disabled.
    pub heartbeat_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// Receiver side of the heartbeat trigger channel — drained by the scheduler each tick.
    pub heartbeat_rx: Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>>,
    /// Sender for system events to the scheduler (task completed, config changed, etc.).
    pub system_event_tx: Option<tokio::sync::mpsc::Sender<crate::heartbeat::SystemEvent>>,
    /// Receiver for system events — drained by the scheduler each tick.
    pub system_event_rx:
        Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<crate::heartbeat::SystemEvent>>>>,
    /// Epoch seconds of the last scheduler tick (0 = scheduler hasn't ticked yet).
    pub last_scheduler_tick: AtomicU64,
}

/// Cumulative stats across server restarts (loaded from disk on startup).
pub struct LifetimeStats {
    pub sats_spent: AtomicU64,
    pub task_count: AtomicUsize,
}

/// A staged high-value transaction awaiting manual approval.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StagedTransaction {
    /// Wallet reference for signing or aborting.
    pub reference: String,
    /// Amount in satoshis.
    pub amount_sats: u64,
    /// Service that requested the payment.
    pub service: String,
    /// When the transaction was staged.
    pub created_at: String,
    /// Current status: "pending", "approved", "aborted".
    pub status: String,
    /// Optional task ID that triggered the staging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Description of what the payment is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Shared application state.
pub struct AppState {
    pub config: DmConfig,
    pub workspace: PathBuf,
    /// Shared wallet backend — the single source of truth for all wallet operations.
    /// Created once at startup (embedded wallet when available, HTTP fallback otherwise).
    pub wallet: std::sync::Arc<dyn crate::wallet::WalletBackend + Send + Sync>,
    pub budget: Mutex<BudgetTracker>,
    pub started_at: Instant,
    /// Broadcast channel for event streaming — (task_id, event) pairs.
    pub events_tx: broadcast::Sender<(String, StepEvent)>,
    /// Task tracking: registry, cancel flags, conversation locks.
    pub task_mgr: TaskManager,
    /// BRC-31 auth state.
    pub auth: AuthState,
    /// Scheduler channels and tick tracking.
    pub scheduler: SchedulerChannels,
    /// Lifetime stats from disk.
    pub stats: LifetimeStats,
    /// Receiver for config hot-reload. `None` when watcher is disabled or not started.
    pub config_rx: Option<tokio::sync::watch::Receiver<crate::config::DmConfig>>,
    /// Cached BSV/USD rate with source metadata and fetch timestamp.
    pub usd_rate_cache: RwLock<Option<super::handlers::budget::CachedRate>>,
    /// Cached tool names from a canonical ToolRegistry built at startup.
    /// Avoids rebuilding the registry on every GET /agent call.
    pub cached_tool_names: Vec<String>,
    /// Cached wallet balance with fetch timestamp. 30-second TTL.
    pub cached_balance: RwLock<Option<(u64, Instant)>>,
    /// Staged high-value transactions awaiting manual approval.
    /// Wrapped in `Arc` so it can be shared with the runner for tool approval workflows.
    pub staged_transactions: Arc<Mutex<HashMap<String, StagedTransaction>>>,
    /// MCP wallet client — shared across all tasks. `None` if binary not found.
    pub wallet_mcp: Option<std::sync::Arc<crate::mcp::client::McpClient>>,
    /// MCP wallet tool definitions — registered into each task's ToolRegistry.
    pub wallet_mcp_tools: Vec<(String, String, serde_json::Value)>,
    /// Certificate revocation flag — when true, the agent refuses new tasks.
    /// Set by: boot check, heartbeat periodic check, POST /certificates/revoke.
    /// Cleared by: POST /certificates/issue (new cert acquisition).
    pub certificate_revoked: Arc<AtomicBool>,
    /// x402 per-service rate limiter — constructed from config + cert overrides.
    /// Shared across all tasks. `disabled()` when rate limiting is off.
    pub rate_limiter: Arc<RateLimiterRegistry>,
    /// Shared skill telemetry — accumulated across all tasks.
    pub skill_telemetry: Mutex<crate::skills::SkillTelemetry>,
    /// Skill definitions loaded at startup (name, description, auto_activate).
    pub skill_definitions: Vec<(String, String, bool)>,
    /// Per-task escalation status, keyed by task ID.
    pub escalation_status: Mutex<HashMap<String, crate::runner::escalation::EscalationStatus>>,
    /// Prometheus metrics registry — shared across all tasks and the /metrics endpoint.
    pub metrics: Arc<crate::metrics::MetricsRegistry>,
    /// Model capabilities discovered from x402-info manifests at startup.
    /// Maps model_name -> ModelCapabilities. Pre-fetched best-effort; empty on failure.
    pub model_capabilities: Arc<RwLock<HashMap<String, crate::think::ModelCapabilities>>>,
    /// x402 circuit breaker registry — shared across all tasks for LLM provider failover.
    /// When a provider fails repeatedly, `think_with_circuit_breaker()` routes to the alternate.
    pub circuit_breakers: Arc<crate::x402::circuit_breaker::CircuitBreakerRegistry>,
    /// Shared MessageBox client — single BRC-31 session with messagebox.babbage.systems.
    /// Shared across heartbeat, runner, tools, and reply routing to prevent session churn.
    pub messagebox: Arc<crate::messagebox::client::MessageBoxClient>,
}

/// In-memory dedupe entry for POST /chat idempotency keys.
pub(crate) struct ChatCommandDedupeEntry {
    pub(crate) created_at: Instant,
    pub(crate) state: ChatCommandDedupeState,
}

pub(crate) enum ChatCommandDedupeState {
    Pending(Arc<Notify>),
    Complete { task_id: String, session_id: String },
}

/// Create shared application state.
///
/// Async because it fetches the server identity key from the wallet.
/// The wallet IS the parent — its identity key is used for BRC-31 auth.
pub async fn create_app_state(mut config: DmConfig, workspace: PathBuf) -> Arc<AppState> {
    let mut budget = BudgetTracker::from_config(&config.budget, &workspace);
    let (events_tx, _) = broadcast::channel::<(String, StepEvent)>(4096);

    // Step 0: Create shared wallet backend.
    //
    // Strategy: HTTP-first, embedded fallback.
    //   1. If the configured wallet URL is reachable → use HttpWalletClient (respect running wallet)
    //   2. If not reachable AND embedded-wallet feature AND DB exists → use EmbeddedWalletClient
    //   3. Otherwise → use HttpWalletClient (will fail on first call, same as pre-refactor)
    //
    // We NEVER open an embedded wallet when an external wallet is running — that would cause
    // two processes accessing the same SQLite DB (lock contention, corruption risk).
    let shared_wallet: std::sync::Arc<dyn crate::wallet::WalletBackend + Send + Sync> = {
        let probe = crate::wallet::HttpWalletClient::from_config(&config.wallet);
        let http_reachable =
            tokio::time::timeout(std::time::Duration::from_secs(2), probe.get_identity_key())
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false);

        if http_reachable {
            tracing::info!("Using HTTP wallet at {}", config.wallet.url);
            std::sync::Arc::new(crate::wallet::HttpWalletClient::from_config(&config.wallet))
        } else {
            #[cfg(feature = "embedded-wallet")]
            {
                match crate::wallet::EmbeddedWalletClient::from_config(&config).await {
                    Ok(w) => {
                        tracing::info!(
                            "External wallet not reachable — using embedded wallet (in-process)"
                        );
                        std::sync::Arc::new(w)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Embedded wallet unavailable: {e} — falling back to HTTP client"
                        );
                        std::sync::Arc::new(crate::wallet::HttpWalletClient::from_config(
                            &config.wallet,
                        ))
                    }
                }
            }
            #[cfg(not(feature = "embedded-wallet"))]
            {
                tracing::warn!(
                    "Wallet not reachable at {} — will retry on first call",
                    config.wallet.url
                );
                std::sync::Arc::new(crate::wallet::HttpWalletClient::from_config(&config.wallet))
            }
        }
    };

    // Step 1: Connect to wallet and fetch identity key.
    // The wallet IS the agent — its identity key is the auth key for BRC-31.
    // Every agent must have proper authentication (self-signed or parent-signed cert).
    let (signing_wallet, server_identity_key) = init_wallet(&config).await;
    if !server_identity_key.is_empty() && config.parent.identity_key.is_empty() {
        config.parent.identity_key = server_identity_key.clone();
        tracing::info!(
            "BRC-31 ENABLED — identity key from wallet: {}",
            &server_identity_key[..16.min(server_identity_key.len())]
        );
    }

    // Step 2: BRC-52 certificate check — apply budget limits or mark revoked.
    let certificate_revoked =
        check_certificate_and_apply_budget(&config, &mut budget, shared_wallet.clone()).await;

    // Step 3: x402 rate limiter (cert overrides config).
    let rate_limiter = init_rate_limiter(&config, shared_wallet.clone()).await;

    // Step 4: Scheduler channels for heartbeat + system events.
    let scheduler = init_scheduler_channels(&config);

    // Step 5: Scan workspace/tasks/ for lifetime stats, orphan recovery, task reconstruction.
    let (lifetime_sats, lifetime_tasks, reconstructed_tasks, reconstructed_sessions) =
        scan_historical_tasks(&workspace);

    // Step 6: Connect to wallet MCP server (if available) for typed wallet tools.
    let (wallet_mcp, wallet_mcp_tools) = connect_wallet_mcp(&config).await;

    // Step 7: Build canonical tool registry cache for /agent endpoint.
    let cached_tool_names = build_tool_name_cache(&config, &workspace, &wallet_mcp_tools);

    // Step 8: Load skill definitions for analytics.
    let skill_definitions = load_skill_definitions();

    // Step 8b: Create shared MessageBox client — one BRC-31 session for the whole agent.
    let messagebox = {
        let mb_auth = crate::auth::AuthriteClient::new(shared_wallet.clone(), &config.wallet.url);
        Arc::new(crate::messagebox::client::MessageBoxClient::with_url(
            mb_auth,
            &config.messagebox.url,
        ))
    };

    let app_state = Arc::new(AppState {
        config,
        workspace,
        wallet: shared_wallet,
        budget: Mutex::new(budget),
        started_at: Instant::now(),
        events_tx,
        task_mgr: TaskManager {
            tasks: Mutex::new(reconstructed_tasks),
            cancel_flags: Mutex::new(HashMap::new()),
            task_sessions: Mutex::new(reconstructed_sessions),
            conversation_semaphores: Mutex::new(HashMap::new()),
            active_task_count: Arc::new(AtomicUsize::new(0)),
            chat_command_dedupe: Mutex::new(HashMap::new()),
        },
        auth: AuthState {
            brc31_sessions: Mutex::new(Brc31SessionStore::new()),
            server_identity_key,
            wallet: signing_wallet,
        },
        scheduler,
        stats: LifetimeStats {
            sats_spent: AtomicU64::new(lifetime_sats),
            task_count: AtomicUsize::new(lifetime_tasks),
        },
        config_rx: None, // Set by serve() when config watcher starts
        usd_rate_cache: RwLock::new(None),
        cached_tool_names,
        cached_balance: RwLock::new(None),
        staged_transactions: Arc::new(Mutex::new(HashMap::new())),
        wallet_mcp,
        wallet_mcp_tools,
        certificate_revoked,
        rate_limiter,
        skill_telemetry: Mutex::new(crate::skills::SkillTelemetry::default()),
        skill_definitions,
        escalation_status: Mutex::new(HashMap::new()),
        metrics: Arc::new(crate::metrics::MetricsRegistry::new()),
        model_capabilities: Arc::new(RwLock::new(HashMap::new())),
        circuit_breakers: Arc::new(crate::x402::circuit_breaker::CircuitBreakerRegistry::new()),
        messagebox,
    });

    // Step 9: Pre-fetch model capabilities (best-effort, non-blocking background task).
    spawn_model_capability_fetch(app_state.model_capabilities.clone());

    // Step 10: Overlay registration (check-then-register, best-effort).
    let overlay_config = &app_state.config.overlay;
    if overlay_config.enabled {
        let ow = app_state.wallet.clone();
        let overlay_url = overlay_config.submit_url.clone();
        // Config/env var name always wins — it's the runtime identity.
        // Cert name is a generic fallback ("dolphin-milk-agent").
        let config_agent_name = app_state.config.certificates.agent_name.clone();
        let config_parent_key = app_state.config.parent.identity_key.clone();
        let cert_mgr = crate::certificates::lifecycle::CertificateManager::new(ow.clone());
        let (agent_name, capabilities, certifier) = match cert_mgr.certificate_status().await {
            Ok(status) => {
                let cert = status.certificate.as_ref();
                let name = config_agent_name.clone();
                let caps = cert
                    .and_then(|c| c.get("fields"))
                    .and_then(|f| f.get("capabilities"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool-use")
                    .to_string();

                // Certifier resolution for overlay registration (EPIC #329 Phase 3):
                //
                // 1. **Parent-signed cert** (cert.certifier != cert.subject) — true
                //    BRC-52 parent certification exists. Use the cert's certifier.
                // 2. **Config parent key declared** — operator has configured
                //    `config.parent.identity_key`. Even if we only hold a self-signed
                //    agent cert today, we are OPERATIONALLY under that parent. Publish
                //    the parent key as our certifier so trust-aware discovery
                //    (findByCertifier) can locate us via the operator's declared root.
                //    This is trust-on-claim — the receiver still verifies delegation
                //    certs cryptographically at verify-time.
                // 3. **Self-signed, no parent configured** — fall back to None, which
                //    makes register_on_overlay use the agent's own identity key as a
                //    self-loop certifier. Default dev-mode behavior.
                let cert_subject = cert.and_then(|c| c.get("subject")).and_then(|v| v.as_str());
                let cert_certifier = cert
                    .and_then(|c| c.get("certifier"))
                    .and_then(|v| v.as_str());
                let is_parent_signed =
                    matches!((cert_subject, cert_certifier), (Some(s), Some(c)) if s != c);

                let certifier_key = if is_parent_signed {
                    cert_certifier.map(|s| s.to_string())
                } else if !config_parent_key.is_empty() {
                    tracing::info!(
                        parent_key = %config_parent_key,
                        "Overlay registration: no parent-signed cert, using configured \
                         parent.identity_key as certifier (trust-on-claim)"
                    );
                    Some(config_parent_key.clone())
                } else {
                    None
                };
                (name, caps, certifier_key)
            }
            Err(_) => {
                let certifier_key = if !config_parent_key.is_empty() {
                    Some(config_parent_key.clone())
                } else {
                    None
                };
                (config_agent_name, "tool-use".to_string(), certifier_key)
            }
        };
        tokio::spawn(async move {
            // Check existing registration — if name/capabilities match, skip.
            // If different, spend the old UTXO (removes from overlay) then re-register.
            let needs_register = match crate::overlay::check_registered(&*ow, &overlay_url).await {
                Ok(Some(existing)) => {
                    let caps_csv = capabilities.clone();
                    let caps_match = existing.capabilities.join(",") == caps_csv;
                    // Compare certifier_key too — if the operator changed
                    // config.parent.identity_key between runs, the existing
                    // registration is stale and needs a re-register so
                    // findByCertifier lookups return the current trust root.
                    let expected_certifier = certifier
                        .clone()
                        .unwrap_or_else(|| existing.identity_key.clone());
                    let certifier_match = existing.certifier_key == expected_certifier;
                    if existing.name == agent_name && caps_match && certifier_match {
                        tracing::info!(
                            name = %existing.name,
                            "Already registered on overlay with correct name/capabilities/certifier"
                        );
                        false
                    } else {
                        tracing::info!(
                            old_name = %existing.name,
                            new_name = %agent_name,
                            old_certifier = %existing.certifier_key,
                            new_certifier = %expected_certifier,
                            caps_match,
                            certifier_match,
                            "Overlay registration outdated — spending old and re-registering"
                        );
                        if let Err(e) = crate::overlay::registration::deregister_from_overlay(
                            &*ow,
                            &overlay_url,
                        )
                        .await
                        {
                            tracing::warn!("Failed to spend old registration: {e}");
                        }
                        true
                    }
                }
                Ok(None) => {
                    tracing::info!("Not yet registered on overlay");
                    true
                }
                Err(e) => {
                    tracing::warn!("Overlay check failed: {e} — registering anyway");
                    true
                }
            };

            if !needs_register {
                return;
            }

            tracing::info!(
                name = %agent_name,
                capabilities = %capabilities,
                "Registering on overlay..."
            );
            match crate::overlay::register_on_overlay(
                &*ow,
                &overlay_url,
                &agent_name,
                &capabilities,
                certifier.as_deref(),
            )
            .await
            {
                Ok(txid) => {
                    tracing::info!(txid = %txid, "Overlay registration complete");
                }
                Err(e) => {
                    tracing::warn!("Overlay registration check failed (non-fatal): {e}");
                }
            }
        });
    }

    app_state
}

// ---------------------------------------------------------------------------
// Builder helpers — each encapsulates one logical phase of create_app_state().
// ---------------------------------------------------------------------------

/// Build `HttpWalletJson` and fetch the server identity key from the wallet.
///
/// The identity key is required for BRC-31 handshake response signing.
/// Returns an empty key on wallet errors (non-fatal).
async fn init_wallet(config: &DmConfig) -> (HttpWalletJson, String) {
    // Build HttpWalletJson that delegates to the wallet service.
    // This gives the worm the SAME identity as the wallet at the configured URL.
    let wallet = HttpWalletJson::new(
        Some("dolphin-milk".to_string()),
        Some(config.wallet.url.clone()),
    );

    // Always fetch the server identity key from the wallet so the server can
    // sign BRC-31 handshake responses regardless of whether a parent key is set.
    // Without this, clients that attempt a handshake get an unsigned response
    // and their wallet's verifySignature call hangs.
    let server_identity_key = match wallet
        .get_public_key(
            GetPublicKeyArgs {
                identity_key: true,
                protocol_id: None,
                key_id: None,
                counterparty: None,
                for_self: None,
            },
            "dolphin-milk",
        )
        .await
    {
        Ok(result) => {
            tracing::info!("Wallet identity key: {}", result.public_key);
            result.public_key
        }
        Err(e) => {
            tracing::warn!("Failed to fetch wallet identity key: {e}");
            String::new()
        }
    };

    (wallet, server_identity_key)
}

/// Run the BRC-52 certificate check and apply cert-derived budget limits.
///
/// Returns an `AtomicBool` flag: `true` if the certificate is revoked or missing
/// (tasks will be blocked), `false` if valid.
async fn check_certificate_and_apply_budget(
    config: &DmConfig,
    budget: &mut BudgetTracker,
    cert_wallet: std::sync::Arc<dyn crate::wallet::WalletBackend + Send + Sync>,
) -> Arc<AtomicBool> {
    let certificate_revoked = Arc::new(AtomicBool::new(false));

    let mgr = crate::certificates::CertificateManager::new(cert_wallet.clone());

    // Clean slate: relinquish ALL existing agent-authorization certs
    // (old worm-* certs, stale parent-signed certs, anything).
    // Then check if we have a valid cert. If not, self-sign one.
    if let Ok(certs) = mgr.list_all().await {
        for cert in &certs {
            let c = cert.get("certificate").unwrap_or(cert);
            let ct = c
                .get("certificateType")
                .or_else(|| c.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if ct == crate::certificates::CERT_TYPE_AGENT_AUTH {
                let serial = c
                    .get("serialNumber")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                // Relinquish old certs (worm-* serials) unconditionally
                if serial.starts_with("worm-") {
                    tracing::info!("Relinquishing old cert: {serial}");
                    let _ = mgr.relinquish(c).await;
                }
            }
        }
    }

    // Cert acquisition with capability-aware re-issue + cleanup.
    //
    // This block is the single source of truth for BRC-52 agent-authorization
    // cert provisioning at boot. Goals:
    //
    //   1. If the wallet already holds a parent-signed cert whose `fields.name`
    //      and `fields.capabilities` match the desired state, reuse it (no-op).
    //   2. Otherwise, acquire a new parent-signed cert from the configured
    //      parent wallet with exactly the desired capabilities. Fall back to
    //      self-signed if the parent is unreachable so the agent still boots.
    //   3. After acquisition (or even if acquisition was skipped), revoke and
    //      relinquish any parent-signed cert whose capabilities DO NOT match
    //      the desired CSV. This enforces exactly-one-cert-per-role state so
    //      `certificate_status()` is deterministic, which keeps the async
    //      overlay registration that runs next from picking up stale caps.
    //
    // Desired capabilities come from `config.certificates.capabilities` (env:
    // `DOLPHIN_MILK_CERT_CAPABILITIES`), falling back to the legacy default
    // set when unset. Test harnesses and per-role production agents set this.
    let default_caps_csv = "llm,tools,wallet,memory,messaging,x402,schedule,orchestration";
    let desired_caps_csv = config
        .certificates
        .capabilities
        .clone()
        .unwrap_or_else(|| default_caps_csv.to_string());
    let capabilities_list: Vec<&str> = desired_caps_csv
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    // Apply budget limits from whatever cert currently validates. Done eagerly
    // so the in-flight loop has correct caps before the (possible) re-issue
    // replaces them. Re-applied from the new cert after acquisition below.
    let cert_result = crate::certificates::check_authorization(cert_wallet.clone()).await;
    if matches!(cert_result, crate::certificates::CertCheckResult::Valid) {
        let cl = crate::certificates::read_cert_budget_limits(cert_wallet.clone()).await;
        if cl.per_task.is_some()
            || cl.per_hour.is_some()
            || cl.per_day.is_some()
            || cl.per_week.is_some()
            || cl.per_month.is_some()
            || cl.lifetime.is_some()
            || cl.enforcement.is_some()
        {
            budget.apply_cert_limits(
                cl.per_task,
                cl.per_hour,
                cl.per_day,
                cl.per_week,
                cl.per_month,
                cl.lifetime,
                cl.enforcement.clone(),
            );
            tracing::info!(
                "Budget tracker updated with cert limits: task={:?} hour={:?} day={:?}",
                cl.per_task,
                cl.per_hour,
                cl.per_day
            );
        }
    }

    let has_matching = mgr
        .has_matching_parent_cert(&config.certificates.agent_name, &desired_caps_csv)
        .await
        .unwrap_or(false);

    if has_matching {
        tracing::info!(
            name = %config.certificates.agent_name,
            capabilities = %desired_caps_csv,
            "BRC-52: existing parent-signed cert matches desired state — skipping re-issue"
        );
    } else if !config.parent.wallet_url.is_empty() {
        // Try parent-signed acquisition via configured parent wallet. This is
        // the real trust-root path — cert.certifier becomes the parent key, so
        // `delegate_task::resolve_root_certifier()` will stamp parent_key as
        // the delegation root.
        tracing::info!(
            parent_url = %config.parent.wallet_url,
            name = %config.certificates.agent_name,
            capabilities = %desired_caps_csv,
            "BRC-52: requesting parent-signed certificate from parent wallet"
        );

        // Clean up any stale self-signed cert first (UNIQUE constraint).
        if let Err(e) = mgr.relinquish_self_signed().await {
            tracing::debug!("relinquish_self_signed (non-fatal): {e}");
        }

        let parent_wallet =
            crate::wallet::HttpWalletClient::new(&config.parent.wallet_url, "http://localhost", 10);

        match mgr
            .acquire_parent_authorization(
                &parent_wallet,
                &config.certificates.agent_name,
                &capabilities_list,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
        {
            Ok(_) => {
                tracing::info!(
                    "BRC-52: parent-signed certificate acquired from {} with capabilities '{}'",
                    config.parent.wallet_url,
                    desired_caps_csv
                );
                // Re-apply budget limits from the freshly-acquired cert so the
                // in-flight loop reflects the new state immediately.
                let cl = crate::certificates::read_cert_budget_limits(cert_wallet.clone()).await;
                if cl.per_task.is_some()
                    || cl.per_hour.is_some()
                    || cl.per_day.is_some()
                    || cl.per_week.is_some()
                    || cl.per_month.is_some()
                    || cl.lifetime.is_some()
                    || cl.enforcement.is_some()
                {
                    budget.apply_cert_limits(
                        cl.per_task,
                        cl.per_hour,
                        cl.per_day,
                        cl.per_week,
                        cl.per_month,
                        cl.lifetime,
                        cl.enforcement.clone(),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    "BRC-52: parent-signed cert acquisition failed ({e}) — \
                     falling back to self-sign so the agent can still boot"
                );
                if let Err(e2) = mgr
                    .acquire_authorization(
                        &config.certificates.agent_name,
                        &capabilities_list,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await
                {
                    tracing::error!("BRC-52: self-sign fallback also failed: {e2}");
                } else {
                    tracing::info!("BRC-52: self-signed certificate issued (fallback)");
                }
            }
        }
    } else {
        // No parent wallet configured — pure self-sign path. Only issue if
        // the wallet has nothing at all.
        let has_any = mgr.has_authorization().await.unwrap_or(false);
        if !has_any {
            tracing::info!("BRC-52: no parent wallet configured — self-signing");
            match mgr
                .acquire_authorization(
                    &config.certificates.agent_name,
                    &capabilities_list,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
            {
                Ok(_) => tracing::info!("BRC-52: self-signed certificate issued"),
                Err(e) => tracing::error!("BRC-52: failed to self-sign certificate: {e}"),
            }
        } else {
            tracing::info!("BRC-52: existing certificate found — skipping re-issue");
        }
    }

    // Cleanup pass: revoke and relinquish any parent-signed cert whose
    // (name, capabilities) does NOT match the desired state. This runs
    // unconditionally so duplicates from prior runs (e.g., ancient default-caps
    // certs left over from earlier cascade tests) are removed deterministically.
    // Matching certs are kept. `revoke_and_relinquish()` spends the revocation
    // UTXO, creates a BRC-18 audit proof, and relinquishes the cert from the
    // wallet store — canonical two-step revocation matching `POST /certificates/revoke`.
    if let Ok(certs) = mgr.list_auth_certs().await {
        let mut revoked_count = 0usize;
        for entry in &certs {
            let c = entry.get("certificate").unwrap_or(entry);
            let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            // Only touch parent-signed certs.
            if subject.is_empty() || certifier.is_empty() || certifier == subject {
                continue;
            }
            let cert_name = c
                .get("fields")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let cert_caps = c
                .get("fields")
                .and_then(|f| f.get("capabilities"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Keep any parent-signed cert that matches both name AND capabilities.
            // Non-matching certs are stale and get revoked.
            if cert_name == config.certificates.agent_name && cert_caps == desired_caps_csv {
                continue;
            }
            let serial = c
                .get("serialNumber")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            tracing::info!(
                stale_serial = %serial,
                stale_name = %cert_name,
                stale_caps = %cert_caps,
                desired_name = %config.certificates.agent_name,
                desired_caps = %desired_caps_csv,
                "BRC-52: revoking stale parent-signed cert"
            );
            if let Err(e) = mgr.revoke_and_relinquish(c).await {
                tracing::warn!(
                    stale_serial = %serial,
                    error = %e,
                    "BRC-52: revoke_and_relinquish failed (non-fatal)"
                );
            } else {
                revoked_count += 1;
            }
        }
        if revoked_count > 0 {
            tracing::info!(revoked_count, "BRC-52: stale cert cleanup complete");
        }
    }

    // Never block tasks on cert issues — the agent should always be able to work
    certificate_revoked
}

/// Build the x402 rate limiter from config, overridden by cert-derived rate limits.
async fn init_rate_limiter(
    config: &DmConfig,
    wallet: std::sync::Arc<dyn crate::wallet::WalletBackend + Send + Sync>,
) -> Arc<RateLimiterRegistry> {
    let cert_rate_limits = crate::certificates::read_cert_rate_limits(wallet).await;
    Arc::new(RateLimiterRegistry::new(
        &(&config.x402.rate_limits).into(),
        &cert_rate_limits,
    ))
}

/// Create heartbeat trigger and system event channels when the scheduler is enabled.
fn init_scheduler_channels(config: &DmConfig) -> SchedulerChannels {
    // Create heartbeat trigger channel when heartbeat is enabled
    let (heartbeat_tx, heartbeat_rx) = if config.heartbeat.enabled {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
        (Some(tx), Some(Arc::new(tokio::sync::Mutex::new(rx))))
    } else {
        (None, None)
    };

    // Create system event channel when heartbeat is enabled
    let (system_event_tx, system_event_rx) = if config.heartbeat.enabled {
        let (tx, rx) = tokio::sync::mpsc::channel::<crate::heartbeat::SystemEvent>(64);
        (Some(tx), Some(Arc::new(tokio::sync::Mutex::new(rx))))
    } else {
        (None, None)
    };

    SchedulerChannels {
        heartbeat_tx,
        heartbeat_rx,
        system_event_tx,
        system_event_rx,
        last_scheduler_tick: AtomicU64::new(0),
    }
}

/// Scan `workspace/tasks/` to compute lifetime spending totals from historical transcripts.
///
/// Also recovers orphaned tasks (have transcript events but no `session_end`),
/// and reconstructs `TaskInfo` entries so `GET /task/{id}` works after restart.
///
/// Returns `(lifetime_sats, lifetime_tasks, reconstructed_tasks, reconstructed_sessions)`.
fn scan_historical_tasks(
    workspace: &Path,
) -> (
    u64,
    usize,
    HashMap<String, super::TaskInfo>,
    HashMap<String, String>,
) {
    let tasks_dir = workspace.join("tasks");
    let mut total_sats: u64 = 0;
    let mut total_tasks: usize = 0;
    let mut orphans_recovered: usize = 0;
    let mut task_map: HashMap<String, super::TaskInfo> = HashMap::new();
    let mut session_map: HashMap<String, String> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let transcript_path = path.join("session.jsonl");
            if !transcript_path.exists() {
                continue;
            }
            let transcript = crate::transcript::Transcript::new(transcript_path.clone());
            total_sats += transcript.total_sats_spent();
            total_tasks += 1;

            let task_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Reconstruct TaskInfo from transcript using typed SessionState.
            // Orphaned tasks (no session_end) are marked Interrupted, not Error.
            // The transcript is NOT modified — interrupted tasks can be resumed.
            let session_state = transcript.reconstruct_state();
            let events = transcript.replay();
            if !events.is_empty() {
                let is_orphan = session_state.is_interrupted();
                if is_orphan {
                    tracing::warn!(
                        "Interrupted task {} — {} events, {} iterations, {} sats spent. Resumable via POST /task/{}/resume.",
                        task_id, events.len(), session_state.iterations, session_state.sats_spent, task_id
                    );
                    orphans_recovered += 1;
                }

                let status = if is_orphan {
                    super::TaskStatus::Interrupted
                } else if session_state.error.is_some() {
                    super::TaskStatus::Error
                } else {
                    super::TaskStatus::Complete
                };

                let started_at = events
                    .first()
                    .map(|e| {
                        let secs = e.ts as i64;
                        let nanos = ((e.ts - secs as f64) * 1_000_000_000.0) as u32;
                        chrono::DateTime::from_timestamp(secs, nanos)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();

                let completed_at = if is_orphan {
                    None
                } else {
                    events.last().map(|e| {
                        let secs = e.ts as i64;
                        let nanos = ((e.ts - secs as f64) * 1_000_000_000.0) as u32;
                        chrono::DateTime::from_timestamp(secs, nanos)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_default()
                    })
                };

                let result = if is_orphan {
                    None
                } else {
                    events
                        .iter()
                        .rev()
                        .find(|e| e.event_type == "session_end")
                        .and_then(|e| e.data.get("result"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                };

                // Extract session_id from session_start event
                let session_id = events
                    .iter()
                    .find(|e| e.event_type == "session_start")
                    .and_then(|e| e.data.get("session_id").or(e.data.get("conversation_id")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if let Some(ref sid) = session_id {
                    session_map.insert(task_id.clone(), sid.clone());
                }

                task_map.insert(
                    task_id.clone(),
                    super::TaskInfo {
                        id: task_id,
                        task: session_state.task.clone(),
                        status,
                        result,
                        error: session_state.error.clone(),
                        iterations: session_state.iterations,
                        sats_spent: session_state.sats_spent,
                        started_at,
                        completed_at,
                        proof_txids: {
                            let mut all_txids = session_state.proof_txids.clone();
                            all_txids.extend(session_state.checkpoint_txids.iter().cloned());
                            all_txids
                        },
                        tags: Vec::new(),
                        origin: String::new(),
                        conversation_id: session_id.clone(),
                    },
                );
            }
        }
    }
    if orphans_recovered > 0 {
        tracing::info!(
            "Recovered {} orphaned task(s) at startup",
            orphans_recovered
        );
    }
    let reconstructed = task_map.len();
    tracing::info!(
        "Lifetime stats from disk: {} tasks, {} sats spent, {} task(s) reconstructed",
        total_tasks,
        total_sats,
        reconstructed,
    );
    (total_sats, total_tasks, task_map, session_map)
}

/// Connect to the wallet MCP server (if available) for typed wallet tools.
///
/// Returns `(None, [])` if the MCP binary is not found or connection fails.
async fn connect_wallet_mcp(
    config: &DmConfig,
) -> (
    Option<std::sync::Arc<crate::mcp::client::McpClient>>,
    Vec<(String, String, serde_json::Value)>,
) {
    match crate::mcp::client::connect_wallet_mcp(&config.mcp.wallet_mcp_command, &config.wallet.url)
        .await
    {
        Ok(Some((client, defs))) => {
            let summaries: Vec<(String, String, serde_json::Value)> = defs
                .iter()
                .map(|d| (d.name.clone(), d.description.clone(), d.parameters.clone()))
                .collect();
            (Some(client), summaries)
        }
        Ok(None) => (None, Vec::new()),
        Err(e) => {
            tracing::warn!("Wallet MCP connection error: {e}");
            (None, Vec::new())
        }
    }
}

/// Build a canonical tool registry once at startup for the /agent endpoint cache.
///
/// Uses the same construction logic as the runner to ensure tool count consistency.
fn build_tool_name_cache(
    config: &DmConfig,
    workspace: &Path,
    wallet_mcp_tools: &[(String, String, serde_json::Value)],
) -> Vec<String> {
    use crate::tools::memory_tools::all_memory_tools;
    use crate::tools::messagebox_tools::all_messagebox_tools;
    use crate::tools::registry::ToolRegistry;
    use crate::tools::sandbox::all_sandbox_tools;
    use crate::tools::wallet_tools::all_wallet_tools;
    use crate::tools::x402_tools::all_x402_tools;

    let mut registry = ToolRegistry::new();
    let ws = workspace.to_path_buf();
    let memory_dir = ws.join("memory");
    for tool in all_sandbox_tools(ws, config.llm.context_window) {
        registry.register(tool);
    }
    for tool in all_wallet_tools(config.wallet.url.clone()) {
        registry.register(tool);
    }
    for tool in all_memory_tools(memory_dir) {
        registry.register(tool);
    }
    for tool in all_messagebox_tools(config.wallet.url.clone()) {
        registry.register(tool);
    }
    for tool in all_x402_tools(config.wallet.url.clone(), config.x402.registry_url.clone()) {
        registry.register(tool);
    }
    // Include MCP wallet tool names in the cache
    for (name, _, _) in wallet_mcp_tools {
        registry.register(crate::tools::registry::ToolDef {
            name: name.clone(),
            description: String::new(),
            parameters: serde_json::Value::Null,
            execute: Box::new(|_| Box::pin(async { String::new() })),
            category: "wallet".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        });
    }
    let mut names: Vec<String> = registry.tool_names();
    names.sort();
    names
}

/// Load skill definitions from the `skills/` directory for the analytics endpoint.
fn load_skill_definitions() -> Vec<(String, String, bool)> {
    // Try CWD skills/ first (backward compat), then data_dir skills/
    let cwd_skills = std::path::Path::new("skills");
    let skills_dir = if cwd_skills.exists() {
        cwd_skills.to_path_buf()
    } else {
        // Fall back to data_dir/skills — requires config, so use default
        crate::config::DmConfig::default().skills_dir()
    };
    let skill_registry = crate::skills::SkillRegistry::load_from_dir(&skills_dir);
    skill_registry
        .all()
        .iter()
        .map(|s| (s.name.clone(), s.description.clone(), s.auto_activate))
        .collect()
}

/// Spawn a background task to pre-fetch model capabilities from x402-info manifests.
///
/// Populates the cache so the runner can use discovered limits instead of hardcoded.
/// Best-effort — failures are logged and the runner falls back to defaults.
fn spawn_model_capability_fetch(
    caps_cache: Arc<RwLock<HashMap<String, crate::think::ModelCapabilities>>>,
) {
    tokio::spawn(async move {
        let mut all_caps: HashMap<String, crate::think::ModelCapabilities> = HashMap::new();
        for base_url in &[
            "https://openai-chat.x402agency.com",
            "https://claude-chat.x402agency.com",
        ] {
            match crate::x402::discovery::fetch_manifest(base_url).await {
                Ok(manifest) => {
                    let discovered = crate::x402::discovery::parse_model_capabilities(&manifest);
                    for (name, cap) in discovered {
                        all_caps.insert(
                            name.clone(),
                            crate::think::ModelCapabilities::from_manifest(
                                &name,
                                cap.context_window,
                                cap.max_output_tokens,
                                cap.supports_tools,
                                cap.supports_vision,
                            ),
                        );
                    }
                    tracing::info!(
                        "Discovered {} model capabilities from {}",
                        all_caps.len(),
                        base_url
                    );
                }
                Err(e) => {
                    tracing::debug!("Could not fetch manifest from {}: {}", base_url, e);
                }
            }
        }
        if !all_caps.is_empty() {
            let mut cache = caps_cache.write().await;
            *cache = all_caps;
        }
    });
}

const CHAT_COMMAND_DEDUPE_TTL_SECS: u64 = 60 * 60;
const CHAT_COMMAND_PENDING_TTL_SECS: u64 = 60;

pub(crate) fn prune_chat_command_dedupe(entries: &mut HashMap<String, ChatCommandDedupeEntry>) {
    let now = Instant::now();
    entries.retain(|_, entry| {
        let age_secs = now.duration_since(entry.created_at).as_secs();
        match entry.state {
            ChatCommandDedupeState::Pending(_) => age_secs < CHAT_COMMAND_PENDING_TTL_SECS,
            ChatCommandDedupeState::Complete { .. } => age_secs < CHAT_COMMAND_DEDUPE_TTL_SECS,
        }
    });
}

/// All `x-bsv-auth-*` headers that BRC-31 clients send.
const BSV_AUTH_HEADERS: [&str; 7] = [
    "x-bsv-auth-version",
    "x-bsv-auth-identity-key",
    "x-bsv-auth-message-type",
    "x-bsv-auth-nonce",
    "x-bsv-auth-your-nonce",
    "x-bsv-auth-signature",
    "x-bsv-auth-request-id",
];

/// Build the axum router from shared state.
pub fn build_router_with_state(state: Arc<AppState>) -> Router {
    let has_auth = !state.auth.server_identity_key.is_empty();

    use super::handlers::{
        agent, analytics, audit, budget, chat, compliance, conversations, marketplace, memory,
        misc, replay, schedules, staging, tasks, wallet_ops,
    };

    let router = Router::new()
        // Public routes (no auth)
        .route("/health", get(agent::health))
        .route("/.well-known/auth", post(super::auth::brc31_handshake))
        // Existing API routes (unchanged)
        .route("/task", post(tasks::submit_task))
        .route("/status", get(tasks::get_status))
        .route("/task/{id}", get(tasks::get_task))
        .route("/message", post(misc::receive_message))
        // Phase 3 — dashboard API
        .route("/tasks", get(tasks::list_tasks))
        .route("/task/{id}/audit", get(tasks::get_audit))
        .route("/task/{id}/audit/export", get(tasks::get_audit_export))
        .route("/task/{id}/proofs", get(tasks::get_proofs))
        .route("/task/{id}/proofs/verify", get(tasks::verify_proofs))
        .route("/task/{id}/proofs/custody", get(tasks::get_custody_proof))
        .route("/output/{basket}/{txid}", get(wallet_ops::get_output))
        .route("/decrypt", post(wallet_ops::decrypt_data))
        .route("/wallet/address", get(wallet_ops::get_funding_address))
        .route("/wallet/check-funding", post(wallet_ops::check_funding))
        .route("/wallet/split", post(wallet_ops::split_wallet))
        .route("/wallet/utxos", get(wallet_ops::get_wallet_utxos))
        .route("/task/{id}/receipts", get(tasks::get_receipts))
        .route("/task/{id}/conversation", get(tasks::get_conversation))
        .route("/task/{id}/events", get(tasks::get_task_events))
        .route("/task/{id}/artifacts", get(tasks::get_artifacts))
        .route(
            "/task/{id}/conversation-id",
            get(tasks::get_task_conversation_id),
        )
        .route("/artifacts", get(tasks::get_all_artifacts))
        .route(
            "/task/{id}/transcript/verify",
            get(tasks::verify_transcript),
        )
        .route("/task/{id}/replay", get(replay::get_replay))
        .route("/task/{id}/fork", post(replay::fork_task))
        .route("/task/{id}/cancel", post(tasks::cancel_task))
        .route("/task/{id}/resume", post(tasks::resume_task))
        .route("/agent", get(agent::get_agent))
        .route("/analytics/skills", get(agent::get_skill_analytics))
        // Certificate management
        .route("/certificates", get(agent::get_certificates))
        .route("/certificates/issue", post(agent::issue_certificate))
        .route(
            "/certificates/relinquish",
            post(agent::relinquish_certificate),
        )
        .route("/certificates/revoke", post(agent::revoke_certificate))
        // Overlay re-registration (staleness-aware, idempotent)
        .route(
            "/overlay/reregister",
            post(agent::reregister_overlay_handler),
        )
        // File serving for task workspace media
        .route("/files/{task_id}/{filename}", get(tasks::serve_task_file))
        // Conversation endpoints
        .route("/conversations", get(conversations::list_conversations))
        .route(
            "/conversations/{id}",
            get(conversations::get_conversation_detail),
        )
        .route(
            "/conversations/{id}/artifacts",
            get(conversations::get_conversation_artifacts),
        )
        .route(
            "/conversations/{id}/verify",
            get(conversations::verify_conversation),
        )
        .route(
            "/conversations/{id}/sync",
            post(conversations::sync_conversation),
        )
        .route(
            "/conversations/{id}/compact",
            post(conversations::compact_conversation),
        )
        // Memory endpoints (search MUST come before {id} to avoid capture)
        .route("/memory", get(memory::list_memories))
        .route("/memory/search", get(memory::search_memories))
        .route("/memory/{id}", get(memory::get_memory))
        // Budget detail endpoint
        .route("/budget/detail", get(budget::get_budget_detail))
        // Analytics — efficiency trends, cost comparison, ROI, benchmarks
        .route("/analytics/efficiency", get(analytics::get_efficiency))
        .route(
            "/analytics/cost-comparison",
            get(analytics::get_cost_comparison),
        )
        .route("/analytics/roi", get(analytics::get_roi))
        .route("/analytics/benchmarks", get(analytics::get_benchmarks))
        // Context window analysis
        .route("/context/analysis", get(analytics::get_context_analysis))
        // Budget export (JSON or PDF)
        .route("/budget/export", get(budget::get_budget_export))
        // Prometheus metrics endpoint
        .route("/metrics", get(get_metrics))
        // Exchange rate proxy
        .route("/rates/bsv-usd", get(budget::get_bsv_usd_rate))
        // Schedule endpoints
        .route("/schedules", get(schedules::list_schedules_endpoint))
        .route("/schedules/{id}", get(schedules::get_schedule_endpoint))
        // Lifecycle status
        .route("/lifecycle/status", get(compliance::lifecycle_status))
        // Compliance report
        .route("/compliance/report", get(compliance::compliance_report))
        // Audit key linkage (BRC-69/70), cross-agent proof linking, and full-text search
        .route("/audit/search", get(audit::audit_search))
        .route("/audit/revelations", get(audit::list_revelations))
        .route(
            "/audit/key-linkage/counterparty",
            post(audit::reveal_counterparty_linkage),
        )
        .route(
            "/audit/key-linkage/specific",
            post(audit::reveal_specific_linkage),
        )
        .route(
            "/audit/cross-reference/{message_hash}",
            get(audit::cross_reference_message),
        )
        // Analytics
        .route("/analytics/time-saved", get(tasks::get_time_saved))
        // Transaction staging (two-phase commit for high-value operations)
        .route("/staged", get(staging::list_staged))
        .route("/staged/{reference}/approve", post(staging::approve_staged))
        .route("/staged/{reference}/abort", post(staging::abort_staged))
        // Service catalog (public, no auth)
        .route("/services/catalog", get(marketplace::services_catalog))
        // Marketplace plugin API
        .route("/marketplace/plugins", get(marketplace::list_plugins))
        .route("/marketplace/plugins", post(marketplace::create_plugin))
        .route("/marketplace/plugins/{name}", get(marketplace::get_plugin))
        .route(
            "/marketplace/plugins/{name}",
            axum::routing::delete(marketplace::delete_plugin),
        )
        // Task escalation API
        .route("/task/{id}/escalation", get(tasks::get_escalation))
        .route(
            "/task/{id}/escalation/proof",
            get(tasks::get_escalation_proof),
        )
        .route(
            "/task/{id}/escalation/resolve",
            post(tasks::resolve_escalation),
        )
        // Heartbeat trigger
        .route("/heartbeat/trigger", post(misc::trigger_heartbeat))
        // BRC-31 authenticated routes (auth checked inside handler when key is set)
        .route("/chat", post(chat::chat))
        .route("/chat/history", get(chat::chat_history))
        .route("/budget", get(budget::get_budget));

    // Static file serving for UI — embedded (default) or filesystem fallback
    #[cfg(feature = "embed-ui")]
    let mut router = {
        let ui_router = axum::Router::new().fallback(super::ui_assets::serve_embedded_ui);
        router
            .route(
                "/ui/",
                axum::routing::get(super::ui_assets::serve_embedded_ui),
            )
            .nest("/ui", ui_router)
    };

    #[cfg(not(feature = "embed-ui"))]
    let mut router = router.nest_service(
        "/ui",
        ServeDir::new("ui/dist").append_index_html_on_directories(true),
    );

    // OpenAI-compatible endpoint — only registered when config enables it
    if state.config.server.openai_compat_enabled || !has_auth {
        router = router.route("/v1/chat/completions", post(chat::openai_chat_completions));
    }

    let mut router = router.with_state(state);

    // CORS: open when no auth; restricted when BRC-31 auth is active
    let cors = if !has_auth {
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        let auth_headers: Vec<HeaderName> = BSV_AUTH_HEADERS
            .iter()
            .map(|h| h.parse::<HeaderName>().unwrap())
            .chain(std::iter::once(header::CONTENT_TYPE))
            .collect();
        let expose_headers: Vec<HeaderName> = BSV_AUTH_HEADERS
            .iter()
            .map(|h| h.parse::<HeaderName>().unwrap())
            .collect();
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers(auth_headers)
            .expose_headers(expose_headers)
    };
    router = router.layer(cors);

    router
}

/// GET /metrics — Prometheus text-format metrics exposition (BRC-31 auth protected).
async fn get_metrics(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    // BRC-31 auth check — same as all other protected endpoints
    let (parts, body_bytes) = {
        let (p, b) = req.into_parts();
        let bytes = axum::body::to_bytes(b, 1024 * 64).await.unwrap_or_default();
        (p, bytes)
    };
    let auth_ctx = match super::auth::check_brc31_auth(
        &state,
        parts.method.as_str(),
        parts.uri.path(),
        parts.uri.query(),
        &parts.headers,
        Some(&body_bytes),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(status) => {
            return axum::response::Response::builder()
                .status(status)
                .body(axum::body::Body::empty())
                .unwrap()
        }
    };

    // Sync the active_tasks gauge from the authoritative AtomicUsize counter.
    let active = state
        .task_mgr
        .active_task_count
        .load(std::sync::atomic::Ordering::Relaxed) as i64;
    state.metrics.active_tasks.set(active);

    let body = state.metrics.gather();
    match super::auth::signed_raw_response(
        &state,
        auth_ctx,
        axum::http::StatusCode::OK,
        "text/plain; version=0.0.4; charset=utf-8",
        body.into_bytes(),
        vec![],
    )
    .await
    {
        Ok(resp) => resp,
        Err(status) => axum::response::Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .unwrap(),
    }
}

/// Build the axum router with shared state (convenience wrapper).
pub async fn build_router(config: DmConfig, workspace: PathBuf) -> Router {
    let state = create_app_state(config, workspace).await;
    build_router_with_state(state)
}

/// Background task: refresh BSV/USD rate at the configured interval.
pub async fn spawn_rate_refresh(state: Arc<AppState>) {
    use super::handlers::budget::{fetch_rate_multi_source, CachedRate};

    // Seed the cache immediately
    if let Ok((rate, source)) = fetch_rate_multi_source().await {
        let mut cache = state.usd_rate_cache.write().await;
        *cache = Some(CachedRate {
            rate,
            source: source.clone(),
            fetched_at: Instant::now(),
            updated_at_rfc3339: chrono::Utc::now().to_rfc3339(),
        });
        tracing::info!("BSV/USD rate seeded: ${:.2} (source: {source})", rate);
    }

    loop {
        let interval = state.config.rates.refresh_interval_secs;
        tokio::time::sleep(Duration::from_secs(interval)).await;
        match fetch_rate_multi_source().await {
            Ok((rate, source)) => {
                let mut cache = state.usd_rate_cache.write().await;
                *cache = Some(CachedRate {
                    rate,
                    source: source.clone(),
                    fetched_at: Instant::now(),
                    updated_at_rfc3339: chrono::Utc::now().to_rfc3339(),
                });
                tracing::debug!("BSV/USD rate refreshed: ${:.2} (source: {source})", rate);
            }
            Err(e) => {
                // Keep the stale cache — better than nothing
                tracing::warn!("BSV/USD rate refresh failed (keeping cached value): {e}");
            }
        }
    }
}

/// Start the HTTP server, optionally with the heartbeat daemon.
pub async fn serve(
    config: DmConfig,
    workspace: PathBuf,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let heartbeat_enabled = config.heartbeat.enabled;
    let heartbeat_poll_secs = config.heartbeat.inbox_poll_secs;
    #[allow(unused_assignments)]
    let mut has_auth_key = false;

    let mut state = create_app_state(config.clone(), workspace).await;
    has_auth_key = !state.auth.server_identity_key.is_empty();

    // Start config file watcher for hot-reload (before building router to get exclusive Arc access)
    // Search order: config.config_path() (data_dir/dolphin-milk.toml), then CWD dolphin-milk.toml
    let config_path = {
        let data_dir_path = config.config_path();
        if data_dir_path.exists() {
            data_dir_path
        } else {
            std::path::PathBuf::from("dolphin-milk.toml")
        }
    };
    if config_path.exists() {
        match crate::config::ConfigWatcher::start(config_path, config.clone()) {
            Ok((_watcher, rx)) => {
                // Leak watcher to keep it alive for process lifetime
                Box::leak(Box::new(_watcher));
                if let Some(inner) = Arc::get_mut(&mut state) {
                    inner.config_rx = Some(rx);
                }
                println!("  Config watcher: ENABLED (watching dolphin-milk.toml)");
            }
            Err(e) => {
                tracing::warn!("config watcher failed to start: {e}");
                println!("  Config watcher: FAILED ({e})");
            }
        }
    } else {
        println!("  Config watcher: DISABLED (no dolphin-milk.toml found)");
    }

    let router = build_router_with_state(Arc::clone(&state));

    // Spawn scheduler (upgraded heartbeat daemon) if enabled, with panic-recovery wrapper
    if heartbeat_enabled {
        let max_concurrent = config.heartbeat.max_concurrent_tasks;
        let sched_config = config.clone();
        let sched_workspace = state.workspace.clone();
        let sched_state = Arc::clone(&state);

        tokio::spawn(async move {
            loop {
                let cfg = sched_config.clone();
                let ws = sched_workspace.clone();
                let st = Arc::clone(&sched_state);
                let (scheduler, _wake_tx) = Scheduler::new(cfg, ws, st);
                let handle = tokio::spawn(scheduler.run());

                match handle.await {
                    Ok(()) => {
                        tracing::error!("Scheduler exited unexpectedly — restarting in 5s");
                    }
                    Err(e) => {
                        tracing::error!("Scheduler panicked: {e} — restarting in 5s");
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });

        println!(
            "  Scheduler: ENABLED (polling every {heartbeat_poll_secs}s, max {max_concurrent} concurrent tasks)"
        );
    } else {
        println!("  Scheduler: DISABLED");
    }

    // Spawn background BSV/USD rate refresh (every 15 minutes)
    tokio::spawn(spawn_rate_refresh(Arc::clone(&state)));

    let addr = format!("0.0.0.0:{port}");

    tracing::info!("Starting server on {addr}");
    println!("Dolphin Milk server listening on http://{addr}");
    println!("  POST /task             — submit a task");
    println!("  GET  /status           — list all tasks");
    println!("  GET  /task/{{id}}        — get task status");
    println!("  GET  /tasks            — list all tasks (memory + disk)");
    println!("  GET  /task/{{id}}/audit  — full audit chain");
    println!("  GET  /task/{{id}}/proofs — proof details");
    println!("  POST /message          — forward MessageBox message");
    println!("  POST /heartbeat/trigger— manual heartbeat trigger (BRC-31 auth)");
    println!("  GET  /budget           — budget report (BRC-31 auth)");
    println!("  GET  /agent            — agent identity & stats");
    println!("  GET  /health           — liveness check");
    println!("  POST /.well-known/auth — BRC-31 handshake");
    println!("  POST /chat             — chat (BRC-31 auth)");
    println!("  GET  /chat/history     — conversation history (BRC-31 auth)");
    println!("  GET  /ui/              — web interface");
    if has_auth_key {
        let key = &state.auth.server_identity_key;
        println!("  Auth: BRC-31 ENABLED");
        println!("  Identity: {key}");
    } else {
        println!("  Auth: DISABLED (wallet unreachable at startup)");
        println!("  Ensure bsv-wallet-cli is running on the configured URL.");
    }

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Graceful shutdown: catch SIGTERM/SIGINT, drain active tasks, then exit
    let shutdown_state = Arc::clone(&state);
    let shutdown_signal = async move {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => { println!("\nReceived SIGINT (Ctrl+C), shutting down..."); }
            _ = terminate => { println!("\nReceived SIGTERM, shutting down..."); }
        }

        // Phase 1: Stop accepting new tasks
        shutdown_state
            .certificate_revoked
            .store(true, std::sync::atomic::Ordering::SeqCst);
        println!("  New tasks blocked");

        // Phase 2: Cancel active tasks and wait for them to drain
        let active = shutdown_state
            .task_mgr
            .active_task_count
            .load(std::sync::atomic::Ordering::SeqCst);
        if active > 0 {
            println!("  Waiting for {active} active task(s) to complete...");

            // Set cancel flags on all active tasks
            {
                let flags = shutdown_state.task_mgr.cancel_flags.lock().await;
                for flag in flags.values() {
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }

            // Wait up to 30 seconds for tasks to drain
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
            loop {
                let remaining = shutdown_state
                    .task_mgr
                    .active_task_count
                    .load(std::sync::atomic::Ordering::SeqCst);
                if remaining == 0 {
                    println!("  All tasks drained");
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    println!("  Shutdown timeout: {remaining} task(s) still active, forcing exit");
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
            }
        }

        // Phase 3: Emit shutdown system event (stops scheduler)
        if let Some(tx) = &shutdown_state.scheduler.system_event_tx {
            let _ = tx.send(crate::heartbeat::SystemEvent::ConfigChanged).await;
        }

        println!("  Shutdown complete");
    };

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    Ok(())
}
