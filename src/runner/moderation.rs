//! Runner-level moderation helpers — engine construction, content checks, transcript recording.
//!
//! The `ModerationEngine` (from `src/moderation.rs`) provides the core matching logic.
//! This module provides the runner integration: building the engine from certificate
//! policy + config, moderating content with logging and transcript recording, and
//! a simple outcome enum for callers.
//!
//! Moderation is applied at four points in the agent loop:
//! 1. Inbox messages (`observe`)
//! 2. LLM response text (`think_step`)
//! 3. Tool inputs (`execute_tools_sequential` / `execute_tools_parallel`)
//! 4. Tool outputs (`execute_tools`)

use std::collections::HashMap;

use serde_json::Value;

use crate::certificates::read_cert_moderation_policy;
use crate::moderation::{ModerationEngine, ModerationMatch, ModerationResult};

use super::DmLoop;

/// Outcome of a moderation check — callers decide the action.
///
/// Unlike `ModerationResult`, this does not carry match data (already recorded
/// in the transcript by `moderate_content`). Callers only need to know whether
/// to proceed, skip, or replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModerationOutcome {
    /// Content passed all checks.
    Pass,
    /// Content was flagged — logged in transcript, caller should continue.
    Flagged,
    /// Content was blocked — logged in transcript, caller should stop/skip/replace.
    Blocked,
}

impl DmLoop {
    /// Construct the moderation engine from certificate policy and config.
    ///
    /// Called during `setup_task()`. Reads the BRC-52 certificate's moderation
    /// policy (which can only tighten, never relax), merges with config defaults.
    pub(crate) async fn build_moderation_engine(&mut self) {
        let cert_policy = read_cert_moderation_policy(self.wallet.clone()).await;
        self.moderation = ModerationEngine::new(&self.config.moderation, Some(&cert_policy));
        if self.config.moderation.enabled {
            tracing::info!(
                "Moderation engine enabled (PII: {}, profanity: {})",
                self.config.moderation.pii_mode,
                self.config.moderation.profanity_mode
            );
        }
    }

    /// Moderate content, log the result, and record a transcript event.
    ///
    /// Returns the outcome — callers decide the action (skip, error, replace, continue).
    ///
    /// # Arguments
    /// * `content` — The text to moderate.
    /// * `source` — Transcript source field: `"user_message"`, `"llm_response"`,
    ///   `"tool_input"`, or `"tool_output"`.
    /// * `tool_name` — Optional tool name for the transcript `"tool"` field.
    ///   Set for tool_input/tool_output, `None` for user_message/llm_response.
    /// * `description` — Human-readable description for log messages, e.g.
    ///   `"inbox message from 02ab..."`, `"LLM response"`, `"tool input for 'web_fetch'"`.
    pub(crate) fn moderate_content(
        &mut self,
        content: &str,
        source: &str,
        tool_name: Option<&str>,
        description: &str,
    ) -> ModerationOutcome {
        match self.moderation.moderate(content) {
            ModerationResult::Blocked(ref matches) => {
                tracing::error!(
                    "Moderation blocked {}: {} match(es)",
                    description,
                    matches.len()
                );
                self.record_moderation_event("blocked", source, tool_name, matches);
                ModerationOutcome::Blocked
            }
            ModerationResult::Flagged(ref matches) => {
                tracing::warn!(
                    "Moderation flagged {}: {} match(es)",
                    description,
                    matches.len()
                );
                self.record_moderation_event("flagged", source, tool_name, matches);
                ModerationOutcome::Flagged
            }
            ModerationResult::Pass => ModerationOutcome::Pass,
        }
    }

    /// Record a moderation event in the JSONL transcript.
    ///
    /// Fields: `type: "moderation"`, `action`, `source`, optional `tool`, `matches`.
    fn record_moderation_event(
        &mut self,
        action: &str,
        source: &str,
        tool_name: Option<&str>,
        matches: &[ModerationMatch],
    ) {
        let mut data = HashMap::new();
        data.insert("type".to_string(), Value::String("moderation".to_string()));
        data.insert("action".to_string(), Value::String(action.to_string()));
        data.insert("source".to_string(), Value::String(source.to_string()));
        if let Some(tool) = tool_name {
            data.insert("tool".to_string(), Value::String(tool.to_string()));
        }
        data.insert(
            "matches".to_string(),
            serde_json::to_value(matches).unwrap_or_default(),
        );
        self.transcript.record("system", data);
    }
}
