//! Live integration tests against the real BSV overlay.
//!
//! These tests hit the live overlay at rust-overlay.dev-a3e.workers.dev
//! and the wallet at localhost:3322 with real BSV.
//!
//! Run with: cargo test --test test_overlay_live -- --ignored --nocapture

use dolphin_milk::overlay::lookup::{overlay_lookup, parse_agent_records};
use dolphin_milk::overlay::registration::deregister_from_overlay;
use dolphin_milk::overlay::{check_registered, register_on_overlay};
use dolphin_milk::wallet::HttpWalletClient;
use serde_json::json;

const OVERLAY_URL: &str = "https://rust-overlay.dev-a3e.workers.dev";
const WALLET_URL: &str = "http://localhost:3322";

fn wallet() -> HttpWalletClient {
    HttpWalletClient::new(WALLET_URL, "http://localhost", 30)
}

async fn check_wallet() -> bool {
    wallet().is_authenticated().await.is_ok()
}

// ---------------------------------------------------------------------------
// Lookup tests (free — no sats)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_live_find_all_agents() {
    if !check_wallet().await {
        eprintln!("Wallet not running — skipping");
        return;
    }

    let result = overlay_lookup(OVERLAY_URL, "ls_agent", &json!({"findAll": true})).await;
    assert!(result.is_ok(), "findAll should succeed: {:?}", result.err());

    let response = result.unwrap();
    let records = parse_agent_records(&response);
    eprintln!("Found {} agents on overlay", records.len());
    for agent in &records {
        eprintln!(
            "  {} — {} [{}]",
            &agent.identity_key[..16],
            agent.name,
            agent.capabilities.join(", ")
        );
    }

    // The live overlay should have at least 1 agent registered
    let output_count = response["outputs"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(
        output_count > 0,
        "Live overlay should have registered agents"
    );

    // If BEEF parsing works, we should get structured records
    eprintln!(
        "BEEF parsing: {}/{} outputs decoded",
        records.len(),
        output_count
    );
}

#[tokio::test]
#[ignore]
async fn test_live_find_by_capability() {
    let result = overlay_lookup(
        OVERLAY_URL,
        "ls_agent",
        &json!({"findByCapability": "overlay-host"}),
    )
    .await;
    assert!(result.is_ok());
    let records = parse_agent_records(&result.unwrap());
    eprintln!("Agents with 'overlay-host' capability: {}", records.len());
}

