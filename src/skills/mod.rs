//! Skills system — SKILL.md files with YAML frontmatter customize agent behavior.
//!
//! Skills are markdown files with YAML frontmatter that define instructions,
//! tool references, and activation rules. The `SkillRegistry` loads them from
//! a directory and injects auto-activated skills into the system prompt.
//!
//! Each skill can optionally have a `config.json` in its directory (loaded at
//! startup, read-only) and a persistent data directory at `working/skills/{name}/`
//! for runtime state (logs, cache, results).
//!
//! ## Hooks (issue #84)
//!
//! Skills can register `PreToolUseHook` handlers that fire before tool execution.
//! Hooks are evaluated in priority order and can Allow, Deny, or Modify tool inputs.
//!
//! ## Composition (issue #106)
//!
//! Skills can declare `requires` and `optional` dependencies on other skills.
//! Required dependencies are auto-activated when the parent skill activates.
//! The loader validates that required deps exist and detects dependency cycles.
//!
//! Pattern proven by OpenClaw (54 skills) and Automaton.

pub mod commands;
pub mod composition;
pub mod hooks;
pub mod loader;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use composition::SkillDependencies;
pub use hooks::{HookCallback, HookRegistry, HookResult, PreToolUseHook};

/// Persistent skill-local state — config loaded from `config.json` plus a data
/// directory for runtime files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillState {
    /// Optional JSON config loaded from `skills/{name}/config.json`.
    /// Read-only after load — skills do not write to config.json at runtime.
    pub config: Option<serde_json::Value>,
    /// Path to the skill's persistent data directory (`working/skills/{name}/`).
    /// Created lazily on first `write_data()` call.
    pub data_dir: PathBuf,
}

impl Default for SkillState {
    fn default() -> Self {
        Self {
            config: None,
            data_dir: PathBuf::new(),
        }
    }
}

impl SkillState {
    /// Create a new `SkillState` with the given config and data directory path.
    pub fn new(config: Option<serde_json::Value>, data_dir: PathBuf) -> Self {
        Self { config, data_dir }
    }

    /// Read a file from the skill's data directory.
    /// Returns `None` if the file does not exist or cannot be read.
    pub fn read_data(&self, filename: &str) -> Option<String> {
        let path = self.data_dir.join(filename);
        match std::fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("Failed to read skill data file {}: {e}", path.display());
                }
                None
            }
        }
    }

    /// Write a file to the skill's data directory.
    /// Creates the data directory if it does not exist.
    /// Best-effort — logs errors but does not panic.
    pub fn write_data(&self, filename: &str, content: &str) -> bool {
        if let Err(e) = std::fs::create_dir_all(&self.data_dir) {
            tracing::warn!(
                "Failed to create skill data dir {}: {e}",
                self.data_dir.display()
            );
            return false;
        }
        let path = self.data_dir.join(filename);
        match std::fs::write(&path, content) {
            Ok(()) => {
                tracing::debug!("Wrote skill data file: {}", path.display());
                true
            }
            Err(e) => {
                tracing::warn!("Failed to write skill data file {}: {e}", path.display());
                false
            }
        }
    }
}

/// A loaded skill definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Whether this skill auto-activates (included in every prompt).
    pub auto_activate: bool,
    /// Tool names this skill references.
    pub tools: Vec<String>,
    /// The full markdown instructions (body after frontmatter).
    pub instructions: String,
    /// Source file path.
    pub source: String,
    /// Persistent skill-local state (config + data directory).
    pub state: SkillState,
    /// Dependency declarations — required and optional skills.
    #[serde(default)]
    pub dependencies: SkillDependencies,

    // ── Wave 5: Expanded frontmatter fields ──────────────────────────
    /// Glob patterns for conditional activation (e.g., "*.rs", "src/**/*.ts").
    /// Skill activates when file_read/file_write/file_edit matches a pattern.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Execution context: "normal" (default) or "fork" (isolated sub-agent).
    #[serde(default)]
    pub context: String,
    /// Model override for this skill (e.g., "claude-haiku-4-5").
    #[serde(default)]
    pub model: Option<String>,
    /// Named argument declarations for $ARGUMENTS/$name substitution.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Tools this skill is allowed to use (maps to BRC-52 capabilities).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Prose guidance for LLM proactive invocation.
    #[serde(default)]
    pub when_to_use: Option<String>,
    /// Semantic version string.
    #[serde(default)]
    pub version: Option<String>,
    /// Whether this skill can be invoked directly by the user (e.g., via /skill-name).
    #[serde(default)]
    pub user_invocable: bool,
    /// Hint text for argument autocomplete.
    #[serde(default)]
    pub argument_hint: Option<String>,
}

/// Per-skill activation statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillActivationStats {
    /// Number of times this skill has been activated.
    pub count: u64,
    /// ISO-8601 timestamp of the last activation. `None` if never activated.
    pub last_activated: Option<String>,
    /// Last N activation contexts (e.g. "auto", "explicit"). Capped at 10.
    pub contexts: Vec<String>,
}

/// Maximum number of activation contexts to retain per skill.
const MAX_CONTEXT_ENTRIES: usize = 10;

/// Aggregate telemetry for skill usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillTelemetry {
    /// Per-skill activation stats, keyed by skill name.
    pub activations: HashMap<String, SkillActivationStats>,
}

