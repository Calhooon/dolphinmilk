# tests/features

> 324 tests across 13 files covering cross-cutting features: analytics, moderation, rate limiting, replay, tagging, escalation, live integration, and more.

## Overview

The `features/` test group validates higher-level product features that span multiple source modules. Unlike the domain-specific test directories (core/, onchain/, etc.), these tests exercise feature-complete behaviors: cost analytics pipelines, cert-driven policy engines, task replay infrastructure, live wallet integration, and marketplace APIs. Many tests use axum oneshot routing to validate server endpoints without a TCP listener.

## Files

| File | Tests | Description |
|------|------:|-------------|
| test_analytics.rs | 47 | Cost analytics: efficiency trends, per-model breakdown, period comparison, cost replay, ROI reporting, provider benchmarks, server routes (`/analytics/*`), `cost_analysis` tool, scenario JSON validation |
| test_moderation.rs | 40 | Cert-driven content moderation: PII detection (SSN, credit card, email), keyword blocking, custom regex patterns, block/flag mode, cert→config→default precedence, wiring integration (user messages, tool I/O, LLM responses) |
| test_tagging.rs | 29 | Task tagging: `TaskInfo`/`TaskSummary` serde with tags, `normalize_tags()` (trim, lowercase, dedup, filter empty), `CertTagPolicy`, `TaskRequest`/`ChatRequest` deserialization, AND-semantics filtering, backward compatibility |
| test_time_estimate.rs | 28 | Time-saved estimation: `parse_time_saved_tag()` (`Xh`/`Xm`/decimal), `default_estimate()` heuristic (5 min/iteration + 2 min/tool call), 480-minute cap, `aggregate_time_saved()` (total/week/month), `TaskSummary.time_saved_minutes` |
| test_live_integration.rs | 29 | End-to-end live tests against real wallets and services: identity key, BRC-42 derivation, ECDSA signatures, HMAC, Authrite handshake, MessageBox (quote/send/list/ack/poll), BRC-18 proofs, BRC-48 state tokens, x402 LLM calls, cross-wallet messaging, conversations, image generation, registry discovery, concurrent spending serialization |
| test_escalation.rs | 26 | Auto-escalation detection: tool loop (3 identical calls), budget threshold (80%), error count (>3), explicit uncertainty phrases, `mark_triggered()`/`reset()`, event building, serde roundtrips for `EscalationReason`/`EscalationEvent`/`EscalationResolution`/`EscalationStatus` |
| test_rate_limit.rs | 26 | Token bucket rate limiter: bucket fill/drain/refill, blocking when empty, concurrent access safety, per-service isolation, cert overrides config, disabled/zero RPM = unlimited, timing verification, `base_url()` extraction, `DmLoop` integration with circuit breaker, tool factory wiring |
| test_replay.rs | 24 | Task replay and fork: `ReplayViewer` (timeline, iteration tracking, cumulative cost, cost timeline, tool usage, seeking, range queries), `ForkExecutor` (message reconstruction, iteration/cost at fork point, custom message/model), server routes (`/task/{id}/replay`, `/task/{id}/fork`) |
| test_tool_approval.rs | 20 | Per-tool approval workflows: `ToolApprovalConfig` (TOML parsing, env overrides, defaults), `CertToolApproval`, cert+config merge, file-based approval/abort mechanism, timeout logic, `StepEvent::ApprovalRequired` serde, staged ref naming, env var isolation (#224) |
| test_cold_read.rs | 16 | Cold-read prediction: extract topic keywords from memory YAML frontmatter + body, extract topics from transcripts via `Trajectory`, compute relevance (direct + substring match), cross-topic low relevance, aggregate prediction, `EvalReport` generation and regression detection |
| test_metrics.rs | 15 | Prometheus metrics: `MetricsRegistry` creation, counters (`dm_tokens_total`, `dm_tasks_total`, `dm_tool_calls_total`, `dm_errors_total`, `dm_budget_spent_sats`), histogram (`dm_request_latency_seconds`), gauge (`dm_active_tasks`), Prometheus text format, label correctness, `/metrics` endpoint |
| test_marketplace.rs | 13 | Plugin marketplace API: CRUD (`/marketplace/plugins`), list (alphabetical sort), get/create/update/delete, 404 handling, JSON disk persistence, validation (empty name, invalid chars, bad JSON), `PluginListing` serde and defaults |
| test_verification.rs | 11 | Verification skill and `verify_output` tool: SKILL.md parsing, tool registration and parameters, output analysis (JSON validity, error detection, empty output, missing params, search-specific checks, verification note) |

## Test tiers for test_live_integration.rs

Live tests are organized into tiers with increasing cost and service requirements:

| Tier | Requirements | Tests | Examples |
|------|-------------|------:|---------|
| 1 (Free) | Wallet on port 3322 | 9 | Identity key, BRC-42 derivation, signatures, HMAC |
| 2 (Free, network) | Wallet + MessageBox | 5 | Authrite handshake, authenticated POST, session reuse, MessageBox quote/list |
| 3 (Paid) | Wallet + `RUN_PAID_TESTS=1` | 9 | BRC-18 proofs, BRC-48 state tokens, x402 LLM calls, image gen, conversations, concurrent spending |
| 4 (Cross-wallet) | Ports 3321+3322 | 3 | Cross-wallet send/receive, worm replies, nudge-forces-send |
| 5 (Discovery) | Network | 3 | Registry list/resolve, manifest fetch |

All live tests auto-skip via `check_wallet()` / `check_messagebox()` guards — no `#[ignore]` needed.

## Key test patterns

### Cert→Config→Default precedence

Multiple feature tests validate the three-layer policy precedence chain where certificate fields override config, which overrides defaults:

- **Moderation**: `CertModerationPolicy` overrides `ModerationConfig` (enabled, pii_mode, profanity_mode)
- **Rate limiting**: `CertRateLimits` overrides `RateLimitConfig` (enabled, default_rpm)
- **Tagging**: `CertTagPolicy` enforces `tags_required`
- **Tool approval**: `CertToolApproval` merges with `ToolApprovalConfig.require`

### Axum oneshot testing

Tests for marketplace, replay, metrics, and analytics endpoints use `tower::ServiceExt::oneshot()` with leaked `TempDir`s:

```rust
async fn test_router_with_workspace() -> (axum::Router, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir); // leaked — router holds Arc<PathBuf>
    let router = build_router(DmConfig::default(), path.clone()).await;
    (router, path)
}
```

### Synthetic transcript generation

`test_analytics.rs` uses a `write_transcript()` helper that creates JSONL files with configurable iterations (model, sats, tokens) and timestamps for deterministic analytics testing.

### LoopState decomposition

`test_live_integration.rs` accesses `DmLoop` state via decomposed sub-structs: `worm.state.exec.*` (iteration, done, error, result), `worm.state.budget.*` (sats_spent), `worm.state.comms.*` (nudge_count). The cancel token is `Arc<AtomicBool>`.

### Token bucket timing

`test_rate_limit.rs` verifies real-time rate limiting behavior with `tokio::time::timeout` and `Instant` measurements to prove that buckets block when empty and refill at the expected rate.

### Env var isolation

`test_tool_approval.rs` uses `save_and_clear_approval_env()` / `restore_approval_env()` to prevent `DOLPHIN_MILK_APPROVAL_*` env vars from leaking between parallel tests (#224). Config parsing tests that don't need env override behavior use `toml::from_str` directly instead of `load_config`.

## Key helpers

| Helper | File | Purpose |
|--------|------|---------|
| `wallet_3322()` | test_live_integration.rs | Factory for worm wallet on port 3322 |
| `wallet_3321()` | test_live_integration.rs | Factory for MetaNet Client wallet on port 3321 |
| `check_wallet()` | test_live_integration.rs | Async guard — returns false if wallet unreachable |
| `check_messagebox()` | test_live_integration.rs | Async guard — returns false if MessageBox unreachable |
| `paid_tests_enabled()` | test_live_integration.rs | Checks `RUN_PAID_TESTS` env var |
| `write_transcript()` | test_analytics.rs | Creates synthetic JSONL transcripts with configurable iterations |
| `date_ts()` | test_analytics.rs | Unix timestamp factory for a given date |
| `test_now()` | test_analytics.rs | Reference timestamp (2026-03-22 noon UTC) |
| `engine_from_config()` | test_moderation.rs | Builds `ModerationEngine` from config only (no cert) |
| `enabled_pii_block_config()` | test_moderation.rs | `ModerationConfig` with PII blocking enabled |
| `enabled_pii_flag_config()` | test_moderation.rs | `ModerationConfig` with PII flagging enabled |
| `config_enabled()` | test_rate_limit.rs | `RateLimitConfig` factory with given RPM |
| `config_disabled()` | test_rate_limit.rs | `RateLimitConfig` factory with `enabled=false` |
| `config_with_service()` | test_rate_limit.rs | `RateLimitConfig` with per-service override |
| `make_summary()` | test_tagging.rs | `TaskSummary` factory with given tags |
| `filter_by_tags()` | test_tagging.rs | AND-semantics tag filter over `TaskSummary` list |
| `create_test_transcript()` | test_replay.rs | Creates a 10-event transcript with tool calls and budget checks |
| `create_minimal_transcript()` | test_replay.rs | Creates a 2-event transcript (user + system) |
| `fixtures_dir()` | test_cold_read.rs | `PathBuf` to `tests/fixtures/eval` |
| `save_and_clear_approval_env()` | test_tool_approval.rs | Removes `DOLPHIN_MILK_APPROVAL_*` env vars, returns prior values for restore |
| `restore_approval_env()` | test_tool_approval.rs | Restores previously saved env vars (prevents parallel test contamination) |
| `test_router_with_workspace()` | test_marketplace.rs, test_replay.rs | Returns `(Router, PathBuf)` for workspace inspection |

## Key imports by feature

| Feature | Source modules |
|---------|---------------|
| Analytics | `dolphin_milk::analytics` (`compute_efficiency_at`, `cost_replay`, `compute_roi`, `record_benchmark`, `aggregate_benchmarks`, etc.) |
| Moderation | `dolphin_milk::moderation` (`ModerationEngine`, `ModerationResult`, `Mode`), `dolphin_milk::certificates::CertModerationPolicy` |
| Rate limiting | `dolphin_milk::x402::rate_limit` (`RateLimiterRegistry`, `RateLimitConfig`, `CertRateLimits`, `base_url`), `dolphin_milk::x402::circuit_breaker::CircuitBreakerRegistry` |
| Replay | `dolphin_milk::replay` (`ReplayViewer`, `ForkExecutor`, `ForkParams`), `dolphin_milk::transcript::Transcript` |
| Tagging | `dolphin_milk::server::types` (`TaskInfo`, `TaskSummary`, `TaskRequest`, `ChatRequest`, `normalize_tags`), `dolphin_milk::certificates::CertTagPolicy` |
| Time estimate | `dolphin_milk::time_estimate` (`parse_time_saved_tag`, `default_estimate`, `estimate_time_saved`, `aggregate_time_saved`, `TimeSavedAnalytics`, `TaskTimeSaved`) |
| Escalation | `dolphin_milk::runner::escalation` (`EscalationDetector`, `EscalationEvent`, `EscalationReason`, `EscalationResolution`, `EscalationStatus`) |
| Tool approval | `dolphin_milk::config` (`ToolApprovalConfig`, `load_config`), `dolphin_milk::certificates::CertToolApproval`, `dolphin_milk::events::StepEvent` |
| Cold read | `dolphin_milk::eval::report` (`EvalReport`, `Baseline`), `dolphin_milk::eval::trajectory::Trajectory` |
| Metrics | `dolphin_milk::metrics::MetricsRegistry` |
| Marketplace | `dolphin_milk::server` (`PluginListing`, `PluginListResponse`) |
| Verification | `dolphin_milk::skills::loader::parse_skill_content`, `dolphin_milk::tools::verification_tools::all_verification_tools` |
| Live integration | `dolphin_milk::wallet::WalletClient`, `dolphin_milk::auth::AuthriteClient`, `dolphin_milk::messagebox::MessageBoxClient`, `dolphin_milk::runner` |

## Running tests

```bash
# All features tests
cargo test --test test_analytics --test test_moderation --test test_rate_limit  # etc.

# By pattern match (matches function names across all feature files)
cargo test moderation
cargo test replay
cargo test escalation
cargo test rate_limit
cargo test cold_read
cargo test marketplace
cargo test time_estimate

# Live integration (needs wallet on localhost:3322)
cargo test -- test_live
RUN_PAID_TESTS=1 cargo test -- test_live           # Include paid tests
cargo test -- --test-threads=1 test_live            # Serial for cross-wallet tests
```

## Related

- [Parent tests/CLAUDE.md](../CLAUDE.md) — Full test directory layout, conventions, all helpers
- [integration/CLAUDE.md](../integration/CLAUDE.md) — Playwright E2E scenarios (JS, not Rust)
- [src/analytics.rs](../../src/analytics.rs) — Cost analytics module
- [src/moderation.rs](../../src/moderation.rs) — Content moderation engine
- [src/x402/rate_limit.rs](../../src/x402/) — Token bucket rate limiter
- [src/replay/](../../src/replay/) — Replay and fork infrastructure
- [src/runner/escalation.rs](../../src/runner/) — Auto-escalation detection
- [src/time_estimate.rs](../../src/time_estimate.rs) — Time-saved estimation
