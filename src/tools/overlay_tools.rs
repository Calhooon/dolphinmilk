//! Overlay lookup tool — query BSV overlay lookup services.
//!
//! General-purpose tool for querying any overlay lookup service (ls_agent,
//! ls_ship, ls_slap, etc.). Returns structured records when possible,
//! raw response otherwise.

use serde_json::{json, Value};
use std::sync::Arc;

use crate::overlay::lookup::{overlay_lookup, parse_agent_records, AgentRecord};
use crate::tools::registry::ToolDef;

/// Create the overlay_lookup tool. Discoverable, NOT always-on.
pub fn all_overlay_tools(overlay_url: String) -> Vec<ToolDef> {
    let url = Arc::new(overlay_url);

    let url_clone = Arc::clone(&url);
    let tool = ToolDef::new(
        "overlay_lookup",
        "Query a BSV overlay lookup service to discover agents or data. \
         Use service='ls_agent' to find other AI agents.\n\n\
         REQUIRED: BOTH `service` AND `query` must be provided. The query MUST be \
         a JSON object containing EXACTLY ONE of these keys:\n\
         - {\"findAll\": true} — list every registered agent\n\
         - {\"findByCapability\": \"web-scraping\"} — agents with a specific capability\n\
         - {\"findByIdentityKey\": \"03abc...\"} — agent by 66-char hex public key\n\
         - {\"findByCertifier\": \"03def...\"} — agents whose registration is signed by this certifier key (use this to filter by trust root)\n\
         - {\"findByEndpoint\": \"https://...\"} — agent at a specific URL\n\n\
         Example: overlay_lookup({\"service\": \"ls_agent\", \"query\": {\"findByCertifier\": \"03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0\"}})\n\
         Example: overlay_lookup({\"service\": \"ls_agent\", \"query\": {\"findAll\": true}})\n\n\
         An empty query or a call without the `query` field is REJECTED — you must \
         decide which lookup to perform and supply the correct key. Do NOT call this \
         tool with just `{\"service\": \"ls_agent\"}` — that is incomplete.",
        json!({
            "type": "object",
            "properties": {
                "service": {
                    "type": "string",
                    "description": "Lookup service name. Use 'ls_agent' to discover agents.",
                    "enum": ["ls_agent", "ls_ship", "ls_slap"]
                },
                "query": {
                    "type": "object",
                    "description": "REQUIRED query object. MUST contain exactly one key: findAll (bool), findByCapability (string), findByIdentityKey (66-hex string), findByCertifier (66-hex string), or findByEndpoint (string). Example: {\"findByCertifier\": \"03ef...\"}",
                    "minProperties": 1
                },
                "overlay_url": {
                    "type": "string",
                    "description": "Override the default overlay URL (optional, rarely needed)"
                }
            },
            "required": ["service", "query"]
        }),
        Box::new(move |params| {
            let url = Arc::clone(&url_clone);
            Box::pin(async move { overlay_lookup_impl(params, &url).await })
        }),
        "overlay",
    )
    .with_deferred(true)
    .with_search_hint("Query BSV overlay for agent discovery and data lookup");

    vec![tool]
}

/// Recognized query keys for the overlay lookup service.
const VALID_QUERY_KEYS: &[&str] = &[
    "findAll",
    "findByCapability",
    "findByIdentityKey",
    "findByCertifier",
    "findByEndpoint",
];

