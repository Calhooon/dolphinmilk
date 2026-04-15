//! Loop detection — 4 detectors + budget drain.
//!
//! Detectors:
//!   1. generic_repeat: same tool+params called N times consecutively
//!   2. no_progress: polling with identical results
//!   3. ping_pong: alternating between two tool call patterns
//!   4. global_circuit_breaker: hard stop after N total tool calls
//!
//! Thresholds: warning at 10, critical at 20, breaker at 30.
//! Extended with budget drain detection (novel — not in OpenClaw).

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

const WARN_THRESHOLD: u32 = 10;
const CRITICAL_THRESHOLD: u32 = 20;
const BREAKER_THRESHOLD: u32 = 30;

/// Result of a loop detection check.
#[derive(Debug, Clone, Default)]
pub struct LoopCheckResult {
    pub stuck: bool,
    pub level: String,
    pub message: String,
    pub detector: String,
}

/// Record of a single tool call for loop detection.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub name: String,
    pub params_hash: String,
    pub result_hash: String,
    pub ts: f64,
    pub sats_cost: u64,
}

/// Detects repetitive tool call patterns and budget drain.
pub struct LoopDetector {
    pub warn_threshold: u32,
    pub critical_threshold: u32,
    pub breaker_threshold: u32,
    pub max_sats_per_hour: u64,
    history: Vec<ToolCallRecord>,
    total_calls: u32,
}

impl LoopDetector {
    pub fn new(
        warn_threshold: u32,
        critical_threshold: u32,
        breaker_threshold: u32,
        max_sats_per_hour: u64,
    ) -> Self {
        Self {
            warn_threshold,
            critical_threshold,
            breaker_threshold,
            max_sats_per_hour,
            history: Vec::new(),
            total_calls: 0,
        }
    }

    pub fn with_defaults(max_sats_per_hour: u64) -> Self {
        Self::new(
            WARN_THRESHOLD,
            CRITICAL_THRESHOLD,
            BREAKER_THRESHOLD,
            max_sats_per_hour,
        )
    }

    fn hash_params(params: &serde_json::Value) -> String {
        let s = serde_json::to_string(params).unwrap_or_default();
        let digest = Sha256::digest(s.as_bytes());
        hex::encode(&digest[..8])
    }

    fn hash_result(result: &str) -> String {
        let digest = Sha256::digest(result.as_bytes());
        hex::encode(&digest[..8])
    }

    fn now_ts() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// Check if calling this tool looks like a loop. Call BEFORE executing.
    pub fn check(&mut self, tool_name: &str, params: &serde_json::Value) -> LoopCheckResult {
        self.total_calls += 1;
        let params_hash = Self::hash_params(params);

        // Global circuit breaker
        if self.total_calls >= self.breaker_threshold {
            return LoopCheckResult {
                stuck: true,
                level: "breaker".to_string(),
                message: format!(
                    "CIRCUIT BREAKER: {} tool calls in this step. Stopping to prevent runaway execution.",
                    self.total_calls
                ),
                detector: "global_circuit_breaker".to_string(),
            };
        }

        // Check for generic repeat
        let repeat = self.check_generic_repeat(tool_name, &params_hash);
        if repeat.stuck {
            return repeat;
        }

        // Check for ping-pong
        let pong = self.check_ping_pong(tool_name, &params_hash);
        if pong.stuck {
            return pong;
        }

        LoopCheckResult::default()
    }

    /// Record the outcome of a tool call. Call AFTER execution.
    pub fn record_outcome(
        &mut self,
        tool_name: &str,
        params: &serde_json::Value,
        result: &str,
        sats_cost: u64,
    ) {
        self.history.push(ToolCallRecord {
            name: tool_name.to_string(),
            params_hash: Self::hash_params(params),
            result_hash: Self::hash_result(result),
            ts: Self::now_ts(),
            sats_cost,
        });

        // Check for no-progress pattern
        self.check_no_progress();
    }

