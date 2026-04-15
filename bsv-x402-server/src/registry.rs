//! x402 service registry client — dynamic discovery of x402agency.com agents.
//!
//! Fetches `/.well-known/agents` from the registry, caches to disk with a
//! 5-minute TTL, and resolves agent names to base URLs.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::X402Error;

/// Default cache TTL in seconds (5 minutes).
pub const DEFAULT_REGISTRY_CACHE_TTL: u64 = 300;

/// Default registry URL.
pub const DEFAULT_REGISTRY_URL: &str = "https://x402agency.com/.well-known/agents";

/// A single agent entry from the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub tagline: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(flatten)]
    pub extra: Value,
}

/// Registry response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryResponse {
    pub agents: Vec<AgentEntry>,
}

/// Cached registry with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedRegistry {
    timestamp: f64,
    agents: Vec<AgentEntry>,
}

/// Default cache path: `$HOME/.local/share/brc31-sessions/x402-registry.json`.
pub fn default_cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".local/share/brc31-sessions")
        .join("x402-registry.json")
}

/// Load agents from cache file if it exists and is fresh.
fn load_cache(path: &Path, cache_ttl: u64) -> Option<Vec<AgentEntry>> {
    let data = std::fs::read_to_string(path).ok()?;
    let cached: CachedRegistry = serde_json::from_str(&data).ok()?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    if (now - cached.timestamp) > cache_ttl as f64 {
        // Expired — delete stale cache
        let _ = std::fs::remove_file(path);
        return None;
    }

    Some(cached.agents)
}

/// Save agents to cache file (best-effort).
fn save_cache(path: &Path, agents: &[AgentEntry]) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let cached = CachedRegistry {
        timestamp: now,
        agents: agents.to_vec(),
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&cached) {
        let _ = std::fs::write(path, json);
    }
}

/// List all agents from the registry, using cache when fresh.
///
/// Convenience wrapper that uses the default registry URL and cache TTL.
pub async fn list_agents(cache_path: Option<&Path>) -> Result<Vec<AgentEntry>, X402Error> {
    list_agents_from(DEFAULT_REGISTRY_URL, cache_path).await
}

/// List all agents from a specific registry URL, using cache when fresh.
///
/// Uses the default cache TTL. For custom TTL, use [`list_agents_with_ttl`].
pub async fn list_agents_from(
    registry_url: &str,
    cache_path: Option<&Path>,
) -> Result<Vec<AgentEntry>, X402Error> {
    list_agents_with_ttl(registry_url, cache_path, DEFAULT_REGISTRY_CACHE_TTL).await
}

