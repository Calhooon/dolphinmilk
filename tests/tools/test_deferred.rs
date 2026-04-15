//! Tests for deferred tool loading: weighted scoring, select: syntax, DeferralMode,
//! hint derivation, MCP auto-defer, ToolSnapshot, and CompactionBoundary persistence.

use dolphin_milk::tools::registry::{
    derive_hint, select_tools, weighted_search, DeferralMode, ToolDef, ToolRegistry, ToolSnapshot,
};
use serde_json::json;

// ─── Helper ───────────────────────────────────────────────────────────────────

fn make_snapshot(name: &str, desc: &str, cat: &str, hint: &str, deferred: bool) -> ToolSnapshot {
    ToolSnapshot {
        name: name.to_string(),
        description: desc.to_string(),
        category: cat.to_string(),
        deferred,
        always_load: false,
        hint: hint.to_string(),
    }
}

fn sample_snapshots() -> Vec<ToolSnapshot> {
    vec![
        make_snapshot(
            "execute_bash",
            "Run a shell command with timeout and output capping",
            "sandbox",
            "Run a shell command with timeout and output capping",
            false,
        ),
        make_snapshot(
            "file_read",
            "Read a file from the workspace, with optional line range",
            "sandbox",
            "Read a file from the workspace, with optional line range",
            false,
        ),
        make_snapshot(
            "browser",
            "Headless Chrome browser automation via CDP. 8 actions: navigate, snapshot, click, type, select, evaluate, screenshot, close.",
            "browser",
            "Headless Chrome browser automation (navigate, click, type, screenshot)",
            true,
        ),
        make_snapshot(
            "discover_agent",
            "Find agents by identity key or certificate attributes via BRC-56 peer discovery",
            "discovery",
            "Find agents by identity key or attributes (BRC-56)",
            true,
        ),
        make_snapshot(
            "generate_image",
            "Generate images via x402 provider. Auto-polls until completion. Returns URL.",
            "x402",
            "Generate images via x402 (~$0.19)",
            true,
        ),
        make_snapshot(
            "wallet_balance",
            "Check the current wallet balance in satoshis and BSV",
            "wallet",
            "Check the current wallet balance in satoshis and BSV",
            false,
        ),
        make_snapshot(
            "search_tools",
            "Search for available tools by keyword or load specific tools by name",
            "system",
            "Search for available tools by keyword or load specific tools by name",
            false,
        ),
        make_snapshot(
            "introspect",
            "Self-query proofs, costs, task history, and on-chain verification state",
            "introspect",
            "Self-query proofs, costs, task history, and on-chain state",
            true,
        ),
    ]
}

// ─── derive_hint ──────────────────────────────────────────────────────────────

#[test]
fn derive_hint_first_sentence() {
    let desc = "Run a shell command with timeout. Supports output capping and exit codes.";
    let hint = derive_hint(desc);
    assert_eq!(hint, "Run a shell command with timeout.");
}

#[test]
fn derive_hint_long_no_period() {
    let desc = "This is a very long description without any sentence boundaries that goes on and on and on and on and on forever";
    let hint = derive_hint(desc);
    assert!(hint.len() <= 84); // 80 chars + "..."
    assert!(hint.ends_with("..."));
}

#[test]
fn derive_hint_short_description() {
    let desc = "Check balance";
    let hint = derive_hint(desc);
    assert_eq!(hint, "Check balance");
}

#[test]
fn derive_hint_empty() {
    assert_eq!(derive_hint(""), "");
}

// ─── weighted_search ──────────────────────────────────────────────────────────

#[test]
fn weighted_search_exact_name_match() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "browser", 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "browser");
    assert_eq!(results[0].score, 10); // exact name match
    assert!(results[0]
        .match_reasons
        .iter()
        .any(|r| r.starts_with("name_exact")));
}

#[test]
fn weighted_search_name_substring() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "bash", 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "execute_bash");
    assert_eq!(results[0].score, 5); // name substring match
}

#[test]
fn weighted_search_hint_match() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "screenshot", 10);
    assert!(!results.is_empty());
    // browser tool has "screenshot" in its hint
    assert_eq!(results[0].name, "browser");
    assert_eq!(results[0].score, 4); // hint match
}

