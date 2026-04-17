//! File-based memory store — markdown files with YAML frontmatter.
//!
//! Memories are stored as `.md` files in category subdirectories:
//!   - `knowledge/` — persistent facts, patterns, insights
//!   - `sessions/`  — session summaries
//!   - `execution/` — tool output caches, action logs

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::DmError;

/// Reserved tag for identity entries stored in the Knowledge category.
/// Identity entries use `MemoryCategory::Knowledge` with this tag rather than
/// a separate enum variant (to avoid breaking serialization of existing entries).
pub const IDENTITY_TAG: &str = "identity";

/// Category of a memory entry, determining its storage directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryCategory {
    /// Persistent facts, patterns, insights.
    Knowledge,
    /// Session summaries.
    Session,
    /// Tool output caches, action logs.
    Execution,
}

impl fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryCategory::Knowledge => write!(f, "knowledge"),
            MemoryCategory::Session => write!(f, "session"),
            MemoryCategory::Execution => write!(f, "execution"),
        }
    }
}

impl FromStr for MemoryCategory {
    type Err = DmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "knowledge" => Ok(MemoryCategory::Knowledge),
            "session" | "sessions" => Ok(MemoryCategory::Session),
            "execution" => Ok(MemoryCategory::Execution),
            other => Err(DmError::memory(format!("unknown memory category: {other}"))),
        }
    }
}

/// YAML frontmatter for serialization/deserialization.
#[derive(Debug, Serialize, Deserialize)]
struct Frontmatter {
    id: String,
    category: MemoryCategory,
    tags: Vec<String>,
    created: String,
    source: String,
}

/// A single memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique identifier (UUID).
    pub id: String,
    /// Category determines storage directory.
    pub category: MemoryCategory,
    /// The actual memory content.
    pub content: String,
    /// Searchable tags.
    pub tags: Vec<String>,
    /// Timestamp of creation.
    pub created: DateTime<Utc>,
    /// Where this memory came from (e.g. "session-2026-02-23").
    pub source: String,
}

impl MemoryEntry {
    /// Create a new memory entry with a fresh UUID and current timestamp.
    pub fn new(
        category: MemoryCategory,
        content: impl Into<String>,
        tags: Vec<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            category,
            content: content.into(),
            tags,
            created: Utc::now(),
            source: source.into(),
        }
    }

    /// Serialize entry to markdown with YAML frontmatter.
    fn to_markdown(&self) -> String {
        let fm = Frontmatter {
            id: self.id.clone(),
            category: self.category.clone(),
            tags: self.tags.clone(),
            created: self.created.to_rfc3339(),
            source: self.source.clone(),
        };
        let yaml = serde_yaml::to_string(&fm).unwrap_or_default();
        format!("---\n{}---\n\n{}\n", yaml, self.content)
    }

    /// Parse a memory entry from markdown with YAML frontmatter.
    fn from_markdown(text: &str) -> Result<Self, DmError> {
        // Split on frontmatter delimiters
        let text = text.trim_start_matches('\u{feff}'); // strip BOM
        if !text.starts_with("---") {
            return Err(DmError::memory(
                "memory file missing YAML frontmatter delimiter",
            ));
        }

        // Find the closing ---
        let after_first = &text[3..];
        let end_idx = after_first.find("\n---").ok_or_else(|| {
            DmError::memory("memory file missing closing YAML frontmatter delimiter")
        })?;

        let yaml_str = &after_first[..end_idx];
        let content_start = 3 + end_idx + 4; // skip past "\n---"
        let content = if content_start < text.len() {
            text[content_start..].trim().to_string()
        } else {
            String::new()
        };

        let fm: Frontmatter = serde_yaml::from_str(yaml_str)
            .map_err(|e| DmError::memory(format!("failed to parse YAML frontmatter: {e}")))?;

        let created = DateTime::parse_from_rfc3339(&fm.created)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(MemoryEntry {
            id: fm.id,
            category: fm.category,
            content,
            tags: fm.tags,
            created,
            source: fm.source,
        })
    }
}

/// File-based memory store.
///
/// Memories are organized into category subdirectories under `base_dir`:
/// ```text
/// base_dir/
///   knowledge/
///   sessions/
///   execution/
/// ```
#[derive(Debug, Clone)]
pub struct MemoryStore {
    base_dir: PathBuf,
}

