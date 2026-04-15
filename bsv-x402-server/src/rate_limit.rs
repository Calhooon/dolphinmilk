//! Token bucket rate limiter for x402 services.
//!
//! Each service (identified by base URL) gets an independent token bucket.
//! Configurable with optional cert-driven overrides following the same pattern
//! as budget limits: BRC-52 cert fields override config, config overrides defaults.
//!
//! Default behavior: no limit (backward compatible). When a limit is configured,
//! callers block via `acquire()` until a token becomes available.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::X402Error;

/// Rate limit configuration for the x402 rate limiter.
///
/// Mirrors the worm's `RateLimitConfig` so this crate is self-contained.
#[derive(Debug, Clone, Default)]
pub struct RateLimitConfig {
    /// Enable rate limiting globally. Default: false (no rate limiting).
    pub enabled: bool,
    /// Default requests per minute for all services. 0 = unlimited.
    pub default_rpm: u32,
    /// Per-service rate limit overrides, keyed by base URL.
    pub per_service: HashMap<String, ServiceRateLimit>,
}

/// Rate limit configuration for a single x402 service.
#[derive(Debug, Clone)]
pub struct ServiceRateLimit {
    /// Requests per minute allowed for this service.
    pub requests_per_minute: u32,
}

/// Per-service token bucket state.
#[derive(Debug, Clone)]
struct TokenBucket {
    /// Maximum tokens (= requests) in the bucket.
    capacity: u32,
    /// Current available tokens (float for fractional refill tracking).
    tokens: f64,
    /// Tokens added per second.
    refill_rate: f64,
    /// Last time we computed a refill.
    last_refill: Instant,
}

impl TokenBucket {
    fn new(requests_per_minute: u32) -> Self {
        let capacity = requests_per_minute;
        Self {
            capacity,
            tokens: capacity as f64,
            refill_rate: capacity as f64 / 60.0,
            last_refill: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time since last refill.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity as f64);
        self.last_refill = now;
    }

    /// Try to consume one token. Returns true if successful.
    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Duration until next token is available.
    fn time_until_available(&mut self) -> Duration {
        self.refill();
        if self.tokens >= 1.0 {
            return Duration::ZERO;
        }
        let deficit = 1.0 - self.tokens;
        Duration::from_secs_f64(deficit / self.refill_rate)
    }
}

/// Rate limit status for a single service.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceRateLimitStatus {
    /// Base URL of the service.
    pub service: String,
    /// Configured requests per minute (0 = unlimited).
    pub requests_per_minute: u32,
    /// Current available tokens.
    pub tokens_available: f64,
    /// Bucket capacity.
    pub capacity: u32,
}

/// Certificate-derived rate limit overrides.
pub struct CertRateLimits {
    /// Global default RPM override from cert. None = no cert override.
    pub default_rpm: Option<u32>,
    /// Whether rate limiting is enabled per cert. None = no cert override.
    pub enabled: Option<bool>,
}

impl CertRateLimits {
    /// No overrides (no cert or no rate limit fields).
    pub fn none() -> Self {
        Self {
            default_rpm: None,
            enabled: None,
        }
    }
}

/// Thread-safe rate limiter registry for x402 services.
///
/// Each service gets an independent token bucket identified by base URL.
/// Configuration merges cert limits (highest priority) with config limits.
#[derive(Debug, Clone)]
pub struct RateLimiterRegistry {
    inner: Arc<Mutex<RateLimiterInner>>,
}

#[derive(Debug)]
struct RateLimiterInner {
    /// Per-service token buckets, keyed by base URL.
    buckets: HashMap<String, TokenBucket>,
    /// Whether rate limiting is enabled.
    enabled: bool,
    /// Default RPM for services without specific config.
    default_rpm: u32,
    /// Per-service RPM overrides from config.
    per_service: HashMap<String, u32>,
}

