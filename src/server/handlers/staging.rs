//! Transaction staging route handlers (list, approve, abort).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

// -- Handlers --

/// GET /staged -- list all staged transactions.
pub(crate) async fn list_staged(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/staged", None, &headers, None).await?;

    let staged = state.staged_transactions.lock().await;
    let items: Vec<super::super::StagedTransaction> = staged.values().cloned().collect();

    let body = serde_json::json!({
        "staged": items,
        "total": items.len(),
        "staging_threshold": state.config.budget.staging_threshold,
    });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// POST /staged/{ref}/approve -- approve and sign a staged transaction.
///
/// For tool approval refs (prefixed with `tool-`), writes an approval file
/// instead of signing a wallet action.
pub(crate) async fn approve_staged(
    State(state): State<Arc<AppState>>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/staged/{reference}/approve"),
        None,
        &headers,
        None,
    )
    .await?;

    // Tool approval: file-based signal instead of wallet signing
    if reference.starts_with("tool-") {
        // Extract task_id from staged_transactions to find the workspace
        let task_id = {
            let staged = state.staged_transactions.lock().await;
            match staged.get(&reference) {
                Some(tx) if tx.status != "pending" => {
                    let body = serde_json::json!({
                        "error": format!("Tool approval is already {}", tx.status),
                    });
                    return signed_json_response(&state, auth_ctx, StatusCode::CONFLICT, &body)
                        .await;
                }
                Some(tx) => tx.task_id.clone(),
                None => None,
            }
        };

        // Extract call_id from the reference: "tool-{name}-{call_id}"
        let call_id = reference.rsplit('-').next().unwrap_or(&reference);

        // Write approval file to task workspace
        let approved = if let Some(ref tid) = task_id {
            let approval_dir = state
                .workspace
                .join("tasks")
                .join(tid)
                .join("pending_approval");
            let approved_file = approval_dir.join(format!("{}-approved.json", call_id));
            let _ = std::fs::write(&approved_file, "{}");
            approved_file.exists()
        } else {
            false
        };

        // Update staged status
        {
            let mut staged = state.staged_transactions.lock().await;
            if let Some(tx) = staged.get_mut(&reference) {
                tx.status = "approved".to_string();
            }
        }

        let body = serde_json::json!({
            "status": "approved",
            "reference": reference,
            "tool_approval": true,
            "file_written": approved,
        });
        return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
    }

    // Check that the staged transaction exists and is pending
    {
        let staged = state.staged_transactions.lock().await;
        match staged.get(&reference) {
            Some(tx) if tx.status != "pending" => {
                let body = serde_json::json!({
                    "error": format!("Transaction is already {}", tx.status),
                });
                return signed_json_response(&state, auth_ctx, StatusCode::CONFLICT, &body).await;
            }
            None => {
                return Err(StatusCode::NOT_FOUND);
            }
            _ => {}
        }
    }

    // Sign the action via wallet
    match state.wallet.sign_action(&reference).await {
        Ok(result) => {
            // Update status to approved
            let amount_sats;
            {
                let mut staged = state.staged_transactions.lock().await;
                if let Some(tx) = staged.get_mut(&reference) {
                    tx.status = "approved".to_string();
                    amount_sats = tx.amount_sats;
                } else {
                    amount_sats = 0;
                }
            }

            // Log in budget
            {
                let mut budget = state.budget.lock().await;
                budget.record(
                    "staging",
                    "approve",
                    amount_sats,
                    serde_json::json!({
                        "reference": reference,
                    }),
                );
            }

            let body = serde_json::json!({
                "status": "approved",
                "reference": reference,
                "result": result,
            });
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
        Err(e) => {
            let body = serde_json::json!({
                "error": format!("Failed to sign transaction: {e}"),
                "reference": reference,
            });
            signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body).await
        }
    }
}

/// POST /staged/{ref}/abort -- abort a staged transaction and release funds.
///
/// For tool approval refs (prefixed with `tool-`), writes an abort file
/// instead of aborting a wallet action.
pub(crate) async fn abort_staged(
    State(state): State<Arc<AppState>>,
    Path(reference): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        &format!("/staged/{reference}/abort"),
        None,
        &headers,
        None,
    )
    .await?;

    // Tool approval: file-based signal instead of wallet abort
    if reference.starts_with("tool-") {
        let task_id = {
            let staged = state.staged_transactions.lock().await;
            match staged.get(&reference) {
                Some(tx) if tx.status != "pending" => {
                    let body = serde_json::json!({
                        "error": format!("Tool approval is already {}", tx.status),
                    });
                    return signed_json_response(&state, auth_ctx, StatusCode::CONFLICT, &body)
                        .await;
                }
                Some(tx) => tx.task_id.clone(),
                None => None,
            }
        };

        let call_id = reference.rsplit('-').next().unwrap_or(&reference);

        let aborted = if let Some(ref tid) = task_id {
            let approval_dir = state
                .workspace
                .join("tasks")
                .join(tid)
                .join("pending_approval");
            let aborted_file = approval_dir.join(format!("{}-aborted.json", call_id));
            let _ = std::fs::write(&aborted_file, "{}");
            aborted_file.exists()
        } else {
            false
        };

        {
            let mut staged = state.staged_transactions.lock().await;
            if let Some(tx) = staged.get_mut(&reference) {
                tx.status = "aborted".to_string();
            }
        }

        let body = serde_json::json!({
            "status": "aborted",
            "reference": reference,
            "tool_approval": true,
            "file_written": aborted,
        });
        return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
    }

    // Check that the staged transaction exists and is pending
    {
        let staged = state.staged_transactions.lock().await;
        match staged.get(&reference) {
            Some(tx) if tx.status != "pending" => {
                let body = serde_json::json!({
                    "error": format!("Transaction is already {}", tx.status),
                });
                return signed_json_response(&state, auth_ctx, StatusCode::CONFLICT, &body).await;
            }
            None => {
                return Err(StatusCode::NOT_FOUND);
            }
            _ => {}
        }
    }

    // Abort the action via wallet
    match state.wallet.abort_action(&reference).await {
        Ok(result) => {
            // Update status to aborted
            {
                let mut staged = state.staged_transactions.lock().await;
                if let Some(tx) = staged.get_mut(&reference) {
                    tx.status = "aborted".to_string();
                }
            }

            // Log in budget
            {
                let mut budget = state.budget.lock().await;
                budget.record(
                    "staging",
                    "abort",
                    0,
                    serde_json::json!({
                        "reference": reference,
                    }),
                );
            }

            let body = serde_json::json!({
                "status": "aborted",
                "reference": reference,
                "result": result,
            });
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
        Err(e) => {
            let body = serde_json::json!({
                "error": format!("Failed to abort transaction: {e}"),
                "reference": reference,
            });
            signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body).await
        }
    }
}
