//! Analytics route handlers — efficiency trends, cost comparison, ROI, benchmarks.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};

use serde::Deserialize;

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

/// GET /analytics/efficiency — efficiency metrics computed from transcript data.
pub(crate) async fn get_efficiency(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/analytics/efficiency", None, &headers, None).await?;

    let report = crate::analytics::compute_efficiency(&state.workspace);

    signed_json_response(&state, auth_ctx, StatusCode::OK, &report).await
}

/// Query params for GET /analytics/cost-comparison.
#[derive(Deserialize)]
pub(crate) struct CostComparisonParams {
    pub task_id: String,
    pub alt_model: String,
}

/// Default pricing table (sats per 1K tokens) for common models.
fn default_pricing() -> HashMap<String, f64> {
    // Blended rate: sats per 1K tokens (input * 0.3 + output * 0.7)
    // All rates from x402-info manifests (verified 2026-03-24)
    HashMap::from([
        ("gpt-5-nano".to_string(), 0.295),
        ("gpt-5-mini".to_string(), 1.475),
        ("gpt-5".to_string(), 7.375),
        ("gpt-5.2".to_string(), 10.325),
        ("o4-mini".to_string(), 3.41),
        ("gpt-5.2-pro".to_string(), 124.9),
        ("claude-haiku-4-5".to_string(), 3.8),
        ("claude-sonnet-4-6".to_string(), 11.4),
        ("claude-opus-4-6".to_string(), 19.0),
    ])
}

/// Look up pricing with prefix fallback for versioned model names.
fn lookup_pricing(pricing: &HashMap<String, f64>, model: &str) -> Option<f64> {
    // Exact match first
    if let Some(&rate) = pricing.get(model) {
        return Some(rate);
    }
    // Prefix match: find the longest pricing key that is a prefix of the model name
    pricing
        .iter()
        .filter(|(key, _)| model.starts_with(key.as_str()))
        .max_by_key(|(key, _)| key.len())
        .map(|(_, &rate)| rate)
}

/// GET /analytics/cost-comparison?task_id=X&alt_model=Y — replay a task with alternative pricing.
pub(crate) async fn get_cost_comparison(
    State(state): State<Arc<AppState>>,
    uri: axum::extract::OriginalUri,
    Query(params): Query<CostComparisonParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/analytics/cost-comparison",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let mut pricing = default_pricing();

    // Pre-resolve the alt_model rate using prefix fallback so that
    // cost_replay() finds it via exact match even for versioned names
    // like "gpt-5-mini-2025-08-07".
    if !pricing.contains_key(&params.alt_model) {
        if let Some(rate) = lookup_pricing(&pricing, &params.alt_model) {
            pricing.insert(params.alt_model.clone(), rate);
        }
    }

    match crate::analytics::cost_replay(
        &state.workspace,
        &params.task_id,
        &params.alt_model,
        &pricing,
    ) {
        Some(comparison) => {
            signed_json_response(&state, auth_ctx, StatusCode::OK, &comparison).await
        }
        None => {
            let body = serde_json::json!({
                "error": "Task not found or has no LLM calls",
                "task_id": params.task_id,
            });
            signed_json_response(&state, auth_ctx, StatusCode::NOT_FOUND, &body).await
        }
    }
}

/// GET /analytics/roi — ROI report for current session.
pub(crate) async fn get_roi(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/analytics/roi", None, &headers, None).await?;

    let report = crate::analytics::compute_roi(&state.workspace);

    signed_json_response(&state, auth_ctx, StatusCode::OK, &report).await
}

