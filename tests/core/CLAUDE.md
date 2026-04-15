# core/

> 427 tests across 14 files covering the agent's brain: LLM inference, context management, compaction, model capabilities, loop prevention, identity, and runner lifecycle.

## Overview

Core tests validate the central modules that drive the agent loop — the think module (LLM calls, provider routing, message conversion), the context pipeline (prompt construction, truncation, compaction, intelligent token analysis, forking), loop/intent detection, self-message prevention, model capability resolution, tool result offloading, and runner lifecycle types (`LoopState` sub-structs, `DmLoop` construction and cancellation). No wallet or network access required; all HTTP is mocked.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_think.rs | 61 | ThinkResult serde, model detection (`is_reasoning_model`, `is_claude_model`), endpoint resolution, OpenAI→Claude message/tool conversion, Claude response normalization (tool call format, stop reason mapping), refund internalization, sats_effective invariants, extended thinking/reasoning effort |
| test_runner_lifecycle.rs | 58 | Runner lifecycle types: `LoopState` sub-struct defaults (`ExecutionState`, `BudgetState`, `CommunicationState`, `OnChainState`, `StorageState`, `AuthState`), `ReplyObligation`/`ConversationChainBreak` construction, `ContinuationState` serde round-trip, `content_hash()` SHA-256, `create_loop()`/`create_loop_with_rate_limiter()` factory, `DmLoop.run()` cancellation + max_iterations, field preservation after construction |
| test_context.rs | 56 | System prompt builder (`build_system_prompt` with `PromptContext`), ContextManager (`build_messages`, truncation, `max_history_turns`, offloading, output token reservation), tool pair sanitization, content compaction (base64 stripping, data URI removal, oversized truncation), compaction summary injection, `estimate_message_tokens`, certificate serial rendering |
| test_compact.rs | 41 | Context compaction: 9-section prompt template (`COMPACTION_SECTIONS`, `COMPACTION_PROMPT`), `extract_summary()` (analysis/summary tag handling, multiline, passthrough), `CompactionBoundary` serde round-trip (all fields, empty, large range, timestamp recency), `CompactionCircuitBreaker` (default, failure→fallback, success reset), `dedup_messages()` (identical content, different roles, tool results, first-occurrence, long content hashing), `message_content_hash()` determinism |
| test_context_intelligence.rs | 34 | Intelligent context management: `TokenBreakdown` totals and utilization, `TokenAnalyzer` budget-aware threshold adjustment (normal: 0.8, 65%→0.7, 85%→0.6), compaction trigger conditions, summary header extraction (user facts, tool outcomes, decisions), reinjection building (system prompt + summary + recent turns), `CompactionEvent` serialization, auto-compaction integration with budget pressure |
| test_model_limits.rs | 26 | `model_input_limit()` and `model_output_limit()` for all supported models (gpt-5.x, o-series, Claude), config-vs-model capping logic, input+output=context_window invariants |
| test_self_message.rs | 26 | Self-message loop prevention: `ReplyObligation` lifecycle, `LoopState.comms.pending_replies` (add, dedup, fulfill), done-signal logic (`exec.done`), nudge reset (`comms.nudge_count`), inbox partitioning (self vs external), end-to-end scenario simulations |
| test_tool_offload.rs | 22 | Tool result offloading: JSON array/object smart previews (item count, fields, bounded first-2, summary field preference), no-offload tool list invariants, paid tools list, file creation, `read_tool_output` tool (transcript lookup, session.jsonl fallback, error cases, system category, not always-on, in no-offload list) |
| test_compact_integration.rs | 21 | End-to-end compaction flows: build_messages→microcompact→auto-compact, boundary preservation (discovered_tools, proof_references, last_message_hash), budget-aware threshold adjustment, PreCompact/PostCompact hook events (`HookEvent`, `HookEventType`), circuit breaker fallback flow, dedup+microcompact combined, 9-section summary extraction, token estimation before/after, compaction summary injection via build_messages |
| test_model_capabilities.rs | 19 | `ModelCapabilities` struct: `from_manifest()` (context window arithmetic, saturating_sub), `from_hardcoded()` (all model families), `parse_model_capabilities()` from `ServiceManifest`, discovery-overrides-hardcoded resolution chain |
| test_identity.rs | 18 | Identity/soul system: `IDENTITY_TAG` constant, `MemoryStore::find_identity_entry()` (empty, wrong tag, wrong category, newest-wins, mixed entries), soul section formatting in `build_system_prompt`, bootstrap idempotency, agent self-update |
| test_context_fork.rs | 16 | `ContextFork` isolation: original context preserved after fork mutation, `ForkResult` record/extract/summary, memory injection, `is_memory_intensive_skill()` YAML detection, message cloning and manipulation |
| test_loop_detector.rs | 16 | `LoopDetector`: generic repeat (warning + critical), ping-pong detection (ABAB pattern), circuit breaker (total call limit), budget drain, reset, `with_defaults()`, end-to-end check+record cycles |
| test_intent_nudge.rs | 13 | `signals_tool_intent()`: intent phrase detection (case-insensitive), exclusion phrases ("let me explain", "let me know"), code block filtering, empty/whitespace, past tense rejection, `MAX_INTENT_NUDGES` and `TOOL_INTENT_NUDGE` constants |

