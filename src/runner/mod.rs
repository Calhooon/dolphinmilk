//! Agent loop runner — OBSERVE → THINK → ACT → RECORD → BUDGET CHECK.
//!
//! This is the core of the worm. Each `step()` call is one iteration of the
//! agent loop. The loop continues until the task is complete, budget is
//! exhausted, or a circuit breaker trips.
//!
//! ## Module layout
//!
//! - **mod.rs** — Types, construction, small helpers
//! - **lifecycle.rs** — setup_task, run_loop, teardown_task, run
//! - **step.rs** — Per-iteration phases: observe, build, think, record, step orchestrator
//! - **execute.rs** — Tool execution: parallel/sequential dispatch, offloading, proofs
//! - **approval.rs** — Tool approval gate: file-based manual approval workflow
//! - **text_extract.rs** — Reasoning model fallback: extract tool calls from text
//! - **escalation.rs** — Auto-escalation detection
//! - **moderation.rs** — Moderation engine construction, content checks, transcript recording

mod approval;
pub mod escalation;
mod execute;
mod lifecycle;
pub(crate) mod moderation;
pub(crate) mod step;
pub(crate) mod text_extract;
pub mod working_memory;

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;

use crate::auth::AuthriteClient;
use crate::budget::BudgetTracker;
use crate::config::DmConfig;
use crate::context::manager::ContextManager;
use crate::context::prompt::PromptContext;
use crate::error::DmError;
use crate::events::StepEvent;
use crate::hooks::HookRegistry;
use crate::loop_detect::detector::LoopDetector;
use crate::memory::search::{self, MemoryIndex};
use crate::memory::store::MemoryStore;
use crate::memory::sync::MemorySync;
use crate::messagebox::client::MessageBoxClient;
use crate::moderation::ModerationEngine;
use crate::proofs;
use crate::sanitize;
use crate::skills::SkillRegistry;
use crate::state;
use crate::think;
#[cfg(feature = "browser")]
use crate::tools::browser_tools::all_browser_tools;
use crate::tools::conversation_tools::all_conversation_tools;
use crate::tools::delegation_tools::all_delegation_tools;
use crate::tools::discovery_tools::all_discovery_tools;
use crate::tools::memory_tools::all_memory_tools;
use crate::tools::messagebox_tools::all_messagebox_tools_shared;
use crate::tools::overlay_tools::all_overlay_tools;
use crate::tools::registry::ToolRegistry;
use crate::tools::sandbox::all_sandbox_tools;
use crate::tools::schedule_tools::all_schedule_tools;
use crate::tools::wallet_tools::all_wallet_tools;
use crate::tools::working_memory_tools::all_working_memory_tools;
// x402 tools registered via crate::tools::x402_tools::all_x402_tools_with_rate_limiter()
use crate::transcript::Transcript;
use crate::wallet::WalletBackend;
use crate::x402::circuit_breaker::CircuitBreakerRegistry;

// =============================================================================
// Constants
// =============================================================================

/// Maximum nudges before giving up on getting the LLM to call send_message.
pub(crate) const MAX_NUDGES: u32 = 2;

/// Maximum intent nudges per iteration before proceeding with whatever response we got.
/// Separate from MAX_NUDGES which is for reply obligation nudges.
pub const MAX_INTENT_NUDGES: u32 = 2;

/// Nudge message injected when the LLM expresses intent to use a tool
/// but doesn't include any tool_calls in its response.
pub const TOOL_INTENT_NUDGE: &str =
    "You described what you intend to do but didn't make a tool call. \
     Please proceed with the actual tool call instead of describing what you will do.";

/// Detect when an LLM response expresses intent to call a tool without actually
/// issuing tool calls. Returns `true` if the text contains phrases like
/// "Let me search…" or "I'll check…" outside of fenced code blocks.
///
/// Exclusion phrases (e.g. "let me explain") are checked first to avoid
/// false positives on conversational language.
pub fn signals_tool_intent(response: &str) -> bool {
    let text = strip_code_blocks(response);
    let lower = text.to_lowercase();

    // Exclusion phrases — bail if any appear (not tool intent)
    const EXCLUSIONS: &[&str] = &[
        "let me explain",
        "let me know",
        "let me think",
        "let me summarize",
        "let me clarify",
        "let me describe",
        "let me help",
        "let me understand",
        "let me break",
        "let me outline",
        "let me walk you",
        "let me provide",
        "let me suggest",
        "let me elaborate",
        "let me start by",
    ];
    if EXCLUSIONS.iter().any(|e| lower.contains(e)) {
        return false;
    }

    // Intent prefixes paired with action verbs
    const PREFIXES: &[&str] = &[
        "let me ",
        "i'll ",
        "i'll now ",
        "i will ",
        "i will now ",
        "i'm going to ",
    ];
    const ACTION_VERBS: &[&str] = &[
        "search",
        "look up",
        "check",
        "fetch",
        "find",
        "read the",
        "run",
        "execute",
        "query",
        "retrieve",
        "look into",
        "look for",
    ];

    for prefix in PREFIXES {
        for (i, _) in lower.match_indices(prefix) {
            let after = &lower[i + prefix.len()..];
            for verb in ACTION_VERBS {
                if after.starts_with(verb) {
                    return true;
                }
            }
        }
    }

    false
}

/// Strip fenced code blocks (``` ... ```) so intent detection only fires on prose.
fn strip_code_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut in_fence = false;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

// =============================================================================
// Types
// =============================================================================

/// A pending reply obligation — an external sender who needs a response.
/// Created when an inbox message arrives from a non-self sender.
/// Fulfilled when `send_message` is called with a matching recipient.
#[derive(Debug, Clone)]
pub struct ReplyObligation {
    pub sender_key: String,
    pub message_box: String,
}

/// Execution state — iteration counter, completion signals, result/error.
#[derive(Default)]
pub struct ExecutionState {
    pub iteration: u32,
    pub done: bool,
    pub result: String,
    pub error: String,
}

/// Budget state — spending, cached balance, advisory over-budget flag.
#[derive(Default)]
pub struct BudgetState {
    pub sats_spent: u64,
    pub cached_balance: u64,
    /// Number of spendable outputs in the default basket (updated each iteration).
    pub cached_spendable_count: u64,
    /// True when budget limit has been exceeded in advisory mode.
    /// Set by the budget pre-check when enforcement is "advisory".
    pub over_budget: bool,
}

/// Communication state — reply obligations and nudge counter.
#[derive(Default)]
pub struct CommunicationState {
    /// Reply obligations — external senders awaiting a response this iteration.
    /// Cleared each iteration; rebuilt from inbox messages. Nudge system fires
    /// only for unfulfilled obligations, not raw inbox count.
    pub pending_replies: Vec<ReplyObligation>,
    pub nudge_count: u32,
}

