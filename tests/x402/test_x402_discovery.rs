//! Integration tests for x402 service discovery (registry + manifests).
//!
//! Uses mockito for HTTP mocking — no real network or wallet needed.

use serde_json::json;
use tempfile::TempDir;

use dolphin_milk::x402::{discovery, registry};

// ---------------------------------------------------------------------------
// Registry tests
// ---------------------------------------------------------------------------

fn mock_agents_json() -> String {
    json!({
        "agents": [
            {
                "name": "banana",
                "display_name": "Banana Pro",
                "url": "https://nano-banana-pro.x402agency.com",
                "tagline": "Image generation",
                "capabilities": ["image"]
            },
            {
                "name": "whisper",
                "display_name": "Whisper Turbo",
                "url": "https://whisper-large-v3-turbo.x402agency.com",
                "tagline": "Audio transcription",
                "capabilities": ["audio"]
            },
            {
                "name": "x-research",
                "display_name": "X Research",
                "url": "https://x-research.x402agency.com",
                "tagline": "Twitter/X search",
                "capabilities": ["search"]
            }
        ]
    })
    .to_string()
}

#[tokio::test]
async fn test_list_agents_from_mock_server() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/.well-known/agents")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_agents_json())
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    let agents = registry::list_agents_from(
        &format!("{}/.well-known/agents", server.url()),
        Some(&cache_path),
    )
    .await
    .unwrap();

    assert_eq!(agents.len(), 3);
    assert_eq!(agents[0].name, "banana");
    assert_eq!(agents[1].name, "whisper");
    assert_eq!(agents[2].name, "x-research");
    mock.assert_async().await;
}

#[tokio::test]
async fn test_list_agents_uses_cache() {
    // Pre-populate cache — no HTTP request should be made
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let cached = json!({
        "timestamp": now,
        "agents": [{
            "name": "cached-agent",
            "display_name": "Cached",
            "url": "https://cached.example.com",
            "tagline": "From cache",
            "capabilities": []
        }]
    });
    std::fs::write(&cache_path, cached.to_string()).unwrap();

    // Use a URL that would fail if actually fetched
    let agents = registry::list_agents_from("http://localhost:1/nope", Some(&cache_path))
        .await
        .unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "cached-agent");
}

#[tokio::test]
async fn test_list_agents_cache_expired_refetches() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    // Write expired cache (timestamp = 0)
    let cached = json!({
        "timestamp": 0.0,
        "agents": [{"name": "stale", "url": "https://stale.com"}]
    });
    std::fs::write(&cache_path, cached.to_string()).unwrap();

    // Mock fresh data
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/.well-known/agents")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_agents_json())
        .create_async()
        .await;

    let agents = registry::list_agents_from(
        &format!("{}/.well-known/agents", server.url()),
        Some(&cache_path),
    )
    .await
    .unwrap();

    assert_eq!(agents.len(), 3);
    assert_eq!(agents[0].name, "banana");
    mock.assert_async().await;
}

#[tokio::test]
async fn test_list_agents_handles_server_error() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/.well-known/agents")
        .with_status(500)
        .with_body("Internal Server Error")
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    let err = registry::list_agents_from(
        &format!("{}/.well-known/agents", server.url()),
        Some(&cache_path),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("500"));
}

#[tokio::test]
async fn test_resolve_from_mock_registry() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/.well-known/agents")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_agents_json())
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    let url = registry::resolve_from(
        "banana",
        &format!("{}/.well-known/agents", server.url()),
        Some(&cache_path),
    )
    .await
    .unwrap();

    assert_eq!(url, "https://nano-banana-pro.x402agency.com");
}

#[tokio::test]
async fn test_resolve_with_path_from_mock_registry() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/.well-known/agents")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_agents_json())
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    let url = registry::resolve_from(
        "banana/generate",
        &format!("{}/.well-known/agents", server.url()),
        Some(&cache_path),
    )
    .await
    .unwrap();

    assert_eq!(url, "https://nano-banana-pro.x402agency.com/generate");
}

