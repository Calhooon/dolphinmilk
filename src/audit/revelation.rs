//! BRC-69 key linkage revelation — structured format and audit logging.
//!
//! Provides a `Revelation` struct that wraps wallet key linkage data with
//! audit metadata (requester, timestamp, deterministic hash), and a
//! `RevelationLog` that persists revelations as an append-only directory
//! of JSON files.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// A structured key linkage revelation with audit metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revelation {
    /// Unique ID for this revelation.
    pub id: String,
    /// Type: "counterparty" or "specific".
    pub revelation_type: String,
    /// The counterparty key involved.
    pub counterparty: String,
    /// Protocol ID used (e.g., `[2, "dolphin milk message signature"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_id: Option<String>,
    /// Key ID used (if specific revelation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    /// Who requested this revelation.
    pub requested_by: String,
    /// When the revelation was made (RFC 3339).
    pub timestamp: String,
    /// The revealed key linkage data from the wallet.
    pub linkage_data: serde_json::Value,
    /// SHA-256 hash of the revelation for audit chain.
    pub revelation_hash: String,
}

impl Revelation {
    /// Create a new counterparty revelation.
    pub fn counterparty(
        counterparty: &str,
        requested_by: &str,
        linkage_data: serde_json::Value,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = Utc::now().to_rfc3339();
        let hash = compute_revelation_hash(
            "counterparty",
            counterparty,
            None,
            None,
            requested_by,
            &timestamp,
            &linkage_data,
        );
        Self {
            id,
            revelation_type: "counterparty".to_string(),
            counterparty: counterparty.to_string(),
            protocol_id: None,
            key_id: None,
            requested_by: requested_by.to_string(),
            timestamp,
            linkage_data,
            revelation_hash: hash,
        }
    }

    /// Create a new specific key linkage revelation.
    pub fn specific(
        counterparty: &str,
        protocol_id: &str,
        key_id: &str,
        requested_by: &str,
        linkage_data: serde_json::Value,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = Utc::now().to_rfc3339();
        let hash = compute_revelation_hash(
            "specific",
            counterparty,
            Some(protocol_id),
            Some(key_id),
            requested_by,
            &timestamp,
            &linkage_data,
        );
        Self {
            id,
            revelation_type: "specific".to_string(),
            counterparty: counterparty.to_string(),
            protocol_id: Some(protocol_id.to_string()),
            key_id: Some(key_id.to_string()),
            requested_by: requested_by.to_string(),
            timestamp,
            linkage_data,
            revelation_hash: hash,
        }
    }
}

/// Compute a deterministic SHA-256 hash for a revelation.
///
/// The hash covers the type, counterparty, optional protocol/key,
/// requester, timestamp, and linkage data — enabling verifiers to
/// confirm the revelation hasn't been tampered with.
pub fn compute_revelation_hash(
    revelation_type: &str,
    counterparty: &str,
    protocol_id: Option<&str>,
    key_id: Option<&str>,
    requested_by: &str,
    timestamp: &str,
    linkage_data: &serde_json::Value,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(revelation_type.as_bytes());
    hasher.update(b"|");
    hasher.update(counterparty.as_bytes());
    hasher.update(b"|");
    hasher.update(protocol_id.unwrap_or("").as_bytes());
    hasher.update(b"|");
    hasher.update(key_id.unwrap_or("").as_bytes());
    hasher.update(b"|");
    hasher.update(requested_by.as_bytes());
    hasher.update(b"|");
    hasher.update(timestamp.as_bytes());
    hasher.update(b"|");
    hasher.update(linkage_data.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

/// Append-only audit log for revelations, persisted as JSON files.
pub struct RevelationLog {
    log_dir: PathBuf,
}

impl RevelationLog {
    /// Create a new log backed by `{workspace}/audit_revelations/`.
    pub fn new(workspace: &str) -> Self {
        let log_dir = PathBuf::from(workspace).join("audit_revelations");
        let _ = fs::create_dir_all(&log_dir);
        Self { log_dir }
    }

    /// Create from an explicit directory path.
    pub fn from_dir(log_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&log_dir);
        Self { log_dir }
    }

    /// Record a revelation to the audit log.
    pub fn record(&self, revelation: &Revelation) -> std::io::Result<()> {
        let path = self.log_dir.join(format!("{}.json", revelation.id));
        let json = serde_json::to_string_pretty(revelation).map_err(std::io::Error::other)?;
        fs::write(path, json)
    }

    /// List all recorded revelations, sorted by timestamp ascending.
    pub fn list(&self) -> Vec<Revelation> {
        let mut revelations = Vec::new();

        let entries = match fs::read_dir(&self.log_dir) {
            Ok(e) => e,
            Err(_) => return revelations,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match serde_json::from_str::<Revelation>(&content) {
                Ok(r) => revelations.push(r),
                Err(_) => continue,
            }
        }

        revelations.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        revelations
    }
}

/// Response format for GET /audit/revelations.
#[derive(Debug, Serialize, Deserialize)]
pub struct RevelationsListResponse {
    /// All recorded revelations.
    pub revelations: Vec<Revelation>,
    /// Total count.
    pub count: usize,
}