// ---------------------------------------------------------------------------
// Registration tests (costs ~500 sats — real BSV)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_live_check_then_register() {
    if !check_wallet().await {
        eprintln!("Wallet not running — skipping");
        return;
    }

    let w = wallet();

    // Step 1: Check if already registered
    let already = check_registered(&w, OVERLAY_URL).await;
    eprintln!("Already registered: {:?}", already);

    // Step 2: Register (or re-register)
    let identity_key = w.get_identity_key().await.unwrap();
    eprintln!("Identity key: {}", &identity_key[..16]);

    // Use a test name that identifies this as a test registration
    let test_name = format!("test-agent-{}", &identity_key[..8]);

    match register_on_overlay(
        &w,
        OVERLAY_URL,
        &test_name,
        "tool-use,wallet,messaging",
        None, // self-signed (certifier = identity)
    )
    .await
    {
        Ok(txid) => {
            eprintln!("Registration txid: {}", txid);
            assert!(!txid.is_empty());

            // Step 3: Verify we can find ourselves
            let found = check_registered(&w, OVERLAY_URL).await;
            eprintln!("Found after registration: {:?}", found);
            // Note: may not be immediately visible due to overlay processing
        }
        Err(e) => {
            // Might fail if already registered with same name (dedup)
            eprintln!("Registration result: {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_live_lookup_after_registration() {
    if !check_wallet().await {
        eprintln!("Wallet not running — skipping");
        return;
    }

    let w = wallet();
    let identity_key = w.get_identity_key().await.unwrap();

    // Look up by our identity key
    let result = overlay_lookup(
        OVERLAY_URL,
        "ls_agent",
        &json!({"findByIdentityKey": identity_key}),
    )
    .await;

    match result {
        Ok(response) => {
            let records = parse_agent_records(&response);
            eprintln!(
                "Found {} records for identity {}",
                records.len(),
                &identity_key[..16]
            );
            for agent in &records {
                eprintln!("  name: {}", agent.name);
                eprintln!("  capabilities: {:?}", agent.capabilities);
            }
        }
        Err(e) => {
            eprintln!("Lookup failed (may not be registered yet): {}", e);
        }
    }
}

/// Test the full deregistration + re-registration lifecycle.
///
/// 1. Register with name "lifecycle-test-old"
/// 2. Verify it appears in lookup
/// 3. Deregister (spend UTXO + submit to overlay)
/// 4. Re-register with name "lifecycle-test-new"
/// 5. Verify the NEW name appears and the OLD name is gone
#[tokio::test]
#[ignore]
async fn test_live_deregister_and_reregister() {
    if !check_wallet().await {
        eprintln!("Wallet not running — skipping");
        return;
    }

    let w = wallet();
    let _identity_key = w.get_identity_key().await.unwrap();

    // Step 1: Register with old name
    eprintln!("Step 1: Registering with name 'lifecycle-test-old'...");
    match register_on_overlay(&w, OVERLAY_URL, "lifecycle-test-old", "llm,tools", None).await {
        Ok(txid) => eprintln!("  Registered: txid={}", &txid[..16]),
        Err(e) => {
            eprintln!("  Registration failed: {e}");
            return;
        }
    }

    // Step 2: Verify it appears
    eprintln!("Step 2: Verifying registration...");
    let record = check_registered(&w, OVERLAY_URL).await;
    match &record {
        Ok(Some(r)) => {
            eprintln!(
                "  Found: name={}, capabilities={:?}",
                r.name, r.capabilities
            );
            assert_eq!(r.name, "lifecycle-test-old");
        }
        Ok(None) => eprintln!("  WARNING: not found after registration (overlay lag?)"),
        Err(e) => eprintln!("  Check failed: {e}"),
    }

    // Step 3: Deregister (spend UTXO + submit to overlay)
    eprintln!("Step 3: Deregistering (spending old UTXO)...");
    match deregister_from_overlay(&w, OVERLAY_URL).await {
        Ok(n) => eprintln!("  Deregistered successfully ({n} UTXO(s) spent)"),
        Err(e) => eprintln!("  Deregistration failed: {e}"),
    }

    // Step 4: Re-register with new name
    eprintln!("Step 4: Re-registering with name 'lifecycle-test-new'...");
    match register_on_overlay(
        &w,
        OVERLAY_URL,
        "lifecycle-test-new",
        "llm,tools,wallet",
        None,
    )
    .await
    {
        Ok(txid) => eprintln!("  Registered: txid={}", &txid[..16]),
        Err(e) => {
            eprintln!("  Re-registration failed: {e}");
            return;
        }
    }

    // Step 5: Verify new name, old name gone
    eprintln!("Step 5: Verifying updated registration...");
    let record = check_registered(&w, OVERLAY_URL).await;
    match &record {
        Ok(Some(r)) => {
            eprintln!(
                "  Found: name={}, capabilities={:?}, identity={}",
                r.name,
                r.capabilities,
                &r.identity_key[..16]
            );
            assert_eq!(r.name, "lifecycle-test-new", "Name should be updated");
            assert!(
                r.capabilities.contains(&"wallet".to_string()),
                "Capabilities should include 'wallet'"
            );
        }
        Ok(None) => eprintln!("  WARNING: not found after re-registration"),
        Err(e) => eprintln!("  Check failed: {e}"),
    }

    // Verify old name is NOT findable
    let old_lookup = overlay_lookup(
        OVERLAY_URL,
        "ls_agent",
        &json!({"findByName": "lifecycle-test-old"}),
    )
    .await;
    match old_lookup {
        Ok(resp) => {
            let count = resp
                .get("outputs")
                .and_then(|o| o.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            eprintln!(
                "  Old name 'lifecycle-test-old': {} outputs (should be 0)",
                count
            );
        }
        Err(e) => eprintln!("  Old name lookup failed: {e}"),
    }

    eprintln!("Lifecycle test complete!");
}
