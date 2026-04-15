//! x402 service manifest parser — fetches `/.well-known/x402-info` from agents.
//!
//! Each x402 agent publishes a manifest describing its endpoints, pricing,
//! capabilities, and auth requirements. This module fetches and parses those
//! manifests, and formats summaries for LLM consumption.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::X402Error;

/// Service manifest from `/.well-known/x402-info`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub server_identity_key: String,
    #[serde(default)]
    pub auth_endpoint: String,
    #[serde(default)]
    pub auth_protocol: String,
    #[serde(default)]
    pub endpoints: Vec<EndpointInfo>,
    #[serde(default)]
    pub pricing: Value,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(flatten)]
    pub extra: Value,
}

/// A single endpoint within a service manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: bool,
    #[serde(default)]
    pub delivery: String,
    #[serde(default)]
    pub payment: Value,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub polling: Value,
    #[serde(default)]
    pub refund: Value,
    #[serde(flatten)]
    pub extra: Value,
}

impl EndpointInfo {
    /// Returns true if the endpoint requires payment (non-null, non-false payment field).
    pub fn has_payment(&self) -> bool {
        !self.payment.is_null() && self.payment != Value::Bool(false)
    }
}

/// Fetch a service manifest from `{base_url}/.well-known/x402-info`.
///
/// This is a plain HTTP GET — no auth or payment needed.
pub async fn fetch_manifest(base_url: &str) -> Result<ServiceManifest, X402Error> {
    let url = format!("{}/.well-known/x402-info", base_url.trim_end_matches('/'));
    fetch_manifest_from_url(&url).await
}

