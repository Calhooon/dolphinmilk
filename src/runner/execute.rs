//! Tool execution — parallel and sequential dispatch, offloading, proofs.
//!
//! Extracted from `step.rs` (#208). All methods remain `impl DmLoop`.

use std::sync::Arc;

use serde_json::Value;

use crate::error::DmError;
use crate::events::StepEvent;
use crate::proofs;
use crate::sanitize;

use crate::tools::registry::ToolRegistry;

use super::step::StepContext;
use super::{content_hash, emit_event, DmLoop};

/// Metadata captured from a successful `send_message` tool call for BRC-18 proof creation.
struct MessageSendInfo {
    hash: String,
    recipient: String,
    box_name: String,
    sats: u64,
    signed: bool,
    encrypted: bool,
}

/// Tool results larger than this (in bytes) are offloaded to workspace files.
/// The LLM receives a persisted-output preview instead of the full content.
///
/// Threshold was raised from 8K to 50K on 2026-04-13 to match claude-code's
/// proven default. At 8K, nearly every real web response (Reddit, HN, Twitter,
/// GitHub) got offloaded and the LLM had to navigate an offload preview just
/// to read an API call — a capability round-trip that was costing budget and
/// triggering injection-defense reflexes in smaller models. At 50K, ~95% of
/// real API responses fit inline with headroom to spare, offloading is rare
/// enough to be a proper exceptional case, and the structural preview format
/// below handles the remaining large cases well.
pub(crate) const TOOL_RESULT_OFFLOAD_THRESHOLD: usize = 50_000;

/// Tools whose results should NEVER be offloaded regardless of size.
/// These are "retrieval" tools — the agent explicitly asked for this data.
/// Offloading them would create a cascading loop: preview says "use file_read"
/// → file_read result gets offloaded → another preview → infinite regression.
const NO_OFFLOAD_TOOLS: &[&str] = &[
    "file_read",
    "memory_search",
    "memory_recall",
    "read_tool_output",
    "execute_bash",
];

/// Tools that make paid x402 requests. Only these should have sats_paid
/// extracted from their output JSON. Other tools (e.g., file_read) may
/// return content that incidentally contains a "sats_paid" field.
const PAID_TOOLS: &[&str] = &["x402_call", "generate_image", "upload_to_nanostore"];

/// System files that should NOT appear in artifact manifests.
const SYSTEM_FILES: &[&str] = &[
    "session.jsonl",
    "budget.jsonl",
    "artifacts.json",
    "fork_context.json",
];

/// System directories that should NOT be scanned for artifacts.
/// System directories excluded from artifact scanning (uploads/ is the exception — it's scanned).
#[allow(dead_code)]
const SYSTEM_DIRS: &[&str] = &["beef", "continuations", "delivery_queue", "screenshots"];

/// Scan workspace for new files and update artifacts.json manifest.
fn update_artifact_manifest(
    workspace: &std::path::Path,
    tool_results: &[(String, String, String)],
) {
    let manifest_path = workspace.join("artifacts.json");

    // Read existing manifest
    let mut artifacts: Vec<serde_json::Value> = if manifest_path.exists() {
        std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Track already-known files
    let known_files: std::collections::HashSet<String> = artifacts
        .iter()
        .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();

    // Scan workspace for files
    let entries = match std::fs::read_dir(workspace) {
        Ok(e) => e,
        Err(_) => return,
    };

    let system_files: std::collections::HashSet<&str> = SYSTEM_FILES.iter().copied().collect();

    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip system files
        if system_files.contains(file_name.as_str()) {
            continue;
        }
        if file_name.starts_with('.') {
            continue;
        }
        if file_name.starts_with("tool_output_") {
            continue;
        }

        // Handle directories: skip system dirs, scan uploads/ for user attachments
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                // Scan uploads/ for user-supplied attachments (images, files)
                if file_name == "uploads" {
                    if let Ok(uploads) = std::fs::read_dir(entry.path()) {
                        for upload in uploads.flatten() {
                            let uname = upload.file_name().to_string_lossy().to_string();
                            if uname.starts_with('.') || known_files.contains(&uname) {
                                continue;
                            }
                            let umeta = match upload.metadata() {
                                Ok(m) => m,
                                Err(_) => continue,
                            };
                            let uext = std::path::Path::new(&uname)
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_lowercase();
                            let utype = match uext.as_str() {
                                "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" => "image",
                                "pdf" => "document",
                                _ => "file",
                            };
                            artifacts.push(serde_json::json!({
                                "name": uname,
                                "type": utype,
                                "size_bytes": umeta.len(),
                                "created_by": "user_upload",
                                "created_at": umeta.modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_secs_f64())
                                    .unwrap_or(0.0),
                            }));
                        }
                    }
                }
                // Skip all directories (including system dirs like beef, screenshots, etc.)
                continue;
            }
        }

        // Skip if already known
        if known_files.contains(&file_name) {
            continue;
        }

        // Get file metadata
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Determine artifact type from extension
        let ext = std::path::Path::new(&file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let artifact_type = match ext.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" => "image",
            "mp4" | "webm" | "mov" | "avi" => "video",
            "mp3" | "wav" | "ogg" | "m4a" | "flac" => "audio",
            "pdf" => "document",
            _ => "file",
        };

        // Try to find which tool created this file by checking tool_results
        let mut created_by = String::new();
        for (_, tool_name, output) in tool_results {
            if output.contains(&file_name) {
                created_by = tool_name.clone();
                break;
            }
        }

        artifacts.push(serde_json::json!({
            "name": file_name,
            "type": artifact_type,
            "size_bytes": metadata.len(),
            "created_by": created_by,
            "created_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
        }));
    }

    // Write manifest
    if let Ok(json) = serde_json::to_string_pretty(&artifacts) {
        let _ = std::fs::write(&manifest_path, json);
    }
}

/// Result of a single tool execution: (index, call_id, name, arguments_str, arguments, output, success).
pub(crate) type ToolExecResult = (usize, String, String, String, Value, String, bool);

// ---------------------------------------------------------------------------
// Persisted-output preview (the "god-tier" format)
// ---------------------------------------------------------------------------
//
// When a tool result exceeds TOOL_RESULT_OFFLOAD_THRESHOLD we persist it to
// disk and hand the LLM a compact preview instead. The preview must serve
// four distinct scenarios simultaneously:
//
//   (1) "I can answer from the preview"     — structural summary + exemplars
//   (2) "I need a specific nested part"     — explicit retrieval syntax
//   (3) "I need to understand the shape"    — recursive schema map
//   (4) "Something's wrong, diagnose it"    — raw byte prefix fallback
//
// The format is:
//
//   <persisted-output>
//   Size: N bytes (offloaded)
//   Retrieve full output: read_tool_output call_id="X" (or file_read path="Y")
//
//   Structure:
//     <recursive JSON summary bounded by PREVIEW_MAX_DEPTH /
//      PREVIEW_MAX_ARRAY_ITEMS / PREVIEW_MAX_OBJECT_KEYS /
//      PREVIEW_MAX_VALUE_LEN>
//
//   Raw prefix (first N chars):
//     <first PREVIEW_RAW_PREFIX_LEN chars of content>
//     ...[truncated]
//   </persisted-output>
//
// See `test_god_tier_preview_hn_response` below for the canonical example
// (a Hacker News Algolia response with titles nested under `hits` — the
// exact shape that broke the old first-N-bytes preview format).

