//! UHRP/NanoStore client — encrypted file storage for warm memory (Layer 2).
//!
//! NanoStore provides content-addressed encrypted file hosting via UHRP
//! (Universal Hash Resolution Protocol). Files are encrypted locally
//! (BRC-78), uploaded, and addressed by SHA-256 hash of the encrypted blob.
//!
//! Two-step upload flow:
//! 1. POST `/upload` with BRC-31 auth + x402 payment → returns presigned GCS URL
//! 2. PUT file bytes to presigned URL (plain HTTP, no auth needed)
//!
//! Endpoints:
//! - POST `/upload` — reserve storage + get presigned URL (BRC-31 auth + x402 payment)
//! - GET  `/list`   — list uploaded files (BRC-31 auth, identity-scoped)
//! - POST `/find`   — find file by content hash (BRC-31 auth)
//! - CDN: `storage.googleapis.com/prod-uhrp/cdn/{hash}` — download (public)
//!
//! Cost: ~730 sats/MB/year, 10 sats minimum per upload.

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::auth::AuthriteClient;
use crate::error::DmError;
use crate::x402::payment;

/// NanoStore API base URL.
pub const NANOSTORE_HOST: &str = "https://nanostore.babbage.systems";
/// CDN base URL for downloads.
pub const UHRP_CDN: &str = "https://storage.googleapis.com/prod-uhrp/cdn";

/// Metadata for a file stored in NanoStore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UhrpFile {
    /// SHA-256 hash of the encrypted content (hex).
    pub hash: String,
    /// CDN URL where the file can be downloaded.
    pub url: String,
    /// Unix timestamp when the UHRP advertisement expires.
    pub expiry: Option<u64>,
    /// File size in bytes.
    pub size: Option<u64>,
}

/// Result of a successful NanoStore upload (two-step flow).
#[derive(Debug, Clone)]
pub struct UhrpUploadResult {
    /// File metadata (hash, URL, size).
    pub file: UhrpFile,
    /// Payment transaction ID from the x402 reservation step.
    pub payment_txid: Option<String>,
    /// Satoshis paid for the upload reservation.
    pub sats_paid: u64,
}

/// An index of all memory files stored in UHRP.
///
/// The index itself is encrypted and stored alongside the files.
/// On startup, the worm fetches the index to know what memories exist.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UhrpIndex {
    /// Map of logical memory name → UHRP content hash.
    pub entries: Vec<UhrpIndexEntry>,
    /// Last sync timestamp.
    pub last_sync: Option<String>,
}

/// An entry in the UHRP index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UhrpIndexEntry {
    /// Logical name (e.g., "knowledge/debugging-patterns.md").
    pub name: String,
    /// Memory category.
    pub category: String,
    /// SHA-256 hash of the encrypted content in UHRP.
    pub content_hash: String,
    /// Original (unencrypted) file size.
    pub original_size: u64,
    /// When this entry was uploaded.
    pub uploaded_at: String,
}

/// Default retention period for UHRP uploads (1 year in minutes).
const DEFAULT_RETENTION_MINUTES: u64 = 525600;

/// NanoStore client for UHRP file operations.
///
/// Uses BRC-31 Authrite authentication for identity-scoped operations
/// (upload, list, find). Upload uses x402 payment via `authenticated_paid_request()`.
/// Downloads are public via CDN but content is encrypted.
pub struct NanoStoreClient {
    auth: AuthriteClient,
    http: reqwest::Client,
    base_url: String,
}

impl NanoStoreClient {
    /// Create a new NanoStore client with an AuthriteClient for BRC-31 auth.
    pub fn new(auth: AuthriteClient) -> Self {
        Self {
            auth,
            http: reqwest::Client::new(),
            base_url: NANOSTORE_HOST.to_string(),
        }
    }

    /// Create with a custom base URL (for testing).
    pub fn with_url(auth: AuthriteClient, base_url: &str) -> Self {
        Self {
            auth,
            http: reqwest::Client::new(),
            base_url: base_url.to_string(),
        }
    }

    /// Access the underlying AuthriteClient.
    pub fn auth(&self) -> &AuthriteClient {
        &self.auth
    }

