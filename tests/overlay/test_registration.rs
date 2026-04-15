//! Unit tests for overlay registration — PushDrop field construction and signing.

use dolphin_milk::overlay::{
    agent_registry_protocol_id, AGENT_REGISTRY_COUNTERPARTY, AGENT_REGISTRY_KEY_ID,
    BASKET_AGENT_REGISTRATION,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn test_basket_name() {
    assert_eq!(BASKET_AGENT_REGISTRATION, "dm-agent-registration");
}

#[test]
fn test_protocol_id() {
    let proto = agent_registry_protocol_id();
    let arr = proto.as_array().unwrap();
    assert_eq!(arr[0].as_u64().unwrap(), 2);
    assert_eq!(arr[1].as_str().unwrap(), "agent registry");
}

#[test]
fn test_key_id() {
    assert_eq!(AGENT_REGISTRY_KEY_ID, "1");
}

#[test]
fn test_counterparty() {
    assert_eq!(AGENT_REGISTRY_COUNTERPARTY, "anyone");
}

// ---------------------------------------------------------------------------
// PushDrop construction matches overlay's expected 6-field format
// ---------------------------------------------------------------------------

#[test]
fn test_agent_pushdrop_field_count() {
    // Build the 6 fields as we do in registration
    let field_0 = b"AGENT".to_vec();
    let identity_hex = "03".to_string() + &"ab".repeat(32);
    let field_1 = hex::decode(&identity_hex).unwrap();
    let field_2 = field_1.clone(); // self-signed
    let field_3 = b"https://agent.example.com".to_vec();
    let field_4 = b"tool-use,messaging".to_vec();
    let field_5 = vec![0x30, 0x44]; // stub signature

    let fields: Vec<&[u8]> = vec![&field_0, &field_1, &field_2, &field_3, &field_4, &field_5];

    // Generator point G as pubkey
    let pubkey = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let script_hex = dolphin_milk::onchain::state::build_push_drop_script(&fields, pubkey).unwrap();

    // Parse back and verify 6 fields
    let parsed = dolphin_milk::onchain::state::parse_push_drop_fields(&script_hex).unwrap();
    assert_eq!(parsed.len(), 6, "AGENT PushDrop must have exactly 6 fields");

    // Field[0] = "AGENT"
    let protocol = String::from_utf8(hex::decode(&parsed[0]).unwrap()).unwrap();
    assert_eq!(protocol, "AGENT");

    // Field[1] = 33-byte identity key
    let id_bytes = hex::decode(&parsed[1]).unwrap();
    assert_eq!(id_bytes.len(), 33);
    assert_eq!(hex::encode(&id_bytes), identity_hex);

    // Field[2] = 33-byte certifier key (same as identity for self-signed)
    let cert_bytes = hex::decode(&parsed[2]).unwrap();
    assert_eq!(cert_bytes.len(), 33);

    // Field[3] = endpoint URL
    let endpoint = String::from_utf8(hex::decode(&parsed[3]).unwrap()).unwrap();
    assert_eq!(endpoint, "https://agent.example.com");

    // Field[4] = capabilities CSV
    let caps = String::from_utf8(hex::decode(&parsed[4]).unwrap()).unwrap();
    assert_eq!(caps, "tool-use,messaging");

    // Field[5] = signature (non-empty)
    let sig_bytes = hex::decode(&parsed[5]).unwrap();
    assert!(!sig_bytes.is_empty(), "Signature must be non-empty");
}

#[test]
fn test_agent_pushdrop_field_ordering() {
    // The overlay's AgentTopicManager expects fields in EXACT order:
    // [0] "AGENT", [1] subject, [2] certifier, [3] endpoint, [4] caps, [5] sig
    // Verify our construction matches
    let identity = hex::decode("03".to_string() + &"ff".repeat(32)).unwrap();
    let certifier = hex::decode("02".to_string() + &"ee".repeat(32)).unwrap();

    let fields: Vec<&[u8]> = vec![
        b"AGENT",
        &identity,
        &certifier,
        b"https://test.example.com",
        b"web-scraping,data-analysis",
        &[0x30, 0x45, 0x02, 0x20], // DER stub
    ];

    let pubkey = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let script_hex = dolphin_milk::onchain::state::build_push_drop_script(&fields, pubkey).unwrap();

    let parsed = dolphin_milk::onchain::state::parse_push_drop_fields(&script_hex).unwrap();

    // Verify order matches overlay expectations
    assert_eq!(
        String::from_utf8(hex::decode(&parsed[0]).unwrap()).unwrap(),
        "AGENT",
        "Field[0] must be protocol tag"
    );
    assert_eq!(
        hex::decode(&parsed[1]).unwrap().len(),
        33,
        "Field[1] must be 33-byte subject"
    );
    assert_eq!(
        hex::decode(&parsed[2]).unwrap().len(),
        33,
        "Field[2] must be 33-byte certifier"
    );
    assert_eq!(
        String::from_utf8(hex::decode(&parsed[3]).unwrap()).unwrap(),
        "https://test.example.com",
        "Field[3] must be endpoint"
    );
    assert_eq!(
        String::from_utf8(hex::decode(&parsed[4]).unwrap()).unwrap(),
        "web-scraping,data-analysis",
        "Field[4] must be capabilities"
    );
    assert!(
        !hex::decode(&parsed[5]).unwrap().is_empty(),
        "Field[5] must be non-empty signature"
    );
}

#[test]
fn test_signature_data_construction() {
    // The overlay verifies: signature over concat(fields[0..5])
    // Verify our concatenation matches what the overlay expects
    let field_0 = b"AGENT";
    let field_1 = hex::decode("03".to_string() + &"ab".repeat(32)).unwrap();
    let field_2 = field_1.clone();
    let field_3 = b"https://example.com";
    let field_4 = b"tool-use";

    let sign_data: Vec<u8> = [
        field_0.as_slice(),
        field_1.as_slice(),
        field_2.as_slice(),
        field_3.as_slice(),
        field_4.as_slice(),
    ]
    .concat();

    // Should be: "AGENT" (5) + identity (33) + certifier (33) + endpoint (19) + caps (8) = 98 bytes
    assert_eq!(
        sign_data.len(),
        5 + 33 + 33 + 19 + 8,
        "Signature data should be concatenation of fields[0..5]"
    );

    // First 5 bytes = "AGENT"
    assert_eq!(&sign_data[..5], b"AGENT");
    // Next 33 bytes = identity key
    assert_eq!(&sign_data[5..38], field_1.as_slice());
}
