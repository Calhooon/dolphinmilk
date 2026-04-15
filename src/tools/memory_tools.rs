//! Memory tools — store and search persistent knowledge.
//!
//! Two tools for the agent:
//!   - `memory_store`: save information for future sessions
//!   - `memory_search`: find relevant memories by query

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::memory::processing;
use crate::memory::search::MemoryIndex;
use crate::memory::store::{MemoryCategory, MemoryEntry, MemoryStore};
use crate::tools::registry::ToolDef;

/// Maximum search results to return.
const DEFAULT_SEARCH_LIMIT: usize = 5;

/// Maximum content length in search result snippets.
const MAX_SNIPPET_LEN: usize = 500;

async fn memory_store_impl(params: Value, memory_dir: PathBuf) -> String {
    let category_str = params
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("knowledge");
    let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let tags: Vec<String> = params
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let source = params
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");

    if content.is_empty() {
        return "Error: content is required".to_string();
    }

    let category = match category_str.parse::<MemoryCategory>() {
        Ok(c) => c,
        Err(e) => return format!("Error: {e}"),
    };

    let store = MemoryStore::new(memory_dir.clone());
    let entry = MemoryEntry::new(category, content, tags, source);
    let entry_id = entry.id.clone();

    if let Err(e) = store.store(&entry) {
        return format!("Error storing memory: {e}");
    }

    // Post-store processing: quality check, dedup, tag extraction
    let mut processing_warnings: Vec<String> = Vec::new();
    let mut possible_duplicate: Option<String> = None;
    let mut extracted_tags: Vec<String> = Vec::new();

    let index_dir = memory_dir.join("index");
    match MemoryIndex::new(index_dir) {
        Ok(index) => {
            // Run post-processing BEFORE adding to index (so dedup searches
            // against existing entries only, not the entry we just stored)
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                processing::post_process(&index, content, &entry_id)
            })) {
                Ok(result) => {
                    processing_warnings = result.warnings;
                    if result.dedup.is_duplicate {
                        possible_duplicate = result.dedup.matching_id;
                    }
                    extracted_tags = result.extracted_tags;
                }
                Err(e) => {
                    tracing::warn!(
                        "Post-processing panicked for {}: {:?}",
                        entry_id,
                        e.downcast_ref::<String>()
                    );
                }
            }

            // Now add the entry to the search index
            if let Err(e) = index.add_entry(&entry) {
                tracing::warn!("Failed to index memory entry: {e}");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to open memory index: {e}");
        }
    }

    let mut response = json!({
        "status": "stored",
        "id": entry_id,
        "category": category_str,
    });

    // Include processing results in the response
    if !processing_warnings.is_empty() {
        response["warnings"] = json!(processing_warnings);
    }
    if let Some(dup_id) = possible_duplicate {
        response["possible_duplicate"] = json!(dup_id);
    }
    if !extracted_tags.is_empty() {
        response["extracted_tags"] = json!(extracted_tags);
    }

    serde_json::to_string(&response).unwrap_or_else(|_| "Error: serialization failed".to_string())
}

