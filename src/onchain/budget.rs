//! Budget tracker — per-service spending with JSONL log and hard limits.
//!
//! Tracks spending across services (LLM, proofs, tokens, messagebox),
//! enforces per-task/hourly/daily limits, and persists a JSONL audit trail.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::BudgetConfig;
use crate::error::DmError;

/// A single spending record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingEntry {
    pub timestamp: DateTime<Utc>,
    pub service: String,
    pub operation: String,
    pub sats: u64,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

/// Per-service spending accumulator.
#[derive(Debug, Clone, Default)]
pub struct ServiceBudget {
    pub total_sats: u64,
    pub count: u64,
}

/// Budget limits (mirrors BudgetConfig but owned).
#[derive(Debug, Clone)]
pub struct BudgetLimits {
    pub max_per_task: u64,
    pub max_per_hour: u64,
    pub max_per_day: u64,
    /// Rolling 7-day limit. 0 = unlimited.
    pub max_per_week: u64,
    /// Rolling 30-day limit. 0 = unlimited.
    pub max_per_month: u64,
    /// Lifetime cumulative limit. 0 = unlimited.
    pub max_lifetime: u64,
    /// Enforcement mode: "strict" (default) or "advisory".
    pub enforcement: String,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self {
            max_per_task: 250_000,
            max_per_hour: 2_500_000,
            max_per_day: 25_000_000,
            max_per_week: 100_000_000,
            max_per_month: 500_000_000,
            max_lifetime: 0,
            enforcement: "strict".into(),
        }
    }
}

impl From<&BudgetConfig> for BudgetLimits {
    fn from(cfg: &BudgetConfig) -> Self {
        Self {
            max_per_task: cfg.max_per_task,
            max_per_hour: cfg.max_per_hour,
            max_per_day: cfg.max_per_day,
            max_per_week: cfg.max_per_week,
            max_per_month: cfg.max_per_month,
            max_lifetime: cfg.max_lifetime,
            enforcement: cfg.enforcement.clone(),
        }
    }
}

