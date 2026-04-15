//! High-level LLM inference via BRC-31 authenticated requests + x402 micropayments.
//!
//! Supports multiple providers (OpenAI, Claude) with automatic format conversion.
//! Internally normalizes all responses to a common ThinkResult format with OpenAI-style
//! tool_calls, so the rest of the codebase (runner, transcript, conversation) is unaffected.
//!
//! ## Module layout
//!
//! - **mod.rs** — ThinkResult, routing, middleware pipeline, re-exports
//! - **openai.rs** — OpenAI request body construction and response parsing
//! - **claude.rs** — Claude request body construction, response parsing, format conversion
//! - **capabilities.rs** — Model capability detection and limits
//!
//! Usage:
//!     let result = think(&auth, &messages, "gpt-5-mini", 4096, None, &config).await?;
//!     let result = think(&auth, &messages, "claude-sonnet-4-6", 4096, None, &config).await?;

pub mod capabilities;
pub mod claude;
pub mod openai;

// Re-export all public items for backward compatibility (callers use `crate::think::X`)
pub use capabilities::{model_input_limit, model_output_limit, ModelCapabilities};
pub use claude::{
    build_claude_body, convert_messages_for_claude, convert_tools_for_claude, flush_tool_results,
};
pub use openai::build_openai_body;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

use crate::auth::AuthriteClient;
use crate::config::DmConfig;
use crate::error::DmError;
use crate::runner::text_extract::extract_tool_calls_from_text;
use crate::x402::circuit_breaker::CircuitBreakerRegistry;
use crate::x402::{payment, refund};

/// Endpoint for the OpenAI x402 agent.
pub const OPENAI_AGENT_URL: &str = "https://openai-chat.x402agency.com/chat";

/// Endpoint for the Claude x402 agent.
pub const CLAUDE_AGENT_URL: &str = "https://claude-chat.x402agency.com/chat";

/// Check if a model name indicates an OpenAI reasoning model (o-series, gpt-5.x, gpt-4.1).
///
/// Reasoning models require `max_completion_tokens` instead of `max_tokens`,
/// don't support `temperature`, and may handle tool calling differently.
pub fn is_reasoning_model(model: &str) -> bool {
    // o-series: o1, o3, o4-mini, etc.
    if model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
        return true;
    }
    // gpt-5.x: gpt-5-mini, gpt-5.2, gpt-5.2-pro, gpt-5-nano
    if model.starts_with("gpt-5") {
        return true;
    }
    // gpt-4.1 series
    if model.starts_with("gpt-4.1") {
        return true;
    }
    false
}

/// Check if a model name is a Claude model.
pub fn is_claude_model(model: &str) -> bool {
    model.starts_with("claude-")
}

/// Resolve the LLM endpoint URL based on the provider name or model.
///
/// Priority: model name auto-detection → config.llm.default_provider → OpenAI default.
/// A claude-* model always routes to the Claude endpoint regardless of config,
/// matching server.rs behavior.
pub fn resolve_endpoint(config: &DmConfig, model: &str) -> &'static str {
    // Model name takes priority — a claude-* model always goes to Claude,
    // regardless of default_provider config.
    if is_claude_model(model) {
        return CLAUDE_AGENT_URL;
    }
    let provider = config.llm.default_provider.as_str();
    match provider {
        "claude-chat" | "claude" => CLAUDE_AGENT_URL,
        "openai-agent" | "openai" => OPENAI_AGENT_URL,
        _ => OPENAI_AGENT_URL,
    }
}

pub const DEFAULT_MODEL: &str = "gpt-5-mini";
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Get the alternate provider endpoint for failover.
/// OpenAI ↔ Claude.
pub fn alternate_endpoint(endpoint: &str) -> &'static str {
    if endpoint == CLAUDE_AGENT_URL {
        OPENAI_AGENT_URL
    } else {
        CLAUDE_AGENT_URL
    }
}

/// Result of a think() call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkResult {
    pub text: String,
    pub model: String,
    pub sats_paid: u64,
    pub sats_effective: u64,
    pub sats_refunded: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub finish_reason: String,
    pub duration_ms: u64,
    /// Tool calls requested by the LLM (normalized to OpenAI function calling format).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<Value>,
    /// Transaction ID of the x402 payment (None if no 402 occurred).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_txid: Option<String>,
    /// Whether the excess refund was successfully internalized.
    /// None = no refund, Some(true) = internalized, Some(false) = failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refund_internalized: Option<bool>,
    /// Whether tool calls were extracted from text (reasoning model fallback).
    /// True when the LLM returned tool calls as text and they were parsed out.
    #[serde(default)]
    pub was_text_extracted: bool,
}