impl MemoryStore {
    /// Create a new memory store, ensuring category directories exist.
    pub fn new(base_dir: PathBuf) -> Self {
        let store = Self { base_dir };
        // Create category directories if they don't exist
        for cat in &[
            MemoryCategory::Knowledge,
            MemoryCategory::Session,
            MemoryCategory::Execution,
        ] {
            let dir = store.category_dir(cat);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!("Failed to create memory dir {}: {e}", dir.display());
            }
        }
        tracing::debug!("Memory store initialized at {}", store.base_dir.display());
        store
    }

    /// Get the directory path for a given category.
    pub fn category_dir(&self, category: &MemoryCategory) -> PathBuf {
        match category {
            MemoryCategory::Knowledge => self.base_dir.join("knowledge"),
            MemoryCategory::Session => self.base_dir.join("sessions"),
            MemoryCategory::Execution => self.base_dir.join("execution"),
        }
    }

    /// Return the base directory of this store.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Write a memory entry to disk as a markdown file with YAML frontmatter.
    ///
    /// Returns the path of the written file.
    pub fn store(&self, entry: &MemoryEntry) -> Result<PathBuf, DmError> {
        let dir = self.category_dir(&entry.category);
        std::fs::create_dir_all(&dir).map_err(|e| {
            DmError::memory(format!(
                "failed to create category dir {}: {e}",
                dir.display()
            ))
        })?;

        let path = dir.join(format!("{}.md", entry.id));
        let markdown = entry.to_markdown();

        std::fs::write(&path, &markdown).map_err(|e| {
            DmError::memory(format!(
                "failed to write memory file {}: {e}",
                path.display()
            ))
        })?;

        tracing::debug!(
            "Stored memory {} ({}) at {}",
            entry.id,
            entry.category,
            path.display()
        );
        Ok(path)
    }

    /// Read and parse a memory file by ID.
    ///
    /// Searches all category directories for the given ID.
    pub fn read(&self, id: &str) -> Result<MemoryEntry, DmError> {
        let filename = format!("{id}.md");

        for cat in &[
            MemoryCategory::Knowledge,
            MemoryCategory::Session,
            MemoryCategory::Execution,
        ] {
            let path = self.category_dir(cat).join(&filename);
            if path.exists() {
                let text = std::fs::read_to_string(&path).map_err(|e| {
                    DmError::memory(format!(
                        "failed to read memory file {}: {e}",
                        path.display()
                    ))
                })?;
                return MemoryEntry::from_markdown(&text);
            }
        }

        Err(DmError::memory(format!("memory not found: {id}")))
    }

    /// List all entries, optionally filtered by category.
    pub fn list(&self, category: Option<MemoryCategory>) -> Result<Vec<MemoryEntry>, DmError> {
        let categories = match category {
            Some(cat) => vec![cat],
            None => vec![
                MemoryCategory::Knowledge,
                MemoryCategory::Session,
                MemoryCategory::Execution,
            ],
        };

        let mut entries = Vec::new();

        for cat in categories {
            let dir = self.category_dir(&cat);
            if !dir.exists() {
                continue;
            }

            let read_dir = std::fs::read_dir(&dir).map_err(|e| {
                DmError::memory(format!("failed to read memory dir {}: {e}", dir.display()))
            })?;

            for dir_entry in read_dir {
                let dir_entry = match dir_entry {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("Error reading dir entry: {e}");
                        continue;
                    }
                };

                let path = dir_entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                match std::fs::read_to_string(&path) {
                    Ok(text) => match MemoryEntry::from_markdown(&text) {
                        Ok(entry) => entries.push(entry),
                        Err(e) => {
                            tracing::warn!("Failed to parse memory file {}: {e}", path.display());
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Failed to read memory file {}: {e}", path.display());
                    }
                }
            }
        }

        // Sort by creation time (newest first)
        entries.sort_by_key(|x| std::cmp::Reverse(x.created));
        Ok(entries)
    }

    /// Find the most recent identity entry in the Knowledge category.
    ///
    /// Identity entries are regular Knowledge entries tagged with [`IDENTITY_TAG`].
    /// Returns the newest matching entry, or `None` if no identity entry exists.
    pub fn find_identity_entry(&self) -> Option<MemoryEntry> {
        let entries = self.list(Some(MemoryCategory::Knowledge)).ok()?;
        // list() returns newest-first, so the first match is the most recent
        entries
            .into_iter()
            .find(|e| e.tags.contains(&IDENTITY_TAG.to_string()))
    }

    /// Write a `.proof` sidecar JSON file for a memory entry.
    ///
    /// The sidecar is stored alongside the `.md` file in the same category directory
    /// and contains the proof txid plus the content hash for fast tamper verification
    /// during recall (without hitting the blockchain).
    pub fn write_proof_sidecar(
        &self,
        entry: &MemoryEntry,
        txid: &str,
        content_hash: &str,
    ) -> Result<PathBuf, DmError> {
        let dir = self.category_dir(&entry.category);
        let path = dir.join(format!("{}.proof", entry.id));

        let sidecar = serde_json::json!({
            "txid": txid,
            "content_hash": content_hash,
        });

        let json = serde_json::to_string_pretty(&sidecar)
            .map_err(|e| DmError::memory(format!("failed to serialize proof sidecar: {e}")))?;

        std::fs::write(&path, json).map_err(|e| {
            DmError::memory(format!(
                "failed to write proof sidecar {}: {e}",
                path.display()
            ))
        })?;

        tracing::debug!(
            "Wrote proof sidecar for memory {} at {}",
            entry.id,
            path.display()
        );
        Ok(path)
    }

    /// Read a `.proof` sidecar JSON file for a memory entry, if one exists.
    ///
    /// Returns `Some((txid, content_hash))` if the sidecar file exists and is valid,
    /// or `None` if no sidecar is found (backward compatible for pre-existing entries).
    pub fn read_proof_sidecar(
        &self,
        id: &str,
        category: &MemoryCategory,
    ) -> Option<(String, String)> {
        let dir = self.category_dir(category);
        let path = dir.join(format!("{id}.proof"));

        if !path.exists() {
            return None;
        }

        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to read proof sidecar {}: {e}", path.display());
                return None;
            }
        };

        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse proof sidecar {}: {e}", path.display());
                return None;
            }
        };

        let txid = parsed.get("txid")?.as_str()?.to_string();
        let content_hash = parsed.get("content_hash")?.as_str()?.to_string();

        Some((txid, content_hash))
    }

    /// Delete a memory file by ID.
    pub fn delete(&self, id: &str) -> Result<(), DmError> {
        let filename = format!("{id}.md");

        for cat in &[
            MemoryCategory::Knowledge,
            MemoryCategory::Session,
            MemoryCategory::Execution,
        ] {
            let path = self.category_dir(cat).join(&filename);
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| {
                    DmError::memory(format!(
                        "failed to delete memory file {}: {e}",
                        path.display()
                    ))
                })?;
                tracing::debug!("Deleted memory {} from {}", id, path.display());
                return Ok(());
            }
        }

        Err(DmError::memory(format!("memory not found: {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_display_and_parse() {
        assert_eq!(MemoryCategory::Knowledge.to_string(), "knowledge");
        assert_eq!(MemoryCategory::Session.to_string(), "session");
        assert_eq!(MemoryCategory::Execution.to_string(), "execution");

        assert_eq!(
            "knowledge".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Knowledge
        );
        assert_eq!(
            "session".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Session
        );
        assert_eq!(
            "sessions".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Session
        );
        assert_eq!(
            "execution".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Execution
        );
        assert!("invalid".parse::<MemoryCategory>().is_err());
    }

    #[test]
    fn test_entry_markdown_roundtrip() {
        let entry = MemoryEntry {
            id: "test-id-001".to_string(),
            category: MemoryCategory::Knowledge,
            content: "The x402 payment protocol uses a 3-message handshake.".to_string(),
            tags: vec!["x402".to_string(), "payment".to_string()],
            created: DateTime::parse_from_rfc3339("2026-02-23T14:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            source: "session-2026-02-23-01".to_string(),
        };

        let md = entry.to_markdown();
        assert!(md.starts_with("---\n"));
        assert!(md.contains("id: test-id-001"));
        assert!(md.contains("x402"));
        assert!(md.contains("The x402 payment protocol"));

        let parsed = MemoryEntry::from_markdown(&md).unwrap();
        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.category, entry.category);
        assert_eq!(parsed.content, entry.content);
        assert_eq!(parsed.tags, entry.tags);
        assert_eq!(parsed.source, entry.source);
    }

    #[test]
    fn test_store_read_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().to_path_buf());

        // Store
        let entry = MemoryEntry::new(
            MemoryCategory::Knowledge,
            "BSV uses the original Bitcoin protocol.",
            vec!["bsv".to_string(), "bitcoin".to_string()],
            "test",
        );
        let id = entry.id.clone();
        let path = store.store(&entry).unwrap();
        assert!(path.exists());

        // Read
        let loaded = store.read(&id).unwrap();
        assert_eq!(loaded.content, "BSV uses the original Bitcoin protocol.");
        assert_eq!(loaded.tags, vec!["bsv", "bitcoin"]);

        // List
        let all = store.list(None).unwrap();
        assert_eq!(all.len(), 1);

        let knowledge = store.list(Some(MemoryCategory::Knowledge)).unwrap();
        assert_eq!(knowledge.len(), 1);

        let sessions = store.list(Some(MemoryCategory::Session)).unwrap();
        assert_eq!(sessions.len(), 0);

        // Delete
        store.delete(&id).unwrap();
        assert!(store.read(&id).is_err());
    }

    #[test]
    fn test_store_creates_directories() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("deep").join("nested").join("memory");
        let store = MemoryStore::new(base.clone());

        assert!(base.join("knowledge").exists());
        assert!(base.join("sessions").exists());
        assert!(base.join("execution").exists());

        // Can store in new directories
        let entry = MemoryEntry::new(
            MemoryCategory::Execution,
            "cached tool output",
            vec![],
            "test",
        );
        store.store(&entry).unwrap();
    }

    #[test]
    fn test_list_multiple_categories() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().to_path_buf());

        let e1 = MemoryEntry::new(MemoryCategory::Knowledge, "fact one", vec![], "test");
        let e2 = MemoryEntry::new(MemoryCategory::Session, "session summary", vec![], "test");
        let e3 = MemoryEntry::new(MemoryCategory::Execution, "execution log", vec![], "test");
        store.store(&e1).unwrap();
        store.store(&e2).unwrap();
        store.store(&e3).unwrap();

        let all = store.list(None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_delete_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path().to_path_buf());
        assert!(store.delete("nonexistent-id").is_err());
    }
}
