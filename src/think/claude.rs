//! Claude provider — request body construction, response parsing, and format conversion.

use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::Instant;

use crate::auth::AuthriteClient;
use crate::error::DmError;

use super::openai::extract_payment_info;
use super::{
    log_think_result, parse_error_response, try_internalize_refund_from_error, ThinkResult,
};

/// Build a Claude Messages API request body.
///
/// Converts OpenAI-format messages and tools to Claude's native format:
/// - Extracts system messages into the `system` parameter
/// - Converts `role: "tool"` messages into `role: "user"` with `tool_result` content blocks
/// - Converts OpenAI tool definitions to Claude's `{ name, input_schema }` format
pub fn build_claude_body(
    messages: &[Value],
    model: &str,
    max_tokens: u32,
    temperature: Option<f64>,
    tools: Option<&[Value]>,
    thinking_budget: Option<u32>,
) -> Value {
    let (system_prompt, claude_messages) = convert_messages_for_claude(messages);

    let mut body = json!({
        "model": model,
        "messages": claude_messages,
        "max_tokens": max_tokens,
    });

    if let Some(sys) = &system_prompt {
        if !sys.is_empty() {
            body["system"] = json!(sys);
        }
    }

    if let Some(temp) = temperature {
        body["temperature"] = json!(temp);
    }

    if let Some(tool_defs) = tools {
        if !tool_defs.is_empty() {
            let claude_tools = convert_tools_for_claude(tool_defs);
            body["tools"] = json!(claude_tools);
            body["tool_choice"] = json!({"type": "auto"});
        }
    }

    if let Some(budget) = thinking_budget {
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
        // Anthropic requires max_tokens >= budget_tokens + 1024 when thinking is enabled
        let min_max = budget + 1024;
        if max_tokens < min_max {
            body["max_tokens"] = json!(min_max);
        }
    }

    body
}

/// Convert OpenAI-format messages to Claude format.
///
/// Returns (system_prompt, messages) where:
/// - System messages are extracted and concatenated into a single string
/// - `role: "tool"` messages are converted to `role: "user"` with `tool_result` content blocks
/// - Assistant messages with `tool_calls` are converted to have `tool_use` content blocks
pub fn convert_messages_for_claude(messages: &[Value]) -> (Option<String>, Vec<Value>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut claude_messages: Vec<Value> = Vec::new();

    // Collect consecutive tool results to group into a single user message
    let mut pending_tool_results: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");

        match role {
            "system" => {
                if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                    system_parts.push(content.to_string());
                }
            }
            "user" => {
                // Flush any pending tool results first
                flush_tool_results(&mut claude_messages, &mut pending_tool_results);

                // Handle multimodal content: content can be a string or an array of content blocks.
                // When it's an array (from user attachments), convert image_url blocks to Claude's
                // native image format and pass through text blocks.
                if let Some(content_array) = msg.get("content").and_then(|v| v.as_array()) {
                    let mut claude_blocks: Vec<Value> = Vec::new();
                    for block in content_array {
                        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                if !text.is_empty() {
                                    claude_blocks.push(json!({"type": "text", "text": text}));
                                }
                            }
                            "image_url" => {
                                // Convert OpenAI image_url format to Claude image format.
                                // OpenAI: { type: "image_url", image_url: { url: "data:image/png;base64,..." } }
                                // Claude: { type: "image", source: { type: "base64", media_type: "image/png", data: "..." } }
                                if let Some(url) = block
                                    .get("image_url")
                                    .and_then(|v| v.get("url"))
                                    .and_then(|v| v.as_str())
                                {
                                    if let Some(rest) = url.strip_prefix("data:") {
                                        if let Some((media_type, b64)) = rest.split_once(";base64,")
                                        {
                                            claude_blocks.push(json!({
                                                "type": "image",
                                                "source": {
                                                    "type": "base64",
                                                    "media_type": media_type,
                                                    "data": b64,
                                                }
                                            }));
                                        }
                                    } else {
                                        // URL-based image — pass as-is (Claude supports URL sources)
                                        claude_blocks.push(json!({
                                            "type": "image",
                                            "source": {
                                                "type": "url",
                                                "url": url,
                                            }
                                        }));
                                    }
                                }
                            }
                            _ => {
                                // Pass through unknown block types
                                claude_blocks.push(block.clone());
                            }
                        }
                    }
                    if claude_blocks.is_empty() {
                        claude_blocks.push(json!({"type": "text", "text": "[empty]"}));
                    }
                    claude_messages.push(json!({
                        "role": "user",
                        "content": claude_blocks,
                    }));
                } else {
                    let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    // Claude rejects empty user content
                    let content = if content.is_empty() {
                        "[empty]"
                    } else {
                        content
                    };
                    claude_messages.push(json!({
                        "role": "user",
                        "content": content,
                    }));
                }
            }
            "assistant" => {
                // Flush any pending tool results first
                flush_tool_results(&mut claude_messages, &mut pending_tool_results);

                // Convert assistant message: text + optional tool_calls → content blocks
                let mut content_blocks: Vec<Value> = Vec::new();

                if let Some(text) = msg.get("content").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        content_blocks.push(json!({
                            "type": "text",
                            "text": text,
                        }));
                    }
                }

                if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let empty_fn = json!({});
                        let function = tc.get("function").unwrap_or(&empty_fn);
                        let name = function.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args_str = function
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                        content_blocks.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }));
                    }
                }

                if content_blocks.is_empty() {
                    // Empty assistant message — Claude rejects empty text content blocks.
                    // Use a placeholder to maintain required user/assistant alternation.
                    content_blocks.push(json!({
                        "type": "text",
                        "text": "[continued]",
                    }));
                }

                claude_messages.push(json!({
                    "role": "assistant",
                    "content": content_blocks,
                }));
            }
            "tool" => {
                // Collect tool results — they'll be grouped into a single user message
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                }));
            }
            _ => {
                // Unknown role — skip
                tracing::warn!("Skipping unknown message role for Claude conversion: {role}");
            }
        }
    }

    // Flush any remaining tool results
    flush_tool_results(&mut claude_messages, &mut pending_tool_results);

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, claude_messages)
}

