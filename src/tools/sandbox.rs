//! Sandbox tools — bash execution, file operations.
//!
//! These are the paper's three meta-capabilities made explicit:
//!   1. Code execution (execute_bash)
//!   2. File management (file_read, file_write, file_search)

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

use crate::security::command_analyzer;
use crate::tools::registry::ToolDef;

/// Maximum timeout for bash commands (seconds).
///
/// Raised from 120s to 600s on 2026-04-13 for the DolphinMilkShake #23
/// per-record proof batch use case: at ~3.3s per sequential wallet
/// createAction call and 100 records per Worker cycle, batches can take
/// ~330s wall clock. Keeping 120s forced Worker to either tiny batches
/// or risky parallelism. 600s leaves ~270s margin for the 100-record case.
const MAX_TIMEOUT: u64 = 600;

/// Proportion of context window allowed for a single tool result (30%).
const MAX_TOOL_RESULT_CONTEXT_SHARE: f64 = 0.3;

/// Absolute ceiling on tool result size (chars).
const HARD_MAX_TOOL_RESULT_CHARS: usize = 400_000;

/// Approximate chars per token for proportional calculations.
const CHARS_PER_TOKEN: usize = 4;

/// Compute the max chars for a single tool result based on context window size.
pub fn max_tool_result_chars(context_window_tokens: usize) -> usize {
    let max_tokens = (context_window_tokens as f64 * MAX_TOOL_RESULT_CONTEXT_SHARE) as usize;
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    max_chars.min(HARD_MAX_TOOL_RESULT_CHARS)
}

/// Truncate a tool result string at a newline boundary, with a truncation marker.
pub fn truncate_tool_result(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    // Look for a newline boundary within the last 20% of allowed space
    let search_start = (max_chars as f64 * 0.8) as usize;
    let break_point = text[search_start..max_chars]
        .rfind('\n')
        .map(|i| search_start + i)
        .unwrap_or(max_chars);
    format!(
        "{}\n\n[truncated — original was {} chars, showing first {}]",
        &text[..break_point],
        text.len(),
        break_point
    )
}

/// Paths the agent must never write to or modify.
const PROTECTED_PATHS: &[&str] = &["dolphin-milk.toml", "Cargo.toml", "Cargo.lock"];

/// Path patterns the agent must never access.
const BLOCKED_PATTERNS: &[&str] = &[".ssh", ".gnupg", ".aws", "/etc/", ".env"];

/// Check if a path is protected from agent modification.
fn is_protected_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    for protected in PROTECTED_PATHS {
        if normalized.ends_with(protected) || normalized.contains(&format!("/{protected}")) {
            return true;
        }
    }
    for pattern in BLOCKED_PATTERNS {
        if normalized.contains(pattern) {
            return true;
        }
    }
    false
}

/// Forbidden command patterns that should never be executed.
const FORBIDDEN_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    ":(){ :|:& };:", // fork bomb
    "mkfs.",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "dd if=/dev/zero of=/dev/",
    "dd if=/dev/random of=/dev/",
    "> /dev/sda",
    "chmod -R 777 /",
    "curl | sh",
    "curl | bash",
    "wget | sh",
    "wget | bash",
];

/// Check if a command matches any forbidden pattern.
fn is_forbidden_command(cmd: &str) -> bool {
    let normalized = cmd.trim().to_lowercase();
    for pattern in FORBIDDEN_PATTERNS {
        if normalized.contains(&pattern.to_lowercase()) {
            return true;
        }
    }
    false
}

