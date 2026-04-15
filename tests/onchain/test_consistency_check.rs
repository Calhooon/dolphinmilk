//! Tests for R.2: BRC-48 state consistency check (#196).
//!
//! Tests cover `check_consistency()` logic, `ConsistencyResult` / `ConsistencyCheck`
//! serde, `ActiveTokenSummary` serde, `consistency_check_interval` config,
//! and `read_active_token_summary()` via mock wallet.

use std::collections::HashMap;

use dolphin_milk::state::{
    check_consistency, ActiveTokenSummary, ConsistencyCheck, ConsistencyResult, BASKET_BUDGET,
    BASKET_STATE,
};

// ─────────────────────────────────────────────
// check_consistency — all checks pass
// ─────────────────────────────────────────────

#[test]
fn test_consistency_result_consistent() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 3);
    basket_health.insert(BASKET_BUDGET.to_string(), 1);

    let result = check_consistency(5, 1000, &basket_health);
    assert!(result.consistent);
    assert_eq!(result.checks.len(), 2);
    for check in &result.checks {
        assert_eq!(check.status, "ok");
    }
}

// ─────────────────────────────────────────────
// check_consistency — divergence detected
// ─────────────────────────────────────────────

#[test]
fn test_consistency_result_diverged() {
    // No tokens at all when iteration > 0 and sats_spent > 0
    let basket_health = HashMap::new();

    let result = check_consistency(5, 1000, &basket_health);
    assert!(!result.consistent);

    let diverged: Vec<_> = result
        .checks
        .iter()
        .filter(|c| c.status == "diverged")
        .collect();
    assert!(!diverged.is_empty(), "Should have at least one divergence");
}

// ─────────────────────────────────────────────
// check_consistency — normal basket counts pass
// ─────────────────────────────────────────────

#[test]
fn test_consistency_check_normal_baskets() {
    let mut basket_health = HashMap::new();
    // Normal: 1-10 state tokens, 1-5 budget tokens
    basket_health.insert(BASKET_STATE.to_string(), 5);
    basket_health.insert(BASKET_BUDGET.to_string(), 2);

    let result = check_consistency(10, 5000, &basket_health);
    assert!(result.consistent);
    assert!(result.checks.iter().all(|c| c.status == "ok"));
}

// ─────────────────────────────────────────────
// check_consistency — high counts are OK (historical accumulation)
// ─────────────────────────────────────────────

#[test]
fn test_consistency_check_high_state_count_ok() {
    let mut basket_health = HashMap::new();
    // Hundreds of tokens from past tasks — this is expected historical state
    basket_health.insert(BASKET_STATE.to_string(), 2380);
    basket_health.insert(BASKET_BUDGET.to_string(), 1);

    let result = check_consistency(5, 500, &basket_health);
    assert!(
        result.consistent,
        "High state count should NOT be flagged as divergence"
    );
}

#[test]
fn test_consistency_check_high_budget_count_ok() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 3);
    // Hundreds of budget tokens from past iterations — expected
    basket_health.insert(BASKET_BUDGET.to_string(), 1017);

    let result = check_consistency(5, 500, &basket_health);
    assert!(
        result.consistent,
        "High budget count should NOT be flagged as divergence"
    );
}

// ─────────────────────────────────────────────
// check_consistency — empty baskets ARE divergence
// ─────────────────────────────────────────────

#[test]
fn test_consistency_check_empty_state_diverged() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 0); // empty when iteration > 0
    basket_health.insert(BASKET_BUDGET.to_string(), 1);

    let result = check_consistency(5, 500, &basket_health);
    assert!(!result.consistent);

    let state_check = result
        .checks
        .iter()
        .find(|c| c.field == "worm-state_count")
        .unwrap();
    assert_eq!(state_check.status, "diverged");
}

#[test]
fn test_consistency_check_empty_budget_diverged() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 3);
    basket_health.insert(BASKET_BUDGET.to_string(), 0); // empty when sats_spent > 0

    let result = check_consistency(5, 500, &basket_health);
    assert!(!result.consistent);

    let budget_check = result
        .checks
        .iter()
        .find(|c| c.field == "worm-budget_count")
        .unwrap();
    assert_eq!(budget_check.status, "diverged");
}

// ─────────────────────────────────────────────
// ActiveTokenSummary — serde roundtrip
// ─────────────────────────────────────────────

#[test]
fn test_active_token_summary_serde() {
    let summary = ActiveTokenSummary {
        basket: "dm-state".to_string(),
        count: 3,
        token_types: vec![
            "task_commitment".to_string(),
            "capability_declaration".to_string(),
        ],
    };

    let json = serde_json::to_string(&summary).unwrap();
    assert!(json.contains("dm-state"));
    assert!(json.contains("task_commitment"));

    let back: ActiveTokenSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(back.basket, "dm-state");
    assert_eq!(back.count, 3);
    assert_eq!(back.token_types.len(), 2);
}

