//! Memory encryption — wallet-native encrypt/decrypt via BRC-42 key derivation.
//!
//! Primary API: `encrypt_with_wallet()` / `decrypt_with_wallet()` delegate all
//! cryptography to the wallet at localhost:3322. The wallet handles key
//! derivation (BRC-42) and AES-256-GCM internally.
//!
//! Legacy API: `legacy_encrypt()` / `legacy_decrypt()` support reading `.enc`
//! files written with the old SymmetricKey-based format (magic `0x42421033`).
//!
//! Protocol ID conventions:
//!   - Memory: `[2, "dolphin milk memory"]`, key_id = category ("knowledge", "sessions", etc.)
//!   - State:  `[2, "dolphin milk state"]`,  key_id = "tokens"
//!   - Counterparty: always "self" (encrypt-to-self)

use bsv::SymmetricKey;
use sha2::{Digest, Sha256};

use crate::error::DmError;
use crate::wallet::WalletBackend;

/// Magic bytes identifying a legacy encrypted memory blob.
pub const ENCRYPT_VERSION: [u8; 4] = [0x42, 0x42, 0x10, 0x33];

/// Legacy key ID size (256 bits).
const KEY_ID_SIZE: usize = 32;
/// Legacy header size: version + key_id.
const HEADER_SIZE: usize = 4 + KEY_ID_SIZE;
/// Minimum SDK blob size: IV (32) + GCM tag (16) for empty plaintext.
const MIN_SDK_BLOB: usize = 48;

// ─────────────────────────────────────────────────────────────────────────────
// Wallet-native encrypt/decrypt
// ─────────────────────────────────────────────────────────────────────────────

/// Encrypt plaintext using the wallet's native encrypt endpoint.
///
/// The wallet handles BRC-42 key derivation + AES-256-GCM internally.
/// Returns the wallet's ciphertext blob (format is wallet-defined).
///
/// `counterparty` is typically `"self"` for encrypt-to-self, or a hex
/// public key for two-party BRC-42 ECDH encryption.
pub async fn encrypt_with_wallet(
    wallet: &dyn WalletBackend,
    plaintext: &[u8],
    protocol_id: &serde_json::Value,
    key_id: &str,
    counterparty: &str,
) -> Result<Vec<u8>, DmError> {
    wallet
        .encrypt(plaintext, protocol_id, key_id, counterparty)
        .await
}

/// Decrypt ciphertext using the wallet's native decrypt endpoint.
///
/// Returns the decrypted plaintext bytes.
///
/// `counterparty` must match the value used during encryption.
pub async fn decrypt_with_wallet(
    wallet: &dyn WalletBackend,
    ciphertext: &[u8],
    protocol_id: &serde_json::Value,
    key_id: &str,
    counterparty: &str,
) -> Result<Vec<u8>, DmError> {
    wallet
        .decrypt(ciphertext, protocol_id, key_id, counterparty)
        .await
}

