//! Proof-related handlers: proof listing, on-chain verification, custody proofs, receipts.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::server::auth::{check_brc31_auth, signed_json_response};
use crate::server::types::*;
use crate::server::AppState;

// -- Local types --

/// Per-proof on-chain verification result.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProofVerification {
    pub txid: String,
    pub hash_recompute: String, // "match" | "mismatch" | "unavailable"
    pub on_chain_match: String, // "match" | "mismatch" | "not_found"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_chain_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recomputed_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub task_id: String,
    pub verifications: Vec<ProofVerification>,
}

/// Response from GET /task/{id}/receipts.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReceiptsResponse {
    pub task_id: String,
    pub receipts: Vec<ReceiptDetail>,
    pub total_sats_paid: u64,
    pub total_sats_refunded: u64,
}

/// A single BEEF receipt detail.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReceiptDetail {
    pub iteration: u32,
    pub model: String,
    pub sats_paid: u64,
    pub sats_effective: u64,
    pub sats_refunded: u64,
    pub tokens: u64,
    pub timestamp: String,
}

/// Response for GET /task/{id}/proofs/custody.
#[derive(Debug, Serialize)]
pub(crate) struct CustodyProofResponse {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<CustodyProofDetail>,
}

/// A single custody proof detail with structured billing fields.
#[derive(Debug, Serialize)]
pub(crate) struct CustodyProofDetail {
    pub txid: String,
    pub hash: String,
    pub timestamp: String,
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    // Structured fields parsed from proof data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_sats: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_chain_length: Option<usize>,
    pub verified: bool,
}

/// Parse a value from a "KEY: value" line in proof data.
fn parse_proof_field<'a>(data: &'a str, prefix: &str) -> Option<&'a str> {
    data.lines()
        .find(|line| line.starts_with(prefix))
        .map(|line| line[prefix.len()..].trim())
}

// -- Handlers --

