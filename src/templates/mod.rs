//! Agent templates — pre-configured agent archetypes for common use cases.
//!
//! Templates are TOML files that define personality, skills, tools, budget,
//! and LLM defaults for a specific agent role (e.g., researcher, coder, trader).
//!
//! Templates are **optional** and **additive** — they override select fields
//! in `DmConfig` without replacing the entire configuration. An agent works
//! fine without any template applied.
//!
//! # Usage
//!
//! ```ignore
//! use dolphin_milk::templates::{AgentTemplate, load_builtin_templates};
//!
//! // Load a single template
//! let template = AgentTemplate::load(Path::new("templates/researcher.toml"))?;
//! template.validate()?;
//! template.apply_to_config(&mut config);
//!
//! // Load all built-in templates
//! let templates = load_builtin_templates(Path::new("templates"))?;
//! ```

pub mod schema;

pub use schema::{
    AgentTemplate, DefaultsConfig, PersonalityConfig, SkillsConfig, TemplateBudgetConfig,
    TemplateError, TemplateMetadata, ToolsConfig, ValidationError,
};

use std::path::Path;

/// Load all `.toml` template files from a directory.
///
/// Returns a vector of successfully parsed templates. Files that fail to parse
/// are logged as warnings and skipped.
pub fn load_builtin_templates(dir: &Path) -> Result<Vec<AgentTemplate>, TemplateError> {
    let mut templates = Vec::new();

    if !dir.exists() {
        tracing::debug!("Templates directory does not exist: {}", dir.display());
        return Ok(templates);
    }

    let entries = std::fs::read_dir(dir).map_err(TemplateError::Io)?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }

        match AgentTemplate::load(&path) {
            Ok(template) => {
                tracing::debug!(
                    "Loaded template: {} from {}",
                    template.name(),
                    path.display()
                );
                templates.push(template);
            }
            Err(e) => {
                tracing::warn!("Failed to load template {}: {e}", path.display());
            }
        }
    }

    // Sort by name for deterministic ordering
    templates.sort_by(|a, b| a.name().cmp(b.name()));

    Ok(templates)
}

/// Find a template by name from a list of loaded templates.
pub fn find_template<'a>(templates: &'a [AgentTemplate], name: &str) -> Option<&'a AgentTemplate> {
    let lower = name.to_lowercase();
    templates.iter().find(|t| t.name().to_lowercase() == lower)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_load_from_nonexistent_dir() {
        let result = load_builtin_templates(Path::new("/nonexistent/path"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_load_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_builtin_templates(dir.path());
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_load_valid_template_file() {
        let dir = tempfile::tempdir().unwrap();
        let toml_content = r#"
[template]
name = "test-agent"
description = "A test agent template"

[personality]
system_prompt = "You are a test agent."
style = "concise"

[skills]
active = ["browser"]

[tools]
enabled = ["web_fetch"]

[budget]
daily_limit_sats = 10000

[defaults]
model = "gpt-5"
temperature = 0.5
max_tokens = 2048
"#;
        fs::write(dir.path().join("test.toml"), toml_content).unwrap();

        let templates = load_builtin_templates(dir.path()).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name(), "test-agent");
    }

    #[test]
    fn test_load_skips_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();

        // Valid template
        let valid = r#"
[template]
name = "valid"
description = "Valid template"
"#;
        fs::write(dir.path().join("valid.toml"), valid).unwrap();

        // Invalid TOML syntax
        fs::write(
            dir.path().join("broken.toml"),
            "this is not valid toml {{{}",
        )
        .unwrap();

        // Non-TOML file (should be skipped)
        fs::write(dir.path().join("readme.txt"), "ignore me").unwrap();

        let templates = load_builtin_templates(dir.path()).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name(), "valid");
    }

    #[test]
    fn test_load_multiple_templates_sorted() {
        let dir = tempfile::tempdir().unwrap();

        for name in ["zebra", "alpha", "middle"] {
            let content = format!(
                r#"
[template]
name = "{name}"
description = "Template {name}"
"#
            );
            fs::write(dir.path().join(format!("{name}.toml")), content).unwrap();
        }

        let templates = load_builtin_templates(dir.path()).unwrap();
        assert_eq!(templates.len(), 3);
        assert_eq!(templates[0].name(), "alpha");
        assert_eq!(templates[1].name(), "middle");
        assert_eq!(templates[2].name(), "zebra");
    }

    #[test]
    fn test_find_template_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let content = r#"
[template]
name = "Researcher"
description = "Research agent"
"#;
        fs::write(dir.path().join("researcher.toml"), content).unwrap();

        let templates = load_builtin_templates(dir.path()).unwrap();
        assert!(find_template(&templates, "researcher").is_some());
        assert!(find_template(&templates, "RESEARCHER").is_some());
        assert!(find_template(&templates, "nonexistent").is_none());
    }

    #[test]
    fn test_all_builtin_templates_parse_and_validate() {
        // This test loads the actual templates/ directory from the workspace root
        let templates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        if !templates_dir.exists() {
            // Skip if templates dir doesn't exist yet (during initial development)
            return;
        }

        let templates = load_builtin_templates(&templates_dir).unwrap();
        assert!(
            !templates.is_empty(),
            "Expected at least one template in {}",
            templates_dir.display()
        );

        for template in &templates {
            let result = template.validate();
            assert!(
                result.is_ok(),
                "Template '{}' failed validation: {:?}",
                template.name(),
                result.unwrap_err()
            );
        }
    }
}
