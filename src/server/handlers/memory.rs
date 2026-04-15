//! Memory browsing route handlers.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::types::*;
use super::super::AppState;

/// GET /memory -- list all memories with optional category filter + pagination.
pub(crate) async fn list_memories(
    State(state): State<Arc<AppState>>,
    uri: axum::extract::OriginalUri,
    Query(params): Query<MemoryListParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/memory",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;
    let memory_dir = state.workspace.join("memory");
    let store = crate::memory::MemoryStore::new(memory_dir);

    let category_filter = params
        .category
        .as_deref()
        .and_then(|c| c.parse::<crate::memory::MemoryCategory>().ok());

    let entries = store
        .list(category_filter)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Compute per-category counts from full list (before pagination)
    let mut categories: HashMap<String, usize> = HashMap::new();
    // Always count all entries (not just filtered ones) for the category counts
    let all_entries = store.list(None).unwrap_or_default();
    for e in &all_entries {
        *categories.entry(e.category.to_string()).or_insert(0) += 1;
    }

    let total = entries.len();

    // Apply pagination
    let offset = params.offset.min(total);
    let limit = params.limit.min(total.saturating_sub(offset));
    let page = &entries[offset..offset + limit];

    let items: Vec<MemoryListItem> = page
        .iter()
        .map(|e| {
            let preview: String = e.content.chars().take(200).collect();
            MemoryListItem {
                id: e.id.clone(),
                category: e.category.to_string(),
                tags: e.tags.clone(),
                created: e.created.to_rfc3339(),
                source: e.source.clone(),
                content_preview: preview,
            }
        })
        .collect();

    let body = MemoryListResponse {
        entries: items,
        total,
        categories,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /memory/search?q=...&limit=5 -- search memories via text matching.
pub(crate) async fn search_memories(
    State(state): State<Arc<AppState>>,
    uri: axum::extract::OriginalUri,
    Query(params): Query<MemorySearchParams>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let query_str = uri.query().map(|q| q.to_string());
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        "/memory/search",
        query_str.as_deref(),
        &headers,
        None,
    )
    .await?;

    let memory_dir = state.workspace.join("memory");
    let index_dir = memory_dir.join("index");

    // Try tantivy index first
    let results = match crate::memory::MemoryIndex::new(index_dir) {
        Ok(index) => {
            // Rebuild index from files if empty (server may not have built it yet)
            if index.entry_count().unwrap_or(0) == 0 {
                let store = crate::memory::MemoryStore::new(memory_dir.clone());
                let all = store.list(None).unwrap_or_default();
                if !all.is_empty() {
                    let _ = index.rebuild(&all);
                }
            }
            match index.search(&params.q, params.limit) {
                Ok(snippets) => snippets
                    .into_iter()
                    .map(|s| MemorySearchResultItem {
                        id: s.id,
                        category: s.category,
                        content: s.content,
                        score: s.score,
                        tags: s.tags,
                        created: s.created,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
        Err(_) => {
            // Fallback: simple text search through files
            let store = crate::memory::MemoryStore::new(memory_dir);
            let all = store.list(None).unwrap_or_default();
            let query_lower = params.q.to_lowercase();
            all.into_iter()
                .filter(|e| {
                    e.content.to_lowercase().contains(&query_lower)
                        || e.tags
                            .iter()
                            .any(|t| t.to_lowercase().contains(&query_lower))
                })
                .take(params.limit)
                .map(|e| MemorySearchResultItem {
                    id: e.id,
                    category: e.category.to_string(),
                    content: e.content,
                    score: 1.0,
                    tags: e.tags,
                    created: e.created.to_rfc3339(),
                })
                .collect()
        }
    };

    let body = MemorySearchResponse {
        query: params.q,
        results,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}

/// GET /memory/{id} -- read a single memory entry by ID.
pub(crate) async fn get_memory(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/memory/{id}"),
        None,
        &headers,
        None,
    )
    .await?;
    let memory_dir = state.workspace.join("memory");
    let store = crate::memory::MemoryStore::new(memory_dir);

    let entry = store.read(&id).map_err(|_| StatusCode::NOT_FOUND)?;

    let body = MemoryDetailResponse {
        id: entry.id,
        category: entry.category.to_string(),
        content: entry.content,
        tags: entry.tags,
        created: entry.created.to_rfc3339(),
        source: entry.source,
    };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &body).await
}