async fn execute_bash_with_workspace(
    params: Value,
    default_cwd: PathBuf,
    max_output: usize,
) -> String {
    let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let timeout = params
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(MAX_TIMEOUT)
        .min(MAX_TIMEOUT);
    let cwd = params.get("cwd").and_then(|v| v.as_str());

    if command.is_empty() {
        return "Error: no command provided".to_string();
    }

    if is_forbidden_command(command) {
        return "Error: command matches a forbidden pattern and cannot be executed".to_string();
    }

    // Compound command security analysis: decompose pipes, chains, subshells
    // and detect dangerous combinations like data exfiltration.
    let analysis = command_analyzer::analyze_command(command);
    if analysis.blocked {
        let reason = analysis
            .block_reason
            .unwrap_or_else(|| "blocked by security analysis".to_string());
        return format!(
            "Error: command blocked by security analysis (risk: {}). {}",
            analysis.overall_risk, reason
        );
    }

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    } else {
        cmd.current_dir(&default_cwd);
    }

    let result = tokio::time::timeout(Duration::from_secs(timeout), cmd.output()).await;

    match result {
        Err(_) => format!("Error: command timed out after {timeout}s"),
        Ok(Err(e)) => format!("Error: {e}"),
        Ok(Ok(output)) => {
            let mut text = String::new();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if !stdout.is_empty() {
                text.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !text.is_empty() {
                    text.push_str("\n--- stderr ---\n");
                }
                text.push_str(&stderr);
            }

            if text.is_empty() {
                text = format!("(exit code {})", output.status.code().unwrap_or(-1));
            } else if !output.status.success() {
                text.push_str(&format!(
                    "\n(exit code {})",
                    output.status.code().unwrap_or(-1)
                ));
            }

            if text.len() > max_output {
                text = truncate_tool_result(&text, max_output);
            }

            text
        }
    }
}

async fn file_read(params: Value, max_file_read: usize) -> String {
    let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    if path_str.is_empty() {
        return "Error: no path provided".to_string();
    }

    let path = Path::new(path_str);
    if !path.exists() {
        return format!("Error: file not found: {path_str}");
    }
    if !path.is_file() {
        return format!("Error: not a file: {path_str}");
    }

    // Pre-flight size check: reject files larger than the output cap before reading.
    // This prevents loading multi-GB binaries (e.g. target/ artifacts) into memory.
    match tokio::fs::metadata(path).await {
        Ok(meta) => {
            let file_size = meta.len() as usize;
            if file_size > max_file_read * 4 {
                return format!(
                    "Error: file too large ({:.1} MB, max {:.1} MB). Use offset/limit to read a portion.",
                    file_size as f64 / 1_048_576.0,
                    (max_file_read * 4) as f64 / 1_048_576.0,
                );
            }
        }
        Err(e) => return format!("Error reading metadata for {path_str}: {e}"),
    }

    match tokio::fs::read_to_string(path).await {
        Err(e) => format!("Error reading {path_str}: {e}"),
        Ok(mut content) => {
            if content.len() > max_file_read {
                content = truncate_tool_result(&content, max_file_read);
            }

            if offset > 0 || limit > 0 {
                let lines: Vec<&str> = content.lines().collect();
                let start = offset.min(lines.len());
                let end = if limit > 0 {
                    (start + limit).min(lines.len())
                } else {
                    lines.len()
                };
                lines[start..end].join("\n")
            } else {
                content
            }
        }
    }
}

async fn file_write(params: Value, workspace: PathBuf) -> String {
    let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

    if path_str.is_empty() {
        return "Error: no path provided".to_string();
    }

    if is_protected_path(path_str) {
        return format!(
            "Error: path '{}' is protected and cannot be modified by the agent",
            path_str
        );
    }

    // Resolve relative paths against workspace (like execute_bash does)
    let path = Path::new(path_str);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };

    if let Some(parent) = resolved.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return format!("Error creating directories: {e}");
        }
    }

    let display = resolved.display();
    match tokio::fs::write(&resolved, content).await {
        Ok(_) => format!("Written {} bytes to {display}", content.len()),
        Err(e) => format!("Error writing {display}: {e}"),
    }
}

/// Directories excluded from file_search by default (build artifacts, deps, runtime data).
pub const SEARCH_EXCLUDED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".claude",
    ".claude-docs-logs",
    ".playwright-mcp",
];

pub fn is_excluded_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    SEARCH_EXCLUDED_DIRS.iter().any(|dir| {
        normalized == *dir
            || normalized.starts_with(&format!("{dir}/"))
            || normalized.contains(&format!("/{dir}/"))
            || normalized.starts_with(&format!("./{dir}/"))
    })
}

