# src/analytics/
> Cost analytics, efficiency metrics, ROI reporting, and provider benchmarks — pure computation from transcript data.

## Overview

Stateless analytics module that scans `workspace/tasks/*/session.jsonl` transcripts to produce cost, efficiency, and ROI reports. All functions are pure — they read transcripts but never modify them. No shared state, no side effects. The module powers 4 HTTP endpoints (`/analytics/efficiency`, `/analytics/cost-comparison`, `/analytics/roi`, `/analytics/benchmarks`) and the `cost_analysis` agent tool.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 66 | Re-exports all public types and functions. Defines `CostAnalysis` combined report struct and `compute_cost_analysis()` entry point that orchestrates all three sub-modules. Lazy benchmark backfill: if `benchmarks.jsonl` is empty, backfills from transcripts on first call. |
| `efficiency.rs` | 456 | Core efficiency metrics. `compute_efficiency()` scans all task transcripts to produce `EfficiencyReport`: cost per task/token, tokens per iteration, 30-day daily trend (including zero-activity days), per-model breakdown, week-over-week comparison. `compute_efficiency_at()` accepts a reference time for deterministic testing. Includes 3 inline unit tests. |
| `cost.rs` | 185 | Model cost comparison / "what-if" replay. `cost_replay()` re-scores a single task transcript with alternative model pricing (sats per 1K tokens). `cost_replay_from_events()` accepts raw events for testing without filesystem. Computes savings in sats and percentage. Falls back to derived rate if the alternative model isn't in the pricing table. |
| `roi.rs` | 226 | Return-on-investment reporting. `compute_roi()` correlates spending with task outcomes (completed vs failed). Reads `eval.json` siblings for quality scores when available (`aggregate_score` or `score` field). `compute_roi_from_summaries()` accepts tuples for testing. `efficiency_score` = tasks completed per million sats. |
| `benchmarks.rs` | 242 | Provider performance tracking. `BenchmarkEntry` observations persisted to `workspace/analytics/benchmarks.jsonl`. `aggregate_benchmarks()` groups by (provider, model) and computes avg latency, avg cost, success rate. `backfill_benchmarks_from_transcripts()` populates from historical `think_response` events. `infer_provider()` maps model names to providers (`claude-*` → `claude-chat`, `gpt-*/o1/o3/o4` → `openai-agent`). |

## Key Exports

### Combined Entry Point

- **`compute_cost_analysis(workspace) -> CostAnalysis`** — Orchestrates all three sub-modules into a single `CostAnalysis { efficiency, roi, benchmarks }` result. Used by the `cost_analysis` agent tool.

### Efficiency (`efficiency.rs`)

- **`compute_efficiency(workspace) -> EfficiencyReport`** — Scans all task transcripts. Returns global averages (cost_per_task_sats, cost_per_token_sats, tokens_per_iteration), 30-day daily_trend, per-model by_model breakdown, and week-over-week period_comparison.
- **`compute_efficiency_at(workspace, now) -> EfficiencyReport`** — Same as above but with injectable reference time for testing.
- **`EfficiencyReport`** — Top-level report struct with `daily_trend: Vec<DayMetrics>`, `by_model: HashMap<String, ModelMetrics>`, `period_comparison: PeriodComparison`.
- **`DayMetrics`** — Per-day aggregation: date (ISO 8601), tasks, total_sats, total_tokens, avg_cost_per_task.
- **`ModelMetrics`** — Per-model aggregation: tasks, total_sats, total_tokens, avg_cost_per_task, avg_tokens_per_task.
- **`PeriodComparison`** — This week vs last week with `cost_change_pct` and `efficiency_change_pct`.
- **`PeriodSummary`** — Single period stats: label (date range string), tasks, total_sats, total_tokens, avg_cost_per_task.

### Cost Replay (`cost.rs`)

- **`cost_replay(workspace, task_id, alt_model, pricing) -> Option<CostComparison>`** — Re-scores a task transcript with alternative model pricing. Returns `None` if transcript is missing or has no think_response events.
- **`cost_replay_from_events(task_id, events, alt_model, pricing) -> Option<CostComparison>`** — Same logic but from raw `TranscriptEvent` slice (no filesystem).
- **`CostComparison`** — Result struct: task_id, original_model, original_cost, original_tokens, alternative_model, alternative_cost, savings_sats (signed), savings_pct.
- **`ModelPricing`** — Pricing entry: model name + cost_per_1k_tokens in satoshis.

### ROI (`roi.rs`)

- **`compute_roi(workspace) -> RoiReport`** — Scans all task transcripts. Determines completion from `session_end` event (error field absent = completed). Reads sibling `eval.json` for quality scores.
- **`compute_roi_from_summaries(summaries) -> RoiReport`** — From pre-built tuples `(sats, tokens, completed, quality_score)` for testing.
- **`RoiReport`** — Spending vs outcomes: total_spent_sats, tasks_completed/failed, avg_cost_per_task, cost_per_completed_task, cost_per_failed_task, efficiency_score (tasks per M sats), total_tokens, avg_quality_score.

### Benchmarks (`benchmarks.rs`)