const PREVIEW_MAX_DEPTH: usize = 3;
const PREVIEW_MAX_ARRAY_ITEMS: usize = 3;
const PREVIEW_MAX_OBJECT_KEYS: usize = 10;
const PREVIEW_MAX_VALUE_LEN: usize = 80;
const PREVIEW_RAW_PREFIX_LEN: usize = 1000;
const PREVIEW_PARSE_SIZE_CAP: usize = 1_000_000;

/// Generate a persisted-output preview for a tool result that has been
/// offloaded to disk. Bounded output regardless of input size.
///
/// `call_id` is the LLM's tool_use id; the LLM retrieves the full content
/// via `read_tool_output(call_id=...)`. `file_path` is the workspace path
/// where the full content was persisted; the LLM can also retrieve via
/// `file_read` with that path.
pub(crate) fn generate_persisted_output_preview(
    content: &str,
    file_path: &str,
    call_id: &str,
) -> String {
    let size = content.len();
    let mut out = String::with_capacity(2048);
    out.push_str("<persisted-output>\n");
    out.push_str(&format!("Size: {} bytes (offloaded)\n", size));
    out.push_str(&format!(
        "Retrieve full output: read_tool_output call_id=\"{}\" (or file_read path=\"{}\")\n\n",
        call_id, file_path
    ));

    // Structural summary — only attempt parsing for content under the size
    // cap, and only if the first non-whitespace char looks like JSON. For
    // plain text or malformed JSON, fall through to the raw prefix with a
    // word count header.
    let trimmed = content.trim_start();
    let looks_like_json = trimmed.starts_with('{') || trimmed.starts_with('[');
    let structural: Option<String> = if looks_like_json && size <= PREVIEW_PARSE_SIZE_CAP {
        serde_json::from_str::<Value>(content)
            .ok()
            .map(|v| format_structural_summary(&v, 0))
    } else {
        None
    };

    if let Some(summary) = structural {
        out.push_str("Structure:\n");
        out.push_str(&summary);
        if !summary.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    } else {
        let word_count = content.split_whitespace().count();
        out.push_str(&format!(
            "Content: {} words (text or unparseable)\n\n",
            word_count
        ));
    }

    // Raw prefix fallback — always included. Errors typically surface in
    // the first bytes of a failed response, and this fallback gives the
    // LLM direct byte-level visibility into those cases regardless of
    // whether structural parsing succeeded.
    out.push_str(&format!(
        "Raw prefix (first {} chars):\n",
        PREVIEW_RAW_PREFIX_LEN
    ));
    let prefix: String = content.chars().take(PREVIEW_RAW_PREFIX_LEN).collect();
    out.push_str(&prefix);
    if content.chars().count() > PREVIEW_RAW_PREFIX_LEN {
        out.push_str("\n...[truncated]\n");
    } else {
        out.push('\n');
    }
    out.push_str("</persisted-output>");
    out
}

/// Recursive structural summary of a JSON value. Each call descends one
/// level; recursion stops at PREVIEW_MAX_DEPTH.
fn format_structural_summary(value: &Value, depth: usize) -> String {
    if depth >= PREVIEW_MAX_DEPTH {
        return format!("{}<...>", indent_for(depth));
    }
    match value {
        Value::Object(obj) => format_object_summary(obj, depth),
        Value::Array(arr) => format_array_summary(arr, depth),
        _ => format!("{}{}", indent_for(depth), format_leaf_value(value)),
    }
}

/// Summarize a JSON object: fields header + descend into interesting child
/// collections (arrays or nested objects) one level deeper.
fn format_object_summary(obj: &serde_json::Map<String, Value>, depth: usize) -> String {
    let ind = indent_for(depth);
    let keys: Vec<&String> = obj.keys().take(PREVIEW_MAX_OBJECT_KEYS).collect();
    let more_keys = obj.len().saturating_sub(keys.len());

    // Header line: {object, N fields: key1(obj), key2(array), key3, ...}
    let mut schema_parts: Vec<String> = Vec::with_capacity(keys.len());
    for k in &keys {
        let v = &obj[*k];
        let shape = shape_hint(v);
        schema_parts.push(format!("{}{}", k, shape));
    }
    let mut header = format!(
        "{}{{object, {} fields: {}",
        ind,
        obj.len(),
        schema_parts.join(", ")
    );
    if more_keys > 0 {
        header.push_str(&format!(", +{} more", more_keys));
    }
    header.push('}');

    // Body: descend one level into interesting collections. Prefer arrays
    // because they almost always hold the payload data.
    let mut body = String::new();
    if depth + 1 < PREVIEW_MAX_DEPTH {
        for k in &keys {
            let v = &obj[*k];
            match v {
                Value::Array(arr) if !arr.is_empty() => {
                    body.push('\n');
                    body.push_str(&indent_for(depth + 1));
                    body.push_str(&format!("{}: ", k));
                    body.push_str(&format_array_inline(arr, depth + 1));
                }
                Value::Object(inner) if !inner.is_empty() => {
                    body.push('\n');
                    body.push_str(&indent_for(depth + 1));
                    body.push_str(&format!("{}: ", k));
                    let inner_summary = format_object_summary(inner, depth + 1);
                    // Strip the leading indent of the inner summary since we
                    // already printed the key+colon prefix on the same line.
                    let trimmed = inner_summary.trim_start();
                    body.push_str(trimmed);
                }
                _ => {}
            }
        }
    }
    header + &body
}

/// Summarize a top-level JSON array: cardinality + shape of items + up to
/// PREVIEW_MAX_ARRAY_ITEMS exemplars.
fn format_array_summary(arr: &[Value], depth: usize) -> String {
    let ind = indent_for(depth);
    let mut out = format!("{}array[{}]", ind, arr.len());
    if let Some(first) = arr.first() {
        out.push_str(&format!(" of {}", shape_name(first)));
    }
    // If items are objects, show field schema from the first item.
    if let Some(Value::Object(first_obj)) = arr.first() {
        let fields: Vec<&String> = first_obj.keys().take(PREVIEW_MAX_OBJECT_KEYS).collect();
        let more = first_obj.len().saturating_sub(fields.len());
        let field_names: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
        out.push_str(&format!(" | fields: {}", field_names.join(", ")));
        if more > 0 {
            out.push_str(&format!(", +{} more", more));
        }
    }
    for (i, item) in arr.iter().take(PREVIEW_MAX_ARRAY_ITEMS).enumerate() {
        out.push('\n');
        out.push_str(&indent_for(depth + 1));
        out.push_str(&format!("[{}] {}", i, format_item_compact(item)));
    }
    if arr.len() > PREVIEW_MAX_ARRAY_ITEMS {
        out.push('\n');
        out.push_str(&indent_for(depth + 1));
        out.push_str(&format!(
            "...({} more items)",
            arr.len() - PREVIEW_MAX_ARRAY_ITEMS
        ));
    }
    out
}

