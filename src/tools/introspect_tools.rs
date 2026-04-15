//! Introspect tool — lets the agent query its own proofs, costs, and task history.
//!
//! Single `introspect` tool with an `action` parameter. Discoverable via `search_tools`,
//! NOT always-on. Actions: recent_proofs, task_costs, task_detail, activity_summary,
//! verify_my_state.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::proofs;
use crate::tools::registry::ToolDef;
use crate::wallet::WalletClient;

/// Maximum number of task directories to scan for activity_summary.
const MAX_SCAN_DIRS: usize = 100;

/// Maximum number of task directories to scan for task_costs.
const MAX_COST_DIRS: usize = 50;

/// Default number of results for list queries.
const DEFAULT_COUNT: usize = 5;

/// Maximum number of results for list queries.
const MAX_COUNT: usize = 20;

/// Create the introspect tool.
pub fn create_introspect_tool(workspace: Arc<PathBuf>) -> ToolDef {
    ToolDef {
        name: "introspect".to_string(),
        description: "Query your own proofs, costs, and task history. Actions: recent_proofs, task_costs, task_detail, activity_summary, verify_my_state".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["recent_proofs", "task_costs", "task_detail", "activity_summary", "verify_my_state"],
                    "description": "What to query"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results (default 5, max 20). Used by recent_proofs and task_costs."
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID for task_detail action"
                }
            },
            "required": ["action"]
        }),
        execute: Box::new(move |params: Value| {
            let ws = workspace.clone();
            Box::pin(async move {
                let action = params
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let count = params
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .map(|c| (c as usize).min(MAX_COUNT))
                    .unwrap_or(DEFAULT_COUNT);

                match action {
                    "recent_proofs" => recent_proofs(&ws, count),
                    "task_costs" => task_costs(&ws, count),
                    "task_detail" => {
                        let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => return "Error: task_id is required for task_detail action".to_string(),
                        };
                        task_detail(&ws, task_id)
                    }
                    "activity_summary" => activity_summary(&ws),
                    "verify_my_state" => verify_my_state(count).await,
                    other => format!("Error: unknown action '{other}'. Use 'recent_proofs', 'task_costs', 'task_detail', 'activity_summary', or 'verify_my_state'"),
                }
            })
        }),
        category: "introspect".to_string(),
        cleanup: None,
    deferred: true,
    always_load: false,
    search_hint: Some("Self-query proofs, costs, task history, and on-chain state".to_string()),
    }
}

/// List task directories sorted by modification time (most recent first).
fn list_task_dirs(workspace: &Path, max: usize) -> Vec<(String, PathBuf)> {
    let tasks_dir = workspace.join("tasks");
    let entries = match std::fs::read_dir(&tasks_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut dirs: Vec<(String, PathBuf, std::time::SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((name, e.path(), mtime))
        })
        .collect();

    // Sort by modification time, most recent first
    dirs.sort_by(|a, b| b.2.cmp(&a.2));
    dirs.truncate(max);

    dirs.into_iter()
        .map(|(name, path, _)| (name, path))
        .collect()
}

/// Read and parse a transcript JSONL file, returning parsed events.
fn read_transcript(transcript_path: &std::path::Path) -> Vec<Value> {
    let content = match std::fs::read_to_string(transcript_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<Value>(line) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("Malformed transcript line: {e}");
                None
            }
        })
        .collect()
}

/// Find the transcript file for a task directory.
fn find_transcript(task_dir: &std::path::Path) -> Option<PathBuf> {
    // Try transcript.jsonl first (standard), then session.jsonl (legacy)
    let transcript = task_dir.join("transcript.jsonl");
    if transcript.exists() {
        return Some(transcript);
    }
    let session = task_dir.join("session.jsonl");
    if session.exists() {
        return Some(session);
    }
    None
}

