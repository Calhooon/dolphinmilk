//! Agent, health, and certificate route handlers.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;

use crate::wallet::HttpWalletClient; // only for parent wallet in issue_certificate

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::types::*;
use super::super::AppState;

// -- Local types --

#[derive(Debug, Deserialize)]
pub(crate) struct RelinquishRequest {
    #[serde(default)]
    serial_number: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct IssueCertRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    capabilities: Option<String>,
    #[serde(default)]
    budget_per_task: Option<u64>,
    #[serde(default)]
    budget_per_hour: Option<u64>,
    #[serde(default)]
    budget_per_day: Option<u64>,
    #[serde(default)]
    budget_per_week: Option<u64>,
    #[serde(default)]
    budget_per_month: Option<u64>,
    #[serde(default)]
    budget_lifetime: Option<u64>,
    #[serde(default)]
    budget_enforcement: Option<String>,
}

// -- Handlers --

pub(crate) async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let last_tick = state.scheduler.last_scheduler_tick.load(Ordering::Relaxed);
    let scheduler_last_tick_secs_ago = if last_tick == 0 {
        None
    } else {
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(now_epoch.saturating_sub(last_tick))
    };

    // Non-blocking wallet connectivity check with 2s timeout.
    // Does NOT affect the HTTP status code — always returns 200.
    let wallet_connected = {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state.wallet.get_identity_key(),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
    };

    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        uptime_secs: state.started_at.elapsed().as_secs(),
        scheduler_last_tick_secs_ago,
        wallet_connected,
        wallet_url: Some(state.config.wallet.url.clone()),
    })
}

