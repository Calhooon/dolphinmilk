# src/runner/
> Core agent loop — OBSERVE → THINK → ACT → RECORD → BUDGET CHECK.

## Overview

The runner is the heart of the agent. It drives the autonomous agent loop: each iteration polls the inbox, constructs a prompt, calls the LLM via x402 payment, executes any tool calls, creates on-chain proofs, and checks budget limits. The loop continues until the LLM returns a text-only response (no tool calls), budget is exhausted, a circuit breaker trips, or the task is cancelled. Maximum 50 iterations per task.

The module is split into eight files by responsibility: types and construction (`mod.rs`), task lifecycle orchestration (`lifecycle.rs`), per-iteration phase logic (`step.rs`), tool execution dispatch (`execute.rs`), moderation engine integration (`moderation.rs`), tool approval gate (`approval.rs`), a fallback parser for reasoning models (`text_extract.rs`), and auto-escalation detection (`escalation.rs`).

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 1401 | Types (`DmLoop`, `LoopState` with 6 sub-structs, `ContinuationState`, `ReplyObligation`, `ConversationChainBreak`), constructors (`new`, `new_with_rate_limiter` — both take `wallet` param), helper methods (`record_proof`, `record_token`, `execute_tool`, `build_prompt_context`, `build_memory_summary`), `register_mcp_tools()` for post-construction MCP tool injection, `apply_model_capabilities()` for discovery-based context limits, `search_tools` registration, `signals_tool_intent()` intent detection, factory functions `create_loop()` and `create_loop_with_rate_limiter()`, tool approval methods (`set_approval_tx`, `merge_cert_approval_tools`, `requires_approval`), `set_conversation_chain_status()` for BRC-60 integrity, `set_user_attachments()` for multimodal image blocks |
| `lifecycle.rs` | 781 | Task lifecycle: `setup_task()` (reset state, apply allowlists, bootstrap identity, parse capabilities, apply cert budget limits with 6 tiers + enforcement mode, build moderation engine, apply cert-driven tool approval, BRC-60 chain break proof, basket health query, create BRC-48 tokens, fire `TaskStarted` hook), `run_loop()` (iterate with error handling + cancellation), `teardown_task()` (tool cleanup, session summary, transcript HMAC with protocol `[2, "dolphin milk transcript"]`, BRC-18 completion/budget/custody proofs, BRC-48 checkpoint with HMAC, relinquish TaskCommitment + CapabilityDeclaration, fire `TaskCompleted`/`TaskFailed` hooks), `run()` (public entry point composing all three). All lifecycle methods take `&StepContext` |
| `step.rs` | 1287 | Per-iteration phases: `StepContext` struct (events sender, task_id, cancel flag), `observe()` (inbox polling with auto-decrypt BRC-78 + auto-verify BRC-77, message partitioning, reply obligations, moderation, BRC-18 MessageReceive proofs, `OnMessageReceived` hook), `build_llm_messages()` (system prompt, history, user attachment injection, offloaded result preview substitution, microcompact, two-phase LLM compaction with `PreCompact`/`PostCompact` hooks, token-based auto-compaction), `think_step()` (budget pre-check with advisory mode, stall detection timeout, rate limiting, LLM call via circuit breaker, sats_effective tracking, Prometheus metrics, `OnPaymentMade` hook, LLM response moderation), `record_and_resolve()` (decision proof with message hashes + cert hash + pre_state_root, budget token update, done/nudge logic, context offloading). `step()` orchestrates all phases plus state checkpoint, `IterationStart`/`IterationEnd` hooks, truncation recovery, terse resume, intent nudge loop, periodic auto-verify, and periodic BRC-48 consistency check |
| `execute.rs` | 1065 | Tool execution: `execute_tools()` (parallel via JoinSet for >1, sequential for 1, leak detection, tool output moderation, side effects, proofs, cross-agent message hash tracking, BRC-18 MessageSend proofs, large result offloading, artifact manifest update), `execute_tools_sequential()` (approval gate, `PreToolExecution`/`PostToolExecution`/`ToolError` hooks, tool input moderation, timing), `execute_tools_parallel()` (JoinSet dispatch), `execute_tool_standalone()` for parallel JoinSet dispatch, `generate_smart_preview()` for content-aware large result previews, `update_artifact_manifest()` for workspace file tracking. Constants: `TOOL_RESULT_OFFLOAD_THRESHOLD`, `NO_OFFLOAD_TOOLS`, `PAID_TOOLS`, `SYSTEM_FILES`, `SYSTEM_DIRS`. Types: `ToolExecResult`, `MessageSendInfo` |
| `moderation.rs` | 120 | Runner-level moderation helpers: `ModerationOutcome` enum (Pass/Flagged/Blocked), `build_moderation_engine()` (cert policy + config construction), `moderate_content()` (moderate + log + record transcript event), `record_moderation_event()` (transcript HashMap helper). Used at 4 points: inbox messages, LLM response, tool inputs, tool outputs |
| `approval.rs` | 164 | Tool approval gate: `wait_for_approval()` file-based manual approval workflow with SSE events + staged map + timeout |
| `text_extract.rs` | 330 | `extract_tool_calls_from_text()` fallback for reasoning models (gpt-5-mini, o4-mini) that emit tool calls as text. Three strategies: JSON code blocks, inline JSON objects, `{"tool_calls": [...]}` wrapper. Includes `find_json_objects()` brace-balanced parser. 13 unit tests |
| `escalation.rs` | 485 | Auto-escalation to human when agent is stuck. `EscalationDetector` monitors 4 signals: tool loops (same tool+params 3+ times), budget threshold (>80%), error threshold (>3 errors), explicit uncertainty (phrase detection in LLM text). Emits `EscalationEvent` with reason, iteration, message, timestamp, and `should_pause` flag. `EscalationResolution` for human-provided guidance. `EscalationStatus` tracks escalated/resolved state. `EscalationProofData` for on-chain proof creation. 14 unit tests |