## Key imports and types

| Import | From | Used in |
|--------|------|---------|
| `ThinkResult` | `dolphin_milk::think` | test_think — serde, field invariants, tool_calls |
| `is_reasoning_model`, `is_claude_model` | `dolphin_milk::think` | test_think — model family detection |
| `resolve_endpoint` | `dolphin_milk::think` | test_think — provider→URL routing |
| `convert_messages_for_claude`, `convert_tools_for_claude` | `dolphin_milk::think` | test_think — OpenAI↔Claude format |
| `build_openai_body`, `build_claude_body` | `dolphin_milk::think` | test_think — extended thinking/reasoning effort |
| `OPENAI_AGENT_URL`, `CLAUDE_AGENT_URL`, `DEFAULT_MODEL`, `DEFAULT_MAX_TOKENS` | `dolphin_milk::think` | test_think — constant verification |
| `model_input_limit`, `model_output_limit` | `dolphin_milk::think` | test_model_limits, test_model_capabilities |
| `ModelCapabilities` | `dolphin_milk::think` | test_model_capabilities — from_manifest, from_hardcoded |
| `parse_model_capabilities` | `dolphin_milk::x402::discovery` | test_model_capabilities — manifest parsing |
| `build_system_prompt`, `PromptContext`, `ToolDesc`, `CertificateInfo` | `dolphin_milk::context::prompt` | test_context, test_identity |
| `ContextManager`, `compact_content`, `estimate_tokens`, `estimate_message_tokens` | `dolphin_milk::context::manager` | test_context, test_compact_integration, test_context_intelligence |
| `TokenBreakdown`, `TokenAnalyzer`, `CompactionEvent`, `DEFAULT_COMPACTION_THRESHOLD` | `dolphin_milk::context::manager` | test_context_intelligence, test_compact_integration — intelligent compaction |
| `extract_summary`, `message_content_hash`, `CompactionBoundary`, `CompactionCircuitBreaker` | `dolphin_milk::context::compact` | test_compact, test_compact_integration — compaction primitives |
| `COMPACTION_PROMPT`, `COMPACTION_SECTIONS`, `dedup_messages` | `dolphin_milk::context::compact` | test_compact, test_compact_integration — prompt template, dedup |
| `HookEvent`, `HookEventType` | `dolphin_milk::hooks::events` | test_compact_integration — PreCompact/PostCompact events |
| `ContextFork`, `ForkResult`, `is_memory_intensive_skill` | `dolphin_milk::context::fork` | test_context_fork |
| `LoopDetector` | `dolphin_milk::loop_detect::detector` | test_loop_detector |
| `LoopState`, `ReplyObligation` | `dolphin_milk::runner` | test_self_message, test_runner_lifecycle |
| `signals_tool_intent`, `MAX_INTENT_NUDGES`, `TOOL_INTENT_NUDGE` | `dolphin_milk::runner` | test_intent_nudge, test_runner_lifecycle |
| `content_hash`, `create_loop`, `create_loop_with_rate_limiter` | `dolphin_milk::runner` | test_runner_lifecycle — factory functions, SHA-256 hashing |
| `ExecutionState`, `BudgetState`, `CommunicationState`, `OnChainState`, `StorageState`, `AuthState` | `dolphin_milk::runner` | test_runner_lifecycle — LoopState sub-struct defaults |
| `ContinuationState`, `ConversationChainBreak` | `dolphin_milk::runner` | test_runner_lifecycle — serde, construction |
| `CircuitBreakerRegistry` | `dolphin_milk::x402::circuit_breaker` | test_runner_lifecycle — create_loop_with_rate_limiter |
| `MemoryStore`, `MemoryEntry`, `MemoryCategory`, `IDENTITY_TAG` | `dolphin_milk::memory::store` | test_identity |
| `DmConfig` | `dolphin_milk::config` | test_think, test_runner_lifecycle — endpoint resolution, loop factory |
| `all_sandbox_tools` | `dolphin_milk::tools::sandbox` | test_tool_offload — read_tool_output tool |