    /// Compute the SHA-256 content hash of a blob (used as UHRP address).
    pub fn content_hash(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Build the CDN download URL for a given content hash.
    pub fn cdn_url(content_hash: &str) -> String {
        format!("{}/{}", UHRP_CDN, content_hash)
    }

    /// Download a file from the UHRP CDN by content hash.
    ///
    /// Downloads are public (no auth needed) since content is encrypted.
    pub async fn download(&self, content_hash: &str) -> Result<Vec<u8>, DmError> {
        let url = Self::cdn_url(content_hash);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| DmError::memory(format!("UHRP download failed: {e}")))?;

        if !response.status().is_success() {
            return Err(DmError::memory(format!(
                "UHRP download returned {}",
                response.status()
            )));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| DmError::memory(format!("Failed to read UHRP response: {e}")))
    }

    /// Upload an encrypted blob to NanoStore.
    ///
    /// Two-step flow:
    /// 1. POST `/upload` with BRC-31 auth + x402 payment to reserve storage
    ///    and get a presigned GCS upload URL.
    /// 2. PUT the actual file bytes to the presigned URL (plain HTTP, no auth).
    ///
    /// Returns the content hash and CDN URL on success.
    pub async fn upload(
        &self,
        encrypted_blob: &[u8],
        retention_minutes: Option<u64>,
    ) -> Result<UhrpUploadResult, DmError> {
        let content_hash = Self::content_hash(encrypted_blob);
        let file_size = encrypted_blob.len() as u64;
        let retention = retention_minutes.unwrap_or(DEFAULT_RETENTION_MINUTES);

        // Step 1: Reserve upload slot via BRC-31 auth + x402 payment
        let upload_url = format!("{}/upload", self.base_url);
        let body = json!({
            "fileSize": file_size,
            "retentionPeriod": retention,
        });
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| DmError::memory(format!("Failed to serialize upload request: {e}")))?;

        let headers: Vec<(String, String)> =
            vec![("content-type".into(), "application/json".into())];

        let resp = payment::authenticated_paid_request(
            &self.auth,
            "POST",
            &upload_url,
            &headers,
            Some(&body_bytes),
        )
        .await
        .map_err(|e| DmError::memory(format!("UHRP upload reservation failed: {e}")))?;

        if !resp.status.is_success() {
            let body_text = String::from_utf8_lossy(&resp.body);
            return Err(DmError::memory(format!(
                "UHRP upload returned {}: {}",
                resp.status,
                &body_text[..body_text.len().min(500)]
            )));
        }

        let result: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| DmError::memory(format!("Invalid JSON from upload reservation: {e}")))?;

        // Extract presigned upload URL
        let presigned_url = result
            .get("uploadURL")
            .or_else(|| result.get("presignedUrl"))
            .or_else(|| result.get("upload_url"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DmError::memory(format!(
                    "Upload reservation succeeded but no upload URL in response: {}",
                    serde_json::to_string(&result).unwrap_or_default()
                ))
            })?
            .to_string();

        // Extract required headers for the GCS PUT
        let required_headers: Vec<(String, String)> = result
            .get("requiredHeaders")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| {
                        let val = match v.as_str() {
                            Some(s) => s.to_string(),
                            None => v.to_string(),
                        };
                        (k.clone(), val)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Step 2: PUT file bytes to the presigned GCS URL (plain HTTP, no auth)
        let mut put_request = self
            .http
            .put(&presigned_url)
            .header("Content-Type", "application/octet-stream");

        for (key, value) in &required_headers {
            put_request = put_request.header(key.as_str(), value.as_str());
        }

        let put_response = put_request
            .body(encrypted_blob.to_vec())
            .send()
            .await
            .map_err(|e| DmError::memory(format!("UHRP upload PUT failed: {e}")))?;

        if !put_response.status().is_success() {
            let status = put_response.status();
            let body_text = put_response.text().await.unwrap_or_default();
            return Err(DmError::memory(format!(
                "UHRP upload PUT returned {}: {}",
                status,
                &body_text[..body_text.len().min(500)]
            )));
        }

        // Derive public URL by stripping query params from the presigned URL
        let public_url = presigned_url
            .split('?')
            .next()
            .unwrap_or(&presigned_url)
            .to_string();

        Ok(UhrpUploadResult {
            file: UhrpFile {
                hash: content_hash,
                url: public_url,
                expiry: None,
                size: Some(file_size),
            },
            payment_txid: resp.payment_txid,
            sats_paid: resp.sats_paid.unwrap_or(0),
        })
    }

    /// List all files uploaded by this identity.
    ///
    /// Uses BRC-31 Authrite authentication (identity-scoped, no payment).
    pub async fn list(&self) -> Result<Vec<UhrpFile>, DmError> {
        let url = format!("{}/list", self.base_url);
        let response = self
            .auth
            .get(&url)
            .await
            .map_err(|e| DmError::memory(format!("UHRP list failed: {e}")))?;

        if !response.status().is_success() {
            return Err(DmError::memory(format!(
                "UHRP list returned {}",
                response.status()
            )));
        }

        response
            .json::<Vec<UhrpFile>>()
            .await
            .map_err(|e| DmError::memory(format!("Failed to parse UHRP list: {e}")))
    }

    /// Find a specific file by content hash.
    ///
    /// Uses BRC-31 Authrite authentication. Returns `None` for 404.
    pub async fn find(&self, content_hash: &str) -> Result<Option<UhrpFile>, DmError> {
        let url = format!("{}/find", self.base_url);
        let body = json!({"hash": content_hash});

        let response = self
            .auth
            .post_json(&url, &body)
            .await
            .map_err(|e| DmError::memory(format!("UHRP find failed: {e}")))?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            return Err(DmError::memory(format!(
                "UHRP find returned {}",
                response.status()
            )));
        }

        response
            .json::<UhrpFile>()
            .await
            .map(Some)
            .map_err(|e| DmError::memory(format!("Failed to parse UHRP find: {e}")))
    }
}