async fn file_search(params: Value) -> String {
    let pattern = params.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    let directory = params
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let content_pattern = params
        .get("content_pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let max_results = params
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;

    if pattern.is_empty() && content_pattern.is_empty() {
        return "Error: provide 'pattern' (glob) or 'content_pattern' (grep)".to_string();
    }

    let mut results: Vec<String> = Vec::new();

    if !pattern.is_empty() {
        let glob_pattern = format!("{directory}/{pattern}");
        if let Ok(entries) = glob::glob(&glob_pattern) {
            for entry in entries.flatten() {
                let path_str = entry.to_string_lossy().to_string();
                if is_excluded_path(&path_str) {
                    continue;
                }
                results.push(path_str);
                if results.len() >= max_results {
                    break;
                }
            }
        }
    }

    if !content_pattern.is_empty() {
        let mut grep_args: Vec<String> = vec!["-rn".to_string(), "-l".to_string()];
        for dir in SEARCH_EXCLUDED_DIRS {
            grep_args.push(format!("--exclude-dir={dir}"));
        }
        grep_args.push(content_pattern.to_string());
        grep_args.push(directory.to_string());

        let output = Command::new("grep").args(&grep_args).output().await;

        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().take(max_results) {
                if !is_excluded_path(line) {
                    results.push(line.to_string());
                }
            }
        }
    }

    if results.is_empty() {
        "No matches found".to_string()
    } else {
        results.join("\n")
    }
}

async fn web_fetch_impl(params: Value, max_chars: usize) -> String {
    let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");

    if url.is_empty() {
        return "Error: url is required".to_string();
    }

    // Basic URL validation
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return "Error: url must start with http:// or https://".to_string();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return format!("Error: failed to create HTTP client: {e}"),
    };

    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status();
            if !status.is_success() {
                return format!("Error: HTTP {status}");
            }
            match response.text().await {
                Ok(text) => {
                    if text.len() > max_chars {
                        truncate_tool_result(&text, max_chars)
                    } else {
                        text
                    }
                }
                Err(e) => format!("Error: failed to read response body: {e}"),
            }
        }
        Err(e) => format!("Error: request failed: {e}"),
    }
}