/// Summarize an array when it appears inline as a child of an object (e.g.
/// `{hits: array(20) of objects | fields: ... [0] ... [1] ... [2] ...}`).
fn format_array_inline(arr: &[Value], depth: usize) -> String {
    let mut out = format!(
        "array({}) of {}",
        arr.len(),
        if let Some(first) = arr.first() {
            shape_name(first)
        } else {
            "unknown".to_string()
        }
    );
    if let Some(Value::Object(first_obj)) = arr.first() {
        let fields: Vec<&String> = first_obj.keys().take(PREVIEW_MAX_OBJECT_KEYS).collect();
        let more = first_obj.len().saturating_sub(fields.len());
        let field_names: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
        out.push_str(&format!(" | fields: {}", field_names.join(", ")));
        if more > 0 {
            out.push_str(&format!(", +{} more", more));
        }
        for (i, item) in arr.iter().take(PREVIEW_MAX_ARRAY_ITEMS).enumerate() {
            out.push('\n');
            out.push_str(&indent_for(depth + 1));
            out.push_str(&format!("[{}] {}", i, format_item_compact(item)));
        }
        if arr.len() > PREVIEW_MAX_ARRAY_ITEMS {
            out.push('\n');
            out.push_str(&indent_for(depth + 1));
            out.push_str(&format!(
                "...({} more)",
                arr.len() - PREVIEW_MAX_ARRAY_ITEMS
            ));
        }
    }
    out
}

/// Compact single-line representation of an item. For objects, prefers
/// human-meaningful fields (title, name, id, ...) and shows up to 4. For
/// arrays and scalars, shows shape/value.
fn format_item_compact(item: &Value) -> String {
    match item {
        Value::Object(obj) => {
            // Prefer "human-interesting" fields, in priority order.
            let priority = [
                "title", "name", "id", "key", "summary", "text", "label", "url", "author",
            ];
            let mut shown: Vec<String> = Vec::new();
            for p in priority {
                if shown.len() >= 4 {
                    break;
                }
                if let Some(v) = obj.get(p) {
                    if !matches!(v, Value::Object(_) | Value::Array(_)) {
                        shown.push(format!("{}={}", p, format_leaf_value(v)));
                    }
                }
            }
            // Fill from remaining scalar fields if we haven't hit 3 yet.
            if shown.len() < 3 {
                for (k, v) in obj.iter().take(PREVIEW_MAX_OBJECT_KEYS) {
                    if shown.len() >= 4 {
                        break;
                    }
                    if shown.iter().any(|s| s.starts_with(&format!("{}=", k))) {
                        continue;
                    }
                    if !matches!(v, Value::Object(_) | Value::Array(_)) {
                        shown.push(format!("{}={}", k, format_leaf_value(v)));
                    }
                }
            }
            format!("{{{}}}", shown.join(", "))
        }
        Value::Array(arr) => format!("array({})", arr.len()),
        _ => format_leaf_value(item),
    }
}

/// Short type hint emitted next to object keys: `hits(array)`, `meta(obj)`,
/// or nothing for scalars.
fn shape_hint(v: &Value) -> &'static str {
    match v {
        Value::Object(_) => "(obj)",
        Value::Array(_) => "(array)",
        _ => "",
    }
}

/// Short type description for array-of-X headers.
fn shape_name(v: &Value) -> String {
    match v {
        Value::Object(obj) => format!("objects ({} fields each)", obj.len()),
        Value::Array(inner) => format!("arrays({})", inner.len()),
        Value::String(_) => "strings".to_string(),
        Value::Number(_) => "numbers".to_string(),
        Value::Bool(_) => "bools".to_string(),
        Value::Null => "nulls".to_string(),
    }
}

/// Render a leaf value (string, number, bool, null) as a short string with
/// length cap. Strings get quoted.
fn format_leaf_value(v: &Value) -> String {
    let s = match v {
        Value::String(s) => format!("\"{}\"", s),
        Value::Null => "null".to_string(),
        _ => v.to_string(),
    };
    if s.chars().count() > PREVIEW_MAX_VALUE_LEN {
        let truncated: String = s.chars().take(PREVIEW_MAX_VALUE_LEN).collect();
        format!("{}...", truncated)
    } else {
        s
    }
}

fn indent_for(depth: usize) -> String {
    "  ".repeat(depth)
}

/// Backwards-compat alias — older call sites that don't have a `call_id`
/// in scope. Synthesizes a placeholder. Prefer
/// `generate_persisted_output_preview` directly.
#[allow(dead_code)]
pub(crate) fn generate_smart_preview(content: &str, file_path: &str) -> String {
    generate_persisted_output_preview(content, file_path, "unknown")
}

/// Execute a tool without requiring `&DmLoop`.
///
/// Standalone function that enables parallel invocation from spawned JoinSet
/// tasks, which cannot borrow `&self`. Replicates the lookup + capability check
/// logic from `DmLoop::execute_tool`.
async fn execute_tool_standalone(
    tools: &Arc<tokio::sync::RwLock<ToolRegistry>>,
    capabilities: &Option<Vec<String>>,
    name: &str,
    arguments: Value,
) -> Result<String, DmError> {
    let future = {
        let registry = tools.read().await;
        let tool = registry
            .get(name)
            .ok_or_else(|| DmError::tool(format!("Unknown tool: {name}")))?;
        if !registry.is_allowed(name) {
            return Err(DmError::tool(format!("Tool not allowed: {name}")));
        }
        if let Some(ref caps) = capabilities {
            let required = crate::tools::registry::required_capability(&tool.category);
            if !caps.iter().any(|c| c == required || c == "all") {
                return Err(DmError::tool(format!(
                    "Insufficient capability for tool '{}' (category '{}'): requires '{}' capability",
                    name, tool.category, required
                )));
            }
        }
        (tool.execute)(arguments)
    };
    Ok(future.await)
}

impl DmLoop {
    /// ACT phase: execute tool calls, handle side effects.
    /// Returns (called_send_message, memory_ids_this_iter, message_hashes_this_iter).
    ///
    /// When multiple tool calls are present, executes them concurrently via
    /// `tokio::task::JoinSet` (IronClaw pattern). Single tool calls use the
    /// existing sequential path to avoid JoinSet overhead.
    pub(crate) async fn execute_tools(
        &mut self,
        effective_tool_calls: &[Value],
        ctx: &StepContext,
    ) -> Result<(bool, Vec<String>, Vec<String>), DmError> {
        let mut called_send_message = false;
        let mut memory_ids_this_iter: Vec<String> = Vec::new();
        let mut message_hashes_this_iter: Vec<String> = Vec::new();

        tracing::info!("Executing {} tool call(s)", effective_tool_calls.len());

        // Pre-parse all tool calls
        let parsed_calls: Vec<(String, String, String, Value)> = effective_tool_calls
            .iter()
            .map(|call| {
                let call_id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let function = call
                    .get("function")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let name = function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let arguments_str = function
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}")
                    .to_string();
                let arguments: Value =
                    serde_json::from_str(&arguments_str).unwrap_or(serde_json::json!({}));
                (call_id, name, arguments_str, arguments)
            })
            .collect();

        // Execute tool calls: parallel via JoinSet for >1, sequential for 1
        let mut exec_results: Vec<ToolExecResult> = if parsed_calls.len() > 1 {
            self.execute_tools_parallel(&parsed_calls, ctx).await
        } else {
            self.execute_tools_sequential(&parsed_calls, ctx).await
        };

        // Leak detection: redact credentials in tool outputs before transcript/LLM context
        for result in &mut exec_results {
            let redacted = sanitize::redact_leaks(&result.5);
            if redacted != result.5 {
                result.5 = redacted;
            }
        }