/// On-chain state — proof chain hash, checkpoint and commitment tokens.
#[derive(Default)]
pub struct OnChainState {
    /// Hash of the last proof in the chain (for linking proofs).
    pub last_proof_hash: Option<String>,
    /// Previous Checkpoint token for spend-and-recreate.
    pub last_checkpoint: Option<state::StateToken>,
    /// TaskCommitment token created at task start, relinquished at task end.
    pub task_commitment: Option<state::StateToken>,
    /// BudgetAllocation token created at task start, updated each iteration.
    pub budget_allocation: Option<state::StateToken>,
    /// CapabilityDeclaration token created at task start, relinquished at task end.
    pub capability_declaration: Option<state::StateToken>,
    /// BRC-60 conversation hash chain verification status.
    /// Set by `set_conversation_chain_status()` before `run()` in server mode.
    /// `None` = no conversation (CLI or new conversation), `Some(true)` = valid,
    /// `Some(false)` = chain break detected.
    pub conversation_chain_verified: Option<bool>,
    /// Details about a conversation chain break (conversation_id, first break seq).
    /// Set alongside `conversation_chain_verified = Some(false)`.
    pub conversation_chain_break: Option<ConversationChainBreak>,
    /// UTXO counts per basket, populated at task start for vital signs.
    pub basket_health: std::collections::HashMap<String, u64>,
}

/// Storage state — task description, ID, offloaded results.
#[derive(Default)]
pub struct StorageState {
    pub task: String,
    /// Task ID for budget tracking (set from `tx` in server mode, empty in CLI).
    pub task_id: String,
    /// Map of tool call_id → (file_path, smart_preview) for offloaded large tool results.
    /// Used by build_llm_messages() to substitute previews in the LLM's message history.
    pub offloaded_results: std::collections::HashMap<String, (String, String)>,
    /// Counter for unique offloaded file names within a task.
    pub offload_counter: usize,
    /// Pending inbound `task_delegation` envelope from a MessageBox commission.
    /// Set by `set_pending_delegation()` before `run()`; consumed once at
    /// `setup_task()` to parse + verify the cert chain and attach the
    /// resulting `DelegationContext` to `LoopState`. See EPIC #329 Phase 3.
    pub pending_delegation_envelope: Option<serde_json::Value>,
    /// Verified delegation context derived from `pending_delegation_envelope`.
    /// `Some` when the receiver successfully validated an inbound commission
    /// and its caveats have been applied to the tool allowlist + budget.
    pub delegation_context: Option<crate::delegation::DelegationContext>,
}

/// Auth state — external origin flag and certificate capabilities.
#[derive(Default)]
pub struct AuthState {
    /// True when this task originated from an external message (POST /message).
    /// When set, tools are restricted to a safe allowlist and the system prompt
    /// includes an injection defense warning.
    pub external_origin: bool,
    /// Parsed capabilities from BRC-52 certificate (Phase 10.2).
    /// `Some(caps)` enforces capability checks on tool execution.
    /// `None` skips checks (no certificate, CLI mode, backward compat).
    /// Contains `"all"` when certificate is absent (default full access).
    pub capabilities: Option<Vec<String>>,
    /// SHA-256 hash of the BRC-52 certificate used for authorization.
    /// Computed at setup_task() and cached for the entire task.
    pub cert_hash: Option<String>,
    /// Certificate type: "parent-signed", "self-signed", or "none".
    pub cert_type: Option<String>,
}

/// Details about a conversation hash chain break, used for BRC-18 proof creation.
#[derive(Debug, Clone)]
pub struct ConversationChainBreak {
    /// Conversation ID where the break was detected.
    pub conversation_id: String,
    /// Sequence number of the first broken message.
    pub first_break_seq: u32,
    /// Expected hash at the first break point.
    pub expected_hash: String,
    /// Actual hash at the first break point.
    pub actual_hash: String,
    /// Total number of breaks in the chain.
    pub total_breaks: usize,
}

/// Mutable state for a running agent loop.
///
/// Decomposed into focused sub-structs for clarity (#209).
#[derive(Default)]
pub struct LoopState {
    pub exec: ExecutionState,
    pub budget: BudgetState,
    pub comms: CommunicationState,
    pub onchain: OnChainState,
    pub storage: StorageState,
    pub auth: AuthState,
}

/// SHA-256 hex digest of full content (for proof data instead of truncation).
pub fn content_hash(content: &str) -> String {
    let hash = sha2::Sha256::digest(content.as_bytes());
    hex::encode(hash)
}

/// State saved when the agent pauses a task for later resumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationState {
    pub id: String,
    pub task: String,
    pub transcript_path: PathBuf,
    pub iteration: u32,
    pub reason: String,
    pub wake_at: Option<String>, // RFC 3339 timestamp
    pub created_at: String,      // RFC 3339 timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_proof_hash: Option<String>,
}

/// The agent loop.
pub struct DmLoop {
    pub config: DmConfig,
    pub workspace: PathBuf,
    /// Top-level shared workspace (parent of `workspace`). Used for
    /// resources that span tasks: schedules, conversations, commission
    /// payment surfacing. Defaults to `workspace` in CLI mode where
    /// there's only one task at a time.
    pub global_workspace: PathBuf,
    /// The conversation_id this task is bound to, if any. Set by
    /// `spawn_task()` before `run()`. Used by lifecycle events that need
    /// to surface activity in the user-facing conversation view (e.g.
    /// commission payment claims). `None` in CLI mode.
    pub current_conversation_id: Option<String>,
    pub transcript: Transcript,
    pub(crate) context: ContextManager,
    pub tools: Arc<tokio::sync::RwLock<ToolRegistry>>,
    pub detector: LoopDetector,
    pub state: LoopState,
    /// Prior conversation messages prepended to history on each LLM call.
    /// Set by `set_prior_messages()` when resuming a multi-turn conversation.
    pub prior_messages: Option<Vec<Value>>,
    pub(crate) wallet: Arc<dyn WalletBackend>,
    pub(crate) auth: AuthriteClient,
    pub(crate) messagebox: std::sync::Arc<MessageBoxClient>,
    pub(crate) memory_store: MemoryStore,
    pub(crate) memory_index: Option<MemoryIndex>,
    pub(crate) memory_sync: MemorySync,
    pub(crate) budget_tracker: BudgetTracker,
    pub(crate) skills: SkillRegistry,
    /// Cached BRC-52 certificate info (fetched once at loop start).
    pub(crate) certificate_info: Option<crate::context::prompt::CertificateInfo>,
    /// Content moderation engine — constructed from cert + config at task start.
    pub(crate) moderation: ModerationEngine,
    /// x402 per-service rate limiter. `None` in CLI mode (no rate limiting).
    /// Set via `create_loop_with_rate_limiter()` from `AppState.rate_limiter`.
    pub rate_limiter: Option<Arc<crate::x402::rate_limit::RateLimiterRegistry>>,
    /// Merged list of tool names that require manual approval before execution.
    /// Built from config `[tool_approval].require` + cert `approval_tools` field.
    pub approval_tools: Vec<String>,
    /// Seconds to wait for approval before auto-aborting. From config `[tool_approval].timeout_secs`.
    pub approval_timeout_secs: u64,
    /// Shared staged-transactions map from AppState — for inserting tool approval entries
    /// visible to the `/staged` API. `None` in CLI mode (file-only approval).
    pub approval_tx: Option<
        Arc<
            tokio::sync::Mutex<std::collections::HashMap<String, crate::server::StagedTransaction>>,
        >,
    >,
    /// Prometheus metrics registry — shared across all tasks. `None` in CLI mode.
    pub metrics: Option<Arc<crate::metrics::MetricsRegistry>>,
    /// x402 circuit breaker registry — shared across all tasks for provider failover.
    /// When the primary LLM provider fails repeatedly, requests are routed to the alternate.
    pub circuit_breakers: Arc<CircuitBreakerRegistry>,
    /// User-provided image attachments as OpenAI content blocks.
    /// Injected into the first user message during `build_llm_messages()`.
    /// Set by `spawn_task()` when the chat request includes attachments.
    pub user_attachment_blocks: Option<Vec<Value>>,
    /// Lifecycle hook registry — fires events at key agent lifecycle points.
    /// When empty, `fire()` is a zero-cost no-op (single empty-vec check).
    pub hook_registry: HookRegistry,
    /// Working memory — scratch pad that persists across iterations within a task.
    /// Shared with tool closures via `Arc<RwLock<_>>`.
    pub working_memory: Arc<tokio::sync::RwLock<working_memory::WorkingMemory>>,
    /// Shared iteration counter for working memory TTL tracking.
    /// Updated at the start of each step; read by working memory tool closures.
    pub(crate) wm_iteration: Arc<std::sync::atomic::AtomicU32>,
}

