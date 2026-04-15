//! Tests for skill usage telemetry — CI.12.

use std::collections::HashMap;

use dolphin_milk::skills::{
    Skill, SkillActivationStats, SkillRegistry, SkillState, SkillTelemetry,
};
use dolphin_milk::transcript::Transcript;

fn make_skill(name: &str, auto_activate: bool) -> Skill {
    Skill {
        name: name.to_string(),
        description: format!("{name} skill"),
        auto_activate,
        tools: vec![],
        instructions: String::new(),
        source: "test".to_string(),
        state: SkillState::default(),
        dependencies: Default::default(),
        paths: vec![],
        context: "normal".to_string(),
        model: None,
        arguments: vec![],
        allowed_tools: vec![],
        when_to_use: None,
        version: None,
        user_invocable: false,
        argument_hint: None,
    }
}

// -- SkillTelemetry struct --

#[test]
fn test_skill_telemetry_default_empty() {
    let t = SkillTelemetry::default();
    assert!(t.activations.is_empty());
}

// -- record_activation --

#[test]
fn test_record_activation_increments_count() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("x402", true));

    reg.record_activation("x402", "auto");
    assert_eq!(reg.telemetry().activations["x402"].count, 1);

    reg.record_activation("x402", "auto");
    assert_eq!(reg.telemetry().activations["x402"].count, 2);
}

#[test]
fn test_record_activation_updates_timestamp() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("wallet", true));

    reg.record_activation("wallet", "auto");
    let ts = reg.telemetry().activations["wallet"]
        .last_activated
        .as_ref()
        .unwrap()
        .clone();
    assert!(!ts.is_empty());

    // Ensure timestamp updates on subsequent activation
    std::thread::sleep(std::time::Duration::from_millis(10));
    reg.record_activation("wallet", "explicit");
    let ts2 = reg.telemetry().activations["wallet"]
        .last_activated
        .as_ref()
        .unwrap();
    assert!(ts2 >= &ts);
}

#[test]
fn test_record_activation_tracks_context() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("messaging", true));

    reg.record_activation("messaging", "auto");
    reg.record_activation("messaging", "explicit");

    let contexts = &reg.telemetry().activations["messaging"].contexts;
    assert_eq!(contexts, &["auto", "explicit"]);
}

#[test]
fn test_multiple_skills_tracked_independently() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("x402", true));
    reg.register(make_skill("wallet", true));

    reg.record_activation("x402", "auto");
    reg.record_activation("x402", "auto");
    reg.record_activation("wallet", "auto");

    let tel = reg.telemetry();
    assert_eq!(tel.activations["x402"].count, 2);
    assert_eq!(tel.activations["wallet"].count, 1);
}

#[test]
fn test_activation_stats_serialization() {
    let stats = SkillActivationStats {
        count: 5,
        last_activated: Some("2026-03-22T12:00:00Z".to_string()),
        contexts: vec!["auto".to_string(), "explicit".to_string()],
    };
    let json = serde_json::to_string(&stats).unwrap();
    let roundtrip: SkillActivationStats = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtrip.count, 5);
    assert_eq!(
        roundtrip.last_activated.as_deref(),
        Some("2026-03-22T12:00:00Z")
    );
    assert_eq!(roundtrip.contexts.len(), 2);
}

#[test]
fn test_unknown_skill_still_tracked() {
    let mut reg = SkillRegistry::new();
    // No skills registered, but we can still track activations
    reg.record_activation("nonexistent", "explicit");
    assert_eq!(reg.telemetry().activations["nonexistent"].count, 1);
}

#[test]
fn test_activation_contexts_capped() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("x402", true));

    // Add 15 activations — only the last 10 contexts should be retained
    for i in 0..15 {
        reg.record_activation("x402", &format!("ctx-{i}"));
    }

    let contexts = &reg.telemetry().activations["x402"].contexts;
    assert_eq!(contexts.len(), 10);
    // First 5 should have been evicted (ctx-0 through ctx-4)
    assert_eq!(contexts[0], "ctx-5");
    assert_eq!(contexts[9], "ctx-14");
}

// -- Transcript event --

#[test]
fn test_transcript_skill_activated_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut transcript = Transcript::new(path);

    let event = transcript.record_skill_activated("x402", "auto");
    assert_eq!(event.event_type, "skill_activated");
    assert_eq!(
        event.data.get("skill_name").and_then(|v| v.as_str()),
        Some("x402")
    );
    assert_eq!(
        event.data.get("context").and_then(|v| v.as_str()),
        Some("auto")
    );
}

// -- Telemetry report format --

#[test]
fn test_skill_telemetry_report_format() {
    let mut telemetry = SkillTelemetry::default();
    telemetry.activations.insert(
        "x402".to_string(),
        SkillActivationStats {
            count: 42,
            last_activated: Some("2026-03-22T12:00:00Z".to_string()),
            contexts: vec!["auto".to_string()],
        },
    );

    let json = serde_json::to_value(&telemetry).unwrap();
    let activations = json["activations"].as_object().unwrap();
    assert!(activations.contains_key("x402"));
    assert_eq!(activations["x402"]["count"], 42);
}

// -- Auto-activated skill recording --

#[test]
fn test_auto_activated_skill_recorded() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("x402", true));
    reg.register(make_skill("browser", false));

    // Simulate what the runner does: record auto-activated skills
    let active: Vec<String> = reg
        .auto_activated()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    for name in &active {
        reg.record_activation(name, "auto");
    }

    let tel = reg.telemetry();
    assert_eq!(tel.activations.get("x402").unwrap().count, 1);
    // browser is not auto-activated, so should not appear
    assert!(!tel.activations.contains_key("browser"));
}

#[test]
fn test_explicit_activation_recorded() {
    let mut reg = SkillRegistry::new();
    reg.register(make_skill("browser", false));

    reg.record_activation("browser", "explicit");
    let tel = reg.telemetry();
    assert_eq!(tel.activations["browser"].count, 1);
    assert_eq!(tel.activations["browser"].contexts, vec!["explicit"]);
}

// -- SkillTelemetry merge (simulating global accumulator) --

#[test]
fn test_skill_telemetry_merge() {
    // Simulate merging per-task telemetry into global accumulator
    let mut global = SkillTelemetry::default();

    // Task 1 telemetry
    let mut task1 = HashMap::new();
    task1.insert(
        "x402".to_string(),
        SkillActivationStats {
            count: 3,
            last_activated: Some("2026-03-22T10:00:00Z".to_string()),
            contexts: vec!["auto".to_string(); 3],
        },
    );

    for (name, stats) in &task1 {
        let global_stats = global.activations.entry(name.clone()).or_default();
        global_stats.count += stats.count;
        if stats.last_activated > global_stats.last_activated {
            global_stats
                .last_activated
                .clone_from(&stats.last_activated);
        }
    }

    // Task 2 telemetry
    let mut task2 = HashMap::new();
    task2.insert(
        "x402".to_string(),
        SkillActivationStats {
            count: 2,
            last_activated: Some("2026-03-22T12:00:00Z".to_string()),
            contexts: vec!["auto".to_string(); 2],
        },
    );

    for (name, stats) in &task2 {
        let global_stats = global.activations.entry(name.clone()).or_default();
        global_stats.count += stats.count;
        if stats.last_activated > global_stats.last_activated {
            global_stats
                .last_activated
                .clone_from(&stats.last_activated);
        }
    }

    assert_eq!(global.activations["x402"].count, 5);
    assert_eq!(
        global.activations["x402"].last_activated.as_deref(),
        Some("2026-03-22T12:00:00Z")
    );
}
