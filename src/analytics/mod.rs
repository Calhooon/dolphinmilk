//! Efficiency analytics — pure computation from transcript data.
//!
//! Scans `workspace/tasks/*/session.jsonl` transcripts to compute:
//! - Cost per task, cost per token, tokens per iteration
//! - Daily time series (last 30 days)
//! - Model comparison breakdown
//! - Period-over-period comparison (this week vs last week)
//! - Cost replay / model comparison (#62)
//! - ROI reporting (#51)
//! - Provider benchmarks (#64)

mod benchmarks;
mod cost;
mod efficiency;
mod roi;

use std::path::Path;

use serde::Serialize;

// ── Re-exports ──────────────────────────────────────────────────────────

pub use benchmarks::{
    aggregate_benchmarks, backfill_benchmarks_from_transcripts, infer_provider, load_benchmarks,
    load_benchmarks_from_path, record_benchmark, BenchmarkEntry, ProviderBenchmark,
};
pub use cost::{cost_replay, cost_replay_from_events, CostComparison, ModelPricing};
pub use efficiency::{
    compute_efficiency, compute_efficiency_at, DayMetrics, EfficiencyReport, ModelMetrics,
    PeriodComparison, PeriodSummary,
};
pub use roi::{compute_roi, compute_roi_from_summaries, RoiReport};

// ── Combined cost analysis (for the agent tool) ──────────────────────

/// Combined cost analysis result returned by the cost_analysis tool.
#[derive(Debug, Clone, Serialize)]
pub struct CostAnalysis {
    pub efficiency: EfficiencyReport,
    pub roi: RoiReport,
    pub benchmarks: Vec<ProviderBenchmark>,
}

/// Compute a combined cost analysis from workspace data.
pub fn compute_cost_analysis(workspace: &Path) -> CostAnalysis {
    let efficiency = compute_efficiency(workspace);
    let roi = compute_roi(workspace);

    // Load or backfill benchmarks
    let mut benchmark_entries = load_benchmarks(workspace);
    if benchmark_entries.is_empty() {
        // Backfill from transcripts on first call
        benchmark_entries = backfill_benchmarks_from_transcripts(workspace);
        // Persist backfilled entries
        for entry in &benchmark_entries {
            record_benchmark(workspace, entry);
        }
    }
    let benchmarks = aggregate_benchmarks(&benchmark_entries);

    CostAnalysis {
        efficiency,
        roi,
        benchmarks,
    }
}