#[tokio::test]
async fn test_resolve_case_insensitive() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/.well-known/agents")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_agents_json())
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("cache.json");

    let url = registry::resolve_from(
        "BANANA",
        &format!("{}/.well-known/agents", server.url()),
        Some(&cache_path),
    )
    .await
    .unwrap();

    assert_eq!(url, "https://nano-banana-pro.x402agency.com");
}

// ---------------------------------------------------------------------------
// Manifest / discovery tests
// ---------------------------------------------------------------------------

fn mock_manifest_json() -> String {
    json!({
        "name": "Banana Pro",
        "description": "AI image generation service",
        "server_identity_key": "0280451234567890abcdef1234567890abcdef1234567890abcdef1234567890ab",
        "auth_endpoint": "https://banana.example.com/.well-known/auth",
        "auth_protocol": "BRC-31",
        "endpoints": [
            {
                "path": "/generate",
                "method": "POST",
                "description": "Generate an image from a text prompt",
                "auth": true,
                "delivery": "async-poll",
                "payment": {"dynamic": true},
                "input": {
                    "type": "object",
                    "properties": {
                        "prompt": {"type": "string"},
                        "resolution": {"type": "string"}
                    }
                },
                "hint": "Keep prompts under 500 chars"
            },
            {
                "path": "/status/{id}",
                "method": "GET",
                "description": "Check generation status",
                "auth": true,
                "payment": null
            }
        ],
        "pricing": {"base_sats": 500}
    })
    .to_string()
}

#[tokio::test]
async fn test_fetch_manifest_from_mock() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/.well-known/x402-info")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_manifest_json())
        .create_async()
        .await;

    let manifest = discovery::fetch_manifest(&server.url()).await.unwrap();

    assert_eq!(manifest.name, "Banana Pro");
    assert_eq!(manifest.endpoints.len(), 2);
    assert_eq!(manifest.endpoints[0].path, "/generate");
    assert!(manifest.endpoints[0].has_payment());
    assert!(!manifest.endpoints[1].has_payment());
    assert_eq!(manifest.auth_protocol, "BRC-31");
    mock.assert_async().await;
}

#[tokio::test]
async fn test_fetch_manifest_with_missing_fields() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/.well-known/x402-info")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name": "Minimal"}"#)
        .create_async()
        .await;

    let manifest = discovery::fetch_manifest(&server.url()).await.unwrap();

    assert_eq!(manifest.name, "Minimal");
    assert!(manifest.endpoints.is_empty());
    assert!(manifest.description.is_empty());
    assert!(manifest.server_identity_key.is_empty());
}

#[tokio::test]
async fn test_fetch_manifest_handles_404() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/.well-known/x402-info")
        .with_status(404)
        .with_body("Not Found")
        .create_async()
        .await;

    let err = discovery::fetch_manifest(&server.url()).await.unwrap_err();
    assert!(err.to_string().contains("404"));
}

#[test]
fn test_format_manifest_summary_structure() {
    let manifest =
        serde_json::from_str::<discovery::ServiceManifest>(&mock_manifest_json()).unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(summary.contains("Banana Pro"));
    assert!(summary.contains("POST /generate"));
    assert!(summary.contains("[auth, paid]"));
    assert!(summary.contains("GET /status/{id}"));
    assert!(summary.contains("prompt"));
    assert!(summary.contains("resolution"));
    assert!(summary.contains("async-poll"));
    assert!(summary.contains("Hint: Keep prompts under 500 chars"));
}

#[test]
fn test_agent_entry_extra_fields_preserved() {
    let json = r#"{"name":"test","custom_field":"hello","url":"https://test.com"}"#;
    let entry: registry::AgentEntry = serde_json::from_str(json).unwrap();
    assert_eq!(entry.name, "test");
    assert_eq!(entry.extra.get("custom_field").unwrap(), "hello");
}

#[test]
fn test_agent_entry_empty_capabilities() {
    let json = r#"{"name":"test","capabilities":[]}"#;
    let entry: registry::AgentEntry = serde_json::from_str(json).unwrap();
    assert!(entry.capabilities.is_empty());
}

// ---------------------------------------------------------------------------
// Enhanced manifest summary tests — polling, output, timing, pricing
// ---------------------------------------------------------------------------