/// GET /context/analysis — token breakdown of the current context window state.
///
/// Returns a `TokenBreakdown` JSON showing how tokens are allocated across
/// system prompt, tool definitions, conversation, memory, and skills.
/// This is a diagnostic endpoint — it shows the template breakdown, not a
/// live task context.
pub(crate) async fn get_context_analysis(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/context/analysis", None, &headers, None).await?;

    // Build a representative token breakdown from the current config.
    // This is a static analysis — no live task context is available at the
    // server level, so we estimate from config defaults.
    let context_window = state.config.llm.context_window;
    let compaction_threshold = state.config.llm.compaction_threshold;

    let body = serde_json::json!({
        "context_window": context_window,
        "compaction_threshold": compaction_threshold,
        "compaction_threshold_pct": compaction_threshold * 100.0,
        "output_token_reservation": state.config.llm.max_tokens as usize,
        "input_budget": context_window.saturating_sub(state.config.llm.max_tokens as usize),
        "max_history_turns": state.config.llm.max_history_turns,
        "min_recent_messages": state.config.llm.min_recent_messages,
        "compaction_enabled": state.config.llm.compaction_enabled,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /analytics/benchmarks — provider benchmark summary.
pub(crate) async fn get_benchmarks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/analytics/benchmarks", None, &headers, None).await?;

    let mut entries = crate::analytics::load_benchmarks(&state.workspace);
    if entries.is_empty() {
        // Backfill from transcripts
        entries = crate::analytics::backfill_benchmarks_from_transcripts(&state.workspace);
        for entry in &entries {
            crate::analytics::record_benchmark(&state.workspace, entry);
        }
    }

    let benchmarks = crate::analytics::aggregate_benchmarks(&entries);

    signed_json_response(&state, auth_ctx, StatusCode::OK, &benchmarks).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_pricing_has_all_current_models() {
        let pricing = default_pricing();
        assert_eq!(pricing.len(), 9);
        assert!(pricing.contains_key("gpt-5-nano"));
        assert!(pricing.contains_key("gpt-5-mini"));
        assert!(pricing.contains_key("gpt-5"));
        assert!(pricing.contains_key("gpt-5.2"));
        assert!(pricing.contains_key("o4-mini"));
        assert!(pricing.contains_key("gpt-5.2-pro"));
        assert!(pricing.contains_key("claude-haiku-4-5"));
        assert!(pricing.contains_key("claude-sonnet-4-6"));
        assert!(pricing.contains_key("claude-opus-4-6"));
    }

    #[test]
    fn test_default_pricing_no_phantom_models() {
        let pricing = default_pricing();
        assert!(!pricing.contains_key("gpt-4.1"));
        assert!(!pricing.contains_key("gpt-4.1-mini"));
        assert!(!pricing.contains_key("gpt-4.1-nano"));
        assert!(!pricing.contains_key("claude-sonnet-4-20250514"));
        assert!(!pricing.contains_key("claude-opus-4-20250514"));
    }

    #[test]
    fn test_lookup_pricing_exact_match() {
        let pricing = default_pricing();
        assert_eq!(lookup_pricing(&pricing, "gpt-5-mini"), Some(1.475));
        assert_eq!(lookup_pricing(&pricing, "claude-sonnet-4-6"), Some(11.4));
    }

    #[test]
    fn test_lookup_pricing_prefix_fallback() {
        let pricing = default_pricing();
        // Versioned model names should match by prefix
        assert_eq!(
            lookup_pricing(&pricing, "gpt-5-mini-2025-08-07"),
            Some(1.475)
        );
        assert_eq!(
            lookup_pricing(&pricing, "claude-haiku-4-5-20251001"),
            Some(3.8)
        );
        assert_eq!(
            lookup_pricing(&pricing, "gpt-5.2-turbo-preview"),
            Some(10.325)
        );
    }

    #[test]
    fn test_lookup_pricing_no_match() {
        let pricing = default_pricing();
        assert_eq!(lookup_pricing(&pricing, "unknown-model"), None);
        assert_eq!(lookup_pricing(&pricing, "llama-3"), None);
    }

    #[test]
    fn test_lookup_pricing_longest_prefix_wins() {
        let pricing = default_pricing();
        // "gpt-5-mini-xxx" should match "gpt-5-mini" (longer) not "gpt-5" (shorter)
        let rate = lookup_pricing(&pricing, "gpt-5-mini-xxx").unwrap();
        assert_eq!(rate, 1.475); // gpt-5-mini rate, not gpt-5 rate (7.375)
    }
}
