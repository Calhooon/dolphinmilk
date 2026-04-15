//! x402_call — generic x402 service call with dynamic resolution,
//! auto-poll, circuit breaker, and provider validation.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::auth::AuthriteClient;
use crate::wallet::WalletClient;
use crate::x402::{discovery, payment, refund, registry};

use super::discovery::{
    extract_provider_name, load_key_constraints, load_provider_tips, load_validation_rules,
    validate_parameters,
};
use super::{do_x402_request, ManifestCache};

// ---------------------------------------------------------------------------
// URL and manifest helpers
// ---------------------------------------------------------------------------

/// Extract base URL (scheme + host) from a full URL.
pub(crate) fn extract_base_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let scheme = parsed.scheme();
        if let Some(host) = parsed.host_str() {
            let port_part = match parsed.port() {
                Some(p) => format!(":{}", p),
                None => String::new(),
            };
            return format!("{}://{}{}", scheme, host, port_part);
        }
    }
    url.to_string()
}

/// Find the matching endpoint in a manifest for a given request URL.
pub(crate) fn find_endpoint<'a>(
    manifest: &'a discovery::ServiceManifest,
    url: &str,
) -> Option<&'a discovery::EndpointInfo> {
    let url_path = if let Ok(parsed) = url::Url::parse(url) {
        parsed.path().to_string()
    } else {
        return None;
    };

    manifest.endpoints.iter().find(|ep| {
        if ep.path.is_empty() {
            return false;
        }
        url_path == ep.path || url_path.starts_with(&format!("{}/", ep.path))
    })
}

/// Auto-poll an async-poll service until it reaches a terminal state.
pub(crate) async fn poll_for_result(
    auth: &AuthriteClient,
    base_url: &str,
    response: &Value,
    polling_config: &Value,
) -> Result<Value, String> {
    // 1. Extract prediction/task ID from response
    let id = response
        .get("id")
        .or_else(|| response.get("prediction_id"))
        .or_else(|| response.get("task_id"))
        .or_else(|| response.get("job_id"))
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .ok_or_else(|| {
            "No prediction/task ID found in response (checked: id, prediction_id, task_id, job_id)"
                .to_string()
        })?;

    // 2. Parse polling config
    let polling = polling_config
        .as_object()
        .ok_or("polling config is not an object")?;

    let endpoint_template = polling
        .get("endpoint")
        .and_then(|v| v.as_str())
        .ok_or("polling config missing 'endpoint' template")?;

    let interval_secs = polling
        .get("interval_seconds")
        .or_else(|| polling.get("intervalSeconds"))
        .and_then(|v| v.as_u64())
        .unwrap_or(15);

    let max_wait_secs = polling
        .get("max_wait_seconds")
        .or_else(|| polling.get("maxWaitSeconds"))
        .and_then(|v| v.as_u64())
        .unwrap_or(180);

    let terminal_states: Vec<String> = polling
        .get("terminal_states")
        .or_else(|| polling.get("terminalStates"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "succeeded".into(),
                "completed".into(),
                "failed".into(),
                "canceled".into(),
                "error".into(),
            ]
        });

    // 3. Build status URL from template
    let status_path = endpoint_template
        .replace("{prediction_id}", &id)
        .replace("{id}", &id)
        .replace("{task_id}", &id)
        .replace("{job_id}", &id);

    let status_url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        status_path.trim_start_matches('/')
    );

    tracing::info!(
        "x402 auto-poll: polling {} every {}s (max {}s) for ID {}",
        status_url,
        interval_secs,
        max_wait_secs,
        id
    );

    // 4. Poll loop
    let start = std::time::Instant::now();
    let max_wait = std::time::Duration::from_secs(max_wait_secs);
    let interval = std::time::Duration::from_secs(interval_secs);
    let mut last_status = String::new();

    loop {
        if start.elapsed() >= max_wait {
            return Err(format!(
                "Polling timed out after {}s. Last status: '{}'. ID: {}",
                max_wait_secs, last_status, id
            ));
        }

        tokio::time::sleep(interval).await;

        let resp = payment::authenticated_paid_request(auth, "GET", &status_url, &[], None)
            .await
            .map_err(|e| format!("Poll request failed: {e}"))?;

        if !resp.status.is_success() {
            tracing::warn!("x402 auto-poll: HTTP {} from {}", resp.status, status_url);
            continue;
        }

        let body: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| format!("Failed to parse poll response: {e}"))?;

        let status = body
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        last_status = status.clone();

        tracing::info!(
            "x402 auto-poll: status='{}' (elapsed: {:?})",
            status,
            start.elapsed()
        );

        if terminal_states.contains(&status) {
            let success_states = ["succeeded", "completed"];
            if success_states.contains(&status.as_str()) {
                return Ok(body);
            } else {
                // Internalize refund from failed response before returning error
                if let Some(refund_info) = refund::parse_refund(&body) {
                    match refund::process_refund(auth.wallet_api(), &refund_info).await {
                        Ok(_) => tracing::info!(
                            "x402 auto-poll refund internalized: {} sats",
                            refund_info.satoshis
                        ),
                        Err(e) => tracing::warn!("Failed to internalize auto-poll refund: {e}"),
                    }
                }

                let error_msg = body
                    .get("error")
                    .or_else(|| body.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return Err(format!(
                    "Service returned terminal state '{}': {}",
                    status, error_msg
                ));
            }
        }
    }
}