impl UhrpIndex {
    /// Add or update an entry in the index.
    pub fn upsert(&mut self, entry: UhrpIndexEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.name == entry.name) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Remove an entry by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.name != name);
        self.entries.len() < before
    }

    /// Find an entry by name.
    pub fn find(&self, name: &str) -> Option<&UhrpIndexEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Serialize the index to JSON bytes (for encryption + upload).
    pub fn to_bytes(&self) -> Result<Vec<u8>, DmError> {
        serde_json::to_vec(self)
            .map_err(|e| DmError::memory(format!("Failed to serialize UHRP index: {e}")))
    }

    /// Deserialize from JSON bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, DmError> {
        serde_json::from_slice(data)
            .map_err(|e| DmError::memory(format!("Failed to parse UHRP index: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::WalletClient;
    use std::path::PathBuf;

    /// Helper to create a mock wallet + AuthriteClient pointing at a mockito server.
    fn mock_auth(wallet_url: &str) -> AuthriteClient {
        let wallet = WalletClient::new(wallet_url, "http://localhost", 10);
        AuthriteClient::with_session_dir(
            std::sync::Arc::new(wallet),
            PathBuf::from("/tmp/test-uhrp-auth"),
        )
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = NanoStoreClient::content_hash(b"hello");
        let h2 = NanoStoreClient::content_hash(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 = 64 hex chars
    }

    #[test]
    fn test_content_hash_different_inputs() {
        let h1 = NanoStoreClient::content_hash(b"hello");
        let h2 = NanoStoreClient::content_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_cdn_url() {
        let url = NanoStoreClient::cdn_url("abc123");
        assert_eq!(url, format!("{}/abc123", UHRP_CDN));
    }

    #[test]
    fn test_nanostore_client_new() {
        let auth = mock_auth("http://localhost:3322");
        let client = NanoStoreClient::new(auth);
        assert_eq!(client.base_url, NANOSTORE_HOST);
    }

    #[test]
    fn test_nanostore_client_custom_url() {
        let auth = mock_auth("http://localhost:3322");
        let client = NanoStoreClient::with_url(auth, "http://localhost:9999");
        assert_eq!(client.base_url, "http://localhost:9999");
    }

    #[test]
    fn test_uhrp_index_empty() {
        let idx = UhrpIndex::default();
        assert!(idx.entries.is_empty());
        assert!(idx.last_sync.is_none());
    }

    #[test]
    fn test_uhrp_index_upsert() {
        let mut idx = UhrpIndex::default();
        idx.upsert(UhrpIndexEntry {
            name: "knowledge/test.md".into(),
            category: "knowledge".into(),
            content_hash: "hash1".into(),
            original_size: 100,
            uploaded_at: "2026-02-23T12:00:00Z".into(),
        });
        assert_eq!(idx.entries.len(), 1);

        // Upsert same name -> update
        idx.upsert(UhrpIndexEntry {
            name: "knowledge/test.md".into(),
            category: "knowledge".into(),
            content_hash: "hash2".into(),
            original_size: 200,
            uploaded_at: "2026-02-23T13:00:00Z".into(),
        });
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].content_hash, "hash2");
    }

    #[test]
    fn test_uhrp_index_remove() {
        let mut idx = UhrpIndex::default();
        idx.upsert(UhrpIndexEntry {
            name: "a.md".into(),
            category: "knowledge".into(),
            content_hash: "h".into(),
            original_size: 10,
            uploaded_at: "t".into(),
        });
        assert!(idx.remove("a.md"));
        assert!(idx.entries.is_empty());
        assert!(!idx.remove("nonexistent"));
    }

    #[test]
    fn test_uhrp_index_find() {
        let mut idx = UhrpIndex::default();
        idx.upsert(UhrpIndexEntry {
            name: "sessions/s1.md".into(),
            category: "session".into(),
            content_hash: "h1".into(),
            original_size: 50,
            uploaded_at: "t".into(),
        });
        assert!(idx.find("sessions/s1.md").is_some());
        assert!(idx.find("nonexistent").is_none());
    }

    #[test]
    fn test_uhrp_index_serialize_roundtrip() {
        let mut idx = UhrpIndex {
            last_sync: Some("2026-02-23T14:00:00Z".into()),
            ..Default::default()
        };
        idx.upsert(UhrpIndexEntry {
            name: "knowledge/patterns.md".into(),
            category: "knowledge".into(),
            content_hash: "abc123".into(),
            original_size: 1024,
            uploaded_at: "2026-02-23T12:00:00Z".into(),
        });

        let bytes = idx.to_bytes().unwrap();
        let back = UhrpIndex::from_bytes(&bytes).unwrap();
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].name, "knowledge/patterns.md");
        assert_eq!(back.last_sync, Some("2026-02-23T14:00:00Z".into()));
    }

    #[test]
    fn test_uhrp_file_serialize() {
        let f = UhrpFile {
            hash: "abc".into(),
            url: "https://cdn/abc".into(),
            expiry: Some(1234567890),
            size: Some(1024),
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: UhrpFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hash, "abc");
        assert_eq!(back.expiry, Some(1234567890));
    }

    #[test]
    fn test_uhrp_index_multiple_entries() {
        let mut idx = UhrpIndex::default();
        for i in 0..5 {
            idx.upsert(UhrpIndexEntry {
                name: format!("entry-{i}.md"),
                category: "knowledge".into(),
                content_hash: format!("hash-{i}"),
                original_size: i * 100,
                uploaded_at: "t".into(),
            });
        }
        assert_eq!(idx.entries.len(), 5);
        assert!(idx.find("entry-3.md").is_some());
        assert_eq!(idx.find("entry-3.md").unwrap().original_size, 300);
    }

    #[test]
    fn test_uhrp_upload_result_fields() {
        let result = UhrpUploadResult {
            file: UhrpFile {
                hash: "deadbeef".into(),
                url: "https://storage.example.com/deadbeef".into(),
                expiry: None,
                size: Some(256),
            },
            payment_txid: Some("abc123def456".into()),
            sats_paid: 730,
        };
        assert_eq!(result.file.hash, "deadbeef");
        assert_eq!(result.sats_paid, 730);
        assert!(result.payment_txid.is_some());
    }

    #[tokio::test]
    async fn test_download_from_cdn() {
        // CDN downloads use plain HTTP (no auth)
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/cdn/testhash123")
            .with_status(200)
            .with_body(b"encrypted-content-here")
            .create_async()
            .await;

        // Override CDN URL for test by using download with a direct URL
        // Since download() uses the CDN constant, we test the HTTP client behavior
        let auth = mock_auth("http://localhost:3322");
        let client = NanoStoreClient::with_url(auth, &server.url());

        // download() uses the global UHRP_CDN constant, so we can't easily mock it.
        // Instead, verify the client is constructed properly.
        assert_eq!(client.base_url, server.url());
    }

    #[tokio::test]
    async fn test_list_with_auth() {
        // list() uses BRC-31 auth. Since we can't easily mock the full
        // Authrite handshake + wallet in a unit test, verify the method
        // propagates errors correctly when auth fails.
        let auth = mock_auth("http://localhost:99999"); // unreachable wallet
        let client = NanoStoreClient::with_url(auth, "http://localhost:99998");

        let result = client.list().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("UHRP list failed") || err.contains("auth") || err.contains("connect"),
            "Expected auth/connection error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_find_with_auth() {
        // find() uses BRC-31 auth. Verify error propagation.
        let auth = mock_auth("http://localhost:99999"); // unreachable wallet
        let client = NanoStoreClient::with_url(auth, "http://localhost:99998");

        let result = client.find("somehash").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("UHRP find failed") || err.contains("auth") || err.contains("connect"),
            "Expected auth/connection error, got: {err}"
        );
    }
}