## Test patterns

### Helper factories

- **`default_ctx()`** (test_context.rs, test_identity.rs, test_context_fork.rs) — Creates a zeroed-out `PromptContext` for prompt builder tests. All fields empty/zero/false. Includes `basket_health: HashMap::new()`.
- **`user_msg()` / `assistant_msg()` / `tool_msg()`** (test_context_intelligence.rs) — Simple message constructors for compaction/analyzer tests.
- **`make_tool_pair()` / `make_conversation()`** (test_compact_integration.rs) — Builds tool call + result message pairs, and full conversations with N user messages and M tool calls.
- **`mock_manifest_with_models()`** (test_model_capabilities.rs) — Returns a `ServiceManifest` with gpt-5-mini and claude-sonnet-4-6 entries.
- **`preview_for_json_array()`** (test_tool_offload.rs) — Mirrors `generate_smart_preview()` logic (pub(crate)) for testing array preview format.
- **`preview_for_json_object()`** (test_tool_offload.rs) — Mirrors `generate_smart_preview()` logic for object previews (summary field detection).
- **`write_mock_transcript()`** (test_tool_offload.rs) — Creates a transcript.jsonl with think_response + tool_result events.

### Test constants

```rust
const OWN_KEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";  // secp256k1 G
const EXTERNAL_KEY: &str = "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
const EXTERNAL_KEY_2: &str = "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
```

All three are valid secp256k1 public keys (generator point G and multiples). Used across test_self_message.rs and test_identity.rs.

### Invariants verified

- **sats_effective <= sats_paid** — ThinkResult cost invariant; anomaly when violated and sats_paid > 0
- **input + output = context_window** — For all known models (gpt-5, o4, Claude Haiku/Sonnet/Opus)
- **No-offload tools** — `file_read`, `memory_search`, `memory_recall`, `read_tool_output`, `execute_bash` must never be offloaded (prevents cascading preview loops)
- **Paid tools list** — Only `x402_call`, `generate_image`, `upload_to_nanostore` should extract sats_paid
- **Claude models always route to Claude endpoint** — Even if provider is explicitly set to "openai-agent"
- **Reasoning models use `max_completion_tokens`** — Not `max_tokens` (o-series, gpt-5.x, gpt-4.1)
- **Tool pair sanitization** — Orphaned tool results and assistant messages with missing tool results are stripped from context
- **Compaction summary only injected when messages are actually trimmed** — Not when history fits
- **Claude stop_reason mapping** — `end_turn`→`stop`, `tool_use`→`tool_calls`, `max_tokens`→`length`, `stop_sequence`→`stop`
- **Empty content handling for Claude** — Empty assistant/user content blocks replaced with non-empty placeholders (Claude API rejects empty blocks)
- **LoopState sub-struct defaults** — All fields zero/empty/None/false after `Default::default()`; verified per sub-struct and as composite
- **content_hash() determinism** — SHA-256 hex digest, 64 chars, known values for empty string and "abc"
- **ContinuationState serde** — `last_proof_hash` uses `skip_serializing_if = "Option::is_none"`; round-trip with and without optional fields
- **DmLoop cancellation** — Pre-set cancel flag → 0 iterations, `done=true`, error contains "Cancelled"; max_iterations=0 → error contains "max iterations"
- **read_tool_output is discoverable** — Not in ALWAYS_ON_TOOLS (accessed via `search_tools`), but is in NO_OFFLOAD_TOOLS (prevents infinite loops), category is "system"
- **Object preview summary preference** — `introspect` tool results use `summary` field when present instead of generic key-listing format
- **COMPACTION_SECTIONS has exactly 9 entries** — Section order in prompt template is monotonically increasing
- **extract_summary removes `<analysis>` but keeps `<summary>` content** — No-tags input passes through unchanged
- **dedup_messages uses first 500 chars** — Long content comparison uses truncated hash, not full content
- **CompactionCircuitBreaker triggers fallback after 3 failures** — Success resets the counter
- **message_content_hash includes role** — Identical content with different roles produces different hashes
- **CompactionBoundary timestamp is recent** — Less than 5 seconds old at creation
- **Budget-aware compaction threshold** — Normal: 0.8, 65% spent→0.7, 85% spent→0.6; high budget usage lowers trigger
- **Reinjection preserves system prompt as first message** — Recent turns count respects history length
- **Microcompact reduces token count but not message count** — Clears tool results, preserves structure
- **PreCompact/PostCompact hook events include timestamp** — Serde round-trip verified

