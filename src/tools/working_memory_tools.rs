//! Working memory tools — scratch pad that persists across iterations.
//!
//! Four tools:
//!   - `working_memory_set`: store a key-value pair
//!   - `working_memory_clear`: remove a key
//!   - `working_memory_list`: list all keys with metadata
//!   - `working_memory_clear_all`: remove all keys

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::runner::working_memory::WorkingMemory;
use crate::tools::registry::ToolDef;

/// Create all working memory tool definitions.
///
/// The `WorkingMemory` is shared between the tools and the runner via `Arc<RwLock<_>>`.
/// The `current_iteration` closure provides the current iteration number for TTL tracking.
pub fn all_working_memory_tools(
    wm: Arc<RwLock<WorkingMemory>>,
    current_iteration: Arc<dyn Fn() -> u32 + Send + Sync>,
) -> Vec<ToolDef> {
    let wm_set = Arc::clone(&wm);
    let wm_clear = Arc::clone(&wm);
    let wm_list = Arc::clone(&wm);
    let wm_clear_all = Arc::clone(&wm);
    let iter_set = Arc::clone(&current_iteration);

    vec![
        ToolDef {
            name: "working_memory_set".to_string(),
            description: "Store a key-value pair in working memory. Working memory persists \
                across iterations within this task and is injected into the system prompt \
                every iteration — immune to context compaction. Use for key findings, \
                intermediate results, and important state. (32 KB total, 8 KB per key)"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Short descriptive key (e.g. 'api_endpoint', 'bug_location', 'plan_step')"
                    },
                    "value": {
                        "type": "string",
                        "description": "The value to store. Keep concise — this appears in every prompt."
                    }
                },
                "required": ["key", "value"]
            }),
            execute: {
                Box::new(move |params| {
                    let wm = Arc::clone(&wm_set);
                    let iter_fn = Arc::clone(&iter_set);
                    Box::pin(async move {
                        let key = params
                            .get("key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let value = params
                            .get("value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        if key.is_empty() {
                            return "Error: key is required".to_string();
                        }
                        if value.is_empty() {
                            return "Error: value is required".to_string();
                        }

                        let iteration = iter_fn();
                        let mut wm = wm.write().await;
                        match wm.set(key.clone(), value, iteration) {
                            Ok((used, remaining)) => {
                                tracing::info!(
                                    key = %key,
                                    bytes_used = used,
                                    bytes_remaining = remaining,
                                    iteration = iteration,
                                    "working_memory_set"
                                );
                                format!(
                                    "{} set ({} bytes used, {} bytes remaining)",
                                    key, used, remaining
                                )
                            }
                            Err(e) => {
                                tracing::warn!(key = %key, error = %e, "working_memory_set failed");
                                format!("Error: {}", e)
                            }
                        }
                    })
                })
            },
            category: "memory".to_string(),
            cleanup: None,
            deferred: false,
            always_load: true,
            search_hint: None,
        },
        ToolDef {
            name: "working_memory_clear".to_string(),
            description: "Remove a key from working memory.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The key to remove"
                    }
                },
                "required": ["key"]
            }),
            execute: {
                Box::new(move |params| {
                    let wm = Arc::clone(&wm_clear);
                    Box::pin(async move {
                        let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");

                        if key.is_empty() {
                            return "Error: key is required".to_string();
                        }

                        let mut wm = wm.write().await;
                        if wm.clear(key) {
                            tracing::info!(key = %key, "working_memory_clear");
                            format!("Cleared {}", key)
                        } else {
                            format!("Key not found: {}", key)
                        }
                    })
                })
            },
            category: "memory".to_string(),
            cleanup: None,
            deferred: false,
            always_load: true,
            search_hint: None,
        },
        ToolDef {
            name: "working_memory_list".to_string(),
            description: "List all keys in working memory with metadata (bytes, iteration set)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            execute: {
                Box::new(move |_params| {
                    let wm = Arc::clone(&wm_list);
                    Box::pin(async move {
                        let wm = wm.read().await;
                        let items = wm.list();
                        let (used, remaining) = wm.usage();

                        let entries: Vec<Value> = items
                            .iter()
                            .map(|(key, bytes, iteration)| {
                                json!({
                                    "key": key,
                                    "bytes": bytes,
                                    "iteration_set": iteration,
                                })
                            })
                            .collect();

                        json!({
                            "entries": entries,
                            "total_bytes_used": used,
                            "total_bytes_remaining": remaining,
                        })
                        .to_string()
                    })
                })
            },
            category: "memory".to_string(),
            cleanup: None,
            deferred: false,
            always_load: true,
            search_hint: None,
        },
        ToolDef {
            name: "working_memory_clear_all".to_string(),
            description: "Clear all keys from working memory.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            execute: {
                Box::new(move |_params| {
                    let wm = Arc::clone(&wm_clear_all);
                    Box::pin(async move {
                        let mut wm = wm.write().await;
                        let count = wm.clear_all();
                        tracing::info!(keys_cleared = count, "working_memory_clear_all");
                        format!("Cleared {} keys", count)
                    })
                })
            },
            category: "memory".to_string(),
            cleanup: None,
            deferred: false,
            always_load: true,
            search_hint: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_all_working_memory_tools_creates_four_tools() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        let tools = all_working_memory_tools(wm, iter_fn);
        assert_eq!(tools.len(), 4);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"working_memory_set"));
        assert!(names.contains(&"working_memory_clear"));
        assert!(names.contains(&"working_memory_list"));
        assert!(names.contains(&"working_memory_clear_all"));
    }

    #[tokio::test]
    async fn test_tools_are_always_on() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        let tools = all_working_memory_tools(wm, iter_fn);
        for tool in &tools {
            assert!(tool.always_load, "{} should be always_load", tool.name);
            assert!(!tool.deferred, "{} should not be deferred", tool.name);
            assert_eq!(tool.category, "memory");
        }
    }

    #[tokio::test]
    async fn test_set_tool_impl() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 5);
        let tools = all_working_memory_tools(Arc::clone(&wm), iter_fn);
        let set_tool = &tools[0];

        let result = (set_tool.execute)(json!({"key": "test_key", "value": "test_value"})).await;
        assert!(result.contains("test_key set"));
        assert!(result.contains("bytes used"));

        let wm = wm.read().await;
        assert_eq!(wm.get("test_key"), Some("test_value"));
    }

    #[tokio::test]
    async fn test_clear_tool_impl() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        {
            let mut w = wm.write().await;
            w.set("k".into(), "v".into(), 1).unwrap();
        }
        let tools = all_working_memory_tools(Arc::clone(&wm), iter_fn);
        let clear_tool = &tools[1];

        let result = (clear_tool.execute)(json!({"key": "k"})).await;
        assert_eq!(result, "Cleared k");

        let result = (clear_tool.execute)(json!({"key": "missing"})).await;
        assert_eq!(result, "Key not found: missing");
    }

    #[tokio::test]
    async fn test_list_tool_impl() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        {
            let mut w = wm.write().await;
            w.set("alpha".into(), "a".into(), 1).unwrap();
            w.set("beta".into(), "b".into(), 2).unwrap();
        }
        let tools = all_working_memory_tools(Arc::clone(&wm), iter_fn);
        let list_tool = &tools[2];

        let result = (list_tool.execute)(json!({})).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["entries"].as_array().unwrap().len(), 2);
        assert!(parsed["total_bytes_used"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_clear_all_tool_impl() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        {
            let mut w = wm.write().await;
            w.set("a".into(), "1".into(), 1).unwrap();
            w.set("b".into(), "2".into(), 1).unwrap();
        }
        let tools = all_working_memory_tools(Arc::clone(&wm), iter_fn);
        let clear_all_tool = &tools[3];

        let result = (clear_all_tool.execute)(json!({})).await;
        assert_eq!(result, "Cleared 2 keys");

        let wm = wm.read().await;
        assert!(wm.is_empty());
    }

    #[tokio::test]
    async fn test_set_empty_key_error() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        let tools = all_working_memory_tools(wm, iter_fn);
        let set_tool = &tools[0];

        let result = (set_tool.execute)(json!({"key": "", "value": "v"})).await;
        assert!(result.starts_with("Error"));
    }

    #[tokio::test]
    async fn test_set_empty_value_error() {
        let wm = Arc::new(RwLock::new(WorkingMemory::new()));
        let iter_fn: Arc<dyn Fn() -> u32 + Send + Sync> = Arc::new(|| 1);
        let tools = all_working_memory_tools(wm, iter_fn);
        let set_tool = &tools[0];

        let result = (set_tool.execute)(json!({"key": "k", "value": ""})).await;
        assert!(result.starts_with("Error"));
    }
}