// ─────────────────────────────────────────────
// ConsistencyResult — serde roundtrip
// ─────────────────────────────────────────────

#[test]
fn test_consistency_result_serde() {
    let result = ConsistencyResult {
        consistent: false,
        checks: vec![ConsistencyCheck {
            field: "worm-state_count".to_string(),
            status: "diverged".to_string(),
            in_memory: "iteration=5".to_string(),
            on_chain: "count=15".to_string(),
        }],
    };

    let json = serde_json::to_string(&result).unwrap();
    let back: ConsistencyResult = serde_json::from_str(&json).unwrap();
    assert!(!back.consistent);
    assert_eq!(back.checks.len(), 1);
    assert_eq!(back.checks[0].field, "worm-state_count");
    assert_eq!(back.checks[0].status, "diverged");
}

// ─────────────────────────────────────────────
// Config — default consistency_check_interval
// ─────────────────────────────────────────────

#[test]
fn test_consistency_config_default() {
    let cfg = dolphin_milk::config::DmConfig::default();
    assert_eq!(cfg.lifecycle.consistency_check_interval, 5);
}

// ─────────────────────────────────────────────
// Config — TOML parsing
// ─────────────────────────────────────────────

#[test]
fn test_consistency_config_toml() {
    let toml_str = r#"
[lifecycle]
consistency_check_interval = 3
"#;
    let cfg: dolphin_milk::config::DmConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.lifecycle.consistency_check_interval, 3);
}

// ─────────────────────────────────────────────
// read_active_token_summary — mock wallet
// ─────────────────────────────────────────────

#[tokio::test]
async fn test_read_active_token_summary_mock() {
    use dolphin_milk::state::read_active_token_summary;
    use dolphin_milk::wallet::WalletClient;

    let mut server = mockito::Server::new_async().await;
    let url = server.url();

    // Mock wallet listOutputs response with 2 PushDrop outputs.
    // Each output has a locking script with token type as first field.
    // We need valid PushDrop scripts. Build minimal ones:
    // field1 = "task_commitment" (hex-encoded), field2 = "{}" (hex-encoded)
    let type_hex = hex::encode(b"task_commitment");
    let data_hex = hex::encode(b"{}");
    // Build a minimal PushDrop-like script:
    // <len1> <type_bytes> <len2> <data_bytes> OP_2DROP <33> <pubkey> OP_CHECKSIG
    let type_bytes = hex::decode(&type_hex).unwrap();
    let data_bytes = hex::decode(&data_hex).unwrap();

    let pubkey_hex = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let script =
        dolphin_milk::state::build_push_drop_script(&[&type_bytes, &data_bytes], pubkey_hex)
            .unwrap();

    let body = serde_json::json!({
        "outputs": [
            {
                "outpoint": "aabb.0",
                "lockingScript": script,
                "satoshis": 1,
            },
            {
                "outpoint": "ccdd.0",
                "lockingScript": script,
                "satoshis": 1,
            },
        ]
    });

    let _m = server
        .mock("POST", "/listOutputs")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&body).unwrap())
        .create_async()
        .await;

    let wallet = WalletClient::new(&url, "http://localhost", 5);
    let summary = read_active_token_summary(&wallet, "dm-state", 100)
        .await
        .unwrap();

    assert_eq!(summary.basket, "dm-state");
    assert_eq!(summary.count, 2);
    assert!(summary.token_types.contains(&"task_commitment".to_string()));
}

// ─────────────────────────────────────────────
// Edge: iteration 0 with no baskets is consistent
// ─────────────────────────────────────────────

#[test]
fn test_consistency_check_iteration_zero_empty() {
    let basket_health = HashMap::new();
    let result = check_consistency(0, 0, &basket_health);
    // At iteration 0 with 0 sats spent, empty baskets are fine
    assert!(result.consistent);
}

// ─────────────────────────────────────────────
// Edge: sats_spent=0 with no budget tokens is ok
// ─────────────────────────────────────────────

#[test]
fn test_consistency_check_zero_sats_no_budget_ok() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 2);
    // No budget entry — but sats_spent=0 so it's fine
    let result = check_consistency(2, 0, &basket_health);
    assert!(result.consistent);
}

// ─────────────────────────────────────────────
// Edge: boundary — 1 token is enough (not diverged)
// ─────────────────────────────────────────────

#[test]
fn test_consistency_check_boundary_state_1() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 1); // 1 is fine
    basket_health.insert(BASKET_BUDGET.to_string(), 1);

    let result = check_consistency(5, 500, &basket_health);
    assert!(result.consistent);
}

#[test]
fn test_consistency_check_boundary_budget_1() {
    let mut basket_health = HashMap::new();
    basket_health.insert(BASKET_STATE.to_string(), 1);
    basket_health.insert(BASKET_BUDGET.to_string(), 1); // 1 is fine

    let result = check_consistency(5, 500, &basket_health);
    assert!(result.consistent);
}
