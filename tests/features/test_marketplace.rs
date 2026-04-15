//! Tests for the plugin marketplace API.

use axum::body::Body;
use axum::http::{self, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use dolphin_milk::server::{PluginListResponse, PluginListing};

#[path = "../common/mod.rs"]
mod common;

// -- CRUD operations --

#[tokio::test]
async fn test_list_plugins_empty() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/marketplace/plugins")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let list: PluginListResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.count, 0);
    assert!(list.plugins.is_empty());
}

#[tokio::test]
async fn test_create_and_get_plugin() {
    let (app, _workspace) = common::test_router_with_workspace().await;

    let plugin = serde_json::json!({
        "name": "test-plugin",
        "description": "A test plugin",
        "version": "1.0.0",
        "author": "test-author",
        "category": "tools",
        "tools": ["tool_a", "tool_b"],
        "skills": [],
        "download_url": "https://example.com/plugin.tar.gz",
        "verified": false
    });

    // Create
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let created: PluginListing = serde_json::from_slice(&body).unwrap();
    assert_eq!(created.name, "test-plugin");
    assert_eq!(created.version, "1.0.0");
    assert!(!created.updated_at.is_empty());

    // Get
    let req = Request::builder()
        .uri("/marketplace/plugins/test-plugin")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let fetched: PluginListing = serde_json::from_slice(&body).unwrap();
    assert_eq!(fetched.name, "test-plugin");
    assert_eq!(fetched.author, "test-author");
    assert_eq!(fetched.tools, vec!["tool_a", "tool_b"]);
}

#[tokio::test]
async fn test_list_plugins_after_create() {
    let (app, _workspace) = common::test_router_with_workspace().await;

    // Create two plugins
    for name in &["alpha-plugin", "beta-plugin"] {
        let plugin = serde_json::json!({
            "name": name,
            "description": format!("Plugin {}", name),
            "version": "1.0.0",
            "author": "author",
            "category": "tools"
        });

        let req = Request::builder()
            .method(http::Method::POST)
            .uri("/marketplace/plugins")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&plugin).unwrap()))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // List
    let req = Request::builder()
        .uri("/marketplace/plugins")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let list: PluginListResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.count, 2);
    // Sorted alphabetically
    assert_eq!(list.plugins[0].name, "alpha-plugin");
    assert_eq!(list.plugins[1].name, "beta-plugin");
}

#[tokio::test]
async fn test_get_nonexistent_plugin() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/marketplace/plugins/nonexistent")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_update_plugin() {
    let (app, _workspace) = common::test_router_with_workspace().await;

    // Create v1
    let plugin_v1 = serde_json::json!({
        "name": "updatable",
        "description": "Version 1",
        "version": "1.0.0",
        "author": "author",
        "category": "tools"
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin_v1).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Update to v2
    let plugin_v2 = serde_json::json!({
        "name": "updatable",
        "description": "Version 2 with new features",
        "version": "2.0.0",
        "author": "author",
        "category": "tools"
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin_v2).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Verify updated
    let req = Request::builder()
        .uri("/marketplace/plugins/updatable")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let fetched: PluginListing = serde_json::from_slice(&body).unwrap();
    assert_eq!(fetched.version, "2.0.0");
    assert_eq!(fetched.description, "Version 2 with new features");
}

#[tokio::test]
async fn test_delete_plugin() {
    let (app, _workspace) = common::test_router_with_workspace().await;

    // Create
    let plugin = serde_json::json!({
        "name": "deletable",
        "description": "To be deleted",
        "version": "1.0.0",
        "author": "author",
        "category": "tools"
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Delete
    let req = Request::builder()
        .method(http::Method::DELETE)
        .uri("/marketplace/plugins/deletable")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify deleted
    let req = Request::builder()
        .uri("/marketplace/plugins/deletable")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_nonexistent_plugin() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::DELETE)
        .uri("/marketplace/plugins/nonexistent")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// -- JSON persistence --

#[tokio::test]
async fn test_plugin_persisted_as_json() {
    let (app, workspace) = common::test_router_with_workspace().await;

    let plugin = serde_json::json!({
        "name": "persistent-plugin",
        "description": "Persisted to disk",
        "version": "1.0.0",
        "author": "author",
        "category": "skills"
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Check file exists on disk
    let file_path = workspace.join("marketplace").join("persistent-plugin.json");
    assert!(file_path.exists());

    let content = std::fs::read_to_string(&file_path).unwrap();
    let on_disk: PluginListing = serde_json::from_str(&content).unwrap();
    assert_eq!(on_disk.name, "persistent-plugin");
    assert_eq!(on_disk.category, "skills");
}

// -- Validation --

#[tokio::test]
async fn test_create_invalid_plugin_no_name() {
    let app = common::test_router().await;

    let plugin = serde_json::json!({
        "name": "",
        "description": "No name",
        "version": "1.0.0",
        "author": "x",
        "category": "tools"
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_invalid_plugin_bad_name() {
    let app = common::test_router().await;

    let plugin = serde_json::json!({
        "name": "bad/name!@#",
        "description": "Invalid chars",
        "version": "1.0.0",
        "author": "x",
        "category": "tools"
    });

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&plugin).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_invalid_json() {
    let app = common::test_router().await;

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/marketplace/plugins")
        .header("content-type", "application/json")
        .body(Body::from("not json"))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// -- Plugin listing serde --

#[test]
fn test_plugin_listing_serde() {
    let listing = PluginListing {
        name: "test".to_string(),
        description: "desc".to_string(),
        version: "1.0.0".to_string(),
        author: "auth".to_string(),
        category: "tools".to_string(),
        tools: vec!["a".to_string()],
        skills: vec![],
        download_url: "https://example.com".to_string(),
        verified: true,
        updated_at: "2026-01-01".to_string(),
    };

    let json = serde_json::to_string(&listing).unwrap();
    let parsed: PluginListing = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "test");
    assert!(parsed.verified);
    assert_eq!(parsed.tools, vec!["a"]);
}

#[test]
fn test_plugin_listing_defaults() {
    let json = r#"{"name":"x","description":"y","version":"1","author":"a","category":"c"}"#;
    let listing: PluginListing = serde_json::from_str(json).unwrap();
    assert!(listing.tools.is_empty());
    assert!(listing.skills.is_empty());
    assert_eq!(listing.download_url, "");
    assert!(!listing.verified);
}