#[test]
fn weighted_search_description_match() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "auto-polls", 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "generate_image");
    assert_eq!(results[0].score, 2); // description match
}

#[test]
fn weighted_search_category_match() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "discovery", 10);
    // discover_agent has category "discovery" (1pt) + "discovery" in description (2pt) + hint (4pt)
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "discover_agent");
}

#[test]
fn weighted_search_multiple_terms_additive() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "browser automation", 10);
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "browser");
    // "browser" exact (10) + "automation" in hint (4) = 14
    assert!(results[0].score >= 14);
}

#[test]
fn weighted_search_no_match() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "xyznonexistent", 10);
    assert!(results.is_empty());
}

#[test]
fn weighted_search_empty_query() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "", 10);
    assert!(results.is_empty());
}

#[test]
fn weighted_search_limit_respected() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "a", 2);
    assert!(results.len() <= 2);
}

#[test]
fn weighted_search_required_term() {
    let snaps = sample_snapshots();
    // +browser requires "browser" to appear somewhere
    let results = weighted_search(&snaps, "+browser screenshot", 10);
    assert!(!results.is_empty());
    // Only browser tool should match (has both "browser" and "screenshot")
    assert!(results.iter().all(|r| {
        let combined = format!("{} {} {} {}", r.name, r.description, r.category, "");
        combined.to_lowercase().contains("browser")
    }));
}

#[test]
fn weighted_search_required_term_no_match() {
    let snaps = sample_snapshots();
    // Require a term that no tool has
    let results = weighted_search(&snaps, "+nonexistent browser", 10);
    assert!(results.is_empty());
}

#[test]
fn weighted_search_results_sorted_by_score() {
    let snaps = sample_snapshots();
    let results = weighted_search(&snaps, "a e i o", 10);
    for w in results.windows(2) {
        assert!(w[0].score >= w[1].score);
    }
}

// ─── select_tools ─────────────────────────────────────────────────────────────

#[test]
fn select_single_tool() {
    let snaps = sample_snapshots();
    let results = select_tools(&snaps, "browser");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "browser");
    assert_eq!(results[0].score, 10);
}

#[test]
fn select_multiple_tools() {
    let snaps = sample_snapshots();
    let results = select_tools(&snaps, "browser,generate_image,file_read");
    assert_eq!(results.len(), 3);
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"browser"));
    assert!(names.contains(&"generate_image"));
    assert!(names.contains(&"file_read"));
}

#[test]
fn select_nonexistent_tool() {
    let snaps = sample_snapshots();
    let results = select_tools(&snaps, "nonexistent_tool");
    assert!(results.is_empty());
}

#[test]
fn select_mixed_existing_and_missing() {
    let snaps = sample_snapshots();
    let results = select_tools(&snaps, "browser,nonexistent,file_read");
    assert_eq!(results.len(), 2); // only existing tools returned
}

#[test]
fn select_with_whitespace() {
    let snaps = sample_snapshots();
    let results = select_tools(&snaps, "browser , file_read");
    assert_eq!(results.len(), 2);
}

// ─── DeferralMode ─────────────────────────────────────────────────────────────

#[test]
fn deferral_mode_default_is_auto() {
    let mode = DeferralMode::default();
    assert_eq!(
        mode,
        DeferralMode::Auto {
            threshold_pct: 50.0
        }
    );
}

#[test]
fn deferral_mode_always() {
    let mode = DeferralMode::Always;
    assert_eq!(mode, DeferralMode::Always);
}

#[test]
fn deferral_mode_never() {
    let mode = DeferralMode::Never;
    assert_eq!(mode, DeferralMode::Never);
}

// ─── ToolDef fields ───────────────────────────────────────────────────────────

#[test]
fn tooldef_new_has_defaults() {
    let def = ToolDef::new(
        "test_tool",
        "A test tool",
        json!({"type": "object"}),
        Box::new(|_| Box::pin(async { "ok".to_string() })),
        "test",
    );
    assert!(!def.deferred);
    assert!(!def.always_load);
    assert!(def.search_hint.is_none());
    assert!(def.cleanup.is_none());
}

