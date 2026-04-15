//! Agent definitions — load agent personas from .md files with YAML frontmatter.
//!
//! Agents are defined as markdown files with YAML frontmatter specifying
//! their name, description, model, capabilities, max_iterations, and budget_fraction.
//!
//! Priority order (highest wins): project `.dolphin-milk/agents/` > user `~/.dolphin-milk/agents/` > built-in

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed agent definition from a .md file.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    /// Unique agent name (from frontmatter).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// LLM model to use (e.g., "gpt-5-mini"). None = use parent's model.
    pub model: Option<String>,
    /// Required BRC-52 capabilities. Agent unavailable if parent cert lacks these.
    pub capabilities: Vec<String>,
    /// Maximum iterations for this agent. None = use default (50).
    pub max_iterations: Option<u32>,
    /// Fraction of parent's budget to allocate (0.0–1.0). Default: 0.1.
    pub budget_fraction: f64,
    /// Tools this agent is allowed to use. Empty = all tools.
    pub allowed_tools: Vec<String>,
    /// The instruction content (markdown body after frontmatter).
    pub instructions: String,
    /// Source path for debugging.
    pub source: PathBuf,
}

/// YAML frontmatter structure for agent .md files.
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    max_iterations: Option<u32>,
    #[serde(default = "default_budget_fraction")]
    budget_fraction: f64,
    #[serde(default)]
    allowed_tools: Vec<String>,
}

fn default_budget_fraction() -> f64 {
    0.1
}

/// Parse a single agent definition from a markdown file with YAML frontmatter.
pub fn parse_agent_file(path: &Path) -> Result<AgentDefinition, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    parse_agent_content(&content, path)
}

/// Parse agent definition from content string (for testing).
pub fn parse_agent_content(content: &str, source: &Path) -> Result<AgentDefinition, String> {
    // Extract YAML frontmatter between --- delimiters
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(format!(
            "Agent file {} missing YAML frontmatter (must start with ---)",
            source.display()
        ));
    }

    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---").ok_or_else(|| {
        format!(
            "Agent file {} missing closing --- for frontmatter",
            source.display()
        )
    })?;

    let yaml_str = &after_first[..end_idx];
    let body_start = end_idx + 4; // skip \n---
    let instructions = after_first[body_start..].trim().to_string();

    let fm: AgentFrontmatter = serde_yaml::from_str(yaml_str)
        .map_err(|e| format!("Invalid YAML in {}: {e}", source.display()))?;

    if fm.name.is_empty() {
        return Err(format!("Agent file {} has empty name", source.display()));
    }

    if !(0.0..=1.0).contains(&fm.budget_fraction) {
        return Err(format!(
            "Agent {} budget_fraction {} out of range [0.0, 1.0]",
            fm.name, fm.budget_fraction
        ));
    }

    Ok(AgentDefinition {
        name: fm.name,
        description: fm.description,
        model: fm.model,
        capabilities: fm.capabilities,
        max_iterations: fm.max_iterations,
        budget_fraction: fm.budget_fraction,
        allowed_tools: fm.allowed_tools,
        instructions,
        source: source.to_path_buf(),
    })
}

/// Load all agent definitions from a directory.
/// Returns a map of name → definition. Later entries override earlier ones.
pub fn load_agents_from_dir(dir: &Path) -> HashMap<String, AgentDefinition> {
    let mut agents = HashMap::new();

    if !dir.exists() || !dir.is_dir() {
        return agents;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("Failed to read agent directory {}: {e}", dir.display());
            return agents;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            match parse_agent_file(&path) {
                Ok(def) => {
                    tracing::debug!(
                        "Loaded agent definition: {} from {}",
                        def.name,
                        path.display()
                    );
                    agents.insert(def.name.clone(), def);
                }
                Err(e) => {
                    tracing::warn!("Skipping invalid agent file: {e}");
                }
            }
        }
    }

    agents
}

/// Load agent definitions with priority merging.
/// Priority: project `.dolphin-milk/agents/` > user `~/.dolphin-milk/agents/` > built-in `agents/`.
pub fn load_all_agents(
    project_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> HashMap<String, AgentDefinition> {
    let mut agents = HashMap::new();

    // Built-in agents (lowest priority)
    let builtin = PathBuf::from("agents");
    if builtin.exists() {
        agents.extend(load_agents_from_dir(&builtin));
    }

    // User agents (~/.dolphin-milk/agents/)
    if let Some(home) = home_dir {
        let user_dir = home.join(".dolphin-milk").join("agents");
        agents.extend(load_agents_from_dir(&user_dir));
    }

    // Project agents (.dolphin-milk/agents/) — highest priority
    if let Some(project) = project_dir {
        let project_dir = project.join(".dolphin-milk").join("agents");
        agents.extend(load_agents_from_dir(&project_dir));
    }

    agents
}

/// Check if an agent's required capabilities are a subset of the parent's.
pub fn agent_available(agent: &AgentDefinition, parent_capabilities: &[String]) -> bool {
    if agent.capabilities.is_empty() {
        return true; // No requirements
    }
    if parent_capabilities.iter().any(|c| c == "all") {
        return true; // Parent has all capabilities
    }
    agent
        .capabilities
        .iter()
        .all(|cap| parent_capabilities.contains(cap))
}
