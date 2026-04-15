//! Verifier cache for delegation certs.
//!
//! See `DELEGATION-DESIGN.md` §4.3. Two TTL modes:
//!
//! - **Positive** — successful verifications are cached for `min(cert.expires_at - now, max_ttl)`.
//! - **Negative** — failed verifications are cached for a fixed short TTL to prevent retry storms on malformed certs.
//!
//! The cache is in-memory per agent process, cleared on restart.
//! Keyed by `(cert_hash, task_hash)` since the same cert used for a different task
//! should not short-circuit verification.
//!
//! **Important:** the cache is a soft-path optimization only. Wallet-spending tool
//! calls (`wallet_call`, `x402_call`, `pay_agent`) MUST re-check revocation
//! synchronously, bypassing the cache. See `runner/execute.rs` Phase 3 integration.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::RwLock;

/// What the cache stores for each (cert_hash, task_hash) key.
#[derive(Debug, Clone)]
pub enum CachedVerdict {
    /// Verification succeeded. Valid until `valid_until`.
    Valid { valid_until: DateTime<Utc> },
    /// Verification failed. Valid until `valid_until` (don't re-check before then).
    Invalid {
        reason: String,
        valid_until: DateTime<Utc>,
    },
}

impl CachedVerdict {
    fn valid_until(&self) -> DateTime<Utc> {
        match self {
            CachedVerdict::Valid { valid_until } => *valid_until,
            CachedVerdict::Invalid { valid_until, .. } => *valid_until,
        }
    }
}

/// Thread-safe in-memory cache of verification verdicts.
///
/// Capacity: unbounded. For long-running agents, entries expire naturally via TTL;
/// for paranoid deployments we could add an LRU eviction later.
pub struct VerifierCache {
    entries: RwLock<HashMap<CacheKey, CachedVerdict>>,
    max_positive_ttl: Duration,
    negative_ttl: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    cert_hash: String,
    task_hash: String,
}