#[test]
fn tooldef_builder_chain() {
    let def = ToolDef::new(
        "test_tool",
        "A test tool",
        json!({"type": "object"}),
        Box::new(|_| Box::pin(async { "ok".to_string() })),
        "test",
    )
    .with_deferred(true)
    .with_always_load(false)
    .with_search_hint("Short hint");

    assert!(def.deferred);
    assert!(!def.always_load);
    assert_eq!(def.search_hint.as_deref(), Some("Short hint"));
}

// ─── ToolRegistry deferred integration ────────────────────────────────────────

#[test]
fn registry_all_tool_snapshots() {
    let mut registry = ToolRegistry::new();
    registry.register(ToolDef {
        name: "my_tool".to_string(),
        description: "A tool for testing".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("Test hint".to_string()),
    });

    let snapshots = registry.all_tool_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].name, "my_tool");
    assert!(snapshots[0].deferred);
    assert_eq!(snapshots[0].hint, "Test hint");
}

#[test]
fn registry_snapshot_derives_hint_when_none() {
    let mut registry = ToolRegistry::new();
    registry.register(ToolDef {
        name: "auto_hint_tool".to_string(),
        description: "This tool automatically derives a hint. It does many things.".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: None,
    });

    let snapshots = registry.all_tool_snapshots();
    assert_eq!(snapshots[0].hint, "This tool automatically derives a hint.");
}

#[test]
fn registry_list_prompt_tools_includes_always_load() {
    let mut registry = ToolRegistry::new();

    // A deferred tool with always_load = true should still appear in prompt tools
    registry.register(ToolDef {
        name: "special_tool".to_string(),
        description: "Always loaded".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: true,
        search_hint: None,
    });

    // A regular deferred tool should NOT appear in prompt tools
    registry.register(ToolDef {
        name: "deferred_tool".to_string(),
        description: "Hidden from prompt".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("A deferred tool".to_string()),
    });

    let prompt_tools = registry.list_prompt_tools();
    let names: Vec<&str> = prompt_tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"special_tool"));
    assert!(!names.contains(&"deferred_tool"));
}

#[test]
fn registry_list_deferred_tools() {
    let mut registry = ToolRegistry::new();

    registry.register(ToolDef {
        name: "always_on".to_string(),
        description: "Not deferred".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: false,
        always_load: false,
        search_hint: None,
    });

    registry.register(ToolDef {
        name: "deferred_one".to_string(),
        description: "A deferred tool".to_string(),
        parameters: json!({"type": "object"}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: Some("Hint one".to_string()),
    });

    let deferred = registry.list_deferred_tools();
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].name, "deferred_one");
    assert!(deferred[0].deferred);
    assert_eq!(deferred[0].hint.as_deref(), Some("Hint one"));
}

#[test]
fn registry_tool_schema_returns_openai_format() {
    let mut registry = ToolRegistry::new();
    registry.register(ToolDef {
        name: "schema_tool".to_string(),
        description: "For schema test".to_string(),
        parameters: json!({"type": "object", "properties": {"x": {"type": "string"}}}),
        execute: Box::new(|_| Box::pin(async { "ok".to_string() })),
        category: "test".to_string(),
        cleanup: None,
        deferred: true,
        always_load: false,
        search_hint: None,
    });

    let schema = registry.tool_schema("schema_tool").unwrap();
    assert_eq!(schema["type"], "function");
    assert_eq!(schema["function"]["name"], "schema_tool");
    assert_eq!(schema["function"]["description"], "For schema test");
    assert!(schema["function"]["parameters"]["properties"]["x"].is_object());
}

#[test]
fn registry_tool_schema_nonexistent() {
    let registry = ToolRegistry::new();
    assert!(registry.tool_schema("nonexistent").is_none());
}

// ─── Config ───────────────────────────────────────────────────────────────────

#[test]
fn tool_deferral_config_default() {
    let config = dolphin_milk::config::ToolDeferralConfig::default();
    assert_eq!(config.deferral_mode, "auto");
    assert!((config.auto_threshold_pct - 50.0).abs() < f64::EPSILON);
}

// ─── ToolSnapshot clone ───────────────────────────────────────────────────────

#[test]
fn tool_snapshot_clone_preserves_fields() {
    let snap = make_snapshot("test", "desc", "cat", "hint", true);
    let cloned = snap.clone();
    assert_eq!(snap.name, cloned.name);
    assert_eq!(snap.deferred, cloned.deferred);
    assert_eq!(snap.hint, cloned.hint);
}