        // Moderate tool outputs — replace blocked content, log flagged content
        for result in &mut exec_results {
            if !result.6 {
                // Skip moderation for already-failed tool calls
                continue;
            }
            let tool_name = result.2.clone();
            let desc = format!("tool output from '{}'", tool_name);
            if self.moderate_content(&result.5, "tool_output", Some(&tool_name), &desc)
                == super::moderation::ModerationOutcome::Blocked
            {
                result.5 = "[MODERATED: content blocked by policy]".to_string();
            }
        }

        // Collect send_message metadata for BRC-18 MessageSend proofs (created after loop)
        let mut message_send_infos: Vec<MessageSendInfo> = Vec::new();

        // Collect tool result summaries for artifact manifest
        let mut tool_result_summaries: Vec<(String, String, String)> = Vec::new();

        // Process results sequentially (transcript, proofs, budget, side effects)
        for (_, call_id, name, arguments_str, arguments, output, success) in &exec_results {
            if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            // Collect tool result summary for artifact manifest (truncate output for matching)
            if *success {
                let output_snippet: String = output.chars().take(4000).collect();
                tool_result_summaries.push((call_id.clone(), name.clone(), output_snippet));
            }

            if name == "send_message" {
                called_send_message = true;
                if let Some(recipient) = arguments.get("recipient").and_then(|v| v.as_str()) {
                    self.state
                        .comms
                        .pending_replies
                        .retain(|r| r.sender_key != recipient);
                }
            }

            // EPIC #329 Phase 3 follow-up: when delegate_task succeeds, write
            // a serial → conv_id index file so the issuer's
            // commission_payment_claim handler can find the original chat
            // conversation that issued the commission and append payment
            // activity there (instead of in a separate ledger).
            //
            // The index lives at:
            //   {global_workspace}/commission_conv_index/{serial}.txt
            //
            // Symmetric to Coral's claim-side index. Best-effort, never
            // fails the tool dispatch.
            if name == "delegate_task" && *success {
                if let (Some(conv_id), Ok(parsed)) = (
                    self.current_conversation_id.as_ref(),
                    serde_json::from_str::<Value>(output),
                ) {
                    if let Some(serial) = parsed.get("serial_number").and_then(|v| v.as_str()) {
                        let index_dir = self.global_workspace.join("commission_conv_index");
                        let _ = std::fs::create_dir_all(&index_dir);
                        let index_path = index_dir.join(format!("{serial}.txt"));
                        if let Err(e) = std::fs::write(&index_path, conv_id) {
                            tracing::warn!(
                                serial,
                                conv_id,
                                "Failed to write commission_conv_index: {e}"
                            );
                        } else {
                            tracing::debug!(
                                serial,
                                conv_id,
                                "Wrote commission_conv_index for issuer payment surfacing"
                            );
                        }
                    }
                }
            }

            let is_send_message = name == "send_message";

            // Only extract sats_paid from known paid tools. Other tools
            // (e.g., file_read) may return content that incidentally contains
            // a "sats_paid" field — extracting it would double-count costs.
            let tool_sats_paid = if *success && PAID_TOOLS.contains(&name.as_str()) {
                let paid = serde_json::from_str::<Value>(output)
                    .ok()
                    .and_then(|j| j.get("sats_paid").and_then(|v| v.as_u64()))
                    .unwrap_or(0);
                if paid == 0 {
                    tracing::warn!(
                        "Paid tool '{}' returned sats_paid=0 — possible payment tracking gap (call_id={})",
                        name, call_id
                    );
                }
                paid
            } else {
                0
            };

            self.transcript
                .record_tool_result(call_id, name, output, *success, tool_sats_paid);

            // Record Prometheus tool call metric
            if let Some(ref m) = self.metrics {
                m.tool_calls_total.with_label_values(&[name]).inc();
                if tool_sats_paid > 0 {
                    m.budget_spent_sats
                        .with_label_values(&["tool"])
                        .inc_by(tool_sats_paid as f64);
                }
                if !*success {
                    m.errors_total.with_label_values(&["tool_failure"]).inc();
                }
            }

            // Track cross-agent message hashes for proof chain linking
            // and capture metadata for BRC-18 MessageSend proofs
            if is_send_message && *success {
                if let Ok(json) = serde_json::from_str::<Value>(output) {
                    if let Some(hash) = json.get("message_hash").and_then(|v| v.as_str()) {
                        message_hashes_this_iter.push(hash.to_string());

                        // Capture send metadata for BRC-18 proof creation after loop
                        let recipient = arguments
                            .get("recipient")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let box_name = arguments
                            .get("message_box")
                            .and_then(|v| v.as_str())
                            .unwrap_or("status_inbox")
                            .to_string();
                        let sats = json.get("sats_paid").and_then(|v| v.as_u64()).unwrap_or(0);
                        let signed = arguments
                            .get("sign")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let encrypted = arguments
                            .get("encrypt")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        message_send_infos.push(MessageSendInfo {
                            hash: hash.to_string(),
                            recipient,
                            box_name,
                            sats,
                            signed,
                            encrypted,
                        });
                    }
                }
            }

            // Record memory_stored event
            if name == "memory_store" && *success {
                if let Ok(json) = serde_json::from_str::<Value>(output) {
                    let memory_id = json.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let category = json.get("category").and_then(|v| v.as_str()).unwrap_or("");
                    let tags: Vec<String> = json
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|t| t.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    if tags.contains(&crate::memory::store::IDENTITY_TAG.to_string()) {
                        tracing::info!("Identity updated — creating proof");
                    }
                    let content_preview: String = json
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .chars()
                        .take(100)
                        .collect();
                    if !memory_id.is_empty() {
                        self.transcript.record_memory_stored(
                            memory_id,
                            category,
                            &tags,
                            &content_preview,
                        );
                        let content_for_hash =
                            json.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        let memory_content_hash = content_hash(content_for_hash);
                        let proof_data = format!(
                            "ENTRIES: 1\nHASHES: {}\nMEMORY_ID: {}\nCATEGORY: {}",
                            memory_content_hash, memory_id, category
                        );
                        let commitment = proofs::ProofCommitment::new(
                            proofs::ProofType::MemoryCommitment,
                            &proof_data,
                            self.state.onchain.last_proof_hash.as_deref(),
                        );
                        self.record_proof(
                            commitment,
                            "memory_commitment",
                            Some(serde_json::json!({"memory_id": memory_id})),
                        )
                        .await;
                    }
                }
            }

            // Track memory_search results for Decision proof enrichment
            if name == "memory_search" && *success {
                if let Ok(json) = serde_json::from_str::<Value>(output) {
                    if let Some(results) = json.get("results").and_then(|v| v.as_array()) {
                        for entry in results {
                            if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
                                memory_ids_this_iter.push(id.to_string());
                            }
                        }
                    }
                }
            }

