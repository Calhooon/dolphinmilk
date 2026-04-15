//! Compliance and lifecycle status route handlers.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

// -- Local types --

/// Query params for compliance report.
#[derive(Debug, Deserialize)]
pub(crate) struct ComplianceReportQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

// -- Handlers --

/// GET /lifecycle/status -- UTXO lifecycle management status.
pub(crate) async fn lifecycle_status(
    State(state): State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    let (state_count, state_oldest) = crate::onchain::state::basket_status(
        state.wallet.as_ref(),
        crate::onchain::state::BASKET_STATE,
    )
    .await;
    let (budget_count, budget_oldest) = crate::onchain::state::basket_status(
        state.wallet.as_ref(),
        crate::onchain::state::BASKET_BUDGET,
    )
    .await;
    let (proofs_count, proofs_oldest) = crate::onchain::state::basket_status(
        state.wallet.as_ref(),
        crate::onchain::state::BASKET_PROOFS,
    )
    .await;

    let last_sweep = state.config.lifecycle.sweep_interval_minutes;

    // Growth rate estimation for proofs basket (informational, for capacity planning)
    let proofs_per_day = crate::heartbeat::estimate_proofs_per_day(proofs_count, proofs_oldest);

    axum::Json(serde_json::json!({
        "baskets": {
            "dm-state": {
                "count": state_count,
                "oldest_age_hours": state_oldest,
            },
            "dm-budget": {
                "count": budget_count,
                "oldest_age_hours": budget_oldest,
            },
            "dm-proofs": {
                "count": proofs_count,
                "oldest_age_hours": proofs_oldest,
                "growth_rate_per_day": proofs_per_day,
                "note": "immutable, not swept",
            },
        },
        "config": {
            "budget_token_max_age_hours": state.config.lifecycle.budget_token_max_age_hours,
            "checkpoint_max_age_hours": state.config.lifecycle.checkpoint_max_age_hours,
            "auto_sweep_enabled": state.config.lifecycle.auto_sweep_enabled,
            "sweep_interval_minutes": last_sweep,
            "basket_monitoring_interval_secs": state.config.lifecycle.basket_monitoring_interval_secs,
        },
    }))
}

/// GET /compliance/report -- compliance status and task summary.
pub(crate) async fn compliance_report(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: axum::extract::OriginalUri,
    Query(query): Query<ComplianceReportQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/compliance/report",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let compliance = &state.config.compliance;

    // Parse date range (defaults to all time)
    let from_ts = query.from.as_ref().and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
    });
    let to_ts = query.to.as_ref().and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp())
    });

    // Scan tasks from workspace to build compliance summary
    let tasks_dir = state.workspace.join("tasks");
    let mut total_tasks = 0u64;
    let mut total_proofs = 0u64;
    let mut total_sats = 0u64;
    let mut oldest_proof: Option<String> = None;
    let mut task_summaries: Vec<serde_json::Value> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let transcript_path = entry.path().join("session.jsonl");
            if !transcript_path.exists() {
                continue;
            }

            let task_id = entry.file_name().to_string_lossy().to_string();
            let transcript = crate::session::transcript::Transcript::new(transcript_path);

            // Check date range filter using session_start event
            let events = transcript.replay();
            let started_at: Option<String> = events.iter().find_map(|e| {
                if e.event_type == "session_start" {
                    e.data
                        .get("started_at")
                        .and_then(|v: &serde_json::Value| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| {
                            // Fallback: use the event's own timestamp
                            chrono::DateTime::from_timestamp(e.ts as i64, 0)
                                .map(|dt| dt.to_rfc3339())
                        })
                } else {
                    None
                }
            });

            if let Some(ref started) = started_at {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(started) {
                    let ts = dt.timestamp();
                    if let Some(from) = from_ts {
                        if ts < from {
                            continue;
                        }
                    }
                    if let Some(to) = to_ts {
                        if ts > to {
                            continue;
                        }
                    }
                }
            }

            total_tasks += 1;

            // Count proofs and sats from transcript
            let mut task_proof_count = 0u64;
            let task_sats = transcript.total_sats_spent();
            total_sats += task_sats;

            let completed_at: Option<String> = events.iter().rev().find_map(|e| {
                if e.event_type == "session_end" {
                    e.data
                        .get("completed_at")
                        .and_then(|v: &serde_json::Value| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            });

            let iterations = events
                .iter()
                .filter(|e| e.event_type == "think_response")
                .count() as u64;

            for event in events {
                if event.event_type == "proof_created" {
                    task_proof_count += 1;
                    if let Some(ts_val) = event.data.get("timestamp") {
                        let ts_owned = ts_val.as_str().unwrap_or_default().to_string();
                        match &oldest_proof {
                            Some(existing) if ts_owned < *existing => oldest_proof = Some(ts_owned),
                            None => oldest_proof = Some(ts_owned),
                            _ => {}
                        }
                    }
                }
            }

            total_proofs += task_proof_count;

            let classification = if compliance.enabled {
                Some(&compliance.financial_classification)
            } else {
                None
            };

            let empty_regs: Vec<String> = Vec::new();
            let compliance_tags = if compliance.enabled {
                &compliance.regulations
            } else {
                &empty_regs
            };

            task_summaries.push(serde_json::json!({
                "task_id": task_id,
                "started_at": started_at,
                "completed_at": completed_at,
                "iterations": iterations,
                "sats_spent": task_sats,
                "proof_count": task_proof_count,
                "compliance_tags": compliance_tags,
                "classification": classification,
            }));
        }
    }

    // Sort tasks by started_at (most recent first)
    task_summaries.sort_by(|a, b| {
        let a_ts = a.get("started_at").and_then(|v| v.as_str()).unwrap_or("");
        let b_ts = b.get("started_at").and_then(|v| v.as_str()).unwrap_or("");
        b_ts.cmp(a_ts)
    });

    // Calculate USD equivalent
    let usd_rate = state
        .usd_rate_cache
        .read()
        .await
        .as_ref()
        .map(|entry| entry.rate)
        .unwrap_or(20.0);
    let total_usd = (total_sats as f64 / 100_000_000.0) * usd_rate;

    // Check retention compliance: are all proofs within retention window?
    let retention_compliant = if compliance.enabled {
        // If oldest proof is within retention_days, we're compliant
        oldest_proof
            .as_ref()
            .map(|ts| {
                chrono::DateTime::parse_from_rfc3339(ts)
                    .map(|dt| {
                        let age_days =
                            (chrono::Utc::now() - dt.with_timezone(&chrono::Utc)).num_days();
                        age_days <= compliance.default_retention_days as i64
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(true) // No proofs = compliant
    } else {
        true
    };

    let body = serde_json::json!({
        "report_period": {
            "from": query.from,
            "to": query.to,
        },
        "compliance_mode": compliance.enabled,
        "worm_mode": compliance.worm_mode,
        "regulations": compliance.regulations,
        "retention_days": compliance.default_retention_days,
        "summary": {
            "total_tasks": total_tasks,
            "total_proofs": total_proofs,
            "total_sats_spent": total_sats,
            "total_usd_spent": (total_usd * 100.0).round() / 100.0,
            "proofs_with_compliance_tags": if compliance.enabled { total_proofs } else { 0 },
            "oldest_proof": oldest_proof,
            "retention_compliant": retention_compliant,
        },
        "tasks": task_summaries,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}