/// Registry of loaded skills.
#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
    telemetry: SkillTelemetry,
    hook_registry: HookRegistry,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            telemetry: SkillTelemetry::default(),
            hook_registry: HookRegistry::new(),
        }
    }

    pub fn register(&mut self, skill: Skill) {
        self.skills.push(skill);
    }

    /// Get all auto-activated skills.
    pub fn auto_activated(&self) -> Vec<&Skill> {
        self.skills.iter().filter(|s| s.auto_activate).collect()
    }

    /// Get all skills.
    pub fn all(&self) -> &[Skill] {
        &self.skills
    }

    /// Find a skill by name (case-insensitive).
    pub fn find(&self, name: &str) -> Option<&Skill> {
        let lower = name.to_lowercase();
        self.skills.iter().find(|s| s.name.to_lowercase() == lower)
    }

    /// Number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Load all skills from a directory, validating dependencies.
    ///
    /// Logs warnings for missing required dependencies and dependency cycles,
    /// but still loads the skills (graceful degradation).
    pub fn load_from_dir(dir: &std::path::Path) -> Self {
        let mut registry = Self::new();
        if let Ok(skills) = loader::load_skills_from_dir(dir) {
            for skill in skills {
                registry.register(skill);
            }
        }

        // Validate dependencies after all skills are loaded
        registry.validate_dependencies();

        registry
    }

    /// Load additional skills from a directory into this registry.
    ///
    /// Skills loaded here are additive — they supplement existing skills.
    /// If a skill with the same name already exists, the new one is still
    /// registered (no deduplication; `find()` returns the first match).
    pub fn load_additional_from_dir(&mut self, dir: &std::path::Path) {
        if let Ok(skills) = loader::load_skills_from_dir(dir) {
            for skill in skills {
                tracing::info!("Loaded project skill: {} from {}", skill.name, skill.source);
                self.register(skill);
            }
        }
    }

    /// Validate all skill dependencies.
    ///
    /// Checks for missing required deps and dependency cycles. Logs warnings
    /// but does not remove skills — graceful degradation.
    pub fn validate_dependencies(&self) {
        let available: HashSet<String> = self.skills.iter().map(|s| s.name.clone()).collect();

        // Check missing required deps
        for skill in &self.skills {
            let errors =
                composition::validate_required_deps(&skill.name, &skill.dependencies, &available);
            for error in errors {
                tracing::warn!("{}", error);
            }
        }

        // Check for cycles
        let graph = self.build_dependency_graph();
        if let Err(cycle_err) = composition::detect_cycles(&graph) {
            tracing::warn!("{}", cycle_err);
        }
    }

    /// Build the dependency graph (skill_name -> required deps) for cycle detection.
    fn build_dependency_graph(&self) -> HashMap<String, Vec<String>> {
        self.skills
            .iter()
            .map(|s| (s.name.clone(), s.dependencies.requires.clone()))
            .collect()
    }

    /// Activate a skill by name, auto-activating required dependencies first.
    ///
    /// Returns the list of skill names activated (in dependency-first order).
    /// Skills already active (auto_activate=true) are not re-activated but are
    /// still included in the returned list.
    pub fn activate_with_deps(&mut self, skill_name: &str) -> Vec<String> {
        let graph = self.build_dependency_graph();
        let order = composition::activation_order(skill_name, &graph);

        for name in &order {
            self.record_activation(name, "dependency");
        }

        order
    }

    /// Register a PreToolUse hook.
    pub fn register_hook(&mut self, hook: PreToolUseHook, callback: HookCallback) {
        self.hook_registry.register(hook, callback);
    }

    /// Remove all hooks registered by a given skill.
    pub fn remove_hooks_for_skill(&mut self, skill_name: &str) {
        self.hook_registry.remove_by_skill(skill_name);
    }

    /// Evaluate PreToolUse hooks for a tool call.
    ///
    /// Returns `HookResult::Allow` if no hooks match or all allow,
    /// `HookResult::Deny(reason)` if any hook blocks, or
    /// `HookResult::Modify(new_input)` if any hook modifies the input.
    pub fn evaluate_hooks(&self, tool_name: &str, input: &serde_json::Value) -> HookResult {
        self.hook_registry.evaluate(tool_name, input)
    }

    /// Access the hook registry.
    pub fn hooks(&self) -> &HookRegistry {
        &self.hook_registry
    }

    /// Format skills section for system prompt.
    ///
    /// `inbox_count` controls conditional injection: the "messaging" skill is only
    /// included when inbox_count > 0 (i.e., there are unread messages to respond to).
    /// This saves ~29 lines of prompt tokens on most iterations.
    pub fn format_for_prompt(&self, inbox_count: usize) -> String {
        let active: Vec<&Skill> = self
            .auto_activated()
            .into_iter()
            .filter(|s| {
                // Skip messaging skill when no inbox messages
                if s.name.to_lowercase() == "messaging" && inbox_count == 0 {
                    return false;
                }
                true
            })
            .collect();
        if active.is_empty() {
            return String::new();
        }
        let mut lines = vec!["## Active Skills".to_string()];
        for skill in &active {
            lines.push(format!("\n### {}", skill.name));
            if !skill.description.is_empty() {
                lines.push(format!("_{}_", skill.description));
            }
            lines.push(skill.instructions.clone());
        }
        lines.join("\n")
    }

    /// Record a skill activation for telemetry.
    ///
    /// `skill_name` is the name of the skill.
    /// `context` describes how it was activated (e.g. "auto", "explicit").
    pub fn record_activation(&mut self, skill_name: &str, context: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let stats = self
            .telemetry
            .activations
            .entry(skill_name.to_string())
            .or_default();
        stats.count += 1;
        stats.last_activated = Some(now);
        stats.contexts.push(context.to_string());
        if stats.contexts.len() > MAX_CONTEXT_ENTRIES {
            stats.contexts.remove(0);
        }
    }

    /// Return a reference to the telemetry data.
    pub fn telemetry(&self) -> &SkillTelemetry {
        &self.telemetry
    }
}