/// GET /task/{id}/proofs -- proof details from transcript.
pub(crate) async fn get_proofs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/proofs"),
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

    // Collect all on-chain records, tagging each as BRC-48 checkpoint or BRC-18 proof.
    let all: Vec<(bool, ProofDetail)> = raw_events
        .iter()
        .filter(|e| e.event_type == "proof_created" || e.event_type == "checkpoint_created")
        .filter_map(|e| {
            let is_checkpoint = e.event_type == "checkpoint_created";
            let txid = e.data.get("txid").and_then(|v| v.as_str())?.to_string();
            let proof_type = e
                .data
                .get("proof_type")
                .or_else(|| e.data.get("token_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let hash = e
                .data
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let proof_data = e
                .data
                .get("proof_data")
                .and_then(|v| v.as_str())
                .map(String::from);
            let proof_timestamp = e
                .data
                .get("proof_timestamp")
                .and_then(|v| v.as_str())
                .map(String::from);
            let checkpoint_data = e.data.get("checkpoint_data").cloned();
            let sats_cost = e.data.get("sats_cost").and_then(|v| v.as_u64());
            let basket = e
                .data
                .get("basket")
                .and_then(|v| v.as_str())
                .map(String::from);
            let iteration = e
                .data
                .get("iteration")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let prev_hash = e
                .data
                .get("prev_hash")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some((
                is_checkpoint,
                ProofDetail {
                    txid,
                    proof_type,
                    hash,
                    timestamp: e.ts,
                    proof_data,
                    proof_timestamp,
                    checkpoint_data,
                    sats_cost,
                    basket,
                    iteration,
                    prev_hash,
                },
            ))
        })
        .collect();

    // Split: BRC-18 OP_RETURN proofs (hash-chained) vs BRC-48 state tokens (spendable UTXOs).
    let (cp_pairs, proof_pairs): (Vec<_>, Vec<_>) = all.into_iter().partition(|(is_cp, _)| *is_cp);
    let mut proofs: Vec<ProofDetail> = proof_pairs.into_iter().map(|(_, d)| d).collect();
    let mut checkpoints: Vec<ProofDetail> = cp_pairs.into_iter().map(|(_, d)| d).collect();

    // Sort both by timestamp
    proofs.sort_by(|a, b| {
        a.timestamp
            .partial_cmp(&b.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    checkpoints.sort_by(|a, b| {
        a.timestamp
            .partial_cmp(&b.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let body = ProofsResponse {
        task_id: id,
        proofs,
        checkpoints,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /task/{id}/proofs/verify -- on-chain verification via wallet.
pub(crate) async fn verify_proofs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/proofs/verify"),
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

    // Collect proof events from transcript
    let proof_events: Vec<_> = raw_events
        .iter()
        .filter(|e| e.event_type == "proof_created")
        .collect();

    // Fetch all outputs from the worm-proofs basket
    let mut on_chain_map: HashMap<String, String> = HashMap::new(); // txid -> hash_hex

    // Paginate through all proof outputs
    let mut offset = 0u64;
    let limit = 100u64;
    loop {
        match state
            .wallet
            .list_outputs("dm-proofs", "locking scripts", limit, offset)
            .await
        {
            Ok(result) => {
                let outputs = result.get("outputs").and_then(|v| v.as_array());
                let arr = match outputs {
                    Some(a) => a,
                    None => break,
                };
                if arr.is_empty() {
                    break;
                }
                for output in arr {
                    // Wallet returns "outpoint" as "txid.vout", not a separate "txid" field
                    let txid = output
                        .get("outpoint")
                        .and_then(|v| v.as_str())
                        .and_then(|op| op.split('.').next())
                        .or_else(|| output.get("txid").and_then(|v| v.as_str()))
                        .unwrap_or_default();
                    // Try locking_script directly, or nested in script_pubkey.hex
                    let script_hex = output
                        .get("lockingScript")
                        .and_then(|v| v.as_str())
                        .or_else(|| output.get("locking_script").and_then(|v| v.as_str()))
                        .or_else(|| {
                            output
                                .get("script_pubkey")
                                .and_then(|v| v.get("hex"))
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or_default();

                    // Parse OP_RETURN: 006a20<64 hex chars>
                    if script_hex.len() >= 70 && script_hex.starts_with("006a20") {
                        let hash_hex = &script_hex[6..70];
                        on_chain_map.insert(txid.to_string(), hash_hex.to_string());
                    }
                }
                if arr.len() < limit as usize {
                    break;
                }
                offset += limit;
            }
            Err(e) => {
                tracing::warn!("Failed to fetch proof outputs from wallet: {e}");
                break;
            }
        }
    }

    // Build verification results
    let verifications: Vec<ProofVerification> = proof_events
        .iter()
        .filter_map(|e| {
            let txid = e.data.get("txid").and_then(|v| v.as_str())?.to_string();
            let transcript_hash = e
                .data
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            // Hash recompute (if proof_data + proof_timestamp are available)
            let (hash_recompute, recomputed_hash) = if let (Some(data), Some(ts)) = (
                e.data.get("proof_data").and_then(|v| v.as_str()),
                e.data.get("proof_timestamp").and_then(|v| v.as_str()),
            ) {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                // Include prev_hash in hash computation (matches proofs.rs compute_proof_hash)
                if let Some(ph) = e.data.get("prev_hash").and_then(|v| v.as_str()) {
                    if let Ok(bytes) = hex::decode(ph) {
                        hasher.update(&bytes);
                    }
                }
                hasher.update(data.as_bytes());
                hasher.update(ts.as_bytes());
                let computed = hex::encode(hasher.finalize());
                let status = if computed == transcript_hash {
                    "match"
                } else {
                    "mismatch"
                };
                (status.to_string(), Some(computed))
            } else {
                ("unavailable".to_string(), None)
            };

            // On-chain match
            let (on_chain_match, on_chain_hash) = match on_chain_map.get(&txid) {
                Some(chain_hash) => {
                    let status = if *chain_hash == transcript_hash {
                        "match"
                    } else {
                        "mismatch"
                    };
                    (status.to_string(), Some(chain_hash.clone()))
                }
                None => ("not_found".to_string(), None),
            };

            let proof_type = e
                .data
                .get("proof_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let iteration = e
                .data
                .get("iteration")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            Some(ProofVerification {
                txid,
                hash_recompute,
                on_chain_match,
                on_chain_hash,
                recomputed_hash,
                proof_type,
                iteration,
            })
        })
        .collect();

    let body = VerifyResponse {
        task_id: id,
        verifications,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /task/{id}/proofs/custody — returns the custody proof for billing verification.
pub(crate) async fn get_custody_proof(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/proofs/custody"),
        None,
        &headers,
        None,
    )
    .await?;

    let transcript_path = state
        .workspace
        .join("tasks")
        .join(&id)
        .join("session.jsonl");
    if !transcript_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let transcript = crate::transcript::Transcript::new(transcript_path);
    let events = transcript.get_events_by_type("proof_created");

    // Find the custody proof event
    let custody_event = events.iter().find(|e| {
        let pt = e
            .data
            .get("proof_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        pt == "custody" || pt == "Custody"
    });

    let proof = custody_event.map(|event| {
        let txid = event
            .data
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let hash = event
            .data
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let timestamp = event
            .data
            .get("proof_timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let data = event
            .data
            .get("proof_data")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let prev_hash = event
            .data
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Parse structured fields from proof data
        let customer_key = parse_proof_field(&data, "CUSTOMER_KEY: ").map(String::from);
        let task_hash = parse_proof_field(&data, "TASK_HASH: ").map(String::from);
        let iterations = parse_proof_field(&data, "ITERATIONS: ").and_then(|v| v.parse().ok());
        let duration_secs =
            parse_proof_field(&data, "DURATION_SECS: ").and_then(|v| v.parse().ok());
        let total_sats = parse_proof_field(&data, "TOTAL_SATS: ").and_then(|v| v.parse().ok());
        let result_hash = parse_proof_field(&data, "RESULT_HASH: ").map(String::from);
        let agent_key = parse_proof_field(&data, "AGENT_KEY: ").map(String::from);
        let proof_chain_length =
            parse_proof_field(&data, "PROOF_CHAIN_LENGTH: ").and_then(|v| v.parse().ok());

        // Verify the proof hash is internally consistent
        let verified = if !hash.is_empty() && !data.is_empty() && !timestamp.is_empty() {
            let commitment = crate::proofs::ProofCommitment::with_timestamp(
                crate::proofs::ProofType::Custody,
                &data,
                &timestamp,
                prev_hash.as_deref(),
            );
            commitment.hash_hex() == hash
        } else {
            false
        };

        CustodyProofDetail {
            txid,
            hash,
            timestamp,
            data,
            prev_hash,
            customer_key,
            task_hash,
            iterations,
            duration_secs,
            total_sats,
            result_hash,
            agent_key,
            proof_chain_length,
            verified,
        }
    });

    let response = CustodyProofResponse { task_id: id, proof };

    signed_json_response(&state, auth_ctx, StatusCode::OK, &response).await
}

/// GET /task/{id}/receipts -- BEEF payment receipts for a task.
pub(crate) async fn get_receipts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/task/{id}/receipts"),
        None,
        &headers,
        None,
    )
    .await?;
    let beef_dir = state.workspace.join(format!("tasks/{id}/beef"));

    let mut receipts: Vec<ReceiptDetail> = Vec::new();

    if beef_dir.exists() {
        let entries =
            std::fs::read_dir(&beef_dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let parsed: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };
            receipts.push(ReceiptDetail {
                iteration: parsed
                    .get("iteration")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                model: parsed
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                sats_paid: parsed
                    .get("sats_paid")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                sats_effective: parsed
                    .get("sats_effective")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                sats_refunded: parsed
                    .get("sats_refunded")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                tokens: parsed.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                timestamp: parsed
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }

    // Sort by iteration
    receipts.sort_by_key(|r| r.iteration);

    let total_sats_paid: u64 = receipts.iter().map(|r| r.sats_paid).sum();
    let total_sats_refunded: u64 = receipts.iter().map(|r| r.sats_refunded).sum();

    let body = ReceiptsResponse {
        task_id: id,
        receipts,
        total_sats_paid,
        total_sats_refunded,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}
