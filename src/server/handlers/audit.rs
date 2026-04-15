//! Audit-related route handlers — key linkage revelation, cross-agent proof linking,
//! and full-text search across audit transcripts.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::revelation::{Revelation, RevelationLog, RevelationsListResponse};

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

/// Request body for key linkage revelation endpoints.
#[derive(Debug, Deserialize)]
pub(crate) struct KeyLinkageRequest {
    /// The counterparty whose key linkage to reveal.
    pub counterparty: String,
    /// The verifier who will receive the linkage revelation.
    pub verifier: String,
    /// Protocol ID for the key derivation, e.g. [2, "dolphin milk transcript"].
    #[serde(default)]
    pub protocol_id: Option<serde_json::Value>,
    /// Key ID for the specific key derivation.
    #[serde(default)]
    pub key_id: Option<String>,
    /// Whether this is a privileged linkage revelation.
    #[serde(default)]
    pub privileged: bool,
}

/// Response from key linkage revelation endpoints.
#[derive(Debug, Serialize)]
pub(crate) struct KeyLinkageResponse {
    #[serde(rename = "type")]
    pub linkage_type: String,
    pub counterparty: String,
    pub verifier: String,
    #[serde(rename = "revelationData")]
    pub revelation_data: serde_json::Value,
    /// Unique revelation ID for audit tracking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revelation_id: Option<String>,
    /// SHA-256 hash of the revelation for integrity verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revelation_hash: Option<String>,
}

/// POST /audit/key-linkage/counterparty — reveal counterparty key linkage.
///
/// BRC-69 counterparty key linkage revelation. Proves to a verifier that
/// the agent's keys for a given counterparty are derived from the same
/// master key, without revealing keys for other counterparties.
pub(crate) async fn reveal_counterparty_linkage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<KeyLinkageRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        "/audit/key-linkage/counterparty",
        None,
        &headers,
        None,
    )
    .await?;

    match state
        .wallet
        .reveal_counterparty_key_linkage(&req.counterparty, &req.verifier, req.privileged)
        .await
    {
        Ok(result) => {
            // Build structured revelation
            let revelation =
                Revelation::counterparty(&req.counterparty, &req.verifier, result.clone());

            // Log revelation to the append-only audit directory
            let log = RevelationLog::new(&state.workspace.to_string_lossy());
            if let Err(e) = log.record(&revelation) {
                tracing::warn!("Failed to persist revelation log: {e}");
            }

            // Log the linkage request in budget JSONL for audit trail
            {
                let mut budget = state.budget.lock().await;
                budget.record(
                    "audit",
                    "counterparty_key_linkage",
                    0,
                    serde_json::json!({
                        "counterparty": req.counterparty,
                        "verifier": req.verifier,
                        "privileged": req.privileged,
                        "revelation_id": revelation.id,
                        "revelation_hash": revelation.revelation_hash,
                    }),
                );
            }

            let body = KeyLinkageResponse {
                linkage_type: "counterparty".to_string(),
                counterparty: req.counterparty,
                verifier: req.verifier,
                revelation_data: result,
                revelation_id: Some(revelation.id),
                revelation_hash: Some(revelation.revelation_hash),
            };
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
        Err(e) => {
            tracing::warn!("Counterparty key linkage revelation failed: {e}");
            let body = serde_json::json!({
                "error": format!("Key linkage revelation failed: {e}"),
            });
            signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body).await
        }
    }
}

