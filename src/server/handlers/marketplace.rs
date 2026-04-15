//! Plugin marketplace API — CRUD for plugin metadata.
//!
//! Stores plugin listings as JSON files in `workspace/marketplace/`.
//! Provides list, get, create/update, and delete operations.
//! All endpoints require BRC-31 authentication.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use super::super::auth::{check_brc31_auth, signed_json_response};
use super::super::AppState;

/// A plugin listing in the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginListing {
    /// Unique plugin name (used as filename).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic version (e.g. "1.0.0").
    pub version: String,
    /// Author name or identity key.
    pub author: String,
    /// Category (e.g. "tools", "skills", "integrations").
    pub category: String,
    /// Tool names provided by this plugin.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Skill names provided by this plugin.
    #[serde(default)]
    pub skills: Vec<String>,
    /// URL to download the plugin.
    #[serde(default)]
    pub download_url: String,
    /// Whether this plugin has been verified by the marketplace operator.
    #[serde(default)]
    pub verified: bool,
    /// ISO 8601 timestamp of creation/last update.
    #[serde(default)]
    pub updated_at: String,
}

/// Response for listing all plugins.
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginListResponse {
    pub plugins: Vec<PluginListing>,
    pub count: usize,
}

/// GET /marketplace/plugins — list all plugins.
pub(crate) async fn list_plugins(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx =
        check_brc31_auth(&state, "GET", "/marketplace/plugins", None, &headers, None).await?;

    let marketplace_dir = state.workspace.join("marketplace");
    let mut plugins = Vec::new();

    if marketplace_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&marketplace_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(plugin) = serde_json::from_str::<PluginListing>(&content) {
                            plugins.push(plugin);
                        }
                    }
                }
            }
        }
    }

    // Sort by name for deterministic ordering
    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    let count = plugins.len();

    let resp = PluginListResponse { plugins, count };
    signed_json_response(&state, auth_ctx, StatusCode::OK, &resp).await
}

/// GET /marketplace/plugins/{name} — get a specific plugin.
pub(crate) async fn get_plugin(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "GET",
        &format!("/marketplace/plugins/{name}"),
        None,
        &headers,
        None,
    )
    .await?;

    let path = state
        .workspace
        .join("marketplace")
        .join(format!("{name}.json"));

    if !path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let content = std::fs::read_to_string(&path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let plugin: PluginListing =
        serde_json::from_str(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    signed_json_response(&state, auth_ctx, StatusCode::OK, &plugin).await
}

/// POST /marketplace/plugins — create or update a plugin listing (auth required).
pub(crate) async fn create_plugin(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "POST",
        "/marketplace/plugins",
        None,
        &headers,
        Some(body.as_ref()),
    )
    .await?;

    let mut plugin: PluginListing =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    // Validate required fields
    if plugin.name.is_empty() || plugin.description.is_empty() || plugin.version.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Sanitize name (only alphanumeric, hyphens, underscores)
    if !plugin
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Set timestamp
    plugin.updated_at = chrono::Utc::now().to_rfc3339();

    let marketplace_dir = state.workspace.join("marketplace");
    std::fs::create_dir_all(&marketplace_dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let path = marketplace_dir.join(format!("{}.json", plugin.name));
    let json =
        serde_json::to_string_pretty(&plugin).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    std::fs::write(&path, json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    signed_json_response(&state, auth_ctx, StatusCode::CREATED, &plugin).await
}

/// DELETE /marketplace/plugins/{name} — delete a plugin listing (auth required).
pub(crate) async fn delete_plugin(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, StatusCode> {
    let auth_ctx = check_brc31_auth(
        &state,
        "DELETE",
        &format!("/marketplace/plugins/{name}"),
        None,
        &headers,
        Some(body.as_ref()),
    )
    .await?;

    let path = state
        .workspace
        .join("marketplace")
        .join(format!("{name}.json"));

    if !path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    std::fs::remove_file(&path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let body = serde_json::json!({"deleted": name});
    signed_json_response(&state, auth_ctx, StatusCode::NO_CONTENT, &body).await
}

/// GET /services/catalog — live x402 service catalog from registry.
///
/// Returns the list of available x402 agents from the public registry.
/// No authentication required — services are public information.
pub(crate) async fn services_catalog(
    State(_state): State<Arc<AppState>>,
) -> Result<axum::response::Response, StatusCode> {
    use bsv_x402_server::registry;

    match registry::list_agents(None).await {
        Ok(agents) => {
            let entries: Vec<serde_json::Value> = agents
                .iter()
                .map(|a| {
                    let category = a
                        .extra
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let description = a
                        .extra
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    serde_json::json!({
                        "name": a.name,
                        "display_name": a.display_name,
                        "url": a.url,
                        "tagline": a.tagline,
                        "capabilities": a.capabilities,
                        "category": category,
                        "description": description,
                    })
                })
                .collect();

            let count = entries.len();
            let body = serde_json::json!({ "services": entries, "count": count });
            Ok(axum::Json(body).into_response())
        }
        Err(_) => {
            let body =
                serde_json::json!({ "services": [], "count": 0, "error": "Registry unavailable" });
            Ok(axum::Json(body).into_response())
        }
    }
}