## Running

```bash
# All core tests
cargo test --test test_think --test test_context --test test_self_message \
  --test test_identity --test test_context_fork --test test_loop_detector \
  --test test_intent_nudge --test test_model_capabilities \
  --test test_model_limits --test test_tool_offload --test test_runner_lifecycle \
  --test test_compact --test test_compact_integration --test test_context_intelligence

# By file
cargo test --test test_think
cargo test --test test_compact
cargo test --test test_context_intelligence

# By pattern (matches across all files)
cargo test reasoning_model
cargo test claude_model
cargo test compact_content
cargo test compaction
cargo test dedup
cargo test loop_detector
cargo test nudge
cargo test offload
cargo test runner_lifecycle
cargo test content_hash
cargo test continuation_state
cargo test token_breakdown
cargo test budget_aware
```

## Gotchas

- **`thread::sleep` in test_identity.rs** — 20ms sleeps between `store()` calls ensure filesystem timestamps differ for newest-wins ordering. Don't remove.
- **`pub(crate)` on offload internals** — `generate_smart_preview()` and `TOOL_RESULT_OFFLOAD_THRESHOLD` aren't publicly exported; test_tool_offload.rs mirrors the logic locally.
- **Tokio tests in test_tool_offload.rs and test_runner_lifecycle.rs** — `read_tool_output` tests and `DmLoop.run()` tests are `#[tokio::test]` because they use async closures/methods.
- **LoopState sub-structs** — `LoopState` fields are accessed via `state.comms.pending_replies`, `state.comms.nudge_count`, and `state.exec.done` (not flat top-level fields).
- **`tempfile` in test_runner_lifecycle.rs and test_compact_integration.rs** — `create_loop()`, `DmLoop.run()`, and `ContextManager` tests create `tempfile::tempdir()` for workspace isolation. The `TempDir` must stay alive for the test duration.
- **`basket_health` in PromptContext** — `default_ctx()` helpers must include `basket_health: std::collections::HashMap::new()`. Added for on-chain basket health monitoring in the system prompt.
- **Compaction boundary timestamp assertions** — test_compact.rs checks that boundary timestamps are less than 5 seconds old. If CI is slow, these could flake (but 5s is generous).
- **Circuit breaker failure count** — `CompactionCircuitBreaker` triggers fallback after exactly 3 failures. Tests rely on this threshold; don't change without updating tests.

## Related

- [../CLAUDE.md](../CLAUDE.md) — Parent test directory overview and conventions
- [../../src/think.rs](../../src/think.rs) — ThinkResult, model detection, provider routing, body builders
- [../../src/runner/](../../src/runner/) — DmLoop, LoopState sub-structs, create_loop, content_hash, ContinuationState
- [../../src/context/CLAUDE.md](../../src/context/CLAUDE.md) — Context pipeline: prompt builder, manager, fork, compact
- [../../src/runner/CLAUDE.md](../../src/runner/CLAUDE.md) — Agent loop, LoopState, self-message prevention, intent nudge
- [../../src/memory/CLAUDE.md](../../src/memory/CLAUDE.md) — MemoryStore, identity entries, IDENTITY_TAG
- [../../src/config/CLAUDE.md](../../src/config/CLAUDE.md) — DmConfig
- [../../src/hooks/CLAUDE.md](../../src/hooks/CLAUDE.md) — Hook events (PreCompact/PostCompact used in compaction integration tests)
