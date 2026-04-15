//! Provider benchmarks (#64).
//!
//! Persists and aggregates per-provider/model performance observations.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::transcript::Transcript;

// ── Output structs ──────────────────────────────────────────────────────

/// Aggregated benchmark for a provider+model pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderBenchmark {
    pub provider: String,
    pub model: String,
    pub avg_latency_ms: f64,
    pub avg_cost_sats: f64,
    pub success_rate: f64,
    pub sample_count: u32,
    pub total_tokens: u64,
}

/// A single benchmark observation, persisted to JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkEntry {
    pub timestamp: String,
    pub provider: String,
    pub model: String,
    pub latency_ms: u64,
    pub cost_sats: u64,
    pub tokens: u64,
    pub success: bool,
}

// ── Persistence ─────────────────────────────────────────────────────────

/// Append a benchmark entry to the JSONL file.
pub fn record_benchmark(workspace: &Path, entry: &BenchmarkEntry) {
    let analytics_dir = workspace.join("analytics");
    let _ = fs::create_dir_all(&analytics_dir);
    let path = analytics_dir.join("benchmarks.jsonl");

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        if let Ok(json_str) = serde_json::to_string(entry) {
            let _ = writeln!(f, "{json_str}");
        }
    }
}

/// Infer provider name from model string.
pub fn infer_provider(model: &str) -> String {
    if model.starts_with("claude-") {
        "claude-chat".to_string()
    } else if model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        "openai-agent".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Load all benchmark entries from the JSONL file.
pub fn load_benchmarks(workspace: &Path) -> Vec<BenchmarkEntry> {
    let path = workspace.join("analytics").join("benchmarks.jsonl");
    load_benchmarks_from_path(&path)
}

/// Load benchmark entries from a specific path.
pub fn load_benchmarks_from_path(path: &Path) -> Vec<BenchmarkEntry> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<BenchmarkEntry>(trimmed) {
            entries.push(entry);
        }
    }

    entries
}

// ── Aggregation ─────────────────────────────────────────────────────────

/// Aggregate benchmark entries into per-provider summaries.
pub fn aggregate_benchmarks(entries: &[BenchmarkEntry]) -> Vec<ProviderBenchmark> {
    let mut groups: HashMap<(String, String), Vec<&BenchmarkEntry>> = HashMap::new();

    for entry in entries {
        groups
            .entry((entry.provider.clone(), entry.model.clone()))
            .or_default()
            .push(entry);
    }

    let mut benchmarks: Vec<ProviderBenchmark> = groups
        .into_iter()
        .map(|((provider, model), entries)| {
            let count = entries.len() as u32;
            let total_latency: u64 = entries.iter().map(|e| e.latency_ms).sum();
            let total_cost: u64 = entries.iter().map(|e| e.cost_sats).sum();
            let total_tokens: u64 = entries.iter().map(|e| e.tokens).sum();
            let successes = entries.iter().filter(|e| e.success).count() as f64;

            ProviderBenchmark {
                provider,
                model,
                avg_latency_ms: if count > 0 {
                    total_latency as f64 / count as f64
                } else {
                    0.0
                },
                avg_cost_sats: if count > 0 {
                    total_cost as f64 / count as f64
                } else {
                    0.0
                },
                success_rate: if count > 0 {
                    successes / count as f64
                } else {
                    0.0
                },
                sample_count: count,
                total_tokens,
            }
        })
        .collect();

    benchmarks.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.model.cmp(&b.model)));
    benchmarks
}

// ── Backfill ────────────────────────────────────────────────────────────

/// Scan all task transcripts and build benchmark entries from think_response events.
///
/// This is a one-time backfill that can populate the benchmark JSONL from historical data.
pub fn backfill_benchmarks_from_transcripts(workspace: &Path) -> Vec<BenchmarkEntry> {
    let tasks_dir = workspace.join("tasks");
    let mut entries = Vec::new();

    let dir_entries = match fs::read_dir(&tasks_dir) {
        Ok(e) => e,
        Err(_) => return entries,
    };

    for entry in dir_entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let transcript_path = path.join("session.jsonl");
        if !transcript_path.exists() {
            continue;
        }

        let transcript = Transcript::new(transcript_path);
        let events = transcript.replay();

        for event in events {
            if event.event_type != "think_response" {
                continue;
            }

            let model = event
                .data
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let sats = event
                .data
                .get("sats_effective")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let prompt_tokens = event
                .data
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let completion_tokens = event
                .data
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let duration_ms = event
                .data
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let finish_reason = event
                .data
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let provider = infer_provider(&model);
            let success = finish_reason != "error";
            let ts_secs = event.ts as i64;
            let timestamp = DateTime::from_timestamp(ts_secs, 0)
                .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap())
                .to_rfc3339();

            entries.push(BenchmarkEntry {
                timestamp,
                provider,
                model,
                latency_ms: duration_ms,
                cost_sats: sats,
                tokens: prompt_tokens + completion_tokens,
                success,
            });
        }
    }

    entries
}
