//! Generic overlay lookup — query any lookup service and parse BEEF responses.
//!
//! The overlay returns `{type: "output-list", outputs: [{beef: [u8], outputIndex: N}]}`.
//! For AGENT records, we decode the BEEF → extract PushDrop → parse the 6 fields
//! into structured `AgentRecord`s.

use bsv::script::templates::PushDrop;
use bsv::transaction::Transaction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DmError;

/// A discovered agent from the overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub identity_key: String,
    pub certifier_key: String,
    pub name: String,
    pub capabilities: Vec<String>,
}

/// Query an overlay lookup service and return the raw JSON response.
///
/// This is the general-purpose function — works with any lookup service
/// (ls_agent, ls_ship, ls_slap, etc.).
pub async fn overlay_lookup(
    overlay_url: &str,
    service: &str,
    query: &Value,
) -> Result<Value, DmError> {
    let body = serde_json::json!({
        "service": service,
        "query": query,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{overlay_url}/lookup"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| DmError::wallet(format!("Overlay lookup failed: {e}")))?;

    let status = resp.status();
    let response: Value = resp
        .json()
        .await
        .map_err(|e| DmError::wallet(format!("Overlay response parse error: {e}")))?;

    if !status.is_success() {
        let msg = response
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(DmError::wallet(format!(
            "Overlay lookup failed (HTTP {status}): {msg}"
        )));
    }

    Ok(response)
}

/// Parse AGENT records from an overlay lookup response.
///
/// Extracts PushDrop fields from each BEEF output and returns structured
/// `AgentRecord`s. Outputs that fail to parse are silently skipped.
pub fn parse_agent_records(response: &Value) -> Vec<AgentRecord> {
    let outputs = match response.get("outputs").and_then(|o| o.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut records = Vec::new();
    for output in outputs {
        if let Some(record) = parse_single_agent_output(output) {
            records.push(record);
        }
    }
    records
}

/// Parse a single BEEF output into an AgentRecord.
///
/// The BEEF contains a transaction whose output at `outputIndex` has a PushDrop
/// locking script with 6 fields: AGENT, identity, certifier, name, capabilities, sig.
///
/// Uses bsv-rs `Transaction::from_beef()` for robust BEEF parsing that handles
/// both standard BEEF and AtomicBEEF with arbitrary BUMP complexity.
fn parse_single_agent_output(output: &Value) -> Option<AgentRecord> {
    let beef_array = output.get("beef")?.as_array()?;
    let output_index = output
        .get("outputIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    // Convert JSON byte array to Vec<u8>
    let beef_bytes: Vec<u8> = beef_array
        .iter()
        .filter_map(|b| b.as_u64().map(|v| v as u8))
        .collect();

    if beef_bytes.is_empty() {
        return None;
    }

    // Parse BEEF using bsv-rs SDK (handles standard BEEF + AtomicBEEF + complex BUMPs)
    let tx = match Transaction::from_beef(&beef_bytes, None) {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!("BEEF parse failed, trying pattern scan: {e}");
            // Fallback: scan raw bytes for an AGENT PushDrop pattern
            let script_hex = find_agent_script_in_bytes(&beef_bytes)?;
            return parse_agent_from_script_hex(&script_hex);
        }
    };

    if output_index >= tx.outputs.len() {
        tracing::debug!(
            "output_index {output_index} >= tx.outputs.len() {}",
            tx.outputs.len()
        );
        return None;
    }

    // Decode PushDrop from the locking script at the target output
    let pd = match PushDrop::decode(&tx.outputs[output_index].locking_script) {
        Ok(pd) => pd,
        Err(e) => {
            tracing::debug!(
                "PushDrop decode failed at output {output_index}: {e} — trying pattern scan"
            );
            // Fallback: pattern scan the raw BEEF bytes
            let script_hex = find_agent_script_in_bytes(&beef_bytes)?;
            return parse_agent_from_script_hex(&script_hex);
        }
    };

    // Extract fields from PushDrop data
    let fields: Vec<Vec<u8>> = pd.fields.iter().map(|f| f.to_vec()).collect();
    if fields.len() < 5 {
        return None;
    }

    // Field[0] should be "AGENT"
    let protocol = String::from_utf8(fields[0].clone()).unwrap_or_default();
    if protocol != "AGENT" {
        return None;
    }

    // Field[1] = identity key (33 bytes → 66 hex chars)
    let identity_key = hex::encode(&fields[1]);
    if identity_key.len() != 66 {
        return None;
    }

    // Field[2] = certifier key
    let certifier_key = hex::encode(&fields[2]);

    // Field[3] = name (UTF-8)
    let name = String::from_utf8(fields[3].clone()).unwrap_or_default();

    // Field[4] = capabilities CSV (UTF-8)
    let capabilities_str = String::from_utf8(fields[4].clone()).unwrap_or_default();
    let capabilities: Vec<String> = capabilities_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Some(AgentRecord {
        identity_key,
        certifier_key,
        name,
        capabilities,
    })
}

/// Parse an AgentRecord from a hex-encoded locking script (fallback path).
fn parse_agent_from_script_hex(script_hex: &str) -> Option<AgentRecord> {
    let fields = crate::onchain::state::parse_push_drop_fields(script_hex)?;
    if fields.len() < 5 {
        return None;
    }

    let protocol = hex_to_utf8(&fields[0]).unwrap_or_default();
    if protocol != "AGENT" {
        return None;
    }

    let identity_key = fields[1].clone();
    if identity_key.len() != 66 {
        return None;
    }

    Some(AgentRecord {
        identity_key,
        certifier_key: fields[2].clone(),
        name: hex_to_utf8(&fields[3]).unwrap_or_default(),
        capabilities: hex_to_utf8(&fields[4])
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    })
}

/// Fallback: scan raw bytes for an AGENT PushDrop script pattern.
///
/// Looks for the byte sequence `05 41 47 45 4e 54` (push 5 bytes "AGENT") followed
/// by a 33-byte pubkey push (0x21). When found, extracts the full script starting
/// from the AGENT push and ending at OP_CHECKSIG (0xAC).
fn find_agent_script_in_bytes(data: &[u8]) -> Option<String> {
    // Pattern: OP_PUSH5 "AGENT" = [0x05, 0x41, 0x47, 0x45, 0x4E, 0x54]
    let pattern: &[u8] = &[0x05, 0x41, 0x47, 0x45, 0x4E, 0x54];

    for i in 0..data.len().saturating_sub(pattern.len()) {
        if data[i..].starts_with(pattern) {
            // Found "AGENT" push. Now find the end of the script (OP_CHECKSIG = 0xAC).
            // The script should be within a few hundred bytes.
            let max_script_len = 1024;
            let search_end = (i + max_script_len).min(data.len());
            for j in (i + 6)..search_end {
                if data[j] == 0xAC {
                    // Found OP_CHECKSIG — extract the script
                    let script_bytes = &data[i..=j];
                    return Some(hex::encode(script_bytes));
                }
            }
        }
    }
    None
}

/// Decode a hex-encoded byte string to UTF-8.
fn hex_to_utf8(hex_str: &str) -> Option<String> {
    let bytes = hex::decode(hex_str).ok()?;
    String::from_utf8(bytes).ok()
}

// NOTE: The manual BEEF parser (extract_locking_script_from_beef, skip_bump, etc.) was removed
// in favor of bsv-rs `Transaction::from_beef()` which correctly handles complex BUMPs.
// See #320 for details on the bug in the old manual parser.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_to_utf8() {
        assert_eq!(hex_to_utf8("4147454e54"), Some("AGENT".to_string()));
        assert_eq!(hex_to_utf8(""), Some(String::new()));
        assert_eq!(hex_to_utf8("zz"), None); // invalid hex
    }

    // read_varint tests removed — manual BEEF parser replaced by bsv-rs Transaction::from_beef()

    #[test]
    fn test_parse_agent_records_empty() {
        let response = serde_json::json!({"type": "output-list", "outputs": []});
        let records = parse_agent_records(&response);
        assert!(records.is_empty());
    }

    #[test]
    fn test_parse_agent_records_no_outputs() {
        let response = serde_json::json!({"type": "output-list"});
        let records = parse_agent_records(&response);
        assert!(records.is_empty());
    }
}
