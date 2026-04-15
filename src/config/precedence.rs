//! Multi-source config precedence: env > project > user > instance > defaults.
//!
//! Merge order (highest wins):
//!   1. DOLPHIN_MILK_* environment variables
//!   2. `.dolphin-milk/config.toml` in workspace (project config)
//!   3. `~/.dolphin-milk/config.toml` (user config)
//!   4. `dolphin-milk.toml` in working directory (instance config)
//!   5. Built-in defaults

use std::path::{Path, PathBuf};

use crate::error::DmError;

use super::loader::{apply_env, env_var};
use super::schema::DmConfig;

/// Tracks which configuration sources were loaded, for debugging and display.
#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    /// Config files that were found and successfully loaded (in load order).
    pub files_loaded: Vec<PathBuf>,
    /// DOLPHIN_MILK_* env vars that were applied as overrides.
    pub env_overrides: Vec<String>,
}

/// Load config by merging multiple sources in precedence order.
///
/// Merge order (highest priority wins):
///   1. DOLPHIN_MILK_* environment variables
///   2. Project config: `{workspace}/.dolphin-milk/config.toml`
///   3. User config: `~/.dolphin-milk/config.toml`
///   4. Instance config: `dolphin-milk.toml` (or explicit path)
///   5. Built-in defaults
///
/// `workspace` is the project root directory. If `None`, project config is skipped.
/// `instance_path` overrides the default instance config location.
pub fn load_config_with_precedence(
    workspace: Option<&Path>,
    instance_path: Option<&Path>,
) -> Result<(DmConfig, ConfigSources), DmError> {
    let mut sources = ConfigSources::default();

    // We merge TOML layers at the toml::Value level, then deserialize once.
    // Start with an empty table (defaults come from serde(default) on DmConfig).
    let mut merged = toml::Value::Table(toml::map::Map::new());

    // Layer 1: Instance config (lowest file priority)
    let inst_path = resolve_instance_path(instance_path);
    if inst_path.exists() {
        let layer = load_toml_value(&inst_path)?;
        deep_merge_toml(&mut merged, &layer);
        sources.files_loaded.push(inst_path);
    }

    // Layer 2: User config (~/.dolphin-milk/config.toml)
    let user_path = resolve_user_config_path();
    if let Some(ref p) = user_path {
        if p.exists() {
            let layer = load_toml_value(p)?;
            deep_merge_toml(&mut merged, &layer);
            sources.files_loaded.push(p.clone());
        }
    }

    // Layer 3: Project config ({workspace}/.dolphin-milk/config.toml)
    if let Some(ws) = workspace {
        let project_path = ws.join(".dolphin-milk").join("config.toml");
        if project_path.exists() {
            let layer = load_toml_value(&project_path)?;
            deep_merge_toml(&mut merged, &layer);
            sources.files_loaded.push(project_path);
        }
    }

    // Deserialize the merged TOML value into DmConfig.
    // serde(default) on DmConfig fills in any missing fields.
    let mut config: DmConfig = merged.try_into().map_err(|e: toml::de::Error| {
        DmError::config(format!("Failed to deserialize merged config: {e}"))
    })?;

    // Layer 4: Environment variables (highest priority)
    sources.env_overrides = collect_env_overrides();
    apply_env(&mut config)?;

    Ok((config, sources))
}

// ---------------------------------------------------------------------------
// Path resolution helpers
// ---------------------------------------------------------------------------

/// Resolve the instance config path. If an explicit path is given, use it.
/// Otherwise, check `$DOLPHIN_MILK_DATA_DIR/dolphin-milk.toml`, then `./dolphin-milk.toml`.
fn resolve_instance_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(data_dir) = env_var("DATA_DIR") {
        let candidate = Path::new(&data_dir).join("dolphin-milk.toml");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("dolphin-milk.toml")
}

/// Resolve the user config path: `~/.dolphin-milk/config.toml`.
/// Returns `None` if `HOME` is not set.
fn resolve_user_config_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|home| Path::new(&home).join(".dolphin-milk").join("config.toml"))
}

// ---------------------------------------------------------------------------
// TOML loading and merging
// ---------------------------------------------------------------------------

/// Load and parse a single TOML file into a `toml::Value`.
fn load_toml_value(path: &Path) -> Result<toml::Value, DmError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| DmError::config(format!("Failed to read {}: {e}", path.display())))?;
    contents
        .parse::<toml::Value>()
        .map_err(|e| DmError::config(format!("Failed to parse {}: {e}", path.display())))
}