/// Send messages to an LLM via BRC-31 auth + x402 payment and return the response.
///
/// This is the worm's primary interface for inference. It handles the
/// full authenticated + paid flow:
/// 1. BRC-31 authenticated request to the LLM endpoint
/// 2. If 402: parse payment requirements, create BRC-29 payment, retry with auth + payment
/// 3. Parse response (OpenAI or Claude format), handle refunds
pub async fn think(
    auth: &AuthriteClient,
    messages: &[Value],
    model: &str,
    max_tokens: u32,
    temperature: Option<f64>,
    config: &DmConfig,
) -> Result<ThinkResult, DmError> {
    think_with_tools(
        auth,
        messages,
        model,
        max_tokens,
        temperature,
        None,
        config,
        None,
        None,
    )
    .await
}

/// Parameters for a circuit-breaker-protected LLM call.
pub struct ThinkRequest<'a> {
    pub auth: &'a AuthriteClient,
    pub messages: &'a [Value],
    pub model: &'a str,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub tools: Option<&'a [Value]>,
    pub config: &'a DmConfig,
    /// Extended thinking budget for Claude models (tokens). None = disabled.
    pub thinking_budget: Option<u32>,
    /// Reasoning effort for OpenAI models. None = API default.
    pub reasoning_effort: Option<&'a str>,
}

/// Send messages to an LLM with circuit breaker protection.
///
/// When the primary provider's circuit is open, automatically routes to the
/// alternate provider. Records success/failure on the provider that was actually used.
pub async fn think_with_circuit_breaker(
    req: &ThinkRequest<'_>,
    circuit_breakers: &CircuitBreakerRegistry,
) -> Result<ThinkResult, DmError> {
    let primary_endpoint = resolve_endpoint(req.config, req.model);
    let alt_endpoint = alternate_endpoint(primary_endpoint);

    // Pick endpoint: primary if circuit allows, else try alternate
    let endpoint = if circuit_breakers.should_allow(primary_endpoint) {
        primary_endpoint
    } else if circuit_breakers.should_allow(alt_endpoint) {
        tracing::warn!(
            "Circuit breaker: primary {} is open, failing over to {}",
            primary_endpoint,
            alt_endpoint
        );
        alt_endpoint
    } else {
        // Both circuits open — try primary anyway (better than returning error)
        tracing::warn!(
            "Circuit breaker: both providers open, attempting primary {} anyway",
            primary_endpoint
        );
        primary_endpoint
    };

    let result = think_to_endpoint(
        req.auth,
        req.messages,
        req.model,
        req.max_tokens,
        req.temperature,
        req.tools,
        endpoint,
        req.thinking_budget,
        req.reasoning_effort,
    )
    .await;

    match &result {
        Ok(_) => circuit_breakers.record_success(endpoint),
        Err(err) => circuit_breakers.record_failure_msg(endpoint, &err.to_string()),
    }

    result
}

/// Send messages to an LLM with tool definitions for function calling.
///
/// Tools are expected in OpenAI format (from `ToolRegistry::to_openai_tools()`).
/// If the target provider is Claude, tools and messages are automatically converted.
///
/// When tool definitions are provided and the response has no tool calls but contains
/// text, attempts to extract tool calls from the text (reasoning model fallback).
/// Sets `was_text_extracted = true` on the result if extraction succeeds.
#[allow(clippy::too_many_arguments)]
pub async fn think_with_tools(
    auth: &AuthriteClient,
    messages: &[Value],
    model: &str,
    max_tokens: u32,
    temperature: Option<f64>,
    tools: Option<&[Value]>,
    config: &DmConfig,
    thinking_budget: Option<u32>,
    reasoning_effort: Option<&str>,
) -> Result<ThinkResult, DmError> {
    let endpoint = resolve_endpoint(config, model);
    let mut result = think_to_endpoint(
        auth,
        messages,
        model,
        max_tokens,
        temperature,
        tools,
        endpoint,
        thinking_budget,
        reasoning_effort,
    )
    .await?;

    // Post-processing: reasoning model text extraction fallback (#211).
    // If the LLM returned no tool calls but we sent tool definitions,
    // try to extract tool calls from the response text.
    if result.tool_calls.is_empty() && !result.text.is_empty() && tools.is_some() {
        let known_tool_names = extract_tool_names_from_defs(tools.unwrap_or(&[]));
        if !known_tool_names.is_empty() {
            let extracted = extract_tool_calls_from_text(&result.text, &known_tool_names);
            if !extracted.is_empty() {
                tracing::info!(
                    "Text extraction fallback: extracted {} tool call(s) from response text",
                    extracted.len()
                );
                result.tool_calls = extracted;
                result.was_text_extracted = true;
            }
        }
    }

    Ok(result)
}