impl VerifierCache {
    /// Create a new cache with the given TTLs.
    ///
    /// - `max_positive_ttl`: upper bound on how long a successful verification is cached.
    ///   Actual positive TTL is `min(cert.expires_at - now, max_positive_ttl)`.
    /// - `negative_ttl`: fixed TTL for failed verifications.
    pub fn new(max_positive_ttl: Duration, negative_ttl: Duration) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            max_positive_ttl,
            negative_ttl,
        }
    }

    /// Default cache: 5 min positive, 30 sec negative (per `DELEGATION-DESIGN.md` §4.3 defaults).
    pub fn with_defaults() -> Self {
        Self::new(Duration::seconds(300), Duration::seconds(30))
    }

    /// Look up a cached verdict. Returns `None` if missing or expired.
    /// Does not mutate the cache on expiry — stale entries are evicted lazily
    /// on the next matching insert.
    pub fn get(
        &self,
        cert_hash: &str,
        task_hash: &str,
        now: DateTime<Utc>,
    ) -> Option<CachedVerdict> {
        let key = CacheKey {
            cert_hash: cert_hash.to_string(),
            task_hash: task_hash.to_string(),
        };
        let entries = self.entries.read().ok()?;
        let entry = entries.get(&key)?;
        if now >= entry.valid_until() {
            return None;
        }
        Some(entry.clone())
    }

    /// Cache a successful verification. The positive TTL is clamped to
    /// `min(cert_expires_at - now, max_positive_ttl)` so a soon-to-expire cert
    /// doesn't linger in cache past its natural lifetime.
    pub fn insert_valid(
        &self,
        cert_hash: &str,
        task_hash: &str,
        cert_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) {
        let remaining = cert_expires_at - now;
        let ttl = remaining.min(self.max_positive_ttl);
        // Don't insert if cert is already expired or TTL is negative
        if ttl <= Duration::zero() {
            return;
        }
        let valid_until = now + ttl;
        let key = CacheKey {
            cert_hash: cert_hash.to_string(),
            task_hash: task_hash.to_string(),
        };
        let verdict = CachedVerdict::Valid { valid_until };
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(key, verdict);
        }
    }

    /// Cache a failed verification with a fixed short TTL.
    pub fn insert_invalid(
        &self,
        cert_hash: &str,
        task_hash: &str,
        reason: String,
        now: DateTime<Utc>,
    ) {
        let valid_until = now + self.negative_ttl;
        let key = CacheKey {
            cert_hash: cert_hash.to_string(),
            task_hash: task_hash.to_string(),
        };
        let verdict = CachedVerdict::Invalid {
            reason,
            valid_until,
        };
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(key, verdict);
        }
    }

    /// Evict entries whose TTL has passed. Optional — normally called by
    /// a background task or at the top of each agent iteration.
    pub fn evict_expired(&self, now: DateTime<Utc>) {
        if let Ok(mut entries) = self.entries.write() {
            entries.retain(|_, v| now < v.valid_until());
        }
    }

    /// Cache size (number of entries, including expired-but-not-yet-evicted).
    pub fn len(&self) -> usize {
        self.entries.read().map(|e| e.len()).unwrap_or(0)
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_at(hour: u32, min: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 1, 1, hour, min, 0).unwrap()
    }

    #[test]
    fn empty_cache_returns_none() {
        let cache = VerifierCache::with_defaults();
        assert!(cache.get("c", "t", now_at(12, 0)).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn positive_hit_before_ttl() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_valid("c", "t", now + Duration::hours(1), now);
        let hit = cache.get("c", "t", now + Duration::seconds(60));
        assert!(matches!(hit, Some(CachedVerdict::Valid { .. })));
    }

    #[test]
    fn positive_miss_after_ttl() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_valid("c", "t", now + Duration::hours(1), now);
        // Default max_positive_ttl is 300s
        let later = now + Duration::seconds(301);
        assert!(cache.get("c", "t", later).is_none());
    }

    #[test]
    fn positive_ttl_clamps_to_cert_expiry() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        // Cert expires in 10s — shorter than default max_positive_ttl (300s)
        cache.insert_valid("c", "t", now + Duration::seconds(10), now);
        // Hit at t+5s
        assert!(matches!(
            cache.get("c", "t", now + Duration::seconds(5)),
            Some(CachedVerdict::Valid { .. })
        ));
        // Miss at t+15s (past cert's expiry)
        assert!(cache.get("c", "t", now + Duration::seconds(15)).is_none());
    }

    #[test]
    fn negative_hit_before_ttl() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_invalid("c", "t", "bad sig".into(), now);
        let hit = cache.get("c", "t", now + Duration::seconds(10));
        match hit {
            Some(CachedVerdict::Invalid { reason, .. }) => assert_eq!(reason, "bad sig"),
            _ => panic!("expected Invalid verdict"),
        }
    }

    #[test]
    fn negative_miss_after_ttl() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_invalid("c", "t", "bad sig".into(), now);
        // Default negative_ttl is 30s
        assert!(cache.get("c", "t", now + Duration::seconds(31)).is_none());
    }

    #[test]
    fn different_tasks_have_separate_entries() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_valid("cert1", "taskA", now + Duration::hours(1), now);
        assert!(cache.get("cert1", "taskA", now).is_some());
        assert!(cache.get("cert1", "taskB", now).is_none());
    }

    #[test]
    fn different_certs_have_separate_entries() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_valid("cert1", "task", now + Duration::hours(1), now);
        assert!(cache.get("cert1", "task", now).is_some());
        assert!(cache.get("cert2", "task", now).is_none());
    }

    #[test]
    fn evict_expired_removes_stale_entries() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_valid("live", "t", now + Duration::hours(1), now);
        cache.insert_invalid("dead", "t", "expired".into(), now - Duration::hours(1));
        assert_eq!(cache.len(), 2);
        cache.evict_expired(now);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("live", "t", now).is_some());
        assert!(cache.get("dead", "t", now).is_none());
    }

    #[test]
    fn insert_valid_on_expired_cert_is_noop() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        // Cert already expired 1 second ago
        cache.insert_valid("c", "t", now - Duration::seconds(1), now);
        assert!(cache.get("c", "t", now).is_none());
    }

    #[test]
    fn new_verdict_overwrites_old() {
        let cache = VerifierCache::with_defaults();
        let now = now_at(12, 0);
        cache.insert_invalid("c", "t", "first".into(), now);
        cache.insert_valid("c", "t", now + Duration::hours(1), now);
        let hit = cache.get("c", "t", now);
        assert!(matches!(hit, Some(CachedVerdict::Valid { .. })));
    }

    #[test]
    fn custom_ttls_respected() {
        let cache = VerifierCache::new(Duration::seconds(10), Duration::seconds(2));
        let now = now_at(12, 0);
        cache.insert_valid("c", "t", now + Duration::hours(1), now);
        assert!(cache.get("c", "t", now + Duration::seconds(9)).is_some());
        assert!(cache.get("c", "t", now + Duration::seconds(11)).is_none());

        cache.insert_invalid("c2", "t", "x".into(), now);
        assert!(cache.get("c2", "t", now + Duration::seconds(1)).is_some());
        assert!(cache.get("c2", "t", now + Duration::seconds(3)).is_none());
    }
}
