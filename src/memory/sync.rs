//! Memory sync — orchestrates encryption + UHRP for persistent memory.
//!
//! Provides encrypted local file storage that can be synced to/from UHRP.
//! The local encrypted store works independently of NanoStore, providing
//! immediate value while the Authrite integration matures.
//!
//! Sync flow:
//! 1. Memory write → encrypt via wallet → write `.enc` file locally → (optional) upload to UHRP
//! 2. Boot → read `.enc` files → decrypt via wallet → populate MemoryStore + MemoryIndex
//! 3. (Future) Boot → download from UHRP → decrypt → populate

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::DmError;
use crate::memory::encrypt;
use crate::memory::store::MemoryEntry;
use crate::memory::uhrp::{UhrpIndex, UhrpIndexEntry};
use crate::wallet::WalletBackend;

fn memory_protocol_id() -> serde_json::Value {
    serde_json::json!([2, "dolphin milk memory"])
}

/// Manages encrypted memory file storage.
///
/// Writes encrypted `.enc` files alongside the plain `.md` files.
/// Maintains a UHRP index for tracking what's been synced.
pub struct MemorySync {
    /// Directory for encrypted files.
    encrypted_dir: PathBuf,
    /// UHRP index tracking encrypted file hashes.
    index: UhrpIndex,
}

impl MemorySync {
    /// Create a new MemorySync, initializing the encrypted storage directory.
    pub fn new(memory_base_dir: &Path) -> Self {
        let encrypted_dir = memory_base_dir.join("encrypted");
        std::fs::create_dir_all(&encrypted_dir).ok();
        Self {
            encrypted_dir,
            index: UhrpIndex::default(),
        }
    }

    /// Get the encrypted directory path.
    pub fn encrypted_dir(&self) -> &Path {
        &self.encrypted_dir
    }

    /// Get a reference to the UHRP index.
    pub fn index(&self) -> &UhrpIndex {
        &self.index
    }

    /// Encrypt a memory entry and save it locally as a `.enc` file.
    ///
    /// Uses the wallet's native encrypt endpoint with the given key_id
    /// (typically the memory category: "knowledge", "sessions", "execution").
    /// Returns the content hash of the encrypted blob.
    pub async fn encrypt_and_store(
        &mut self,
        entry: &MemoryEntry,
        wallet: &dyn WalletBackend,
        key_id: &str,
    ) -> Result<String, DmError> {
        let plaintext = entry.content.as_bytes();

        let blob =
            encrypt::encrypt_with_wallet(wallet, plaintext, &memory_protocol_id(), key_id, "self")
                .await?;

        let content_hash = sha256_hex(&blob);

        let enc_path = self.encrypted_dir.join(format!("{}.enc", entry.id));
        std::fs::write(&enc_path, &blob)
            .map_err(|e| DmError::memory(format!("Failed to write encrypted file: {e}")))?;

        let logical_name = format!("{}/{}.md", entry.category, entry.id);
        self.index.upsert(UhrpIndexEntry {
            name: logical_name,
            category: entry.category.to_string(),
            content_hash: content_hash.clone(),
            original_size: plaintext.len() as u64,
            uploaded_at: chrono::Utc::now().to_rfc3339(),
        });

        Ok(content_hash)
    }

    /// Decrypt an encrypted file by entry ID.
    ///
    /// Checks for legacy format first — if the blob starts with magic bytes
    /// `0x42421033`, falls back to legacy decryption (requires wallet for
    /// key derivation). Otherwise uses the wallet's native decrypt endpoint.
    pub async fn decrypt_entry(
        &self,
        entry_id: &str,
        wallet: &dyn WalletBackend,
        key_id: &str,
    ) -> Result<String, DmError> {
        let enc_path = self.encrypted_dir.join(format!("{}.enc", entry_id));

        let blob = std::fs::read(&enc_path)
            .map_err(|e| DmError::memory(format!("Failed to read encrypted file: {e}")))?;

        let plaintext = if encrypt::is_legacy_format(&blob) {
            // Legacy format: extract key_id from blob, derive key, decrypt
            let legacy_key_id = encrypt::legacy_extract_key_id(&blob)?;
            let aes_key = encrypt::legacy_derive_key(wallet, &legacy_key_id).await?;
            encrypt::legacy_decrypt(&aes_key, &blob)?
        } else {
            // Wallet-native format
            encrypt::decrypt_with_wallet(wallet, &blob, &memory_protocol_id(), key_id, "self")
                .await?
        };

        String::from_utf8(plaintext)
            .map_err(|e| DmError::memory(format!("Decrypted content is not valid UTF-8: {e}")))
    }