/// Read transcript line-by-line, only parsing lines that contain one of the given event types.
///
/// This avoids loading the entire JSONL file into memory when only a few event types are needed.
/// A cheap `str::contains` check on each raw line filters out irrelevant events before JSON parsing.
fn read_transcript_events(transcript_path: &std::path::Path, event_types: &[&str]) -> Vec<Value> {
    let file = match std::fs::File::open(transcript_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = std::io::BufReader::new(file);
    let mut results = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        // Cheap string check: skip lines that don't contain any of the target event types.
        // This avoids JSON parsing for the majority of lines in a typical transcript.
        let dominated = event_types.iter().any(|et| line.contains(et));
        if !dominated {
            continue;
        }

        match serde_json::from_str::<Value>(&line) {
            Ok(v) => {
                // Verify the parsed event type actually matches (the string check may have
                // matched a substring in a different field, e.g. "proof_created" in content).
                if let Some(actual_type) = v.get("type").and_then(|t| t.as_str()) {
                    if event_types.contains(&actual_type) {
                        results.push(v);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Malformed transcript line: {e}");
            }
        }
    }

    results
}

// =========================================================================
// Action: recent_proofs
// =========================================================================

fn recent_proofs(workspace: &Path, count: usize) -> String {
    let task_dirs = list_task_dirs(workspace, MAX_SCAN_DIRS);
    let mut all_proofs: Vec<Value> = Vec::new();

    for (task_id, task_dir) in &task_dirs {
        let transcript_path = match find_transcript(task_dir) {
            Some(p) => p,
            None => continue,
        };

        let events = read_transcript_events(&transcript_path, &["proof_created"]);
        for event in &events {
            let mut proof = json!({
                "task_id": task_id,
            });

            // Copy relevant fields
            for field in &[
                "txid",
                "proof_type",
                "hash",
                "sats_cost",
                "iteration",
                "ts",
                "proof_data",
            ] {
                if let Some(val) = event.get(field) {
                    proof[field] = val.clone();
                }
            }

            all_proofs.push(proof);
        }
    }

    // Sort by timestamp descending
    all_proofs.sort_by(|a, b| {
        let ts_a = a.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let ts_b = b.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ts_b.partial_cmp(&ts_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    let total = all_proofs.len();
    all_proofs.truncate(count);

    // Build self-contained summary
    let summary = if all_proofs.is_empty() {
        "No proofs found.".to_string()
    } else {
        let latest: Vec<String> = all_proofs
            .iter()
            .take(3)
            .map(|p| {
                let ptype = p
                    .get("proof_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let hash = p.get("hash").and_then(|v| v.as_str()).unwrap_or("?");
                let short_hash = if hash.len() > 8 { &hash[..8] } else { hash };
                format!("{ptype} ({short_hash}…)")
            })
            .collect();
        format!(
            "{} of {} proofs. Latest: {}",
            all_proofs.len(),
            total,
            latest.join(", ")
        )
    };

    let result = json!({
        "summary": summary,
        "proofs": all_proofs,
        "total": total,
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Error: {e}"))
}

// =========================================================================
// Action: task_costs
// =========================================================================

fn task_costs(workspace: &Path, count: usize) -> String {
    let task_dirs = list_task_dirs(workspace, MAX_COST_DIRS);
    let mut tasks: Vec<Value> = Vec::new();

    for (task_id, task_dir) in &task_dirs {
        let transcript_path = match find_transcript(task_dir) {
            Some(p) => p,
            None => continue,
        };

        let events = read_transcript(&transcript_path);

        let mut total_sats: u64 = 0;
        let mut iterations: u64 = 0;
        let mut status = "Unknown".to_string();
        let mut model = String::new();
        let mut started_at: Option<f64> = None;
        let mut completed_at: Option<f64> = None;

        for event in &events {
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match event_type {
                "session_start" => {
                    if let Some(ts) = event.get("ts").and_then(|v| v.as_f64()) {
                        started_at = Some(ts);
                    }
                }
                "session_end" => {
                    if let Some(s) = event.get("status").and_then(|v| v.as_str()) {
                        status = s.to_string();
                    }
                    if let Some(i) = event.get("iterations").and_then(|v| v.as_u64()) {
                        iterations = i;
                    }
                    if let Some(ts) = event.get("ts").and_then(|v| v.as_f64()) {
                        completed_at = Some(ts);
                    }
                }
                "think_response" => {
                    if let Some(sats) = event.get("sats_effective").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                    }
                    if model.is_empty() {
                        if let Some(m) = event.get("model").and_then(|v| v.as_str()) {
                            model = m.to_string();
                        }
                    }
                }
                "tool_result" => {
                    if let Some(sats) = event.get("sats_paid").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                    }
                }
                "proof_created" => {
                    if let Some(sats) = event.get("sats_cost").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                    }
                }
                _ => {}
            }
        }

        let duration_ms = match (started_at, completed_at) {
            (Some(s), Some(e)) => Some(((e - s) * 1000.0) as u64),
            _ => None,
        };

        let mut task_info = json!({
            "task_id": task_id,
            "status": status,
            "iterations": iterations,
            "total_sats": total_sats,
            "model": model,
        });

        if let Some(s) = started_at {
            task_info["started_at"] = json!(s);
        }
        if let Some(d) = duration_ms {
            task_info["duration_ms"] = json!(d);
        }

        tasks.push(task_info);
    }

    // Sort by started_at descending (most recent first)
    tasks.sort_by(|a, b| {
        let ts_a = a.get("started_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let ts_b = b.get("started_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
        ts_b.partial_cmp(&ts_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    let total = tasks.len();
    tasks.truncate(count);

    // Build self-contained summary
    let total_sats_all: u64 = tasks
        .iter()
        .map(|t| t.get("total_sats").and_then(|v| v.as_u64()).unwrap_or(0))
        .sum();
    let avg_sats = if tasks.is_empty() {
        0
    } else {
        total_sats_all / tasks.len() as u64
    };
    let summary = if tasks.is_empty() {
        "No tasks found.".to_string()
    } else {
        let most_recent = &tasks[0];
        let mr_id = most_recent
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let mr_status = most_recent
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let mr_sats = most_recent
            .get("total_sats")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let mr_iters = most_recent
            .get("iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        format!(
            "{} of {} tasks shown. Total: {} sats (avg {}/task). Most recent: {} ({}, {} sats, {} iters)",
            tasks.len(), total, total_sats_all, avg_sats, mr_id, mr_status, mr_sats, mr_iters
        )
    };

    let result = json!({
        "summary": summary,
        "tasks": tasks,
        "total": total,
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Error: {e}"))
}

// =========================================================================
// Action: task_detail
// =========================================================================

fn task_detail(workspace: &Path, task_id: &str) -> String {
    let task_dir = workspace.join("tasks").join(task_id);
    if !task_dir.exists() {
        return format!("Error: Task '{task_id}' not found");
    }

    let transcript_path = match find_transcript(&task_dir) {
        Some(p) => p,
        None => return format!("Error: No transcript found for task '{task_id}'"),
    };

    let events = read_transcript(&transcript_path);

    let mut task_desc = String::new();
    let mut status = "Unknown".to_string();
    let mut iterations: u64 = 0;
    let mut total_sats: u64 = 0;
    let mut proof_txids: Vec<Value> = Vec::new();
    let mut tools_used: Vec<String> = Vec::new();
    let mut model = String::new();
    let mut prompt_tokens: u64 = 0;
    let mut completion_tokens: u64 = 0;
    let mut started_at: Option<f64> = None;
    let mut completed_at: Option<f64> = None;

    // Per-iteration cost breakdown
    let mut iter_costs: Vec<Value> = Vec::new();
    let mut current_iter: u64 = 0;
    let mut current_iter_sats: u64 = 0;
    let mut current_iter_tools: Vec<String> = Vec::new();

    for event in &events {
        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "session_start" => {
                if task_desc.is_empty() {
                    if let Some(t) = event.get("task").and_then(|v| v.as_str()) {
                        task_desc = t.to_string();
                    }
                }
                if let Some(ts) = event.get("ts").and_then(|v| v.as_f64()) {
                    started_at = Some(ts);
                }
            }
            "user" => {
                if task_desc.is_empty() {
                    if let Some(c) = event.get("content").and_then(|v| v.as_str()) {
                        task_desc = c.chars().take(200).collect();
                    }
                }
            }
            "session_end" => {
                if let Some(s) = event.get("status").and_then(|v| v.as_str()) {
                    status = s.to_string();
                }
                if let Some(i) = event.get("iterations").and_then(|v| v.as_u64()) {
                    iterations = i;
                }
                if let Some(ts) = event.get("ts").and_then(|v| v.as_f64()) {
                    completed_at = Some(ts);
                }
            }
            "think_response" => {
                // Each think_response marks the start of a new iteration.
                // Flush the previous iteration before recording the new one.
                if current_iter > 0 {
                    iter_costs.push(json!({
                        "iteration": current_iter,
                        "sats": current_iter_sats,
                        "tools": current_iter_tools,
                    }));
                    current_iter_sats = 0;
                    current_iter_tools = Vec::new();
                }
                current_iter += 1;

                if let Some(sats) = event.get("sats_effective").and_then(|v| v.as_u64()) {
                    total_sats += sats;
                    current_iter_sats += sats;
                }
                if model.is_empty() {
                    if let Some(m) = event.get("model").and_then(|v| v.as_str()) {
                        model = m.to_string();
                    }
                }
                if let Some(pt) = event.get("prompt_tokens").and_then(|v| v.as_u64()) {
                    prompt_tokens += pt;
                }
                if let Some(ct) = event.get("completion_tokens").and_then(|v| v.as_u64()) {
                    completion_tokens += ct;
                }
            }
            "tool_result" => {
                if let Some(name) = event.get("name").and_then(|v| v.as_str()) {
                    if !tools_used.contains(&name.to_string()) {
                        tools_used.push(name.to_string());
                    }
                    current_iter_tools.push(name.to_string());
                }
                if let Some(sats) = event.get("sats_paid").and_then(|v| v.as_u64()) {
                    total_sats += sats;
                    current_iter_sats += sats;
                }
            }
            "proof_created" => {
                if let Some(sats) = event.get("sats_cost").and_then(|v| v.as_u64()) {
                    total_sats += sats;
                }
                let mut proof_entry = json!({});
                if let Some(txid) = event.get("txid") {
                    proof_entry["txid"] = txid.clone();
                }
                if let Some(pt) = event.get("proof_type") {
                    proof_entry["proof_type"] = pt.clone();
                }
                proof_txids.push(proof_entry);
            }
            _ => {}
        }
    }

    // Flush last iteration
    if current_iter > 0 && (current_iter_sats > 0 || !current_iter_tools.is_empty()) {
        iter_costs.push(json!({
            "iteration": current_iter,
            "sats": current_iter_sats,
            "tools": current_iter_tools,
        }));
    }

    let duration_ms = match (started_at, completed_at) {
        (Some(s), Some(e)) => Some(((e - s) * 1000.0) as u64),
        _ => None,
    };

    // Build self-contained summary
    let task_preview: String = task_desc.chars().take(80).collect();
    let tools_preview: Vec<&str> = tools_used.iter().map(|s| s.as_str()).take(5).collect();
    let duration_str = match duration_ms {
        Some(d) => format!(", {}s", d / 1000),
        None => String::new(),
    };
    let summary = format!(
        "{}: \"{}\" — {}, {} iterations, {} sats, model {}. Tools: [{}]. {} proofs{}",
        task_id,
        task_preview,
        status,
        iterations,
        total_sats,
        model,
        tools_preview.join(", "),
        proof_txids.len(),
        duration_str
    );

    let mut result = json!({
        "summary": summary,
        "task_id": task_id,
        "task": task_desc,
        "status": status,
        "iterations": iterations,
        "total_sats": total_sats,
        "proofs": proof_txids,
        "tools_used": tools_used,
        "model": model,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "per_iteration": iter_costs,
    });

    if let Some(d) = duration_ms {
        result["duration_ms"] = json!(d);
    }

    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Error: {e}"))
}

// =========================================================================
// Action: activity_summary
// =========================================================================

fn activity_summary(workspace: &Path) -> String {
    let task_dirs = list_task_dirs(workspace, MAX_SCAN_DIRS);
    let total_tasks = task_dirs.len();

    let mut total_proofs: u64 = 0;
    let mut total_sats: u64 = 0;
    let mut services: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut recent_task_ids: Vec<String> = Vec::new();

    for (task_id, task_dir) in &task_dirs {
        if recent_task_ids.len() < 10 {
            recent_task_ids.push(task_id.clone());
        }

        let transcript_path = match find_transcript(task_dir) {
            Some(p) => p,
            None => continue,
        };

        let events = read_transcript_events(
            &transcript_path,
            &[
                "proof_created",
                "think_response",
                "tool_result",
                "checkpoint_created",
            ],
        );
        for event in &events {
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match event_type {
                "proof_created" => {
                    total_proofs += 1;
                    if let Some(sats) = event.get("sats_cost").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                        *services.entry("proofs".to_string()).or_insert(0) += sats;
                    }
                }
                "think_response" => {
                    if let Some(sats) = event.get("sats_effective").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                        *services.entry("llm".to_string()).or_insert(0) += sats;
                    }
                }
                "tool_result" => {
                    if let Some(sats) = event.get("sats_paid").and_then(|v| v.as_u64()) {
                        if sats > 0 {
                            total_sats += sats;
                            // Categorize by tool name prefix
                            let tool_name = event
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("other");
                            let service_key = if tool_name.starts_with("x402")
                                || tool_name == "generate_image"
                                || tool_name == "upload_to_nanostore"
                            {
                                "x402_services"
                            } else {
                                "tools"
                            };
                            *services.entry(service_key.to_string()).or_insert(0) += sats;
                        }
                    }
                }
                "checkpoint_created" => {
                    if let Some(sats) = event.get("sats_cost").and_then(|v| v.as_u64()) {
                        total_sats += sats;
                        *services.entry("tokens".to_string()).or_insert(0) += sats;
                    }
                }
                _ => {}
            }
        }
    }

    // Also read budget.jsonl for a complementary view
    let budget_path = workspace.join("budget.jsonl");
    let mut budget_total_sats: u64 = 0;
    let mut budget_services: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();

    if budget_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&budget_path) {
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<Value>(line) {
                    if let Some(sats) = entry.get("sats").and_then(|v| v.as_u64()) {
                        budget_total_sats += sats;
                        if let Some(service) = entry.get("service").and_then(|v| v.as_str()) {
                            *budget_services.entry(service.to_string()).or_insert(0) += sats;
                        }
                    }
                }
            }
        }
    }

    // Convert service maps to JSON objects
    let services_json: Value = services
        .into_iter()
        .map(|(k, v)| (k, json!(v)))
        .collect::<serde_json::Map<String, Value>>()
        .into();

    let budget_services_json: Value = budget_services
        .into_iter()
        .map(|(k, v)| (k, json!(v)))
        .collect::<serde_json::Map<String, Value>>()
        .into();

    // Build self-contained summary
    let svc_parts: Vec<String> = services_json
        .as_object()
        .map(|m| m.iter().map(|(k, v)| format!("{} {}", k, v)).collect())
        .unwrap_or_default();
    let summary = format!(
        "{} tasks, {} proofs, {} sats total. By service: {}",
        total_tasks,
        total_proofs,
        total_sats,
        if svc_parts.is_empty() {
            "none".to_string()
        } else {
            svc_parts.join(", ")
        }
    );

    let result = json!({
        "summary": summary,
        "total_tasks": total_tasks,
        "total_proofs": total_proofs,
        "total_sats_spent": total_sats,
        "services": services_json,
        "budget_journal": {
            "total_sats": budget_total_sats,
            "services": budget_services_json,
        },
        "recent_task_ids": recent_task_ids,
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Error: {e}"))
}

// =========================================================================
// Action: verify_my_state
// =========================================================================

/// Read the agent's proof chain from the blockchain and return a verification report.
///
/// Since the introspect tool is a stateless closure without access to the runner's
/// in-memory state, this action reads proofs and returns them for the agent to
/// compare against its own knowledge. The agent (or the periodic auto-verify in
/// step.rs) can then reason about consistency.
async fn verify_my_state(limit: usize) -> String {
    let wallet = WalletClient::new("http://localhost:3322", "http://localhost", 30);
    let chain_limit = limit.max(DEFAULT_COUNT) * 10; // Read more proofs for a fuller picture

    match proofs::read_proof_chain(&wallet, chain_limit).await {
        Ok(chain) => {
            let status = if chain.is_empty() {
                "no_proofs_found"
            } else {
                "proofs_retrieved"
            };
            let last_hash = chain.first().map(|p| p.hash.as_str());

            // Build self-contained summary
            let summary = if chain.is_empty() {
                "No proofs found on-chain.".to_string()
            } else {
                format!(
                    "chain_length: {}, status: {}, last_proof_hash: {}",
                    chain.len(),
                    status,
                    last_hash.unwrap_or("none")
                )
            };

            let result = json!({
                "summary": summary,
                "chain_length": chain.len(),
                "recent_proofs": chain.iter().take(limit).map(|p| {
                    json!({
                        "txid": p.txid,
                        "hash": p.hash,
                    })
                }).collect::<Vec<Value>>(),
                "status": status,
                "last_proof_hash": last_hash,
            });
            serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Error: {e}"))
        }
        Err(e) => {
            let summary = format!("Error reading proof chain: {e}");
            let result = json!({
                "summary": summary,
                "status": "error",
                "message": format!("Failed to read proof chain from wallet: {e}"),
            });
            serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("Error: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_workspace() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();
        fs::create_dir_all(workspace.join("tasks")).unwrap();
        (tmp, workspace)
    }

    fn write_transcript(workspace: &Path, task_id: &str, events: &[Value]) {
        let task_dir = workspace.join("tasks").join(task_id);
        fs::create_dir_all(&task_dir).unwrap();
        let lines: Vec<String> = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        fs::write(task_dir.join("transcript.jsonl"), lines.join("\n")).unwrap();
    }

    #[test]
    fn test_recent_proofs_empty_workspace() {
        let (_tmp, workspace) = setup_workspace();
        let result = recent_proofs(&workspace, 5);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 0);
        assert_eq!(parsed["proofs"].as_array().unwrap().len(), 0);
        assert!(parsed["summary"]
            .as_str()
            .unwrap()
            .contains("No proofs found"));
    }

    #[test]
    fn test_recent_proofs_with_data() {
        let (_tmp, workspace) = setup_workspace();
        let events = vec![
            json!({"type": "session_start", "ts": 1000.0, "task": "test"}),
            json!({"type": "proof_created", "ts": 1001.0, "txid": "abc123", "proof_type": "Decision", "hash": "deadbeef", "sats_cost": 200, "iteration": 1, "proof_data": "DECISION: test\nCERT_HASH: sha256:abc\nCERT_TYPE: parent-signed"}),
            json!({"type": "proof_created", "ts": 1002.0, "txid": "def456", "proof_type": "TaskCompletion", "hash": "cafebabe", "sats_cost": 200, "iteration": 2}),
        ];
        write_transcript(&workspace, "task-1", &events);

        let result = recent_proofs(&workspace, 5);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 2);
        let proofs = parsed["proofs"].as_array().unwrap();
        assert_eq!(proofs.len(), 2);
        // Should be sorted by timestamp descending
        assert_eq!(proofs[0]["txid"], "def456");
        assert_eq!(proofs[1]["txid"], "abc123");
        assert_eq!(proofs[0]["task_id"], "task-1");
        // proof_data should be included when present
        assert_eq!(
            proofs[1]["proof_data"],
            "DECISION: test\nCERT_HASH: sha256:abc\nCERT_TYPE: parent-signed"
        );
        // proof_data should be null/absent when not present in event
        assert!(proofs[0].get("proof_data").is_none() || proofs[0]["proof_data"].is_null());
        // Summary should mention proof types
        let summary = parsed["summary"].as_str().unwrap();
        assert!(summary.contains("2 of 2 proofs"), "summary: {summary}");
        assert!(summary.contains("TaskCompletion"), "summary: {summary}");
    }

    #[test]
    fn test_recent_proofs_count_limit() {
        let (_tmp, workspace) = setup_workspace();
        let mut events: Vec<Value> =
            vec![json!({"type": "session_start", "ts": 1000.0, "task": "test"})];
        for i in 0..10 {
            events.push(json!({
                "type": "proof_created",
                "ts": 1001.0 + i as f64,
                "txid": format!("tx-{i}"),
                "proof_type": "Decision",
                "hash": format!("hash-{i}"),
                "sats_cost": 200,
                "iteration": i
            }));
        }
        write_transcript(&workspace, "task-1", &events);

        let result = recent_proofs(&workspace, 3);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 10);
        assert_eq!(parsed["proofs"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_task_costs_empty() {
        let (_tmp, workspace) = setup_workspace();
        let result = task_costs(&workspace, 5);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 0);
        assert!(parsed["summary"]
            .as_str()
            .unwrap()
            .contains("No tasks found"));
    }

    #[test]
    fn test_task_costs_with_data() {
        let (_tmp, workspace) = setup_workspace();
        let events = vec![
            json!({"type": "session_start", "ts": 1000.0, "task": "research"}),
            json!({"type": "think_response", "ts": 1001.0, "sats_effective": 500, "model": "gpt-5-mini", "prompt_tokens": 100, "completion_tokens": 50}),
            json!({"type": "tool_result", "ts": 1002.0, "name": "web_fetch", "sats_paid": 0, "success": true}),
            json!({"type": "proof_created", "ts": 1003.0, "txid": "abc", "sats_cost": 200}),
            json!({"type": "session_end", "ts": 1004.0, "status": "Complete", "iterations": 2}),
        ];
        write_transcript(&workspace, "task-abc", &events);

        let result = task_costs(&workspace, 5);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 1);
        let tasks = parsed["tasks"].as_array().unwrap();
        assert_eq!(tasks[0]["task_id"], "task-abc");
        assert_eq!(tasks[0]["status"], "Complete");
        assert_eq!(tasks[0]["total_sats"], 700); // 500 + 200
        assert_eq!(tasks[0]["model"], "gpt-5-mini");
        assert_eq!(tasks[0]["iterations"], 2);
        // Summary should include key metrics
        let summary = parsed["summary"].as_str().unwrap();
        assert!(summary.contains("1 of 1 tasks"), "summary: {summary}");
        assert!(summary.contains("700 sats"), "summary: {summary}");
        assert!(summary.contains("task-abc"), "summary: {summary}");
    }

    #[test]
    fn test_task_detail_not_found() {
        let (_tmp, workspace) = setup_workspace();
        let result = task_detail(&workspace, "nonexistent");
        assert!(result.contains("Error: Task 'nonexistent' not found"));
    }

    #[test]
    fn test_task_detail_with_data() {
        let (_tmp, workspace) = setup_workspace();
        let events = vec![
            json!({"type": "session_start", "ts": 1000.0, "task": "Analyze data"}),
            json!({"type": "think_response", "ts": 1001.0, "sats_effective": 500, "model": "gpt-5-mini", "prompt_tokens": 200, "completion_tokens": 100}),
            json!({"type": "tool_result", "ts": 1002.0, "name": "file_read", "sats_paid": 0, "success": true}),
            json!({"type": "tool_result", "ts": 1003.0, "name": "web_fetch", "sats_paid": 0, "success": true}),
            json!({"type": "proof_created", "ts": 1004.0, "txid": "proof1", "proof_type": "Decision", "sats_cost": 200}),
            json!({"type": "think_response", "ts": 1005.0, "sats_effective": 600, "model": "gpt-5-mini", "prompt_tokens": 300, "completion_tokens": 150}),
            json!({"type": "proof_created", "ts": 1006.0, "txid": "proof2", "proof_type": "TaskCompletion", "sats_cost": 200}),
            json!({"type": "session_end", "ts": 1007.0, "status": "Complete", "iterations": 2}),
        ];
        write_transcript(&workspace, "task-xyz", &events);

        let result = task_detail(&workspace, "task-xyz");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["task_id"], "task-xyz");
        assert_eq!(parsed["task"], "Analyze data");
        assert_eq!(parsed["status"], "Complete");
        assert_eq!(parsed["total_sats"], 1500); // 500 + 200 + 600 + 200
        assert_eq!(parsed["prompt_tokens"], 500); // 200 + 300
        assert_eq!(parsed["completion_tokens"], 250); // 100 + 150

        let tools = parsed["tools_used"].as_array().unwrap();
        assert!(tools.contains(&json!("file_read")));
        assert!(tools.contains(&json!("web_fetch")));

        let proofs = parsed["proofs"].as_array().unwrap();
        assert_eq!(proofs.len(), 2);

        // Summary should contain key task info
        let summary = parsed["summary"].as_str().unwrap();
        assert!(summary.contains("task-xyz"), "summary: {summary}");
        assert!(summary.contains("Analyze data"), "summary: {summary}");
        assert!(summary.contains("Complete"), "summary: {summary}");
        assert!(summary.contains("1500 sats"), "summary: {summary}");
        assert!(summary.contains("2 proofs"), "summary: {summary}");
    }

    #[test]
    fn test_task_detail_requires_task_id() {
        let (_tmp, workspace) = setup_workspace();
        // Simulate calling via the tool with missing task_id
        let ws = Arc::new(workspace);
        let tool = create_introspect_tool(ws);
        let result_future = (tool.execute)(json!({"action": "task_detail"}));
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(result_future);
        assert!(result.contains("Error: task_id is required"));
    }

    #[test]
    fn test_activity_summary_empty() {
        let (_tmp, workspace) = setup_workspace();
        let result = activity_summary(&workspace);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total_tasks"], 0);
        assert_eq!(parsed["total_proofs"], 0);
        assert_eq!(parsed["total_sats_spent"], 0);
        assert!(parsed["summary"].as_str().unwrap().contains("0 tasks"));
    }

    #[test]
    fn test_activity_summary_with_data() {
        let (_tmp, workspace) = setup_workspace();

        // Task 1
        let events1 = vec![
            json!({"type": "session_start", "ts": 1000.0, "task": "task one"}),
            json!({"type": "think_response", "ts": 1001.0, "sats_effective": 500, "model": "gpt-5-mini"}),
            json!({"type": "proof_created", "ts": 1002.0, "txid": "p1", "sats_cost": 200}),
        ];
        write_transcript(&workspace, "task-1", &events1);

        // Task 2
        let events2 = vec![
            json!({"type": "session_start", "ts": 2000.0, "task": "task two"}),
            json!({"type": "think_response", "ts": 2001.0, "sats_effective": 800, "model": "gpt-5-mini"}),
            json!({"type": "proof_created", "ts": 2002.0, "txid": "p2", "sats_cost": 200}),
            json!({"type": "proof_created", "ts": 2003.0, "txid": "p3", "sats_cost": 200}),
        ];
        write_transcript(&workspace, "task-2", &events2);

        let result = activity_summary(&workspace);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total_tasks"], 2);
        assert_eq!(parsed["total_proofs"], 3);
        assert_eq!(parsed["total_sats_spent"], 1900); // 500 + 200 + 800 + 200 + 200
        assert_eq!(parsed["services"]["llm"], 1300); // 500 + 800
        assert_eq!(parsed["services"]["proofs"], 600); // 200 + 200 + 200
                                                       // Summary should contain aggregate stats
        let summary = parsed["summary"].as_str().unwrap();
        assert!(summary.contains("2 tasks"), "summary: {summary}");
        assert!(summary.contains("3 proofs"), "summary: {summary}");
        assert!(summary.contains("1900 sats"), "summary: {summary}");
        assert!(summary.contains("llm"), "summary: {summary}");
    }

    #[test]
    fn test_activity_summary_with_budget_journal() {
        let (_tmp, workspace) = setup_workspace();

        // Write a budget.jsonl
        let budget_entries = [
            json!({"timestamp": "2026-03-23T00:00:00Z", "service": "llm", "operation": "think", "sats": 500}),
            json!({"timestamp": "2026-03-23T00:01:00Z", "service": "proofs", "operation": "create_proof", "sats": 200}),
        ];
        let lines: Vec<String> = budget_entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        fs::write(workspace.join("budget.jsonl"), lines.join("\n")).unwrap();

        let result = activity_summary(&workspace);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["budget_journal"]["total_sats"], 700);
        assert_eq!(parsed["budget_journal"]["services"]["llm"], 500);
        assert_eq!(parsed["budget_journal"]["services"]["proofs"], 200);
    }

    #[test]
    fn test_unknown_action() {
        let (_tmp, workspace) = setup_workspace();
        let ws = Arc::new(workspace);
        let tool = create_introspect_tool(ws);
        let result_future = (tool.execute)(json!({"action": "invalid"}));
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(result_future);
        assert!(result.contains("Error: unknown action 'invalid'"));
    }

    #[test]
    fn test_tool_metadata() {
        let tmp = TempDir::new().unwrap();
        let ws = Arc::new(tmp.path().to_path_buf());
        let tool = create_introspect_tool(ws);
        assert_eq!(tool.name, "introspect");
        assert_eq!(tool.category, "introspect");
        assert!(tool.cleanup.is_none());
        assert!(tool.description.contains("proofs"));
        assert!(tool.description.contains("costs"));
    }

    #[test]
    fn test_malformed_transcript_lines_skipped() {
        let (_tmp, workspace) = setup_workspace();
        let task_dir = workspace.join("tasks").join("task-bad");
        fs::create_dir_all(&task_dir).unwrap();
        let content = r#"{"type": "proof_created", "ts": 1000.0, "txid": "good", "sats_cost": 200}
not valid json at all
{"type": "proof_created", "ts": 1001.0, "txid": "also_good", "sats_cost": 200}
"#;
        fs::write(task_dir.join("transcript.jsonl"), content).unwrap();

        let result = recent_proofs(&workspace, 10);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 2);
    }

    #[test]
    fn test_session_jsonl_fallback() {
        let (_tmp, workspace) = setup_workspace();
        let task_dir = workspace.join("tasks").join("task-legacy");
        fs::create_dir_all(&task_dir).unwrap();
        // Write to session.jsonl (legacy name)
        let events =
            [json!({"type": "proof_created", "ts": 1000.0, "txid": "legacy-tx", "sats_cost": 200})];
        let lines: Vec<String> = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        fs::write(task_dir.join("session.jsonl"), lines.join("\n")).unwrap();

        let result = recent_proofs(&workspace, 5);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 1);
        assert_eq!(parsed["proofs"].as_array().unwrap()[0]["txid"], "legacy-tx");
    }

    #[test]
    fn test_count_clamped_to_max() {
        let (_tmp, workspace) = setup_workspace();
        let ws = Arc::new(workspace);
        let tool = create_introspect_tool(ws);
        // Pass count=100, should be clamped to MAX_COUNT (20)
        let result_future = (tool.execute)(json!({"action": "recent_proofs", "count": 100}));
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(result_future);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        // With no data, this just validates the clamping didn't cause an error
        assert_eq!(parsed["total"], 0);
    }

    #[test]
    fn test_task_detail_per_iteration_breakdown() {
        let (_tmp, workspace) = setup_workspace();
        let events = vec![
            json!({"type": "session_start", "ts": 1000.0, "task": "multi-iter"}),
            json!({"type": "think_response", "ts": 1001.0, "sats_effective": 500, "model": "gpt-5-mini", "prompt_tokens": 100, "completion_tokens": 50}),
            json!({"type": "tool_result", "ts": 1002.0, "name": "file_read", "sats_paid": 0, "success": true}),
            json!({"type": "think_response", "ts": 1003.0, "sats_effective": 600, "model": "gpt-5-mini", "prompt_tokens": 200, "completion_tokens": 100}),
            json!({"type": "tool_result", "ts": 1004.0, "name": "web_fetch", "sats_paid": 50, "success": true}),
            json!({"type": "session_end", "ts": 1005.0, "status": "Complete", "iterations": 2}),
        ];
        write_transcript(&workspace, "task-multi", &events);

        let result = task_detail(&workspace, "task-multi");
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let per_iter = parsed["per_iteration"].as_array().unwrap();
        assert_eq!(per_iter.len(), 2);
        // First iteration: 500 sats (LLM) + file_read tool
        assert_eq!(per_iter[0]["iteration"], 1);
        assert_eq!(per_iter[0]["sats"], 500);
        assert!(per_iter[0]["tools"]
            .as_array()
            .unwrap()
            .contains(&json!("file_read")));
        // Second iteration: 600 sats (LLM) + 50 sats (tool) + web_fetch tool
        assert_eq!(per_iter[1]["iteration"], 2);
        assert_eq!(per_iter[1]["sats"], 650);
        assert!(per_iter[1]["tools"]
            .as_array()
            .unwrap()
            .contains(&json!("web_fetch")));
    }
}