// =============================================================================
// Helpers
// =============================================================================

/// Emit a step event to the SSE broadcast channel (server mode).
/// No-op when the StepContext has no event sender (CLI mode).
pub(crate) fn emit_event(ctx: &step::StepContext, event: StepEvent) {
    if let Some(ref sender) = ctx.events {
        let _ = sender.send((ctx.task_id.clone(), event));
    }
}

// =============================================================================
// Construction + small methods
// =============================================================================

impl DmLoop {
    /// Create a new agent loop.
    ///
    /// `workspace` is the per-task directory (transcripts, budget, context).
    /// `memory_dir` is the global memory directory shared across all tasks.
    /// If `None`, defaults to `workspace/memory` (appropriate for CLI mode).
    /// `global_workspace` is the top-level workspace for shared resources (schedules,
    /// conversations). If `None`, defaults to `workspace` (appropriate for CLI mode).
    pub fn new(
        config: DmConfig,
        workspace: PathBuf,
        memory_dir: Option<PathBuf>,
        global_workspace: Option<PathBuf>,
        wallet: Arc<dyn WalletBackend + Send + Sync>,
    ) -> Self {
        Self::new_with_rate_limiter(
            config,
            workspace,
            memory_dir,
            global_workspace,
            wallet,
            None,
            Arc::new(CircuitBreakerRegistry::new()),
            None,
        )
    }

