//! Circuit breaker for x402 endpoints.
//!
//! Implements the Closed -> Open -> HalfOpen state machine pattern to avoid
//! hammering degraded providers. When a provider fails repeatedly (5 transient
//! failures by default), the circuit opens and callers are routed to the
//! alternate provider. After a recovery window (30s default), a single probe
//! request is allowed (HalfOpen); success closes the circuit, failure re-opens.
//!
//! Inspired by IronClaw's `CircuitBreakerProvider` pattern, adapted for our
//! dual-provider x402 LLM architecture.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Circuit breaker state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation -- requests allowed, tracking consecutive failures.
    Closed,
    /// Rejecting requests -- waiting for recovery window to expire.
    Open { opened_at: Instant },
    /// Probe mode -- one test request allowed to check if provider recovered.
    HalfOpen,
}

/// Per-endpoint circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    pub state: CircuitState,
    pub failure_count: u32,
    pub threshold: u32,
    pub recovery_window: Duration,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given threshold and recovery window.
    pub fn new(threshold: u32, recovery_window: Duration) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            threshold,
            recovery_window,
        }
    }

    /// Check whether a request should be allowed through.
    ///
    /// - Closed: always allowed
    /// - Open: allowed (transitions to HalfOpen) if recovery window has elapsed
    /// - HalfOpen: allowed (one probe request)
    pub fn should_allow(&mut self) -> bool {
        match &self.state {
            CircuitState::Closed => true,
            CircuitState::Open { opened_at } => {
                if opened_at.elapsed() >= self.recovery_window {
                    tracing::info!(
                        "Circuit breaker: recovery window elapsed, transitioning to HalfOpen"
                    );
                    self.state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record a successful request. Resets failure count and closes circuit if HalfOpen.
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::HalfOpen => {
                tracing::info!("Circuit breaker: probe succeeded, closing circuit");
                self.state = CircuitState::Closed;
                self.failure_count = 0;
            }
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::Open { .. } => {
                // Shouldn't happen -- success while open means we missed a transition.
                // Reset anyway.
                self.state = CircuitState::Closed;
                self.failure_count = 0;
            }
        }
    }

    /// Record a failed request. Only transient failures count toward the threshold.
    pub fn record_failure(&mut self, is_transient: bool) {
        if !is_transient {
            return;
        }

        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.threshold {
                    tracing::warn!(
                        "Circuit breaker: {} consecutive transient failures (threshold={}), opening circuit",
                        self.failure_count,
                        self.threshold
                    );
                    self.state = CircuitState::Open {
                        opened_at: Instant::now(),
                    };
                }
            }
            CircuitState::HalfOpen => {
                tracing::warn!("Circuit breaker: probe failed, re-opening circuit");
                self.failure_count = self.threshold; // Keep at threshold
                self.state = CircuitState::Open {
                    opened_at: Instant::now(),
                };
            }
            CircuitState::Open { .. } => {
                // Already open -- nothing to do
            }
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(DEFAULT_FAILURE_THRESHOLD, DEFAULT_RECOVERY_WINDOW)
    }
}

/// Default number of consecutive transient failures before the circuit opens.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 5;

/// Default time the circuit stays open before allowing a probe request.
pub const DEFAULT_RECOVERY_WINDOW: Duration = Duration::from_secs(30);

/// Check if an error message indicates a transient failure.
///
/// Transient: timeouts, connection failures, 5xx server errors, 429 rate limits.
/// Non-transient: 4xx client errors (except 429), auth failures, budget errors.
pub fn is_transient_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();

    // Timeouts and connection failures
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("dns")
        || lower.contains("network")
        || lower.contains("broken pipe")
    {
        return true;
    }

    // HTTP 5xx server errors
    if lower.contains("http 500")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
        || lower.contains("internal server error")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
        || lower.contains("gateway timeout")
    {
        return true;
    }

    // Rate limiting (429)
    if lower.contains("http 429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        return true;
    }

    // ERR_SERVICE_FAILED (x402 proxy returning upstream failure)
    if lower.contains("err_service_failed") {
        return true;
    }

    false
}

/// Registry of per-endpoint circuit breakers.
///
/// Thread-safe via `Arc<Mutex>`. Keyed by base URL (e.g., `https://openai-chat.x402agency.com`).
#[derive(Debug, Clone)]
pub struct CircuitBreakerRegistry {
    breakers: Arc<Mutex<HashMap<String, CircuitBreaker>>>,
    threshold: u32,
    recovery_window: Duration,
}

impl CircuitBreakerRegistry {
    /// Create a new registry with default threshold and recovery window.
    pub fn new() -> Self {
        Self {
            breakers: Arc::new(Mutex::new(HashMap::new())),
            threshold: DEFAULT_FAILURE_THRESHOLD,
            recovery_window: DEFAULT_RECOVERY_WINDOW,
        }
    }

    /// Create a new registry with custom threshold and recovery window.
    pub fn with_config(threshold: u32, recovery_window: Duration) -> Self {
        Self {
            breakers: Arc::new(Mutex::new(HashMap::new())),
            threshold,
            recovery_window,
        }
    }

    /// Extract the base URL (scheme + host) from a full endpoint URL.
    pub fn base_url(url: &str) -> String {
        if let Ok(parsed) = url::Url::parse(url) {
            format!(
                "{}://{}",
                parsed.scheme(),
                parsed.host_str().unwrap_or("unknown")
            )
        } else {
            url.to_string()
        }
    }

    /// Check whether a request to the given endpoint should be allowed.
    pub fn should_allow(&self, endpoint: &str) -> bool {
        let key = Self::base_url(endpoint);
        let mut breakers = self.breakers.lock().unwrap();
        let breaker = breakers
            .entry(key)
            .or_insert_with(|| CircuitBreaker::new(self.threshold, self.recovery_window));
        breaker.should_allow()
    }

    /// Record a successful request to the given endpoint.
    pub fn record_success(&self, endpoint: &str) {
        let key = Self::base_url(endpoint);
        let mut breakers = self.breakers.lock().unwrap();
        if let Some(breaker) = breakers.get_mut(&key) {
            breaker.record_success();
        }
    }

    /// Record a failed request to the given endpoint with an error message.
    ///
    /// The message is checked for transient failure indicators (timeouts,
    /// connection errors, 5xx, 429) to determine whether it counts toward
    /// the circuit breaker threshold.
    pub fn record_failure_msg(&self, endpoint: &str, error_message: &str) {
        let key = Self::base_url(endpoint);
        let transient = is_transient_message(error_message);
        let mut breakers = self.breakers.lock().unwrap();
        let breaker = breakers
            .entry(key)
            .or_insert_with(|| CircuitBreaker::new(self.threshold, self.recovery_window));
        breaker.record_failure(transient);
    }

    /// Get the current state of a circuit for the given endpoint.
    pub fn get_state(&self, endpoint: &str) -> CircuitState {
        let key = Self::base_url(endpoint);
        let breakers = self.breakers.lock().unwrap();
        breakers
            .get(&key)
            .map(|b| b.state.clone())
            .unwrap_or(CircuitState::Closed)
    }
}

impl Default for CircuitBreakerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
