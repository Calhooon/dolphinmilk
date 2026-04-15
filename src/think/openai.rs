//! OpenAI provider — request body construction and response parsing.

use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::Instant;

use crate::auth::AuthriteClient;
use crate::error::DmError;
use crate::x402::refund;

use super::{
    is_reasoning_model, log_think_result, parse_error_response, try_internalize_refund_from_error,
    ThinkResult,
};

/// Build an OpenAI chat completions request body.
pub fn build_openai_body(
    messages: &[Value],
    model: &str,
    max_tokens: u32,
    temperature: Option<f64>,
    tools: Option<&[Value]>,
    reasoning_effort: Option<&str>,
) -> Value {
    let is_reasoning = is_reasoning_model(model);
    let mut body = json!({
        "model": model,
        "messages": messages,
    });
    if is_reasoning {
        body["max_completion_tokens"] = json!(max_tokens);
        // Also send max_tokens so x402 proxies that only read this field
        // use the correct value for pricing and upstream forwarding.
        body["max_tokens"] = json!(max_tokens);
    } else {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = temperature {
        if !is_reasoning {
            body["temperature"] = json!(temp);
        }
    }
    if let Some(tool_defs) = tools {
        if !tool_defs.is_empty() {
            body["tools"] = json!(tool_defs);
            body["tool_choice"] = json!("auto");
        }
    }
    if let Some(effort) = reasoning_effort {
        if is_reasoning {
            body["reasoning_effort"] = json!(effort);
        }
    }
    body
}

/// Parse an OpenAI chat completions response into a ThinkResult.
pub(super) async fn parse_openai_response(
    body: &[u8],
    status: StatusCode,
    auth: &AuthriteClient,
    model: &str,
    t0: Instant,
    payment_txid: Option<String>,
) -> Result<ThinkResult, DmError> {
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    if body.is_empty() {
        return Err(DmError::payment(format!(
            "LLM endpoint returned empty response (HTTP {}). \
             This usually means the request was rejected before processing — \
             check if the payment was too large for HTTP headers.",
            status.as_u16()
        )));
    }

    let data: Value = serde_json::from_slice(body).map_err(|e| {
        let preview = String::from_utf8_lossy(&body[..body.len().min(200)]);
        tracing::error!("Non-JSON OpenAI response ({}B): {}", body.len(), preview);
        DmError::payment(format!(
            "LLM response is not valid JSON: {e}\n  Response preview ({} bytes): {preview}",
            body.len()
        ))
    })?;

    if !status.is_success() {
        // Attempt to internalize any refund before returning the error
        try_internalize_refund_from_error(&data, auth).await;
        return Err(parse_error_response(&data, status));
    }

    // Extract the assistant's reply
    let choices = data.get("choices").and_then(|v| v.as_array());
    let choice = choices
        .and_then(|c| c.first())
        .ok_or_else(|| DmError::payment("LLM response has no choices"))?;

    let empty_obj = json!({});
    let message = choice.get("message").unwrap_or(&empty_obj);
    let text = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Extract tool calls if present
    let tool_calls = message
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Extract usage stats
    let usage = data.get("usage").unwrap_or(&empty_obj);
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Extract payment + refund info
    let (sats_paid, sats_effective, sats_refunded, refund_internalized) =
        extract_payment_info(&data, auth).await;

    let result = ThinkResult {
        text,
        model: data
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(model)
            .to_string(),
        sats_paid,
        sats_effective,
        sats_refunded,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        finish_reason,
        duration_ms: elapsed_ms,
        tool_calls,
        payment_txid,
        refund_internalized,
        was_text_extracted: false,
    };

    log_think_result(&result);
    Ok(result)
}

/// Extract payment and refund info from an x402 response (works for both providers).
/// Returns (sats_paid, sats_effective, sats_refunded, refund_internalized).
pub(super) async fn extract_payment_info(
    data: &Value,
    auth: &AuthriteClient,
) -> (u64, u64, u64, Option<bool>) {
    let empty_obj = json!({});
    let payment_info = data.get("payment").unwrap_or(&empty_obj);
    let sats_paid = payment_info
        .get("satoshis_paid")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let sats_effective = payment_info
        .get("satoshis_effective")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut sats_refunded = 0u64;
    let mut refund_internalized: Option<bool> = None;
    if let Some(excess_refund) = data.get("excessRefund") {
        if let Some(sat) = excess_refund.get("satoshis").and_then(|v| v.as_u64()) {
            sats_refunded = sat;
        }
        if let Some(refund_info) = refund::parse_refund(data) {
            match refund::process_refund(auth.wallet_api(), &refund_info).await {
                Ok(_) => {
                    tracing::info!("Excess refund internalized: {} sats", sats_refunded);
                    refund_internalized = Some(true);
                }
                Err(e) => {
                    tracing::warn!("Failed to internalize excess refund: {e}");
                    refund_internalized = Some(false);
                }
            }
        }
    }

    (
        sats_paid,
        sats_effective,
        sats_refunded,
        refund_internalized,
    )
}
