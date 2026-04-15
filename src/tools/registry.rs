//! Tool registry — register, describe, and execute tools.
//!
//! Tools are how the worm acts on the world. Each tool has:
//!   - name: unique identifier
//!   - description: for the LLM system prompt
//!   - parameters: JSON schema for the LLM to generate valid calls
//!   - execute: the actual implementation
//!
//! Compatible with OpenAI function calling format.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use crate::error::DmError;

/// Async tool function type.
pub type ToolFunc =
    Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync>;

/// Async cleanup function type — called at task teardown to release resources.
pub type CleanupFunc = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Definition of a registered tool.
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub execute: ToolFunc,
    pub category: String,
    /// Optional cleanup function called at task teardown (e.g. shut down Chrome).
    pub cleanup: Option<CleanupFunc>,
    /// If true, this tool's full schema is deferred — only a lightweight hint
    /// appears in the system prompt. The agent uses `search_tools` to discover it.
    pub deferred: bool,
    /// If true, always include full schema in prompt regardless of deferral mode.
    /// Overrides `deferred`. Used for essential tools (file_read, execute_bash, etc.).
    pub always_load: bool,
    /// Short hint for deferred tools (~10 tokens). Shown in system prompt instead of
    /// full description. If None, derived from first sentence of description.
    pub search_hint: Option<String>,
}

impl ToolDef {
    /// Create a new ToolDef with sensible defaults for deferred loading fields.
    /// `deferred`, `always_load`, and `search_hint` default to false/None.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
        execute: ToolFunc,
        category: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            execute,
            category: category.into(),
            cleanup: None,
            deferred: false,
            always_load: false,
            search_hint: None,
        }
    }

    /// Set deferred flag (tool schema not sent to LLM, only a hint).
    pub fn with_deferred(mut self, deferred: bool) -> Self {
        self.deferred = deferred;
        self
    }

    /// Set always_load flag (overrides deferred, always sends full schema).
    pub fn with_always_load(mut self, always_load: bool) -> Self {
        self.always_load = always_load;
        self
    }

    /// Set a custom search hint (~10 tokens) for deferred discovery.
    pub fn with_search_hint(mut self, hint: impl Into<String>) -> Self {
        self.search_hint = Some(hint.into());
        self
    }

    /// Set a cleanup function called at task teardown.
    pub fn with_cleanup(mut self, cleanup: CleanupFunc) -> Self {
        self.cleanup = Some(cleanup);
        self
    }
}

/// Controls how tool schemas are loaded into the LLM context.
#[derive(Debug, Clone, PartialEq)]
pub enum DeferralMode {
    /// Always defer non-always_load tools — only hints in prompt.
    Always,
    /// Defer when tool schemas exceed threshold_pct of context window.
    Auto { threshold_pct: f64 },
    /// Never defer — all tools get full schemas (legacy behavior).
    Never,
}

impl Default for DeferralMode {
    fn default() -> Self {
        DeferralMode::Auto {
            threshold_pct: 50.0,
        }
    }
}

/// A scored search result from weighted tool search.
#[derive(Debug, Clone)]
pub struct ToolSearchResult {
    pub name: String,
    pub description: String,
    pub category: String,
    pub score: u32,
    pub match_reasons: Vec<String>,
}

/// Snapshot of a tool for search — includes deferred metadata.
#[derive(Debug, Clone)]
pub struct ToolSnapshot {
    pub name: String,
    pub description: String,
    pub category: String,
    pub deferred: bool,
    pub always_load: bool,
    pub hint: String,
}

/// Tools shown in every system prompt. All others are discoverable via `search_tools`.
pub const ALWAYS_ON_TOOLS: &[&str] = &[
    // Sandbox (6)
    "execute_bash",
    "file_read",
    "file_write",
    "file_search",
    "web_fetch",
    "continue_task",
    // Memory (2)
    "memory_store",
    "memory_search",
    // Wallet core (3)
    "wallet_balance",
    "wallet_identity",
    "wallet_call",
    // x402 generic (3)
    "discover_services",
    "discover_endpoints",
    "x402_call",
    // Discovery (2)
    "search_tools",
    "list_commands",
    // Delegation (1) — scoped cross-agent task commissioning (EPIC #329)
    "delegate_task",
];

/// Maps a tool's category to the certificate capability required to use it.
///
/// Certificate capabilities are declared in BRC-52 `fields.capabilities` as a
/// comma-separated string (e.g. `"tools,llm,wallet,messaging,x402,memory,schedule"`).
/// This function maps each tool category to the corresponding capability name.
pub fn required_capability(category: &str) -> &str {
    match category {
        "sandbox" | "system" => "tools",
        "wallet" => "wallet",
        "messagebox" | "conversation" => "messaging",
        "x402" => "tools",
        "memory" => "memory",
        "schedule" => "schedule",
        "browser" => "tools",
        "discovery" => "tools",
        "overlay" => "tools",
        "analytics" => "tools",
        "introspect" => "tools",
        "orchestration" => "tools",
        "delegation" => "messaging", // delegate_task sends a commission via MessageBox
        _ => "tools",                // Default: require "tools" capability
    }
}

