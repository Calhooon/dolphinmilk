//! Hook executor — dispatches events to registered handlers.
//!
//! Supports 4 handler modes:
//! - **Command**: Spawn a shell process, capture output, enforce timeout. FULLY IMPLEMENTED.
//! - **Http**: POST event JSON to an external webhook URL with SSRF guard, header interpolation,
//!   and response-driven action (allow/block/modify). FULLY IMPLEMENTED.
//! - **Prompt**: Inject a prompt template into context. STUBBED.
//! - **Agent**: Spawn a sub-agent task. STUBBED.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::events::HookEvent;

/// The result of executing a hook handler.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HookResult {
    /// Allow the operation to proceed.
    Allow,
    /// Block the operation with a reason.
    Block(String),
    /// Modify the operation by providing replacement data.
    Modify(Value),
}

/// How a hook handler executes when its event fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookHandler {
    /// Execute a shell command. Template variables from the event
    /// (e.g., `{tool_name}`) are substituted into `cmd` before execution.
    Command {
        cmd: String,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    /// Inject a prompt template into the agent's context.
    /// NOT YET IMPLEMENTED — returns `HookResult::Allow`.
    Prompt { template: String },
    /// Call an external HTTP endpoint with SSRF protection.
    ///
    /// POSTs event JSON to the URL (or other method if specified).
    /// Custom headers support `$VAR` environment variable interpolation.
    /// Response JSON with `{ "action": "block", "reason": "..." }` blocks;
    /// `{ "action": "modify", "data": {...} }` modifies. All other responses
    /// and errors fail-open (return `Allow`).
    Http {
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    /// Spawn a sub-agent task to handle the event.
    /// NOT YET IMPLEMENTED — returns `HookResult::Allow`.
    Agent {
        task_description: String,
        #[serde(default)]
        budget_sats: u64,
    },
}

fn default_timeout() -> u64 {
    5000
}

fn default_http_method() -> String {
    "POST".to_string()
}

impl HookHandler {
    /// Execute this handler for the given event.
    ///
    /// For `Command` mode, spawns a shell process with event data as
    /// template variables and environment variables. The process is
    /// killed if it exceeds `timeout_ms`.
    ///
    /// Other modes are stubbed and return `HookResult::Allow`.
    pub async fn execute(&self, event: &HookEvent, timeout: Duration) -> HookResult {
        match self {
            HookHandler::Command { cmd, timeout_ms } => {
                let effective_timeout = Duration::from_millis(if *timeout_ms > 0 {
                    *timeout_ms
                } else {
                    timeout.as_millis() as u64
                });
                execute_command(cmd, event, effective_timeout).await
            }
            HookHandler::Prompt { .. } => {
                tracing::warn!("Hook handler mode 'prompt' is not yet implemented");
                HookResult::Allow
            }
            HookHandler::Http {
                url,
                method,
                headers,
            } => execute_http(url, method, headers, event, timeout).await,
            HookHandler::Agent { .. } => {
                tracing::warn!("Hook handler mode 'agent' is not yet implemented");
                HookResult::Allow
            }
        }
    }
}

// =============================================================================
// SSRF guard helpers
// =============================================================================

/// Check whether an IP address belongs to a private, loopback, or link-local range.
///
/// Blocked ranges:
/// - `127.0.0.0/8` (loopback)
/// - `10.0.0.0/8` (private)
/// - `172.16.0.0/12` (private)
/// - `192.168.0.0/16` (private)
/// - `169.254.0.0/16` (link-local)
/// - `::1` (IPv6 loopback)
pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 127.0.0.0/8 — loopback
            if octets[0] == 127 {
                return true;
            }
            // 10.0.0.0/8 — private
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12 — private (172.16.x.x through 172.31.x.x)
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }
            // 192.168.0.0/16 — private
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // 169.254.0.0/16 — link-local
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            // ::1 — IPv6 loopback
            *v6 == std::net::Ipv6Addr::LOCALHOST
        }
    }
}

