//! BRC-56 Peer Discovery — find and verify other agents by identity key or attributes.
//!
//! Wraps the wallet's `discoverByIdentityKey` and `discoverByAttributes` endpoints
//! to enable service delegation, sender verification, and fleet management.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::error::DmError;
use crate::wallet::WalletClient;

/// Information about a discovered peer agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// The peer's identity public key (66-char hex compressed).
    pub identity_key: String,
    /// Human-readable name (from certificate fields, if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Capabilities advertised by the peer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    /// Certificate type (e.g., "agent-authorization").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_type: Option<String>,
    /// The certifier's identity key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certifier: Option<String>,
    /// Additional certificate fields discovered.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, String>,
    /// Raw certificate data from the wallet response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// Result of verifying a peer's certificates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerVerification {
    /// The peer's identity key.
    pub identity_key: String,
    /// Whether any certificates were found for this peer.
    pub has_certificates: bool,
    /// Number of certificates discovered.
    pub certificate_count: usize,
    /// Whether any certificate is parent-signed (certifier != subject).
    pub has_parent_signed: bool,
    /// List of certificate types found.
    pub certificate_types: Vec<String>,
    /// List of certifiers found.
    pub certifiers: Vec<String>,
}

/// BRC-56 peer discovery service.
pub struct PeerDiscovery {
    wallet: WalletClient,
}

impl PeerDiscovery {
    /// Create a new PeerDiscovery instance.
    pub fn new(wallet: WalletClient) -> Self {
        Self { wallet }
    }

    /// Discover peers by identity key.
    ///
    /// Queries the wallet's BRC-56 `discoverByIdentityKey` endpoint to find
    /// certificates associated with the given public key.
    pub async fn discover_by_key(
        &self,
        identity_key: &str,
        limit: u64,
    ) -> Result<Vec<PeerInfo>, DmError> {
        let result = self
            .wallet
            .discover_by_identity_key(identity_key, None, limit)
            .await?;
        Ok(parse_discovery_result(&result, Some(identity_key)))
    }

    /// Discover peers by certificate attributes.
    ///
    /// Queries the wallet's BRC-56 `discoverByAttributes` endpoint to find
    /// certificates matching the given attribute key-value pairs.
    pub async fn discover_by_attributes(
        &self,
        attrs: &HashMap<String, String>,
        limit: u64,
    ) -> Result<Vec<PeerInfo>, DmError> {
        let attrs_value = serde_json::to_value(attrs)
            .map_err(|e| DmError::wallet(format!("Failed to serialize attributes: {e}")))?;
        let result = self
            .wallet
            .discover_by_attributes(&attrs_value, None, limit)
            .await?;
        Ok(parse_discovery_result(&result, None))
    }

    /// Verify a peer by checking their certificates.
    ///
    /// Discovers certificates for the given identity key and summarizes
    /// the verification status.
    pub async fn verify_peer(&self, identity_key: &str) -> Result<PeerVerification, DmError> {
        let peers = self.discover_by_key(identity_key, 100).await?;

        let mut certificate_types = Vec::new();
        let mut certifiers = Vec::new();
        let mut has_parent_signed = false;

        for peer in &peers {
            if let Some(ref cert_type) = peer.certificate_type {
                if !certificate_types.contains(cert_type) {
                    certificate_types.push(cert_type.clone());
                }
            }
            if let Some(ref certifier) = peer.certifier {
                if !certifiers.contains(certifier) {
                    certifiers.push(certifier.clone());
                }
                // Parent-signed: certifier is different from the subject
                if certifier != identity_key {
                    has_parent_signed = true;
                }
            }
        }

        Ok(PeerVerification {
            identity_key: identity_key.to_string(),
            has_certificates: !peers.is_empty(),
            certificate_count: peers.len(),
            has_parent_signed,
            certificate_types,
            certifiers,
        })
    }
}