/// Registry of available tools for the agent loop.
pub struct ToolRegistry {
    tools: HashMap<String, ToolDef>,
    allowed: Option<HashSet<String>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            allowed: None,
        }
    }

    /// Register a tool.
    pub fn register(&mut self, tool: ToolDef) {
        if self.tools.contains_key(&tool.name) {
            tracing::warn!("Overwriting existing tool: {}", tool.name);
        }
        tracing::debug!("Registered tool: {} ({})", tool.name, tool.category);
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(name)
    }

    /// Check if a tool is in the allow list.
    pub fn is_allowed(&self, name: &str) -> bool {
        match &self.allowed {
            None => self.tools.contains_key(name),
            Some(set) => set.contains(name),
        }
    }

    /// Set explicit allowlist. Only these tools can be executed.
    pub fn set_allowlist(&mut self, names: HashSet<String>) {
        self.allowed = Some(names);
    }

    /// Execute a tool by name with the given arguments.
    ///
    /// If `capabilities` is `Some`, checks that the tool's category is authorized
    /// by the certificate capability list. `"all"` bypasses all checks. `None`
    /// skips the check (backward compat for CLI, tests, MCP).
    pub async fn execute(
        &self,
        name: &str,
        arguments: Value,
        capabilities: Option<&[String]>,
    ) -> Result<String, DmError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| DmError::tool(format!("Unknown tool: {name}")))?;

        if !self.is_allowed(name) {
            return Err(DmError::tool(format!("Tool not allowed: {name}")));
        }

        // Capability enforcement: check if certificate grants access to this tool's category
        if let Some(caps) = capabilities {
            let required = required_capability(&tool.category);
            if !caps.iter().any(|c| c == required || c == "all") {
                return Err(DmError::tool(format!(
                    "Insufficient capability for tool '{}' (category '{}'): requires '{}' capability",
                    name, tool.category, required
                )));
            }
        }

        let result = (tool.execute)(arguments).await;
        Ok(result)
    }

    /// Return tool descriptions for the system prompt.
    pub fn list_descriptions(&self) -> Vec<crate::context::prompt::ToolDesc> {
        self.tools
            .values()
            .filter(|t| self.is_allowed(&t.name))
            .map(|t| crate::context::prompt::ToolDesc {
                name: t.name.clone(),
                description: t.description.clone(),
                category: t.category.clone(),
                deferred: t.deferred,
                hint: t.search_hint.clone(),
            })
            .collect()
    }

    /// Returns tools for the system prompt: always-on tools get full descriptions,
    /// deferred tools get lightweight hints. Respects the allowlist.
    pub fn list_prompt_tools(&self) -> Vec<crate::context::prompt::ToolDesc> {
        self.tools
            .values()
            .filter(|t| {
                self.is_allowed(&t.name)
                    && (t.always_load || ALWAYS_ON_TOOLS.contains(&t.name.as_str()))
            })
            .map(|t| {
                let is_deferred =
                    t.deferred && !t.always_load && !ALWAYS_ON_TOOLS.contains(&t.name.as_str());
                crate::context::prompt::ToolDesc {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    category: t.category.clone(),
                    deferred: is_deferred,
                    hint: t.search_hint.clone().or_else(|| {
                        if is_deferred {
                            Some(derive_hint(&t.description))
                        } else {
                            None
                        }
                    }),
                }
            })
            .collect()
    }

    /// Returns deferred tools (not always-on) for lightweight hints in the prompt.
    pub fn list_deferred_tools(&self) -> Vec<crate::context::prompt::ToolDesc> {
        self.tools
            .values()
            .filter(|t| {
                self.is_allowed(&t.name)
                    && t.deferred
                    && !t.always_load
                    && !ALWAYS_ON_TOOLS.contains(&t.name.as_str())
            })
            .map(|t| crate::context::prompt::ToolDesc {
                name: t.name.clone(),
                description: t.description.clone(),
                category: t.category.clone(),
                deferred: true,
                hint: t
                    .search_hint
                    .clone()
                    .or_else(|| Some(derive_hint(&t.description))),
            })
            .collect()
    }

    /// Returns (name, description, category) for all registered tools. Used by search_tools snapshot.
    pub fn all_tool_summaries(&self) -> Vec<(String, String, String)> {
        self.tools
            .values()
            .map(|t| (t.name.clone(), t.description.clone(), t.category.clone()))
            .collect()
    }

    /// Returns snapshots of all registered tools with deferred metadata.
    pub fn all_tool_snapshots(&self) -> Vec<ToolSnapshot> {
        self.tools
            .values()
            .map(|t| ToolSnapshot {
                name: t.name.clone(),
                description: t.description.clone(),
                category: t.category.clone(),
                deferred: t.deferred,
                always_load: t.always_load,
                hint: t
                    .search_hint
                    .clone()
                    .unwrap_or_else(|| derive_hint(&t.description)),
            })
            .collect()
    }

    /// Return tool definitions in OpenAI function calling format.
    pub fn to_openai_tools(&self) -> Vec<Value> {
        self.tools
            .values()
            .filter(|t| self.is_allowed(&t.name))
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                })
            })
            .collect()
    }

    /// Register multiple tools at once.
    pub fn register_many(&mut self, tools: Vec<ToolDef>) {
        for tool in tools {
            self.register(tool);
        }
    }

    /// Remove a tool by name.
    pub fn remove(&mut self, name: &str) {
        if self.tools.remove(name).is_some() {
            tracing::debug!("Removed tool: {name}");
        }
    }

    /// Check if a tool exists in the registry.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Run all registered cleanup functions (e.g. shut down Chrome).
    pub async fn cleanup_all(&self) {
        for tool in self.tools.values() {
            if let Some(ref cleanup) = tool.cleanup {
                cleanup().await;
            }
        }
    }

    /// Returns the OpenAI function schema for a tool by name (for discovered tools).
    pub fn tool_schema(&self, name: &str) -> Option<Value> {
        self.tools.get(name).map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                },
            })
        })
    }
}