    /// List all encrypted files in the local store.
    pub fn list_encrypted(&self) -> Result<Vec<String>, DmError> {
        let mut ids = Vec::new();
        let entries = std::fs::read_dir(&self.encrypted_dir)
            .map_err(|e| DmError::memory(format!("Failed to read encrypted dir: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| DmError::memory(format!("Dir entry error: {e}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_suffix(".enc") {
                ids.push(id.to_string());
            }
        }
        Ok(ids)
    }

    /// Delete an encrypted file by entry ID.
    pub fn delete_encrypted(&mut self, entry_id: &str) -> Result<(), DmError> {
        let enc_path = self.encrypted_dir.join(format!("{}.enc", entry_id));
        if enc_path.exists() {
            std::fs::remove_file(&enc_path)
                .map_err(|e| DmError::memory(format!("Failed to delete encrypted file: {e}")))?;
        }
        let logical_pattern = format!("/{}.md", entry_id);
        self.index
            .entries
            .retain(|e| !e.name.ends_with(&logical_pattern));
        Ok(())
    }

    /// Save the UHRP index to disk (encrypted via wallet).
    pub async fn save_index(
        &self,
        wallet: &dyn WalletBackend,
        key_id: &str,
    ) -> Result<(), DmError> {
        let index_bytes = self.index.to_bytes()?;
        let blob = encrypt::encrypt_with_wallet(
            wallet,
            &index_bytes,
            &memory_protocol_id(),
            key_id,
            "self",
        )
        .await?;
        let index_path = self.encrypted_dir.join("index.enc");
        std::fs::write(&index_path, blob)
            .map_err(|e| DmError::memory(format!("Failed to write encrypted index: {e}")))
    }

    /// Load the UHRP index from disk (decrypted via wallet).
    ///
    /// Handles legacy-format index files transparently.
    pub async fn load_index(
        &mut self,
        wallet: &dyn WalletBackend,
        key_id: &str,
    ) -> Result<(), DmError> {
        let index_path = self.encrypted_dir.join("index.enc");
        if !index_path.exists() {
            return Ok(()); // No index yet — start fresh
        }

        let blob = std::fs::read(&index_path)
            .map_err(|e| DmError::memory(format!("Failed to read encrypted index: {e}")))?;

        let plaintext = if encrypt::is_legacy_format(&blob) {
            let legacy_key_id = encrypt::legacy_extract_key_id(&blob)?;
            let aes_key = encrypt::legacy_derive_key(wallet, &legacy_key_id).await?;
            encrypt::legacy_decrypt(&aes_key, &blob)?
        } else {
            encrypt::decrypt_with_wallet(wallet, &blob, &memory_protocol_id(), key_id, "self")
                .await?
        };

        self.index = UhrpIndex::from_bytes(&plaintext)?;
        Ok(())
    }

    /// Get a summary of the encrypted store for diagnostics.
    pub fn summary(&self) -> SyncSummary {
        let file_count = self.list_encrypted().map(|v| v.len()).unwrap_or(0);
        SyncSummary {
            encrypted_dir: self.encrypted_dir.clone(),
            file_count,
            index_entries: self.index.entries.len(),
            last_sync: self.index.last_sync.clone(),
        }
    }
}

/// Diagnostic summary of the sync state.
#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub encrypted_dir: PathBuf,
    pub file_count: usize,
    pub index_entries: usize,
    pub last_sync: Option<String>,
}

/// SHA-256 hex hash of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::MemoryCategory;
    use chrono::Utc;

