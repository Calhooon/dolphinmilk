//! Tests for ModelCapabilities struct and discovery-driven context limits (#155).
//!
//! Verifies:
//! - ModelCapabilities::from_manifest() computes max_input_tokens correctly
//! - ModelCapabilities::from_hardcoded() matches model_input_limit() + model_output_limit()
//! - parse_model_capabilities() parses mock manifests correctly
//! - Resolution chain: discovery takes precedence over hardcoded

use std::collections::HashMap;

use dolphin_milk::think::{model_input_limit, model_output_limit, ModelCapabilities};
use dolphin_milk::x402::discovery::{parse_model_capabilities, ServiceManifest};
use serde_json::json;

// =============================================================================
// ModelCapabilities::from_manifest tests
// =============================================================================

#[test]
fn test_from_manifest_basic() {
    let cap = ModelCapabilities::from_manifest("gpt-5-mini", 400_000, 128_000, true, true);
    assert_eq!(cap.model, "gpt-5-mini");
    assert_eq!(cap.context_window, 400_000);
    assert_eq!(cap.max_output_tokens, 128_000);
    assert_eq!(cap.max_input_tokens, 272_000); // 400K - 128K
    assert!(cap.supports_tools);
    assert!(cap.supports_vision);
}

#[test]
fn test_from_manifest_computes_input_tokens() {
    let cap = ModelCapabilities::from_manifest("test-model", 200_000, 64_000, false, false);
    assert_eq!(cap.max_input_tokens, 136_000); // 200K - 64K
    assert!(!cap.supports_tools);
    assert!(!cap.supports_vision);
}

#[test]
fn test_from_manifest_saturating_sub() {
    // Edge case: output > context (shouldn't happen, but saturating_sub handles it)
    let cap = ModelCapabilities::from_manifest("weird", 100, 200, true, false);
    assert_eq!(cap.max_input_tokens, 0);
}

#[test]
fn test_from_manifest_zero_context() {
    let cap = ModelCapabilities::from_manifest("zero", 0, 0, true, true);
    assert_eq!(cap.context_window, 0);
    assert_eq!(cap.max_output_tokens, 0);
    assert_eq!(cap.max_input_tokens, 0);
}

// =============================================================================
// ModelCapabilities::from_hardcoded tests
// =============================================================================

#[test]
fn test_from_hardcoded_gpt5() {
    let cap = ModelCapabilities::from_hardcoded("gpt-5-mini");
    assert_eq!(cap.max_input_tokens, model_input_limit("gpt-5-mini"));
    assert_eq!(cap.max_output_tokens, model_output_limit("gpt-5-mini"));
    assert_eq!(
        cap.context_window,
        model_input_limit("gpt-5-mini") + model_output_limit("gpt-5-mini")
    );
    assert!(cap.supports_tools);
    assert!(cap.supports_vision);
}

#[test]
fn test_from_hardcoded_claude_sonnet() {
    let cap = ModelCapabilities::from_hardcoded("claude-sonnet-4-6");
    assert_eq!(cap.max_input_tokens, 936_000);
    assert_eq!(cap.max_output_tokens, 64_000);
    assert_eq!(cap.context_window, 1_000_000);
}

#[test]
fn test_from_hardcoded_claude_opus() {
    let cap = ModelCapabilities::from_hardcoded("claude-opus-4-6");
    assert_eq!(cap.max_input_tokens, 872_000);
    assert_eq!(cap.max_output_tokens, 128_000);
    assert_eq!(cap.context_window, 1_000_000);
}

#[test]
fn test_from_hardcoded_o4_mini() {
    let cap = ModelCapabilities::from_hardcoded("o4-mini");
    assert_eq!(cap.max_input_tokens, 100_000);
    assert_eq!(cap.max_output_tokens, 100_000);
    assert_eq!(cap.context_window, 200_000);
}

#[test]
fn test_from_hardcoded_unknown_model() {
    let cap = ModelCapabilities::from_hardcoded("unknown-model-v99");
    assert_eq!(cap.max_input_tokens, 128_000);
    assert_eq!(cap.max_output_tokens, 16_384);
    assert_eq!(cap.context_window, 128_000 + 16_384);
}

#[test]
fn test_from_hardcoded_matches_functions() {
    // Verify from_hardcoded always matches the standalone functions
    for model in &[
        "gpt-5-mini",
        "gpt-5",
        "o4-mini",
        "o3",
        "claude-haiku-4-5",
        "claude-sonnet-4-6",
        "claude-opus-4-6",
        "unknown",
    ] {
        let cap = ModelCapabilities::from_hardcoded(model);
        assert_eq!(
            cap.max_input_tokens,
            model_input_limit(model),
            "input mismatch for {model}"
        );
        assert_eq!(
            cap.max_output_tokens,
            model_output_limit(model),
            "output mismatch for {model}"
        );
    }
}

// =============================================================================
// parse_model_capabilities tests (via bsv-x402-server discovery)
// =============================================================================

fn mock_manifest_with_models() -> ServiceManifest {
    serde_json::from_value(json!({
        "name": "test-provider",
        "pricing": {
            "models": [
                {
                    "model": "gpt-5-mini",
                    "context_window": 400000,
                    "max_output_tokens": 128000,
                    "capabilities": ["tools", "vision"]
                },
                {
                    "model": "claude-sonnet-4-6",
                    "context_window": 1000000,
                    "max_output_tokens": 64000,
                    "capabilities": ["tools"]
                }
            ]
        }
    }))
    .unwrap()
}

