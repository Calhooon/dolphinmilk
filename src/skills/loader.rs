//! SKILL.md file loader — parses YAML frontmatter + markdown body.
//!
//! Each SKILL.md file has the format:
//! ```text
//! ---
//! name: skill-name
//! description: What this skill does
//! auto_activate: true
//! tools: [tool_a, tool_b]
//! requires: [other-skill]
//! optional: [nice-to-have]
//! ---
//! # Markdown instructions here
//! ```
//!
//! Skills may also have an optional `config.json` file in the same directory,
//! loaded as `SkillState.config` (arbitrary JSON, read-only after load).

use std::path::Path;

use super::{Skill, SkillDependencies, SkillState};

/// Load all SKILL.md files from a directory (recursive).
pub fn load_skills_from_dir(dir: &Path) -> Result<Vec<Skill>, std::io::Error> {
    let mut skills = Vec::new();
    if !dir.exists() {
        return Ok(skills);
    }

    visit_dir(dir, &mut skills)?;
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

fn visit_dir(dir: &Path, skills: &mut Vec<Skill>) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_dir(&path, skills)?;
        } else if path.file_name().map(|f| f == "SKILL.md").unwrap_or(false) {
            match parse_skill_file(&path) {
                Ok(skill) => {
                    tracing::info!("Loaded skill: {} from {}", skill.name, path.display());
                    skills.push(skill);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse skill file {}: {e}", path.display());
                }
            }
        }
    }
    Ok(())
}

/// Parse a single SKILL.md file, loading config.json from the same directory
/// if present.
pub fn parse_skill_file(path: &Path) -> Result<Skill, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let mut skill = parse_skill_content(&content, path.to_string_lossy().as_ref())?;

    // Try to load config.json from the skill directory (parent of SKILL.md)
    if let Some(skill_dir) = path.parent() {
        let config_path = skill_dir.join("config.json");
        if config_path.exists() {
            match load_config_json(&config_path) {
                Ok(config) => {
                    tracing::debug!(
                        "Loaded config.json for skill '{}' from {}",
                        skill.name,
                        config_path.display()
                    );
                    skill.state.config = Some(config);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse config.json for skill '{}': {e}",
                        skill.name
                    );
                    // config stays None — skill still loads
                }
            }
        }
    }

    Ok(skill)
}

/// Load and parse a config.json file as arbitrary JSON.
fn load_config_json(path: &Path) -> Result<serde_json::Value, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))
}

/// Parse skill content (YAML frontmatter + markdown body).
///
/// Creates a default `SkillState` with no config and a conventional data
/// directory path at `working/skills/{name}/`. Use `parse_skill_file` for
/// filesystem-backed loading that also reads `config.json`.
pub fn parse_skill_content(content: &str, source: &str) -> Result<Skill, String> {
    // Split frontmatter from body
    let (frontmatter, body) = split_frontmatter(content)?;

    // Parse YAML frontmatter
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&frontmatter).map_err(|e| format!("Invalid YAML frontmatter: {e}"))?;

    let name = yaml
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'name' in frontmatter")?
        .to_string();

    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let auto_activate = yaml
        .get("auto_activate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tools = yaml
        .get("tools")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Parse dependency declarations (requires / optional)
    let requires = yaml
        .get("requires")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let optional = yaml
        .get("optional")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let dependencies = SkillDependencies { requires, optional };

    // ── Wave 5: Parse expanded frontmatter fields ──────────────────

    let paths = yaml
        .get("paths")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let context = yaml
        .get("context")
        .and_then(|v| v.as_str())
        .unwrap_or("normal")
        .to_string();

    let model = yaml
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let arguments = yaml
        .get("arguments")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let allowed_tools = yaml
        .get("allowed_tools")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let when_to_use = yaml
        .get("when_to_use")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let version = yaml
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let user_invocable = yaml
        .get("user_invocable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let argument_hint = yaml
        .get("argument_hint")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Default data directory: working/skills/{name}/
    let data_dir = std::path::PathBuf::from("working")
        .join("skills")
        .join(&name);
    let state = SkillState::new(None, data_dir);

    Ok(Skill {
        name,
        description,
        auto_activate,
        tools,
        instructions: body.trim().to_string(),
        source: source.to_string(),
        state,
        dependencies,
        paths,
        context,
        model,
        arguments,
        allowed_tools,
        when_to_use,
        version,
        user_invocable,
        argument_hint,
    })
}

/// Split YAML frontmatter (between --- delimiters) from markdown body.
fn split_frontmatter(content: &str) -> Result<(String, String), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err("No YAML frontmatter found (must start with ---)".to_string());
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let end = after_first
        .find("\n---")
        .ok_or("No closing --- for frontmatter")?;

    let frontmatter = after_first[..end].trim().to_string();
    let body = after_first[end + 4..].to_string(); // skip \n---

    Ok((frontmatter, body))
}

/// Substitute argument placeholders in skill instructions.
///
/// Patterns supported:
/// - `$ARGUMENTS` → full argument string
/// - `$ARGUMENTS[N]` → Nth positional argument (0-indexed)
/// - `$N` → Nth positional argument (0-indexed, shorthand)
/// - `$name` → named argument (if skill declares `arguments: [name, ...]`)
///
/// Arguments are split by whitespace (shell-quote aware via simple splitting).
pub fn substitute_arguments(instructions: &str, args: &str, declared_args: &[String]) -> String {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let mut result = instructions.to_string();

    // $ARGUMENTS[N] → Nth positional (must come BEFORE $ARGUMENTS to avoid clobbering)
    for (i, part) in parts.iter().enumerate() {
        let pattern = format!("$ARGUMENTS[{i}]");
        result = result.replace(&pattern, part);
    }

    // $ARGUMENTS → full argument string
    result = result.replace("$ARGUMENTS", args);

    // $N → Nth positional (0-indexed)
    // Only replace $0, $1, ..., $9 to avoid conflicts with $name
    for (i, part) in parts.iter().enumerate().take(10) {
        let pattern = format!("${i}");
        result = result.replace(&pattern, part);
    }

    // $name → named argument
    for (i, name) in declared_args.iter().enumerate() {
        let pattern = format!("${name}");
        let value = parts.get(i).unwrap_or(&"");
        result = result.replace(&pattern, value);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter_basic() {
        let content = "---\nname: test\n---\n# Body\nContent here.";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert_eq!(fm, "name: test");
        assert!(body.contains("# Body"));
        assert!(body.contains("Content here."));
    }

    #[test]
    fn test_split_frontmatter_no_opening() {
        let content = "# No frontmatter\nJust body.";
        let result = split_frontmatter(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No YAML frontmatter"));
    }

    #[test]
    fn test_split_frontmatter_no_closing() {
        let content = "---\nname: test\nNo closing delimiter.";
        let result = split_frontmatter(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No closing ---"));
    }
}