/// POST /audit/key-linkage/specific — reveal specific key linkage.
///
/// BRC-70 specific key linkage revelation. Proves to a verifier the
/// derivation relationship for a specific protocol+keyID combination,
/// without revealing linkage for other protocol/key pairs.
pub(crate) async fn reveal_specific_linkage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<KeyLinkageRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        "/audit/key-linkage/specific",
        None,
        &headers,
        None,
    )
    .await?;

    // Specific linkage requires protocol_id and key_id
    let protocol_id = match &req.protocol_id {
        Some(p) => p.clone(),
        None => {
            let body = serde_json::json!({
                "error": "protocol_id is required for specific key linkage",
            });
            return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
        }
    };

    let key_id = match &req.key_id {
        Some(k) => k.clone(),
        None => {
            let body = serde_json::json!({
                "error": "key_id is required for specific key linkage",
            });
            return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
        }
    };

    match state
        .wallet
        .reveal_specific_key_linkage(
            &req.counterparty,
            &req.verifier,
            &protocol_id,
            &key_id,
            req.privileged,
        )
        .await
    {
        Ok(result) => {
            // Build structured revelation
            let protocol_str = protocol_id.to_string();
            let revelation = Revelation::specific(
                &req.counterparty,
                &protocol_str,
                &key_id,
                &req.verifier,
                result.clone(),
            );

            // Log revelation to the append-only audit directory
            let log = RevelationLog::new(&state.workspace.to_string_lossy());
            if let Err(e) = log.record(&revelation) {
                tracing::warn!("Failed to persist revelation log: {e}");
            }

            // Log the linkage request in budget JSONL for audit trail
            {
                let mut budget = state.budget.lock().await;
                budget.record(
                    "audit",
                    "specific_key_linkage",
                    0,
                    serde_json::json!({
                        "counterparty": req.counterparty,
                        "verifier": req.verifier,
                        "protocol_id": protocol_id,
                        "key_id": key_id,
                        "privileged": req.privileged,
                        "revelation_id": revelation.id,
                        "revelation_hash": revelation.revelation_hash,
                    }),
                );
            }

            let body = KeyLinkageResponse {
                linkage_type: "specific".to_string(),
                counterparty: req.counterparty,
                verifier: req.verifier,
                revelation_data: result,
                revelation_id: Some(revelation.id),
                revelation_hash: Some(revelation.revelation_hash),
            };
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
        Err(e) => {
            tracing::warn!("Specific key linkage revelation failed: {e}");
            let body = serde_json::json!({
                "error": format!("Key linkage revelation failed: {e}"),
            });
            signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body).await
        }
    }
}

/// A cross-reference match — a proof that references a given message hash.
#[derive(Debug, Serialize)]
pub(crate) struct CrossReferenceMatch {
    /// The proof transaction ID.
    pub proof_txid: String,
    /// The task ID containing this proof.
    pub task_id: String,
    /// The iteration within the task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    /// The proof type (e.g., "decision").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_type: Option<String>,
    /// Timestamp of the proof event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Response for GET /audit/cross-reference/{message_hash}.
#[derive(Debug, Serialize)]
pub(crate) struct CrossReferenceResponse {
    /// The message hash being queried.
    pub message_hash: String,
    /// Proofs that reference this message hash.
    pub matches: Vec<CrossReferenceMatch>,
    /// Total number of matches found.
    pub count: usize,
}

/// GET /audit/cross-reference/{message_hash} — find proofs referencing a message hash.
///
/// Scans all task transcripts for Decision proofs containing the given
/// cross-agent message hash. Enables verifying that both sender and receiver
/// recorded the same message exchange in their proof chains.
pub(crate) async fn cross_reference_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(message_hash): Path<String>,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/audit/cross-reference/{message_hash}"),
        None,
        &headers,
        None,
    )
    .await?;

    // Validate message hash format (should be 64-char hex SHA-256)
    if message_hash.len() != 64 || !message_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        let body = serde_json::json!({
            "error": "Invalid message hash: expected 64-char hex SHA-256 digest",
        });
        return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
    }

    let mut matches = Vec::new();

    // Scan all task transcript directories
    let tasks_dir = state.workspace.join("tasks");
    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let task_id = entry.file_name().to_string_lossy().to_string();
            let transcript_path = path.join("session.jsonl");
            if !transcript_path.exists() {
                continue;
            }

            let transcript = crate::transcript::Transcript::new(transcript_path);
            let events = transcript.replay();

            // Check proof_created events for message_hash references
            for event in events {
                if event.event_type != "proof_created" {
                    continue;
                }

                // Check if the proof data contains the message hash
                let proof_data = match event.data.get("data") {
                    Some(v) => v.as_str().unwrap_or(""),
                    None => "",
                };

                if !proof_data.contains(&message_hash) {
                    continue;
                }

                let proof_txid = match event.data.get("txid") {
                    Some(v) => v.as_str().unwrap_or("").to_string(),
                    None => String::new(),
                };
                let iteration = event
                    .data
                    .get("iteration")
                    .and_then(|v: &serde_json::Value| v.as_u64())
                    .map(|i| i as u32);
                let proof_type = event
                    .data
                    .get("proof_type")
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .map(|s: &str| s.to_string());

                if !proof_txid.is_empty() {
                    matches.push(CrossReferenceMatch {
                        proof_txid,
                        task_id: task_id.clone(),
                        iteration,
                        proof_type,
                        timestamp: Some(format!("{}", event.ts)),
                    });
                }
            }
        }
    }

    let count = matches.len();
    let body = CrossReferenceResponse {
        message_hash,
        matches,
        count,
    };

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

