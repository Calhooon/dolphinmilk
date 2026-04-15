//! BRC-31 session persistence.
//!
//! Stores sessions per-server so we can reuse them without re-handshaking.
//! Sessions are saved as JSON files keyed by SHA-256(server_url)[:16].
//! Default TTL: 1 hour. Expired sessions are auto-deleted on load.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Default session TTL in seconds (1 hour).
pub const DEFAULT_TTL: u64 = 3600;

/// Default session storage directory.
fn default_session_dir() -> PathBuf {
    dirs_home().join(".local/share/brc31-sessions")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// A BRC-31 session with a specific server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub server_url: String,
    pub server_identity_key: String,
    pub server_nonce_b64: String,
    pub client_nonce_b64: String,
    pub timestamp: f64,
}

impl AuthSession {
    /// Create a new session with current timestamp.
    pub fn new(
        server_url: String,
        server_identity_key: String,
        server_nonce_b64: String,
        client_nonce_b64: String,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        Self {
            server_url,
            server_identity_key,
            server_nonce_b64,
            client_nonce_b64,
            timestamp,
        }
    }

    /// Check whether this session has exceeded its TTL.
    pub fn is_expired(&self, ttl: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        (now - self.timestamp) > ttl as f64
    }
}

/// Get the file path for a session, keyed by URL hash.
fn session_path(server_url: &str, session_dir: &Path) -> PathBuf {
    let hash = hex::encode(Sha256::digest(server_url.as_bytes()));
    session_dir.join(format!("{}.json", &hash[..16]))
}

/// Save a session to disk. Returns the file path.
pub fn save_session(session: &AuthSession) -> Result<PathBuf, String> {
    save_session_to(session, &default_session_dir())
}

/// Save a session to a specific directory.
pub fn save_session_to(session: &AuthSession, session_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(session_dir)
        .map_err(|e| format!("Failed to create session dir: {e}"))?;
    let path = session_path(&session.server_url, session_dir);
    let json = serde_json::to_string_pretty(session)
        .map_err(|e| format!("Failed to serialize session: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write session file: {e}"))?;
    Ok(path)
}

/// Load a session from disk. Returns None if not found or expired.
pub fn load_session(server_url: &str, ttl: u64) -> Option<AuthSession> {
    load_session_from(server_url, ttl, &default_session_dir())
}

/// Load a session from a specific directory.
pub fn load_session_from(server_url: &str, ttl: u64, session_dir: &Path) -> Option<AuthSession> {
    let path = session_path(server_url, session_dir);
    if !path.exists() {
        return None;
    }
    let data = std::fs::read_to_string(&path).ok()?;
    let session: AuthSession = serde_json::from_str(&data).ok()?;
    if session.is_expired(ttl) {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(session)
}

/// Delete a stored session. Returns true if deleted.
pub fn clear_session(server_url: &str) -> bool {
    clear_session_from(server_url, &default_session_dir())
}

/// Delete a stored session from a specific directory.
pub fn clear_session_from(server_url: &str, session_dir: &Path) -> bool {
    let path = session_path(server_url, session_dir);
    if path.exists() {
        std::fs::remove_file(&path).is_ok()
    } else {
        false
    }
}

/// Extract base URL (scheme://host[:port]) from a full URL.
///
/// Sessions are keyed by base URL, so callers that have a full endpoint URL
/// (e.g. `https://nanostore.babbage.systems/upload`) must extract the base
/// before calling `clear_session_from` or `load_session_from`.
pub fn base_url_from(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let base = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
            if let Some(port) = parsed.port() {
                format!("{base}:{port}")
            } else {
                base
            }
        }
        Err(_) => url.to_string(),
    }
}