fn rich_manifest_json() -> String {
    json!({
        "name": "Rich Service",
        "description": "A fully-featured async service",
        "server_identity_key": "0280451234567890abcdef1234567890abcdef1234567890abcdef1234567890ab",
        "auth_protocol": "BRC-31",
        "endpoints": [
            {
                "path": "/generate",
                "method": "POST",
                "description": "Generate content",
                "auth": true,
                "delivery": "async-poll",
                "payment": {
                    "dynamic": true,
                    "tiers": {
                        "1K": { "example_sats": 37000, "usd": 0.19 },
                        "4K": { "example_sats": 75000, "usd": 0.38 }
                    }
                },
                "input": {
                    "schema": {
                        "prompt": {"type": "string", "required": true}
                    }
                },
                "output": {
                    "contentType": "application/json",
                    "schema": {
                        "prediction_id": "string",
                        "status": "string",
                        "poll_url": "string"
                    }
                },
                "polling": {
                    "endpoint": "/status/{prediction_id}",
                    "method": "GET",
                    "interval_seconds": 15,
                    "max_wait_seconds": 180,
                    "terminal_states": ["succeeded", "failed", "canceled"],
                    "identity_scoped": true,
                    "note": "Only the paying identity can poll"
                },
                "refund": {
                    "supported": true,
                    "delivery": "inline"
                },
                "timing": {
                    "response": "immediate",
                    "generation": "~2 minutes"
                }
            }
        ]
    })
    .to_string()
}

#[test]
fn test_format_manifest_summary_includes_polling() {
    let manifest: discovery::ServiceManifest = serde_json::from_str(&rich_manifest_json()).unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(summary.contains("Polling:"), "should show Polling section");
    assert!(
        summary.contains("/status/{prediction_id}"),
        "should show polling endpoint pattern"
    );
    assert!(summary.contains("15"), "should show interval");
    assert!(summary.contains("180"), "should show max wait");
    assert!(summary.contains("succeeded"), "should show terminal states");
    assert!(
        summary.contains("identity_scoped"),
        "should show identity scoping"
    );
}

#[test]
fn test_format_manifest_summary_includes_output_schema() {
    let manifest: discovery::ServiceManifest = serde_json::from_str(&rich_manifest_json()).unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(summary.contains("Output:"), "should show Output section");
    assert!(
        summary.contains("prediction_id"),
        "should show output fields"
    );
    assert!(summary.contains("status"), "should show status field");
    assert!(summary.contains("poll_url"), "should show poll_url field");
}

#[test]
fn test_format_manifest_summary_includes_payment_tiers() {
    let manifest: discovery::ServiceManifest = serde_json::from_str(&rich_manifest_json()).unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(
        summary.contains("Payment tiers:"),
        "should show payment tiers section"
    );
    assert!(summary.contains("37000"), "should show 1K tier sats");
    assert!(summary.contains("75000"), "should show 4K tier sats");
    assert!(summary.contains("$0.19"), "should show 1K tier USD");
}

#[test]
fn test_format_manifest_summary_includes_refund() {
    let manifest: discovery::ServiceManifest = serde_json::from_str(&rich_manifest_json()).unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(
        summary.contains("Refund: supported"),
        "should show refund policy"
    );
}

#[test]
fn test_format_manifest_summary_includes_timing() {
    let manifest: discovery::ServiceManifest = serde_json::from_str(&rich_manifest_json()).unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(summary.contains("Timing:"), "should show timing section");
    assert!(
        summary.contains("~2 minutes"),
        "should show generation timing"
    );
}

#[test]
fn test_format_manifest_summary_no_polling_for_sync() {
    // Sync endpoint should not show polling section
    let manifest: discovery::ServiceManifest = serde_json::from_str(
        &json!({
            "name": "Sync Service",
            "endpoints": [{
                "path": "/search",
                "method": "POST",
                "auth": true,
                "delivery": "synchronous",
                "payment": {"dynamic": true}
            }]
        })
        .to_string(),
    )
    .unwrap();
    let summary = discovery::format_manifest_summary(&manifest);

    assert!(
        !summary.contains("Polling:"),
        "sync service should not show polling"
    );
    assert!(summary.contains("synchronous"), "should show sync delivery");
}