/// GET /agent -- agent identity and stats.
pub(crate) async fn get_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/agent", None, &headers, None).await?;
    // Identity key: use server_identity_key if available, else try wallet
    let identity_key = if !state.auth.server_identity_key.is_empty() {
        state.auth.server_identity_key.clone()
    } else {
        state.wallet.get_identity_key().await.unwrap_or_default()
    };

    // Balance: use cached value with 30s TTL, fall back to fresh wallet call
    let balance = {
        const BALANCE_TTL_SECS: u64 = 30;
        let cached = {
            let guard = state.cached_balance.read().await;
            guard.as_ref().and_then(|(bal, ts)| {
                if ts.elapsed().as_secs() < BALANCE_TTL_SECS {
                    Some(*bal)
                } else {
                    None
                }
            })
        };
        if let Some(bal) = cached {
            bal
        } else {
            let bal = state.wallet.get_balance().await.unwrap_or(0);
            let mut guard = state.cached_balance.write().await;
            *guard = Some((bal, std::time::Instant::now()));
            bal
        }
    };

    // Lifetime task stats (includes historical tasks from disk + current session)
    let total_tasks = state.stats.task_count.load(Ordering::Relaxed);
    let total_sats_spent = state.stats.sats_spent.load(Ordering::Relaxed);

    // Use cached tool names from startup — avoids rebuilding the registry per request.
    let tools = state.cached_tool_names.clone();

    // Certificate status and revocation check (best-effort)
    let (certificate_status, certificate_revoked) = {
        let mgr = crate::certificates::CertificateManager::new(state.wallet.clone());
        match mgr.certificate_status().await.ok() {
            Some(status) => {
                let revoked = if let Some(ref cert) = status.certificate {
                    mgr.is_revoked(cert).await.ok()
                } else {
                    None
                };
                (Some(status.status), revoked)
            }
            None => (None, None),
        }
    };

    // Build available models list from discovered capabilities or hardcoded fallback.
    let available_models: Vec<super::super::types::ModelInfo> = {
        let caps = state.model_capabilities.read().await;
        if caps.is_empty() {
            // Fallback: hardcoded list for all known models
            [
                "gpt-5-nano",
                "gpt-5-mini",
                "gpt-5",
                "gpt-5.2",
                "o4-mini",
                "gpt-5.2-pro",
                "claude-haiku-4-5",
                "claude-sonnet-4-6",
                "claude-opus-4-6",
            ]
            .iter()
            .map(|id| {
                let input = crate::think::model_input_limit(id);
                let output = crate::think::model_output_limit(id);
                super::super::types::ModelInfo {
                    id: id.to_string(),
                    provider: if id.starts_with("claude") {
                        "claude".into()
                    } else {
                        "openai".into()
                    },
                    context_window: input + output,
                    max_output_tokens: output,
                    max_input_tokens: input,
                    supports_tools: true,
                    supports_vision: true,
                }
            })
            .collect()
        } else {
            caps.values()
                .map(|c| super::super::types::ModelInfo {
                    id: c.model.clone(),
                    provider: if c.model.starts_with("claude") {
                        "claude".into()
                    } else {
                        "openai".into()
                    },
                    context_window: c.context_window,
                    max_output_tokens: c.max_output_tokens,
                    max_input_tokens: c.max_input_tokens,
                    supports_tools: c.supports_tools,
                    supports_vision: c.supports_vision,
                })
                .collect()
        }
    };

    let body = AgentResponse {
        identity_key,
        balance,
        version: env!("CARGO_PKG_VERSION").into(),
        uptime_secs: state.started_at.elapsed().as_secs(),
        total_tasks,
        total_sats_spent,
        tools,
        certificate_status,
        certificate_revoked,
        default_model: state.config.llm.default_model.clone(),
        available_models,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /certificates -- current certificate status and details.
pub(crate) async fn get_certificates(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(&state, "GET", "/certificates", None, &headers, None).await?;
    let mgr = crate::certificates::CertificateManager::new(state.wallet.clone());

    let status = mgr.certificate_status().await.map_err(|e| {
        tracing::error!("Failed to get certificate status: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Enrich with revocation status (best-effort)
    let is_revoked = if let Some(ref cert) = status.certificate {
        mgr.is_revoked(cert).await.ok()
    } else {
        None
    };

    let body = serde_json::json!({
        "status": status.status,
        "valid": status.status != "none",
        "certificate": status.certificate,
        "identity_key": status.identity_key,
        "is_revoked": is_revoked,
    });

    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// POST /certificates/issue -- trigger parent-signed certificate acquisition.
pub(crate) async fn issue_certificate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, StatusCode> {
    // NOTE: body excluded from signature verification. The BRC-104 signing flow
    // in the JS test client and most handlers use body=None because axum consumes
    // the body before auth can verify it. Keep consistent across all handlers.
    let auth_ctx =
        check_brc31_auth(&state, "POST", "/certificates/issue", None, &headers, None).await?;

    let req: IssueCertRequest = if body.is_empty() {
        IssueCertRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => {
                let body = serde_json::json!({"error": "Invalid JSON body"});
                return signed_json_response(
                    &state,
                    auth_ctx,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &body,
                )
                .await;
            }
        }
    };
    let agent_name_owned = req.name.clone();
    let agent_name = agent_name_owned
        .as_deref()
        .unwrap_or(&state.config.certificates.agent_name);
    let caps_string = req.capabilities.clone().unwrap_or_else(|| {
        "llm,tools,wallet,memory,messaging,x402,schedule,orchestration".to_string()
    });
    let capabilities: Vec<&str> = caps_string.split(',').map(|s| s.trim()).collect();

    let mgr = crate::certificates::CertificateManager::new(state.wallet.clone());

    let parent_url = &state.config.parent.wallet_url;
    if parent_url.is_empty() {
        let body = serde_json::json!({"error": "No parent wallet configured"});
        return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
    }

    let parent_wallet = HttpWalletClient::new(parent_url, "http://localhost", 10);

    tracing::info!("BRC-52: issuing parent-signed certificate for agent '{agent_name}' with capabilities '{caps_string}'");

    // Relinquish any existing SELF-SIGNED agent-auth certs first to avoid UNIQUE
    // constraint errors. Parent-signed certs (certifier != subject) are preserved
    // so the agent retains its previous authorization if the new acquisition fails.
    if let Ok(certs) = mgr.list_all().await {
        for entry in &certs {
            let c = entry.get("certificate").unwrap_or(entry);
            let ct = c
                .get("certificateType")
                .or_else(|| c.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if ct == crate::certificates::CERT_TYPE_AGENT_AUTH {
                let subject = c.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                let certifier = c.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
                let is_self_signed =
                    certifier.is_empty() || subject.is_empty() || certifier == subject;
                if is_self_signed {
                    let _ = mgr.relinquish(c).await;
                } else {
                    tracing::info!(
                        "BRC-52: preserving parent-signed cert (certifier={}, subject={})",
                        &certifier[..8.min(certifier.len())],
                        &subject[..8.min(subject.len())],
                    );
                }
            }
        }
    }

    match mgr
        .acquire_parent_authorization(
            &parent_wallet,
            agent_name,
            &capabilities,
            req.budget_per_task,
            req.budget_per_hour,
            req.budget_per_day,
            req.budget_per_week,
            req.budget_per_month,
            req.budget_lifetime,
        )
        .await
    {
        Ok(cert) => {
            // Clear revocation flag — agent is re-authorized
            state
                .certificate_revoked
                .store(false, std::sync::atomic::Ordering::SeqCst);

            // Sync new cert's budget limits to the shared budget tracker so
            // GET /budget reflects the updated cert-enforced limits.
            {
                let mut global_budget = state.budget.lock().await;
                global_budget.apply_cert_limits(
                    req.budget_per_task,
                    req.budget_per_hour,
                    req.budget_per_day,
                    req.budget_per_week,
                    req.budget_per_month,
                    req.budget_lifetime,
                    req.budget_enforcement.clone(),
                );
            }

            let relinquished = 0usize; // already relinquished above
            let status = mgr.certificate_status().await.map_err(|e| {
                tracing::error!("Failed to get cert status after issue: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            // Extract revocation UTXO txid from the fresh certificate status
            let txid = status
                .certificate
                .as_ref()
                .and_then(|c| c.get("revocationOutpoint"))
                .and_then(|v| v.as_str())
                .filter(|s| s.len() >= 64)
                .map(|s| &s[..64]);

            // Staleness-aware overlay re-registration (best-effort).
            //
            // The agent registers on the overlay exactly once at boot from
            // whatever cert it had at that moment. A later cert issuance with
            // different capabilities would leave the overlay pointing at the
            // stale default-capabilities registration — making
            // overlay_lookup(findByCapability: ...) return empty for the new
            // role. `reregister_on_overlay` senses that staleness, spends the
            // old UTXOs, and re-publishes with the current cert's fields.
            //
            // Errors here are warnings — the cert is still issued. Tests that
            // need the overlay update can call POST /overlay/reregister
            // explicitly and observe the structured result.
            let mut reregistration_result: Option<crate::overlay::ReregistrationResult> = None;
            if state.config.overlay.enabled {
                // Resolve the certifier the same way boot-time registration
                // does: prefer the cert's certifier when it's truly
                // parent-signed (certifier != subject), otherwise fall back
                // to the configured parent identity key (trust-on-claim).
                let cert_subject = cert.get("subject").and_then(|v| v.as_str());
                let cert_certifier = cert.get("certifier").and_then(|v| v.as_str());
                let is_parent_signed = matches!(
                    (cert_subject, cert_certifier),
                    (Some(s), Some(c)) if s != c
                );
                let config_parent_key = state.config.parent.identity_key.clone();
                let certifier_owned: Option<String> = if is_parent_signed {
                    cert_certifier.map(|s| s.to_string())
                } else if !config_parent_key.is_empty() {
                    Some(config_parent_key)
                } else {
                    None
                };

                match crate::overlay::reregister_on_overlay(
                    &*state.wallet,
                    &state.config.overlay.submit_url,
                    agent_name,
                    &caps_string,
                    certifier_owned.as_deref(),
                )
                .await
                {
                    Ok(r) => {
                        tracing::info!(
                            stale_found = r.stale_found,
                            stale_spent = r.stale_spent,
                            fresh = r.skipped_because_already_fresh,
                            new_txid = ?r.new_registration_txid,
                            "overlay re-registration after cert issuance"
                        );
                        reregistration_result = Some(r);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "overlay re-registration failed after cert issuance: {e} \
                             (cert was still issued successfully)"
                        );
                    }
                }
            }

            let body = serde_json::json!({
                "issued": true,
                "relinquished_self_signed": relinquished,
                "certificate": cert,
                "status": status,
                "txid": txid,
                "overlay_reregistration": reregistration_result,
            });
            signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
        }
        Err(e) => {
            let body =
                serde_json::json!({"error": format!("Failed to issue parent certificate: {e}")});
            signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body).await
        }
    }
}

/// POST /certificates/revoke -- parent revokes the agent's certificate.
///
/// BRC-31 auth required. Only the parent (config.parent.identity_key) can call this.
/// Spends the revocation UTXO in the `worm-revocation` basket, which signals
/// that the certificate is revoked. The agent will detect this on next boot
/// or periodic heartbeat check.
pub(crate) async fn revoke_certificate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        "/certificates/revoke",
        None,
        &headers,
        Some(&body),
    )
    .await?;

    // Revocation requires BRC-31 auth (parent only). In dev mode, allow it.
    let wallet = state.wallet.clone();
    let mgr = crate::certificates::CertificateManager::new(wallet.clone());

    // Find the parent-signed certificate
    let status = match mgr.certificate_status().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to get certificate status for revocation: {e}");
            let body =
                serde_json::json!({"error": format!("Failed to get certificate status: {e}")});
            return signed_json_response(
                &state,
                auth_ctx,
                StatusCode::INTERNAL_SERVER_ERROR,
                &body,
            )
            .await;
        }
    };

    let cert = match &status.certificate {
        Some(c) if status.status == "parent-signed" => c.clone(),
        _ => {
            let body = serde_json::json!({"error": "No parent-signed certificate to revoke"});
            return signed_json_response(&state, auth_ctx, StatusCode::NOT_FOUND, &body).await;
        }
    };

    // Find the revocation outpoint — tries cert field, then basket scan
    match mgr.find_revocation_outpoint(&cert).await {
        Some((txid, vout)) => {
            // Two-step revocation:
            // 1. Relinquish the revocation UTXO from the basket (is_revoked() checks basket)
            // 2. Create an on-chain BRC-18 proof recording the revocation (verifiable txid)
            //
            // We don't spend the PushDrop UTXO directly because the wallet's createAction
            // requires a two-step signing flow for non-P2PKH scripts. The relinquish +
            // proof approach is simpler and gives us a verifiable on-chain record.

            // Step 1: Remove from basket — try worm-revocation first, fall back to worm-state
            let relinquish_result = wallet
                .relinquish_output(crate::onchain::state::BASKET_REVOCATION, &txid, vout)
                .await;
            if relinquish_result.is_err() {
                let _ = wallet
                    .relinquish_output(crate::onchain::state::BASKET_STATE, &txid, vout)
                    .await;
            }

            // Immediately block new tasks
            state
                .certificate_revoked
                .store(true, std::sync::atomic::Ordering::SeqCst);

            // Step 2: Create an on-chain BRC-18 proof recording the revocation
            let serial = cert
                .get("serialNumber")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let subject = cert
                .get("subject")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let certifier = cert
                .get("certifier")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let proof_data = format!(
                "REVOCATION: {}\nSUBJECT: {}\nCERTIFIER: {}\nREVOCATION_OUTPOINT: {}:{}",
                serial, subject, certifier, txid, vout
            );
            let commitment = crate::onchain::proofs::ProofCommitment::new(
                crate::onchain::proofs::ProofType::CertificateRevocation,
                &proof_data,
                None,
            );

            match crate::onchain::proofs::create_proof(&*wallet, commitment).await {
                Ok(proof_result) => {
                    let proof_txid = &proof_result.txid;
                    tracing::warn!(
                        "BRC-52: certificate REVOKED — revocation UTXO {}:{} relinquished, proof tx {}",
                        &txid[..16], vout, &proof_txid[..16]
                    );
                    {
                        let mut budget = state.budget.lock().await;
                        budget.record(
                            "certificates",
                            "revoke",
                            0,
                            serde_json::json!({
                                "serial_number": serial,
                                "subject": subject,
                                "revocation_txid": txid,
                                "proof_txid": proof_txid,
                            }),
                        );
                    }
                    let outpoint_str = format!("{}{:08x}", txid, vout);
                    let body = serde_json::json!({
                        "revoked": true,
                        "revocation_outpoint": outpoint_str,
                        "txid": proof_txid,
                    });
                    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
                }
                Err(e) => {
                    // Relinquish already succeeded, so the cert IS revoked even if proof fails.
                    // Return success with a warning.
                    tracing::error!(
                        "BRC-52: revocation UTXO relinquished but proof creation failed: {e}"
                    );
                    let outpoint_str = format!("{}{:08x}", txid, vout);
                    let body = serde_json::json!({
                        "revoked": true,
                        "revocation_outpoint": outpoint_str,
                        "warning": format!("Revocation succeeded but on-chain proof failed: {e}"),
                    });
                    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
                }
            }
        }
        None => {
            // No revocation UTXO — legacy cert issued before revocation was implemented.
            // Fall back to relinquishing the certificate directly from the wallet.
            tracing::warn!(
                "BRC-52: no revocation UTXO found for cert {:?}, falling back to relinquish",
                cert.get("serialNumber")
            );
            match mgr.relinquish(&cert).await {
                Ok(_) => {
                    // Block new tasks
                    state
                        .certificate_revoked
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    {
                        let mut budget = state.budget.lock().await;
                        budget.record(
                            "certificates",
                            "revoke",
                            0,
                            serde_json::json!({
                                "serial_number": cert.get("serialNumber"),
                                "subject": cert.get("subject"),
                                "method": "relinquish_fallback",
                            }),
                        );
                    }
                    let body = serde_json::json!({
                        "revoked": true,
                        "method": "relinquish_fallback",
                    });
                    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
                }
                Err(e) => {
                    tracing::error!("Failed to relinquish certificate as revocation fallback: {e}");
                    let body = serde_json::json!({
                        "error": format!("Failed to revoke certificate: {e}")
                    });
                    signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body)
                        .await
                }
            }
        }
    }
}

/// POST /certificates/relinquish -- relinquish the current certificate.
pub(crate) async fn relinquish_certificate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        "/certificates/relinquish",
        None,
        &headers,
        Some(&body),
    )
    .await?;

    let req: RelinquishRequest = if body.is_empty() {
        RelinquishRequest {
            serial_number: None,
        }
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => {
                let body = serde_json::json!({"error": "Invalid JSON body"});
                return signed_json_response(
                    &state,
                    auth_ctx,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &body,
                )
                .await;
            }
        }
    };

    let mgr = crate::certificates::CertificateManager::new(state.wallet.clone());

    let certs = match mgr.list_all().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to list certs for relinquish: {e}");
            let body = serde_json::json!({"error": format!("Failed to list certificates: {e}")});
            return signed_json_response(
                &state,
                auth_ctx,
                StatusCode::INTERNAL_SERVER_ERROR,
                &body,
            )
            .await;
        }
    };

    // Find matching cert by serial_number, or take the first auth cert
    let target = if let Some(ref serial) = req.serial_number {
        certs.iter().find(|entry| {
            let c = entry.get("certificate").unwrap_or(entry);
            c.get("serialNumber").and_then(|v| v.as_str()) == Some(serial.as_str())
        })
    } else {
        // Relinquish first agent-authorization cert
        certs.iter().find(|entry| {
            let c = entry.get("certificate").unwrap_or(entry);
            let ct = c
                .get("certificateType")
                .or_else(|| c.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            ct == crate::certificates::CERT_TYPE_AGENT_AUTH
        })
    };

    let Some(cert_entry) = target else {
        let body = serde_json::json!({"error": "No matching certificate found"});
        return signed_json_response(&state, auth_ctx, StatusCode::NOT_FOUND, &body).await;
    };

    let cert = cert_entry.get("certificate").unwrap_or(cert_entry);

    // Guard: cannot relinquish a parent-issued certificate that has a
    // revocation mechanism. The parent should revoke it instead.
    //
    // However, legacy certs issued before revocation was implemented have
    // a null outpoint — those CAN be relinquished since there's no other
    // way to remove them.
    let subject = cert.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let certifier = cert.get("certifier").and_then(|v| v.as_str()).unwrap_or("");

    let has_revocation_outpoint = cert
        .get("revocationOutpoint")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty() && !crate::certificates::is_null_revocation_outpoint(s))
        .unwrap_or(false);

    let is_parent_issued = if !certifier.is_empty() && !subject.is_empty() {
        certifier != subject
    } else if !certifier.is_empty() && subject.is_empty() {
        true
    } else {
        has_revocation_outpoint
    };

    if is_parent_issued && has_revocation_outpoint {
        // Allow relinquish if the certificate has already been revoked —
        // the revocation UTXO is spent, so the guard no longer applies and
        // relinquish is the only way to clean up the cert from the wallet.
        let already_revoked = mgr.is_revoked(cert).await.unwrap_or(false);
        if !already_revoked {
            let body = serde_json::json!({
                "error": "Cannot relinquish a parent-issued certificate with a revocation mechanism. Use POST /certificates/revoke instead."
            });
            return signed_json_response(&state, auth_ctx, StatusCode::FORBIDDEN, &body).await;
        }
    }

    if let Err(e) = mgr.relinquish(cert).await {
        tracing::error!("Failed to relinquish certificate: {e}");
        let body = serde_json::json!({"error": format!("Failed to relinquish certificate: {e}")});
        return signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body)
            .await;
    }

    let status = match mgr.certificate_status().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to get cert status after relinquish: {e}");
            let body = serde_json::json!({"error": format!("Relinquished but failed to refresh status: {e}")});
            return signed_json_response(
                &state,
                auth_ctx,
                StatusCode::INTERNAL_SERVER_ERROR,
                &body,
            )
            .await;
        }
    };

    let body = serde_json::json!({
        "relinquished": true,
        "status": status,
    });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// POST /overlay/reregister -- force staleness-aware overlay re-registration.