/// Fetch a service manifest from an explicit URL.
///
/// Used when the manifest URL is known directly (e.g. from the registry's
/// `x402_info` field for third-party agents with hosted manifests).
pub async fn fetch_manifest_from_url(url: &str) -> Result<ServiceManifest, X402Error> {
    tracing::info!("x402 discovery: fetching manifest from {url}");
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| X402Error::payment(format!("Failed to fetch manifest from {url}: {e}")))?;

    if !resp.status().is_success() {
        return Err(X402Error::payment(format!(
            "Manifest fetch failed: HTTP {} from {url}",
            resp.status()
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| X402Error::payment(format!("Failed to read manifest response: {e}")))?;

    let manifest: ServiceManifest = serde_json::from_str(&body)
        .map_err(|e| X402Error::payment(format!("Failed to parse manifest JSON: {e}")))?;

    tracing::info!(
        "x402 discovery: {} — {} endpoints",
        manifest.name,
        manifest.endpoints.len()
    );

    Ok(manifest)
}

/// Format a service manifest as a human/LLM-readable summary.
///
/// Shows full input schemas with types, required markers, defaults, enums,
/// and descriptions for every property — not just field names.
pub fn format_manifest_summary(manifest: &ServiceManifest) -> String {
    use crate::schema::format_input_schema;

    let mut out = String::new();

    out.push_str(&format!("Service: {}\n", manifest.name));
    if !manifest.description.is_empty() {
        out.push_str(&format!("Description: {}\n", manifest.description));
    }
    if !manifest.server_identity_key.is_empty() {
        out.push_str(&format!(
            "Identity: {}...{}\n",
            &manifest.server_identity_key[..manifest.server_identity_key.len().min(8)],
            &manifest.server_identity_key[manifest.server_identity_key.len().saturating_sub(4)..]
        ));
    }
    if !manifest.auth_protocol.is_empty() {
        out.push_str(&format!("Auth: {}\n", manifest.auth_protocol));
    }

    if manifest.endpoints.is_empty() {
        out.push_str("\nNo endpoints listed.\n");
        return out;
    }

    out.push_str(&format!("\nEndpoints ({}):\n", manifest.endpoints.len()));

    for ep in &manifest.endpoints {
        let method = if ep.method.is_empty() {
            "POST"
        } else {
            &ep.method
        };

        let mut flags = Vec::new();
        if ep.auth {
            flags.push("auth");
        }
        if ep.has_payment() {
            flags.push("paid");
        }
        let flag_str = if flags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", flags.join(", "))
        };

        out.push_str(&format!("  {method} {}{flag_str}\n", ep.path));

        if !ep.description.is_empty() {
            out.push_str(&format!("    {}\n", ep.description));
        }
        if !ep.hint.is_empty() {
            out.push_str(&format!("    Hint: {}\n", ep.hint));
        }

        // Show full input schema with types, required, defaults, enums
        let input_summary = format_input_schema(&ep.input);
        if !input_summary.is_empty() {
            out.push_str(&input_summary);
            out.push('\n');
        }

        // Show payment cost info
        if ep.has_payment() {
            if let Some(sats) = ep.payment.get("satoshis").and_then(|v| v.as_u64()) {
                out.push_str(&format!("    Cost: {} sats\n", sats));
            } else if ep
                .payment
                .get("dynamic")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                out.push_str("    Cost: dynamic (varies by request)\n");
            }
        }

        // Show delivery mode
        if !ep.delivery.is_empty() {
            out.push_str(&format!("    Delivery: {}\n", ep.delivery));
        }

        // Show timing info if available
        if let Some(timing) = ep.extra.get("timing").and_then(|t| t.as_object()) {
            let parts: Vec<String> = timing
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v.as_str().unwrap_or(&v.to_string())))
                .collect();
            if !parts.is_empty() {
                out.push_str(&format!("    Timing: {}\n", parts.join(", ")));
            }
        }

        // Show polling config if available (critical for async services)
        if !ep.polling.is_null() {
            if let Some(polling_obj) = ep.polling.as_object() {
                out.push_str("    Polling:\n");
                if let Some(endpoint) = polling_obj.get("endpoint").and_then(|v| v.as_str()) {
                    out.push_str(&format!("      endpoint: {}\n", endpoint));
                }
                if let Some(method) = polling_obj.get("method").and_then(|v| v.as_str()) {
                    out.push_str(&format!("      method: {}\n", method));
                }
                if let Some(interval) = polling_obj
                    .get("interval_seconds")
                    .or(polling_obj.get("intervalSeconds"))
                {
                    out.push_str(&format!("      interval: {}s\n", interval));
                }
                if let Some(max_wait) = polling_obj
                    .get("max_wait_seconds")
                    .or(polling_obj.get("maxWaitSeconds"))
                {
                    out.push_str(&format!("      max_wait: {}s\n", max_wait));
                }
                if let Some(states) = polling_obj
                    .get("terminal_states")
                    .or(polling_obj.get("terminalStates"))
                {
                    if let Some(arr) = states.as_array() {
                        let s: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                        out.push_str(&format!("      terminal_states: [{}]\n", s.join(", ")));
                    }
                }
                if let Some(scoped) = polling_obj
                    .get("identity_scoped")
                    .or(polling_obj.get("identityScoped"))
                {
                    if scoped.as_bool().unwrap_or(false) {
                        out.push_str(
                            "      identity_scoped: true (only the paying identity can poll)\n",
                        );
                    }
                }
                if let Some(note) = polling_obj.get("note").and_then(|v| v.as_str()) {
                    out.push_str(&format!("      note: {}\n", note));
                }
            }
        }

        // Show payment tiers if available (actual satoshi costs)
        if ep.has_payment() {
            if let Some(tiers) = ep.payment.get("tiers").and_then(|t| t.as_object()) {
                out.push_str("    Payment tiers:\n");
                for (tier_name, tier_info) in tiers {
                    let sats = tier_info
                        .get("example_sats")
                        .or(tier_info.get("satoshis"))
                        .and_then(|v| v.as_u64());
                    let usd = tier_info
                        .get("usd")
                        .or(tier_info.get("usd_total"))
                        .and_then(|v| v.as_f64());
                    match (sats, usd) {
                        (Some(s), Some(u)) => {
                            out.push_str(&format!("      {}: {} sats (~${:.4})\n", tier_name, s, u))
                        }
                        (Some(s), None) => {
                            out.push_str(&format!("      {}: {} sats\n", tier_name, s))
                        }
                        (None, Some(u)) => {
                            out.push_str(&format!("      {}: ~${:.4}\n", tier_name, u))
                        }
                        (None, None) => {
                            out.push_str(&format!("      {}: (see pricing)\n", tier_name))
                        }
                    }
                }
            }
        }

        // Show output schema if available
        let output_obj = if let Some(schema) = ep.output.get("schema").and_then(|s| s.as_object()) {
            Some(schema)
        } else {
            ep.output.as_object()
        };
        if let Some(obj) = output_obj {
            if !obj.is_empty() {
                out.push_str("    Output:\n");
                for (key, val) in obj.iter().take(15) {
                    let type_str = if let Some(s) = val.as_str() {
                        s.to_string()
                    } else if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
                        t.to_string()
                    } else if val.is_object() {
                        // Nested object — show a compact summary
                        let nested: Vec<String> = val
                            .as_object()
                            .unwrap()
                            .iter()
                            .take(5)
                            .map(|(k, v)| {
                                format!("{}: {}", k, v.as_str().unwrap_or(&v.to_string()))
                            })
                            .collect();
                        format!("{{{}}}", nested.join(", "))
                    } else {
                        val.to_string()
                    };
                    out.push_str(&format!("      - {}: {}\n", key, type_str));
                }
            }
        }

        // Show refund policy if available
        if !ep.refund.is_null() {
            if let Some(supported) = ep.refund.get("supported").and_then(|v| v.as_bool()) {
                if supported {
                    let delivery = ep
                        .refund
                        .get("delivery")
                        .and_then(|v| v.as_str())
                        .unwrap_or("auto");
                    out.push_str(&format!("    Refund: supported ({})\n", delivery));
                }
            }
        }
    }

    out
}

