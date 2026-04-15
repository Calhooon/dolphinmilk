//! Unit tests for overlay lookup — BEEF parsing, PushDrop field extraction, AgentRecord parsing.

use dolphin_milk::overlay::lookup::{overlay_lookup, parse_agent_records, AgentRecord};
use serde_json::json;

// ---------------------------------------------------------------------------
// AgentRecord parsing from synthetic overlay responses
// ---------------------------------------------------------------------------

#[test]
fn test_parse_agent_records_empty_outputs() {
    let response = json!({"type": "output-list", "outputs": []});
    let records = parse_agent_records(&response);
    assert!(records.is_empty());
}

#[test]
fn test_parse_agent_records_missing_outputs() {
    let response = json!({"type": "output-list"});
    let records = parse_agent_records(&response);
    assert!(records.is_empty());
}

#[test]
fn test_parse_agent_records_null_response() {
    let response = json!(null);
    let records = parse_agent_records(&response);
    assert!(records.is_empty());
}

#[test]
fn test_parse_agent_records_invalid_beef() {
    // Invalid BEEF bytes — should be silently skipped
    let response = json!({
        "type": "output-list",
        "outputs": [
            {"beef": [0, 0, 0, 0], "outputIndex": 0}
        ]
    });
    let records = parse_agent_records(&response);
    assert!(records.is_empty());
}

#[test]
fn test_parse_agent_records_empty_beef() {
    let response = json!({
        "type": "output-list",
        "outputs": [
            {"beef": [], "outputIndex": 0}
        ]
    });
    let records = parse_agent_records(&response);
    assert!(records.is_empty());
}

#[test]
fn test_parse_agent_records_missing_beef_field() {
    let response = json!({
        "type": "output-list",
        "outputs": [
            {"outputIndex": 0}
        ]
    });
    let records = parse_agent_records(&response);
    assert!(records.is_empty());
}

// ---------------------------------------------------------------------------
// AgentRecord struct
// ---------------------------------------------------------------------------

#[test]
fn test_agent_record_serde_roundtrip() {
    let record = AgentRecord {
        identity_key: "03".to_string() + &"ab".repeat(32),
        certifier_key: "02".to_string() + &"cd".repeat(32),
        name: "test-agent".to_string(),
        capabilities: vec!["tool-use".to_string(), "messaging".to_string()],
    };

    let json = serde_json::to_string(&record).unwrap();
    let parsed: AgentRecord = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.identity_key, record.identity_key);
    assert_eq!(parsed.name, record.name);
    assert_eq!(parsed.capabilities, record.capabilities);
}

// ---------------------------------------------------------------------------
// PushDrop field extraction from synthetic BEEF
// ---------------------------------------------------------------------------

/// Build a minimal raw transaction with a single PushDrop output containing
/// the 6 AGENT fields.
fn build_synthetic_agent_tx(
    identity_hex: &str,
    certifier_hex: &str,
    name: &str,
    capabilities: &str,
) -> Vec<u8> {
    // Use build_push_drop_script to create the locking script
    let field_0 = b"AGENT";
    let identity_bytes = hex::decode(identity_hex).unwrap();
    let certifier_bytes = hex::decode(certifier_hex).unwrap();
    let name_bytes = name.as_bytes();
    let capabilities_bytes = capabilities.as_bytes();
    let signature = vec![0x30, 0x44]; // minimal DER-ish stub

    let fields: Vec<&[u8]> = vec![
        field_0,
        &identity_bytes,
        &certifier_bytes,
        name_bytes,
        capabilities_bytes,
        &signature,
    ];

    // Use a valid secp256k1 pubkey (generator point G)
    let pubkey = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let script_hex = dolphin_milk::onchain::state::build_push_drop_script(&fields, pubkey).unwrap();
    let script_bytes = hex::decode(&script_hex).unwrap();

    // Build a minimal raw transaction with 1 dummy input + 1 output + locktime
    let mut tx = Vec::new();
    // Version 1
    tx.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    // 1 input (required for Transaction::from_beef() parsing)
    tx.push(0x01);
    // prev_txid (32 zero bytes)
    tx.extend_from_slice(&[0x00; 32]);
    // prev_vout (0, LE u32)
    tx.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    // scriptSig length: 1 byte (OP_0)
    tx.push(0x01);
    tx.push(0x00); // OP_0
                   // sequence: 0xFFFFFFFF
    tx.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    // 1 output
    tx.push(0x01);
    // Satoshis: 1 (LE u64)
    tx.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    // Script length (varint)
    let script_len = script_bytes.len();
    if script_len < 0xFD {
        tx.push(script_len as u8);
    } else {
        tx.push(0xFD);
        tx.extend_from_slice(&(script_len as u16).to_le_bytes());
    }
    // Script
    tx.extend_from_slice(&script_bytes);
    // Locktime
    tx.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    tx
}

/// Wrap a raw transaction in BEEF format (minimal: 0 BUMPs, 1 tx with no BUMP).
fn wrap_in_beef(raw_tx: &[u8]) -> Vec<u8> {
    let mut beef = Vec::new();
    // BEEF magic: 0100BEEF
    beef.extend_from_slice(&[0x01, 0x00, 0xBE, 0xEF]);
    // nBUMPs: 0
    beef.push(0x00);
    // nTransactions: 1
    beef.push(0x01);
    // has_bump: 0 (no BUMP for this tx)
    beef.push(0x00);
    // Raw transaction
    beef.extend_from_slice(raw_tx);
    beef
}