## Key Exports

### Types

| Type | Visibility | Description |
|------|-----------|-------------|
| `DmLoop` | `pub` | The agent loop struct. Owns config, workspace, transcript, context manager, tool registry (`Arc<RwLock<ToolRegistry>>`), loop detector, mutable `LoopState`, wallet/auth/messagebox clients, memory store+index+sync, budget tracker, skill registry, cached certificate info, moderation engine, optional rate limiter, tool approval config, optional staged-transactions map, optional Prometheus metrics, circuit breaker registry, optional user attachment blocks, hook registry |
| `LoopState` | `pub` | Mutable per-task state, decomposed into 6 focused sub-structs (#209): `exec: ExecutionState`, `budget: BudgetState`, `comms: CommunicationState`, `onchain: OnChainState`, `storage: StorageState`, `auth: AuthState` |
| `ExecutionState` | `pub` | Iteration counter, done flag, result text, error text |
| `BudgetState` | `pub` | `sats_spent`, `cached_balance`, `cached_spendable_count` (output count in default basket), `over_budget` (advisory mode flag) |
| `CommunicationState` | `pub` | `pending_replies: Vec<ReplyObligation>`, `nudge_count` |
| `OnChainState` | `pub` | `last_proof_hash`, `last_checkpoint`, `task_commitment`, `budget_allocation`, `capability_declaration`, `conversation_chain_verified`, `conversation_chain_break`, `basket_health` |
| `StorageState` | `pub` | `task`, `task_id`, `offloaded_results` (HashMap of call_id -> (file_path, preview)), `offload_counter` |
| `AuthState` | `pub` | `external_origin`, `capabilities`, `cert_hash`, `cert_type` |
| `ContinuationState` | `pub` | Serializable pause/resume state: `id`, `task`, `transcript_path`, `iteration`, `reason`, `wake_at`, `created_at`, `last_proof_hash` |
| `ConversationChainBreak` | `pub` | BRC-60 chain break details: `conversation_id`, `first_break_seq`, `expected_hash`, `actual_hash`, `total_breaks` |
| `ReplyObligation` | `pub` | Tracks an external sender needing a response: `sender_key` + `message_box` |
| `StepContext` | `pub(crate)` | Stable context for step methods: `events` (broadcast sender), `task_id`, `cancel` (AtomicBool) |
| `EscalationDetector` | `pub` | Monitors agent behavior for stuck patterns (from `escalation.rs`) |
| `EscalationEvent` | `pub` | Describes why escalation was triggered: reason, iteration, message, timestamp, should_pause |
| `EscalationReason` | `pub` | Enum: `ToolLoop`, `BudgetThreshold`, `ErrorThreshold`, `ExplicitUncertainty` |
| `EscalationResolution` | `pub` | Human-provided guidance to resume a paused task |
| `EscalationStatus` | `pub` | Current escalation state: escalated flag, event, resolution |
| `EscalationProofData` | `pub` | Data for on-chain escalation proof: reason, trigger_type, iteration |
| `ModerationOutcome` | `pub(crate)` | Outcome of a moderation check: `Pass`, `Flagged`, `Blocked`. Action-free — callers decide what to do (from `moderation.rs`) |
| `ToolExecResult` | `pub(crate)` | Type alias for tool execution result tuple: `(index, call_id, name, arguments_str, arguments, output, success)` (from `execute.rs`) |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_NUDGES` | 2 | Attempts to nudge LLM to call `send_message` before force-sending (reply obligation nudges) |
| `MAX_INTENT_NUDGES` | 2 | Attempts to nudge LLM when it expresses tool intent without making tool calls |
| `TOOL_INTENT_NUDGE` | string | Nudge message injected when LLM describes tool intent without calling tools |
| `TOOL_RESULT_OFFLOAD_THRESHOLD` | 8000 | Tool results larger than this (bytes) are offloaded to workspace files; LLM receives a smart preview (in `execute.rs`) |
| `NO_OFFLOAD_TOOLS` | 5 tools | Tools whose results are never offloaded: `file_read`, `memory_search`, `memory_recall`, `read_tool_output`, `execute_bash` — retrieval tools where offloading would cause cascading preview loops (in `execute.rs`) |
| `PAID_TOOLS` | 3 tools | Tools that make paid x402 requests: `x402_call`, `generate_image`, `upload_to_nanostore` — only these have `sats_paid` extracted from output (in `execute.rs`) |
| `SYSTEM_FILES` | 4 files | Files excluded from artifact manifest: `session.jsonl`, `budget.jsonl`, `artifacts.json`, `fork_context.json` (in `execute.rs`) |
| `SYSTEM_DIRS` | 4 dirs | Directories excluded from artifact scanning: `beef`, `continuations`, `delivery_queue`, `screenshots` (in `execute.rs`) |
| `TOOL_LOOP_THRESHOLD` | 3 | Same tool+params calls before escalation (in `escalation.rs`) |
| `ERROR_THRESHOLD` | 3 | Errors before escalation triggers (in `escalation.rs`) |
| `BUDGET_THRESHOLD_PERCENT` | 80.0 | Budget percentage that triggers escalation (in `escalation.rs`) |
| `MAX_TRUNCATION_RETRIES` | 2 | Retries for empty truncated LLM responses (in `step.rs`) |
| `MAX_TERSE_RETRIES` | 3 | Terse resume continuations for truncated-but-has-content responses (in `step.rs`) |

### Functions

| Function | Visibility | Description |
|----------|-----------|-------------|
| `create_loop(config, workspace, memory_dir, global_workspace, wallet)` | `pub` | Factory that calls `DmLoop::new()` |
| `create_loop_with_rate_limiter(config, workspace, memory_dir, global_workspace, wallet, rate_limiter, circuit_breakers)` | `pub` | Factory for server mode with x402 rate limiting and circuit breaker registry |
| `content_hash(content)` | `pub` | SHA-256 hex digest of content string |
| `signals_tool_intent(response)` | `pub` | Detect when LLM describes tool actions (e.g. "Let me search...") without issuing tool calls. Strips code blocks, checks exclusion phrases ("let me explain", etc.), then matches intent prefixes + action verbs |
| `emit_event(ctx, event)` | `pub(crate)` | Send `StepEvent` to broadcast channel (no-op in CLI mode). Takes `&StepContext` |
| `extract_tool_calls_from_text(text, known_tools)` | `pub(crate)` | Parse tool calls from LLM text output (reasoning model fallback) |
| `generate_smart_preview(content, file_path)` | `pub(crate)` | Content-aware preview of large tool results: detects JSON arrays (count + fields + first 2 items), JSON objects (keys + preview, or `summary` field shortcut), or plain text (word count + preview). Capped at 2000 chars, with 1MB fast-path for very large content |
| `execute_tool_standalone(tools, capabilities, name, arguments)` | file-private | Standalone tool execution for parallel JoinSet tasks (replicates capability check logic without `&self`) |
| `update_artifact_manifest(workspace, tool_results)` | file-private | Scan workspace for new files and update `artifacts.json` manifest. Skips system files/dirs, scans `uploads/` for user attachments. Tracks file type, size, creating tool |

### Key Methods on `DmLoop`

| Method | File | Description |
|--------|------|-------------|
| `new(config, workspace, memory_dir, global_workspace, wallet)` | `mod.rs` | Constructor: delegates to `new_with_rate_limiter` with `None` rate limiter and new `CircuitBreakerRegistry`. Takes `wallet: Arc<dyn WalletBackend>` |
| `new_with_rate_limiter(config, workspace, memory_dir, global_workspace, wallet, rate_limiter, circuit_breakers)` | `mod.rs` | Full constructor: initializes all subsystems, registers tools (sandbox, wallet, memory, messagebox, x402 generic+recipe, schedule, conversation, browser, analytics, discovery, introspect, orchestration, search_tools). Browser, discovery, analytics, introspect, and orchestration tools are discoverable, not always-on. Initializes `HookRegistry` from config |
| `register_mcp_tools(tool_defs)` | `mod.rs` | Register MCP-proxied wallet tools post-construction and rebuild the `search_tools` snapshot. Called before `run()` when MCP wallet bridge is available |
| `apply_model_capabilities(capabilities)` | `mod.rs` | Apply discovered model capabilities to update context window limits. Resolution chain: discovered caps -> hardcoded fallback -> config cap. Config values act as caps (user can lower for cost control). Called by `spawn_task()` when model capabilities have been pre-fetched |
| `set_conversation_chain_status(verified, chain_break)` | `mod.rs` | Set BRC-60 hash chain verification result. Called by `spawn_task()`. If `verified=false`, runner creates a ConversationBreak proof in `setup_task()` |
| `set_user_attachments(blocks)` | `mod.rs` | Set user-provided image attachments as OpenAI-format content blocks. Injected into the first user message during `build_llm_messages()` |
| `run(task, max_iterations, tx, cancel)` | `lifecycle.rs` | Public entry point: creates `StepContext`, then `setup_task` -> `run_loop` -> `teardown_task` |
| `setup_task(task, ctx)` | `lifecycle.rs` | Reset state (preserving external_origin and chain status), apply tool allowlist for external-origin tasks, record session start, fetch BRC-52 cert info, compute cert hash, build moderation engine via `build_moderation_engine()`, parse capabilities for runtime enforcement, apply certificate-derived budget limits (6 tiers + enforcement mode), apply cert-driven tool approval, bootstrap identity from certificate, create BRC-60 ConversationBreak proof if chain broken, query basket UTXO health, create BRC-48 tokens (TaskCommitment, BudgetAllocation, CapabilityDeclaration), fire `TaskStarted` hook |
| `run_loop(max_iterations, ctx)` | `lifecycle.rs` | Iterate `step()` until done/cancelled/max iterations. Classifies errors as budget/loop/worm |
| `teardown_task(ctx)` | `lifecycle.rs` | Clean up tool resources via `cleanup_all()`, record session end, fire `TaskCompleted`/`TaskFailed` hook, create session summary in memory, encrypt summary, compute transcript HMAC (protocol `[2, "dolphin milk transcript"]`), create TaskCompletion + BudgetSnapshot + Custody proofs, create Checkpoint token (with HMAC), relinquish TaskCommitment + CapabilityDeclaration, update final BudgetAllocation |
| `step(ctx)` | `step.rs` | One iteration: state checkpoint (every 5 iters) -> `IterationStart` hook -> periodic auto-verify -> periodic BRC-48 consistency check -> observe -> build -> think -> truncation recovery -> terse resume -> intent nudge loop -> act -> record -> `IterationEnd` hook |
| `observe()` | `step.rs` | Poll inbox with auto-decrypt BRC-78 + auto-verify BRC-77, partition self vs external messages, acknowledge all, moderate inbox messages, inject external messages with sanitization (Layer 1+2), build `ReplyObligation` list, create BRC-18 MessageReceive proofs, fire `OnMessageReceived` hook |
| `build_llm_messages(has_external)` | `step.rs` | Build system prompt via `PromptContext`, record skill telemetry, prepend prior conversation messages, inject user attachment content blocks into last user message, substitute offloaded tool result previews, run microcompact (zero-cost old tool result cleanup), two-phase LLM compaction with hooks (budget-aware: skips when >90% spent), token-based auto-compaction via `check_and_compact()` |
| `think_step(messages, ctx)` | `step.rs` | Budget pre-check (500 sat estimate) with advisory mode, acquire rate limit token, stall detection timeout (`stall_timeout_secs`, default 120s), call `think_with_circuit_breaker()` for provider failover, record sats_effective, record Prometheus metrics, fire `OnPaymentMade` hook, moderate LLM response text, emit SSE events |
| `execute_tools(tool_calls, ctx)` | `execute.rs` | Dispatch tool calls: parallel via JoinSet for >1, sequential for 1. Leak detection, moderate outputs, track `send_message` for obligations and message hashes, create proofs, offload large results, handle `continue_task` pause, feed loop detector, update artifact manifest |
| `record_and_resolve(result, ...)` | `step.rs` | Create BRC-18 Decision proof (enriched with model/sats/txid/memory_ids/message_hashes/cert_hash/cert_type/pre_state_root), update BRC-48 BudgetAllocation token, store BEEF payment receipt, check budget drain, handle done/nudge logic, offload large response text |

## Iteration Lifecycle

Each `step()` call runs these phases:

0. **STATE CHECKPOINT** (in `step()`) — Every 5 iterations (#273), records a snapshot of key loop state (iteration, sats_spent, last_proof_hash, model) for crash recovery. Fires `IterationStart` hook.

1. **AUTO-VERIFY** (in `step()`) — Periodic proof chain verification. Every `config.lifecycle.verify_interval` iterations (0 = disabled), reads the proof chain from on-chain UTXOs and compares against in-memory state.

1b. **BRC-48 CONSISTENCY CHECK** (in `step()`) — Periodic token consistency check (#196). Every `config.lifecycle.consistency_check_interval` iterations (0 = disabled), reads basket token counts and compares against in-memory `LoopState`.

2. **OBSERVE** (`observe`) — Poll MessageBox inboxes. Fetch wallet balance and spendable output count. Partition messages into external vs self-sent. Acknowledge all. Self-messages become "DELIVERY CONFIRMED" transcript entries. External messages are auto-decrypted (BRC-78) and auto-verified (BRC-77), then moderated, sanitized, and injected. Reply obligations built from external senders. BRC-18 MessageReceive proofs created. `OnMessageReceived` hook fired.

3. **BUILD** (`build_llm_messages`) — Construct `PromptContext`. Build system prompt. Record skill telemetry. Prepend prior conversation messages. Inject user attachment content blocks (multimodal images) into the last user message. Substitute offloaded tool result previews. Run microcompact (zero-cost cleanup of stale tool results). Run two-phase LLM compaction if needed (with `PreCompact`/`PostCompact` hooks; budget-aware: skips when >90% spent). Run token-based auto-compaction via `check_and_compact()`.

4. **THINK** (`think_step`) — Check budget limit (estimate 500 sats). Advisory mode support. Acquire rate limit token. Stall detection: abort LLM call if no response within `stall_timeout_secs` (default 120s, configurable). Call `think_with_circuit_breaker()`. Record sats_effective. Record Prometheus metrics. Fire `OnPaymentMade` hook. Moderate LLM response text.

4b. **TRUNCATION RECOVERY** (in `step()`) — If the LLM response was truncated (`finish_reason=length`) and produced no usable output, retry with a conciseness instruction. Max `MAX_TRUNCATION_RETRIES` (2) retries, then force-stop to prevent runaway spending.

4c. **TERSE RESUME** (in `step()`) — If the LLM response was truncated but HAS content, auto-continue with a "no recap" instruction. Saves 200-500 tokens that would otherwise be wasted on preamble. Max `MAX_TERSE_RETRIES` (3) continuations per iteration.

5. **INTENT NUDGE** (in `step()`) — If the LLM returned text-only but `signals_tool_intent()` detects it described tool actions, inject `TOOL_INTENT_NUDGE` and re-call. Up to `MAX_INTENT_NUDGES` (2) retries.

6. **ACT** (`execute_tools` in `execute.rs`) — Dispatch tool calls with parallel/sequential dispatch. Approval gate, hook integration (`PreToolExecution`/`PostToolExecution`/`ToolError`), moderation, leak detection, Prometheus metrics, capability checks, cost tracking, message hash tracking, proofs, offloading, artifact manifest update, loop detection.

7. **RECORD** (`record_and_resolve`) — Budget recording, drain check, BRC-18 Decision proof, BRC-48 BudgetAllocation update, BEEF receipt, done/nudge logic. Fire `IterationEnd` hook.

## Hook Integration

The runner fires lifecycle hooks at key points via `HookRegistry` (from `src/hooks/`). Hooks are configured in `[hooks]` config section. Blocking hooks (where applicable) can prevent actions.

| Hook Event | Phase | Blocking | Description |
|------------|-------|----------|-------------|
| `TaskStarted` | setup | no | After BRC-48 token creation |
| `TaskCompleted` / `TaskFailed` | teardown | no | After session end recording |
| `IterationStart` | step | no | Before auto-verify |
| `IterationEnd` | step | no | After record_and_resolve |
| `PreToolExecution` | execute | **yes** | Before tool execution — can block |
| `PostToolExecution` | execute | no | After successful tool execution |
| `ToolError` | execute | no | After failed tool execution |
| `OnProofCreated` | record_proof | no | After BRC-18 proof creation |
| `OnPaymentMade` | think_step | no | After LLM payment |
| `OnMessageReceived` | observe | no | After inbox message receive |
| `PreCompact` | build | **yes** | Before LLM compaction — can block |
| `PostCompact` | build | no | After compaction |

## Resilience Features (#273, #259, #278)

Three resilience mechanisms added in Phase 1:

**State Checkpoints** — Every 5 iterations, `step()` records a `state_checkpoint` transcript event with iteration, sats_spent, last_proof_hash, and model. Enables crash recovery without replaying the entire transcript.

**Stall Detection** — `think_step()` wraps the LLM call in `tokio::time::timeout()`. Default 120s, configurable via `config.llm.stall_timeout_secs`. Stalled requests produce a `DmError::tool` error that stops the iteration cleanly.

**Truncation Recovery** — Two strategies in `step()`:
1. Empty truncated response (no text, no tools): retry with conciseness instruction, max 2 retries, then force-stop
2. Truncated-with-content: terse resume with "no recap" instruction, max 3 continuations per iteration

## Content Moderation (`moderation.rs`)

Centralized in `moderation.rs`. `moderate_content()` returns `ModerationOutcome` (Pass/Flagged/Blocked) — callers decide the action. Called at four points:

1. **Inbox messages** (`observe`) — Blocked -> skip message
2. **LLM response text** (`think_step`) — Blocked -> error+done
3. **Tool inputs** (`execute_tools_sequential`/`execute_tools_parallel`) — Blocked -> error result
4. **Tool outputs** (`execute_tools`) — Blocked -> replace with `[MODERATED: content blocked by policy]`

## Tool Approval Gate (`approval.rs`)

Tools listed in `approval_tools` require manual human approval before execution. `wait_for_approval()` stages the call, emits `ApprovalRequired` SSE event, polls for approval/abort/timeout. Parallel execution falls back to sequential when any tool in the batch requires approval.

## Auto-Escalation (`escalation.rs`)

Detects when the agent is stuck: tool loops (3+ identical calls), budget threshold (>80%), error threshold (>3 errors), explicit uncertainty (15 phrases, case-insensitive). `mark_triggered()` prevents duplicates. `build_event()` constructs `EscalationEvent`. `build_proof_data()` produces on-chain proof data.

## Artifact Manifest (`execute.rs`)

`update_artifact_manifest()` runs after tool execution to track workspace files in `artifacts.json`. Scans the workspace root for new files, plus `uploads/` for user-provided attachments. Excludes system files (`session.jsonl`, `budget.jsonl`, etc.) and system directories (`beef`, `continuations`, etc.). Tracks file name, type (image/video/audio/document/file), size, creating tool, and timestamp.

## On-Chain Artifacts Per Task

| Phase | Artifact | Type | Description |
|-------|----------|------|-------------|
| Setup | TaskCommitment | BRC-48 | Task hash, budget cap, model, skills hash |
| Setup | BudgetAllocation | BRC-48 | Initial budget cap |
| Setup | CapabilityDeclaration | BRC-48 | Tools, skills, identity, version |
| Setup | ConversationBreak | BRC-18 | If BRC-60 chain integrity broken |
| Per-iteration | MessageReceive | BRC-18 | Per external inbox message |
| Per-iteration | Decision | BRC-18 | Tool names, model, sats, txid, memory IDs, message hashes, cert hash, cert type, pre_state_root |
| Per-iteration | BudgetAllocation | BRC-48 | Updated task_spent + iteration |
| Per-iteration | MemoryCommitment | BRC-18 | Content hash of stored memory entry |
| Per-iteration | CapabilityProof | BRC-18 | For paid tools and discovery tools |
| Per-iteration | MessageSend | BRC-18 | Per successful send_message |
| Teardown | TaskCompletion | BRC-18 | Task hash + result content hash |
| Teardown | BudgetSnapshot | BRC-18 | Total sats, iterations, per-service breakdown |
| Teardown | Custody | BRC-18 | Billing verification (#205) |
| Teardown | Checkpoint | BRC-48 | Task summary, iterations, sats, result preview, transcript HMAC |
| Teardown | BudgetAllocation | BRC-48 | Final with full service breakdown |

## Reply Obligation Protocol

Prevents the agent from returning text-only responses when external senders need replies:

1. `observe()` clears `pending_replies` each iteration, rebuilds from external inbox messages (deduped by sender key)
2. `execute_tools()` fulfills obligations when `send_message` is called with matching recipient
3. `record_and_resolve()` checks unfulfilled obligations:
   - Nudges remaining -> inject explicit instruction to call `send_message`
   - Nudges exhausted -> force-send replies with task result
   - All fulfilled + only messaging tools -> mark done

## Injection Defense (4 Layers)

External-origin tasks get four layers of protection:

1. **Content sanitization** (`sanitize_external_content`) — Strip control chars, truncate to 2000 chars
2. **Boundary markers** (`wrap_with_boundary`) — Random-ID markers around untrusted content
3. **Tool allowlist** (`external_tool_allowlist`) — Restrict to 10 safe tools (applied in `setup_task`)
4. **System prompt warning** — `has_external_messages` flag triggers injection defense section in prompt

Parent sender (matching `config.parent.identity_key`) bypasses all restrictions.

## Tool Result Offloading (`execute.rs`)

Large tool results (>8KB) are offloaded to workspace files. `generate_smart_preview()` detects content type: JSON arrays (count + fields + first 2 items), JSON objects (keys + preview, or `summary` field shortcut), plain text (word count + preview). Very large content (>1MB) uses fast-path. Retrieval tools in `NO_OFFLOAD_TOOLS` are exempt.

## Related

- [`../CLAUDE.md`](../CLAUDE.md) — Full source directory inventory and module descriptions
- [`../server/CLAUDE.md`](../server/CLAUDE.md) — HTTP API that spawns tasks via `spawn_task()` using `DmLoop::run()`
- `../think/` — LLM inference called by `think_step()`, circuit breaker wrapping, text extraction (#211)
- `../context/` — `ContextManager` and `PromptContext` used by `build_llm_messages()`
- `../onchain/` — `proofs` and `state` modules used by `record_proof()` and `record_token()`
- `../session/` — `Transcript` and `StepEvent` used throughout
- `../sanitize.rs` — 4-layer injection defense + `redact_leaks()` used in `observe()`, `setup_task()`, and `execute_tools()`
- `../moderation.rs` — `ModerationEngine` used at 4 moderation points
- `../tools/registry.rs` — `ToolRegistry` shared via `Arc<RwLock>`, `required_capability()` for capability enforcement
- `../hooks/` — `HookRegistry` for lifecycle hook events, `HookEvent` variants, `is_blocked()` check
- `../memory/search.rs` — BM25 auto-recall used in `build_prompt_context()`
- `../metrics.rs` — `MetricsRegistry` for Prometheus counters/histograms
- `../x402/circuit_breaker.rs` — `CircuitBreakerRegistry` for LLM provider failover
- `../../tests/test_self_message.rs` — 26 integration tests for self-message prevention and reply obligations
