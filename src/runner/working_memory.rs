//! Working memory — scratch pad that persists across iterations within a task.
//!
//! The agent writes key findings here, and they're injected into the system
//! prompt every iteration — immune to context compaction.
//!
//! Caps: 32 KB total, 8 KB per key, auto-expire after 50 iterations.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Total working memory cap (32 KB).
pub const WORKING_MEMORY_CAP: usize = 32_768;

/// Per-key value cap (8 KB).
pub const WORKING_MEMORY_KEY_CAP: usize = 8_192;

/// Auto-expire entries older than this many iterations.
pub const WORKING_MEMORY_TTL_ITERATIONS: u32 = 50;

/// A single working memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemoryEntry {
    pub value: String,
    pub iteration_set: u32,
    pub byte_size: usize,
}

/// In-memory scratch pad scoped to a single task run.
#[derive(Debug, Default)]
pub struct WorkingMemory {
    entries: HashMap<String, WorkingMemoryEntry>,
    total_bytes: usize,
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
        }
    }

    /// Set a key. Returns `Ok((bytes_used, bytes_remaining))` or `Err` if over cap.
    pub fn set(
        &mut self,
        key: String,
        value: String,
        iteration: u32,
    ) -> Result<(usize, usize), String> {
        let byte_size = key.len() + value.len();

        // Per-key cap
        if value.len() > WORKING_MEMORY_KEY_CAP {
            return Err(format!(
                "Value too large: {} bytes (max {} per key)",
                value.len(),
                WORKING_MEMORY_KEY_CAP
            ));
        }

        // Remove old entry's bytes if overwriting
        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.byte_size);
        }

        // Total cap
        if self.total_bytes + byte_size > WORKING_MEMORY_CAP {
            return Err(format!(
                "Working memory full: {} + {} > {} bytes",
                self.total_bytes, byte_size, WORKING_MEMORY_CAP
            ));
        }

        self.total_bytes += byte_size;
        self.entries.insert(
            key,
            WorkingMemoryEntry {
                value,
                iteration_set: iteration,
                byte_size,
            },
        );

        let remaining = WORKING_MEMORY_CAP.saturating_sub(self.total_bytes);
        Ok((self.total_bytes, remaining))
    }

    /// Clear a key. Returns `true` if it existed.
    pub fn clear(&mut self, key: &str) -> bool {
        if let Some(entry) = self.entries.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.byte_size);
            true
        } else {
            false
        }
    }

    /// Clear all keys. Returns number of keys removed.
    pub fn clear_all(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        self.total_bytes = 0;
        count
    }

    /// List all keys with metadata: `(key, bytes, iteration_set)`.
    pub fn list(&self) -> Vec<(String, usize, u32)> {
        let mut items: Vec<_> = self
            .entries
            .iter()
            .map(|(k, e)| (k.clone(), e.byte_size, e.iteration_set))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    /// Total bytes used and remaining.
    pub fn usage(&self) -> (usize, usize) {
        (
            self.total_bytes,
            WORKING_MEMORY_CAP.saturating_sub(self.total_bytes),
        )
    }

    /// Expire entries older than `WORKING_MEMORY_TTL_ITERATIONS` iterations.
    /// Returns the keys that were expired.
    pub fn expire(&mut self, current_iteration: u32) -> Vec<String> {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                current_iteration.saturating_sub(e.iteration_set) >= WORKING_MEMORY_TTL_ITERATIONS
            })
            .map(|(k, _)| k.clone())
            .collect();

        for key in &expired {
            if let Some(entry) = self.entries.remove(key) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.byte_size);
            }
        }

        expired
    }

    /// Format for system prompt injection.
    ///
    /// Returns a human-readable block like:
    /// ```text
    /// **key1**: value1
    /// **key2**: value2
    /// (2 keys, 0.1 KB / 32 KB used)
    /// ```
    pub fn format_for_prompt(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut lines: Vec<String> = Vec::new();
        let mut sorted: Vec<_> = self.entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));

        for (key, entry) in &sorted {
            lines.push(format!("**{}**: {}", key, entry.value));
        }

        let kb_used = self.total_bytes as f64 / 1024.0;
        let kb_cap = WORKING_MEMORY_CAP as f64 / 1024.0;
        lines.push(format!(
            "({} {}, {:.1} KB / {:.0} KB used)",
            self.entries.len(),
            if self.entries.len() == 1 {
                "key"
            } else {
                "keys"
            },
            kb_used,
            kb_cap,
        ));

        lines.join("\n")
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get a value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|e| e.value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_set_get_clear() {
        let mut wm = WorkingMemory::new();
        assert!(wm.is_empty());

        let result = wm.set("key1".into(), "value1".into(), 1);
        assert!(result.is_ok());
        assert_eq!(wm.get("key1"), Some("value1"));
        assert!(!wm.is_empty());

        assert!(wm.clear("key1"));
        assert!(wm.is_empty());
        assert_eq!(wm.get("key1"), None);
    }

    #[test]
    fn test_key_overwrite_updates_bytes() {
        let mut wm = WorkingMemory::new();
        wm.set("key".into(), "short".into(), 1).unwrap();
        let (used1, _) = wm.usage();

        wm.set("key".into(), "a much longer value".into(), 2)
            .unwrap();
        let (used2, _) = wm.usage();

        // New entry is larger, so total bytes should increase
        assert!(used2 > used1);
        assert_eq!(wm.get("key"), Some("a much longer value"));
    }

    #[test]
    fn test_per_key_cap() {
        let mut wm = WorkingMemory::new();
        let big = "x".repeat(WORKING_MEMORY_KEY_CAP + 1);
        let result = wm.set("big".into(), big, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Value too large"));
    }

    #[test]
    fn test_total_cap() {
        let mut wm = WorkingMemory::new();
        // Fill up to near the cap
        let chunk = "x".repeat(WORKING_MEMORY_KEY_CAP);
        for i in 0..3 {
            wm.set(format!("k{i}"), chunk.clone(), 1).unwrap();
        }
        // This should push over the 32KB limit
        let result = wm.set("overflow".into(), chunk, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Working memory full"));
    }

    #[test]
    fn test_ttl_expiry() {
        let mut wm = WorkingMemory::new();
        wm.set("old".into(), "stale".into(), 1).unwrap();
        wm.set("new".into(), "fresh".into(), 40).unwrap();

        let expired = wm.expire(51);
        assert_eq!(expired, vec!["old".to_string()]);
        assert_eq!(wm.get("old"), None);
        assert_eq!(wm.get("new"), Some("fresh"));
    }

    #[test]
    fn test_ttl_nothing_expired() {
        let mut wm = WorkingMemory::new();
        wm.set("recent".into(), "val".into(), 10).unwrap();
        let expired = wm.expire(15);
        assert!(expired.is_empty());
    }

    #[test]
    fn test_clear_all() {
        let mut wm = WorkingMemory::new();
        wm.set("a".into(), "1".into(), 1).unwrap();
        wm.set("b".into(), "2".into(), 1).unwrap();
        wm.set("c".into(), "3".into(), 1).unwrap();

        let count = wm.clear_all();
        assert_eq!(count, 3);
        assert!(wm.is_empty());
        assert_eq!(wm.usage(), (0, WORKING_MEMORY_CAP));
    }

    #[test]
    fn test_clear_nonexistent() {
        let mut wm = WorkingMemory::new();
        assert!(!wm.clear("nope"));
    }

    #[test]
    fn test_list_returns_correct_metadata() {
        let mut wm = WorkingMemory::new();
        wm.set("alpha".into(), "val_a".into(), 5).unwrap();
        wm.set("beta".into(), "val_b".into(), 10).unwrap();

        let list = wm.list();
        assert_eq!(list.len(), 2);
        // Sorted alphabetically
        assert_eq!(list[0].0, "alpha");
        assert_eq!(list[0].2, 5); // iteration_set
        assert_eq!(list[1].0, "beta");
        assert_eq!(list[1].2, 10);
    }

    #[test]
    fn test_format_for_prompt_output() {
        let mut wm = WorkingMemory::new();
        wm.set("api_endpoint".into(), "https://example.com".into(), 1)
            .unwrap();
        wm.set("finding".into(), "The bug is in line 42".into(), 2)
            .unwrap();

        let output = wm.format_for_prompt();
        assert!(output.contains("**api_endpoint**: https://example.com"));
        assert!(output.contains("**finding**: The bug is in line 42"));
        assert!(output.contains("2 keys"));
        assert!(output.contains("KB used"));
    }

    #[test]
    fn test_empty_format_for_prompt() {
        let wm = WorkingMemory::new();
        assert_eq!(wm.format_for_prompt(), "");
    }

    #[test]
    fn test_usage_tracking() {
        let mut wm = WorkingMemory::new();
        let (used, remaining) = wm.usage();
        assert_eq!(used, 0);
        assert_eq!(remaining, WORKING_MEMORY_CAP);

        wm.set("k".into(), "v".into(), 1).unwrap();
        let (used, remaining) = wm.usage();
        assert_eq!(used, 2); // "k" + "v" = 2 bytes
        assert_eq!(remaining, WORKING_MEMORY_CAP - 2);
    }

    #[test]
    fn test_set_returns_usage() {
        let mut wm = WorkingMemory::new();
        let (used, remaining) = wm.set("key".into(), "val".into(), 1).unwrap();
        assert_eq!(used, 6); // "key" + "val"
        assert_eq!(remaining, WORKING_MEMORY_CAP - 6);
    }

    #[test]
    fn test_exact_key_cap_allowed() {
        let mut wm = WorkingMemory::new();
        let val = "x".repeat(WORKING_MEMORY_KEY_CAP);
        assert!(wm.set("k".into(), val, 1).is_ok());
    }

    #[test]
    fn test_single_key_format() {
        let mut wm = WorkingMemory::new();
        wm.set("solo".into(), "only one".into(), 1).unwrap();
        let output = wm.format_for_prompt();
        assert!(output.contains("1 key,"));
    }
}