/// Wrap BEEF in AtomicBEEF format (4-byte magic + 32-byte reversed txid + BEEF).
fn wrap_in_atomic_beef(beef: &[u8]) -> Vec<u8> {
    let mut atomic = Vec::new();
    // AtomicBEEF magic
    atomic.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
    // Reversed txid (32 zero bytes as placeholder)
    atomic.extend_from_slice(&[0x00; 32]);
    // BEEF data
    atomic.extend_from_slice(beef);
    atomic
}

#[test]
fn test_parse_agent_from_synthetic_beef() {
    let identity = "03".to_string() + &"ab".repeat(32);
    let certifier = "02".to_string() + &"cd".repeat(32);
    let name = "test-agent";
    let capabilities = "tool-use,messaging";

    let raw_tx = build_synthetic_agent_tx(&identity, &certifier, name, capabilities);
    let beef = wrap_in_beef(&raw_tx);

    // Build overlay response
    let beef_json: Vec<serde_json::Value> =
        beef.iter().map(|b| serde_json::json!(*b as u64)).collect();

    let response = json!({
        "type": "output-list",
        "outputs": [
            {"beef": beef_json, "outputIndex": 0}
        ]
    });

    let records = parse_agent_records(&response);
    assert_eq!(records.len(), 1, "Should parse 1 agent record");

    let agent = &records[0];
    assert_eq!(agent.identity_key, identity);
    assert_eq!(agent.certifier_key, certifier);
    assert_eq!(agent.name, name);
    assert_eq!(agent.capabilities, vec!["tool-use", "messaging"]);
}

#[test]
fn test_parse_agent_from_atomic_beef() {
    let identity = "03".to_string() + &"ab".repeat(32);
    let certifier = identity.clone(); // self-signed
    let name = "local-agent";
    let capabilities = "web-scraping";

    let raw_tx = build_synthetic_agent_tx(&identity, &certifier, name, capabilities);
    let beef = wrap_in_beef(&raw_tx);
    let atomic = wrap_in_atomic_beef(&beef);

    let beef_json: Vec<serde_json::Value> = atomic
        .iter()
        .map(|b| serde_json::json!(*b as u64))
        .collect();

    let response = json!({
        "type": "output-list",
        "outputs": [
            {"beef": beef_json, "outputIndex": 0}
        ]
    });

    let records = parse_agent_records(&response);
    assert_eq!(records.len(), 1, "Should parse agent from AtomicBEEF");

    let agent = &records[0];
    assert_eq!(agent.name, name);
    assert_eq!(agent.capabilities, vec!["web-scraping"]);
}

#[test]
fn test_parse_agent_from_raw_tx() {
    let identity = "03".to_string() + &"ab".repeat(32);
    let certifier = identity.clone();
    let name = "research-agent";
    let capabilities = "research-orchestration,peer-discovery";

    let raw_tx = build_synthetic_agent_tx(&identity, &certifier, name, capabilities);

    // Submit as raw tx (no BEEF wrapping) — should still work
    let beef_json: Vec<serde_json::Value> = raw_tx
        .iter()
        .map(|b| serde_json::json!(*b as u64))
        .collect();

    let response = json!({
        "type": "output-list",
        "outputs": [
            {"beef": beef_json, "outputIndex": 0}
        ]
    });

    let records = parse_agent_records(&response);
    assert_eq!(records.len(), 1, "Should parse agent from raw tx");
    assert_eq!(
        records[0].capabilities,
        vec!["research-orchestration", "peer-discovery"]
    );
}

#[test]
fn test_parse_multiple_agents() {
    let agents = [
        (
            "03".to_string() + &"11".repeat(32),
            "research-agent",
            "orchestration",
        ),
        (
            "03".to_string() + &"22".repeat(32),
            "http://localhost:3002",
            "web-scraping",
        ),
        (
            "03".to_string() + &"33".repeat(32),
            "http://localhost:3003",
            "analysis",
        ),
    ];

    let outputs: Vec<serde_json::Value> = agents
        .iter()
        .map(|(identity, name, caps)| {
            let raw_tx = build_synthetic_agent_tx(identity, identity, name, caps);
            let beef = wrap_in_beef(&raw_tx);
            let beef_json: Vec<serde_json::Value> =
                beef.iter().map(|b| serde_json::json!(*b as u64)).collect();
            json!({"beef": beef_json, "outputIndex": 0})
        })
        .collect();

    let response = json!({"type": "output-list", "outputs": outputs});
    let records = parse_agent_records(&response);

    assert_eq!(records.len(), 3, "Should parse all 3 agents");
    assert_eq!(records[0].capabilities, vec!["orchestration"]);
    assert_eq!(records[1].capabilities, vec!["web-scraping"]);
    assert_eq!(records[2].capabilities, vec!["analysis"]);
}

// ---------------------------------------------------------------------------
// overlay_lookup network call (integration — requires live overlay)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires live overlay
async fn test_overlay_lookup_find_all() {
    let result = overlay_lookup(
        "https://rust-overlay.dev-a3e.workers.dev",
        "ls_agent",
        &json!({"findAll": true}),
    )
    .await;

    assert!(result.is_ok(), "Should succeed: {:?}", result.err());
    let response = result.unwrap();
    assert_eq!(response["type"], "output-list");
    let outputs = response["outputs"].as_array().unwrap();
    assert!(
        !outputs.is_empty(),
        "Live overlay should have registered agents"
    );
}
