//! Budget route handlers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use serde::Deserialize;

use super::super::auth::{check_brc31_auth, signed_json_response, signed_raw_response};
use super::super::types::*;
use super::super::AppState;

// -- Constants --

const DEFAULT_BSV_USD_RATE: f64 = 20.0;

// -- Handlers --

pub(crate) async fn get_budget(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/budget", None, &headers, None).await?;
    let balance = state.wallet.get_balance().await.unwrap_or(0);
    let budget = state.budget.lock().await;
    let body = budget.report_with_balance(balance);
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// Cached rate entry with source metadata.
#[derive(Debug, Clone)]
pub struct CachedRate {
    pub rate: f64,
    pub source: String,
    pub fetched_at: Instant,
    pub updated_at_rfc3339: String,
}

/// GET /rates/bsv-usd -- BSV/USD exchange rate (served from cache, background-refreshed).
pub(crate) async fn get_bsv_usd_rate(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let margin_percent = state.config.rates.margin_percent;
    let stale_threshold = state.config.rates.stale_threshold_secs;

    let cached = state.usd_rate_cache.read().await;
    if let Some(entry) = cached.as_ref() {
        let age_secs = entry.fetched_at.elapsed().as_secs();
        let stale = age_secs > stale_threshold;
        let rate_with_margin = apply_margin(entry.rate, margin_percent);
        return Json(serde_json::json!({
            "rate": entry.rate,
            "rate_with_margin": rate_with_margin,
            "source": entry.source,
            "updated_at": entry.updated_at_rfc3339,
            "stale": stale,
            "margin_percent": margin_percent,
        }));
    }
    drop(cached);

    // No cache yet -- do a synchronous fetch to seed it
    let (rate, source) = fetch_rate_multi_source()
        .await
        .unwrap_or((DEFAULT_BSV_USD_RATE, "fallback".to_string()));
    let now_rfc3339 = chrono::Utc::now().to_rfc3339();
    let rate_with_margin = apply_margin(rate, margin_percent);

    let mut cache = state.usd_rate_cache.write().await;
    *cache = Some(CachedRate {
        rate,
        source: source.clone(),
        fetched_at: Instant::now(),
        updated_at_rfc3339: now_rfc3339.clone(),
    });

    Json(serde_json::json!({
        "rate": rate,
        "rate_with_margin": rate_with_margin,
        "source": source,
        "updated_at": now_rfc3339,
        "stale": false,
        "margin_percent": margin_percent,
    }))
}

/// Apply a percentage margin to a rate.
pub(crate) fn apply_margin(rate: f64, margin_percent: f64) -> f64 {
    rate * (1.0 + margin_percent / 100.0)
}

/// Sane range for BSV/USD rate to catch obviously wrong values from the API.
const MIN_SANE_RATE: f64 = 1.0;
const MAX_SANE_RATE: f64 = 100_000.0;

/// Fetch BSV/USD rate with multi-source fallback.
/// Tries WhatsOnChain first, then CoinGecko. Returns (rate, source_name).
pub(crate) async fn fetch_rate_multi_source() -> Result<(f64, String), crate::error::DmError> {
    // Source 1: WhatsOnChain
    match fetch_rate_whatsOnChain().await {
        Ok(rate) => return Ok((rate, "whatsOnChain".to_string())),
        Err(e) => {
            tracing::warn!("WhatsOnChain rate fetch failed, trying CoinGecko: {e}");
        }
    }

    // Source 2: CoinGecko
    match fetch_rate_coingecko().await {
        Ok(rate) => Ok((rate, "coinGecko".to_string())),
        Err(e) => {
            tracing::warn!("CoinGecko rate fetch also failed: {e}");
            Err(e)
        }
    }
}

/// Fetch BSV/USD rate from WhatsOnChain (no API key needed).
/// Returns an error if the rate is outside the sane range [$1, $100,000].
#[allow(non_snake_case)]
pub(crate) async fn fetch_rate_whatsOnChain() -> Result<f64, crate::error::DmError> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.whatsonchain.com/v1/bsv/main/exchangerate")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| crate::error::DmError::wallet(format!("WoC rate fetch failed: {e}")))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::DmError::wallet(format!("WoC rate parse failed: {e}")))?;

    // WoC returns {"currency":"USD","rate":"45.23"} or similar
    let rate = data["rate"]
        .as_str()
        .and_then(|s: &str| s.parse::<f64>().ok())
        .or_else(|| data["rate"].as_f64())
        .ok_or_else(|| crate::error::DmError::wallet("WoC response missing rate field"))?;

    validate_rate(rate, "WoC")
}

/// Fetch BSV/USD rate from CoinGecko (no API key needed for public endpoints).
pub(crate) async fn fetch_rate_coingecko() -> Result<f64, crate::error::DmError> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.coingecko.com/api/v3/simple/price?ids=bitcoin-cash-sv&vs_currencies=usd")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| crate::error::DmError::wallet(format!("CoinGecko rate fetch failed: {e}")))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::DmError::wallet(format!("CoinGecko rate parse failed: {e}")))?;

    // CoinGecko returns {"bitcoin-cash-sv":{"usd":45.23}}
    let rate = data["bitcoin-cash-sv"]["usd"]
        .as_f64()
        .ok_or_else(|| crate::error::DmError::wallet("CoinGecko response missing rate field"))?;

    validate_rate(rate, "CoinGecko")
}

/// Validate a rate is within sane bounds.
fn validate_rate(rate: f64, source: &str) -> Result<f64, crate::error::DmError> {
    if !(MIN_SANE_RATE..=MAX_SANE_RATE).contains(&rate) || rate.is_nan() || rate.is_infinite() {
        tracing::warn!("BSV/USD rate from {source} out of sane range: {rate}");
        return Err(crate::error::DmError::wallet(format!(
            "BSV/USD rate {rate} from {source} outside sane range [{MIN_SANE_RATE}, {MAX_SANE_RATE}]"
        )));
    }
    Ok(rate)
}