    fn test_entry(id: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            category: MemoryCategory::Knowledge,
            content: content.to_string(),
            tags: vec!["test".into()],
            created: Utc::now(),
            source: "test".to_string(),
        }
    }

    /// Create a mock wallet that responds to encrypt/decrypt.
    async fn mock_wallet() -> (mockito::ServerGuard, crate::wallet::WalletClient) {
        let server = mockito::Server::new_async().await;
        let wallet = crate::wallet::WalletClient::new(&server.url(), "http://localhost", 10);
        (server, wallet)
    }

    /// Set up encrypt mock that echoes back "encrypted:" + plaintext bytes.
    async fn setup_encrypt_mock(server: &mut mockito::ServerGuard) -> mockito::Mock {
        // We can't easily echo, so we return a fixed ciphertext for tests
        server
            .mock("POST", "/encrypt")
            .with_status(200)
            .with_body(r#"{"ciphertext":[99,105,112,104,101,114]}"#)
            .create_async()
            .await
    }

    /// Set up decrypt mock that returns fixed plaintext.
    async fn setup_decrypt_mock(
        server: &mut mockito::ServerGuard,
        plaintext: &str,
    ) -> mockito::Mock {
        let bytes: Vec<u8> = plaintext.bytes().collect();
        let body = format!(
            r#"{{"plaintext":{}}}"#,
            serde_json::to_string(&bytes).unwrap()
        );
        server
            .mock("POST", "/decrypt")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await
    }

    #[test]
    fn test_sync_creates_encrypted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let sync = MemorySync::new(dir.path());
        assert!(sync.encrypted_dir().is_dir());
    }

    #[tokio::test]
    async fn test_encrypt_store_and_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        let mut sync = MemorySync::new(dir.path());
        let (mut server, wallet) = mock_wallet().await;

        let _enc_mock = setup_encrypt_mock(&mut server).await;
        let _dec_mock = setup_decrypt_mock(&mut server, "secret knowledge about BSV").await;

        let entry = test_entry("entry-001", "secret knowledge about BSV");
        let hash = sync
            .encrypt_and_store(&entry, &wallet, "knowledge")
            .await
            .unwrap();
        assert_eq!(hash.len(), 64); // SHA-256 hex

        let decrypted = sync
            .decrypt_entry("entry-001", &wallet, "knowledge")
            .await
            .unwrap();
        assert_eq!(decrypted, "secret knowledge about BSV");
    }

    #[tokio::test]
    async fn test_list_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let mut sync = MemorySync::new(dir.path());
        let (mut server, wallet) = mock_wallet().await;

        let _enc_mock = setup_encrypt_mock(&mut server).await;

        sync.encrypt_and_store(&test_entry("a", "data a"), &wallet, "knowledge")
            .await
            .unwrap();
        sync.encrypt_and_store(&test_entry("b", "data b"), &wallet, "knowledge")
            .await
            .unwrap();

        let mut ids = sync.list_encrypted().unwrap();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_delete_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let mut sync = MemorySync::new(dir.path());
        let (mut server, wallet) = mock_wallet().await;

        let _enc_mock = setup_encrypt_mock(&mut server).await;

        sync.encrypt_and_store(&test_entry("del-me", "content"), &wallet, "knowledge")
            .await
            .unwrap();
        assert_eq!(sync.list_encrypted().unwrap().len(), 1);

        sync.delete_encrypted("del-me").unwrap();
        assert_eq!(sync.list_encrypted().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_index_updated_on_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut sync = MemorySync::new(dir.path());
        let (mut server, wallet) = mock_wallet().await;

        let _enc_mock = setup_encrypt_mock(&mut server).await;

        sync.encrypt_and_store(&test_entry("idx-test", "content"), &wallet, "knowledge")
            .await
            .unwrap();
        assert_eq!(sync.index().entries.len(), 1);
        assert!(sync.index().find("knowledge/idx-test.md").is_some());
    }

    #[tokio::test]
    async fn test_save_and_load_index() {
        let dir = tempfile::tempdir().unwrap();
        let (mut server, wallet) = mock_wallet().await;

        // We need the encrypt mock to return something, and the decrypt mock
        // to return the serialized index
        let _enc_mock = setup_encrypt_mock(&mut server).await;

        // Create and save
        {
            let mut sync = MemorySync::new(dir.path());
            sync.encrypt_and_store(&test_entry("persist", "data"), &wallet, "knowledge")
                .await
                .unwrap();
            sync.save_index(&wallet, "index").await.unwrap();
        }

        // For load, we need to set up a decrypt mock that returns the index JSON.
        // Since the saved file uses wallet ciphertext (the mock returns [99,105,...]),
        // the load will call decrypt with that ciphertext.
        // We need to return a valid UhrpIndex JSON.
        let index_json = serde_json::json!({
            "entries": [{
                "name": "knowledge/persist.md",
                "category": "knowledge",
                "content_hash": "abc123",
                "original_size": 4,
                "uploaded_at": "2026-02-24T00:00:00Z"
            }],
            "last_sync": null
        });
        let index_bytes: Vec<u8> = serde_json::to_vec(&index_json).unwrap();
        let body = format!(
            r#"{{"plaintext":{}}}"#,
            serde_json::to_string(&index_bytes).unwrap()
        );
        let _dec_mock = server
            .mock("POST", "/decrypt")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        // Load in new instance
        {
            let mut sync = MemorySync::new(dir.path());
            assert!(sync.index().entries.is_empty());
            sync.load_index(&wallet, "index").await.unwrap();
            assert_eq!(sync.index().entries.len(), 1);
            assert!(sync.index().find("knowledge/persist.md").is_some());
        }
    }

    #[tokio::test]
    async fn test_load_index_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_server, wallet) = mock_wallet().await;
        let mut sync = MemorySync::new(dir.path());
        // Should succeed with empty index when no file exists
        sync.load_index(&wallet, "index").await.unwrap();
        assert!(sync.index().entries.is_empty());
    }

    #[tokio::test]
    async fn test_summary() {
        let dir = tempfile::tempdir().unwrap();
        let mut sync = MemorySync::new(dir.path());
        let (mut server, wallet) = mock_wallet().await;

        let _enc_mock = setup_encrypt_mock(&mut server).await;

        sync.encrypt_and_store(&test_entry("s1", "data"), &wallet, "knowledge")
            .await
            .unwrap();
        sync.encrypt_and_store(&test_entry("s2", "data"), &wallet, "knowledge")
            .await
            .unwrap();

        let summary = sync.summary();
        assert_eq!(summary.file_count, 2);
        assert_eq!(summary.index_entries, 2);
    }
}