impl RateLimiterRegistry {
    /// Create a new rate limiter from config, optionally overridden by cert limits.
    pub fn new(config: &RateLimitConfig, cert_limits: &CertRateLimits) -> Self {
        // Cert overrides config (same pattern as budget limits)
        let enabled = cert_limits.enabled.unwrap_or(config.enabled);
        let default_rpm = cert_limits.default_rpm.unwrap_or(config.default_rpm);

        let per_service: HashMap<String, u32> = config
            .per_service
            .iter()
            .map(|(k, v)| (k.clone(), v.requests_per_minute))
            .collect();

        if enabled {
            tracing::info!(
                "x402 rate limiter enabled: default_rpm={} (0=unlimited), {} per-service overrides",
                default_rpm,
                per_service.len()
            );
        } else {
            tracing::debug!("x402 rate limiter disabled (default: no rate limiting)");
        }

        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                enabled,
                default_rpm,
                per_service,
            })),
        }
    }

    /// Create a disabled (no-limit) rate limiter. Used as default.
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                enabled: false,
                default_rpm: 0,
                per_service: HashMap::new(),
            })),
        }
    }

    /// Acquire a token for the given service URL. Blocks until a token is
    /// available (via `tokio::time::sleep`). Returns immediately if rate
    /// limiting is disabled or the service has no limit (RPM=0).
    pub async fn acquire(&self, service_url: &str) -> Result<(), X402Error> {
        loop {
            let wait_duration = {
                let mut inner = self.inner.lock().unwrap();

                if !inner.enabled {
                    return Ok(());
                }

                let base = base_url(service_url);
                let rpm = inner
                    .per_service
                    .get(&base)
                    .copied()
                    .unwrap_or(inner.default_rpm);

                // 0 RPM means unlimited
                if rpm == 0 {
                    return Ok(());
                }

                let bucket = inner
                    .buckets
                    .entry(base.clone())
                    .or_insert_with(|| TokenBucket::new(rpm));

                if bucket.try_consume() {
                    return Ok(());
                }

                // Calculate wait time
                bucket.time_until_available()
            };
            // Mutex released -- safe to await
            tracing::info!(
                "x402 rate limit: throttled for {service_url}, waiting {:.1}s",
                wait_duration.as_secs_f64()
            );
            tokio::time::sleep(wait_duration).await;
        }
    }

    /// Get current rate limit status for all tracked services.
    pub fn status(&self) -> Vec<ServiceRateLimitStatus> {
        let mut inner = self.inner.lock().unwrap();
        let mut result = Vec::new();

        for (service, bucket) in inner.buckets.iter_mut() {
            bucket.refill();
            result.push(ServiceRateLimitStatus {
                service: service.clone(),
                requests_per_minute: bucket.capacity,
                tokens_available: bucket.tokens,
                capacity: bucket.capacity,
            });
        }

        result
    }

    /// Check if rate limiting is enabled.
    pub fn is_enabled(&self) -> bool {
        self.inner.lock().unwrap().enabled
    }
}

/// Extract base URL (scheme://host) from a full URL, matching the circuit
/// breaker's key strategy.
pub fn base_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let scheme = parsed.scheme();
        let host = parsed.host_str().unwrap_or("unknown");
        if let Some(port) = parsed.port() {
            format!("{scheme}://{host}:{port}")
        } else {
            format!("{scheme}://{host}")
        }
    } else {
        url.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(enabled: bool, default_rpm: u32) -> RateLimitConfig {
        RateLimitConfig {
            enabled,
            default_rpm,
            per_service: HashMap::new(),
        }
    }

    #[test]
    fn test_token_bucket_starts_full() {
        let bucket = TokenBucket::new(60);
        assert_eq!(bucket.capacity, 60);
        assert!((bucket.tokens - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_token_bucket_drains_on_consume() {
        let mut bucket = TokenBucket::new(10);
        assert!(bucket.try_consume());
        assert!(bucket.tokens < 10.0);
    }

    #[test]
    fn test_token_bucket_rejects_when_empty() {
        let mut bucket = TokenBucket::new(2);
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        // Immediately after draining, should reject
        assert!(!bucket.try_consume());
    }

    #[test]
    fn test_base_url_extraction() {
        assert_eq!(
            base_url("https://openai-chat.x402agency.com/chat"),
            "https://openai-chat.x402agency.com"
        );
        assert_eq!(
            base_url("https://claude-chat.x402agency.com/chat"),
            "https://claude-chat.x402agency.com"
        );
        assert_eq!(
            base_url("http://localhost:8080/v1/completions"),
            "http://localhost:8080"
        );
    }

    #[test]
    fn test_disabled_registry_always_allows() {
        let registry = RateLimiterRegistry::disabled();
        assert!(!registry.is_enabled());
    }

    #[test]
    fn test_zero_rpm_means_unlimited() {
        let config = test_config(true, 0);
        let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());
        assert!(registry.is_enabled());
        // Zero RPM = no bucket created = unlimited
        let status = registry.status();
        assert!(status.is_empty()); // no buckets tracked yet
    }
}
