//! In-memory LRU+TTL response cache for paid API calls.
//!
//! Caches responses keyed by SHA-256 of request parameters.
//! Saves satoshis by avoiding duplicate x402 payments for identical requests.
//!
//! Generic over the cached response type `T`. The caller provides a
//! `should_cache` predicate to decide whether a response should be stored
//! (e.g., skip caching LLM responses containing tool_calls).
//!
//! Inspired by IronClaw's `CachedProvider` pattern, adapted for x402 payment model.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use lru::LruCache;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;

/// A cached response with metadata for TTL and cost tracking.
#[derive(Debug, Clone)]
pub struct CacheEntry<T: Clone> {
    pub response: T,
    pub inserted_at: Instant,
    pub sats_cost: u64,
}

/// Cache statistics for monitoring and budget dashboards.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub total_sats_saved: u64,
    pub evictions: u64,
}

impl CacheStats {
    /// Hit rate as a percentage (0.0-100.0).
    pub fn hit_rate_pct(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        (self.hits as f64 / total as f64) * 100.0
    }
}

/// In-memory LRU+TTL response cache.
///
/// Thread-safe via `Mutex`. The lock is never held across `.await`.
pub struct ResponseCache<T: Clone> {
    inner: Mutex<CacheInner<T>>,
    ttl: Duration,
}

struct CacheInner<T: Clone> {
    cache: LruCache<String, CacheEntry<T>>,
    stats: CacheStats,
}

impl<T: Clone> ResponseCache<T> {
    /// Create a new cache with the given TTL and max entries.
    ///
    /// If `ttl_secs` is 0, the cache is effectively disabled (all lookups miss).
    /// If `max_entries` is 0, it's set to 1 (LruCache requires NonZeroUsize).
    pub fn new(ttl_secs: u64, max_entries: usize) -> Self {
        let cap = NonZeroUsize::new(max_entries.max(1)).unwrap();
        Self {
            inner: Mutex::new(CacheInner {
                cache: LruCache::new(cap),
                stats: CacheStats::default(),
            }),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Build a deterministic cache key from the request parameters.
    ///
    /// SHA-256 of `model | messages_json | temperature | max_tokens`.
    pub fn cache_key(
        model: &str,
        messages: &[Value],
        temperature: Option<f64>,
        max_tokens: u32,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model.as_bytes());
        hasher.update(b"|");
        // Messages as deterministic JSON
        if let Ok(json) = serde_json::to_string(messages) {
            hasher.update(json.as_bytes());
        }
        hasher.update(b"|");
        if let Some(temp) = temperature {
            hasher.update(temp.to_le_bytes());
        }
        hasher.update(b"|");
        hasher.update(max_tokens.to_le_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Look up a cached response. Returns `None` on miss or TTL expiry.
    pub fn get(&self, key: &str) -> Option<T> {
        if self.ttl.is_zero() {
            return None;
        }
        let mut inner = self.inner.lock().unwrap();
        // Peek to check existence and TTL without taking a long-lived borrow
        let hit_info = inner.cache.peek(key).map(|entry| {
            let expired = entry.inserted_at.elapsed() >= self.ttl;
            if expired {
                None
            } else {
                Some((entry.response.clone(), entry.sats_cost))
            }
        });

        match hit_info {
            Some(Some((response, sats_cost))) => {
                // Promote in LRU order
                inner.cache.promote(key);
                inner.stats.hits += 1;
                inner.stats.total_sats_saved += sats_cost;
                Some(response)
            }
            Some(None) => {
                // Entry exists but expired -- remove it
                inner.cache.pop(key);
                inner.stats.misses += 1;
                None
            }
            None => {
                // Not in cache
                inner.stats.misses += 1;
                None
            }
        }
    }

    /// Store a response in the cache unconditionally.
    ///
    /// Returns `true` if the entry was stored, `false` if the cache is disabled.
    /// Use `put_if` for conditional caching (e.g., skip tool_call responses).
    pub fn put(&self, key: String, response: T, sats_cost: u64) -> bool {
        if self.ttl.is_zero() {
            return false;
        }

        let mut inner = self.inner.lock().unwrap();
        let was_full = inner.cache.len() == inner.cache.cap().get();
        inner.cache.put(
            key,
            CacheEntry {
                response,
                inserted_at: Instant::now(),
                sats_cost,
            },
        );
        if was_full {
            inner.stats.evictions += 1;
        }
        true
    }

    /// Store a response in the cache only if the predicate returns true.
    ///
    /// Returns `true` if stored, `false` if skipped or cache disabled.
    pub fn put_if<F>(&self, key: String, response: T, sats_cost: u64, should_cache: F) -> bool
    where
        F: FnOnce(&T) -> bool,
    {
        if self.ttl.is_zero() {
            return false;
        }
        if !should_cache(&response) {
            return false;
        }
        self.put(key, response, sats_cost)
    }

    /// Get a snapshot of cache statistics.
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().unwrap();
        inner.stats.clone()
    }

    /// Current number of entries in the cache.
    pub fn len(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.cache.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The configured TTL.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

impl<T: Clone + std::fmt::Debug> std::fmt::Debug for ResponseCache<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap();
        f.debug_struct("ResponseCache")
            .field("entries", &inner.cache.len())
            .field("ttl", &self.ttl)
            .field("stats", &inner.stats)
            .finish()
    }
}