/// Create all sandbox tool definitions.
///
/// `context_window_tokens` controls proportional tool result capping (30% of context).
pub fn all_sandbox_tools(workspace: PathBuf, context_window_tokens: usize) -> Vec<ToolDef> {
    let tool_max_chars = max_tool_result_chars(context_window_tokens);
    let bash_workspace = workspace.clone();
    let ws_display = workspace.display().to_string();
    let bash_max = tool_max_chars;
    let file_max = tool_max_chars;
    let web_max = tool_max_chars;
    let mut tools = vec![
        ToolDef {
            name: "execute_bash".to_string(),
            description: format!(
                "Execute a shell command. Returns stdout+stderr. Timeout: 600s max. \
                 Default working directory: {ws_display}"
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The shell command to execute"},
                    "timeout": {"type": "integer", "description": "Timeout in seconds (max 600)"},
                    "cwd": {"type": "string", "description": format!("Working directory (default: {ws_display})")},
                },
                "required": ["command"],
            }),
            execute: Box::new(move |params| {
                let ws = bash_workspace.clone();
                Box::pin(execute_bash_with_workspace(params, ws, bash_max))
            }),
            category: "sandbox".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        },
        ToolDef {
            name: "file_read".to_string(),
            description: "Read a file's contents. Supports optional line offset and limit."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute or relative file path"},
                    "offset": {"type": "integer", "description": "Start from this line number (0-indexed)"},
                    "limit": {"type": "integer", "description": "Max number of lines to return"},
                },
                "required": ["path"],
            }),
            execute: Box::new(move |params| Box::pin(file_read(params, file_max))),
            category: "sandbox".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        },
        ToolDef {
            name: "file_write".to_string(),
            description: format!(
                "Create or overwrite a file with the given content. \
                 Relative paths resolve to workspace: {ws_display}"
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": format!("File path to write to (relative paths resolve to {ws_display})")},
                    "content": {"type": "string", "description": "Content to write"},
                },
                "required": ["path", "content"],
            }),
            execute: {
                let ws = workspace.clone();
                Box::new(move |params| {
                    let ws = ws.clone();
                    Box::pin(file_write(params, ws))
                })
            },
            category: "sandbox".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        },
        ToolDef {
            name: "file_search".to_string(),
            description: "Search for files by glob pattern or grep through file contents."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Glob pattern (e.g., '**/*.py')"},
                    "directory": {"type": "string", "description": "Directory to search in (default: '.')"},
                    "content_pattern": {"type": "string", "description": "Search file contents for this regex pattern"},
                    "max_results": {"type": "integer", "description": "Max results to return (default: 50)"},
                },
            }),
            execute: Box::new(|params| Box::pin(file_search(params))),
            category: "sandbox".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        },
        ToolDef {
            name: "web_fetch".to_string(),
            description: "Fetch a web page via HTTP GET. Free (no payment). \
                Returns the page content as text, truncated to 50K characters. \
                Useful for reading documentation, APIs, or web content."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch (must start with http:// or https://)"
                    }
                },
                "required": ["url"]
            }),
            execute: Box::new(move |params| Box::pin(web_fetch_impl(params, web_max))),
            category: "sandbox".to_string(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        },
    ];

    // read_tool_output tool — reads full original output from session transcript
    let transcript_workspace = workspace.clone();
    tools.push(ToolDef {
        name: "read_tool_output".to_string(),
        description: "Read the full, original output of a previous tool call from the session transcript. Use when a tool result was large and you received a preview instead of the complete data.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "call_id": {
                    "type": "string",
                    "description": "The tool_call_id to retrieve the full output for"
                }
            },
            "required": ["call_id"]
        }),
        execute: Box::new(move |params: Value| {
            let ws = transcript_workspace.clone();
            Box::pin(async move {
                let call_id = match params.get("call_id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return "Error: 'call_id' parameter is required".to_string(),
                };

                // Try transcript.jsonl first (current), then session.jsonl (legacy)
                let transcript_path = ws.join("transcript.jsonl");
                let session_path = ws.join("session.jsonl");
                let path = if transcript_path.exists() {
                    transcript_path
                } else {
                    session_path
                };

                match std::fs::read_to_string(&path) {
                    Ok(contents) => {
                        for line in contents.lines() {
                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                                if event.get("type").and_then(|v| v.as_str())
                                    == Some("tool_result")
                                    && event.get("call_id").and_then(|v| v.as_str())
                                        == Some(call_id)
                                {
                                    if let Some(content) =
                                        event.get("content").and_then(|v| v.as_str())
                                    {
                                        return content.to_string();
                                    }
                                }
                            }
                        }
                        format!("No tool_result found for call_id '{}'", call_id)
                    }
                    Err(e) => format!("Error reading transcript: {}", e),
                }
            })
        }),
        category: "system".to_string(),
        cleanup: None,
    deferred: true,
    always_load: false,
    search_hint: Some("Read full original output of a previous tool call".to_string()),
    });

    // continue_task tool — saves current progress and schedules resumption
    let ws = workspace;
    tools.push(ToolDef {
        name: "continue_task".into(),
        description: "Save current progress and schedule resumption. Use when a task needs more time, external input, or should continue after a delay.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why the task is pausing"
                },
                "delay_seconds": {
                    "type": "integer",
                    "description": "Seconds to wait before resuming (0 = next heartbeat cycle)"
                }
            },
            "required": ["reason"]
        }),
        category: "system".into(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        execute: Box::new(move |params| {
            let ws = ws.clone();
            Box::pin(async move {
                let reason = params.get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unspecified");
                let delay = params.get("delay_seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now();
                let wake_at = if delay > 0 {
                    Some((now + chrono::Duration::seconds(delay as i64)).to_rfc3339())
                } else {
                    None
                };

                let cont_dir = ws.join("continuations");
                if let Err(e) = tokio::fs::create_dir_all(&cont_dir).await {
                    return format!("Error creating continuations dir: {e}");
                }

                let state = serde_json::json!({
                    "id": id,
                    "reason": reason,
                    "wake_at": wake_at,
                    "created_at": now.to_rfc3339(),
                });

                let path = cont_dir.join(format!("{id}.json"));
                match tokio::fs::write(&path, serde_json::to_string_pretty(&state).unwrap_or_default()).await {
                    Ok(_) => serde_json::json!({
                        "status": "continuation_saved",
                        "continuation_id": id,
                        "resume_at": wake_at.unwrap_or_else(|| "next_heartbeat".into()),
                    }).to_string(),
                    Err(e) => format!("Error saving continuation: {e}"),
                }
            })
        }),
    });

    tools
}