///
/// This endpoint is for test tooling (cluster.js Phase 2's
/// `verifyOverlayRegistrations`) and operators who need to force a refresh
/// without issuing a new cert. It reads the agent's current cert, resolves
/// name + capabilities + certifier the same way boot-time registration does,
/// then calls [`crate::overlay::reregister_on_overlay`].
///
/// Idempotent: if the existing overlay record already matches the target
/// state, returns `skipped_because_already_fresh: true` with no spending.
///
/// Requires BRC-31 authentication (same pattern as `/certificates/issue`).
pub(crate) async fn reregister_overlay_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "POST", "/overlay/reregister", None, &headers, None).await?;

    if !state.config.overlay.enabled {
        let body = serde_json::json!({
            "error": "Overlay integration is disabled (config.overlay.enabled = false)"
        });
        return signed_json_response(&state, auth_ctx, StatusCode::BAD_REQUEST, &body).await;
    }

    // Resolve current cert to pull name + capabilities + certifier.
    let mgr = crate::certificates::CertificateManager::new(state.wallet.clone());
    let status = match mgr.certificate_status().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("reregister_overlay_handler: cert status failed: {e}");
            let body = serde_json::json!({
                "error": format!("Failed to read certificate status: {e}")
            });
            return signed_json_response(
                &state,
                auth_ctx,
                StatusCode::INTERNAL_SERVER_ERROR,
                &body,
            )
            .await;
        }
    };

    // Capabilities come from the cert's fields.capabilities string.
    // Falls back to the hardcoded default if no cert is present (matches
    // the boot-time behaviour in `app_state.rs`).
    let config_agent_name = state.config.certificates.agent_name.clone();
    let cert = status.certificate.as_ref();
    let capabilities = cert
        .and_then(|c| c.get("fields"))
        .and_then(|f| f.get("capabilities"))
        .and_then(|v| v.as_str())
        .unwrap_or("llm,tools,wallet,memory,messaging,x402,schedule,orchestration")
        .to_string();

    // Certifier resolution mirrors app_state.rs::create_app_state Step 10:
    //   1. parent-signed cert → use cert's certifier
    //   2. config.parent.identity_key set → use it (trust-on-claim)
    //   3. fall back to self-loop (None → register_on_overlay uses identity)
    let cert_subject = cert.and_then(|c| c.get("subject")).and_then(|v| v.as_str());
    let cert_certifier = cert
        .and_then(|c| c.get("certifier"))
        .and_then(|v| v.as_str());
    let is_parent_signed = matches!(
        (cert_subject, cert_certifier),
        (Some(s), Some(c)) if s != c
    );
    let config_parent_key = state.config.parent.identity_key.clone();
    let certifier_owned: Option<String> = if is_parent_signed {
        cert_certifier.map(|s| s.to_string())
    } else if !config_parent_key.is_empty() {
        Some(config_parent_key)
    } else {
        None
    };

    match crate::overlay::reregister_on_overlay(
        &*state.wallet,
        &state.config.overlay.submit_url,
        &config_agent_name,
        &capabilities,
        certifier_owned.as_deref(),
    )
    .await
    {
        Ok(result) => {
            tracing::info!(
                stale_found = result.stale_found,
                stale_spent = result.stale_spent,
                fresh = result.skipped_because_already_fresh,
                "POST /overlay/reregister: complete"
            );
            signed_json_response(&state, auth_ctx, StatusCode::OK, &result).await
        }
        Err(e) => {
            tracing::error!("POST /overlay/reregister: {e}");
            let body = serde_json::json!({
                "error": format!("Overlay re-registration failed: {e}")
            });
            signed_json_response(&state, auth_ctx, StatusCode::INTERNAL_SERVER_ERROR, &body).await
        }
    }
}

/// GET /analytics/skills -- skill usage telemetry.
pub(crate) async fn get_skill_analytics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/analytics/skills", None, &headers, None).await?;

    let telemetry = state.skill_telemetry.lock().await;

    let skills: Vec<serde_json::Value> = state
        .skill_definitions
        .iter()
        .map(|(name, _desc, auto_activate)| {
            let stats = telemetry.activations.get(name);
            serde_json::json!({
                "name": name,
                "activations": stats.map(|s| s.count).unwrap_or(0),
                "last_activated": stats.and_then(|s| s.last_activated.clone()),
                "auto_activate": auto_activate,
                "contexts": stats.map(|s| &s.contexts).cloned().unwrap_or_default(),
            })
        })
        .collect();

    let body = serde_json::json!({ "skills": skills });
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}