/// Flush pending tool results into a single user message with tool_result content blocks.
pub fn flush_tool_results(messages: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "user",
        "content": std::mem::take(pending),
    }));
}

/// Convert OpenAI function-calling tool definitions to Claude format.
///
/// OpenAI: `{ "type": "function", "function": { "name", "description", "parameters" } }`
/// Claude: `{ "name", "description", "input_schema" }`
pub fn convert_tools_for_claude(openai_tools: &[Value]) -> Vec<Value> {
    openai_tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let name = function.get("name")?.as_str()?;
            let description = function
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let parameters = function.get("parameters").cloned().unwrap_or(json!({
                "type": "object",
                "properties": {},
            }));

            Some(json!({
                "name": name,
                "description": description,
                "input_schema": parameters,
            }))
        })
        .collect()
}

/// Parse a Claude Messages API response into a ThinkResult.
///
/// Claude response format:
/// ```json
/// {
///   "content": [{"type": "text", "text": "..."}, {"type": "tool_use", ...}],
///   "stop_reason": "end_turn" | "tool_use",
///   "usage": {"input_tokens": N, "output_tokens": N},
///   "model": "claude-...",
///   "payment": {...},
///   "excessRefund": {...}
/// }
/// ```
///
/// Tool use blocks are normalized to OpenAI tool_calls format so the runner
/// can process them identically.
pub(super) async fn parse_claude_response(
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
            "Claude endpoint returned empty response (HTTP {}). \
             This usually means the request was rejected before processing — \
             check if the payment was too large for HTTP headers.",
            status.as_u16()
        )));
    }

    let data: Value = serde_json::from_slice(body).map_err(|e| {
        let preview = String::from_utf8_lossy(&body[..body.len().min(200)]);
        tracing::error!("Non-JSON Claude response ({}B): {}", body.len(), preview);
        DmError::payment(format!(
            "Claude response is not valid JSON: {e}\n  Response preview ({} bytes): {preview}",
            body.len()
        ))
    })?;

    if !status.is_success() {
        // Attempt to internalize any refund before returning the error
        try_internalize_refund_from_error(&data, auth).await;
        return Err(parse_error_response(&data, status));
    }

    // Claude returns content as an array of blocks
    let content_blocks = data
        .get("content")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Extract text from text blocks
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    for block in &content_blocks {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    text_parts.push(t.to_string());
                }
            }
            "tool_use" => {
                // Normalize to OpenAI tool_calls format
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = block.get("input").cloned().unwrap_or(json!({}));

                // Serialize input back to string (OpenAI format stores arguments as string)
                let arguments_str = serde_json::to_string(&input).unwrap_or_else(|_| "{}".into());

                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments_str,
                    },
                }));
            }
            "thinking" => {
                // Extended thinking block — log but do NOT include in ThinkResult.text.
                // The thinking tokens are already counted in usage.output_tokens by the API.
                if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                    tracing::debug!(
                        "Claude extended thinking ({} chars): {}",
                        t.len(),
                        if t.len() > 200 { &t[..200] } else { t }
                    );
                }
            }
            _ => {
                tracing::debug!("Ignoring unknown Claude content block type: {block_type}");
            }
        }
    }

    let text = text_parts.join("\n");

    // Map stop_reason to finish_reason
    let finish_reason = data
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(|sr| match sr {
            "end_turn" => "stop",
            "tool_use" => "tool_calls",
            "max_tokens" => "length",
            "stop_sequence" => "stop",
            other => other,
        })
        .unwrap_or("")
        .to_string();

    // Extract usage stats (Claude uses input_tokens/output_tokens)
    let empty_obj = json!({});
    let usage = data.get("usage").unwrap_or(&empty_obj);
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_tokens = prompt_tokens + completion_tokens;

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