// ---------------------------------------------------------------------------
// Revelation audit log listing
// ---------------------------------------------------------------------------

/// GET /audit/revelations — list all past key linkage revelations.
///
/// Returns all revelations persisted in the `audit_revelations/` directory,
/// sorted by timestamp ascending. Each revelation includes its deterministic
/// SHA-256 hash for integrity verification.
pub(crate) async fn list_revelations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/audit/revelations", None, &headers, None).await?;

    let log = RevelationLog::new(&state.workspace.to_string_lossy());
    let revelations = log.list();
    let count = revelations.len();

    let body = RevelationsListResponse { revelations, count };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

// ---------------------------------------------------------------------------
// Full-text search across audit transcripts
// ---------------------------------------------------------------------------

/// Query parameters for GET /audit/search.
#[derive(Debug, Deserialize)]
pub(crate) struct AuditSearchParams {
    /// Search query — case-insensitive substring match.
    pub q: Option<String>,
    /// Maximum results to return (default 20, max 100).
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// Offset for pagination (default 0).
    #[serde(default)]
    pub offset: usize,
}

fn default_search_limit() -> usize {
    20
}

/// A single search hit from an audit transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSearchResult {
    /// The task ID containing this event.
    pub task_id: String,
    /// The transcript event type (e.g. "think_response", "tool_call").
    pub event_type: String,
    /// Unix timestamp of the event.
    pub timestamp: f64,
    /// Matched content snippet (the field value that contained the query).
    pub matched_content: String,
    /// Iteration number within the task, if determinable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
}

/// Response for GET /audit/search.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditSearchResponse {
    /// The original search query.
    pub query: String,
    /// Matching events.
    pub results: Vec<AuditSearchResult>,
    /// Total number of matches (before pagination).
    pub total: usize,
    /// Limit used.
    pub limit: usize,
    /// Offset used.
    pub offset: usize,
}