- **`record_benchmark(workspace, entry)`** — Appends a `BenchmarkEntry` to `workspace/analytics/benchmarks.jsonl`.
- **`load_benchmarks(workspace) -> Vec<BenchmarkEntry>`** — Reads all entries from the JSONL file.
- **`load_benchmarks_from_path(path) -> Vec<BenchmarkEntry>`** — Reads from a specific path.
- **`aggregate_benchmarks(entries) -> Vec<ProviderBenchmark>`** — Groups by (provider, model), computes avg_latency_ms, avg_cost_sats, success_rate, sample_count, total_tokens. Sorted by provider then model.
- **`backfill_benchmarks_from_transcripts(workspace) -> Vec<BenchmarkEntry>`** — One-time backfill from historical `think_response` events across all task transcripts.
- **`infer_provider(model) -> String`** — Model name to provider mapping.
- **`BenchmarkEntry`** — Single observation: timestamp, provider, model, latency_ms, cost_sats, tokens, success.
- **`ProviderBenchmark`** — Aggregated stats per provider+model pair.

## Data Flow

All analytics derive from the same source: **`workspace/tasks/*/session.jsonl`** transcript files produced by `session/transcript.rs`.

```
workspace/tasks/*/session.jsonl   ──┬──▶ efficiency.rs  ──▶ EfficiencyReport
  (think_response events)           ├──▶ cost.rs        ──▶ CostComparison
  (session_start/end events)        ├──▶ roi.rs         ──▶ RoiReport
                                    └──▶ benchmarks.rs  ──▶ Vec<ProviderBenchmark>
                                                              │
workspace/analytics/benchmarks.jsonl ◀──── persisted ─────────┘
workspace/tasks/*/eval.json ──▶ roi.rs (optional quality scores)
```

Key transcript event fields consumed:
- **`think_response`**: `model`, `sats_effective`, `prompt_tokens`, `completion_tokens`, `duration_ms`, `finish_reason`
- **`session_start`**: timestamp (used for task date)
- **`session_end`**: `error` field (presence = failed task)
- **`tool_call_complete`**: `sats_paid` (tool spending, via `Transcript::total_sats_spent()`)

## Usage

```rust
use std::path::Path;
use bsv_worm::analytics::{compute_cost_analysis, compute_efficiency, cost_replay, compute_roi};

let workspace = Path::new("workspace");

// Combined analysis (used by cost_analysis tool)
let analysis = compute_cost_analysis(workspace);
println!("Cost per task: {} sats", analysis.efficiency.cost_per_task_sats);
println!("Tasks completed: {}", analysis.roi.tasks_completed);

// Individual reports
let efficiency = compute_efficiency(workspace);
let roi = compute_roi(workspace);

// What-if model comparison
let mut pricing = std::collections::HashMap::new();
pricing.insert("gpt-5-mini".to_string(), 0.5);
pricing.insert("claude-haiku-4-5-20251001".to_string(), 0.3);
if let Some(comparison) = cost_replay(workspace, "task-123", "claude-haiku-4-5-20251001", &pricing) {
    println!("Savings: {} sats ({}%)", comparison.savings_sats, comparison.savings_pct);
}
```

## API Endpoints

Served by `server/handlers/analytics.rs`:

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/analytics/efficiency` | `handle_efficiency` | Full `EfficiencyReport` |
| GET | `/analytics/cost-comparison?task_id=X&model=Y` | `handle_cost_comparison` | `CostComparison` with 9-model pricing table + prefix-match for versioned names |
| GET | `/analytics/roi` | `handle_roi` | `RoiReport` |
| GET | `/analytics/benchmarks` | `handle_benchmarks` | `Vec<ProviderBenchmark>` |

## Design Decisions

- **Pure functions over stateful services**: All analytics functions take a `&Path` workspace and return computed results. No caching, no shared state. Simple to test and reason about.
- **Lazy benchmark backfill**: `compute_cost_analysis()` populates `benchmarks.jsonl` from transcripts on first call if the file is empty. Subsequent calls read from the persisted JSONL.
- **Zero-activity days included**: `build_daily_trend()` generates entries for all 30 days including days with no tasks, so the daily_trend array is always exactly 30 elements.
- **Completion is error-absence**: ROI determines task success from the `session_end` event — no error field means completed. Missing `session_end` means incomplete/failed.
- **Cost replay fallback rate**: If the alternative model isn't in the pricing table, `cost_replay` derives a rate from the original task's cost/token ratio. This avoids returning `None` for unknown models.
- **Efficiency measured as tokens per sat**: Higher = more efficient. The period_comparison reports percentage change in this metric.

## Related

- [../session/CLAUDE.md](../session/CLAUDE.md) — `Transcript` struct and `TranscriptEvent` type consumed by all analytics
- [../server/CLAUDE.md](../server/CLAUDE.md) — HTTP endpoints that serve analytics results
- [../../src/time_estimate.rs](../time_estimate.rs) — Human-equivalent time-saved estimation (separate module, related ROI concept)
- [../eval/CLAUDE.md](../eval/CLAUDE.md) — Evaluation framework; `eval.json` quality scores consumed by `roi.rs`
- [../CLAUDE.md](../CLAUDE.md) — Parent module documentation
