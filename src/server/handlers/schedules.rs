//! Schedule management route handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

// -- Handlers --

/// GET /schedules -- list all scheduled tasks.
pub(crate) async fn list_schedules_endpoint(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/schedules", None, &headers, None).await?;
    let schedules_dir = state.workspace.join("schedules");

    let entries = match std::fs::read_dir(&schedules_dir) {
        Ok(e) => e,
        Err(_) => {
            return signed_json_response(&state, auth_ctx, StatusCode::OK, &serde_json::json!([]))
                .await
        }
    };

    let mut schedules: Vec<serde_json::Value> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let schedule: serde_json::Value = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        schedules.push(schedule);
    }

    // Sort by next_run for consistent output
    schedules.sort_by(|a, b| {
        let a_next = a.get("next_run").and_then(|v| v.as_str()).unwrap_or("");
        let b_next = b.get("next_run").and_then(|v| v.as_str()).unwrap_or("");
        a_next.cmp(b_next)
    });

    signed_json_response(
        &state,
        auth_ctx,
        StatusCode::OK,
        &serde_json::json!(schedules),
    )
    .await
}

/// GET /schedules/{id} -- get a specific schedule.
pub(crate) async fn get_schedule_endpoint(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/schedules/{id}"),
        None,
        &headers,
        None,
    )
    .await?;
    let path = state.workspace.join("schedules").join(format!("{id}.json"));

    let content = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;

    let schedule: serde_json::Value =
        serde_json::from_str(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    signed_json_response(&state, auth_ctx, StatusCode::OK, &schedule).await
}