/// GET /audit/search?q=<query>&limit=20&offset=0 — full-text search across all task transcripts.
///
/// Scans all `working/tasks/*/session.jsonl` transcripts for events containing the
/// query string (case-insensitive substring match). Searches across event content,
/// tool names, model names, error messages, and proof data.
pub(crate) async fn audit_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditSearchParams>,
) -> Result<Json<AuditSearchResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Validate query parameter
    let query = match &params.q {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Missing or empty query parameter 'q'"
                })),
            ));
        }
    };

    let limit = params.limit.min(100);
    let offset = params.offset;
    let query_lower = query.to_lowercase();

    let mut all_matches: Vec<AuditSearchResult> = Vec::new();

    // Scan all task transcript directories
    let tasks_dir = state.workspace.join("tasks");
    if let Ok(entries) = std::fs::read_dir(&tasks_dir) {
        // Collect and sort task dirs for deterministic ordering
        let mut task_dirs: Vec<_> = entries.flatten().collect();
        task_dirs.sort_by_key(|e| e.file_name());

        for entry in task_dirs {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let task_id = entry.file_name().to_string_lossy().to_string();
            let transcript_path = path.join("session.jsonl");
            if !transcript_path.exists() {
                continue;
            }

            let transcript = crate::transcript::Transcript::new(transcript_path);
            let events = transcript.replay();

            // Track iteration count for enrichment
            let mut iteration: u32 = 0;

            for event in events {
                if event.event_type == "think_request" {
                    iteration += 1;
                }

                // Build a searchable text from the event data
                let searchable = build_searchable_text(event);

                if searchable.to_lowercase().contains(&query_lower) {
                    // Extract a matched snippet — find the portion containing the query
                    let matched_content = extract_snippet(&searchable, &query_lower);

                    all_matches.push(AuditSearchResult {
                        task_id: task_id.clone(),
                        event_type: event.event_type.clone(),
                        timestamp: event.ts,
                        matched_content,
                        iteration: if iteration > 0 { Some(iteration) } else { None },
                    });
                }
            }
        }
    }

    let total = all_matches.len();

    // Apply pagination
    let results: Vec<AuditSearchResult> =
        all_matches.into_iter().skip(offset).take(limit).collect();

    Ok(Json(AuditSearchResponse {
        query,
        results,
        total,
        limit,
        offset,
    }))
}

/// Build a searchable text string from a transcript event's data fields.
///
/// Concatenates relevant string fields from the event data so that a single
/// substring check covers content, tool names, model names, errors, etc.
fn build_searchable_text(event: &crate::transcript::TranscriptEvent) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Always include the event type itself
    parts.push(event.event_type.clone());

    // Extract string values from well-known fields
    for key in &[
        "content",
        "name",
        "model",
        "error",
        "role",
        "call_id",
        "proof_type",
        "data",
        "result",
        "finish_reason",
        "basket",
    ] {
        if let Some(val) = event.data.get(*key) {
            if let Some(s) = val.as_str() {
                parts.push(s.to_string());
            }
        }
    }

    // For tool_call events, also stringify the arguments
    if let Some(args) = event.data.get("arguments") {
        parts.push(args.to_string());
    }

    // For tool_result events, search the content (already covered above)
    // For proof events, include txid
    if let Some(txid) = event.data.get("txid") {
        if let Some(s) = txid.as_str() {
            parts.push(s.to_string());
        }
    }

    parts.join(" ")
}

/// Extract a snippet of text around where the query matched.
///
/// Returns up to 200 chars centered around the first occurrence of the query.
fn extract_snippet(text: &str, query_lower: &str) -> String {
    let text_lower = text.to_lowercase();
    let max_snippet = 200;

    if let Some(pos) = text_lower.find(query_lower) {
        // Work with char indices to avoid slicing inside multi-byte UTF-8
        let char_indices: Vec<(usize, char)> = text.char_indices().collect();
        // Find the char index corresponding to the byte position
        let char_pos = char_indices
            .iter()
            .position(|(b, _)| *b >= pos)
            .unwrap_or(0);
        let char_start = char_pos.saturating_sub(40);
        let char_end = (char_pos + query_lower.chars().count() + 40).min(char_indices.len());
        let byte_start = char_indices[char_start].0;
        let byte_end = if char_end >= char_indices.len() {
            text.len()
        } else {
            char_indices[char_end].0
        };
        let snippet = &text[byte_start..byte_end];
        if snippet.chars().count() > max_snippet {
            let truncated: String = snippet.chars().take(max_snippet).collect();
            format!("{}...", truncated)
        } else {
            snippet.to_string()
        }
    } else {
        // Shouldn't happen, but fallback
        if text.chars().count() > max_snippet {
            let truncated: String = text.chars().take(max_snippet).collect();
            format!("{}...", truncated)
        } else {
            text.to_string()
        }
    }
}