/// Extract tool names from OpenAI-format tool definitions.
fn extract_tool_names_from_defs(tools: &[Value]) -> Vec<String> {
    tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Core LLM inference to a specific endpoint.
///
/// Extracted from `think_with_tools` so that `think_with_circuit_breaker` can
/// route to an alternate endpoint without duplicating logic.
#[allow(clippy::too_many_arguments)]
async fn think_to_endpoint(
    auth: &AuthriteClient,
    messages: &[Value],
    model: &str,
    max_tokens: u32,
    temperature: Option<f64>,
    tools: Option<&[Value]>,
    endpoint: &str,
    thinking_budget: Option<u32>,
    reasoning_effort: Option<&str>,
) -> Result<ThinkResult, DmError> {
    let use_claude = endpoint == CLAUDE_AGENT_URL;
    let body = if use_claude {
        build_claude_body(
            messages,
            model,
            max_tokens,
            temperature,
            tools,
            thinking_budget,
        )
    } else {
        build_openai_body(
            messages,
            model,
            max_tokens,
            temperature,
            tools,
            reasoning_effort,
        )
    };

    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| DmError::payment(format!("Failed to serialize request body: {e}")))?;

    let body_size = body_bytes.len();
    let tool_count = tools.map(|t| t.len()).unwrap_or(0);
    tracing::info!(
        "think: provider={}, model={}, messages={}, tools={}, max_tokens={}, body_size={} bytes",
        if use_claude { "claude" } else { "openai" },
        model,
        messages.len(),
        tool_count,
        max_tokens,
        body_size,
    );

    let t0 = Instant::now();

    let headers: Vec<(String, String)> = vec![("content-type".into(), "application/json".into())];

    // BRC-31 auth + x402 payment via shared helper (handles retry loop)
    let resp =
        payment::authenticated_paid_request(auth, "POST", endpoint, &headers, Some(&body_bytes))
            .await?;

    if use_claude {
        claude::parse_claude_response(&resp.body, resp.status, auth, model, t0, resp.payment_txid)
            .await
    } else {
        openai::parse_openai_response(&resp.body, resp.status, auth, model, t0, resp.payment_txid)
            .await
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Attempt to internalize a refund from an error response body.
///
/// When a proxy returns 500 with ERR_SERVICE_FAILED_REFUND_ISSUED, the refund
/// data is in the response body. Without this, the refund sats go unclaimed.
async fn try_internalize_refund_from_error(data: &Value, auth: &AuthriteClient) {
    if let Some(refund_info) = refund::parse_refund(data) {
        let sats = refund_info.satoshis;
        match refund::process_refund(auth.wallet_api(), &refund_info).await {
            Ok(_) => tracing::info!(
                "Refund internalized from error response: {} sats recovered",
                sats
            ),
            Err(e) => tracing::warn!(
                "Failed to internalize refund from error response ({} sats): {e}",
                sats
            ),
        }
    }
}

/// Parse an error response from either provider.
fn parse_error_response(data: &Value, status: StatusCode) -> DmError {
    // Log the FULL response body so we can debug upstream failures
    tracing::error!(
        "LLM error response (HTTP {}): {}",
        status.as_u16(),
        serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
    );

    // Try OpenAI-style error
    let error_code = data
        .get("code")
        .and_then(|v| v.as_str())
        // Also try Claude-style error.type
        .or_else(|| {
            data.get("error")
                .and_then(|e| e.get("type"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("UNKNOWN");
    let description = data
        .get("description")
        .and_then(|v| v.as_str())
        // Also try Claude-style error.message
        .or_else(|| {
            data.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");

    // Extract any upstream error detail the proxy might include
    let upstream_detail = data
        .get("upstream_error")
        .or_else(|| data.get("detail"))
        .or_else(|| data.get("error").and_then(|e| e.get("detail")))
        .map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                v.to_string()
            }
        });

    let mut msg = format!(
        "LLM call failed: HTTP {} {}: {}",
        status.as_u16(),
        error_code,
        description
    );

    if let Some(detail) = upstream_detail {
        msg.push_str(&format!(" | upstream: {}", detail));
    }

    // Log refund info if present
    if let Some(refund_sats) = data
        .get("excessRefund")
        .or_else(|| data.get("refund"))
        .and_then(|r| r.get("satoshis"))
        .and_then(|v| v.as_u64())
    {
        msg.push_str(&format!(" | {} sats refunded", refund_sats));
    }

    DmError::payment(msg)
}

/// Log a ThinkResult summary.
fn log_think_result(result: &ThinkResult) {
    tracing::info!(
        "think complete: {} tokens ({}+{}), {} sats paid, {} sats effective, {} sats refunded, {}ms",
        result.total_tokens,
        result.prompt_tokens,
        result.completion_tokens,
        result.sats_paid,
        result.sats_effective,
        result.sats_refunded,
        result.duration_ms,
    );
}