    fn check_generic_repeat(&self, name: &str, params_hash: &str) -> LoopCheckResult {
        if self.history.is_empty() {
            return LoopCheckResult::default();
        }

        let mut consecutive = 0u32;
        for record in self.history.iter().rev() {
            if record.name == name && record.params_hash == params_hash {
                consecutive += 1;
            } else {
                break;
            }
        }

        if consecutive >= self.critical_threshold {
            return LoopCheckResult {
                stuck: true,
                level: "critical".to_string(),
                message: format!(
                    "LOOP DETECTED: {name} called {consecutive} times with identical parameters. You are stuck. Try a completely different approach."
                ),
                detector: "generic_repeat".to_string(),
            };
        }
        if consecutive >= self.warn_threshold {
            return LoopCheckResult {
                stuck: true,
                level: "warning".to_string(),
                message: format!(
                    "WARNING: {name} called {consecutive} times with identical parameters. Consider a different approach."
                ),
                detector: "generic_repeat".to_string(),
            };
        }

        LoopCheckResult::default()
    }

    fn check_ping_pong(&self, name: &str, params_hash: &str) -> LoopCheckResult {
        if self.history.len() < 4 {
            return LoopCheckResult::default();
        }

        let recent = &self.history[self.history.len() - 4..];
        let key = |r: &ToolCallRecord| format!("{}:{}", r.name, r.params_hash);
        let keys: Vec<String> = recent.iter().map(key).collect();

        if keys[0] == keys[2] && keys[1] == keys[3] && keys[0] != keys[1] {
            let current_key = format!("{name}:{params_hash}");
            if current_key == keys[0] || current_key == keys[1] {
                let pattern_len: usize = self
                    .history
                    .iter()
                    .filter(|r| {
                        let k = format!("{}:{}", r.name, r.params_hash);
                        k == keys[0] || k == keys[1]
                    })
                    .count();

                if pattern_len >= self.warn_threshold as usize {
                    return LoopCheckResult {
                        stuck: true,
                        level: "warning".to_string(),
                        message: format!(
                            "PING-PONG DETECTED: Alternating between two tool calls for {pattern_len} iterations. Break the cycle."
                        ),
                        detector: "ping_pong".to_string(),
                    };
                }
            }
        }

        LoopCheckResult::default()
    }

    fn check_no_progress(&self) {
        if self.history.len() < self.warn_threshold as usize {
            return;
        }

        let start = self.history.len() - self.warn_threshold as usize;
        let recent = &self.history[start..];
        let result_hashes: std::collections::HashSet<&str> = recent
            .iter()
            .filter(|r| !r.result_hash.is_empty())
            .map(|r| r.result_hash.as_str())
            .collect();

        if result_hashes.len() == 1 {
            tracing::warn!(
                "No-progress detected: last {} tool calls all returned identical results",
                self.warn_threshold
            );
        }
    }

    /// Check if spending rate is excessive.
    pub fn check_budget_drain(&self, _sats_spent_session: u64) -> LoopCheckResult {
        if self.history.is_empty() {
            return LoopCheckResult::default();
        }

        let one_hour_ago = Self::now_ts() - 3600.0;
        let recent_spend: u64 = self
            .history
            .iter()
            .filter(|r| r.ts > one_hour_ago)
            .map(|r| r.sats_cost)
            .sum();
        let recent_calls: usize = self.history.iter().filter(|r| r.ts > one_hour_ago).count();

        if recent_spend > self.max_sats_per_hour {
            return LoopCheckResult {
                stuck: true,
                level: "critical".to_string(),
                message: format!(
                    "BUDGET DRAIN: Spent {} sats in the last hour ({} calls). Max allowed: {} sats/hour. Consider cheaper tools or stopping.",
                    recent_spend, recent_calls, self.max_sats_per_hour
                ),
                detector: "budget_drain".to_string(),
            };
        }

        LoopCheckResult::default()
    }

    /// Reset all detection state.
    pub fn reset(&mut self) {
        self.history.clear();
        self.total_calls = 0;
    }

    pub fn total_calls(&self) -> u32 {
        self.total_calls
    }
}