/// Parse a wallet discovery response into a list of `PeerInfo` structs.
fn parse_discovery_result(result: &Value, known_key: Option<&str>) -> Vec<PeerInfo> {
    let mut peers = Vec::new();

    // The wallet may return certificates in various formats:
    // - { "certificates": [...] }
    // - { "totalCertificates": N, "certificates": [...] }
    // - Array of certificates directly
    let certs = if let Some(arr) = result.get("certificates").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = result.as_array() {
        arr.clone()
    } else {
        // Single result — wrap in array
        vec![result.clone()]
    };

    for cert in &certs {
        let subject = cert.get("subject").and_then(|v| v.as_str()).unwrap_or("");
        let certifier = cert.get("certifier").and_then(|v| v.as_str()).unwrap_or("");
        let cert_type = cert
            .get("certificateType")
            .or_else(|| cert.get("type"))
            .and_then(|v| v.as_str());

        let identity_key = if !subject.is_empty() {
            subject.to_string()
        } else if let Some(key) = known_key {
            key.to_string()
        } else {
            continue;
        };

        // Extract fields
        let mut fields = HashMap::new();
        let mut name = None;
        let mut capabilities = None;

        if let Some(field_obj) = cert.get("fields").and_then(|v| v.as_object()) {
            for (k, v) in field_obj {
                if let Some(s) = v.as_str() {
                    fields.insert(k.clone(), s.to_string());
                    if k == "name" {
                        name = Some(s.to_string());
                    }
                    if k == "capabilities" {
                        capabilities = Some(
                            s.split(',')
                                .map(|c| c.trim().to_string())
                                .filter(|c| !c.is_empty())
                                .collect(),
                        );
                    }
                }
            }
        }

        peers.push(PeerInfo {
            identity_key,
            name,
            capabilities,
            certificate_type: cert_type.map(|s| s.to_string()),
            certifier: if !certifier.is_empty() {
                Some(certifier.to_string())
            } else {
                None
            },
            fields,
            raw: Some(cert.clone()),
        });
    }

    peers
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_info_serde_roundtrip() {
        let peer = PeerInfo {
            identity_key: "02abc".to_string(),
            name: Some("TestAgent".to_string()),
            capabilities: Some(vec!["tools".to_string(), "llm".to_string()]),
            certificate_type: Some("agent-authorization".to_string()),
            certifier: Some("02def".to_string()),
            fields: HashMap::new(),
            raw: None,
        };
        let json = serde_json::to_string(&peer).unwrap();
        let back: PeerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.identity_key, "02abc");
        assert_eq!(back.name.as_deref(), Some("TestAgent"));
        assert_eq!(back.capabilities.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_peer_info_skip_none_fields() {
        let peer = PeerInfo {
            identity_key: "02abc".to_string(),
            name: None,
            capabilities: None,
            certificate_type: None,
            certifier: None,
            fields: HashMap::new(),
            raw: None,
        };
        let json = serde_json::to_string(&peer).unwrap();
        assert!(!json.contains("name"));
        assert!(!json.contains("capabilities"));
        assert!(!json.contains("certificate_type"));
    }

    #[test]
    fn test_peer_verification_no_certs() {
        let v = PeerVerification {
            identity_key: "02abc".to_string(),
            has_certificates: false,
            certificate_count: 0,
            has_parent_signed: false,
            certificate_types: vec![],
            certifiers: vec![],
        };
        assert!(!v.has_certificates);
        assert!(!v.has_parent_signed);
    }

    #[test]
    fn test_peer_verification_with_parent_signed() {
        let v = PeerVerification {
            identity_key: "02abc".to_string(),
            has_certificates: true,
            certificate_count: 1,
            has_parent_signed: true,
            certificate_types: vec!["agent-authorization".to_string()],
            certifiers: vec!["02parent".to_string()],
        };
        assert!(v.has_certificates);
        assert!(v.has_parent_signed);
    }

    #[test]
    fn test_parse_discovery_result_certificates_array() {
        let result = serde_json::json!({
            "certificates": [
                {
                    "subject": "02abc",
                    "certifier": "02def",
                    "certificateType": "agent-authorization",
                    "fields": {
                        "name": "Agent1",
                        "capabilities": "tools,llm"
                    }
                }
            ]
        });
        let peers = parse_discovery_result(&result, None);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].identity_key, "02abc");
        assert_eq!(peers[0].name.as_deref(), Some("Agent1"));
        assert_eq!(peers[0].capabilities.as_ref().unwrap(), &["tools", "llm"]);
    }

    #[test]
    fn test_parse_discovery_result_direct_array() {
        let result = serde_json::json!([
            {
                "subject": "02abc",
                "certifier": "02abc",
                "type": "self-signed"
            }
        ]);
        let peers = parse_discovery_result(&result, None);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].certifier.as_deref(), Some("02abc"));
        assert_eq!(peers[0].certificate_type.as_deref(), Some("self-signed"));
    }

    #[test]
    fn test_parse_discovery_result_known_key_fallback() {
        let result = serde_json::json!({
            "certificates": [
                {
                    "certifier": "02def",
                    "fields": {"name": "Test"}
                }
            ]
        });
        let peers = parse_discovery_result(&result, Some("02known"));
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].identity_key, "02known");
    }

    #[test]
    fn test_parse_discovery_result_empty() {
        let result = serde_json::json!({"certificates": []});
        let peers = parse_discovery_result(&result, None);
        assert!(peers.is_empty());
    }

    #[test]
    fn test_parse_discovery_result_skips_no_subject_no_known() {
        let result = serde_json::json!({
            "certificates": [
                {"certifier": "02def", "fields": {}}
            ]
        });
        let peers = parse_discovery_result(&result, None);
        assert!(peers.is_empty());
    }
}