#[test]
fn test_parse_capabilities_from_manifest() {
    let manifest = mock_manifest_with_models();
    let caps = parse_model_capabilities(&manifest);
    assert_eq!(caps.len(), 2);

    let gpt5 = caps.get("gpt-5-mini").unwrap();
    assert_eq!(gpt5.context_window, 400_000);
    assert_eq!(gpt5.max_output_tokens, 128_000);
    assert!(gpt5.supports_tools);
    assert!(gpt5.supports_vision);

    let claude = caps.get("claude-sonnet-4-6").unwrap();
    assert_eq!(claude.context_window, 1_000_000);
    assert_eq!(claude.max_output_tokens, 64_000);
    assert!(claude.supports_tools);
    assert!(!claude.supports_vision);
}

#[test]
fn test_parse_capabilities_empty_pricing() {
    let manifest: ServiceManifest = serde_json::from_value(json!({
        "name": "empty"
    }))
    .unwrap();
    let caps = parse_model_capabilities(&manifest);
    assert!(caps.is_empty());
}

#[test]
fn test_parse_capabilities_no_models_array() {
    let manifest: ServiceManifest = serde_json::from_value(json!({
        "name": "no-models",
        "pricing": {"base": 500}
    }))
    .unwrap();
    let caps = parse_model_capabilities(&manifest);
    assert!(caps.is_empty());
}

#[test]
fn test_parse_capabilities_model_without_name() {
    let manifest: ServiceManifest = serde_json::from_value(json!({
        "name": "test",
        "pricing": {
            "models": [
                {"context_window": 100000, "max_output_tokens": 50000}
            ]
        }
    }))
    .unwrap();
    let caps = parse_model_capabilities(&manifest);
    // Model without a name should be skipped
    assert!(caps.is_empty());
}

// =============================================================================
// Resolution chain tests
// =============================================================================

#[test]
fn test_resolution_chain_discovery_overrides_hardcoded() {
    // Simulate discovered capabilities with different values than hardcoded
    let mut discovered: HashMap<String, ModelCapabilities> = HashMap::new();
    discovered.insert(
        "gpt-5-mini".to_string(),
        ModelCapabilities::from_manifest("gpt-5-mini", 500_000, 150_000, true, true),
    );

    let hardcoded = ModelCapabilities::from_hardcoded("gpt-5-mini");

    // Discovery should provide different (larger) values
    let discovered_cap = discovered.get("gpt-5-mini").unwrap();
    assert_eq!(discovered_cap.context_window, 500_000);
    assert_eq!(discovered_cap.max_input_tokens, 350_000);
    assert_eq!(discovered_cap.max_output_tokens, 150_000);

    // Hardcoded has the original values
    assert_eq!(hardcoded.context_window, 400_000);
    assert_eq!(hardcoded.max_input_tokens, 272_000);
    assert_eq!(hardcoded.max_output_tokens, 128_000);

    // Resolution: use discovered if available, otherwise hardcoded
    let resolved = discovered
        .get("gpt-5-mini")
        .cloned()
        .unwrap_or_else(|| ModelCapabilities::from_hardcoded("gpt-5-mini"));
    assert_eq!(resolved.context_window, 500_000);
    assert_eq!(resolved.max_input_tokens, 350_000);
}

#[test]
fn test_resolution_chain_falls_back_to_hardcoded() {
    let discovered: HashMap<String, ModelCapabilities> = HashMap::new();

    // Model not in discovered map -> falls back to hardcoded
    let resolved = discovered
        .get("gpt-5-mini")
        .cloned()
        .unwrap_or_else(|| ModelCapabilities::from_hardcoded("gpt-5-mini"));
    assert_eq!(resolved.max_input_tokens, 272_000); // hardcoded value
}

#[test]
fn test_config_caps_discovered_values() {
    // Config context_window (128K) < discovered input limit (350K) -> config wins
    let discovered = ModelCapabilities::from_manifest("gpt-5-mini", 500_000, 150_000, true, true);
    let config_window: usize = 128_000;
    let effective = config_window.min(discovered.max_input_tokens);
    assert_eq!(effective, 128_000);
}

#[test]
fn test_discovered_caps_small_config() {
    // Config context_window (999K) > discovered input limit (350K) -> discovered wins
    let discovered = ModelCapabilities::from_manifest("gpt-5-mini", 500_000, 150_000, true, true);
    let config_window: usize = 999_000;
    let effective = config_window.min(discovered.max_input_tokens);
    assert_eq!(effective, 350_000);
}

// =============================================================================
// Conversion from DiscoveredModelCapability to ModelCapabilities
// =============================================================================

#[test]
fn test_discovered_to_model_capabilities_conversion() {
    // Simulate the conversion done in app_state.rs
    let manifest = mock_manifest_with_models();
    let discovered = parse_model_capabilities(&manifest);

    let mut converted: HashMap<String, ModelCapabilities> = HashMap::new();
    for (name, cap) in &discovered {
        converted.insert(
            name.clone(),
            ModelCapabilities::from_manifest(
                name,
                cap.context_window,
                cap.max_output_tokens,
                cap.supports_tools,
                cap.supports_vision,
            ),
        );
    }

    assert_eq!(converted.len(), 2);
    let gpt5 = converted.get("gpt-5-mini").unwrap();
    assert_eq!(gpt5.max_input_tokens, 272_000); // 400K - 128K
    assert_eq!(gpt5.max_output_tokens, 128_000);
}
