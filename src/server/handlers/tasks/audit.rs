//! Audit trail handlers: audit chain, export (CSV/JSON/PDF), transcript HMAC verification.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use base64::Engine;

use crate::server::auth::{check_brc31_auth, signed_json_response, signed_raw_response};
use crate::server::types::*;
use crate::server::AppState;

// -- Local types --

/// Query params for GET /task/{id}/audit/export.
#[derive(Deserialize)]
pub(crate) struct AuditExportQuery {
    #[serde(default = "default_csv")]
    format: String,
}

fn default_csv() -> String {
    "csv".to_string()
}

/// Response for GET /task/{id}/transcript/verify.
#[derive(Debug, Serialize)]
pub(crate) struct TranscriptVerifyResponse {
    pub task_id: String,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hmac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// -- Handlers --

/// GET /task/{id}/audit -- full audit chain from transcript.
pub(crate) async fn get_audit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/audit"),
        None,
        &headers,
        None,
    )
    .await?;
    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let raw_events = transcript.replay();

    let events: Vec<AuditEvent> = raw_events
        .iter()
        .map(|e| {
            // Combine the flat data fields into a single JSON value
            let data = serde_json::to_value(&e.data)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            AuditEvent {
                timestamp: e.ts,
                event_type: e.event_type.clone(),
                data,
            }
        })
        .collect();

    // Compute summary
    let total_events = events.len();
    let tool_calls = raw_events
        .iter()
        .filter(|e| e.event_type == "tool_call")
        .count();
    let proof_count = raw_events
        .iter()
        .filter(|e| e.event_type == "proof_created" || e.event_type == "checkpoint_created")
        .count();
    let iterations = raw_events
        .iter()
        .filter(|e| e.event_type == "think_request")
        .count() as u32;
    let sats_spent = transcript.total_sats_spent();

    let duration_secs = if let (Some(first), Some(last)) = (raw_events.first(), raw_events.last()) {
        last.ts - first.ts
    } else {
        0.0
    };

    let summary = AuditSummary {
        total_events,
        iterations,
        sats_spent,
        proof_count,
        tool_calls,
        duration_secs,
    };

    let conversation_id = {
        let sessions = state.task_mgr.task_sessions.lock().await;
        sessions.get(&id).cloned()
    };
    let body = AuditResponse {
        task_id: id,
        events,
        summary,
        conversation_id,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /task/{id}/audit/export -- downloadable audit report (CSV or signed JSON).
pub(crate) async fn get_audit_export(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    uri: axum::extract::OriginalUri,
    Query(query): Query<AuditExportQuery>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/audit/export"),
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let raw_events = transcript.replay();

    // Build events with flattened fields
    let mut current_iteration: u32 = 0;
    let events: Vec<serde_json::Value> = raw_events
        .iter()
        .map(|e| {
            if e.event_type == "think_request" {
                current_iteration += 1;
            }
            let model = e
                .data
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_name = e
                .data
                .get("tool_name")
                .or_else(|| e.data.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sats_spent = e
                .data
                .get("sats_effective")
                .or_else(|| e.data.get("sats_paid"))
                .or_else(|| e.data.get("sats"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let proof_txid = e
                .data
                .get("txid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Build a one-line detail summary
            let detail = match e.event_type.as_str() {
                "think_request" => format!("model={model}"),
                "think_response" => {
                    let tokens = e
                        .data
                        .get("total_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    format!("model={model} tokens={tokens}")
                }
                "tool_call" | "tool_result" => {
                    let args = e
                        .data
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let truncated = if args.len() > 100 { &args[..100] } else { args };
                    format!("{tool_name}: {truncated}")
                }
                "proof_created" | "checkpoint_created" => {
                    let pt = e
                        .data
                        .get("proof_type")
                        .or_else(|| e.data.get("token_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    format!("{pt} txid={proof_txid}")
                }
                "error" => {
                    let msg = e
                        .data
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    msg.chars().take(120).collect()
                }
                _ => String::new(),
            };

            serde_json::json!({
                "timestamp": e.ts,
                "iteration": current_iteration,
                "event_type": e.event_type,
                "model": model,
                "tool_name": tool_name,
                "sats_spent": sats_spent,
                "proof_txid": proof_txid,
                "detail": detail,
                "data": &e.data,
            })
        })
        .collect();

    // Build summary
    let total_events = events.len();
    let tool_calls = raw_events
        .iter()
        .filter(|e| e.event_type == "tool_call")
        .count();
    let proof_count = raw_events
        .iter()
        .filter(|e| e.event_type == "proof_created" || e.event_type == "checkpoint_created")
        .count();
    let iterations = raw_events
        .iter()
        .filter(|e| e.event_type == "think_request")
        .count() as u32;
    let sats_spent = transcript.total_sats_spent();
    let duration_secs = if let (Some(first), Some(last)) = (raw_events.first(), raw_events.last()) {
        last.ts - first.ts
    } else {
        0.0
    };

    let summary = serde_json::json!({
        "total_events": total_events,
        "iterations": iterations,
        "sats_spent": sats_spent,
        "proof_count": proof_count,
        "tool_calls": tool_calls,
        "duration_secs": duration_secs,
    });

    // Build proofs list
    let proofs: Vec<serde_json::Value> = raw_events
        .iter()
        .filter(|e| e.event_type == "proof_created" || e.event_type == "checkpoint_created")
        .filter_map(|e| {
            let txid = e.data.get("txid").and_then(|v| v.as_str())?;
            let proof_type = e
                .data
                .get("proof_type")
                .or_else(|| e.data.get("token_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let hash = e.data.get("hash").and_then(|v| v.as_str()).unwrap_or("");
            let iteration = e.data.get("iteration").and_then(|v| v.as_u64());
            Some(serde_json::json!({
                "txid": txid,
                "proof_type": proof_type,
                "hash": hash,
                "timestamp": e.ts,
                "iteration": iteration,
            }))
        })
        .collect();

    let short_id = if id.len() > 8 { &id[..8] } else { &id };
    let today = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Simple date formatting without chrono dependency
        let days = now / 86400;
        let years = (days * 400 / 146097) + 1970;
        // Approximate — good enough for filenames
        format!(
            "{years}-{:02}-{:02}",
            (days % 365) / 30 + 1,
            (days % 30) + 1
        )
    };

    match query.format.as_str() {
        "json" => {
            // Build the signable payload (events + proofs + summary, without signature)
            let payload = serde_json::json!({
                "events": &events,
                "proofs": &proofs,
                "summary": &summary,
            });
            let payload_str = serde_json::to_string(&payload).unwrap_or_default();

            // SHA-256 hash of the payload
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(payload_str.as_bytes());
            let hash_hex = hex::encode(hash);

            // Sign with agent's identity key via wallet (best-effort)
            let (signature_hex, agent_identity) = match state
                .wallet
                .create_signature(
                    &hash,
                    &serde_json::json!([2, "dolphin milk audit"]),
                    "audit-export",
                    "self",
                )
                .await
            {
                Ok(sig) => {
                    let identity = state.wallet.get_identity_key().await.unwrap_or_default();
                    (hex::encode(&sig), identity)
                }
                Err(e) => {
                    tracing::warn!("Failed to sign audit export: {e}");
                    (String::new(), String::new())
                }
            };

            let now_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();

            let export = serde_json::json!({
                "export_version": "1.0",
                "exported_at": now_ts,
                "agent_identity": agent_identity,
                "task_id": &id,
                "summary": &summary,
                "events": &events,
                "proofs": &proofs,
                "signature": signature_hex,
                "signature_hash": hash_hex,
            });

            let body_bytes = serde_json::to_vec_pretty(&export).unwrap_or_default();
            let filename = format!("task-{short_id}-audit-{today}.json");
            signed_raw_response(
                &state,
                auth_ctx,
                StatusCode::OK,
                "application/json",
                body_bytes,
                vec![(
                    "content-disposition".into(),
                    format!("attachment; filename=\"{filename}\""),
                )],
            )
            .await
        }
        "pdf" => {
            // PDF format — render HTML template, then convert to PDF via Chrome
            let html =
                crate::server::pdf_template::render_audit_html(&id, &events, &proofs, &summary);

            match html_to_pdf(&html).await {
                Ok(pdf_bytes) => {
                    let filename = format!("task-{short_id}-audit-{today}.pdf");
                    signed_raw_response(
                        &state,
                        auth_ctx,
                        StatusCode::OK,
                        "application/pdf",
                        pdf_bytes,
                        vec![(
                            "content-disposition".into(),
                            format!("attachment; filename=\"{filename}\""),
                        )],
                    )
                    .await
                }
                Err(e) => {
                    tracing::warn!("PDF generation failed: {e}");
                    let body = serde_json::json!({
                        "error": format!("PDF generation unavailable: {e}"),
                        "hint": "Ensure Chrome/Chromium is installed, or use format=csv or format=json",
                    });
                    signed_json_response(&state, auth_ctx, StatusCode::SERVICE_UNAVAILABLE, &body)
                        .await
                }
            }
        }
        _ => {
            // CSV format
            let mut csv = String::from(
                "timestamp,iteration,event_type,model,tool_name,sats_spent,proof_txid,detail\n",
            );
            for ev in &events {
                let ts = ev.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let iter = ev.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0);
                let etype = ev.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
                let model = ev.get("model").and_then(|v| v.as_str()).unwrap_or("");
                let tool = ev.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                let sats = ev.get("sats_spent").and_then(|v| v.as_u64()).unwrap_or(0);
                let txid = ev.get("proof_txid").and_then(|v| v.as_str()).unwrap_or("");
                let detail = ev.get("detail").and_then(|v| v.as_str()).unwrap_or("");
                // Escape detail for CSV (RFC 4180)
                let detail_escaped =
                    if detail.contains(',') || detail.contains('"') || detail.contains('\n') {
                        format!("\"{}\"", detail.replace('"', "\"\""))
                    } else {
                        detail.to_string()
                    };
                csv.push_str(&format!(
                    "{ts},{iter},{etype},{model},{tool},{sats},{txid},{detail_escaped}\n"
                ));
            }

            let filename = format!("task-{short_id}-audit-{today}.csv");
            signed_raw_response(
                &state,
                auth_ctx,
                StatusCode::OK,
                "text/csv",
                csv.into_bytes(),
                vec![(
                    "content-disposition".into(),
                    format!("attachment; filename=\"{filename}\""),
                )],
            )
            .await
        }
    }
}

/// GET /task/{id}/transcript/verify -- verify transcript HMAC integrity.
///
/// Reads the transcript JSONL, extracts the HMAC from the Checkpoint token
/// event, and calls wallet.verify_hmac() to confirm the transcript has not
/// been tampered with since the session ended.
pub(crate) async fn verify_transcript(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/transcript/verify"),
        None,
        &headers,
        None,
    )
    .await?;

    let transcript_path = state.workspace.join(format!("tasks/{id}/session.jsonl"));
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);

    // Find the HMAC from the checkpoint_created event's checkpoint_data
    let checkpoint_events = transcript.get_events_by_type("checkpoint_created");
    let hmac_b64 = checkpoint_events
        .iter()
        .rev() // most recent checkpoint first
        .find_map(|e| {
            e.data
                .get("checkpoint_data")
                .and_then(|cd| cd.get("hmac"))
                .and_then(|v| v.as_str())
                .map(String::from)
        });

    let hmac_b64 = match hmac_b64 {
        Some(h) => h,
        None => {
            let body = TranscriptVerifyResponse {
                task_id: id,
                valid: false,
                hmac: None,
                error: Some("No HMAC found in checkpoint data".to_string()),
            };
            return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
        }
    };

    // Decode the HMAC from base64
    let hmac_bytes = match base64::engine::general_purpose::STANDARD.decode(&hmac_b64) {
        Ok(bytes) => bytes,
        Err(e) => {
            let body = TranscriptVerifyResponse {
                task_id: id,
                valid: false,
                hmac: Some(hmac_b64),
                error: Some(format!("Invalid base64 HMAC: {e}")),
            };
            return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
        }
    };

    // Read the transcript content
    let transcript_bytes = transcript.read_bytes();
    if transcript_bytes.is_empty() {
        let body = TranscriptVerifyResponse {
            task_id: id,
            valid: false,
            hmac: Some(hmac_b64),
            error: Some("Transcript file is empty".to_string()),
        };
        return signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await;
    }

    // Verify HMAC via wallet
    match state
        .wallet
        .verify_hmac(
            &transcript_bytes,
            &hmac_bytes,
            &serde_json::json!([2, "dolphin milk transcript"]),
            &id,
            "self",
        )
        .await
    {
        Ok(valid) => {
            let body = TranscriptVerifyResponse {
                task_id: id,
                valid,
                hmac: Some(hmac_b64),
                error: None,
            };
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
        Err(e) => {
            let body = TranscriptVerifyResponse {
                task_id: id,
                valid: false,
                hmac: Some(hmac_b64),
                error: Some(format!("Wallet HMAC verification failed: {e}")),
            };
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
    }
}

// =============================================================================
// PDF generation via chromiumoxide
// =============================================================================

/// Convert an HTML string to PDF bytes using a headless Chrome instance.
///
/// Launches Chrome, creates a page, sets the HTML content, then prints to PDF.
/// Returns an error string if Chrome is not available or PDF generation fails.
#[cfg(feature = "browser")]
pub(crate) async fn html_to_pdf(html: &str) -> Result<Vec<u8>, String> {
    use chromiumoxide::browser::{Browser, BrowserConfig as ChromeBrowserConfig};
    use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
    use futures::StreamExt;

    let builder = ChromeBrowserConfig::builder()
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage");

    let config = builder
        .build()
        .map_err(|e| format!("Failed to build Chrome config: {e}"))?;

    let (browser, mut handler) = Browser::launch(config).await.map_err(|e| {
        format!("Chrome not available. Ensure Chrome/Chromium is installed. Error: {e}")
    })?;

    // Drive the CDP WebSocket event loop
    let handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser
        .new_page("about:blank")
        .await
        .map_err(|e| format!("Failed to create page: {e}"))?;

    page.set_content(html)
        .await
        .map_err(|e| format!("Failed to set page content: {e}"))?;

    // Small delay to let CSS render
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let pdf_params = PrintToPdfParams::builder().print_background(true).build();

    let pdf_bytes = page
        .pdf(pdf_params)
        .await
        .map_err(|e| format!("PDF generation failed: {e}"))?;

    // Clean up
    drop(browser);
    handle.abort();

    Ok(pdf_bytes)
}

#[cfg(not(feature = "browser"))]
pub(crate) async fn html_to_pdf(_html: &str) -> Result<Vec<u8>, String> {
    Err("PDF export requires the 'browser' feature (chromiumoxide). Build with: cargo build --features browser".into())
}