    /// Create a new agent loop with an optional x402 rate limiter and shared circuit breakers.
    ///
    /// The rate limiter is passed through to x402 tools and stored on the loop
    /// for use during LLM inference rate limiting in `think_step()`.
    /// The circuit breaker registry is shared across all tasks for provider failover.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_rate_limiter(
        config: DmConfig,
        workspace: PathBuf,
        memory_dir: Option<PathBuf>,
        global_workspace: Option<PathBuf>,
        wallet: Arc<dyn WalletBackend + Send + Sync>,
        rate_limiter: Option<Arc<crate::x402::rate_limit::RateLimiterRegistry>>,
        circuit_breakers: Arc<CircuitBreakerRegistry>,
        shared_messagebox: Option<std::sync::Arc<MessageBoxClient>>,
    ) -> Self {
        let _ = std::fs::create_dir_all(&workspace);

        let transcript = Transcript::new(workspace.join("session.jsonl"));
        // Use model-derived limits, with config values as caps
        // (users can lower limits for cost control via config)
        let model = &config.llm.default_model;
        let model_input = think::model_input_limit(model);
        let model_output = think::model_output_limit(model);
        let effective_context = config.llm.context_window.min(model_input);
        let effective_output = (config.llm.max_tokens as usize).min(model_output);
        let mut context = ContextManager::with_output_tokens(
            effective_context,
            workspace.join("context"),
            config.llm.min_recent_messages,
            config.llm.max_history_turns,
            effective_output,
        );
        context.set_compaction_threshold(config.llm.compaction_threshold);
        let mut tools = ToolRegistry::new();
        let detector = LoopDetector::with_defaults(config.budget.max_per_hour);
        let wallet_url = config.wallet.url.clone();

        // Initialize memory system — use global memory_dir if provided
        let memory_dir = memory_dir.unwrap_or_else(|| workspace.join("memory"));
        let memory_store = MemoryStore::new(memory_dir.clone());
        let memory_index = match MemoryIndex::new(memory_dir.join("index")) {
            Ok(idx) => Some(idx),
            Err(e) => {
                tracing::warn!("Failed to open memory index: {e}");
                None
            }
        };

        // Initialize encrypted memory sync
        let memory_sync = MemorySync::new(&memory_dir);

        // Initialize budget tracker
        let budget_tracker = BudgetTracker::from_config(&config.budget, &workspace);

        // Initialize BRC-31 auth client for LLM think() calls (stored in DmLoop).
        // MessageBox client is shared across the whole agent to prevent session churn.
        let auth = AuthriteClient::new(wallet.clone(), &config.wallet.url);
        let messagebox = shared_messagebox.unwrap_or_else(|| {
            // CLI mode: no shared client, create a standalone one with configured URL
            let mb_auth = AuthriteClient::new(wallet.clone(), &config.wallet.url);
            std::sync::Arc::new(MessageBoxClient::with_url(mb_auth, &config.messagebox.url))
        });

        // Load skills from project-root skills/ directory, then workspace/skills as overlay,
        // then project-level .dolphin-milk/skills/ as additive
        let mut skills = SkillRegistry::load_from_dir(std::path::Path::new("skills"));
        let workspace_skills = SkillRegistry::load_from_dir(&workspace.join("skills"));
        for skill in workspace_skills.all() {
            skills.register(skill.clone());
        }
        // Load project-level skills from .dolphin-milk/skills/ (additive)
        let project_skills_dir = workspace.join(".dolphin-milk").join("skills");
        skills.load_additional_from_dir(&project_skills_dir);
        if !skills.is_empty() {
            tracing::info!("Loaded {} skill(s)", skills.len());
        }

        // Register default tools
        for tool in all_sandbox_tools(workspace.clone(), config.llm.context_window) {
            tools.register(tool);
        }
        for tool in all_wallet_tools(config.wallet.url.clone()) {
            tools.register(tool);
        }
        for tool in all_memory_tools(memory_dir.clone()) {
            tools.register(tool);
        }
        for tool in all_messagebox_tools_shared(messagebox.clone(), wallet_url.clone()) {
            tools.register(tool);
        }

        // Register delegation tools (1: delegate_task — always-on, see EPIC #329).
        // Shares the MessageBox client for commission dispatch and the wallet for
        // BRC-77 signing + revocation UTXO creation. The overlay submit URL
        // is passed through so the tool can publish the revocation tx to
        // tm_dm_delegation; an empty string disables the publish step.
        // Trust root for delegation cert root_certifier resolution: prefer
        // the operator-declared config.trust.certifiers[0] (EPIC #329 §8 —
        // the trust root is a config surface, not derived from BRC-52
        // alone). delegate_task::resolve_root_certifier() falls back to the
        // agent's parent-signed cert and finally to self if this is None.
        let trust_root_for_delegation = config.trust.certifiers.first().cloned();
        for tool in all_delegation_tools(
            wallet.clone(),
            messagebox.clone(),
            config.overlay.submit_url.clone(),
            trust_root_for_delegation,
        ) {
            tools.register(tool);
        }

        // Register x402 tools (3 generic: discover_services, discover_endpoints, x402_call)
        for tool in crate::tools::x402_tools::all_x402_tools_with_rate_limiter(
            wallet_url.clone(),
            config.x402.registry_url.clone(),
            rate_limiter.clone(),
        ) {
            tools.register(tool);
        }

        // Register x402 recipe tools (2 atomic: generate_image, upload_to_nanostore)
        for tool in crate::tools::x402_tools::all_x402_recipe_tools_with_rate_limiter(
            wallet_url.clone(),
            rate_limiter.clone(),
        ) {
            tools.register(tool);
        }

        // Register schedule tools (3: create_schedule, list_schedules, cancel_schedule)
        // Use global workspace so schedules are shared across tasks and visible to
        // the heartbeat scanner and /schedules endpoint.
        let global_ws = global_workspace.unwrap_or_else(|| workspace.clone());
        for tool in all_schedule_tools(global_ws.clone()) {
            tools.register(tool);
        }

        // Register conversation tools (2: list_conversations, read_conversation)
        for tool in all_conversation_tools(global_ws.clone()) {
            tools.register(tool);
        }

        // Register browser tool (1: browser — discoverable, not always-on)
        #[cfg(feature = "browser")]
        for tool in all_browser_tools(workspace.clone(), config.browser.clone()) {
            tools.register(tool);
        }

        // Register analytics tools (1: cost_analysis — discoverable, not always-on)
        for tool in crate::tools::analytics_tools::all_analytics_tools(global_ws.clone()) {
            tools.register(tool);
        }

        // Register discovery tools (2: discover_agent, verify_agent — discoverable, not always-on)
        for tool in all_discovery_tools(wallet_url.clone()) {
            tools.register(tool);
        }

        // Register overlay tools (1: overlay_lookup — discoverable, not always-on)
        for tool in all_overlay_tools(config.overlay.submit_url.clone()) {
            tools.register(tool);
        }

        // Register introspect tool (1: introspect — discoverable, not always-on)
        tools.register(crate::tools::introspect_tools::create_introspect_tool(
            Arc::new(global_ws.clone()),
        ));

        // Register orchestration tools (4: spawn_agent, check_agent, list_agents, kill_agent — discoverable)
        {
            let task_registry = crate::orchestration::shared_task_registry();
            let budget_pool = Arc::new(tokio::sync::Mutex::new(
                crate::orchestration::budget::BudgetPool::new(config.budget.max_per_task),
            ));
            let spawner: Arc<tokio::sync::Mutex<dyn crate::orchestration::AgentSpawner>> = Arc::new(
                tokio::sync::Mutex::new(crate::orchestration::LocalAgentSpawner::new(
                    config.clone(),
                    task_registry,
                    budget_pool,
                    global_ws.clone(),
                    memory_dir.clone(),
                )),
            );
            for tool in crate::tools::orchestration_tools::all_orchestration_tools(spawner) {
                tools.register(tool);
            }
        }

        // MCP wallet tools are registered later via register_mcp_tools() after async connect.

        // Register list_commands tool — returns available quick commands from commands/ dirs
        {
            let ws_for_cmds = global_ws.clone();
            tools.register(crate::tools::registry::ToolDef {
                name: "list_commands".to_string(),
                description: "List available quick commands. Commands are markdown files in commands/ directories that can be invoked via /command-name in chat.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
                execute: Box::new(move |_params| {
                    let ws = ws_for_cmds.clone();
                    Box::pin(async move {
                        let commands = crate::skills::commands::load_commands(&ws);
                        if commands.is_empty() {
                            return "No commands found. Add .md files to commands/ directory or ~/.dolphin-milk/commands/ to create commands.".to_string();
                        }
                        let list: Vec<serde_json::Value> = commands
                            .iter()
                            .map(|c| {
                                serde_json::json!({
                                    "name": c.name,
                                    "description": c.description,
                                    "source": format!("{:?}", c.source),
                                })
                            })
                            .collect();
                        serde_json::to_string_pretty(&list).unwrap_or_default()
                    })
                }),
                category: "system".to_string(),
                cleanup: None,
                deferred: false,
                always_load: true,
                search_hint: None,
            });
        }

        // Phase 2.3: Register search_tools — weighted scoring + select: syntax
        {
            let snapshot = tools.all_tool_snapshots();
            tools.register(crate::tools::registry::ToolDef {
                name: "search_tools".to_string(),
                description: "Search for available tools by keyword or load specific tools by name. Use 'select:tool1,tool2' for direct loading, or keywords for weighted search. Scoring: name match (10pts) > substring (5pts) > hint (4pts) > description (2pts). Use +keyword to require a term.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Keywords to search (e.g. 'browser automation'), or 'select:tool_name' for direct loading. Prefix with + to require a term (e.g. '+browser screenshot')."
                        }
                    },
                    "required": ["query"]
                }),
                execute: Box::new(move |params| {
                    let snaps = snapshot.clone();
                    Box::pin(async move {
                        let query = params.get("query")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        if query.trim().is_empty() {
                            // Empty query: list all tools with hints
                            let all: Vec<serde_json::Value> = snaps.iter()
                                .map(|s| serde_json::json!({
                                    "name": s.name,
                                    "hint": s.hint,
                                    "category": s.category,
                                    "deferred": s.deferred,
                                }))
                                .collect();
                            return serde_json::to_string_pretty(&all).unwrap_or_default();
                        }

                        // select: syntax for direct tool lookup
                        if let Some(names) = query.strip_prefix("select:") {
                            let results = crate::tools::registry::select_tools(&snaps, names);
                            if results.is_empty() {
                                return format!("No tools found matching select: query. Available tools: {}",
                                    snaps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "));
                            }
                            let json: Vec<serde_json::Value> = results.iter()
                                .map(|r| serde_json::json!({
                                    "name": r.name,
                                    "description": r.description,
                                    "category": r.category,
                                    "match_score": r.score,
                                    "match_reasons": r.match_reasons,
                                }))
                                .collect();
                            return serde_json::to_string_pretty(&json).unwrap_or_default();
                        }

                        // Weighted search
                        let results = crate::tools::registry::weighted_search(&snaps, query, 10);

                        if results.is_empty() {
                            return "No tools found matching your query. Try different keywords or use select:tool_name for direct lookup.".to_string();
                        }

                        let json: Vec<serde_json::Value> = results.iter()
                            .map(|r| serde_json::json!({
                                "name": r.name,
                                "description": r.description,
                                "category": r.category,
                                "match_score": r.score,
                                "match_reasons": r.match_reasons,
                            }))
                            .collect();

                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    })
                }),
                category: "system".to_string(),
                cleanup: None,
                deferred: false,
                always_load: true,
                search_hint: None,
            });
        }

        // Working memory — scratch pad that persists across iterations within a task.
        // Shared between tool closures and the runner for prompt injection.
        let working_memory = Arc::new(tokio::sync::RwLock::new(
            working_memory::WorkingMemory::new(),
        ));
        let wm_iteration = Arc::new(std::sync::atomic::AtomicU32::new(0));
        {
            let iter_ref = Arc::clone(&wm_iteration);
            let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> =
                Arc::new(move || iter_ref.load(std::sync::atomic::Ordering::Relaxed));
            for tool in all_working_memory_tools(Arc::clone(&working_memory), iter_fn) {
                tools.register(tool);
            }
        }

        // Wrap registry in Arc<RwLock> for shared access
        let tools = Arc::new(tokio::sync::RwLock::new(tools));

        let approval_tools = config.tool_approval.require.clone();
        let approval_timeout_secs = config.tool_approval.timeout_secs;

        // Initialize lifecycle hook registry from config
        let hook_registry = HookRegistry::from_config(&config.hooks);
        if !hook_registry.is_empty() {
            tracing::info!(
                "Hook registry initialized with {} hook(s)",
                hook_registry.len()
            );
        }

        Self {
            config,
            workspace,
            global_workspace: global_ws.clone(),
            current_conversation_id: None,
            transcript,
            context,
            tools,
            detector,
            state: LoopState::default(),
            prior_messages: None,
            wallet,
            auth,
            messagebox,
            memory_store,
            memory_index,
            memory_sync,
            budget_tracker,
            skills,
            certificate_info: None,
            moderation: ModerationEngine::disabled(),
            rate_limiter,
            approval_tools,
            approval_timeout_secs,
            approval_tx: None,
            metrics: None,
            circuit_breakers,
            user_attachment_blocks: None,
            hook_registry,
            working_memory,
            wm_iteration,
        }
    }

    /// Register MCP-proxied tools and rebuild the search_tools snapshot.
    ///
    /// Called after construction when MCP wallet tools are available.
    /// Must be called before `run()` so the search snapshot includes them.
    pub async fn register_mcp_tools(&self, tool_defs: Vec<crate::tools::registry::ToolDef>) {
        if tool_defs.is_empty() {
            return;
        }
        let count = tool_defs.len();
        let mut registry = self.tools.write().await;
        for def in tool_defs {
            registry.register(def);
        }
        tracing::info!("Registered {count} MCP wallet tools in runner");

        // Rebuild the search_tools snapshot to include MCP tools
        let snapshot = registry.all_tool_snapshots();
        registry.remove("search_tools");
        registry.register(crate::tools::registry::ToolDef {
            name: "search_tools".to_string(),
            description: "Search for available tools by keyword or load specific tools by name. Use 'select:tool1,tool2' for direct loading, or keywords for weighted search.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keywords to search (e.g. 'browser automation'), or 'select:tool_name' for direct loading."
                    }
                },
                "required": ["query"]
            }),
            execute: Box::new(move |params| {
                let snaps = snapshot.clone();
                Box::pin(async move {
                    let query = params.get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if query.trim().is_empty() {
                        let all: Vec<serde_json::Value> = snaps.iter()
                            .map(|s| serde_json::json!({"name": s.name, "hint": s.hint, "category": s.category, "deferred": s.deferred}))
                            .collect();
                        return serde_json::to_string_pretty(&all).unwrap_or_default();
                    }

                    if let Some(names) = query.strip_prefix("select:") {
                        let results = crate::tools::registry::select_tools(&snaps, names);
                        if results.is_empty() {
                            return "No tools found matching select: query.".to_string();
                        }
                        let json: Vec<serde_json::Value> = results.iter()
                            .map(|r| serde_json::json!({"name": r.name, "description": r.description, "category": r.category, "match_score": r.score}))
                            .collect();
                        return serde_json::to_string_pretty(&json).unwrap_or_default();
                    }

                    let results = crate::tools::registry::weighted_search(&snaps, query, 10);
                    if results.is_empty() {
                        return "No tools found matching your query. Try different keywords or use select:tool_name.".to_string();
                    }
                    let json: Vec<serde_json::Value> = results.iter()
                        .map(|r| serde_json::json!({"name": r.name, "description": r.description, "category": r.category, "match_score": r.score, "match_reasons": r.match_reasons}))
                        .collect();
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                })
            }),
            category: "system".to_string(),
            cleanup: None,
            deferred: false,
            always_load: true,
            search_hint: None,
        });
    }

    /// Apply discovered model capabilities to update context window limits.
    ///
    /// Resolution chain: discovered caps → hardcoded fallback → config cap.
    /// Config values still act as caps (user can lower for cost control).
    /// Called by `spawn_task()` when model capabilities have been pre-fetched.
    pub fn apply_model_capabilities(
        &mut self,
        capabilities: &std::collections::HashMap<String, think::ModelCapabilities>,
    ) {
        let model = &self.config.llm.default_model;
        if let Some(caps) = capabilities.get(model) {
            let effective_context = self.config.llm.context_window.min(caps.max_input_tokens);
            let effective_output =
                (self.config.llm.max_tokens as usize).min(caps.max_output_tokens);
            self.context.max_tokens = effective_context;
            self.context.set_output_tokens(effective_output);
            tracing::info!(
                "Applied discovered model capabilities for {}: context={}, output={}",
                model,
                effective_context,
                effective_output
            );
        }
        // If model not found in discovered capabilities, keep the hardcoded values
        // already set during construction.
    }

    /// Set prior conversation messages to prepend to LLM history.
    /// Called when resuming a multi-turn conversation.
    pub fn set_prior_messages(&mut self, messages: Vec<Value>) {
        self.prior_messages = Some(messages);
    }

    /// Set user-provided image attachments as OpenAI-format content blocks.
    /// These will be injected into the first user message during `build_llm_messages()`.
    pub fn set_user_attachments(&mut self, blocks: Vec<Value>) {
        self.user_attachment_blocks = Some(blocks);
    }

    /// Set conversation hash chain verification status.
    ///
    /// Called by `spawn_task()` after running `verify_chain()` on the conversation.
    /// If `verified` is `false`, the runner will log a warning and create a BRC-18
    /// `ConversationBreak` proof in `setup_task()`.
    pub fn set_conversation_chain_status(
        &mut self,
        verified: bool,
        chain_break: Option<ConversationChainBreak>,
    ) {
        self.state.onchain.conversation_chain_verified = Some(verified);
        self.state.onchain.conversation_chain_break = chain_break;
    }

    /// Set the conversation_id this task is bound to. Called by
    /// `spawn_task()` so lifecycle events (e.g. commission payment claims)
    /// can surface activity in the user-facing conversation view.
    pub fn set_current_conversation_id(&mut self, id: String) {
        self.current_conversation_id = Some(id);
    }

    /// Append a system-role message to the current conversation, if one
    /// is bound. Best-effort: errors are logged at warn level but never
    /// bubble up. Used by `emit_commission_payment_claim()` and friends
    /// to surface payment activity in the UI without going through the
    /// LLM dialogue.
    pub(crate) fn append_commission_conversation_event(&self, conv_id: &str, content: &str) {
        let mgr = crate::session::conversation::ConversationManager::new(&self.global_workspace);
        // Auto-create if missing — commission conversations are created
        // by spawn_task at the time the delegation envelope arrives, but
        // we tolerate the not-yet-created case so the runner doesn't fail.
        if mgr.load(conv_id).ok().flatten().is_none() {
            let _ = mgr.create_with_id(conv_id, "commission:system", "Commission activity");
        }
        let task_id = self.state.storage.task_id.clone();
        if let Err(e) = mgr.append_system_message(
            conv_id,
            content,
            if task_id.is_empty() {
                None
            } else {
                Some(&task_id)
            },
        ) {
            tracing::warn!("Failed to append system message to conv {conv_id}: {e}");
        }
    }

    /// Stash an inbound `task_delegation` envelope on the loop state so
    /// `setup_task()` can parse, verify, and apply its caveats before the
    /// agent loop runs.
    ///
    /// Phase 3 of EPIC #329. Called by `spawn_task()` when an inbound
    /// MessageBox body has `type == "task_delegation"`. The envelope carries
    /// a signed BRC-52 `agent-delegation` cert plus an optional parent chain.
    pub fn set_pending_delegation(&mut self, envelope: serde_json::Value) {
        self.state.storage.pending_delegation_envelope = Some(envelope);
    }

    /// Mark this task as originating from an external message (POST /message).
    ///
    /// When set, tools are restricted to a safe allowlist (Layer 3 injection
    /// defense) and the system prompt includes an external message warning (Layer 4).
    /// If `sender` matches the parent identity key, the restriction is bypassed.
    pub fn set_external_origin(&mut self, sender: &str) {
        let is_parent = sanitize::is_parent_sender(sender, &self.config.parent.identity_key);
        if is_parent {
            tracing::info!("External message from parent — full tool access retained");
        } else {
            tracing::info!("External message from untrusted sender — applying tool allowlist");
            self.state.auth.external_origin = true;
        }
    }

    /// Set the shared staged-transactions map for tool approval visibility.
    /// Called by `spawn_task()` when the server provides `AppState.staged_transactions`.
    pub fn set_approval_tx(
        &mut self,
        tx: Arc<
            tokio::sync::Mutex<std::collections::HashMap<String, crate::server::StagedTransaction>>,
        >,
    ) {
        self.approval_tx = Some(tx);
    }

    /// Merge cert-driven approval tools into the config-based approval list.
    /// Called during `setup_task()` after reading certificate fields.
    pub fn merge_cert_approval_tools(&mut self, cert_tools: Vec<String>) {
        for tool in cert_tools {
            if !self.approval_tools.contains(&tool) {
                self.approval_tools.push(tool);
            }
        }
        if !self.approval_tools.is_empty() {
            tracing::info!("Tool approval required for: {:?}", self.approval_tools);
        }
    }

    /// Check if a tool requires manual approval before execution.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        self.approval_tools.iter().any(|t| t == tool_name)
    }

    pub(crate) async fn get_identity_key(&self) -> String {
        self.wallet.get_identity_key().await.unwrap_or_default()
    }

    /// Fetch BRC-52 certificate info from the wallet (best-effort, cached per loop).
    pub(crate) async fn fetch_certificate_info(
        &self,
    ) -> Option<crate::context::prompt::CertificateInfo> {
        use crate::certificates::{CertificateManager, CERT_TYPE_AGENT_AUTH};

        let mgr = CertificateManager::new(self.wallet.clone());
        let certs = mgr.list_all().await.ok()?;

        // Find the first agent-authorization certificate
        for entry in &certs {
            // Wallet returns each as {"certificate": {...}} or flat
            let cert = entry.get("certificate").unwrap_or(entry);
            let cert_type = cert
                .get("certificateType")
                .or_else(|| cert.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cert_type != CERT_TYPE_AGENT_AUTH {
                continue;
            }
            let subject = cert.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let certifier = cert.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
            let name = cert
                .get("fields")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let capabilities = cert
                .get("fields")
                .and_then(|f| f.get("capabilities"))
                .and_then(|v| v.as_str())
                .unwrap_or("none");
            let budget_per_task = cert
                .get("fields")
                .and_then(|f| f.get("budget_per_task"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let budget_per_hour = cert
                .get("fields")
                .and_then(|f| f.get("budget_per_hour"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let budget_per_day = cert
                .get("fields")
                .and_then(|f| f.get("budget_per_day"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let budget_per_week = cert
                .get("fields")
                .and_then(|f| f.get("budget_per_week"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let budget_per_month = cert
                .get("fields")
                .and_then(|f| f.get("budget_per_month"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let budget_lifetime = cert
                .get("fields")
                .and_then(|f| f.get("budget_lifetime"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let budget_enforcement = cert
                .get("fields")
                .and_then(|f| f.get("budget_enforcement"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            return Some(crate::context::prompt::CertificateInfo {
                certifier: certifier.to_string(),
                self_signed: subject == certifier,
                capabilities: capabilities.to_string(),
                name: name.to_string(),
                budget_per_task,
                budget_per_hour,
                budget_per_day,
                budget_per_week,
                budget_per_month,
                budget_lifetime,
                budget_enforcement,
            });
        }
        None
    }

    /// Get the configured max_tokens for the current model.
    pub(crate) fn max_tokens(&self) -> u32 {
        let model = &self.config.llm.default_model;
        if think::is_reasoning_model(model) {
            self.config.llm.max_tokens_reasoning
        } else {
            self.config.llm.max_tokens
        }
    }

    /// Create a BRC-18 proof, record in transcript + budget, update proof chain.
    /// Returns the proof txid on success, None on failure (logged as warning).
    pub(crate) async fn record_proof(
        &mut self,
        commitment: proofs::ProofCommitment,
        proof_type: &str,
        extra_budget_meta: Option<serde_json::Value>,
    ) -> Option<String> {
        match proofs::create_proof(&self.wallet, commitment).await {
            Ok(proof_result) => {
                let hash_hex = proof_result.commitment.hash_hex();
                tracing::info!("BRC-18 {proof_type} proof: txid={}", proof_result.txid);
                self.transcript.record_proof_created(
                    &proof_result.txid,
                    proof_type,
                    &hash_hex,
                    Some(&proof_result.commitment.data),
                    Some(&proof_result.commitment.timestamp),
                    Some(200),
                    Some(self.state.exec.iteration),
                    Some("dm-proofs"),
                    proof_result.commitment.prev_hash.as_deref(),
                );
                let mut meta = serde_json::json!({
                    "txid": proof_result.txid,
                    "task_id": self.state.storage.task_id,
                });
                if let Some(extra) = extra_budget_meta {
                    if let (Some(base), Some(ext)) = (meta.as_object_mut(), extra.as_object()) {
                        for (k, v) in ext {
                            base.insert(k.clone(), v.clone());
                        }
                    }
                }
                self.budget_tracker
                    .record("proofs", &format!("brc18_{proof_type}"), 200, meta);
                self.state.onchain.last_proof_hash = Some(hash_hex);
                self.state.budget.sats_spent += 200;

                // Fire OnProofCreated hook (notification-only)
                let _ = self
                    .hook_registry
                    .fire(crate::hooks::HookEvent::OnProofCreated {
                        proof_type: proof_type.to_string(),
                        txid: proof_result.txid.clone(),
                        sats_cost: 200,
                    })
                    .await;

                Some(proof_result.txid)
            }
            Err(e) => {
                tracing::warn!("Failed to create {proof_type} proof: {e}");
                None
            }
        }
    }

    /// Create or update a BRC-48 state token, record in transcript + budget.
    /// Returns the new token on success, None on failure (logged as warning).
    pub(crate) async fn record_token(
        &mut self,
        token_type: &str,
        basket: &str,
        data: serde_json::Value,
        old_token: Option<&state::StateToken>,
    ) -> Option<state::StateToken> {
        let result = if let Some(old) = old_token {
            state::update_token(&self.wallet, old, data.clone(), true, "self").await
        } else {
            let stt = match token_type {
                "task_commitment" => state::TokenType::TaskCommitment,
                "budget_allocation" => state::TokenType::BudgetAllocation,
                "capability_declaration" => state::TokenType::CapabilityDeclaration,
                _ => state::TokenType::Checkpoint,
            };
            let token = state::StateToken::new(stt, data.clone());
            state::create_token(&self.wallet, token, true, "self").await
        };
        match result {
            Ok(token_result) => {
                tracing::info!("BRC-48 {token_type} token: txid={}", token_result.txid);
                self.transcript.record_checkpoint_created(
                    &token_result.txid,
                    token_type,
                    basket,
                    Some(&data),
                );
                self.budget_tracker.record(
                    "state",
                    &format!("brc48_{token_type}"),
                    200,
                    serde_json::json!({"txid": token_result.txid, "task_id": self.state.storage.task_id}),
                );
                self.state.budget.sats_spent += 200;
                Some(token_result.token)
            }
            Err(e) => {
                tracing::warn!("Failed to create/update {token_type} token (non-fatal): {e}");
                None
            }
        }
    }

    /// Execute a tool by name. Gets the future under a read lock, then drops
    /// the lock before awaiting — so synthesized tools can write back to the
    /// registry (e.g. discover_endpoints) without deadlocking.
    ///
    /// Phase 10.2: Checks BRC-52 certificate capabilities against tool category.
    pub(crate) async fn execute_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<String, DmError> {
        let future = {
            let registry = self.tools.read().await;
            let tool = registry
                .get(name)
                .ok_or_else(|| DmError::tool(format!("Unknown tool: {name}")))?;
            if !registry.is_allowed(name) {
                return Err(DmError::tool(format!("Tool not allowed: {name}")));
            }
            // Phase 10.2: Capability enforcement from BRC-52 certificate
            if let Some(ref caps) = self.state.auth.capabilities {
                let required = crate::tools::registry::required_capability(&tool.category);
                if !caps.iter().any(|c| c == required || c == "all") {
                    return Err(DmError::tool(format!(
                        "Insufficient capability for tool '{}' (category '{}'): requires '{}' capability",
                        name, tool.category, required
                    )));
                }
            }
            (tool.execute)(arguments)
        };
        // Read lock dropped; the tool can now acquire write locks if needed
        Ok(future.await)
    }

    pub(crate) async fn build_prompt_context(
        &self,
        identity_key: &str,
        inbox_count: usize,
        has_external_messages: bool,
    ) -> PromptContext {
        let memory_summary = self.build_memory_summary();
        let skills_section = self.skills.format_for_prompt(inbox_count);

        // Load identity/soul from memory (Knowledge category, tag "identity")
        let identity_soul = self
            .memory_store
            .find_identity_entry()
            .map(|e| e.content.clone());

        // Phase 9.2: Automatic BM25 memory recall
        let (recalled_section, auto_recall_ids) = if let Some(ref index) = self.memory_index {
            // Extract query hints from task description + last assistant response
            let last_assistant = self.transcript.last_think_response_text();
            let query =
                search::extract_query_hints(&self.state.storage.task, last_assistant.as_deref());

            // Get IDs already shown in static summary to de-duplicate
            let static_ids = self.get_static_memory_ids();

            let recalled = search::auto_recall(index, &query, 5, &static_ids);
            let ids: Vec<String> = recalled.iter().map(|s| s.id.clone()).collect();
            let section = search::format_recalled_memories(&recalled);
            (section, ids)
        } else {
            (String::new(), Vec::new())
        };

        // Combine static memory summary with auto-recalled section
        let full_memory = if recalled_section.is_empty() {
            memory_summary
        } else if memory_summary.is_empty() {
            recalled_section
        } else {
            format!("{memory_summary}\n\n{recalled_section}")
        };

        PromptContext {
            identity_key: identity_key.to_string(),
            balance_sats: self.state.budget.cached_balance,
            model: self.config.llm.default_model.clone(),
            tools: self.tools.read().await.list_prompt_tools(),
            memory_summary: full_memory,
            available_files: self.context.offloaded_files().to_vec(),
            task: self.state.storage.task.clone(),
            budget_remaining: self
                .config
                .budget
                .max_per_task
                .saturating_sub(self.state.budget.sats_spent),
            low_power: self.state.budget.sats_spent > (self.config.budget.max_per_task * 80 / 100),
            inbox_count,
            skills_section,
            workspace_path: self.workspace.display().to_string(),
            certificate_info: self.certificate_info.as_ref().map(|ci| {
                crate::context::prompt::CertificateInfo {
                    certifier: ci.certifier.clone(),
                    self_signed: ci.self_signed,
                    capabilities: ci.capabilities.clone(),
                    name: ci.name.clone(),
                    budget_per_task: ci.budget_per_task,
                    budget_per_hour: ci.budget_per_hour,
                    budget_per_day: ci.budget_per_day,
                    budget_per_week: ci.budget_per_week,
                    budget_per_month: ci.budget_per_month,
                    budget_lifetime: ci.budget_lifetime,
                    budget_enforcement: ci.budget_enforcement.clone(),
                }
            }),
            has_external_messages,
            identity_soul,
            auto_recall_ids,
            basket_health: self.state.onchain.basket_health.clone(),
            spendable_output_count: self.state.budget.cached_spendable_count,
            instructions: {
                use crate::context::instructions::{format_instructions, load_instructions};
                let files = load_instructions(&self.workspace);
                format_instructions(&files)
            },
            env_snapshot: Some(self.build_env_snapshot().await),
            working_memory_section: {
                let wm = self.working_memory.read().await;
                wm.format_for_prompt()
            },
        }
    }

    /// Build environment snapshot for the system prompt (#287).
    ///
    /// Pre-loads tool categories, model capabilities, and workspace listing
    /// so the agent doesn't waste turns discovering them.
    async fn build_env_snapshot(&self) -> crate::context::prompt::EnvSnapshot {
        use crate::context::prompt::{EnvSnapshot, RecentTask};

        let registry = self.tools.read().await;

        // Tool category counts
        let mut cat_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut always_on = 0usize;
        for tool in registry.list_prompt_tools() {
            *cat_counts.entry(tool.category.clone()).or_insert(0) += 1;
            always_on += 1;
        }
        let discoverable = registry
            .all_tool_summaries()
            .len()
            .saturating_sub(always_on);
        let mut tool_categories: Vec<(String, usize)> = cat_counts.into_iter().collect();
        tool_categories.sort_by(|a, b| b.1.cmp(&a.1)); // sort by count desc

        // Model info
        let model = &self.config.llm.default_model;
        let ctx_window = self.config.llm.context_window;
        let max_tokens = self.config.llm.max_tokens;
        let model_info =
            format!("{model}. Context: {ctx_window} tokens, max output: {max_tokens} tokens.",);

        // Recent tasks from transcript history on disk (last 3)
        let mut recent_tasks = Vec::new();
        let tasks_dir = self.workspace.parent().map(|p| p.join("tasks"));
        if let Some(ref tasks_dir) = tasks_dir {
            if let Ok(entries) = std::fs::read_dir(tasks_dir) {
                let mut task_dirs: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .collect();
                task_dirs.sort_by(|a, b| {
                    b.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        .cmp(
                            &a.metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                        )
                });
                for entry in task_dirs.iter().take(3) {
                    let session = entry.path().join("session.jsonl");
                    if session.exists() {
                        let transcript = crate::transcript::Transcript::new(session);
                        let state = transcript.reconstruct_state();
                        recent_tasks.push(RecentTask {
                            task: state.task,
                            status: if state.completed {
                                "complete".to_string()
                            } else {
                                "interrupted".to_string()
                            },
                            sats_spent: state.sats_spent,
                            iterations: state.iterations,
                        });
                    }
                }
            }
        }

        // Workspace top-level listing
        let mut workspace_listing = Vec::new();
        let mut workspace_file_count = 0usize;
        if let Ok(entries) = std::fs::read_dir(&self.workspace) {
            for entry in entries.flatten() {
                workspace_file_count += 1;
                if let Some(name) = entry.file_name().to_str() {
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    workspace_listing.push(if is_dir {
                        format!("{name}/")
                    } else {
                        name.to_string()
                    });
                }
            }
            workspace_listing.sort();
        }

        EnvSnapshot {
            tool_categories,
            always_on_count: always_on,
            discoverable_count: discoverable,
            model_info,
            recent_tasks,
            workspace_listing,
            workspace_file_count,
        }
    }

    /// Get the IDs of memory entries already shown in the static memory summary.
    ///
    /// These are the last 3 session entries and last 5 knowledge entries
    /// that `build_memory_summary()` includes. Used to de-duplicate
    /// auto-recalled results.
    fn get_static_memory_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();

        if let Ok(sessions) = self
            .memory_store
            .list(Some(crate::memory::store::MemoryCategory::Session))
        {
            for entry in sessions.into_iter().take(3) {
                ids.push(entry.id);
            }
        }

        if let Ok(knowledge) = self
            .memory_store
            .list(Some(crate::memory::store::MemoryCategory::Knowledge))
        {
            for entry in knowledge.into_iter().take(5) {
                ids.push(entry.id);
            }
        }

        ids
    }

    /// Build a memory summary from stored knowledge and recent sessions.
    ///
    /// Loads the most recent entries from each category and formats them
    /// into a concise summary for the system prompt.
    fn build_memory_summary(&self) -> String {
        let mut sections = Vec::new();

        // Recent session summaries (last 3)
        if let Ok(sessions) = self
            .memory_store
            .list(Some(crate::memory::store::MemoryCategory::Session))
        {
            let recent: Vec<_> = sessions.into_iter().take(3).collect();
            if !recent.is_empty() {
                let mut lines = vec!["## Recent Sessions".to_string()];
                for entry in &recent {
                    let preview = if entry.content.len() > 200 {
                        format!(
                            "{}...",
                            &entry.content[..entry
                                .content
                                .char_indices()
                                .take_while(|(i, _)| *i < 200)
                                .last()
                                .map(|(i, c)| i + c.len_utf8())
                                .unwrap_or(200)]
                        )
                    } else {
                        entry.content.clone()
                    };
                    lines.push(format!(
                        "- **{}**: {}",
                        entry.created.format("%Y-%m-%d %H:%M"),
                        preview
                    ));
                }
                sections.push(lines.join("\n"));
            }
        }

        // Knowledge entries (last 5)
        if let Ok(knowledge) = self
            .memory_store
            .list(Some(crate::memory::store::MemoryCategory::Knowledge))
        {
            let recent: Vec<_> = knowledge.into_iter().take(5).collect();
            if !recent.is_empty() {
                let mut lines = vec!["## Knowledge".to_string()];
                for entry in &recent {
                    let preview = if entry.content.len() > 150 {
                        format!(
                            "{}...",
                            &entry.content[..entry
                                .content
                                .char_indices()
                                .take_while(|(i, _)| *i < 150)
                                .last()
                                .map(|(i, c)| i + c.len_utf8())
                                .unwrap_or(150)]
                        )
                    } else {
                        entry.content.clone()
                    };
                    let tags = if entry.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", entry.tags.join(", "))
                    };
                    lines.push(format!("- {preview}{tags}"));
                }
                sections.push(lines.join("\n"));
            }
        }

        if sections.is_empty() {
            String::new()
        } else {
            sections.join("\n\n")
        }
    }

    /// Extract all proof txids from the transcript.
    pub fn proof_txids(&self) -> Vec<String> {
        let mut txids: Vec<String> = Vec::new();
        for event_type in &["proof_created", "checkpoint_created"] {
            for e in self.transcript.get_events_by_type(event_type) {
                if let Some(txid) = e.data.get("txid").and_then(|v| v.as_str()) {
                    txids.push(txid.to_string());
                }
            }
        }
        txids
    }
}

// =============================================================================
// Factory function
// =============================================================================

/// Create a new agent loop with default configuration.
///
/// For CLI mode, `memory_dir` and `global_workspace` should be `None`
/// (defaults to `workspace/memory` and `workspace` respectively).
/// For HTTP server mode, pass the global memory directory and global workspace
/// so memories and schedules are shared across tasks.
pub fn create_loop(
    config: DmConfig,
    workspace: PathBuf,
    memory_dir: Option<PathBuf>,
    global_workspace: Option<PathBuf>,
    wallet: Arc<dyn WalletBackend + Send + Sync>,
) -> DmLoop {
    DmLoop::new(config, workspace, memory_dir, global_workspace, wallet)
}

/// Create a worm loop with an x402 rate limiter and circuit breaker registry (server mode).
#[allow(clippy::too_many_arguments)]
pub fn create_loop_with_rate_limiter(
    config: DmConfig,
    workspace: PathBuf,
    memory_dir: Option<PathBuf>,
    global_workspace: Option<PathBuf>,
    wallet: std::sync::Arc<dyn WalletBackend + Send + Sync>,
    rate_limiter: Option<std::sync::Arc<crate::x402::rate_limit::RateLimiterRegistry>>,
    circuit_breakers: std::sync::Arc<CircuitBreakerRegistry>,
    shared_messagebox: Option<std::sync::Arc<MessageBoxClient>>,
) -> DmLoop {
    DmLoop::new_with_rate_limiter(
        config,
        workspace,
        memory_dir,
        global_workspace,
        wallet,
        rate_limiter,
        circuit_breakers,
        shared_messagebox,
    )
}

// =============================================================================
// Tests (LoopState defaults)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_state_defaults() {
        let state = LoopState::default();
        assert!(state.onchain.task_commitment.is_none());
        assert!(state.onchain.budget_allocation.is_none());
        assert!(state.onchain.last_checkpoint.is_none());
        assert!(state.onchain.last_proof_hash.is_none());
        assert_eq!(state.exec.iteration, 0);
        assert_eq!(state.budget.sats_spent, 0);
    }

    #[test]
    fn test_loop_state_token_fields() {
        let mut state = LoopState::default();
        let token = state::StateToken::new(
            state::TokenType::TaskCommitment,
            serde_json::json!({"task_hash": "abc123"}),
        );
        state.onchain.task_commitment = Some(token);
        assert!(state.onchain.task_commitment.is_some());
        assert_eq!(
            state.onchain.task_commitment.as_ref().unwrap().token_type,
            state::TokenType::TaskCommitment,
        );

        let budget_token = state::StateToken::new(
            state::TokenType::BudgetAllocation,
            serde_json::json!({"budget_cap": 20_000_000}),
        );
        state.onchain.budget_allocation = Some(budget_token);
        assert!(state.onchain.budget_allocation.is_some());
        assert_eq!(
            state.onchain.budget_allocation.as_ref().unwrap().token_type,
            state::TokenType::BudgetAllocation,
        );
    }
}