/// Check whether a URL targets a private/loopback address by resolving its hostname.
///
/// Returns `true` (blocked) if:
/// - The hostname is `localhost` (blocked without DNS resolution)
/// - DNS resolution yields any private IP address
/// - The URL cannot be parsed
///
/// Returns `false` (allowed) if the hostname resolves to only public IPs.
pub fn is_private_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return true, // Unparseable URLs are blocked
    };

    let host_str = match parsed.host_str() {
        Some(h) => h,
        None => return true, // No host = blocked
    };

    // Block "localhost" directly, regardless of what it resolves to
    if host_str.eq_ignore_ascii_case("localhost") {
        return true;
    }

    // If the host is already an IP literal, check it directly
    if let Ok(ip) = host_str.parse::<IpAddr>() {
        return is_private_ip(&ip);
    }

    // Resolve hostname and check all returned addresses
    let port = parsed.port().unwrap_or(80);
    let socket_addr = format!("{host_str}:{port}");
    match std::net::ToSocketAddrs::to_socket_addrs(&socket_addr) {
        Ok(addrs) => {
            for addr in addrs {
                if is_private_ip(&addr.ip()) {
                    return true;
                }
            }
            false
        }
        Err(_) => {
            // DNS resolution failure — fail-open for SSRF check but the HTTP
            // request will fail anyway, so this is safe. However, to be safe
            // against DNS rebinding, block unresolvable hosts.
            true
        }
    }
}

/// Interpolate `$VAR` patterns in a string with environment variable values.
///
/// Patterns like `$MY_TOKEN` or `$API_KEY` are replaced with their
/// environment variable values. Unknown variables are left as-is.
fn interpolate_env_vars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Collect the variable name (alphanumeric + underscore)
            let mut var_name = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    var_name.push(next);
                    chars.next();
                } else {
                    break;
                }
            }

            if var_name.is_empty() {
                result.push('$');
            } else if let Ok(val) = std::env::var(&var_name) {
                result.push_str(&val);
            } else {
                // Unknown variable — leave the original text
                result.push('$');
                result.push_str(&var_name);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

// =============================================================================
// HTTP hook handler
// =============================================================================

/// Execute an HTTP webhook call with the event payload.
///
/// Sends the event as a JSON body to the specified URL. The response is parsed
/// to determine the hook result:
///
/// - HTTP 2xx with JSON `{ "action": "block", "reason": "..." }` → `HookResult::Block`
/// - HTTP 2xx with JSON `{ "action": "modify", "data": {...} }` → `HookResult::Modify`
/// - HTTP 2xx (other) → `HookResult::Allow`
/// - HTTP 4xx/5xx → `HookResult::Allow` (fail-open)
/// - Network error → `HookResult::Allow` (fail-open)
///
/// SSRF protection blocks requests to private IP ranges before connecting.
async fn execute_http(
    url: &str,
    method: &str,
    headers: &HashMap<String, String>,
    event: &HookEvent,
    timeout: Duration,
) -> HookResult {
    // SSRF guard: block private/loopback addresses.
    // The DOLPHIN_MILK_HOOK_SSRF_BYPASS env var disables the guard for testing with
    // local mock servers. Never set this in production.
    let ssrf_bypass = std::env::var("DOLPHIN_MILK_HOOK_SSRF_BYPASS").is_ok();
    if !ssrf_bypass && is_private_url(url) {
        tracing::warn!("Hook HTTP handler blocked by SSRF guard: {url}");
        return HookResult::Allow;
    }

    // Serialize event to JSON
    let event_json = match serde_json::to_value(event) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to serialize hook event for HTTP: {e}");
            return HookResult::Allow;
        }
    };

    // Build the HTTP client with timeout
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to build HTTP client for hook: {e}");
            return HookResult::Allow;
        }
    };

    // Determine the HTTP method
    let req_method = match method.to_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        other => {
            tracing::warn!("Unsupported HTTP method for hook: {other}, defaulting to POST");
            reqwest::Method::POST
        }
    };

    // Build request with headers (environment variable interpolation)
    let mut request = client.request(req_method, url).json(&event_json);

    for (key, value) in headers {
        let interpolated_value = interpolate_env_vars(value);
        request = request.header(key.as_str(), interpolated_value);
    }

    // Send the request
    let response = match request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!("Hook HTTP request failed: {e}");
            return HookResult::Allow;
        }
    };

    let status = response.status();

    // Non-2xx → fail-open
    if !status.is_success() {
        tracing::warn!("Hook HTTP endpoint returned {status} for {url} — failing open",);
        return HookResult::Allow;
    }

    // Parse response body
    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Failed to read hook HTTP response body: {e}");
            return HookResult::Allow;
        }
    };

    // Try to parse as JSON with action field
    if let Ok(parsed) = serde_json::from_str::<Value>(&body) {
        if let Some(action) = parsed.get("action").and_then(|a| a.as_str()) {
            match action {
                "block" => {
                    let reason = parsed
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("blocked by webhook")
                        .to_string();
                    tracing::info!("Hook HTTP endpoint blocked: {reason}");
                    return HookResult::Block(reason);
                }
                "modify" => {
                    if let Some(data) = parsed.get("data") {
                        tracing::info!("Hook HTTP endpoint returned modification");
                        return HookResult::Modify(data.clone());
                    }
                    tracing::warn!(
                        "Hook HTTP endpoint returned 'modify' action without 'data' field"
                    );
                    return HookResult::Allow;
                }
                _ => {
                    // Unknown action (including "allow") → Allow
                }
            }
        }
    }

    HookResult::Allow
}