/// GET /budget/detail -- granular spending breakdown by service and operation.
pub(crate) async fn get_budget_detail(
    State(state): State<Arc<AppState>>,
    uri: axum::extract::OriginalUri,
    Query(params): Query<BudgetDetailParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/budget/detail",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let budget = state.budget.lock().await;

    // Convert entries to DTOs, optionally filtering by task_id
    let entries: Vec<SpendingEntryDto> = budget
        .entries()
        .iter()
        .filter(|e| {
            if let Some(ref task_id) = params.task_id {
                // Filter by task_id in details metadata
                e.details
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .map(|t| t == task_id)
                    .unwrap_or(false)
            } else {
                true
            }
        })
        .map(|e| SpendingEntryDto {
            service: e.service.clone(),
            operation: e.operation.clone(),
            sats: e.sats,
            timestamp: e.timestamp.to_rfc3339(),
            details: e.details.clone(),
        })
        .collect();

    // Group by service -> operation
    let mut by_service: HashMap<String, ServiceBreakdown> = HashMap::new();
    let mut total_sats: u64 = 0;

    for entry in &entries {
        total_sats += entry.sats;
        let service = by_service
            .entry(entry.service.clone())
            .or_insert_with(|| ServiceBreakdown {
                total_sats: 0,
                operations: HashMap::new(),
            });
        service.total_sats += entry.sats;
        let op = service
            .operations
            .entry(entry.operation.clone())
            .or_insert_with(|| OperationStats {
                count: 0,
                total_sats: 0,
                avg_sats: 0,
            });
        op.count += 1;
        op.total_sats += entry.sats;
    }

    // Compute averages
    for service in by_service.values_mut() {
        for op in service.operations.values_mut() {
            op.avg_sats = if op.count > 0 {
                op.total_sats / op.count
            } else {
                0
            };
        }
    }

    // Include rate limit status if enabled
    let rate_limit_status = if state.rate_limiter.is_enabled() {
        Some(state.rate_limiter.status())
    } else {
        None
    };

    let body = serde_json::json!({
        "entries": entries,
        "by_service": by_service,
        "total_sats": total_sats,
        "rate_limits": rate_limit_status,
    });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// Query params for GET /budget/export.
#[derive(Deserialize)]
pub(crate) struct BudgetExportQuery {
    #[serde(default = "default_budget_format")]
    pub format: String,
}

fn default_budget_format() -> String {
    "json".to_string()
}

/// GET /budget/export -- downloadable budget report (JSON or PDF).
pub(crate) async fn get_budget_export(
    State(state): State<Arc<AppState>>,
    uri: axum::extract::OriginalUri,
    Query(query): Query<BudgetExportQuery>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/budget/export",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let budget = state.budget.lock().await;

    // Build the same detail structure as get_budget_detail
    let entries: Vec<SpendingEntryDto> = budget
        .entries()
        .iter()
        .map(|e| SpendingEntryDto {
            service: e.service.clone(),
            operation: e.operation.clone(),
            sats: e.sats,
            timestamp: e.timestamp.to_rfc3339(),
            details: e.details.clone(),
        })
        .collect();

    let mut by_service: HashMap<String, ServiceBreakdown> = HashMap::new();
    let mut total_sats: u64 = 0;

    for entry in &entries {
        total_sats += entry.sats;
        let service = by_service
            .entry(entry.service.clone())
            .or_insert_with(|| ServiceBreakdown {
                total_sats: 0,
                operations: HashMap::new(),
            });
        service.total_sats += entry.sats;
        let op = service
            .operations
            .entry(entry.operation.clone())
            .or_insert_with(|| OperationStats {
                count: 0,
                total_sats: 0,
                avg_sats: 0,
            });
        op.count += 1;
        op.total_sats += entry.sats;
    }

    for service in by_service.values_mut() {
        for op in service.operations.values_mut() {
            op.avg_sats = if op.count > 0 {
                op.total_sats / op.count
            } else {
                0
            };
        }
    }

    let detail = BudgetDetailResponse {
        entries,
        by_service,
        total_sats,
    };

    drop(budget);

    match query.format.as_str() {
        "pdf" => {
            let detail_json = serde_json::to_value(&detail).unwrap_or_default();
            let html = crate::server::pdf_template::render_budget_html(&detail_json);

            match super::tasks::html_to_pdf(&html).await {
                Ok(pdf_bytes) => {
                    signed_raw_response(
                        &state,
                        auth_ctx,
                        StatusCode::OK,
                        "application/pdf",
                        pdf_bytes,
                        vec![(
                            "content-disposition".into(),
                            "attachment; filename=\"budget-report.pdf\"".into(),
                        )],
                    )
                    .await
                }
                Err(e) => {
                    tracing::warn!("Budget PDF generation failed: {e}");
                    let body = serde_json::json!({
                        "error": format!("PDF generation unavailable: {e}"),
                        "hint": "Ensure Chrome/Chromium is installed, or use format=json",
                    });
                    signed_json_response(&state, auth_ctx, StatusCode::SERVICE_UNAVAILABLE, &body)
                        .await
                }
            }
        }
        _ => {
            // JSON format (default)
            let body_bytes = serde_json::to_vec_pretty(&detail).unwrap_or_default();
            signed_raw_response(
                &state,
                auth_ctx,
                StatusCode::OK,
                "application/json",
                body_bytes,
                vec![(
                    "content-disposition".into(),
                    "attachment; filename=\"budget-report.json\"".into(),
                )],
            )
            .await
        }
    }
}
