//! Agent template schema — defines the TOML structure for pre-configured agent templates.

use serde::Deserialize;

/// A pre-configured agent template that provides sensible defaults for a specific use case.
///
/// Templates are TOML files loaded from `templates/` at the workspace root.
/// They override select fields in `DmConfig` without replacing the entire config.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentTemplate {
    /// Template metadata.
    pub template: TemplateMetadata,
    /// Personality configuration (system prompt, style).
    #[serde(default)]
    pub personality: PersonalityConfig,
    /// Active skills configuration.
    #[serde(default)]
    pub skills: SkillsConfig,
    /// Tool enable/disable configuration.
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Budget limits for this template.
    #[serde(default)]
    pub budget: TemplateBudgetConfig,
    /// LLM and operational defaults.
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

/// Template metadata — name, description, version, author.
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateMetadata {
    /// Unique template name (e.g., "researcher", "coder").
    pub name: String,
    /// Human-readable description of what this template is for.
    pub description: String,
    /// Semantic version string.
    #[serde(default = "default_version")]
    pub version: String,
    /// Template author.
    #[serde(default = "default_author")]
    pub author: String,
}

fn default_version() -> String {
    "1.0".into()
}

fn default_author() -> String {
    "dolphin-milk".into()
}

/// Personality configuration — system prompt and communication style.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PersonalityConfig {
    /// System prompt prepended to the agent's context.
    pub system_prompt: String,
    /// Communication style hint (e.g., "analytical", "concise", "friendly").
    pub style: String,
}

/// Skills to activate for this template.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SkillsConfig {
    /// Skill names to activate (must match names in `skills/` directory).
    pub active: Vec<String>,
}

/// Tool enable/disable configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ToolsConfig {
    /// Tool names to explicitly enable.
    pub enabled: Vec<String>,
    /// Tool names to explicitly disable (takes precedence over enabled).
    pub disabled: Vec<String>,
}

/// Budget configuration specific to this template.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TemplateBudgetConfig {
    /// Maximum satoshis per day. 0 = use default.
    pub daily_limit_sats: u64,
    /// Maximum satoshis per task. 0 = use default.
    pub per_task_limit_sats: u64,
    /// Budget warning threshold as a fraction (0.0 - 1.0). 0 = use default.
    pub warning_threshold: f64,
}

/// LLM and operational defaults.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct DefaultsConfig {
    /// LLM model name (e.g., "gpt-5", "claude-sonnet-4-6").
    pub model: String,
    /// Sampling temperature. 0 = use default.
    pub temperature: f64,
    /// Max tokens per response. 0 = use default.
    pub max_tokens: u32,
}