/// Delete all stored sessions. Returns count deleted.
pub fn clear_all_sessions_from(session_dir: &Path) -> usize {
    if !session_dir.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(session_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && std::fs::remove_file(&path).is_ok()
            {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(url: &str) -> AuthSession {
        AuthSession::new(
            url.to_string(),
            "028155878063d691f01cfc0eeb626404ebe9303ec50f9542c234c5c85100a98ca1".into(),
            "c2VydmVyX25vbmNl".into(),
            "Y2xpZW50X25vbmNl".into(),
        )
    }

    #[test]
    fn test_session_creation() {
        let session = make_session("https://example.com");
        assert_eq!(session.server_url, "https://example.com");
        assert!(!session.is_expired(3600));
    }

    #[test]
    fn test_session_expired() {
        let mut session = make_session("https://example.com");
        session.timestamp -= 7200.0; // 2 hours ago
        assert!(session.is_expired(3600));
    }

    #[test]
    fn test_session_not_expired() {
        let session = make_session("https://example.com");
        assert!(!session.is_expired(3600));
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_path_buf();

        let session = make_session("https://messagebox.babbage.systems");
        save_session_to(&session, &session_dir).unwrap();

        let loaded = load_session_from("https://messagebox.babbage.systems", 3600, &session_dir);
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.server_url, session.server_url);
        assert_eq!(loaded.server_identity_key, session.server_identity_key);
        assert_eq!(loaded.server_nonce_b64, session.server_nonce_b64);
        assert_eq!(loaded.client_nonce_b64, session.client_nonce_b64);
    }

    #[test]
    fn test_load_expired_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_path_buf();

        let mut session = make_session("https://expired.example.com");
        session.timestamp -= 7200.0; // 2 hours ago
        save_session_to(&session, &session_dir).unwrap();

        // Should return None and delete the file
        let loaded = load_session_from("https://expired.example.com", 3600, &session_dir);
        assert!(loaded.is_none());

        // File should be gone
        let hash = hex::encode(Sha256::digest("https://expired.example.com".as_bytes()));
        let path = session_dir.join(format!("{}.json", &hash[..16]));
        assert!(!path.exists());
    }

    #[test]
    fn test_load_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_session_from("https://nonexistent.com", 3600, dir.path());
        assert!(loaded.is_none());
    }

    #[test]
    fn test_clear_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_path_buf();

        let session = make_session("https://clear-test.com");
        save_session_to(&session, &session_dir).unwrap();

        assert!(clear_session_from("https://clear-test.com", &session_dir));
        assert!(!clear_session_from("https://clear-test.com", &session_dir)); // already gone
    }

    #[test]
    fn test_clear_all_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_path_buf();

        save_session_to(&make_session("https://a.com"), &session_dir).unwrap();
        save_session_to(&make_session("https://b.com"), &session_dir).unwrap();
        save_session_to(&make_session("https://c.com"), &session_dir).unwrap();

        let cleared = clear_all_sessions_from(&session_dir);
        assert_eq!(cleared, 3);
    }

    #[test]
    fn test_session_serde_roundtrip() {
        let session = make_session("https://serde-test.com");
        let json = serde_json::to_string(&session).unwrap();
        let deserialized: AuthSession = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.server_url, session.server_url);
        assert_eq!(
            deserialized.server_identity_key,
            session.server_identity_key
        );
    }

    #[test]
    fn test_session_path_deterministic() {
        let dir = PathBuf::from("/tmp/test-sessions");
        let p1 = session_path("https://example.com", &dir);
        let p2 = session_path("https://example.com", &dir);
        assert_eq!(p1, p2);

        let p3 = session_path("https://other.com", &dir);
        assert_ne!(p1, p3);
    }

    #[test]
    fn test_base_url_from() {
        assert_eq!(
            base_url_from("https://nanostore.babbage.systems/upload"),
            "https://nanostore.babbage.systems"
        );
        assert_eq!(
            base_url_from("https://nanostore.babbage.systems"),
            "https://nanostore.babbage.systems"
        );
        assert_eq!(
            base_url_from("http://localhost:3322/api/v1/keys"),
            "http://localhost:3322"
        );
        assert_eq!(
            base_url_from("https://example.com/a/b/c?query=1"),
            "https://example.com"
        );
        // Invalid URL falls back to original string
        assert_eq!(base_url_from("not-a-url"), "not-a-url");
    }

    #[test]
    fn test_clear_session_from_full_url() {
        // Regression test: sessions are stored by base URL, so clearing by
        // full endpoint URL must still find and delete the session file.
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_path_buf();

        // Save session keyed by base URL (as client.rs does)
        let session = make_session("https://nanostore.babbage.systems");
        save_session_to(&session, &session_dir).unwrap();

        // Clearing by full endpoint URL should NOT work (different hash)
        assert!(!clear_session_from(
            "https://nanostore.babbage.systems/upload",
            &session_dir
        ));

        // But clearing by base URL (extracted via base_url_from) DOES work
        let base = base_url_from("https://nanostore.babbage.systems/upload");
        assert!(clear_session_from(&base, &session_dir));
    }
}