/// Derive a short hint (~10 tokens) from a tool description.
/// Takes the first sentence or first 80 characters, whichever is shorter.
pub fn derive_hint(description: &str) -> String {
    // Find first sentence boundary
    let first_sentence_end = description
        .find(". ")
        .or_else(|| description.find(".\n"))
        .map(|i| i + 1)
        .unwrap_or(description.len());

    let end = first_sentence_end.min(80);
    let hint: String = description.chars().take(end).collect();
    if hint.len() < description.len() && !hint.ends_with('.') {
        format!("{hint}...")
    } else {
        hint
    }
}

/// Weighted search over tool snapshots.
///
/// Scoring: exact name match (10), name substring (5), hint match (4), description match (2).
/// Required terms (prefixed with `+`) must appear somewhere in the tool.
/// Results sorted by score descending.
pub fn weighted_search(
    snapshots: &[ToolSnapshot],
    query: &str,
    limit: usize,
) -> Vec<ToolSearchResult> {
    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();

    if terms.is_empty() {
        return Vec::new();
    }

    // Split required terms (prefixed with +) from regular terms
    let mut required: Vec<&str> = Vec::new();
    let mut regular: Vec<&str> = Vec::new();
    for term in &terms {
        if let Some(stripped) = term.strip_prefix('+') {
            if !stripped.is_empty() {
                required.push(stripped);
            }
        } else {
            regular.push(term);
        }
    }

    let mut results: Vec<ToolSearchResult> = snapshots
        .iter()
        .filter_map(|snap| {
            let name_lower = snap.name.to_lowercase();
            let hint_lower = snap.hint.to_lowercase();
            let desc_lower = snap.description.to_lowercase();
            let cat_lower = snap.category.to_lowercase();
            let haystack = format!("{name_lower} {hint_lower} {desc_lower} {cat_lower}");

            // Required terms must all appear somewhere
            for req in &required {
                if !haystack.contains(req) {
                    return None;
                }
            }

            let mut score: u32 = 0;
            let mut reasons: Vec<String> = Vec::new();

            let all_terms: Vec<&str> = required.iter().chain(regular.iter()).copied().collect();

            for term in &all_terms {
                if name_lower == *term {
                    score += 10;
                    reasons.push(format!("name_exact:{term}"));
                } else if name_lower.contains(term) {
                    score += 5;
                    reasons.push(format!("name_sub:{term}"));
                } else if hint_lower.contains(term) {
                    score += 4;
                    reasons.push(format!("hint:{term}"));
                } else if desc_lower.contains(term) {
                    score += 2;
                    reasons.push(format!("desc:{term}"));
                } else if cat_lower.contains(term) {
                    score += 1;
                    reasons.push(format!("cat:{term}"));
                }
            }

            if score > 0 {
                Some(ToolSearchResult {
                    name: snap.name.clone(),
                    description: snap.description.clone(),
                    category: snap.category.clone(),
                    score,
                    match_reasons: reasons,
                })
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(limit);
    results
}

/// Direct tool lookup by name(s). Used for `select:name1,name2` syntax.
pub fn select_tools(snapshots: &[ToolSnapshot], names: &str) -> Vec<ToolSearchResult> {
    let requested: Vec<&str> = names.split(',').map(|s| s.trim()).collect();
    let mut results = Vec::new();
    for name in requested {
        if let Some(snap) = snapshots.iter().find(|s| s.name == name) {
            results.push(ToolSearchResult {
                name: snap.name.clone(),
                description: snap.description.clone(),
                category: snap.category.clone(),
                score: 10,
                match_reasons: vec!["select".to_string()],
            });
        }
    }
    results
}