/// Parsed model capabilities from an x402-info manifest.
///
/// This is a standalone struct that doesn't depend on `think.rs`,
/// keeping the x402 crate independent. The consuming crate converts
/// these into its own `ModelCapabilities` type.
#[derive(Debug, Clone)]
pub struct DiscoveredModelCapability {
    pub model: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

/// Parse model capabilities from a service manifest's pricing data.
///
/// Looks for `pricing.models` array in the manifest. Each entry should have:
/// - `model` (string): model name
/// - `context_window` (number): total context window size
/// - `max_output_tokens` (number): maximum output tokens
/// - `capabilities` (array of strings): e.g. `["tools", "vision"]`
///
/// Returns a map of model_name -> DiscoveredModelCapability.
pub fn parse_model_capabilities(
    manifest: &ServiceManifest,
) -> std::collections::HashMap<String, DiscoveredModelCapability> {
    let mut caps = std::collections::HashMap::new();

    // Parse from manifest.pricing.models array if present
    if let Some(models) = manifest.pricing.get("models").and_then(|v| v.as_array()) {
        for model_info in models {
            if let Some(model_name) = model_info.get("model").and_then(|v| v.as_str()) {
                let context_window = model_info
                    .get("context_window")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(128_000) as usize;
                let max_output = model_info
                    .get("max_output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(16_384) as usize;
                let capabilities_arr = model_info.get("capabilities").and_then(|v| v.as_array());
                let supports_tools = capabilities_arr
                    .map(|arr| arr.iter().any(|c| c.as_str() == Some("tools")))
                    .unwrap_or(true);
                let supports_vision = capabilities_arr
                    .map(|arr| arr.iter().any(|c| c.as_str() == Some("vision")))
                    .unwrap_or(false);

                caps.insert(
                    model_name.to_string(),
                    DiscoveredModelCapability {
                        model: model_name.to_string(),
                        context_window,
                        max_output_tokens: max_output,
                        supports_tools,
                        supports_vision,
                    },
                );
            }
        }
    }

    caps
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_manifest() -> ServiceManifest {
        ServiceManifest {
            name: "Test Service".into(),
            description: "A test service".into(),
            server_identity_key: "0280451234567890abcdef".into(),
            auth_endpoint: "https://test.com/.well-known/auth".into(),
            auth_protocol: "BRC-31".into(),
            endpoints: vec![EndpointInfo {
                path: "/generate".into(),
                method: "POST".into(),
                description: "Generate something".into(),
                auth: true,
                delivery: "async-poll".into(),
                payment: json!({"dynamic": true}),
                input: json!({"properties": {"prompt": {"type": "string"}}}),
                output: json!({}),
                hint: "Use short prompts".into(),
                polling: Value::Null,
                refund: Value::Null,
                extra: Value::Null,
            }],
            pricing: json!({"base": 500}),
            capabilities: json!(["image"]),
            extra: Value::Null,
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let m = sample_manifest();
        let json = serde_json::to_string(&m).unwrap();
        let parsed: ServiceManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "Test Service");
        assert_eq!(parsed.endpoints.len(), 1);
        assert_eq!(parsed.endpoints[0].path, "/generate");
    }

    #[test]
    fn test_tolerant_missing_fields() {
        let json = r#"{"name":"minimal"}"#;
        let m: ServiceManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "minimal");
        assert!(m.endpoints.is_empty());
        assert!(m.description.is_empty());
        assert!(m.server_identity_key.is_empty());
    }

    #[test]
    fn test_full_endpoint_parse() {
        let json = r#"{
            "path": "/search",
            "method": "POST",
            "description": "Search X",
            "auth": true,
            "delivery": "direct",
            "payment": {"dynamic": true},
            "input": {"properties": {"query": {"type": "string"}}},
            "output": {"type": "object"},
            "hint": "Use keywords",
            "polling": null,
            "refund": {"policy": "auto"}
        }"#;
        let ep: EndpointInfo = serde_json::from_str(json).unwrap();
        assert_eq!(ep.path, "/search");
        assert!(ep.auth);
        assert!(ep.has_payment());
        assert_eq!(ep.hint, "Use keywords");
    }

