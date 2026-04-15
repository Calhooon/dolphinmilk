//! Cost replay / model comparison (#62).
//!
//! Re-scores task transcripts with alternative model pricing to answer
//! "what would this task have cost on a different model?"

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::transcript::{Transcript, TranscriptEvent};

// ── Output structs ──────────────────────────────────────────────────────

/// Pricing entry for a model: cost per 1K tokens in satoshis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model: String,
    pub cost_per_1k_tokens: f64,
}

/// Result of re-scoring a task transcript with alternative model pricing.
#[derive(Debug, Clone, Serialize)]
pub struct CostComparison {
    pub task_id: String,
    pub original_model: String,
    pub original_cost: u64,
    pub original_tokens: u64,
    pub alternative_model: String,
    pub alternative_cost: u64,
    pub savings_sats: i64,
    pub savings_pct: f64,
}

// ── Internal intermediate data ──────────────────────────────────────────

/// Per-iteration cost detail used by cost replay.
#[derive(Debug, Clone)]
struct IterationCost {
    model: String,
    sats_effective: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
}

/// Extract per-iteration cost data from a transcript.
fn extract_iteration_costs(events: &[TranscriptEvent]) -> Vec<IterationCost> {
    events
        .iter()
        .filter(|e| e.event_type == "think_response")
        .map(|e| IterationCost {
            model: e
                .data
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            sats_effective: e
                .data
                .get("sats_effective")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            prompt_tokens: e
                .data
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            completion_tokens: e
                .data
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        })
        .collect()
}

// ── Core computation ────────────────────────────────────────────────────

/// Re-score a task transcript with alternative model pricing.
///
/// Given a pricing table (model → cost per 1K tokens), recomputes what the task
/// would have cost on a different model.
pub fn cost_replay(
    workspace: &Path,
    task_id: &str,
    alt_model: &str,
    pricing: &HashMap<String, f64>,
) -> Option<CostComparison> {
    let transcript_path = workspace.join("tasks").join(task_id).join("session.jsonl");
    if !transcript_path.exists() {
        return None;
    }

    let transcript = Transcript::new(transcript_path);
    let events = transcript.replay();
    let iterations = extract_iteration_costs(events);

    if iterations.is_empty() {
        return None;
    }

    let original_model = iterations[0].model.clone();
    let original_cost: u64 = iterations.iter().map(|i| i.sats_effective).sum();
    let total_tokens: u64 = iterations
        .iter()
        .map(|i| i.prompt_tokens + i.completion_tokens)
        .sum();

    // Compute alternative cost using the pricing table
    let alt_rate = pricing.get(alt_model).copied().unwrap_or_else(|| {
        // Fall back: derive from original cost if we have token data
        if total_tokens > 0 {
            (original_cost as f64 / total_tokens as f64) * 1000.0
        } else {
            0.0
        }
    });

    let alternative_cost = ((total_tokens as f64 / 1000.0) * alt_rate) as u64;
    let savings_sats = original_cost as i64 - alternative_cost as i64;
    let savings_pct = if original_cost > 0 {
        (savings_sats as f64 / original_cost as f64) * 100.0
    } else {
        0.0
    };

    Some(CostComparison {
        task_id: task_id.to_string(),
        original_model,
        original_cost,
        original_tokens: total_tokens,
        alternative_model: alt_model.to_string(),
        alternative_cost,
        savings_sats,
        savings_pct,
    })
}

/// Re-score a task transcript from raw events (for testing without filesystem).
pub fn cost_replay_from_events(
    task_id: &str,
    events: &[TranscriptEvent],
    alt_model: &str,
    pricing: &HashMap<String, f64>,
) -> Option<CostComparison> {
    let iterations = extract_iteration_costs(events);

    if iterations.is_empty() {
        return None;
    }

    let original_model = iterations[0].model.clone();
    let original_cost: u64 = iterations.iter().map(|i| i.sats_effective).sum();
    let total_tokens: u64 = iterations
        .iter()
        .map(|i| i.prompt_tokens + i.completion_tokens)
        .sum();

    let alt_rate = pricing.get(alt_model).copied().unwrap_or_else(|| {
        if total_tokens > 0 {
            (original_cost as f64 / total_tokens as f64) * 1000.0
        } else {
            0.0
        }
    });

    let alternative_cost = ((total_tokens as f64 / 1000.0) * alt_rate) as u64;
    let savings_sats = original_cost as i64 - alternative_cost as i64;
    let savings_pct = if original_cost > 0 {
        (savings_sats as f64 / original_cost as f64) * 100.0
    } else {
        0.0
    };

    Some(CostComparison {
        task_id: task_id.to_string(),
        original_model,
        original_cost,
        original_tokens: total_tokens,
        alternative_model: alt_model.to_string(),
        alternative_cost,
        savings_sats,
        savings_pct,
    })
}