/// Check if a blob uses the legacy encryption format (magic `0x42421033`).
pub fn is_legacy_format(blob: &[u8]) -> bool {
    blob.len() >= 4 && blob[..4] == ENCRYPT_VERSION
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy encrypt/decrypt (for reading old .enc files)
// ─────────────────────────────────────────────────────────────────────────────

/// Legacy: encrypt plaintext with a 32-byte AES-256-GCM key.
///
/// Returns: `[VERSION:4] [KEY_ID:32] [IV:32] [CIPHERTEXT:var] [TAG:16]`
///
/// Uses the BSV SDK's `SymmetricKey` (32-byte nonce for cross-SDK compat).
pub fn legacy_encrypt(
    key: &[u8; 32],
    key_id: &[u8; 32],
    plaintext: &[u8],
) -> Result<Vec<u8>, DmError> {
    let sym_key = SymmetricKey::from_bytes(key)
        .map_err(|e| DmError::memory(format!("SymmetricKey init failed: {e}")))?;

    let sdk_blob = sym_key
        .encrypt(plaintext)
        .map_err(|e| DmError::memory(format!("Encryption failed: {e}")))?;

    let mut blob = Vec::with_capacity(HEADER_SIZE + sdk_blob.len());
    blob.extend_from_slice(&ENCRYPT_VERSION);
    blob.extend_from_slice(key_id);
    blob.extend_from_slice(&sdk_blob);

    Ok(blob)
}

/// Legacy: decrypt a blob produced by `legacy_encrypt()`.
pub fn legacy_decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, DmError> {
    if blob.len() < HEADER_SIZE + MIN_SDK_BLOB {
        return Err(DmError::memory(format!(
            "Encrypted blob too short: {} bytes (need at least {})",
            blob.len(),
            HEADER_SIZE + MIN_SDK_BLOB
        )));
    }

    if blob[..4] != ENCRYPT_VERSION {
        return Err(DmError::memory(format!(
            "Invalid encryption version: {:02x}{:02x}{:02x}{:02x}",
            blob[0], blob[1], blob[2], blob[3]
        )));
    }

    let sdk_blob = &blob[HEADER_SIZE..];

    let sym_key = SymmetricKey::from_bytes(key)
        .map_err(|e| DmError::memory(format!("SymmetricKey init failed: {e}")))?;

    sym_key
        .decrypt(sdk_blob)
        .map_err(|e| DmError::memory(format!("Decryption failed (wrong key or tampered): {e}")))
}

/// Legacy: extract the key_id from a legacy encrypted blob.
pub fn legacy_extract_key_id(blob: &[u8]) -> Result<[u8; 32], DmError> {
    if blob.len() < HEADER_SIZE + MIN_SDK_BLOB {
        return Err(DmError::memory("Blob too short to extract key_id"));
    }
    if blob[..4] != ENCRYPT_VERSION {
        return Err(DmError::memory("Invalid encryption version"));
    }
    let mut key_id = [0u8; 32];
    key_id.copy_from_slice(&blob[4..36]);
    Ok(key_id)
}

/// Legacy: derive an AES-256 key from the wallet using BRC-42 + SHA-256(signature).
///
/// Kept for decrypting old `.enc` files that used this key derivation path.
pub async fn legacy_derive_key(
    wallet: &dyn WalletBackend,
    key_id: &[u8; 32],
) -> Result<[u8; 32], DmError> {
    let identity_key = wallet.get_identity_key().await?;
    let key_id_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key_id);

    let derive_msg = b"brc78-derive-encryption-key";
    let protocol_id = serde_json::json!([2, "message encryption"]);

    let signature = wallet
        .create_signature(derive_msg, &protocol_id, &key_id_b64, &identity_key)
        .await?;

    let mut hasher = Sha256::new();
    hasher.update(&signature);
    Ok(hasher.finalize().into())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::WalletClient;

    fn test_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = 0xDE;
        k[31] = 0xAD;
        k
    }

    fn test_key_id() -> [u8; 32] {
        [0x42u8; 32]
    }

    // -- Legacy format tests (reading old .enc files) --

    #[test]
    fn test_legacy_encrypt_decrypt_roundtrip() {
        let key = test_key();
        let key_id = test_key_id();
        let plaintext = b"Hello, BRC-78 encryption!";

        let blob = legacy_encrypt(&key, &key_id, plaintext).unwrap();
        let decrypted = legacy_decrypt(&key, &blob).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_legacy_encrypt_decrypt_empty() {
        let key = test_key();
        let key_id = test_key_id();

        let blob = legacy_encrypt(&key, &key_id, b"").unwrap();
        let decrypted = legacy_decrypt(&key, &blob).unwrap();
        assert_eq!(decrypted, b"");
    }

    #[test]
    fn test_legacy_encrypt_decrypt_large() {
        let key = test_key();
        let key_id = test_key_id();
        let plaintext = vec![0xABu8; 100_000];

        let blob = legacy_encrypt(&key, &key_id, &plaintext).unwrap();
        let decrypted = legacy_decrypt(&key, &blob).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_legacy_blob_has_version_header() {
        let key = test_key();
        let key_id = test_key_id();
        let blob = legacy_encrypt(&key, &key_id, b"test").unwrap();
        assert_eq!(&blob[..4], &ENCRYPT_VERSION);
    }

    #[test]
    fn test_legacy_blob_has_key_id() {
        let key = test_key();
        let key_id = test_key_id();
        let blob = legacy_encrypt(&key, &key_id, b"test").unwrap();
        assert_eq!(&blob[4..36], &key_id);
    }

    #[test]
    fn test_legacy_extract_key_id() {
        let key = test_key();
        let key_id = test_key_id();
        let blob = legacy_encrypt(&key, &key_id, b"test").unwrap();
        let extracted = legacy_extract_key_id(&blob).unwrap();
        assert_eq!(extracted, key_id);
    }

    #[test]
    fn test_legacy_different_key_id_different_blob() {
        let key = test_key();
        let kid1 = [0x01u8; 32];
        let kid2 = [0x02u8; 32];
        let blob1 = legacy_encrypt(&key, &kid1, b"same data").unwrap();
        let blob2 = legacy_encrypt(&key, &kid2, b"same data").unwrap();
        assert_ne!(blob1, blob2);
        assert_eq!(legacy_decrypt(&key, &blob1).unwrap(), b"same data");
        assert_eq!(legacy_decrypt(&key, &blob2).unwrap(), b"same data");
    }

    #[test]
    fn test_legacy_wrong_key_fails_decrypt() {
        let key1 = test_key();
        let mut key2 = test_key();
        key2[0] = 0xFF;
        let key_id = test_key_id();

        let blob = legacy_encrypt(&key1, &key_id, b"secret").unwrap();
        let result = legacy_decrypt(&key2, &blob);
        assert!(result.is_err(), "Wrong key should fail decryption");
    }

    #[test]
    fn test_legacy_tampered_ciphertext_fails() {
        let key = test_key();
        let key_id = test_key_id();
        let mut blob = legacy_encrypt(&key, &key_id, b"secret data").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;
        let result = legacy_decrypt(&key, &blob);
        assert!(result.is_err(), "Tampered ciphertext should fail");
    }

    #[test]
    fn test_legacy_truncated_blob_fails() {
        let key = test_key();
        let result = legacy_decrypt(&key, &[0x42, 0x42, 0x10]);
        assert!(result.is_err(), "Too-short blob should fail");
    }

    #[test]
    fn test_legacy_wrong_version_fails() {
        let key = test_key();
        let key_id = test_key_id();
        let mut blob = legacy_encrypt(&key, &key_id, b"test").unwrap();
        blob[0] = 0xFF;
        let result = legacy_decrypt(&key, &blob);
        assert!(result.is_err(), "Wrong version should fail");
    }

    #[test]
    fn test_legacy_blob_minimum_size() {
        let key = test_key();
        let key_id = test_key_id();
        let blob = legacy_encrypt(&key, &key_id, b"").unwrap();
        assert!(blob.len() >= HEADER_SIZE + MIN_SDK_BLOB);
    }

    #[test]
    fn test_legacy_encrypt_is_non_deterministic() {
        let key = test_key();
        let key_id = test_key_id();
        let blob1 = legacy_encrypt(&key, &key_id, b"same input").unwrap();
        let blob2 = legacy_encrypt(&key, &key_id, b"same input").unwrap();
        assert_ne!(blob1, blob2);
        assert_eq!(legacy_decrypt(&key, &blob1).unwrap(), b"same input");
        assert_eq!(legacy_decrypt(&key, &blob2).unwrap(), b"same input");
    }

    // -- is_legacy_format tests --

    #[test]
    fn test_is_legacy_format_true() {
        let key = test_key();
        let key_id = test_key_id();
        let blob = legacy_encrypt(&key, &key_id, b"test").unwrap();
        assert!(is_legacy_format(&blob));
    }

    #[test]
    fn test_is_legacy_format_false_for_short_data() {
        assert!(!is_legacy_format(&[0x42, 0x42]));
        assert!(!is_legacy_format(&[]));
    }

    #[test]
    fn test_is_legacy_format_false_for_wallet_ciphertext() {
        // Wallet ciphertext won't start with 0x42421033
        let fake_wallet_blob = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        assert!(!is_legacy_format(&fake_wallet_blob));
    }

    // -- Wallet-based encrypt/decrypt tests (mockito) --

    #[tokio::test]
    async fn test_wallet_encrypt_calls_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/encrypt")
            .match_body(mockito::Matcher::JsonString(
                serde_json::json!({
                    "plaintext": [104, 101, 108, 108, 111],
                    "protocolID": [2, "dolphin milk memory"],
                    "keyID": "sessions",
                    "counterparty": "self"
                })
                .to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"ciphertext":[1,2,3,4,5]}"#)
            .create_async()
            .await;

        let wallet = WalletClient::new(&server.url(), "http://localhost", 10);
        let protocol_id = serde_json::json!([2, "dolphin milk memory"]);
        let result = encrypt_with_wallet(&wallet, b"hello", &protocol_id, "sessions", "self").await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn test_wallet_decrypt_calls_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/decrypt")
            .match_body(mockito::Matcher::JsonString(
                serde_json::json!({
                    "ciphertext": [1, 2, 3, 4, 5],
                    "protocolID": [2, "dolphin milk memory"],
                    "keyID": "sessions",
                    "counterparty": "self"
                })
                .to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"plaintext":[104,101,108,108,111]}"#)
            .create_async()
            .await;

        let wallet = WalletClient::new(&server.url(), "http://localhost", 10);
        let protocol_id = serde_json::json!([2, "dolphin milk memory"]);
        let result =
            decrypt_with_wallet(&wallet, &[1, 2, 3, 4, 5], &protocol_id, "sessions", "self").await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn test_wallet_encrypt_error_propagates() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/encrypt")
            .with_status(500)
            .with_body(r#"{"error":"internal failure"}"#)
            .create_async()
            .await;

        let wallet = WalletClient::new(&server.url(), "http://localhost", 10);
        let protocol_id = serde_json::json!([2, "dolphin milk memory"]);
        let result = encrypt_with_wallet(&wallet, b"hello", &protocol_id, "test", "self").await;
        assert!(result.is_err());
    }
}