async fn memory_search_impl(params: Value, memory_dir: PathBuf) -> String {
    let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_SEARCH_LIMIT as u64) as usize;

    if query.is_empty() {
        return "Error: query is required".to_string();
    }

    let index_dir = memory_dir.join("index");
    let index = match MemoryIndex::new(index_dir) {
        Ok(idx) => idx,
        Err(e) => return format!("Error opening memory index: {e}"),
    };

    // Rebuild index from files if empty (e.g. after server restart or index deletion)
    if index.entry_count().unwrap_or(0) == 0 {
        let store = MemoryStore::new(memory_dir.clone());
        if let Ok(all) = store.list(None) {
            if !all.is_empty() {
                let _ = index.rebuild(&all);
            }
        }
    }

    match index.search(query, limit) {
        Ok(snippets) => {
            if snippets.is_empty() {
                return json!({
                    "results": [],
                    "query": query,
                    "message": "No matching memories found"
                })
                .to_string();
            }

            let results: Vec<Value> = snippets
                .iter()
                .map(|s| {
                    let content = if s.content.len() > MAX_SNIPPET_LEN {
                        let truncated = &s.content[..s
                            .content
                            .char_indices()
                            .take_while(|(i, _)| *i < MAX_SNIPPET_LEN)
                            .last()
                            .map(|(i, c)| i + c.len_utf8())
                            .unwrap_or(MAX_SNIPPET_LEN)];
                        format!("{truncated}...")
                    } else {
                        s.content.clone()
                    };

                    json!({
                        "id": s.id,
                        "category": s.category,
                        "content": content,
                        "score": s.score,
                        "tags": s.tags,
                        "created": s.created,
                    })
                })
                .collect();

            serde_json::to_string(&json!({
                "results": results,
                "query": query,
                "count": results.len(),
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string())
        }
        Err(e) => format!("Error searching memory: {e}"),
    }
}

/// Create all memory tool definitions.
///
/// `memory_dir` is the root directory for memory files (e.g. `workspace/memory/`).
/// The path is captured by the tool closures for use at execution time.
pub fn all_memory_tools(memory_dir: PathBuf) -> Vec<ToolDef> {
    let store_dir = Arc::new(memory_dir.clone());
    let search_dir = Arc::new(memory_dir);

    vec![
        ToolDef {
            name: "memory_store".to_string(),
            description: "Store information in persistent memory for future sessions. \
                Use this to save important facts, patterns, or insights you discover."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Category: 'knowledge' (facts/patterns), 'execution' (action logs), or 'session' (session summaries)",
                        "enum": ["knowledge", "execution", "session"]
                    },
                    "content": {
                        "type": "string",
                        "description": "The information to store"
                    },
                    "tags": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Searchable tags for this memory"
                    },
                    "source": {
                        "type": "string",
                        "description": "Where this information came from"
                    }
                },
                "required": ["content"]
            }),
            execute: {
                let dir = Arc::clone(&store_dir);
                Box::new(move |params| {
                    let d = dir.as_ref().clone();
                    Box::pin(memory_store_impl(params, d))
                })
            },
            category: "memory".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        ToolDef {
            name: "memory_search".to_string(),
            description: "Search persistent memory using natural language queries. \
                Returns ranked results by relevance (BM25). Use to recall previously stored knowledge."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 5)"
                    }
                },
                "required": ["query"]
            }),
            execute: {
                let dir = Arc::clone(&search_dir);
                Box::new(move |params| {
                    let d = dir.as_ref().clone();
                    Box::pin(memory_search_impl(params, d))
                })
            },
            category: "memory".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_store_tool() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().to_path_buf();

        let result = memory_store_impl(
            json!({
                "category": "knowledge",
                "content": "BSV uses the original Bitcoin protocol",
                "tags": ["bsv", "bitcoin"],
                "source": "test"
            }),
            memory_dir,
        )
        .await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "stored");
        assert!(parsed["id"].as_str().is_some());
        assert_eq!(parsed["category"], "knowledge");
    }

    #[tokio::test]
    async fn test_memory_store_empty_content() {
        let dir = tempfile::tempdir().unwrap();
        let result = memory_store_impl(json!({"content": ""}), dir.path().to_path_buf()).await;
        assert!(result.starts_with("Error"));
    }

    #[tokio::test]
    async fn test_memory_store_invalid_category() {
        let dir = tempfile::tempdir().unwrap();
        let result = memory_store_impl(
            json!({"category": "invalid", "content": "test"}),
            dir.path().to_path_buf(),
        )
        .await;
        assert!(result.starts_with("Error"));
    }

    #[tokio::test]
    async fn test_memory_search_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let result = memory_search_impl(json!({"query": ""}), dir.path().to_path_buf()).await;
        assert!(result.starts_with("Error"));
    }

    #[tokio::test]
    async fn test_memory_store_then_search() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().to_path_buf();

        // Store a memory
        memory_store_impl(
            json!({
                "category": "knowledge",
                "content": "The x402 payment protocol uses BEEF transactions for payment verification",
                "tags": ["x402", "beef", "payment"]
            }),
            memory_dir.clone(),
        )
        .await;

        // Search for it
        let result = memory_search_impl(json!({"query": "x402 payment BEEF"}), memory_dir).await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["count"].as_u64().unwrap() > 0);
        let first = &parsed["results"][0];
        assert!(first["content"].as_str().unwrap().contains("x402"));
    }

    #[tokio::test]
    async fn test_memory_search_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().to_path_buf();

        // Store something
        memory_store_impl(
            json!({
                "content": "BSV blockchain facts",
                "tags": ["bsv"]
            }),
            memory_dir.clone(),
        )
        .await;

        // Search for something else entirely
        let result =
            memory_search_impl(json!({"query": "quantum computing algorithms"}), memory_dir).await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        // May or may not match — BM25 might find partial matches
        // Just verify the response is valid JSON
        assert!(parsed["results"].is_array());
    }

    #[tokio::test]
    async fn test_all_memory_tools_creates_two_tools() {
        let dir = tempfile::tempdir().unwrap();
        let tools = all_memory_tools(dir.path().to_path_buf());
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"memory_store"));
        assert!(names.contains(&"memory_search"));
    }
}
