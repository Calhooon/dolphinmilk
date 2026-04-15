//! x402 service tools — discovery, generic call, and recipe tools.
//!
//! Three generic tools:
//!   - `discover_services`: list available x402 agents from registry
//!   - `discover_endpoints`: get endpoint info for a specific service
//!   - `x402_call`: generic x402 call with auto auth + payment
//!
//! Two recipe tools:
//!   - `generate_image`: banana image generation with auto-poll
//!   - `upload_to_nanostore`: two-step reserve + PUT upload

pub(crate) mod call;
pub(crate) mod discovery;
pub(crate) mod recipe;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::auth::AuthriteClient;
use crate::error::DmError;
use crate::tools::registry::ToolDef;
use crate::x402::rate_limit::RateLimiterRegistry;
use crate::x402::{payment, refund};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

type ManifestCache = Arc<Mutex<HashMap<String, crate::x402::discovery::ServiceManifest>>>;

// ---------------------------------------------------------------------------
// Shared x402 helper
// ---------------------------------------------------------------------------

/// Generic BRC-31 auth + 402 payment flow for any x402 service.
pub async fn do_x402_request(
    auth: &AuthriteClient,
    url: &str,
    body: Value,
) -> Result<Value, DmError> {
    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| DmError::payment(format!("Failed to serialize request: {e}")))?;

    let headers: Vec<(String, String)> = vec![("content-type".into(), "application/json".into())];

    let resp =
        payment::authenticated_paid_request(auth, "POST", url, &headers, Some(&body_bytes)).await?;

    if !resp.status.is_success() {
        let data: Value = serde_json::from_slice(&resp.body).unwrap_or(json!({}));
        return Err(DmError::payment(format!(
            "x402 request failed: HTTP {}: {data}",
            resp.status
        )));
    }

    let mut result: Value = serde_json::from_slice(&resp.body)
        .map_err(|e| DmError::payment(format!("Invalid JSON response: {e}")))?;

    // Handle refunds
    if let Some(refund_info) = refund::parse_refund(&result) {
        match refund::process_refund(auth.wallet_api(), &refund_info).await {
            Ok(_) => tracing::info!("x402 refund internalized: {} sats", refund_info.satoshis),
            Err(e) => tracing::warn!("Failed to internalize x402 refund: {e}"),
        }
    }

    // Inject payment metadata
    if let Some(obj) = result.as_object_mut() {
        if let Some(txid) = &resp.payment_txid {
            obj.insert("payment_txid".to_string(), json!(txid));
        }
        if let Some(sats) = resp.sats_paid {
            obj.insert("sats_paid".to_string(), json!(sats));
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tool registration
// ---------------------------------------------------------------------------

/// Create the 3 x402 tool definitions (discovery + generic call).
///
/// `registry_url` is captured at registration time and used for `discover_services`.
/// `rate_limiter` is optional — when provided, `x402_call` acquires a token before each request.
pub fn all_x402_tools(wallet_url: String, registry_url: String) -> Vec<ToolDef> {
    all_x402_tools_with_rate_limiter(wallet_url, registry_url, None)
}

/// Create x402 tools with an optional rate limiter for server mode.
pub fn all_x402_tools_with_rate_limiter(
    wallet_url: String,
    registry_url: String,
    rate_limiter: Option<Arc<RateLimiterRegistry>>,
) -> Vec<ToolDef> {
    let url = Arc::new(wallet_url);
    let reg_url = Arc::new(registry_url);
    let discovered: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let failures: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let manifests: ManifestCache = Arc::new(Mutex::new(HashMap::new()));
    let tips_shown: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    vec![
        // discover_services
        ToolDef {
            name: "discover_services".to_string(),
            description: "List available x402 paid services from the registry. FREE.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Optional filter by capability category"
                    }
                }
            }),
            execute: {
                let u = Arc::clone(&url);
                let r = Arc::clone(&reg_url);
                Box::new(move |params| {
                    let wallet_url = u.as_ref().clone();
                    let registry = r.as_ref().clone();
                    Box::pin(discovery::discover_services_impl(params, wallet_url, registry))
                })
            },
            category: "x402".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        // discover_endpoints
        ToolDef {
            name: "discover_endpoints".to_string(),
            description: "Get detailed endpoint info for an x402 service — pricing, input schemas, delivery mode. FREE. Use after discover_services to learn a specific agent's API before calling x402_call. Also resets circuit breaker for this provider.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "Agent name (e.g. 'banana') or full URL"
                    }
                },
                "required": ["agent"]
            }),
            execute: {
                let u = Arc::clone(&url);
                let f = Arc::clone(&failures);
                Box::new(move |params| {
                    let wallet_url = u.as_ref().clone();
                    let fails = Arc::clone(&f);
                    Box::pin(discovery::discover_endpoints_impl(params, wallet_url, Some(fails)))
                })
            },
            category: "x402".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
        // x402_call
        ToolDef {
            name: "x402_call".to_string(),
            description: "Call any x402 service endpoint with automatic auth + payment. IMPORTANT: For POST requests, you MUST include the endpoint's required fields inside the 'parameters' object. Example: x402_call({\"service\": \"banana/generate\", \"method\": \"POST\", \"parameters\": {\"prompt\": \"a cat\", \"resolution\": \"1K\"}})".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "service": {
                        "type": "string",
                        "description": "Service endpoint (e.g. 'banana/generate') or full URL"
                    },
                    "method": {
                        "type": "string",
                        "description": "HTTP method (default: POST)",
                        "enum": ["GET", "POST"],
                        "default": "POST"
                    },
                    "parameters": {
                        "type": "object",
                        "description": "REQUIRED for POST: the endpoint's request body fields. Put all required fields here (e.g. {\"prompt\": \"...\", \"resolution\": \"1K\"} for banana, {\"fileSize\": 100, \"retentionPeriod\": 180} for nanostore). Without this, the request will fail."
                    }
                },
                "required": ["service"]
            }),
            execute: {
                let u = Arc::clone(&url);
                let d = Arc::clone(&discovered);
                let f = Arc::clone(&failures);
                let m = Arc::clone(&manifests);
                let t = Arc::clone(&tips_shown);
                let rl = rate_limiter.clone();
                Box::new(move |params| {
                    let wallet_url = u.as_ref().clone();
                    let disc = Arc::clone(&d);
                    let fails = Arc::clone(&f);
                    let mans = Arc::clone(&m);
                    let tips = Arc::clone(&t);
                    let rate_lim = rl.clone();
                    Box::pin(async move {
                        // Acquire rate limit token before x402 call
                        if let Some(ref rl) = rate_lim {
                            // Extract service URL for rate limiting from params
                            let service = params.get("service")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if !service.is_empty() {
                                // Resolve to full URL for rate limiting key
                                let url = crate::x402::registry::resolve(service, None)
                                    .await
                                    .unwrap_or_else(|_| service.to_string());
                                if let Err(e) = rl.acquire(&url).await {
                                    return format!("Rate limit error: {e}");
                                }
                            }
                        }
                        call::x402_call_impl(params, wallet_url, disc, fails, mans, tips).await
                    })
                })
            },
            category: "x402".to_string(),
            cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
        },
    ]
}