            // Offload large tool results to workspace files.
            // Skip retrieval tools — the agent explicitly asked for this data,
            // offloading would create a cascading preview loop.
            if output.len() > TOOL_RESULT_OFFLOAD_THRESHOLD
                && !NO_OFFLOAD_TOOLS.contains(&name.as_str())
            {
                let safe_name: String = name
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let file_path = format!(
                    "{}/tool_output_{}_{}.json",
                    self.workspace.display(),
                    safe_name,
                    self.state.storage.offload_counter
                );
                self.state.storage.offload_counter += 1;
                match std::fs::write(&file_path, output.as_bytes()) {
                    Ok(()) => {
                        let preview =
                            generate_persisted_output_preview(output, &file_path, call_id);
                        self.transcript.record_system(&format!(
                            "Tool result from '{}' offloaded to {} ({} bytes → preview). Retrieve via read_tool_output call_id=\"{}\" or file_read.",
                            name, file_path, output.len(), call_id
                        ));
                        self.state
                            .storage
                            .offloaded_results
                            .insert(call_id.clone(), (file_path, preview));
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to offload tool result for '{}' to {}: {}",
                            name,
                            file_path,
                            e
                        );
                    }
                }
            }

            emit_event(
                ctx,
                StepEvent::ToolCallComplete {
                    iteration: self.state.exec.iteration,
                    call_id: call_id.clone(),
                    name: name.clone(),
                    output: output.clone(),
                    success: *success,
                },
            );

            // Update state + budget tracker
            if *success {
                if let Ok(json) = serde_json::from_str::<Value>(output) {
                    let sats = json.get("sats_paid").and_then(|v| v.as_u64()).unwrap_or(0);
                    let txid = json
                        .get("payment_txid")
                        .or_else(|| json.get("txid"))
                        .and_then(|v| v.as_str());
                    if sats > 0 || txid.is_some() {
                        self.state.budget.sats_spent += sats;
                        self.budget_tracker.record(
                            "tool", name, sats,
                            serde_json::json!({"txid": txid, "sats_paid": sats, "task_id": self.state.storage.task_id}),
                        );
                    }

                    if sats > 0 {
                        let args_hash = content_hash(arguments_str);
                        let result_hash = content_hash(output);
                        let commitment = proofs::capability_proof(
                            name,
                            &args_hash,
                            &result_hash,
                            sats,
                            txid,
                            self.state.onchain.last_proof_hash.as_deref(),
                        );
                        self.record_proof(commitment, "capability_proof", None)
                            .await;
                    }

                    if sats == 0 && (name == "discover_endpoints" || name == "discover_services") {
                        let args_hash = content_hash(arguments_str);
                        let result_hash = content_hash(output);
                        let commitment = proofs::capability_proof(
                            name,
                            &args_hash,
                            &result_hash,
                            0,
                            None,
                            self.state.onchain.last_proof_hash.as_deref(),
                        );
                        self.record_proof(commitment, "capability_proof", None)
                            .await;
                    }
                }
            }

            // Detect continue_task
            if name == "continue_task" && *success {
                if let Ok(result_json) = serde_json::from_str::<Value>(output) {
                    if let Some(cont_id) =
                        result_json.get("continuation_id").and_then(|v| v.as_str())
                    {
                        let cont_path =
                            self.workspace.join(format!("continuations/{cont_id}.json"));
                        if let Ok(content) = std::fs::read_to_string(&cont_path) {
                            if let Ok(mut state) = serde_json::from_str::<Value>(&content) {
                                if let Some(obj) = state.as_object_mut() {
                                    obj.insert(
                                        "task".into(),
                                        Value::String(self.state.storage.task.clone()),
                                    );
                                    obj.insert(
                                        "transcript_path".into(),
                                        Value::String(
                                            self.transcript.path.to_string_lossy().to_string(),
                                        ),
                                    );
                                    obj.insert(
                                        "iteration".into(),
                                        Value::Number(self.state.exec.iteration.into()),
                                    );
                                    if let Some(ref hash) = self.state.onchain.last_proof_hash {
                                        obj.insert(
                                            "last_proof_hash".into(),
                                            Value::String(hash.clone()),
                                        );
                                    }
                                }
                                let _ = std::fs::write(
                                    &cont_path,
                                    serde_json::to_string_pretty(&state).unwrap_or_default(),
                                );
                                if let Some(tasks_dir) = self.workspace.parent() {
                                    if let Some(global_ws) = tasks_dir.parent() {
                                        let global_cont_dir = global_ws.join("continuations");
                                        let _ = std::fs::create_dir_all(&global_cont_dir);
                                        let global_path =
                                            global_cont_dir.join(format!("{cont_id}.json"));
                                        let _ = std::fs::write(
                                            &global_path,
                                            serde_json::to_string_pretty(&state)
                                                .unwrap_or_default(),
                                        );
                                    }
                                }
                            }
                        }
                        let wake_at = result_json.get("resume_at").and_then(|v| v.as_str());
                        let reason = result_json
                            .get("continuation_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        self.transcript
                            .record_continuation_save(cont_id, reason, wake_at);

                        self.state.exec.done = true;
                        self.state.exec.result = format!("Task paused for continuation: {cont_id}");

                        emit_event(
                            ctx,
                            StepEvent::Response {
                                iteration: self.state.exec.iteration,
                                text: self.state.exec.result.clone(),
                            },
                        );
                    }
                }
            }

            // Feed loop detector — pass actual tool parameters so generic_repeat
            // can detect repeated calls with identical params (not just call IDs).
            let loop_check = self.detector.check(name, arguments);
            if loop_check.stuck && loop_check.level == "critical" {
                return Err(DmError::loop_err(loop_check.message));
            }

            // Record outcome so no_progress and ping_pong detectors can work.
            self.detector
                .record_outcome(name, arguments, output, tool_sats_paid);
        }

        // BRC-18 MessageSend proofs — best-effort, never fails the iteration
        for info in &message_send_infos {
            let commitment = proofs::message_send_proof(
                &info.hash,
                &info.recipient,
                &info.box_name,
                info.sats,
                info.signed,
                info.encrypted,
                self.state.onchain.last_proof_hash.as_deref(),
            );
            self.record_proof(commitment, "message_send", None).await;
        }

        // Reset nudge counter when obligations fulfilled
        if called_send_message && self.state.comms.pending_replies.is_empty() {
            self.state.comms.nudge_count = 0;
        }

        // Update artifact manifest — scan workspace for new files created by tools
        update_artifact_manifest(&self.workspace, &tool_result_summaries);

        Ok((
            called_send_message,
            memory_ids_this_iter,
            message_hashes_this_iter,
        ))
    }

    /// Execute tool calls sequentially (single tool call path — no JoinSet overhead).
    /// Returns Vec of (index, call_id, name, arguments_str, arguments, output, success).
    async fn execute_tools_sequential(
        &mut self,
        parsed_calls: &[(String, String, String, Value)],
        ctx: &StepContext,
    ) -> Vec<ToolExecResult> {
        let mut results = Vec::with_capacity(parsed_calls.len());

        for (idx, (call_id, name, arguments_str, arguments)) in parsed_calls.iter().enumerate() {
            if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::info!("Cancel detected between tool calls, aborting remaining tools");
                break;
            }

            tracing::info!("Tool call: {}({}) [{}]", name, arguments_str, call_id);

            self.transcript.record_tool_call(call_id, name, arguments);

            emit_event(
                ctx,
                StepEvent::ToolCallStarted {
                    iteration: self.state.exec.iteration,
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments_str.to_string(),
                },
            );

            // Check if tool requires manual approval before execution
            if self.requires_approval(name) {
                let (output, success) = self
                    .wait_for_approval(name, call_id, arguments_str, ctx)
                    .await;
                if !success {
                    results.push((
                        idx,
                        call_id.clone(),
                        name.clone(),
                        arguments_str.clone(),
                        arguments.clone(),
                        output,
                        false,
                    ));
                    continue;
                }
                // Approval granted — fall through to normal execution
            }

            // Fire PreToolExecution hook — blocking hooks can prevent execution
            let pre_results = self
                .hook_registry
                .fire(crate::hooks::HookEvent::PreToolExecution {
                    tool_name: name.clone(),
                    parameters: arguments.clone(),
                })
                .await;
            if let Some(reason) = crate::hooks::is_blocked(&pre_results) {
                tracing::info!("Tool '{}' blocked by hook: {}", name, reason);
                results.push((
                    idx,
                    call_id.clone(),
                    name.clone(),
                    arguments_str.clone(),
                    arguments.clone(),
                    format!("Error: blocked by hook — {reason}"),
                    false,
                ));
                continue;
            }

            // Moderate tool input arguments
            let desc = format!("tool input for '{}' (call_id={})", name, call_id);
            let input_outcome =
                self.moderate_content(arguments_str, "tool_input", Some(name), &desc);
            let start_time = std::time::Instant::now();
            let (output, success) =
                if input_outcome == super::moderation::ModerationOutcome::Blocked {
                    (
                        "Error: tool input blocked by moderation policy".to_string(),
                        false,
                    )
                } else {
                    // Flagged or Pass: execute the tool
                    match self.execute_tool(name, arguments.clone()).await {
                        Ok(result) => {
                            tracing::info!("Tool {} returned {} chars", name, result.len());
                            (result, true)
                        }
                        Err(e) => {
                            tracing::warn!("Tool {} failed: {e}", name);
                            (format!("Error: {e}"), false)
                        }
                    }
                };
            let duration_ms = start_time.elapsed().as_millis() as u64;

            // Fire PostToolExecution or ToolError hook
            if success {
                self.hook_registry
                    .fire(crate::hooks::HookEvent::PostToolExecution {
                        tool_name: name.clone(),
                        result: output.chars().take(1000).collect(),
                        duration_ms,
                    })
                    .await;
            } else {
                self.hook_registry
                    .fire(crate::hooks::HookEvent::ToolError {
                        tool_name: name.clone(),
                        error: output.clone(),
                    })
                    .await;
            }

            results.push((
                idx,
                call_id.clone(),
                name.clone(),
                arguments_str.clone(),
                arguments.clone(),
                output,
                success,
            ));
        }

        results
    }

    /// Execute multiple tool calls concurrently via `tokio::task::JoinSet`.
    ///
    /// Each tool is spawned into its own task with the original index for
    /// re-ordering. Panicked tasks produce error results instead of crashing
    /// the iteration. Results are returned in original request order.
    async fn execute_tools_parallel(
        &mut self,
        parsed_calls: &[(String, String, String, Value)],
        ctx: &StepContext,
    ) -> Vec<ToolExecResult> {
        tracing::info!(
            "Executing {} tools in parallel via JoinSet",
            parsed_calls.len()
        );

        // Emit ToolCallStarted events and record transcript entries for all tools up front
        for (call_id, name, arguments_str, arguments) in parsed_calls {
            self.transcript.record_tool_call(call_id, name, arguments);
            emit_event(
                ctx,
                StepEvent::ToolCallStarted {
                    iteration: self.state.exec.iteration,
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments_str.to_string(),
                },
            );
        }

        // If any tool requires approval, fall back to sequential execution
        // (approval polling is inherently sequential and can't be spawned into JoinSet)
        if parsed_calls
            .iter()
            .any(|(_, name, _, _)| self.requires_approval(name))
        {
            tracing::info!(
                "Approval-required tool(s) detected in parallel batch — using sequential path"
            );
            return self.execute_tools_sequential(parsed_calls, ctx).await;
        }

        // Moderate tool inputs and spawn executions into JoinSet
        let mut join_set = tokio::task::JoinSet::new();
        let tools = self.tools.clone();
        let capabilities = self.state.auth.capabilities.clone();

        // Pre-moderate inputs: blocked tools get immediate error results
        let mut blocked_results: Vec<ToolExecResult> = Vec::new();

        for (idx, (call_id, name, arguments_str, arguments)) in parsed_calls.iter().enumerate() {
            let desc = format!("tool input for '{}' (call_id={}, parallel)", name, call_id);
            let input_outcome =
                self.moderate_content(arguments_str, "tool_input", Some(name), &desc);
            if input_outcome == super::moderation::ModerationOutcome::Blocked {
                blocked_results.push((
                    idx,
                    call_id.clone(),
                    name.clone(),
                    arguments_str.clone(),
                    arguments.clone(),
                    "Error: tool input blocked by moderation policy".to_string(),
                    false,
                ));
                continue;
            }
            // Flagged or Pass: continue to spawn

            let tools = tools.clone();
            let capabilities = capabilities.clone();
            let name = name.clone();
            let arguments = arguments.clone();
            let call_id = call_id.clone();
            let arguments_str = arguments_str.clone();

            join_set.spawn(async move {
                let result =
                    execute_tool_standalone(&tools, &capabilities, &name, arguments.clone()).await;
                let (output, success) = match result {
                    Ok(out) => {
                        tracing::info!("Tool {} returned {} chars (parallel)", name, out.len());
                        (out, true)
                    }
                    Err(e) => {
                        tracing::warn!("Tool {} failed (parallel): {e}", name);
                        (format!("Error: {e}"), false)
                    }
                };
                (
                    idx,
                    call_id,
                    name,
                    arguments_str,
                    arguments,
                    output,
                    success,
                )
            });
        }

        // Collect results, reorder by original index
        let count = parsed_calls.len();
        let mut ordered: Vec<Option<ToolExecResult>> = (0..count).map(|_| None).collect();

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok(result) => {
                    let idx = result.0;
                    ordered[idx] = Some(result);
                }
                Err(e) => {
                    // JoinError: task panicked or was cancelled
                    tracing::error!("Tool execution task failed: {e}");
                    // Find the first empty slot and fill with error
                    let empty_idx = ordered.iter().position(|s| s.is_none());
                    if let Some(idx) = empty_idx {
                        ordered[idx] = Some((
                            idx,
                            "unknown".to_string(),
                            "unknown".to_string(),
                            "{}".to_string(),
                            serde_json::json!({}),
                            format!("Error: tool execution panicked: {e}"),
                            false,
                        ));
                    }
                }
            }
        }

        // Insert blocked results (from pre-moderation) into ordered slots
        for blocked in blocked_results {
            let idx = blocked.0;
            ordered[idx] = Some(blocked);
        }

        // Flatten — fill any remaining None slots with error results
        ordered
            .into_iter()
            .enumerate()
            .map(|(i, slot)| {
                slot.unwrap_or_else(|| {
                    (
                        i,
                        "unknown".to_string(),
                        "unknown".to_string(),
                        "{}".to_string(),
                        serde_json::json!({}),
                        "Error: tool execution did not complete".to_string(),
                        false,
                    )
                })
            })
            .collect()
    }
}