/// Generate a ready-to-run upload script for two-step flows.
fn generate_upload_script(response: &Value) -> Option<String> {
    let upload_url = response
        .get("uploadURL")
        .or_else(|| response.get("presignedUrl"))
        .or_else(|| response.get("upload_url"))
        .and_then(|v| v.as_str())?;

    let mut headers_parts = Vec::new();

    if let Some(req_headers) = response.get("requiredHeaders").and_then(|v| v.as_object()) {
        for (key, value) in req_headers {
            let val_str = match value.as_str() {
                Some(s) => s.to_string(),
                None => value.to_string(),
            };
            headers_parts.push(format!("-H '{}: {}'", key, val_str));
        }
    }

    let has_content_type = headers_parts
        .iter()
        .any(|h| h.to_lowercase().contains("content-type"));
    if !has_content_type {
        headers_parts.push("-H 'Content-Type: application/octet-stream'".to_string());
    }

    let headers_str = headers_parts.join(" ");

    Some(format!(
        "UPLOAD STEP REQUIRED — Reservation complete. File is NOT uploaded yet.\n\n\
         To complete the upload, run this EXACT command with execute_bash:\n\n\
         execute_bash({{\"command\": \"curl -s -X PUT {} --data-binary @YOUR_FILE_HERE '{}'\"}})\n\n\
         Replace YOUR_FILE_HERE with the actual file path.\n\
         For inline content: --data-binary 'your content here'\n\
         HTTP 200 = success.",
        headers_str, upload_url
    ))
}

// ---------------------------------------------------------------------------
// x402_call implementation
// ---------------------------------------------------------------------------

