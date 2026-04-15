//! Task lifecycle — setup, main loop, teardown.
//!
//! High-level orchestration of the agent loop. `run()` composes
//! setup_task → run_loop → teardown_task. Each phase is a separate
//! method for testability.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::Value;
use sha2::Digest;
use tokio::sync::broadcast;

use crate::error::DmError;
use crate::events::StepEvent;
use crate::memory::session::SessionSummarizer;
use crate::proofs;
use crate::sanitize;
use crate::state;

use super::step::StepContext;
use super::{content_hash, emit_event, DmLoop};

// =============================================================================
// Setup
// =============================================================================

impl DmLoop {
    /// Setup phase: reset state, apply allowlists, record session start,
    /// bootstrap identity, create BRC-48 tokens.
    pub(crate) async fn setup_task(&mut self, task: &str, ctx: &StepContext) {
        // Preserve flags set before run() (via set_external_origin, set_conversation_chain_status, set_pending_delegation)
        let external_origin = self.state.auth.external_origin;
        let conversation_chain_verified = self.state.onchain.conversation_chain_verified.take();
        let conversation_chain_break = self.state.onchain.conversation_chain_break.take();
        let pending_delegation_envelope = self.state.storage.pending_delegation_envelope.take();
        self.state = super::LoopState {
            storage: super::StorageState {
                task: task.to_string(),
                task_id: ctx.task_id.clone(),
                pending_delegation_envelope,
                ..Default::default()
            },
            auth: super::AuthState {
                external_origin,
                ..Default::default()
            },
            onchain: super::OnChainState {
                conversation_chain_verified,
                conversation_chain_break,
                ..Default::default()
            },
            ..Default::default()
        };

        // Layer 3: Tool allowlist for external-origin tasks.
        if self.state.auth.external_origin {
            let allowlist = sanitize::external_tool_allowlist();
            tracing::info!(
                "External-origin task: restricting tools to {} allowed tools",
                allowlist.len()
            );
            self.tools.write().await.set_allowlist(allowlist);
        }

        // EPIC #329 Phase 3: verify inbound delegation envelope (if any) and
        // apply its caveats. On success, the delegation's allowlist + budget
        // cap REPLACE the generic external_origin restrictions. On failure
        // (bad signature, expired, revoked, unknown root, etc.) the task
        // falls back to the external_origin allowlist already applied above
        // — graceful degradation, never blocks an existing flow.
        self.apply_pending_delegation(task).await;

        // Record session start
        let mut session_data = HashMap::new();
        session_data.insert("task".to_string(), Value::String(task.to_string()));
        self.transcript.record("session_start", session_data);
        self.transcript.record_user(task);

        tracing::info!("Starting agent loop: {}", task);

        // Fetch BRC-52 certificate info (cached for the loop)
        self.certificate_info = self.fetch_certificate_info().await;

        // Compute certificate hash for Decision proof enrichment (#198).
        if let Some(ref cert_info) = self.certificate_info {
            let cert_data = format!(
                "{}:{}:{}:{}",
                cert_info.certifier,
                cert_info.name,
                cert_info.capabilities,
                if cert_info.self_signed {
                    "self-signed"
                } else {
                    "parent-signed"
                }
            );
            let hash = sha2::Sha256::digest(cert_data.as_bytes());
            self.state.auth.cert_hash = Some(hex::encode(hash));
            self.state.auth.cert_type = Some(
                if cert_info.self_signed {
                    "self-signed"
                } else {
                    "parent-signed"
                }
                .to_string(),
            );
        }

        // Construct moderation engine from cert policy + config
        self.build_moderation_engine().await;

        // Phase 10.2: Parse capabilities from certificate for runtime enforcement.
        // When a certificate exists, parse its capabilities field as comma-separated.
        // When no certificate exists, default to all capabilities (backward compat).
        self.state.auth.capabilities = match &self.certificate_info {
            Some(ci) if ci.capabilities != "none" => {
                let caps: Vec<String> = ci
                    .capabilities
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                tracing::info!("Capability enforcement enabled: {:?}", caps);
                Some(caps)
            }
            _ => {
                // No certificate or capabilities="none" — default to unrestricted
                tracing::debug!("No certificate capabilities — defaulting to unrestricted");
                None
            }
        };

        // Apply certificate-derived budget limits if present.
        if let Some(ci) = &self.certificate_info {
            if ci.budget_per_task.is_some()
                || ci.budget_per_hour.is_some()
                || ci.budget_per_day.is_some()
                || ci.budget_per_week.is_some()
                || ci.budget_per_month.is_some()
                || ci.budget_lifetime.is_some()
                || ci.budget_enforcement.is_some()
            {
                self.budget_tracker.apply_cert_limits(
                    ci.budget_per_task,
                    ci.budget_per_hour,
                    ci.budget_per_day,
                    ci.budget_per_week,
                    ci.budget_per_month,
                    ci.budget_lifetime,
                    ci.budget_enforcement.clone(),
                );
                tracing::info!(
                    "Cert-derived budget limits applied: task={:?} hour={:?} day={:?} week={:?} month={:?} lifetime={:?}",
                    ci.budget_per_task, ci.budget_per_hour, ci.budget_per_day,
                    ci.budget_per_week, ci.budget_per_month, ci.budget_lifetime
                );
            }
        }

        // Apply certificate-driven tool approval requirements
        {
            let cert_approval =
                crate::certificates::read_cert_tool_approval(self.wallet.clone()).await;
            if let Some(cert_tools) = cert_approval.tools {
                self.merge_cert_approval_tools(cert_tools);
            }
        }

        // Identity bootstrap: seed from certificate if no identity entry exists
        if self.memory_store.find_identity_entry().is_none() {
            let identity_key = self.wallet.get_identity_key().await.unwrap_or_default();
            let (name, capabilities, certifier, deployed) = match &self.certificate_info {
                Some(ci) => (
                    ci.name.clone(),
                    ci.capabilities.clone(),
                    if ci.self_signed {
                        "self".to_string()
                    } else {
                        ci.certifier.clone()
                    },
                    chrono::Utc::now().to_rfc3339(),
                ),
                None => (
                    "unnamed".to_string(),
                    "none".to_string(),
                    "self".to_string(),
                    chrono::Utc::now().to_rfc3339(),
                ),
            };
            let content = format!(
                "# Agent Identity\n\n\
                 ## Core (from certificate)\n\
                 - **Name**: {name}\n\
                 - **Identity Key**: {identity_key}\n\
                 - **Capabilities**: {capabilities}\n\
                 - **Certifier**: {certifier}\n\
                 - **Deployed**: {deployed}\n\n\
                 ## Purpose\n\
                 Autonomous BSV agent. Purpose evolving through experience.\n\n\
                 ## Operational Notes\n\
                 No operational observations yet."
            );
            let entry = crate::memory::store::MemoryEntry::new(
                crate::memory::store::MemoryCategory::Knowledge,
                content,
                vec![crate::memory::store::IDENTITY_TAG.to_string()],
                "identity-bootstrap",
            );
            if let Err(e) = self.memory_store.store(&entry) {
                tracing::warn!("Failed to bootstrap identity entry: {e}");
            } else {
                if let Some(ref mut idx) = self.memory_index {
                    let _ = idx.add_entry(&entry);
                }
                tracing::info!("Identity bootstrapped from certificate");
            }
        }

        // R.3: BRC-60 hash chain verification at conversation-bound task start.
        // If spawn_task() verified the conversation and found a break, log a warning
        // and create a BRC-18 ConversationBreak proof to record the integrity violation.
        if let Some(false) = self.state.onchain.conversation_chain_verified {
            if let Some(ref chain_break) = self.state.onchain.conversation_chain_break {
                tracing::warn!(
                    "BRC-60 hash chain break detected in conversation {}: {} break(s), first at message {}",
                    chain_break.conversation_id,
                    chain_break.total_breaks,
                    chain_break.first_break_seq,
                );
                let commitment = proofs::chain_break_proof(
                    &chain_break.conversation_id,
                    &chain_break.expected_hash,
                    &chain_break.actual_hash,
                    chain_break.first_break_seq as usize,
                    self.state.onchain.last_proof_hash.as_deref(),
                );
                self.record_proof(commitment, "conversation_break", None)
                    .await;
            }
        }

        // Query basket UTXO counts for vital signs
        let baskets = [
            state::BASKET_STATE,
            state::BASKET_BUDGET,
            state::BASKET_PROOFS,
            state::BASKET_REVOCATION,
        ];
        for basket_name in &baskets {
            let (count, _oldest) = state::basket_status(&self.wallet, basket_name).await;
            self.state
                .onchain
                .basket_health
                .insert(basket_name.to_string(), count);
        }
        tracing::debug!("Basket health: {:?}", self.state.onchain.basket_health);

        // BRC-48 TaskCommitment token
        let task_hash = {
            let mut h = sha2::Sha256::new();
            h.update(task.as_bytes());
            hex::encode(&h.finalize()[..8])
        };
        let skills_hash = {
            let mut skill_names: Vec<String> = self
                .skills
                .auto_activated()
                .iter()
                .map(|s| s.name.clone())
                .collect();
            skill_names.sort();
            let mut h = sha2::Sha256::new();
            for name in &skill_names {
                h.update(name.as_bytes());
                h.update(b"|");
            }
            hex::encode(&h.finalize()[..16])
        };
        let effective_budget_cap = self.budget_tracker.limits().max_per_task;
        let commitment_data = serde_json::json!({
            "task_hash": task_hash, "budget_cap": effective_budget_cap,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "model": self.config.llm.default_model, "skills_hash": skills_hash,
        });
        if let Some(token) = self
            .record_token(
                "task_commitment",
                state::BASKET_STATE,
                commitment_data,
                None,
            )
            .await
        {
            self.state.onchain.task_commitment = Some(token);
        }

        // BRC-48 BudgetAllocation token at task start
        let initial_budget_data = serde_json::json!({
            "budget_cap": effective_budget_cap,
            "task_spent": 0, "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        if let Some(token) = self
            .record_token(
                "budget_allocation",
                state::BASKET_BUDGET,
                initial_budget_data,
                None,
            )
            .await
        {
            self.state.onchain.budget_allocation = Some(token);
        }

        // Fire TaskStarted hook
        self.hook_registry
            .fire(crate::hooks::HookEvent::TaskStarted {
                task_id: ctx.task_id.clone(),
            })
            .await;

        // BRC-48 CapabilityDeclaration token
        {
            let tool_names = {
                let registry = self.tools.read().await;
                let mut names = registry.tool_names();
                names.sort();
                names
            };
            let skill_names: Vec<String> = self
                .skills
                .auto_activated()
                .iter()
                .map(|s| s.name.clone())
                .collect();
            let identity = self.wallet.get_identity_key().await.unwrap_or_default();
            let cap_data = serde_json::json!({
                "tools": tool_names, "tool_count": tool_names.len(),
                "skills": skill_names, "identity_key": identity,
                "version": env!("CARGO_PKG_VERSION"), "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            if let Some(token) = self
                .record_token(
                    "capability_declaration",
                    state::BASKET_STATE,
                    cap_data,
                    None,
                )
                .await
            {
                self.state.onchain.capability_declaration = Some(token);
            }
        }
    }

    /// EPIC #329 Phase 3 — parse, verify, and apply an inbound
    /// `task_delegation` envelope from a MessageBox commission.
    ///
    /// Called from `setup_task()` after the external_origin allowlist has been
    /// applied. On success, the verified delegation's effective capabilities
    /// and budget cap are applied as caveats, overriding the generic
    /// external-origin allowlist. On failure, the external-origin allowlist
    /// remains in force (graceful degradation).
    ///
    /// Always a no-op if `state.storage.pending_delegation_envelope` is `None`.
    async fn apply_pending_delegation(&mut self, task: &str) {
        let envelope = match self.state.storage.pending_delegation_envelope.take() {
            Some(e) => e,
            None => return,
        };

        use crate::delegation::{
            verify_delegation_chain, DelegationCert, DelegationContext, OverlayRevocationChecker,
            SystemClock, VerifyInputs, WalletSignatureVerifier,
        };

        // Parse leaf cert
        let cert_value = match envelope.get("delegation_cert") {
            Some(v) => v,
            None => {
                tracing::warn!("delegation envelope missing `delegation_cert` field");
                self.record_delegation_rejected("envelope missing delegation_cert");
                return;
            }
        };
        let leaf = match DelegationCert::from_value(cert_value) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("delegation cert parse failed: {e}");
                self.record_delegation_rejected(&format!("parse leaf cert: {e}"));
                return;
            }
        };

        // Parse optional parent chain (Phase 2 sends it as an empty array).
        // The chain is oldest-first (root at index 0) with the leaf appended last.
        let mut chain: Vec<DelegationCert> = Vec::new();
        if let Some(arr) = envelope.get("delegation_chain").and_then(|v| v.as_array()) {
            for parent in arr {
                match DelegationCert::from_value(parent) {
                    Ok(c) => chain.push(c),
                    Err(e) => {
                        tracing::warn!("delegation parent cert parse failed: {e}");
                        self.record_delegation_rejected(&format!("parse parent cert: {e}"));
                        return;
                    }
                }
            }
        }
        chain.push(leaf);

        // Build trusted certifier set from config
        let trusted: std::collections::HashSet<String> =
            self.config.trust.certifiers.iter().cloned().collect();
        if trusted.is_empty() {
            tracing::warn!(
                "trust.certifiers is empty — cannot verify inbound delegation (set \
                 DOLPHIN_MILK_TRUST_CERTIFIERS or config.trust.certifiers)"
            );
            self.record_delegation_rejected("empty trust.certifiers");
            return;
        }

        let my_key = self.wallet.get_identity_key().await.unwrap_or_default();
        if my_key.is_empty() {
            tracing::warn!("could not fetch own identity key for delegation verification");
            self.record_delegation_rejected("missing own identity key");
            return;
        }

        let signer = WalletSignatureVerifier::new();
        let revoker = OverlayRevocationChecker::new(&self.config.overlay.submit_url);
        let clock = SystemClock;

        let inputs = VerifyInputs {
            chain: &chain,
            task_description: task,
            my_identity_key: &my_key,
            trusted_certifiers: &trusted,
        };

        match verify_delegation_chain(inputs, &signer, &revoker, &clock).await {
            Ok(verified) => {
                let caps = verified.effective_capabilities.clone();
                let budget_cap = verified.effective_budget_cap_sats;
                let expires_at = verified.effective_expires_at;
                let chain_depth = verified.chain_depth;
                let root_certifier = verified.root_certifier.clone();

                // Apply the delegation's tool allowlist. This REPLACES the
                // external_origin allowlist — the cert's capability set is
                // more specific and authoritative.
                //
                // System escape hatches always available regardless of cert
                // caps: `read_tool_output` (for retrieving offloaded tool
                // results by call_id) and `file_read` (for reading workspace
                // files, including the offload directory). These operate on
                // the agent's own workspace only, so they're within the
                // delegation's trust boundary — the delegator has already
                // authorized this agent to do its work, and managing its own
                // context window to complete that work is part of that. With
                // a cert that only grants `web_fetch`, a delegated agent
                // still needs to be able to read back a large response it
                // just fetched; without these escape hatches the agent sees
                // a preview it can't expand and loops on retries. See
                // `generate_persisted_output_preview` in execute.rs for the
                // preview format that leans on these tools.
                let mut allowlist: std::collections::HashSet<String> =
                    caps.iter().cloned().collect();
                allowlist.insert("read_tool_output".to_string());
                allowlist.insert("file_read".to_string());
                self.tools.write().await.set_allowlist(allowlist);

                // Apply the delegation's budget cap as a per-task ceiling.
                // Other tiers (hour/day/week/month/lifetime) are left alone.
                self.budget_tracker.apply_cert_limits(
                    Some(budget_cap),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                );

                tracing::info!(
                    "delegation verified: depth={chain_depth}, root={}, caps={caps:?}, \
                     budget_cap={budget_cap}, expires_at={expires_at}",
                    &root_certifier[..16.min(root_certifier.len())]
                );

                let mut data = HashMap::new();
                data.insert("chain_depth".into(), Value::from(chain_depth));
                data.insert("capabilities".into(), serde_json::json!(caps));
                data.insert("budget_cap_sats".into(), Value::from(budget_cap));
                data.insert("expires_at".into(), Value::from(expires_at.to_rfc3339()));
                data.insert("root_certifier".into(), Value::from(root_certifier));
                self.transcript.record("delegation_verified", data);

                self.state.storage.delegation_context = Some(DelegationContext { verified, chain });
            }
            Err(e) => {
                tracing::warn!(
                    "delegation verification failed (falling back to external-origin allowlist): {e}"
                );
                self.record_delegation_rejected(&e.to_string());
            }
        }
    }

    /// Helper: emit a `delegation_rejected` transcript event.
    fn record_delegation_rejected(&mut self, reason: &str) {
        let mut data = HashMap::new();
        data.insert("reason".into(), Value::from(reason.to_string()));
        self.transcript.record("delegation_rejected", data);
    }

    /// EPIC #329 Phase 3 §C.2: emit a `commission_payment_claim` message
    /// to the delegation cert's issuer at session_end.
    ///
    /// Reads `state.storage.delegation_context.verified.cert.payment` for
    /// the BRC-29 invoice + max_total. Generates a unique receive address
    /// from the wallet via `receive_address(serial_number)`. Wraps the
    /// claim in a MessageBox body and sends to the cert.certifier (the
    /// agent that issued the commission). The issuer's heartbeat handler
    /// (Phase C.3 in heartbeat/sources.rs) validates + pays.
    ///
    /// All fields are recorded in the transcript so deep inspection can
    /// verify what was claimed and where it was sent.
    async fn emit_commission_payment_claim(&mut self) {
        let Some(ref ctx) = self.state.storage.delegation_context else {
            return; // Not a commission task
        };
        let Some(ref payment_terms) = ctx.verified.cert.payment else {
            tracing::debug!("commission task has no PaymentTerms in cert — skipping payment claim");
            return;
        };
        if payment_terms.max_total == 0 {
            tracing::debug!("commission PaymentTerms.max_total == 0 — skipping payment claim");
            return;
        }

        let issuer = ctx.verified.cert.certifier.clone();
        let serial = ctx.verified.cert.serial_number.clone();
        let amount_sats = payment_terms.max_total;
        let derivation_invoice = if payment_terms.derivation_invoice.is_empty() {
            serial.clone()
        } else {
            payment_terms.derivation_invoice.clone()
        };

        // Derive a fresh receive address using the derivation_invoice as
        // the suffix. This generates a unique address per commission so
        // payments are independently traceable on-chain.
        let receive = match self.wallet.receive_address(&derivation_invoice).await {
            Ok((pubkey, script_hex, _suffix)) => {
                match crate::tools::wallet_tools::script_to_address(&script_hex) {
                    Ok(address) => Some((pubkey, script_hex, address)),
                    Err(e) => {
                        tracing::warn!("script_to_address failed: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!("wallet.receive_address failed for commission claim: {e}");
                None
            }
        };

        let Some((pubkey, _script_hex, address)) = receive else {
            self.transcript.record(
                "commission_payment_claim_failed",
                HashMap::from([(
                    "reason".to_string(),
                    Value::from("could not derive receive address"),
                )]),
            );
            return;
        };

        let result_hash = super::content_hash(&self.state.exec.result);
        let task_hash = {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(self.state.storage.task.as_bytes());
            hex::encode(&h.finalize()[..8])
        };

        let body = serde_json::json!({
            "type": "commission_payment_claim",
            "commission_serial": serial,
            "amount_sats": amount_sats,
            "receive_address": address,
            "receive_pubkey": pubkey,
            "derivation_invoice": derivation_invoice,
            "completed_task_hash": format!("sha256:{task_hash}"),
            "result_hash": format!("sha256:{result_hash}"),
            "claimant_identity_key": self.wallet.get_identity_key().await.unwrap_or_default(),
        });

        match self
            .messagebox
            .send_message(&issuer, crate::messagebox::types::BOX_TASK_INBOX, &body)
            .await
        {
            Ok(resp) => {
                let sent_id = resp
                    .get("sentMessageId")
                    .or_else(|| resp.get("messageId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                tracing::info!(
                    issuer = %&issuer[..16.min(issuer.len())],
                    serial = %serial,
                    amount_sats,
                    sent_id = %sent_id,
                    "commission payment claim sent to issuer"
                );

                let mut data = HashMap::new();
                data.insert("commission_serial".into(), Value::from(serial.clone()));
                data.insert("issuer".into(), Value::from(issuer.clone()));
                data.insert("amount_sats".into(), Value::from(amount_sats));
                data.insert("receive_address".into(), Value::from(address.clone()));
                data.insert("derivation_invoice".into(), Value::from(derivation_invoice));
                data.insert("sent_message_id".into(), Value::from(sent_id));
                self.transcript.record("commission_payment_claim", data);

                // Surface the claim in the commission conversation so the
                // user can see payment activity in the UI without having to
                // dig into the activity/transcript pane. EPIC #329 Phase 3.
                if let Some(ref conv_id) = self.current_conversation_id.clone() {
                    self.append_commission_conversation_event(
                        conv_id,
                        &format!(
                            "💰 Payment claim sent — {amount_sats} sats to {address} \
                             (commission {serial})"
                        ),
                    );

                    // Persist a serial → conv_id mapping so the heartbeat
                    // handler that internalizes the eventual
                    // commission_payment_sent reply can find the right
                    // conversation to append the receipt to. The runner has
                    // the conv binding; the heartbeat doesn't.
                    let index_dir = self.global_workspace.join("commission_conv_index");
                    let _ = std::fs::create_dir_all(&index_dir);
                    let _ = std::fs::write(index_dir.join(format!("{serial}.txt")), conv_id);
                }
            }
            Err(e) => {
                tracing::warn!(
                    issuer = %&issuer[..16.min(issuer.len())],
                    serial = %serial,
                    "Failed to send commission payment claim: {e}"
                );
                let mut data = HashMap::new();
                data.insert("commission_serial".into(), Value::from(serial));
                data.insert("error".into(), Value::from(e.to_string()));
                self.transcript
                    .record("commission_payment_claim_failed", data);
            }
        }
    }

    // =============================================================================
    // Main loop
    // =============================================================================

    /// Main iteration loop with error handling and cancellation.
    pub(crate) async fn run_loop(&mut self, max_iterations: u32, ctx: &StepContext) {
        while !self.state.exec.done
            && self.state.exec.iteration < max_iterations
            && !ctx.cancel.load(std::sync::atomic::Ordering::Relaxed)
        {
            match self.step(ctx).await {
                Ok(_) => {}
                Err(e) => {
                    let error_type = match &e {
                        DmError::Budget { .. } => {
                            tracing::warn!("Budget exhausted: {e}");
                            "budget"
                        }
                        DmError::Loop { .. } => {
                            tracing::warn!("Loop breaker triggered: {e}");
                            "loop"
                        }
                        _ => {
                            tracing::error!("Error in agent loop: {e}");
                            "worm"
                        }
                    };
                    self.state.exec.error = e.to_string();
                    self.state.exec.done = true;
                    let mut err_ctx = HashMap::new();
                    err_ctx.insert("type".to_string(), Value::String(error_type.to_string()));
                    self.transcript.record_error(&e.to_string(), Some(err_ctx));
                    emit_event(
                        ctx,
                        StepEvent::Error {
                            iteration: self.state.exec.iteration,
                            message: e.to_string(),
                        },
                    );
                }
            }
        }

        if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            self.state.exec.error = "Cancelled by user".to_string();
            self.state.exec.done = true;
            let mut err_ctx = HashMap::new();
            err_ctx.insert("type".to_string(), Value::String("cancelled".to_string()));
            self.transcript
                .record_error(&self.state.exec.error, Some(err_ctx));
            tracing::info!(
                "Task cancelled by user after {} iterations",
                self.state.exec.iteration
            );
            emit_event(
                ctx,
                StepEvent::Error {
                    iteration: self.state.exec.iteration,
                    message: "Cancelled by user".to_string(),
                },
            );
        } else if self.state.exec.iteration >= max_iterations {
            self.state.exec.error = format!("Hit max iterations ({max_iterations})");
            let mut err_ctx = HashMap::new();
            err_ctx.insert(
                "type".to_string(),
                Value::String("max_iterations".to_string()),
            );
            self.transcript
                .record_error(&self.state.exec.error, Some(err_ctx));
        }
    }

    // =============================================================================
    // Teardown
    // =============================================================================

    /// Teardown phase: record session end, session summarization, BRC proofs/tokens,
    /// relinquish TaskCommitment.
    pub(crate) async fn teardown_task(&mut self, ctx: &StepContext) {
        // Clean up tool resources (e.g. shut down Chrome)
        self.tools.read().await.cleanup_all().await;

        // Record session end
        let mut end_data = HashMap::new();
        end_data.insert(
            "iterations".to_string(),
            serde_json::json!(self.state.exec.iteration),
        );
        end_data.insert(
            "sats_spent".to_string(),
            serde_json::json!(self.state.budget.sats_spent),
        );
        let result_preview: String = if self.state.exec.result.chars().count() > 500 {
            self.state.exec.result.chars().take(500).collect()
        } else {
            self.state.exec.result.clone()
        };
        end_data.insert("result".to_string(), Value::String(result_preview));
        end_data.insert(
            "error".to_string(),
            Value::String(self.state.exec.error.clone()),
        );
        self.transcript.record("session_end", end_data);

        emit_event(
            ctx,
            StepEvent::Done {
                iterations: self.state.exec.iteration,
                sats_spent: self.state.budget.sats_spent,
                result: self.state.exec.result.clone(),
            },
        );

        // Fire TaskCompleted or TaskFailed hook
        if self.state.exec.error.is_empty() {
            self.hook_registry
                .fire(crate::hooks::HookEvent::TaskCompleted {
                    task_id: ctx.task_id.clone(),
                    result_summary: self.state.exec.result.chars().take(500).collect(),
                })
                .await;
        } else {
            self.hook_registry
                .fire(crate::hooks::HookEvent::TaskFailed {
                    task_id: ctx.task_id.clone(),
                    error: self.state.exec.error.clone(),
                })
                .await;
        }

        // EPIC #329 Phase 3 §C.2: emit a commission_payment_claim message to
        // the cert issuer (Captain) at task completion. This is automatic —
        // no LLM tool call needed — so the receiver always claims its
        // earnings whether or not the task LLM remembered to.
        //
        // Skipped if:
        //   - no delegation_context (this wasn't a commission task)
        //   - cert has no PaymentTerms (delegate_task didn't populate them)
        //   - the task errored out (no work done = no payment owed)
        if self.state.exec.error.is_empty() {
            self.emit_commission_payment_claim().await;
        }

        // Session summarization
        let events: Vec<Value> = self
            .transcript
            .replay()
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect();
        let session_id = format!("session-{}", chrono::Utc::now().format("%Y-%m-%d-%H%M%S"));
        let session_entry = SessionSummarizer::create_session_entry(&events, &session_id);

        if let Err(e) = self.memory_store.store(&session_entry) {
            tracing::warn!("Failed to store session summary: {e}");
        } else {
            if let Some(ref index) = self.memory_index {
                if let Err(e) = index.add_entry(&session_entry) {
                    tracing::warn!("Failed to index session summary: {e}");
                }
            }
            tracing::info!("Session summary stored: {}", session_entry.id);

            // Encrypt session summary
            match self
                .memory_sync
                .encrypt_and_store(&session_entry, &self.wallet, "sessions")
                .await
            {
                Ok(uhrp_hash) => {
                    tracing::debug!("Session summary encrypted, UHRP hash: {}", uhrp_hash)
                }
                Err(e) => tracing::warn!("Failed to encrypt session summary: {e}"),
            }

            // BRC-18 TaskCompletion proof
            let task_hash = {
                let mut h = sha2::Sha256::new();
                h.update(self.state.storage.task.as_bytes());
                hex::encode(&h.finalize()[..8])
            };
            let result_content_hash = content_hash(&self.state.exec.result);
            let commitment = proofs::task_completion_proof(
                &format!("task:{task_hash}"),
                &format!(
                    "{} iterations, result: sha256:{}",
                    self.state.exec.iteration, result_content_hash
                ),
                self.state.onchain.last_proof_hash.as_deref(),
            );
            self.record_proof(commitment, "task_completion", None).await;

            // BRC-18 BudgetSnapshot proof
            let report = self.budget_tracker.report();
            let service_names: Vec<String> = report.services.keys().cloned().collect();
            let service_details: Vec<String> = report
                .services
                .iter()
                .map(|(name, sr)| format!("{}={}", name, sr.total_sats))
                .collect();
            let budget_data = format!(
                "TASK_SATS: {}\nITERATIONS: {}\nSERVICES: {}\nBREAKDOWN: {}",
                self.state.budget.sats_spent,
                self.state.exec.iteration,
                if service_names.is_empty() {
                    "none".to_string()
                } else {
                    service_names.join(",")
                },
                if service_details.is_empty() {
                    "none".to_string()
                } else {
                    service_details.join(",")
                },
            );
            let budget_commitment = proofs::ProofCommitment::new(
                proofs::ProofType::BudgetSnapshot,
                &budget_data,
                self.state.onchain.last_proof_hash.as_deref(),
            );
            self.record_proof(budget_commitment, "budget_snapshot", None)
                .await;

            // BRC-18 Custody proof for billing verification (#205)
            {
                let identity_key = self.get_identity_key().await;
                let result_content_hash = content_hash(if self.state.exec.result.is_empty() {
                    ""
                } else {
                    &self.state.exec.result
                });
                // Extract unique tool names from transcript tool_call events
                let tools_used: Vec<String> = {
                    let mut names: Vec<String> = self
                        .transcript
                        .get_events_by_type("tool_call")
                        .iter()
                        .filter_map(|e| {
                            e.data
                                .get("name")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                        })
                        .collect();
                    names.sort();
                    names.dedup();
                    names
                };
                // Compute duration from session_start timestamp to now
                let duration_secs = self
                    .transcript
                    .get_events_by_type("session_start")
                    .first()
                    .map(|e| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs_f64();
                        (now - e.ts).max(0.0) as u64
                    })
                    .unwrap_or(0);
                let custody_commitment = proofs::custody_proof(
                    None, // TODO: extract caller_key from task metadata when available
                    &task_hash,
                    self.state.exec.iteration,
                    duration_secs,
                    &tools_used,
                    self.state.budget.sats_spent,
                    &result_content_hash,
                    self.state.onchain.last_proof_hash.as_deref(),
                    self.proof_txids().len(),
                    &identity_key,
                    self.state.auth.cert_hash.as_deref(),
                    self.state.onchain.last_proof_hash.as_deref(),
                );
                if self
                    .record_proof(custody_commitment, "custody", None)
                    .await
                    .is_none()
                {
                    tracing::warn!("Custody proof creation failed (non-fatal)");
                }
            }

            // Compute HMAC of transcript for integrity verification.
            // The HMAC is stored in the Checkpoint token so auditors can verify
            // that the transcript has not been tampered with after the session ended.
            let transcript_hmac = {
                let transcript_bytes = self.transcript.read_bytes();
                if transcript_bytes.is_empty() {
                    None
                } else {
                    match self
                        .wallet
                        .create_hmac(
                            &transcript_bytes,
                            &serde_json::json!([2, "dolphin milk transcript"]),
                            &self.state.storage.task_id,
                            "self",
                        )
                        .await
                    {
                        Ok(hmac_bytes) => {
                            let hmac_b64 = base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &hmac_bytes,
                            );
                            tracing::info!(
                                "Transcript HMAC computed for task {}",
                                self.state.storage.task_id
                            );
                            Some(hmac_b64)
                        }
                        Err(e) => {
                            tracing::warn!("Failed to compute transcript HMAC (non-fatal): {e}");
                            None
                        }
                    }
                }
            };

            // BRC-48 Checkpoint token
            let mut checkpoint_data = serde_json::json!({
                "task": self.state.storage.task.chars().take(200).collect::<String>(),
                "iterations": self.state.exec.iteration, "sats_spent": self.state.budget.sats_spent,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "result_preview": self.state.exec.result.chars().take(100).collect::<String>(),
            });
            if let Some(ref hmac) = transcript_hmac {
                checkpoint_data["hmac"] = serde_json::Value::String(hmac.clone());
            }
            let old_cp = self.state.onchain.last_checkpoint.clone();
            if let Some(token) = self
                .record_token(
                    "checkpoint",
                    state::BASKET_STATE,
                    checkpoint_data,
                    old_cp.as_ref(),
                )
                .await
            {
                self.state.onchain.last_checkpoint = Some(token);
            }

            // Relinquish TaskCommitment token
            if let Some(ref tc) = self.state.onchain.task_commitment {
                if let (Some(ref txid), Some(vout)) = (&tc.txid, tc.vout) {
                    match self
                        .wallet
                        .relinquish_output(state::BASKET_STATE, txid, vout)
                        .await
                    {
                        Ok(_) => {
                            tracing::info!("Relinquished TaskCommitment token: {}:{}", txid, vout)
                        }
                        Err(e) => tracing::warn!(
                            "Failed to relinquish TaskCommitment token (non-fatal): {e}"
                        ),
                    }
                }
            }

            // Relinquish CapabilityDeclaration token
            if let Some(ref cd) = self.state.onchain.capability_declaration {
                if let (Some(ref txid), Some(vout)) = (&cd.txid, cd.vout) {
                    match self
                        .wallet
                        .relinquish_output(state::BASKET_STATE, txid, vout)
                        .await
                    {
                        Ok(_) => tracing::info!(
                            "Relinquished CapabilityDeclaration token: {}:{}",
                            txid,
                            vout
                        ),
                        Err(e) => tracing::warn!(
                            "Failed to relinquish CapabilityDeclaration token (non-fatal): {e}"
                        ),
                    }
                }
            }

            // BRC-48 BudgetAllocation token (final)
            let budget_report = self.budget_tracker.report();
            let service_breakdown: serde_json::Value = budget_report
                .services
                .iter()
                .map(|(name, sr)| {
                    (
                        name.clone(),
                        serde_json::json!({"sats": sr.total_sats, "ops": sr.count}),
                    )
                })
                .collect::<serde_json::Map<String, serde_json::Value>>()
                .into();
            let budget_token_data = serde_json::json!({
                "balance": self.wallet.get_balance().await.unwrap_or(0),
                "task_spent": self.state.budget.sats_spent, "hour_spent": budget_report.hourly_sats,
                "day_spent": budget_report.daily_sats, "services": service_breakdown,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let old_ba = self.state.onchain.budget_allocation.clone();
            self.record_token(
                "budget_allocation",
                state::BASKET_BUDGET,
                budget_token_data,
                old_ba.as_ref(),
            )
            .await;
        }
    }

    // =============================================================================
    // Run (public entry point)
    // =============================================================================

    /// Run the agent loop for a given task.
    ///
    /// Orchestrates setup → loop → teardown phases.
    /// When `tx` is `Some`, emits `StepEvent`s for SSE streaming (server mode).
    pub async fn run(
        &mut self,
        task: &str,
        max_iterations: u32,
        tx: Option<(&broadcast::Sender<(String, StepEvent)>, &str)>,
        cancel: Arc<AtomicBool>,
    ) {
        let ctx = StepContext {
            events: tx.as_ref().map(|(sender, _)| (*sender).clone()),
            task_id: tx
                .as_ref()
                .map(|(_, id)| id.to_string())
                .unwrap_or_default(),
            cancel,
        };

        self.setup_task(task, &ctx).await;
        self.run_loop(max_iterations, &ctx).await;
        self.teardown_task(&ctx).await;

        tracing::info!(
            "Agent loop complete: {} iterations, {} sats spent",
            self.state.exec.iteration,
            self.state.budget.sats_spent
        );
    }
}