    #[test]
    fn test_minimal_endpoint_parse() {
        let json = r#"{"path": "/health"}"#;
        let ep: EndpointInfo = serde_json::from_str(json).unwrap();
        assert_eq!(ep.path, "/health");
        assert!(!ep.auth);
        assert!(!ep.has_payment());
        assert!(ep.method.is_empty());
    }

    #[test]
    fn test_has_payment_null() {
        let ep = EndpointInfo {
            payment: Value::Null,
            ..Default::default()
        };
        assert!(!ep.has_payment());
    }

    #[test]
    fn test_has_payment_false() {
        let ep = EndpointInfo {
            payment: Value::Bool(false),
            ..Default::default()
        };
        assert!(!ep.has_payment());
    }

    #[test]
    fn test_has_payment_object() {
        let ep = EndpointInfo {
            payment: json!({"dynamic": true}),
            ..Default::default()
        };
        assert!(ep.has_payment());
    }

    #[test]
    fn test_format_manifest_summary() {
        let m = sample_manifest();
        let summary = format_manifest_summary(&m);
        assert!(summary.contains("Test Service"));
        assert!(summary.contains("POST /generate"));
        assert!(summary.contains("[auth, paid]"));
        assert!(summary.contains("prompt"));
        assert!(summary.contains("Hint: Use short prompts"));
    }

    #[test]
    fn test_format_empty_manifest_summary() {
        let m = ServiceManifest {
            name: "Empty".into(),
            endpoints: vec![],
            ..Default::default()
        };
        let summary = format_manifest_summary(&m);
        assert!(summary.contains("Empty"));
        assert!(summary.contains("No endpoints listed"));
    }

    #[test]
    fn test_endpoint_extra_fields_preserved() {
        let json = r#"{"path":"/test","custom":"value"}"#;
        let ep: EndpointInfo = serde_json::from_str(json).unwrap();
        assert_eq!(ep.extra.get("custom").unwrap(), "value");
    }

    impl Default for EndpointInfo {
        fn default() -> Self {
            Self {
                path: String::new(),
                method: String::new(),
                description: String::new(),
                auth: false,
                delivery: String::new(),
                payment: Value::Null,
                input: Value::Null,
                output: Value::Null,
                hint: String::new(),
                polling: Value::Null,
                refund: Value::Null,
                extra: Value::Null,
            }
        }
    }

    impl Default for ServiceManifest {
        fn default() -> Self {
            Self {
                name: String::new(),
                description: String::new(),
                server_identity_key: String::new(),
                auth_endpoint: String::new(),
                auth_protocol: String::new(),
                endpoints: vec![],
                pricing: Value::Null,
                capabilities: Value::Null,
                extra: Value::Null,
            }
        }
    }
}