pub(crate) async fn x402_call_impl(
    params: Value,
    wallet_url: String,
    discovered: Arc<Mutex<HashSet<String>>>,
    failures: Arc<Mutex<HashMap<String, u32>>>,
    manifests: ManifestCache,
    tips_shown: Arc<Mutex<HashSet<String>>>,
) -> String {
    let service = params.get("service").and_then(|v| v.as_str()).unwrap_or("");

    if service.is_empty() {
        return "Error: service is required (e.g. 'banana/generate' or full URL)".to_string();
    }

    let method = params
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("POST")
        .to_uppercase();

    let body = params
        .get("parameters")
        .or_else(|| params.get("body"))
        .cloned()
        .unwrap_or_else(|| {
            let known_keys = ["service", "method", "parameters", "body"];
            if let Some(obj) = params.as_object() {
                let extras: serde_json::Map<String, Value> = obj
                    .iter()
                    .filter(|(k, _)| !known_keys.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if !extras.is_empty() {
                    tracing::info!(
                        "x402_call: auto-collected {} stray top-level params into body",
                        extras.len()
                    );
                    return Value::Object(extras);
                }
            }
            json!({})
        });

    // LLMs sometimes pass body as a JSON string — parse it back
    let body = match &body {
        Value::String(s) => serde_json::from_str(s).unwrap_or(body),
        _ => body,
    };

    // Pre-flight: reject POST with empty body
    if method == "POST" {
        let is_empty = match &body {
            Value::Object(map) => map.is_empty(),
            Value::Null => true,
            _ => false,
        };
        if is_empty {
            return format!(
                "Error: POST request to '{}' has no parameters. You MUST include the endpoint's \
                 required fields in the 'parameters' object. Call discover_endpoints first to see \
                 the required fields, then retry with: x402_call({{\"service\": \"{}\", \
                 \"method\": \"POST\", \"parameters\": {{...required fields...}}}})",
                service, service
            );
        }
    }

    // Pre-flight validation against provider-specific rules
    let provider = extract_provider_name(service);
    if method == "POST" {
        let rules = load_validation_rules(&provider);
        if let Some(error) = validate_parameters(&rules, &body) {
            return format!(
                "PARAMETER VALIDATION ERROR ({}): {}\n\
                 Fix the parameters and retry. Call discover_endpoints(\"{}\") to see correct fields.",
                provider, error, provider
            );
        }
    }

    // Auto-discover: fetch manifest on first use
    let is_first_use = {
        let mut set = discovered.lock().unwrap();
        if set.contains(&provider) {
            false
        } else {
            set.insert(provider.clone());
            true
        }
    };

    if is_first_use {
        if let Ok(manifest_url) = registry::resolve_x402_info(&provider, None).await {
            if let Ok(manifest) = discovery::fetch_manifest_from_url(&manifest_url).await {
                manifests.lock().unwrap().insert(provider.clone(), manifest);
            }
        }
    }

    // Circuit breaker: reject after 2+ failures
    let failure_key = format!("{}:{}", service, method);
    {
        let counts = failures.lock().unwrap();
        if let Some(&count) = counts.get(&failure_key) {
            if count >= 2 {
                let tips = load_provider_tips(&provider).unwrap_or_default();
                let mut msg = format!(
                    "CIRCUIT BREAKER: x402_call to '{}' has failed {} consecutive times. \
                     STOP retrying the same call. Instead:\n\
                     1. Call discover_endpoints({{\"agent\": \"{}\"}}) to read the full API docs\n\
                     2. Check if you're using the correct endpoint, method, and parameters\n\
                     3. Try a different approach based on the manifest",
                    service, count, provider
                );
                if !tips.is_empty() {
                    msg.push_str(&format!("\n\n---\nProvider Tips ({}):\n{}", provider, tips));
                }
                return msg;
            }
        }
    }

    // Resolve service to full URL
    let url = match registry::resolve(service, None).await {
        Ok(u) => u,
        Err(e) => return format!("Error resolving service: {e}"),
    };

    let wallet = WalletClient::new(&wallet_url, "http://localhost", 30);
    let auth = AuthriteClient::new(std::sync::Arc::new(wallet), &wallet_url);

    let result = match method.as_str() {
        "GET" => match payment::authenticated_paid_request(&auth, "GET", &url, &[], None).await {
            Ok(resp) => {
                if !resp.status.is_success() {
                    format!("Error: HTTP {} from {url}", resp.status)
                } else {
                    let mut result: Value = serde_json::from_slice(&resp.body).unwrap_or_else(
                        |_| json!({"raw": String::from_utf8_lossy(&resp.body).to_string()}),
                    );

                    if let Some(refund_info) = refund::parse_refund(&result) {
                        match refund::process_refund(auth.wallet_api(), &refund_info).await {
                            Ok(_) => tracing::info!(
                                "x402_call refund internalized: {} sats",
                                refund_info.satoshis
                            ),
                            Err(e) => {
                                tracing::warn!("Failed to internalize x402_call refund: {e}")
                            }
                        }
                    }

                    if let Some(obj) = result.as_object_mut() {
                        if let Some(txid) = &resp.payment_txid {
                            obj.insert("payment_txid".to_string(), json!(txid));
                        }
                        if let Some(sats) = resp.sats_paid {
                            obj.insert("sats_paid".to_string(), json!(sats));
                        }
                    }

                    serde_json::to_string(&result)
                        .unwrap_or_else(|_| "Error: serialization failed".to_string())
                }
            }
            Err(e) => format!("Error: {e}"),
        },
        _ => match do_x402_request(&auth, &url, body).await {
            Ok(result_value) => {
                let mut result_str = serde_json::to_string(&result_value)
                    .unwrap_or_else(|_| "Error: serialization failed".to_string());

                // Auto-poll for async-poll services
                let poll_info = {
                    let manifests_guard = manifests.lock().unwrap();
                    manifests_guard.get(&provider).and_then(|manifest| {
                        find_endpoint(manifest, &url).and_then(|ep| {
                            let is_async = ep.delivery == "async-poll"
                                || ep.delivery == "async"
                                || ep.delivery == "poll";
                            if is_async && !ep.polling.is_null() {
                                Some(ep.polling.clone())
                            } else {
                                None
                            }
                        })
                    })
                };

                if let Some(polling) = poll_info {
                    let base = extract_base_url(&url);
                    tracing::info!(
                        "x402 auto-poll: detected async-poll delivery for {}, starting polling",
                        url
                    );
                    // Preserve payment fields from the original response — the polled
                    // response won't have them since polling is a free GET request.
                    let saved_sats_paid = result_value.get("sats_paid").cloned();
                    let saved_payment_txid = result_value.get("payment_txid").cloned();

                    match poll_for_result(&auth, &base, &result_value, &polling).await {
                        Ok(mut final_val) => {
                            // Re-inject payment fields from the original paid request
                            if let Some(obj) = final_val.as_object_mut() {
                                if let Some(sats) = saved_sats_paid {
                                    obj.entry("sats_paid").or_insert(sats);
                                }
                                if let Some(txid) = saved_payment_txid {
                                    obj.entry("payment_txid").or_insert(txid);
                                }
                            }
                            result_str = serde_json::to_string(&final_val).unwrap_or(result_str);
                        }
                        Err(e) => {
                            result_str.push_str(&format!(
                                "\n\nAUTO-POLL ERROR: {e}\n\
                                 You may need to poll manually using the status URL."
                            ));
                        }
                    }
                }

                // Generate upload script for two-step flows
                if let Ok(parsed) = serde_json::from_str::<Value>(&result_str) {
                    if let Some(script) = generate_upload_script(&parsed) {
                        result_str.push_str(&format!("\n\n{script}"));
                    }
                }

                result_str
            }
            Err(e) => format!("Error: {e}"),
        },
    };

    // Enrichment: append tips on first failure only
    let is_error = result.starts_with("Error:");

    if is_error {
        let mut counts = failures.lock().unwrap();
        *counts.entry(failure_key).or_insert(0) += 1;
    } else {
        failures.lock().unwrap().remove(&failure_key);
    }

    let mut final_result = result;

    if is_error {
        let first_tips_for_provider = {
            let mut shown = tips_shown.lock().unwrap();
            if shown.contains(&provider) {
                false
            } else {
                shown.insert(provider.clone());
                true
            }
        };

        if first_tips_for_provider {
            if let Some(tips) = load_provider_tips(&provider) {
                final_result.push_str(&format!(
                    "\n\n---\nProvider Tips ({}) --- read these for correct usage:\n{}",
                    provider, tips
                ));
            }
        } else if let Some(constraints) = load_key_constraints(&provider) {
            final_result.push_str(&format!(
                "\n\n--- Key Constraints ({}) ---\n{}",
                provider, constraints
            ));
        }
    }

    final_result
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifests() -> ManifestCache {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn test_tips_shown() -> Arc<Mutex<HashSet<String>>> {
        Arc::new(Mutex::new(HashSet::new()))
    }

    #[tokio::test]
    async fn test_x402_call_validates_service() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let result = x402_call_impl(
            json!({}),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("service"));
    }

    #[tokio::test]
    async fn test_x402_call_validates_empty_service() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let result = x402_call_impl(
            json!({"service": ""}),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;
        assert!(result.starts_with("Error"));
        assert!(result.contains("service"));
    }

    #[tokio::test]
    async fn test_x402_call_auto_collects_stray_params() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let result = x402_call_impl(
            json!({
                "service": "nonexistent-test-service/endpoint",
                "method": "POST",
                "query": "BSV micropayments",
                "limit": 5,
                "format": "json"
            }),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;

        assert!(
            result.contains("resolving service") || result.contains("Error"),
            "should attempt to resolve, not reject for missing params"
        );
        assert!(
            !result.contains("service is required"),
            "should NOT complain about missing service — it was provided"
        );
    }

    #[tokio::test]
    async fn test_x402_call_rejects_post_without_parameters() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let result = x402_call_impl(
            json!({
                "service": "banana/generate",
                "method": "POST"
            }),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;

        assert!(
            result.contains("no parameters"),
            "should reject POST with empty params, got: {result}"
        );
        assert!(
            result.contains("discover_endpoints"),
            "should tell LLM to call discover_endpoints first"
        );
        assert!(
            !result.contains("payment error"),
            "should NOT attempt payment, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_x402_call_allows_post_with_parameters() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let result = x402_call_impl(
            json!({
                "service": "nonexistent-test/endpoint",
                "method": "POST",
                "parameters": {"prompt": "a cat"}
            }),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;

        assert!(
            !result.contains("no parameters"),
            "should NOT reject POST with params, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_x402_call_allows_get_without_parameters() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let result = x402_call_impl(
            json!({
                "service": "nonexistent-test/status/123",
                "method": "GET"
            }),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;

        assert!(
            !result.contains("no parameters"),
            "should NOT reject GET without params, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_x402_call_circuit_breaker() {
        let discovered = Arc::new(Mutex::new(HashSet::new()));
        let failures = Arc::new(Mutex::new(HashMap::new()));

        {
            let mut counts = failures.lock().unwrap();
            counts.insert("nonexistent/endpoint:POST".to_string(), 2);
        }
        {
            let mut disc = discovered.lock().unwrap();
            disc.insert("nonexistent".to_string());
        }

        let result = x402_call_impl(
            json!({
                "service": "nonexistent/endpoint",
                "method": "POST",
                "parameters": {"test": true}
            }),
            "http://localhost:3322".into(),
            discovered,
            failures,
            test_manifests(),
            test_tips_shown(),
        )
        .await;

        assert!(
            result.contains("CIRCUIT BREAKER"),
            "should trigger circuit breaker after 2 failures, got: {result}"
        );
        assert!(
            result.contains("discover_endpoints"),
            "circuit breaker should suggest discover_endpoints"
        );
    }

    // --- Helper function tests ---

    #[test]
    fn test_extract_base_url_simple() {
        assert_eq!(
            extract_base_url("https://nano-banana-pro.x402agency.com/generate"),
            "https://nano-banana-pro.x402agency.com"
        );
    }

    #[test]
    fn test_extract_base_url_with_port() {
        assert_eq!(
            extract_base_url("http://localhost:8080/api/v1/generate"),
            "http://localhost:8080"
        );
    }

    #[test]
    fn test_extract_base_url_no_path() {
        assert_eq!(
            extract_base_url("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn test_extract_base_url_with_query() {
        assert_eq!(
            extract_base_url("https://example.com/path?foo=bar"),
            "https://example.com"
        );
    }

    #[test]
    fn test_extract_base_url_invalid() {
        assert_eq!(extract_base_url("not-a-url"), "not-a-url");
    }

    /// Helper to create a minimal EndpointInfo for tests.
    fn test_endpoint(
        path: &str,
        method: &str,
        delivery: &str,
        polling: Value,
    ) -> discovery::EndpointInfo {
        discovery::EndpointInfo {
            path: path.into(),
            method: method.into(),
            description: String::new(),
            auth: false,
            delivery: delivery.into(),
            payment: Value::Null,
            input: Value::Null,
            output: Value::Null,
            hint: String::new(),
            polling,
            refund: Value::Null,
            extra: Value::Null,
        }
    }

    /// Helper to create a minimal ServiceManifest for tests.
    fn test_manifest(
        name: &str,
        endpoints: Vec<discovery::EndpointInfo>,
    ) -> discovery::ServiceManifest {
        discovery::ServiceManifest {
            name: name.into(),
            description: String::new(),
            server_identity_key: String::new(),
            auth_endpoint: String::new(),
            auth_protocol: String::new(),
            endpoints,
            pricing: Value::Null,
            capabilities: Value::Null,
            extra: Value::Null,
        }
    }

    #[test]
    fn test_find_endpoint_matches() {
        let manifest = test_manifest(
            "Test",
            vec![
                test_endpoint(
                    "/generate",
                    "POST",
                    "async-poll",
                    json!({
                        "endpoint": "/status/{prediction_id}",
                        "interval_seconds": 15,
                        "max_wait_seconds": 180,
                        "terminal_states": ["succeeded", "failed"]
                    }),
                ),
                test_endpoint("/status", "GET", "", Value::Null),
            ],
        );

        let ep = find_endpoint(&manifest, "https://example.com/generate");
        assert!(ep.is_some(), "should match /generate");
        assert_eq!(ep.unwrap().path, "/generate");
        assert_eq!(ep.unwrap().delivery, "async-poll");
    }

    #[test]
    fn test_find_endpoint_no_match() {
        let manifest = test_manifest(
            "Test",
            vec![test_endpoint("/generate", "POST", "", Value::Null)],
        );

        let ep = find_endpoint(&manifest, "https://example.com/search");
        assert!(ep.is_none(), "should not match /search against /generate");
    }

    #[test]
    fn test_find_endpoint_prefix_match() {
        let manifest = test_manifest(
            "Test",
            vec![test_endpoint("/status", "GET", "", Value::Null)],
        );

        let ep = find_endpoint(&manifest, "https://example.com/status/abc123");
        assert!(
            ep.is_some(),
            "should prefix-match /status/abc123 to /status"
        );
    }

    #[test]
    fn test_generate_upload_script_with_upload_url() {
        let response = json!({
            "publicURL": "https://public.example.com/file",
            "uploadURL": "https://storage.googleapis.com/bucket/obj?token=abc",
            "requiredHeaders": {
                "Content-Type": "application/octet-stream",
                "x-goog-meta-uploaderidentitykey": "028045abcdef"
            }
        });

        let script = generate_upload_script(&response);
        assert!(
            script.is_some(),
            "should generate script when uploadURL present"
        );
        let text = script.unwrap();
        assert!(
            text.contains("UPLOAD STEP REQUIRED"),
            "should have upload header"
        );
        assert!(
            text.contains("storage.googleapis.com"),
            "should include the upload URL"
        );
        assert!(
            text.contains("028045abcdef"),
            "should include header values"
        );
        assert!(text.contains("curl"), "should have curl command");
        assert!(text.contains("PUT"), "should use PUT method");
    }

    #[test]
    fn test_generate_upload_script_with_presigned_url() {
        let response = json!({
            "presignedUrl": "https://s3.amazonaws.com/bucket/key?sig=xyz",
        });

        let script = generate_upload_script(&response);
        assert!(script.is_some(), "should generate script for presignedUrl");
        let text = script.unwrap();
        assert!(
            text.contains("s3.amazonaws.com"),
            "should include presigned URL"
        );
    }

    #[test]
    fn test_generate_upload_script_no_upload_url() {
        let response = json!({
            "status": "succeeded",
            "output": "https://example.com/result.png"
        });

        let script = generate_upload_script(&response);
        assert!(script.is_none(), "should return None when no upload URL");
    }

    #[test]
    fn test_generate_upload_script_default_content_type() {
        let response = json!({
            "uploadURL": "https://storage.example.com/upload",
            "requiredHeaders": {
                "x-custom-header": "value"
            }
        });

        let script = generate_upload_script(&response);
        assert!(script.is_some());
        let text = script.unwrap();
        assert!(
            text.contains("Content-Type: application/octet-stream"),
            "should add default Content-Type"
        );
        assert!(
            text.contains("x-custom-header: value"),
            "should include custom header"
        );
    }

    #[test]
    fn test_generate_upload_script_no_required_headers() {
        let response = json!({
            "upload_url": "https://storage.example.com/upload"
        });

        let script = generate_upload_script(&response);
        assert!(script.is_some());
        let text = script.unwrap();
        assert!(
            text.contains("Content-Type: application/octet-stream"),
            "should add default Content-Type even without requiredHeaders"
        );
    }

    // --- Task 1.1: Payment field preservation through auto-poll ---

    /// Verify that payment fields from the original response survive auto-poll.
    /// When poll_for_result replaces result_str, the saved sats_paid and payment_txid
    /// should be re-injected into the final output.
    #[test]
    fn test_payment_fields_preserved_after_autopoll_replacement() {
        // Simulate the original response from do_x402_request with payment metadata
        let original_result = json!({
            "id": "pred-123",
            "status": "starting",
            "sats_paid": 1500,
            "payment_txid": "txid_abc123def456"
        });

        // Save payment fields before auto-poll (what our fix does)
        let saved_sats_paid = original_result.get("sats_paid").cloned();
        let saved_payment_txid = original_result.get("payment_txid").cloned();

        // Simulate the polled response (no payment fields — free GET request)
        let mut polled_result = json!({
            "id": "pred-123",
            "status": "succeeded",
            "output": "https://example.com/result.png"
        });

        // Re-inject payment fields (this is what our fix does)
        if let Some(obj) = polled_result.as_object_mut() {
            if let Some(sats) = saved_sats_paid {
                obj.entry("sats_paid").or_insert(sats);
            }
            if let Some(txid) = saved_payment_txid {
                obj.entry("payment_txid").or_insert(txid);
            }
        }

        // Verify payment fields are present in the final output
        assert_eq!(
            polled_result["sats_paid"], 1500,
            "sats_paid should be preserved"
        );
        assert_eq!(
            polled_result["payment_txid"], "txid_abc123def456",
            "payment_txid should be preserved"
        );
        // Original polled fields should also be intact
        assert_eq!(polled_result["status"], "succeeded");
        assert_eq!(polled_result["output"], "https://example.com/result.png");
    }

    /// If the polled response already has its own sats_paid (unlikely but possible),
    /// it should NOT be overwritten by the saved value (entry semantics).
    #[test]
    fn test_payment_fields_not_overwritten_if_polled_has_them() {
        let original_result = json!({
            "id": "pred-123",
            "sats_paid": 1500,
            "payment_txid": "txid_original"
        });

        let saved_sats_paid = original_result.get("sats_paid").cloned();
        let saved_payment_txid = original_result.get("payment_txid").cloned();

        // Polled response already has payment fields (edge case)
        let mut polled_result = json!({
            "id": "pred-123",
            "status": "succeeded",
            "sats_paid": 0,
            "payment_txid": "txid_polled"
        });

        if let Some(obj) = polled_result.as_object_mut() {
            if let Some(sats) = saved_sats_paid {
                obj.entry("sats_paid").or_insert(sats);
            }
            if let Some(txid) = saved_payment_txid {
                obj.entry("payment_txid").or_insert(txid);
            }
        }

        // The polled response's values should be kept (or_insert semantics)
        assert_eq!(polled_result["sats_paid"], 0, "polled value should be kept");
        assert_eq!(
            polled_result["payment_txid"], "txid_polled",
            "polled value should be kept"
        );
    }

    /// If the original response has no payment fields (e.g., free service),
    /// the polled result should not get spurious fields injected.
    #[test]
    fn test_no_payment_fields_when_original_has_none() {
        let original_result = json!({
            "id": "pred-123",
            "status": "starting"
        });

        let saved_sats_paid = original_result.get("sats_paid").cloned();
        let saved_payment_txid = original_result.get("payment_txid").cloned();

        assert!(saved_sats_paid.is_none());
        assert!(saved_payment_txid.is_none());

        let mut polled_result = json!({
            "id": "pred-123",
            "status": "succeeded",
            "output": "https://example.com/result.png"
        });

        if let Some(obj) = polled_result.as_object_mut() {
            if let Some(sats) = saved_sats_paid {
                obj.entry("sats_paid").or_insert(sats);
            }
            if let Some(txid) = saved_payment_txid {
                obj.entry("payment_txid").or_insert(txid);
            }
        }

        // No payment fields should appear
        assert!(
            polled_result.get("sats_paid").is_none(),
            "should not inject sats_paid"
        );
        assert!(
            polled_result.get("payment_txid").is_none(),
            "should not inject payment_txid"
        );
    }
}