/// List all agents from a specific registry URL with a custom cache TTL.
pub async fn list_agents_with_ttl(
    registry_url: &str,
    cache_path: Option<&Path>,
    cache_ttl: u64,
) -> Result<Vec<AgentEntry>, X402Error> {
    let cache = cache_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_cache_path);

    // Check cache first
    if let Some(agents) = load_cache(&cache, cache_ttl) {
        tracing::debug!("x402 registry: loaded {} agents from cache", agents.len());
        return Ok(agents);
    }

    // Fetch from registry
    tracing::info!("x402 registry: fetching from {registry_url}");
    let client = reqwest::Client::new();
    let resp = client
        .get(registry_url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| X402Error::payment(format!("Failed to fetch x402 registry: {e}")))?;

    if !resp.status().is_success() {
        return Err(X402Error::payment(format!(
            "x402 registry returned HTTP {}",
            resp.status()
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| X402Error::payment(format!("Failed to read registry response: {e}")))?;

    let registry: RegistryResponse = serde_json::from_str(&body)
        .map_err(|e| X402Error::payment(format!("Failed to parse registry JSON: {e}")))?;

    // Save to cache
    save_cache(&cache, &registry.agents);

    tracing::info!("x402 registry: fetched {} agents", registry.agents.len());

    Ok(registry.agents)
}

/// Resolve an agent identifier to a full URL.
///
/// - Full URLs (`https://...`) pass through unchanged.
/// - Bare names (`banana`) resolve to the agent's base URL.
/// - Names with paths (`banana/generate`) resolve to URL + path.
/// - Case-insensitive matching.
pub async fn resolve(identifier: &str, cache_path: Option<&Path>) -> Result<String, X402Error> {
    resolve_from(identifier, DEFAULT_REGISTRY_URL, cache_path).await
}

/// Resolve an agent identifier to its x402-info manifest URL.
///
/// For agents with an `x402_info` field in the registry (e.g. third-party
/// services with hosted manifests), returns that URL directly. Otherwise
/// falls back to `{agent_url}/.well-known/x402-info`.
///
/// Full URLs get `/.well-known/x402-info` appended.
pub async fn resolve_x402_info(
    identifier: &str,
    cache_path: Option<&Path>,
) -> Result<String, X402Error> {
    // Full URLs: append the standard well-known path
    if identifier.starts_with("http://") || identifier.starts_with("https://") {
        let base = identifier.trim_end_matches('/');
        return Ok(format!("{base}/.well-known/x402-info"));
    }

    let name = match identifier.find('/') {
        Some(idx) => &identifier[..idx],
        None => identifier,
    };

    let agents = list_agents(cache_path).await?;
    let name_lower = name.to_lowercase();

    for agent in &agents {
        if agent.name.to_lowercase() == name_lower {
            // Check for hosted manifest URL (third-party services)
            if let Some(info_url) = agent.extra.get("x402_info").and_then(|v| v.as_str()) {
                if !info_url.is_empty() {
                    return Ok(info_url.to_string());
                }
            }
            // Fall back to standard well-known path
            let base = agent.url.trim_end_matches('/');
            return Ok(format!("{base}/.well-known/x402-info"));
        }
    }

    Err(X402Error::payment(format!(
        "Unknown x402 agent: '{name}'. Available: {}",
        agents
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Resolve an agent identifier using a specific registry URL.
pub async fn resolve_from(
    identifier: &str,
    registry_url: &str,
    cache_path: Option<&Path>,
) -> Result<String, X402Error> {
    // Full URLs pass through
    if identifier.starts_with("http://") || identifier.starts_with("https://") {
        return Ok(identifier.to_string());
    }

    // Split name/path
    let (name, path) = match identifier.find('/') {
        Some(idx) => (&identifier[..idx], Some(&identifier[idx..])),
        None => (identifier, None),
    };

    let agents = list_agents_from(registry_url, cache_path).await?;
    let name_lower = name.to_lowercase();

    let agent = agents
        .iter()
        .find(|a| a.name.to_lowercase() == name_lower)
        .ok_or_else(|| {
            X402Error::payment(format!(
                "Unknown x402 agent: '{name}'. Available: {}",
                agents
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    let base_url = agent.url.trim_end_matches('/');
    match path {
        Some(p) => Ok(format!("{base_url}{p}")),
        None => Ok(base_url.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_agents() -> Vec<AgentEntry> {
        vec![
            AgentEntry {
                name: "banana".into(),
                display_name: "Banana Pro".into(),
                url: "https://nano-banana-pro.x402agency.com".into(),
                tagline: "Image generation".into(),
                capabilities: vec!["image".into()],
                extra: Value::Null,
            },
            AgentEntry {
                name: "whisper".into(),
                display_name: "Whisper".into(),
                url: "https://whisper-large-v3-turbo.x402agency.com".into(),
                tagline: "Transcription".into(),
                capabilities: vec!["audio".into()],
                extra: Value::Null,
            },
        ]
    }

    #[test]
    fn test_serde_roundtrip() {
        let agents = sample_agents();
        let json = serde_json::to_string(&agents).unwrap();
        let parsed: Vec<AgentEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "banana");
    }

    #[test]
    fn test_tolerant_deserialization() {
        let json = r#"{"name":"test","unknown_field":"val","url":"https://test.com"}"#;
        let entry: AgentEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.name, "test");
        assert_eq!(entry.url, "https://test.com");
        assert!(entry.display_name.is_empty());
    }

    #[test]
    fn test_cache_save_load() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("registry.json");
        let agents = sample_agents();

        save_cache(&cache_path, &agents);
        let loaded = load_cache(&cache_path, DEFAULT_REGISTRY_CACHE_TTL).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "banana");
    }

    #[test]
    fn test_cache_expiry() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("registry.json");

        // Write a cache with old timestamp
        let cached = CachedRegistry {
            timestamp: 0.0, // epoch — definitely expired
            agents: sample_agents(),
        };
        std::fs::write(&cache_path, serde_json::to_string(&cached).unwrap()).unwrap();

        let loaded = load_cache(&cache_path, DEFAULT_REGISTRY_CACHE_TTL);
        assert!(loaded.is_none());
        // Cache file should be deleted
        assert!(!cache_path.exists());
    }

    #[test]
    fn test_cache_missing() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("nonexistent.json");
        let loaded = load_cache(&cache_path, DEFAULT_REGISTRY_CACHE_TTL);
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_resolve_full_url_passthrough() {
        let url = "https://example.com/api";
        let result = resolve(url, None).await.unwrap();
        assert_eq!(result, url);
    }

    #[tokio::test]
    async fn test_resolve_name_via_cache() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("registry.json");
        save_cache(&cache_path, &sample_agents());

        let result = resolve_from("banana", "http://unused", Some(&cache_path))
            .await
            .unwrap();
        assert_eq!(result, "https://nano-banana-pro.x402agency.com");
    }

    #[tokio::test]
    async fn test_resolve_name_with_path() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("registry.json");
        save_cache(&cache_path, &sample_agents());

        let result = resolve_from("banana/generate", "http://unused", Some(&cache_path))
            .await
            .unwrap();
        assert_eq!(result, "https://nano-banana-pro.x402agency.com/generate");
    }

    #[tokio::test]
    async fn test_resolve_case_insensitive() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("registry.json");
        save_cache(&cache_path, &sample_agents());

        let result = resolve_from("BANANA", "http://unused", Some(&cache_path))
            .await
            .unwrap();
        assert_eq!(result, "https://nano-banana-pro.x402agency.com");
    }

    #[tokio::test]
    async fn test_resolve_unknown_name_error() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("registry.json");
        save_cache(&cache_path, &sample_agents());

        let err = resolve_from("nonexistent", "http://unused", Some(&cache_path))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Unknown x402 agent"));
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn test_agent_entry_empty_capabilities() {
        let json = r#"{"name":"test","capabilities":[]}"#;
        let entry: AgentEntry = serde_json::from_str(json).unwrap();
        assert!(entry.capabilities.is_empty());
    }

    #[test]
    fn test_agent_entry_extra_fields_preserved() {
        let json = r#"{"name":"test","custom_field":"hello","nested":{"a":1}}"#;
        let entry: AgentEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.extra.get("custom_field").unwrap(), "hello");
        assert_eq!(entry.extra.get("nested").unwrap()["a"], 1);
    }
}