/// Summary report of budget usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetReport {
    pub task_sats: u64,
    pub total_operations: u64,
    pub services: HashMap<String, ServiceReport>,
    pub hourly_sats: u64,
    pub daily_sats: u64,
    #[serde(default)]
    pub weekly_sats: u64,
    #[serde(default)]
    pub monthly_sats: u64,
    #[serde(default)]
    pub lifetime_sats: u64,
    pub limits: LimitsReport,
    /// Wallet balance in satoshis (0 if not fetched).
    #[serde(default)]
    pub balance: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceReport {
    pub total_sats: u64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsReport {
    pub max_per_task: u64,
    pub max_per_hour: u64,
    pub max_per_day: u64,
    #[serde(default)]
    pub max_per_week: u64,
    #[serde(default)]
    pub max_per_month: u64,
    #[serde(default)]
    pub max_lifetime: u64,
    pub task_remaining: u64,
    pub hourly_remaining: u64,
    pub daily_remaining: u64,
    #[serde(default)]
    pub weekly_remaining: u64,
    #[serde(default)]
    pub monthly_remaining: u64,
    #[serde(default)]
    pub lifetime_remaining: u64,
    /// Enforcement mode: "strict" or "advisory".
    #[serde(default = "default_enforcement")]
    pub enforcement: String,
}

fn default_enforcement() -> String {
    "strict".into()
}

/// Tracks per-service spending with JSONL persistence and hard limits.
pub struct BudgetTracker {
    services: HashMap<String, ServiceBudget>,
    entries: Vec<SpendingEntry>,
    log_path: Option<PathBuf>,
    limits: BudgetLimits,
    task_sats: u64,
}

impl BudgetTracker {
    /// Create a new budget tracker with optional JSONL log path.
    pub fn new(log_path: Option<PathBuf>, limits: BudgetLimits) -> Self {
        Self {
            services: HashMap::new(),
            entries: Vec::new(),
            log_path,
            limits,
            task_sats: 0,
        }
    }

    /// Create from config and workspace path.
    ///
    /// Replays any existing `budget.jsonl` so that hourly/daily spending
    /// limits and `/budget/detail` data survive server restarts.
    pub fn from_config(config: &BudgetConfig, workspace: &Path) -> Self {
        let log_path = workspace.join("budget.jsonl");
        let mut tracker = Self::new(Some(log_path), BudgetLimits::from(config));
        tracker.replay_log();
        tracker
    }

    /// Replay the JSONL audit log to restore entries and per-service totals.
    ///
    /// Best-effort: malformed lines are logged and skipped rather than
    /// causing an error.  `task_sats` is intentionally left at zero because
    /// it is a per-task counter that should start fresh on each new task.
    pub fn replay_log(&mut self) {
        let path = match self.log_path {
            Some(ref p) if p.exists() => p.clone(),
            _ => return,
        };

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("budget: failed to open log for replay: {e}");
                return;
            }
        };

        let reader = BufReader::new(file);
        let mut replayed = 0u64;
        let mut skipped = 0u64;

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!("budget: read error at line {}: {e}", line_num + 1);
                    skipped += 1;
                    continue;
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_str::<SpendingEntry>(trimmed) {
                Ok(entry) => {
                    let budget = self.services.entry(entry.service.clone()).or_default();
                    budget.total_sats += entry.sats;
                    budget.count += 1;
                    self.entries.push(entry);
                    replayed += 1;
                }
                Err(e) => {
                    tracing::warn!("budget: skipping malformed line {}: {e}", line_num + 1);
                    skipped += 1;
                }
            }
        }

        if replayed > 0 || skipped > 0 {
            tracing::info!("budget: replayed {replayed} entries from log ({skipped} skipped)");
        }
    }

    /// Record a spending event. Appends to JSONL log.
    pub fn record(&mut self, service: &str, operation: &str, sats: u64, details: Value) {
        let entry = SpendingEntry {
            timestamp: Utc::now(),
            service: service.to_string(),
            operation: operation.to_string(),
            sats,
            details,
        };

        // Update per-service totals
        let budget = self.services.entry(service.to_string()).or_default();
        budget.total_sats += sats;
        budget.count += 1;

        self.task_sats += sats;
        self.entries.push(entry.clone());

        // Append to JSONL log (best-effort)
        if let Some(ref path) = self.log_path {
            if let Ok(line) = serde_json::to_string(&entry) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{}", line);
                }
            }
        }

        tracing::debug!(
            "budget: {} {} {} sats (task total: {})",
            service,
            operation,
            sats,
            self.task_sats,
        );
    }

    /// Pre-flight check: will spending `sats` exceed any limit?
    ///
    /// Returns `Ok(())` if within all limits, or `Err(BudgetError)` with the
    /// first limit that would be exceeded. Checks in order:
    /// task → hourly → daily → weekly → monthly → lifetime.
    pub fn check_limit(&self, sats: u64) -> Result<(), DmError> {
        // Task limit
        if self.task_sats + sats > self.limits.max_per_task {
            return Err(DmError::budget(format!(
                "Task budget limit reached — spent {} of {} sats limit",
                self.task_sats, self.limits.max_per_task
            )));
        }

        // Hourly limit
        let hourly = self.hourly_sats();
        if hourly + sats > self.limits.max_per_hour {
            return Err(DmError::budget(format!(
                "Hourly budget limit reached — spent {} of {} sats limit",
                hourly, self.limits.max_per_hour
            )));
        }

        // Daily limit
        let daily = self.daily_sats();
        if daily + sats > self.limits.max_per_day {
            return Err(DmError::budget(format!(
                "Daily budget limit reached — spent {} of {} sats limit",
                daily, self.limits.max_per_day
            )));
        }

        // Weekly limit (0 = unlimited)
        if self.limits.max_per_week > 0 {
            let weekly = self.weekly_sats();
            if weekly + sats > self.limits.max_per_week {
                return Err(DmError::budget(format!(
                    "Weekly budget limit reached — spent {} of {} sats limit",
                    weekly, self.limits.max_per_week
                )));
            }
        }

        // Monthly limit (0 = unlimited)
        if self.limits.max_per_month > 0 {
            let monthly = self.monthly_sats();
            if monthly + sats > self.limits.max_per_month {
                return Err(DmError::budget(format!(
                    "Monthly budget limit reached — spent {} of {} sats limit",
                    monthly, self.limits.max_per_month
                )));
            }
        }

        // Lifetime limit (0 = unlimited)
        if self.limits.max_lifetime > 0 {
            let lifetime = self.lifetime_sats();
            if lifetime + sats > self.limits.max_lifetime {
                return Err(DmError::budget(format!(
                    "Lifetime budget limit reached — spent {} of {} sats limit",
                    lifetime, self.limits.max_lifetime
                )));
            }
        }

        Ok(())
    }

    /// Whether the budget enforcement mode is advisory (warn but don't block).
    pub fn is_advisory(&self) -> bool {
        self.limits.enforcement == "advisory"
    }

    /// Total sats spent in the given time window.
    fn sats_in_window(&self, hours: i64) -> u64 {
        let cutoff = Utc::now() - chrono::Duration::hours(hours);
        self.entries
            .iter()
            .filter(|e| e.timestamp > cutoff)
            .map(|e| e.sats)
            .sum()
    }

    /// Total sats spent in the current task.
    pub fn task_sats(&self) -> u64 {
        self.task_sats
    }

    /// Access the raw spending entries for detailed reporting.
    pub fn entries(&self) -> &[SpendingEntry] {
        &self.entries
    }

    /// Total sats in the last hour.
    pub fn hourly_sats(&self) -> u64 {
        self.sats_in_window(1)
    }

    /// Total sats in the last 24 hours.
    pub fn daily_sats(&self) -> u64 {
        self.sats_in_window(24)
    }

    /// Total sats in the last 7 days (168 hours).
    pub fn weekly_sats(&self) -> u64 {
        self.sats_in_window(168)
    }

    /// Total sats in the last 30 days (720 hours).
    pub fn monthly_sats(&self) -> u64 {
        self.sats_in_window(720)
    }

    /// Total sats across all time (lifetime).
    pub fn lifetime_sats(&self) -> u64 {
        self.entries.iter().map(|e| e.sats).sum()
    }

    /// Generate a spending report (with balance = 0; use `report_with_balance` to include wallet balance).
    pub fn report(&self) -> BudgetReport {
        self.report_with_balance(0)
    }

    /// Generate a spending report with the given wallet balance.
    pub fn report_with_balance(&self, balance: u64) -> BudgetReport {
        let mut services = HashMap::new();
        for (name, budget) in &self.services {
            services.insert(
                name.clone(),
                ServiceReport {
                    total_sats: budget.total_sats,
                    count: budget.count,
                },
            );
        }

        let hourly = self.hourly_sats();
        let daily = self.daily_sats();
        let weekly = self.weekly_sats();
        let monthly = self.monthly_sats();
        let lifetime = self.lifetime_sats();

        BudgetReport {
            task_sats: self.task_sats,
            total_operations: self.entries.len() as u64,
            services,
            hourly_sats: hourly,
            daily_sats: daily,
            weekly_sats: weekly,
            monthly_sats: monthly,
            lifetime_sats: lifetime,
            limits: LimitsReport {
                max_per_task: self.limits.max_per_task,
                max_per_hour: self.limits.max_per_hour,
                max_per_day: self.limits.max_per_day,
                max_per_week: self.limits.max_per_week,
                max_per_month: self.limits.max_per_month,
                max_lifetime: self.limits.max_lifetime,
                task_remaining: self.limits.max_per_task.saturating_sub(self.task_sats),
                hourly_remaining: self.limits.max_per_hour.saturating_sub(hourly),
                daily_remaining: self.limits.max_per_day.saturating_sub(daily),
                weekly_remaining: if self.limits.max_per_week > 0 {
                    self.limits.max_per_week.saturating_sub(weekly)
                } else {
                    0 // 0 = unlimited, remaining is meaningless
                },
                monthly_remaining: if self.limits.max_per_month > 0 {
                    self.limits.max_per_month.saturating_sub(monthly)
                } else {
                    0
                },
                lifetime_remaining: if self.limits.max_lifetime > 0 {
                    self.limits.max_lifetime.saturating_sub(lifetime)
                } else {
                    0
                },
                enforcement: self.limits.enforcement.clone(),
            },
            balance,
        }
    }

    /// Return sorted list of distinct service names that have recorded spending.
    pub fn services_used(&self) -> Vec<String> {
        let mut names: Vec<String> = self.services.keys().cloned().collect();
        names.sort();
        names
    }

    /// Reset task spending counter (call when starting a new task).
    pub fn reset_task(&mut self) {
        self.task_sats = 0;
    }

    /// Set task_sats to the last completed task's spending.
    /// Used by the global tracker so GET /budget returns meaningful per-task data
    /// between tasks (otherwise it's always 0 since each runner has its own tracker).
    pub fn set_last_task_sats(&mut self, sats: u64) {
        self.task_sats = sats;
    }

    /// Apply certificate-derived budget limits, overriding config values for non-zero fields.
    /// Called from runner/lifecycle.rs after BRC-52 cert is loaded at task start.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_cert_limits(
        &mut self,
        per_task: Option<u64>,
        per_hour: Option<u64>,
        per_day: Option<u64>,
        per_week: Option<u64>,
        per_month: Option<u64>,
        lifetime: Option<u64>,
        enforcement: Option<String>,
    ) {
        if let Some(v) = per_task {
            if v > 0 {
                self.limits.max_per_task = v;
            }
        }
        if let Some(v) = per_hour {
            if v > 0 {
                self.limits.max_per_hour = v;
            }
        }
        if let Some(v) = per_day {
            if v > 0 {
                self.limits.max_per_day = v;
            }
        }
        if let Some(v) = per_week {
            if v > 0 {
                self.limits.max_per_week = v;
            }
        }
        if let Some(v) = per_month {
            if v > 0 {
                self.limits.max_per_month = v;
            }
        }
        if let Some(v) = lifetime {
            if v > 0 {
                self.limits.max_lifetime = v;
            }
        }
        if let Some(e) = enforcement {
            if e == "strict" || e == "advisory" {
                self.limits.enforcement = e;
            }
        }
    }

    /// Returns the current effective limits (may be cert-derived or config-derived).
    pub fn limits(&self) -> &BudgetLimits {
        &self.limits
    }

    /// Insert a pre-built spending entry directly. Useful for setting up test
    /// scenarios with specific timestamps. Does NOT append to JSONL log.
    pub fn insert_entry(&mut self, entry: SpendingEntry) {
        let budget = self.services.entry(entry.service.clone()).or_default();
        budget.total_sats += entry.sats;
        budget.count += 1;
        self.entries.push(entry);
    }

    /// Merge entries from a task-level tracker into this (global) tracker.
    ///
    /// Updates in-memory state and appends to the JSONL log so that data
    /// survives server restarts.  Does NOT update `task_sats` because these
    /// entries belong to a different task context.
    pub fn merge_from(&mut self, entries: &[SpendingEntry]) {
        for entry in entries {
            let budget = self.services.entry(entry.service.clone()).or_default();
            budget.total_sats += entry.sats;
            budget.count += 1;
            self.entries.push(entry.clone());

            // Persist to global JSONL (best-effort)
            if let Some(ref path) = self.log_path {
                if let Ok(line) = serde_json::to_string(entry) {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                        let _ = writeln!(f, "{}", line);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_record_and_totals() {
        let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
        tracker.record(
            "llm",
            "think",
            200,
            serde_json::json!({"model": "gpt-5-mini"}),
        );
        tracker.record(
            "llm",
            "think",
            300,
            serde_json::json!({"model": "gpt-5-mini"}),
        );
        tracker.record("proofs", "brc18", 50, Value::Null);

        assert_eq!(tracker.task_sats(), 550);
        assert_eq!(tracker.services["llm"].total_sats, 500);
        assert_eq!(tracker.services["llm"].count, 2);
        assert_eq!(tracker.services["proofs"].total_sats, 50);
        assert_eq!(tracker.services["proofs"].count, 1);
    }

    #[test]
    fn test_check_limit_task() {
        let limits = BudgetLimits {
            max_per_task: 1000,
            max_per_hour: 100_000,
            max_per_day: 1_000_000,
            ..Default::default()
        };
        let mut tracker = BudgetTracker::new(None, limits);
        tracker.record("llm", "think", 900, Value::Null);

        // 900 + 200 > 1000 — should fail
        assert!(tracker.check_limit(200).is_err());
        // 900 + 50 <= 1000 — should pass
        assert!(tracker.check_limit(50).is_ok());
    }

    #[test]
    fn test_jsonl_persistence() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("budget.jsonl");

        let mut tracker = BudgetTracker::new(Some(log_path.clone()), BudgetLimits::default());
        tracker.record(
            "llm",
            "think",
            100,
            serde_json::json!({"model": "gpt-5-mini"}),
        );
        tracker.record("proofs", "brc18", 50, Value::Null);

        // Read back the JSONL
        let contents = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = contents.trim().lines().collect();
        assert_eq!(lines.len(), 2);

        let entry: SpendingEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry.service, "llm");
        assert_eq!(entry.sats, 100);

        let entry: SpendingEntry = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry.service, "proofs");
        assert_eq!(entry.sats, 50);
    }

    #[test]
    fn test_report() {
        let limits = BudgetLimits {
            max_per_task: 10_000,
            max_per_hour: 100_000,
            max_per_day: 1_000_000,
            ..Default::default()
        };
        let mut tracker = BudgetTracker::new(None, limits);
        tracker.record("llm", "think", 200, Value::Null);
        tracker.record("llm", "think", 150, Value::Null);
        tracker.record("proofs", "brc18", 50, Value::Null);

        let report = tracker.report();
        assert_eq!(report.task_sats, 400);
        assert_eq!(report.total_operations, 3);
        assert_eq!(report.services.len(), 2);
        assert_eq!(report.services["llm"].total_sats, 350);
        assert_eq!(report.services["llm"].count, 2);
        assert_eq!(report.limits.task_remaining, 9600);
    }

    #[test]
    fn test_reset_task() {
        let mut tracker = BudgetTracker::new(None, BudgetLimits::default());
        tracker.record("llm", "think", 500, Value::Null);
        assert_eq!(tracker.task_sats(), 500);

        tracker.reset_task();
        assert_eq!(tracker.task_sats(), 0);
        // But hourly/daily totals still include old entries
        assert_eq!(tracker.hourly_sats(), 500);
    }

    #[test]
    fn test_from_config() {
        let config = BudgetConfig {
            max_per_task: 1000,
            max_per_hour: 2000,
            max_per_day: 3000,
            low_power_threshold: 500,
            staging_threshold: 500_000,
            ..Default::default()
        };
        let dir = tempdir().unwrap();
        let tracker = BudgetTracker::from_config(&config, dir.path());
        assert_eq!(tracker.limits.max_per_task, 1000);
        assert_eq!(tracker.limits.max_per_hour, 2000);
        assert_eq!(tracker.limits.max_per_day, 3000);
    }

    #[test]
    fn test_spending_entry_serde() {
        let entry = SpendingEntry {
            timestamp: Utc::now(),
            service: "llm".into(),
            operation: "think".into(),
            sats: 200,
            details: serde_json::json!({"model": "gpt-5-mini", "tokens": 150}),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: SpendingEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.service, "llm");
        assert_eq!(parsed.sats, 200);
    }

    #[test]
    fn test_null_details_skipped() {
        let entry = SpendingEntry {
            timestamp: Utc::now(),
            service: "proofs".into(),
            operation: "brc18".into(),
            sats: 50,
            details: Value::Null,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("details"));
    }

    #[test]
    fn test_merge_from() {
        let dir = tempdir().unwrap();
        let global_log = dir.path().join("global_budget.jsonl");

        // Global tracker (simulating the server's tracker)
        let mut global = BudgetTracker::new(Some(global_log.clone()), BudgetLimits::default());
        global.record("llm", "think", 100, Value::Null);
        assert_eq!(global.entries().len(), 1);

        // Task-level tracker (simulating the runner's tracker)
        let task_log = dir.path().join("task_budget.jsonl");
        let mut task = BudgetTracker::new(Some(task_log), BudgetLimits::default());
        task.record("llm", "think", 200, Value::Null);
        task.record(
            "tool",
            "x402_call",
            500,
            serde_json::json!({"service": "x-research"}),
        );
        task.record("proofs", "brc18", 200, Value::Null);

        // Merge task entries into global
        let entries = task.entries().to_vec();
        global.merge_from(&entries);

        // In-memory state updated
        assert_eq!(global.entries().len(), 4); // 1 original + 3 merged
        assert_eq!(global.services["llm"].total_sats, 300); // 100 + 200
        assert_eq!(global.services["tool"].total_sats, 500);
        assert_eq!(global.services["proofs"].total_sats, 200);

        // task_sats NOT updated (belongs to different task context)
        assert_eq!(global.task_sats(), 100); // only the original record()

        // Global JSONL persisted (1 original + 3 merged = 4 lines)
        let lines: Vec<String> = std::fs::read_to_string(&global_log)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_hourly_window() {
        let mut tracker = BudgetTracker::new(None, BudgetLimits::default());

        // Add a current entry
        tracker.record("llm", "think", 100, Value::Null);

        // Manually add an old entry (2 hours ago)
        tracker.entries.push(SpendingEntry {
            timestamp: Utc::now() - chrono::Duration::hours(2),
            service: "llm".into(),
            operation: "think".into(),
            sats: 999,
            details: Value::Null,
        });

        // Hourly should only count the recent one
        assert_eq!(tracker.hourly_sats(), 100);
        // Daily should count both
        assert_eq!(tracker.daily_sats(), 1099);
    }
}
