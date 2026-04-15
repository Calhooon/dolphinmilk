//! Tests for overlay_lookup tool — registration, discoverability, and search.

use dolphin_milk::tools::overlay_tools::all_overlay_tools;
use dolphin_milk::tools::registry::{
    required_capability, weighted_search, ToolRegistry, ALWAYS_ON_TOOLS,
};

// ─── Registration ────────────────────────────────────────────────────────────

#[test]
fn overlay_tools_returns_one_tool() {
    let tools = all_overlay_tools("https://example.com".into());
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "overlay_lookup");
}

#[test]
fn overlay_lookup_is_deferred() {
    let tools = all_overlay_tools("https://example.com".into());
    assert!(tools[0].deferred, "overlay_lookup should be deferred");
}

#[test]
fn overlay_lookup_has_overlay_category() {
    let tools = all_overlay_tools("https://example.com".into());
    assert_eq!(tools[0].category, "overlay");
}

#[test]
fn overlay_lookup_not_always_on() {
    assert!(
        !ALWAYS_ON_TOOLS.contains(&"overlay_lookup"),
        "overlay_lookup should not be in ALWAYS_ON_TOOLS"
    );
}

#[test]
fn overlay_lookup_has_search_hint() {
    let tools = all_overlay_tools("https://example.com".into());
    assert!(
        tools[0].search_hint.is_some(),
        "overlay_lookup should have a search_hint for discoverability"
    );
}

// ─── Capability mapping ─────────────────────────────────────────────────────

#[test]
fn overlay_category_maps_to_tools_capability() {
    assert_eq!(required_capability("overlay"), "tools");
}

// ─── Registry integration ────────────────────────────────────────────────────

#[test]
fn overlay_lookup_registered_in_registry() {
    let mut registry = ToolRegistry::new();
    for tool in all_overlay_tools("https://example.com".into()) {
        registry.register(tool);
    }
    assert!(registry.has("overlay_lookup"));
    assert_eq!(registry.tool_count(), 1);
}

#[test]
fn overlay_lookup_appears_in_snapshots() {
    let mut registry = ToolRegistry::new();
    for tool in all_overlay_tools("https://example.com".into()) {
        registry.register(tool);
    }
    let snapshots = registry.all_tool_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].name, "overlay_lookup");
    assert!(snapshots[0].deferred);
    assert_eq!(snapshots[0].category, "overlay");
}

// ─── Search discoverability ──────────────────────────────────────────────────

#[test]
fn search_finds_overlay_by_name() {
    let mut registry = ToolRegistry::new();
    for tool in all_overlay_tools("https://example.com".into()) {
        registry.register(tool);
    }
    let snapshots = registry.all_tool_snapshots();
    let results = weighted_search(&snapshots, "overlay", 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "overlay_lookup");
}

#[test]
fn search_finds_overlay_by_agent_discovery() {
    let mut registry = ToolRegistry::new();
    for tool in all_overlay_tools("https://example.com".into()) {
        registry.register(tool);
    }
    let snapshots = registry.all_tool_snapshots();
    let results = weighted_search(&snapshots, "discover agents", 10);
    assert!(!results.is_empty());
    assert!(
        results.iter().any(|r| r.name == "overlay_lookup"),
        "overlay_lookup should be discoverable via 'discover agents'"
    );
}

#[test]
fn search_finds_overlay_by_lookup() {
    let mut registry = ToolRegistry::new();
    for tool in all_overlay_tools("https://example.com".into()) {
        registry.register(tool);
    }
    let snapshots = registry.all_tool_snapshots();
    let results = weighted_search(&snapshots, "lookup", 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "overlay_lookup");
}

// ─── Parameter schema ────────────────────────────────────────────────────────

#[test]
fn overlay_lookup_schema_has_required_fields() {
    let tools = all_overlay_tools("https://example.com".into());
    let params = &tools[0].parameters;
    let props = params.get("properties").expect("should have properties");
    assert!(
        props.get("service").is_some(),
        "should have 'service' param"
    );
    assert!(props.get("query").is_some(), "should have 'query' param");
    let required = params
        .get("required")
        .and_then(|v| v.as_array())
        .expect("should have 'required' array");
    let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(required_names.contains(&"service"));
}

// ─── Query normalization ────────────────────────────────────────────────────

use dolphin_milk::tools::overlay_tools::normalize_query;
use serde_json::json;

#[test]
fn normalize_query_none_defaults_to_find_all() {
    let q = normalize_query(None);
    assert_eq!(q, json!({"findAll": true}));
}

#[test]
fn normalize_query_null_defaults_to_find_all() {
    let null = json!(null);
    let q = normalize_query(Some(&null));
    assert_eq!(q, json!({"findAll": true}));
}

#[test]
fn normalize_query_empty_object_defaults_to_find_all() {
    let empty = json!({});
    let q = normalize_query(Some(&empty));
    assert_eq!(q, json!({"findAll": true}));
}

#[test]
fn normalize_query_valid_find_all_passes_through() {
    let valid = json!({"findAll": true});
    let q = normalize_query(Some(&valid));
    assert_eq!(q, json!({"findAll": true}));
}

#[test]
fn normalize_query_valid_find_by_capability_passes_through() {
    let valid = json!({"findByCapability": "web-scraping"});
    let q = normalize_query(Some(&valid));
    assert_eq!(q, json!({"findByCapability": "web-scraping"}));
}

#[test]
fn normalize_query_valid_find_by_identity_key_passes_through() {
    let valid = json!({"findByIdentityKey": "03abc123"});
    let q = normalize_query(Some(&valid));
    assert_eq!(q, json!({"findByIdentityKey": "03abc123"}));
}

#[test]
fn normalize_query_string_findall() {
    let s = json!("findAll");
    let q = normalize_query(Some(&s));
    assert_eq!(q, json!({"findAll": true}));
}

#[test]
fn normalize_query_string_capability() {
    let s = json!("web-scraping");
    let q = normalize_query(Some(&s));
    assert_eq!(q, json!({"findByCapability": "web-scraping"}));
}

#[test]
fn normalize_query_snake_case_find_all() {
    let obj = json!({"find_all": true});
    let q = normalize_query(Some(&obj));
    assert_eq!(q, json!({"findAll": true}));
}

#[test]
fn normalize_query_snake_case_capability() {
    let obj = json!({"capability": "analysis"});
    let q = normalize_query(Some(&obj));
    assert_eq!(q, json!({"findByCapability": "analysis"}));
}

#[test]
fn normalize_query_snake_case_identity_key() {
    let obj = json!({"identity_key": "03abc123"});
    let q = normalize_query(Some(&obj));
    assert_eq!(q, json!({"findByIdentityKey": "03abc123"}));
}

#[test]
fn normalize_query_unrecognized_keys_default_to_find_all() {
    let obj = json!({"garbage": "nonsense"});
    let q = normalize_query(Some(&obj));
    assert_eq!(q, json!({"findAll": true}));
}