// ===========================================================================
// Tests for the persisted-output preview format.
//
// The preview function is `pub(crate)` so these tests live inline rather
// than in `tests/`. They exercise each branch of the structural summary
// (JSON object, JSON array, nested tree, plain text, malformed) and the
// canonical HN Algolia response shape that exposed the old first-N-bytes
// preview's failure mode.
// ===========================================================================

#[cfg(test)]
mod preview_tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Envelope — size header, retrieval syntax, tag wrapping
    // ---------------------------------------------------------------------

    #[test]
    fn envelope_contains_open_and_close_tags() {
        let p = generate_persisted_output_preview("hello world", "/tmp/x.txt", "call_a");
        assert!(p.starts_with("<persisted-output>"), "missing open tag: {p}");
        assert!(p.ends_with("</persisted-output>"), "missing close tag: {p}");
    }

    #[test]
    fn envelope_reports_size_in_bytes() {
        let content = "x".repeat(1234);
        let p = generate_persisted_output_preview(&content, "/tmp/x.txt", "call_b");
        assert!(p.contains("Size: 1234 bytes"), "missing byte count: {p}");
    }

    #[test]
    fn envelope_exposes_call_id_retrieval_syntax() {
        let p = generate_persisted_output_preview("data", "/tmp/x.json", "call_xyz");
        assert!(
            p.contains("read_tool_output call_id=\"call_xyz\""),
            "missing call_id retrieval: {p}"
        );
    }

    #[test]
    fn envelope_exposes_file_read_retrieval_syntax() {
        let p = generate_persisted_output_preview("data", "/tmp/tool_output_web.json", "call_xyz");
        assert!(
            p.contains("file_read path=\"/tmp/tool_output_web.json\""),
            "missing file_read retrieval: {p}"
        );
    }

    #[test]
    fn envelope_always_includes_raw_prefix_fallback() {
        let p = generate_persisted_output_preview("hello world", "/tmp/x.txt", "call_c");
        assert!(p.contains("Raw prefix"), "missing raw prefix header: {p}");
        assert!(p.contains("hello world"), "missing actual content: {p}");
    }

    #[test]
    fn envelope_raw_prefix_truncates_at_cap() {
        let content = "y".repeat(PREVIEW_RAW_PREFIX_LEN + 500);
        let p = generate_persisted_output_preview(&content, "/tmp/x.txt", "call_d");
        assert!(p.contains("...[truncated]"), "missing truncation marker");
        // The full content should NOT appear verbatim in the raw prefix
        // (it was capped) — total preview should be well under content len.
        assert!(p.len() < content.len());
    }

    // ---------------------------------------------------------------------
    // JSON object — top-level schema
    // ---------------------------------------------------------------------

    #[test]
    fn json_object_structure_lists_fields() {
        let content = r#"{"name":"alice","age":30,"city":"NYC"}"#;
        let p = generate_persisted_output_preview(content, "/tmp/x.json", "call_e");
        assert!(p.contains("object, 3 fields"), "missing object header: {p}");
        assert!(p.contains("name"), "missing field name: {p}");
        assert!(p.contains("age"), "missing field age: {p}");
        assert!(p.contains("city"), "missing field city: {p}");
    }

    #[test]
    fn json_object_marks_nested_shapes() {
        let content = r#"{"meta":{"ver":1},"items":[1,2,3]}"#;
        let p = generate_persisted_output_preview(content, "/tmp/x.json", "call_f");
        assert!(
            p.contains("meta(obj)"),
            "missing nested-object shape hint: {p}"
        );
        assert!(
            p.contains("items(array)"),
            "missing nested-array shape hint: {p}"
        );
    }

    #[test]
    fn json_object_with_many_keys_indicates_more() {
        // 15 keys, cap is 10 → should see "+5 more"
        let mut map = serde_json::Map::new();
        for i in 0..15 {
            map.insert(format!("k{}", i), Value::from(i));
        }
        let content = serde_json::to_string(&Value::Object(map)).unwrap();
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_g");
        assert!(p.contains("+5 more"), "missing more-keys indicator: {p}");
    }

    // ---------------------------------------------------------------------
    // JSON array — cardinality + schema + exemplars
    // ---------------------------------------------------------------------

    #[test]
    fn json_array_shows_count() {
        let arr: Vec<Value> = (0..7)
            .map(|i| serde_json::json!({"id": i, "name": format!("item_{}", i)}))
            .collect();
        let content = serde_json::to_string(&Value::Array(arr)).unwrap();
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_h");
        assert!(p.contains("array[7]"), "missing count: {p}");
    }

    #[test]
    fn json_array_shows_field_schema_from_first_item() {
        let arr: Vec<Value> = (0..3)
            .map(|i| serde_json::json!({"id": i, "title": format!("t{}", i), "score": i * 10}))
            .collect();
        let content = serde_json::to_string(&Value::Array(arr)).unwrap();
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_i");
        // serde_json::Map preserves insertion order only with the preserve_order
        // feature, and sorts alphabetically otherwise — which this crate's build
        // does not enable. Assert the set of fields, not the order.
        assert!(p.contains("fields:"), "missing schema header: {p}");
        assert!(p.contains("id"), "missing id field: {p}");
        assert!(p.contains("title"), "missing title field: {p}");
        assert!(p.contains("score"), "missing score field: {p}");
    }

    #[test]
    fn json_array_shows_first_three_items_as_exemplars() {
        let arr: Vec<Value> = (0..5)
            .map(|i| serde_json::json!({"title": format!("Title {}", i)}))
            .collect();
        let content = serde_json::to_string(&Value::Array(arr)).unwrap();
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_j");
        assert!(p.contains("[0] {"), "missing [0] exemplar: {p}");
        assert!(p.contains("[1] {"), "missing [1] exemplar: {p}");
        assert!(p.contains("[2] {"), "missing [2] exemplar: {p}");
        // [3] and [4] should NOT appear as exemplars (cap = 3)
        assert!(!p.contains("[3] {"), "should cap at 3 exemplars: {p}");
        assert!(p.contains("(2 more items)"), "missing more-items indicator");
    }

    #[test]
    fn json_array_empty_degrades_gracefully() {
        let p = generate_persisted_output_preview("[]", "/tmp/x.json", "call_k");
        assert!(
            p.contains("array[0]"),
            "empty array should still parse: {p}"
        );
    }

    // ---------------------------------------------------------------------
    // Nested — the HN Algolia shape that broke the old format
    // ---------------------------------------------------------------------

    #[test]
    fn god_tier_preview_hn_response() {
        // This is the exact pathology that broke the old preview format:
        // a JSON OBJECT with the interesting data nested under "hits". Old
        // format showed top-level keys (exhaustive, exhaustiveNbHits, hits,
        // ...) + first 2000 bytes of content (which is the metadata envelope,
        // NOT the titles). The god-tier format recurses into "hits" and
        // shows actual titles.
        let hn = serde_json::json!({
            "exhaustive": { "nbHits": true, "typo": true },
            "exhaustiveNbHits": true,
            "exhaustiveTypo": true,
            "hits": [
                { "title": "Gitlab runner TLS debugging story",
                  "author": "littlecranky67",
                  "points": 87,
                  "url": "https://example.com/a",
                  "created_at": "2026-04-13T10:00:00Z" },
                { "title": "Rust async closures are here",
                  "author": "steveklabnik",
                  "points": 142,
                  "url": "https://example.com/b",
                  "created_at": "2026-04-13T09:30:00Z" },
                { "title": "Why I left Google",
                  "author": "alice",
                  "points": 203,
                  "url": "https://example.com/c",
                  "created_at": "2026-04-13T09:00:00Z" },
                { "title": "The state of Postgres in 2026",
                  "author": "bob",
                  "points": 98,
                  "url": "https://example.com/d",
                  "created_at": "2026-04-13T08:30:00Z" },
                { "title": "Apple releases new Macbook",
                  "author": "carol",
                  "points": 56,
                  "url": "https://example.com/e",
                  "created_at": "2026-04-13T08:00:00Z" },
            ],
            "nbHits": 5,
            "nbPages": 1,
        });
        let content = serde_json::to_string(&hn).unwrap();
        let p = generate_persisted_output_preview(&content, "/tmp/hn.json", "call_hn");

        // The preview MUST surface titles — that's the whole point.
        assert!(
            p.contains("Gitlab runner TLS debugging story"),
            "preview should contain the first title:\n{p}"
        );
        assert!(
            p.contains("Rust async closures are here"),
            "preview should contain the second title:\n{p}"
        );
        assert!(
            p.contains("Why I left Google"),
            "preview should contain the third title:\n{p}"
        );
        // It should also report cardinality and the nested shape correctly.
        assert!(
            p.contains("hits: array(5)"),
            "should describe nested hits array:\n{p}"
        );
        assert!(p.contains("object, 6 fields"));
        assert!(p.contains("hits(array)"));
        // The full preview should be reasonably bounded.
        assert!(
            p.len() < 3000,
            "preview should stay compact (got {} chars):\n{p}",
            p.len()
        );
    }

    #[test]
    fn nested_object_descent_respects_max_depth() {
        // A 5-level deep object — we should stop at PREVIEW_MAX_DEPTH = 3.
        let deep = serde_json::json!({
            "l0": { "l1": { "l2": { "l3": { "l4": "deep" } } } }
        });
        let content = serde_json::to_string(&deep).unwrap();
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_deep");
        // Should show the nested structure up to depth 3 but not dereference
        // deeper scalars inline. The recursive summary should remain bounded.
        assert!(p.contains("l0"));
        // Preview should stay small regardless of nesting depth.
        assert!(p.len() < 2000, "got {} chars", p.len());
    }

    // ---------------------------------------------------------------------
    // Value truncation
    // ---------------------------------------------------------------------

    #[test]
    fn leaf_values_are_truncated() {
        let long = "a".repeat(500);
        let content = serde_json::json!({ "long_field": long }).to_string();
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_trunc");
        // Ensure the full 500-char value is NOT rendered verbatim in the
        // structural summary. The raw prefix may still include some of it,
        // but the structural schema line should be bounded.
        let structural_section = p.split("Raw prefix").next().unwrap_or("");
        // Long field won't fit inline in structural summary past PREVIEW_MAX_VALUE_LEN.
        // Expect a "..." truncation marker or the field to be omitted.
        // Either way, the structural section itself should stay small.
        assert!(
            structural_section.len() < 1000,
            "structural section should be bounded: {} chars",
            structural_section.len()
        );
    }

    // ---------------------------------------------------------------------
    // Plain text / malformed JSON
    // ---------------------------------------------------------------------

    #[test]
    fn plain_text_uses_word_count_header() {
        let content = "this is plain text with multiple words in it yes sir";
        let p = generate_persisted_output_preview(content, "/tmp/x.txt", "call_text");
        assert!(
            p.contains("words (text or unparseable)"),
            "missing word count header: {p}"
        );
        assert!(
            p.contains(content),
            "should include full content in raw prefix"
        );
    }

    #[test]
    fn malformed_json_falls_through_to_plain_text_path() {
        // Starts with { but isn't valid JSON — parser returns Err, we fall
        // through to word count + raw prefix.
        let content = "{not valid json: missing quotes, broken: [";
        let p = generate_persisted_output_preview(content, "/tmp/x.json", "call_bad");
        assert!(
            p.contains("words (text or unparseable)") || p.contains("Content:"),
            "malformed JSON should fall through to text path: {p}"
        );
        assert!(
            p.contains("not valid json"),
            "raw prefix should have content"
        );
    }

    #[test]
    fn very_large_content_skips_json_parsing() {
        // Over PREVIEW_PARSE_SIZE_CAP — parser is skipped to keep preview
        // generation fast. Falls through to plain-text handling.
        let huge = format!("[{}]", "1,".repeat(600_000));
        assert!(huge.len() > PREVIEW_PARSE_SIZE_CAP);
        let p = generate_persisted_output_preview(&huge, "/tmp/x.json", "call_huge");
        // Should NOT have parsed successfully — should be in the text fallback path.
        assert!(
            !p.contains("array["),
            "should skip JSON parse for huge content: {}",
            &p[..400]
        );
        // Raw prefix still present.
        assert!(p.contains("Raw prefix"));
        // Total preview still bounded.
        assert!(p.len() < 5000, "preview blew up: {} chars", p.len());
    }

    // ---------------------------------------------------------------------
    // Item field selection — prefers human-interesting fields
    // ---------------------------------------------------------------------

    #[test]
    fn item_compact_prefers_title_name_id() {
        let item = serde_json::json!({
            "_internal": "xyz",
            "debug_tag": "abc",
            "title": "The title",
            "author": "someone",
            "id": 42,
        });
        let rendered = format_item_compact(&item);
        assert!(
            rendered.contains("title=\"The title\""),
            "missing title: {rendered}"
        );
        assert!(rendered.contains("id=42"), "missing id: {rendered}");
    }

    #[test]
    fn item_compact_falls_back_to_first_scalar_fields() {
        // Item has none of the priority fields — should pick up regular
        // scalar fields in insertion order.
        let item = serde_json::json!({
            "alpha": 1,
            "beta": "two",
            "gamma": true,
            "delta": null,
        });
        let rendered = format_item_compact(&item);
        assert!(rendered.contains("alpha=1"), "missing alpha: {rendered}");
        assert!(rendered.contains("beta="), "missing beta: {rendered}");
    }

    // ---------------------------------------------------------------------
    // Overall preview size is bounded
    // ---------------------------------------------------------------------

    #[test]
    fn preview_size_is_bounded_for_pathological_input() {
        // 300 items × 15 fields × 50-char values keeps the total under the
        // 1MB PREVIEW_PARSE_SIZE_CAP so the parser actually runs. (Larger
        // inputs fall through to the plain-text path, which is tested
        // separately in `very_large_content_skips_json_parsing`.)
        let mut arr = Vec::new();
        for i in 0..300 {
            let mut obj = serde_json::Map::new();
            for j in 0..15 {
                obj.insert(format!("f_{}_{}", i, j), Value::String("v".repeat(50)));
            }
            arr.push(Value::Object(obj));
        }
        let content = serde_json::to_string(&Value::Array(arr)).unwrap();
        assert!(
            content.len() < PREVIEW_PARSE_SIZE_CAP,
            "test setup: content must be under parse cap (got {} bytes)",
            content.len()
        );
        let p = generate_persisted_output_preview(&content, "/tmp/x.json", "call_big");
        assert!(
            p.len() < 5000,
            "pathological input blew up preview to {} chars",
            p.len()
        );
        // Cardinality and some structure should still be visible in the
        // structural summary section.
        assert!(p.contains("array[300]"), "missing cardinality: {p}");
    }

    // ---------------------------------------------------------------------
    // Backwards-compat alias still works for callers without a call_id
    // ---------------------------------------------------------------------

    #[test]
    fn backwards_compat_alias_still_produces_valid_preview() {
        let p = generate_smart_preview("hello", "/tmp/x.txt");
        assert!(p.starts_with("<persisted-output>"));
        assert!(p.contains("read_tool_output call_id=\"unknown\""));
    }
}
