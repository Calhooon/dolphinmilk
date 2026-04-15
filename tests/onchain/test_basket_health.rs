//! Tests for basket UTXO health vital signs in the system prompt (#199).

use std::collections::HashMap;

use dolphin_milk::context::prompt::{build_system_prompt, PromptContext};

fn default_ctx() -> PromptContext {
    PromptContext {
        identity_key: String::new(),
        balance_sats: 0,
        model: String::new(),
        tools: Vec::new(),
        memory_summary: String::new(),
        available_files: Vec::new(),
        task: String::new(),
        budget_remaining: 0,
        low_power: false,
        inbox_count: 0,
        skills_section: String::new(),
        workspace_path: String::new(),
        certificate_info: None,
        has_external_messages: false,
        identity_soul: None,
        auto_recall_ids: vec![],
        basket_health: HashMap::new(),
        spendable_output_count: 0,
        instructions: String::new(),
        env_snapshot: None,
        working_memory_section: String::new(),
    }
}

// ---------------------------------------------------------------------------
// 1. Basket health section appears in prompt when populated
// ---------------------------------------------------------------------------

#[test]
fn test_basket_health_in_prompt() {
    let mut ctx = default_ctx();
    ctx.basket_health.insert("dm-state".to_string(), 3);
    ctx.basket_health.insert("dm-budget".to_string(), 1);
    ctx.basket_health.insert("dm-proofs".to_string(), 47);
    ctx.basket_health.insert("dm-revocation".to_string(), 1);

    let prompt = build_system_prompt(&ctx);

    assert!(
        prompt.contains("## On-Chain State"),
        "prompt should contain On-Chain State section"
    );
    assert!(
        prompt.contains("dm-state: 3 UTXOs"),
        "prompt should show state basket count"
    );
    assert!(
        prompt.contains("dm-budget: 1 UTXOs"),
        "prompt should show budget basket count"
    );
    assert!(
        prompt.contains("dm-proofs: 47 UTXOs"),
        "prompt should show proofs basket count"
    );
    assert!(
        prompt.contains("dm-revocation: 1 UTXOs"),
        "prompt should show revocation basket count"
    );
}

// ---------------------------------------------------------------------------
// 2. Empty basket_health skips the section entirely
// ---------------------------------------------------------------------------

#[test]
fn test_basket_health_empty_skips_section() {
    let ctx = default_ctx();
    let prompt = build_system_prompt(&ctx);

    assert!(
        !prompt.contains("## On-Chain State"),
        "empty basket_health should not produce On-Chain State section"
    );
}

// ---------------------------------------------------------------------------
// 3. No warnings at any count — UTXO counts are just data, not alarms
// ---------------------------------------------------------------------------

#[test]
fn test_basket_health_normal_no_warnings() {
    let mut ctx = default_ctx();
    ctx.basket_health.insert("dm-state".to_string(), 3);
    ctx.basket_health.insert("dm-budget".to_string(), 900);
    ctx.basket_health.insert("dm-proofs".to_string(), 4000);
    ctx.basket_health.insert("dm-revocation".to_string(), 1);

    let prompt = build_system_prompt(&ctx);

    // No warnings at any count — UTXO counts are data, not alarms
    assert!(
        !prompt.contains("WARNING"),
        "basket counts should never trigger warnings"
    );
    assert!(
        !prompt.contains("Note:"),
        "basket counts should never trigger notes"
    );
}

// ---------------------------------------------------------------------------
// 5. High state/budget counts show raw numbers without false alarms
// ---------------------------------------------------------------------------

#[test]
fn test_basket_health_high_counts_no_false_alarm() {
    let mut ctx = default_ctx();
    ctx.basket_health.insert("dm-state".to_string(), 50);
    ctx.basket_health.insert("dm-budget".to_string(), 903);

    let prompt = build_system_prompt(&ctx);

    // The counts should be visible so the agent can reason about them
    assert!(prompt.contains("dm-state: 50 UTXOs"));
    assert!(prompt.contains("dm-budget: 903 UTXOs"));
    // But no false alarm warnings
    assert!(!prompt.contains("WARNING"));
    assert!(!prompt.contains("orphaned"));
    assert!(!prompt.contains("failed"));
}