/// Execute a shell command with event data substituted as template variables.
///
/// Template variables like `{tool_name}`, `{error}`, `{task_id}` are
/// extracted from the event's JSON representation and replaced in the
/// command string.
///
/// The command's exit code determines the result:
/// - Exit 0 → `HookResult::Allow`
/// - Exit 1 → `HookResult::Block` with stdout as reason
/// - Exit 2 → `HookResult::Modify` with stdout parsed as JSON
/// - Other / timeout / error → `HookResult::Allow` (fail-open)
///
/// The event JSON is also passed via the `HOOK_EVENT` environment variable.
async fn execute_command(cmd: &str, event: &HookEvent, timeout: Duration) -> HookResult {
    // Serialize event to JSON for template substitution and env var
    let event_json = match serde_json::to_value(event) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to serialize hook event: {e}");
            return HookResult::Allow;
        }
    };

    // Substitute template variables from event fields
    let mut expanded_cmd = cmd.to_string();
    if let Value::Object(map) = &event_json {
        for (key, value) in map {
            let placeholder = format!("{{{key}}}");
            let replacement = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            expanded_cmd = expanded_cmd.replace(&placeholder, &replacement);
        }
    }

    let event_str = event_json.to_string();

    // Spawn the command with a timeout
    let result = tokio::time::timeout(timeout, async {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&expanded_cmd)
            .env("HOOK_EVENT", &event_str)
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if !stderr.is_empty() {
                tracing::debug!("Hook command stderr: {stderr}");
            }

            match output.status.code() {
                Some(0) => {
                    tracing::debug!("Hook command succeeded: {expanded_cmd}");
                    HookResult::Allow
                }
                Some(1) => {
                    let reason = if stdout.is_empty() {
                        "blocked by hook".to_string()
                    } else {
                        stdout
                    };
                    tracing::info!("Hook command blocked: {reason}");
                    HookResult::Block(reason)
                }
                Some(2) => {
                    // Try to parse stdout as JSON for Modify
                    match serde_json::from_str::<Value>(&stdout) {
                        Ok(val) => {
                            tracing::info!("Hook command returned modification");
                            HookResult::Modify(val)
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Hook command exited 2 but stdout is not valid JSON: {e}"
                            );
                            HookResult::Allow
                        }
                    }
                }
                code => {
                    tracing::warn!("Hook command exited with code {:?}: {expanded_cmd}", code);
                    HookResult::Allow
                }
            }
        }
        Ok(Err(e)) => {
            tracing::warn!("Hook command failed to spawn: {e}");
            HookResult::Allow
        }
        Err(_) => {
            tracing::warn!(
                "Hook command timed out after {}ms: {expanded_cmd}",
                timeout.as_millis()
            );
            HookResult::Allow
        }
    }
}