/// Errors that can occur during template loading and validation.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    /// Failed to read the template file.
    #[error("failed to read template file: {0}")]
    Io(#[from] std::io::Error),
    /// Failed to parse the TOML content.
    #[error("failed to parse template TOML: {0}")]
    Parse(#[from] toml::de::Error),
    /// Template validation failed.
    #[error("template validation failed: {0}")]
    Validation(String),
}

/// A single validation issue found in a template.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Which field has the issue.
    pub field: String,
    /// Description of the problem.
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl AgentTemplate {
    /// Load a template from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self, TemplateError> {
        let contents = std::fs::read_to_string(path)?;
        let template: AgentTemplate = toml::from_str(&contents)?;
        Ok(template)
    }

    /// Validate the template and return any issues found.
    ///
    /// Returns `Ok(())` if valid, or `Err` with a list of validation errors.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.template.name.is_empty() {
            errors.push(ValidationError {
                field: "template.name".into(),
                message: "name is required".into(),
            });
        }

        if self.template.description.is_empty() {
            errors.push(ValidationError {
                field: "template.description".into(),
                message: "description is required".into(),
            });
        }

        if self.budget.warning_threshold < 0.0 || self.budget.warning_threshold > 1.0 {
            errors.push(ValidationError {
                field: "budget.warning_threshold".into(),
                message: "must be between 0.0 and 1.0".into(),
            });
        }

        if self.defaults.temperature < 0.0 || self.defaults.temperature > 2.0 {
            errors.push(ValidationError {
                field: "defaults.temperature".into(),
                message: "must be between 0.0 and 2.0".into(),
            });
        }

        // Check for tools listed in both enabled and disabled
        for tool in &self.tools.enabled {
            if self.tools.disabled.contains(tool) {
                errors.push(ValidationError {
                    field: "tools".into(),
                    message: format!("'{}' is in both enabled and disabled lists", tool),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Apply this template's settings to a `DmConfig`.
    ///
    /// Only overrides fields that the template explicitly sets (non-zero, non-empty).
    /// The original config is used as the base.
    pub fn apply_to_config(&self, config: &mut crate::config::DmConfig) {
        // Budget overrides
        if self.budget.daily_limit_sats > 0 {
            config.budget.max_per_day = self.budget.daily_limit_sats;
        }
        if self.budget.per_task_limit_sats > 0 {
            config.budget.max_per_task = self.budget.per_task_limit_sats;
        }

        // LLM defaults
        if !self.defaults.model.is_empty() {
            config.llm.default_model = self.defaults.model.clone();
        }
        if self.defaults.max_tokens > 0 {
            config.llm.max_tokens = self.defaults.max_tokens;
        }
    }

    /// Return the template name.
    pub fn name(&self) -> &str {
        &self.template.name
    }

    /// Return the template description.
    pub fn description(&self) -> &str {
        &self.template.description
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_metadata_defaults() {
        let toml_str = r#"
[template]
name = "test"
description = "A test template"
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        assert_eq!(template.template.name, "test");
        assert_eq!(template.template.version, "1.0");
        assert_eq!(template.template.author, "dolphin-milk");
    }

    #[test]
    fn test_empty_name_fails_validation() {
        let toml_str = r#"
[template]
name = ""
description = "A test template"
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let result = template.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.field == "template.name"));
    }

    #[test]
    fn test_empty_description_fails_validation() {
        let toml_str = r#"
[template]
name = "test"
description = ""
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let result = template.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.field == "template.description"));
    }

    #[test]
    fn test_conflicting_tools_fails_validation() {
        let toml_str = r#"
[template]
name = "test"
description = "A test"

[tools]
enabled = ["browser", "wallet_balance"]
disabled = ["browser"]
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let result = template.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.field == "tools"));
    }

    #[test]
    fn test_invalid_warning_threshold() {
        let toml_str = r#"
[template]
name = "test"
description = "A test"

[budget]
warning_threshold = 1.5
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let result = template.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_temperature() {
        let toml_str = r#"
[template]
name = "test"
description = "A test"

[defaults]
temperature = 3.0
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let result = template.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_template_passes_validation() {
        let toml_str = r#"
[template]
name = "researcher"
description = "Deep research agent"
version = "1.0"

[personality]
system_prompt = "You are a thorough researcher."
style = "analytical"

[skills]
active = ["browser", "x402"]

[tools]
enabled = ["browser", "web_fetch", "memory_store"]
disabled = []

[budget]
daily_limit_sats = 50000
per_task_limit_sats = 10000
warning_threshold = 0.8

[defaults]
model = "gpt-5"
temperature = 0.3
max_tokens = 4096
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        assert!(template.validate().is_ok());
        assert_eq!(template.name(), "researcher");
    }

    #[test]
    fn test_apply_to_config() {
        let toml_str = r#"
[template]
name = "test"
description = "A test"

[budget]
daily_limit_sats = 100000
per_task_limit_sats = 25000

[defaults]
model = "claude-sonnet-4-6"
max_tokens = 8192
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let mut config = crate::config::DmConfig::default();
        template.apply_to_config(&mut config);

        assert_eq!(config.budget.max_per_day, 100000);
        assert_eq!(config.budget.max_per_task, 25000);
        assert_eq!(config.llm.default_model, "claude-sonnet-4-6");
        assert_eq!(config.llm.max_tokens, 8192);
    }

    #[test]
    fn test_apply_does_not_override_zero_values() {
        let toml_str = r#"
[template]
name = "test"
description = "A test"
"#;
        let template: AgentTemplate = toml::from_str(toml_str).unwrap();
        let mut config = crate::config::DmConfig::default();
        let original_budget = config.budget.max_per_day;
        let original_model = config.llm.default_model.clone();
        template.apply_to_config(&mut config);

        // Unchanged because template values are 0/empty
        assert_eq!(config.budget.max_per_day, original_budget);
        assert_eq!(config.llm.default_model, original_model);
    }
}