/// Deep-merge `overlay` into `base` at the `toml::Value` level.
///
/// - Tables: merge recursively (field-by-field, overlay wins)
/// - Arrays: concatenate and deduplicate
/// - Scalars: overlay replaces base
fn deep_merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_val) in overlay_table {
                if let Some(base_val) = base_table.get_mut(key) {
                    match (base_val.clone(), overlay_val) {
                        (toml::Value::Table(_), toml::Value::Table(_)) => {
                            deep_merge_toml(base_val, overlay_val);
                        }
                        (toml::Value::Array(ref mut base_arr), toml::Value::Array(overlay_arr)) => {
                            // Concatenate and deduplicate
                            for item in overlay_arr {
                                if !base_arr.contains(item) {
                                    base_arr.push(item.clone());
                                }
                            }
                            *base_val = toml::Value::Array(base_arr.clone());
                        }
                        (_, _) => {
                            // Scalar: overlay wins
                            *base_val = overlay_val.clone();
                        }
                    }
                } else {
                    base_table.insert(key.clone(), overlay_val.clone());
                }
            }
        }
        (base, overlay) => {
            // Non-table overlay replaces base entirely
            *base = overlay.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Env var collection
// ---------------------------------------------------------------------------

/// Scan for DOLPHIN_MILK_* environment variables that are currently set.
fn collect_env_overrides() -> Vec<String> {
    let mut overrides = Vec::new();
    for (key, _) in std::env::vars() {
        if key.starts_with("DOLPHIN_MILK_") {
            overrides.push(key);
        }
    }
    overrides.sort();
    overrides
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_env_overrides_returns_sorted() {
        let overrides = collect_env_overrides();
        let mut sorted = overrides.clone();
        sorted.sort();
        assert_eq!(overrides, sorted);
    }

    #[test]
    fn test_resolve_user_config_path_under_home() {
        let path = resolve_user_config_path();
        if std::env::var("HOME").is_ok() {
            let p = path.unwrap();
            assert!(p.ends_with(".dolphin-milk/config.toml"));
        }
    }

    #[test]
    fn test_deep_merge_toml_scalar_override() {
        let mut base: toml::Value = toml::from_str("[wallet]\nurl = \"http://base:3322\"").unwrap();
        let overlay: toml::Value =
            toml::from_str("[wallet]\nurl = \"http://overlay:9999\"").unwrap();

        deep_merge_toml(&mut base, &overlay);

        let table = base.as_table().unwrap();
        let wallet = table["wallet"].as_table().unwrap();
        assert_eq!(wallet["url"].as_str().unwrap(), "http://overlay:9999");
    }

    #[test]
    fn test_deep_merge_toml_preserves_base_for_missing_overlay_fields() {
        let mut base: toml::Value =
            toml::from_str("[wallet]\nurl = \"http://base:3322\"\ntimeout = 60").unwrap();
        let overlay: toml::Value = toml::from_str("[wallet]\ntimeout = 90").unwrap();

        deep_merge_toml(&mut base, &overlay);

        let table = base.as_table().unwrap();
        let wallet = table["wallet"].as_table().unwrap();
        // base's url should be preserved since overlay doesn't set it
        assert_eq!(wallet["url"].as_str().unwrap(), "http://base:3322");
        // overlay's timeout should win
        assert_eq!(wallet["timeout"].as_integer().unwrap(), 90);
    }

    #[test]
    fn test_deep_merge_toml_arrays_concatenate_and_dedup() {
        let mut base: toml::Value =
            toml::from_str("[tool_approval]\nrequire = [\"tool_a\", \"tool_b\"]").unwrap();
        let overlay: toml::Value =
            toml::from_str("[tool_approval]\nrequire = [\"tool_b\", \"tool_c\"]").unwrap();

        deep_merge_toml(&mut base, &overlay);

        let arr = base["tool_approval"]["require"].as_array().unwrap();
        let items: Vec<&str> = arr.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(items.contains(&"tool_a"));
        assert!(items.contains(&"tool_b"));
        assert!(items.contains(&"tool_c"));
        assert_eq!(items.iter().filter(|&&t| t == "tool_b").count(), 1);
    }

    #[test]
    fn test_deep_merge_toml_new_section_added() {
        let mut base: toml::Value = toml::from_str("[wallet]\nurl = \"http://base:3322\"").unwrap();
        let overlay: toml::Value = toml::from_str("[budget]\nmax_per_task = 100").unwrap();

        deep_merge_toml(&mut base, &overlay);

        let table = base.as_table().unwrap();
        // Both sections should be present
        assert!(table.contains_key("wallet"));
        assert!(table.contains_key("budget"));
        assert_eq!(table["budget"]["max_per_task"].as_integer().unwrap(), 100);
    }
}