/// Normalize a query parameter into a valid overlay query.
///
/// Handles common LLM mistakes:
/// - Missing query → defaults to `{"findAll": true}`
/// - Empty object `{}` → defaults to `{"findAll": true}`
/// - String value (e.g. `"findAll"`) → converts to `{"findAll": true}`
/// - Query with unrecognized keys → tries to extract a valid sub-query
pub fn normalize_query(raw: Option<&Value>) -> Value {
    match raw {
        None => json!({"findAll": true}),
        Some(Value::Null) => json!({"findAll": true}),
        Some(Value::String(s)) => {
            // LLM might pass "findAll" as a string
            let trimmed = s.trim();
            if trimmed.eq_ignore_ascii_case("findall") || trimmed == "*" || trimmed.is_empty() {
                return json!({"findAll": true});
            }
            // Might be a capability name — treat as findByCapability
            if !trimmed.starts_with('{') {
                return json!({"findByCapability": trimmed});
            }
            // Try parsing as JSON
            match serde_json::from_str::<Value>(trimmed) {
                Ok(v) if v.is_object() => normalize_query(Some(&v)),
                _ => json!({"findAll": true}),
            }
        }
        Some(obj @ Value::Object(_)) => {
            let map = obj.as_object().unwrap();
            // Empty object → findAll
            if map.is_empty() {
                return json!({"findAll": true});
            }
            // Check if any valid key exists
            for key in VALID_QUERY_KEYS {
                if map.contains_key(*key) {
                    return obj.clone();
                }
            }
            // No valid key found — check for common misspellings/patterns
            // "find_all", "findall", "all" → findAll
            if map.contains_key("find_all")
                || map.contains_key("findall")
                || map.contains_key("all")
            {
                return json!({"findAll": true});
            }
            // "capability" or "capabilities" → findByCapability
            if let Some(v) = map
                .get("capability")
                .or_else(|| map.get("capabilities"))
                .and_then(|v| v.as_str())
            {
                return json!({"findByCapability": v});
            }
            // "identity_key" or "identityKey" → findByIdentityKey
            if let Some(v) = map
                .get("identity_key")
                .or_else(|| map.get("identityKey"))
                .and_then(|v| v.as_str())
            {
                return json!({"findByIdentityKey": v});
            }
            // Give up, default to findAll
            json!({"findAll": true})
        }
        _ => json!({"findAll": true}),
    }
}

async fn overlay_lookup_impl(params: Value, default_url: &str) -> String {
    let service = match params.get("service").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "ls_agent".to_string(), // Default to agent discovery
    };

    // Defense in depth (EPIC #329): the JSON schema marks `query` as required,
    // but providers' tool dispatch can still drop it. Hard-reject any call that
    // omits the query or supplies an empty object — that lets the LLM see the
    // error message and retry with an explicit query, instead of silently
    // running an unintended findAll over the entire overlay.
    let raw_query = params.get("query");
    let query_explicit = match raw_query {
        Some(Value::Object(m)) if !m.is_empty() => true,
        Some(Value::String(s)) if !s.trim().is_empty() => true,
        _ => false,
    };
    if !query_explicit {
        return "Error: overlay_lookup requires an explicit `query` object. \
                Examples: {\"findByCertifier\": \"03ef...\"}, \
                {\"findByCapability\": \"llm\"}, {\"findAll\": true}. \
                Do NOT call overlay_lookup with just `service` — pick a query type."
            .to_string();
    }

    // Normalize the query — be forgiving about format
    let query = normalize_query(raw_query);

    let url = params
        .get("overlay_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(default_url);

    let response: Value = match overlay_lookup(url, &service, &query).await {
        Ok(r) => r,
        Err(e) => return format!("Error: {e}"),
    };

    // For ls_agent, parse BEEF outputs into structured AgentRecords
    if service == "ls_agent" {
        let records: Vec<AgentRecord> = parse_agent_records(&response);
        let output_count = response
            .get("outputs")
            .and_then(|o: &Value| o.as_array())
            .map(|a: &Vec<Value>| a.len())
            .unwrap_or(0);

        let result = json!({
            "service": service,
            "total_outputs": output_count,
            "agents": records.iter().map(|r| json!({
                "identity_key": r.identity_key,
                "certifier_key": r.certifier_key,
                "name": r.name,
                "capabilities": r.capabilities,
            })).collect::<Vec<_>>(),
            "parsed_count": records.len(),
        });

        return serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| "Error: serialization failed".to_string());
    }

    // For other services, return the raw response
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|_| "Error: serialization failed".to_string())
}