/// Create recipe tool definitions for common x402 services.
pub fn all_x402_recipe_tools(wallet_url: String) -> Vec<ToolDef> {
    all_x402_recipe_tools_with_rate_limiter(wallet_url, None)
}

/// Create recipe tools with an optional rate limiter for server mode.
pub fn all_x402_recipe_tools_with_rate_limiter(
    wallet_url: String,
    rate_limiter: Option<Arc<RateLimiterRegistry>>,
) -> Vec<ToolDef> {
    let url = Arc::new(wallet_url);

    vec![
        ToolDef {
            name: "generate_image".to_string(),
            description: "Generate an AI image. Returns the image URL. Cost: ~$0.19. Takes 1-3 minutes.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Text description of the image to generate"
                    },
                    "resolution": {
                        "type": "string",
                        "description": "Image resolution: 1K, 2K, or 4K",
                        "default": "1K"
                    },
                    "aspect_ratio": {
                        "type": "string",
                        "description": "Aspect ratio (e.g. '1:1', '16:9', '9:16')",
                        "default": "1:1"
                    }
                },
                "required": ["prompt"]
            }),
            execute: {
                let u = Arc::clone(&url);
                let rl = rate_limiter.clone();
                Box::new(move |params| {
                    let wallet_url = u.as_ref().clone();
                    let rate_lim = rl.clone();
                    Box::pin(async move {
                        if let Some(ref rl) = rate_lim {
                            let url = crate::x402::registry::resolve("banana/generate", None)
                                .await
                                .unwrap_or_else(|_| "https://nano-banana-pro.x402agency.com/generate".to_string());
                            if let Err(e) = rl.acquire(&url).await {
                                return format!("Rate limit error: {e}");
                            }
                        }
                        recipe::generate_image_impl(params, wallet_url).await
                    })
                })
            },
            category: "x402".to_string(),
            cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("Generate images via x402 (~$0.19)".to_string()),
        },
        ToolDef {
            name: "upload_to_nanostore".to_string(),
            description: "Upload content or a file to permanent decentralized storage. Returns the public URL. Cost: ~730 sats/MB/year.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The content to upload (text or base64-encoded binary). Required if file_path is not provided."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Path to a file to upload. Resolves from the project root. Use this instead of content for files on disk."
                    },
                    "retention_minutes": {
                        "type": "integer",
                        "description": "How long to store the content in minutes (default: 525600 = 1 year, minimum: 180)",
                        "default": 525600
                    },
                    "content_type": {
                        "type": "string",
                        "description": "MIME type for the upload (e.g. 'text/html', 'application/json', 'image/png'). Auto-detected from file extension or content when omitted."
                    }
                }
            }),
            execute: {
                let u = Arc::clone(&url);
                let rl = rate_limiter.clone();
                Box::new(move |params| {
                    let wallet_url = u.as_ref().clone();
                    let rate_lim = rl.clone();
                    Box::pin(async move {
                        if let Some(ref rl) = rate_lim {
                            let url = crate::x402::registry::resolve("nanostore/upload", None)
                                .await
                                .unwrap_or_else(|_| "https://nanostore.babbage.systems/upload".to_string());
                            if let Err(e) = rl.acquire(&url).await {
                                return format!("Rate limit error: {e}");
                            }
                        }
                        recipe::upload_to_nanostore_impl(params, wallet_url).await
                    })
                })
            },
            category: "x402".to_string(),
            cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("Upload files to NanoStore (~730 sats/MB/yr)".to_string()),
        },
        // search_twitter removed in Phase 2.1
    ]
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_x402_tools_creates_three_tools() {
        let tools = all_x402_tools(
            "http://localhost:3322".into(),
            crate::x402::registry::DEFAULT_REGISTRY_URL.into(),
        );
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"discover_services"));
        assert!(names.contains(&"discover_endpoints"));
        assert!(names.contains(&"x402_call"));
    }

    #[test]
    fn test_all_x402_tools_category() {
        let tools = all_x402_tools(
            "http://localhost:3322".into(),
            crate::x402::registry::DEFAULT_REGISTRY_URL.into(),
        );
        for tool in &tools {
            assert_eq!(
                tool.category, "x402",
                "tool {} has wrong category",
                tool.name
            );
        }
    }

    #[test]
    fn test_all_x402_recipe_tools_creates_two_tools() {
        let tools = all_x402_recipe_tools("http://localhost:3322".into());
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"generate_image"));
        assert!(names.contains(&"upload_to_nanostore"));
    }

    #[test]
    fn test_recipe_tools_category() {
        let tools = all_x402_recipe_tools("http://localhost:3322".into());
        for tool in &tools {
            assert_eq!(
                tool.category, "x402",
                "tool {} has wrong category",
                tool.name
            );
        }
    }
}
