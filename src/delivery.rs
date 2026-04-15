//! Delivery queue — persistent retry for failed outbound MessageBox messages.
//!
//! When a task completes and the reply message fails to send (e.g., MessageBox
//! server unavailable, network timeout), the message is enqueued as a JSON file
//! in `workspace/delivery_queue/`. The scheduler retries due items on each tick
//! with exponential backoff (5s, 25s, 120s, 600s, 600s cap). After 5 failures,
//! the delivery is moved to `delivery_queue/failed/` for manual inspection.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A pending outbound delivery awaiting retry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDelivery {
    pub id: String,
    pub recipient: String,
    pub message_box: String,
    pub body: serde_json::Value,
    pub created_at: String, // RFC 3339
    pub retry_count: u32,
    pub next_retry_at: String, // RFC 3339
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Backoff schedule in seconds: 5, 25, 120, 600, 600 (capped at 10 minutes).
const BACKOFF_SECS: &[u64] = &[5, 25, 120, 600, 600];

/// Maximum number of retry attempts before moving to failed/.
const MAX_RETRIES: u32 = 5;

/// Persistent delivery queue backed by JSON files in `workspace/delivery_queue/`.
pub struct DeliveryQueue {
    dir: PathBuf,
}

impl DeliveryQueue {
    /// Create a new DeliveryQueue backed by `workspace/delivery_queue/`.
    pub fn new(workspace: &std::path::Path) -> Self {
        Self {
            dir: workspace.join("delivery_queue"),
        }
    }

    /// Persist a new pending delivery to disk. Returns the delivery ID.
    pub fn enqueue(
        &self,
        recipient: &str,
        message_box: &str,
        body: serde_json::Value,
    ) -> Result<String, std::io::Error> {
        std::fs::create_dir_all(&self.dir)?;

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let next_retry = now + chrono::Duration::seconds(BACKOFF_SECS[0] as i64);

        let delivery = PendingDelivery {
            id: id.clone(),
            recipient: recipient.to_string(),
            message_box: message_box.to_string(),
            body,
            created_at: now.to_rfc3339(),
            retry_count: 0,
            next_retry_at: next_retry.to_rfc3339(),
            last_error: None,
        };

        let path = self.dir.join(format!("{id}.json"));
        let content = serde_json::to_string_pretty(&delivery).map_err(std::io::Error::other)?;
        std::fs::write(path, content)?;
        Ok(id)
    }

    /// Return all deliveries whose `next_retry_at` has elapsed.
    pub fn due_items(&self) -> Vec<PendingDelivery> {
        let now = chrono::Utc::now();
        let mut result = Vec::new();

        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return result,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let delivery: PendingDelivery = match serde_json::from_str(&content) {
                Ok(d) => d,
                Err(_) => continue,
            };

            if let Ok(next_retry) = chrono::DateTime::parse_from_rfc3339(&delivery.next_retry_at) {
                if next_retry <= now {
                    result.push(delivery);
                }
            }
        }

        result
    }

    /// Record a successful delivery and remove the file.
    pub fn mark_success(&self, id: &str) {
        let path = self.dir.join(format!("{id}.json"));
        let _ = std::fs::remove_file(path);
    }

    /// Record a failed attempt. Increment retry count, update next_retry_at with backoff.
    /// If max retries exceeded, move to `failed/` subdirectory.
    pub fn mark_failed(&self, id: &str, error: &str) {
        let path = self.dir.join(format!("{id}.json"));
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut delivery: PendingDelivery = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(_) => return,
        };

        delivery.retry_count += 1;
        delivery.last_error = Some(error.to_string());

        if delivery.retry_count >= MAX_RETRIES {
            // Move to failed/ subdirectory
            let failed_dir = self.dir.join("failed");
            let _ = std::fs::create_dir_all(&failed_dir);
            let failed_path = failed_dir.join(format!("{id}.json"));
            if let Ok(updated) = serde_json::to_string_pretty(&delivery) {
                let _ = std::fs::write(&failed_path, updated);
            }
            let _ = std::fs::remove_file(path);
            tracing::warn!(
                "Delivery {id}: permanently failed after {MAX_RETRIES} retries. Error: {error}",
            );
        } else {
            // Apply exponential backoff
            let backoff_idx = (delivery.retry_count as usize).min(BACKOFF_SECS.len() - 1);
            let next_retry =
                chrono::Utc::now() + chrono::Duration::seconds(BACKOFF_SECS[backoff_idx] as i64);
            delivery.next_retry_at = next_retry.to_rfc3339();

            if let Ok(updated) = serde_json::to_string_pretty(&delivery) {
                let _ = std::fs::write(path, updated);
            }
            tracing::info!(
                "Delivery {id}: retry {}/{MAX_RETRIES} scheduled in {}s. Error: {error}",
                delivery.retry_count,
                BACKOFF_SECS[backoff_idx],
            );
        }
    }

    /// Count all pending deliveries (excluding failed/).
    pub fn pending_count(&self) -> usize {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        entries
            .flatten()
            .filter(|e| {
                let p = e.path();
                p.is_file() && p.extension().is_some_and(|ext| ext == "json")
            })
            .count()
    }

    /// Scan for pending deliveries on startup (crash recovery).
    /// Returns the count of pending deliveries found.
    pub fn scan_on_startup(&self) -> usize {
        self.pending_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enqueue_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());

        let id = queue
            .enqueue(
                "recipient-key",
                "results_inbox",
                serde_json::json!({"result": "ok"}),
            )
            .unwrap();

        let path = dir.path().join("delivery_queue").join(format!("{id}.json"));
        assert!(path.exists());

        let content: PendingDelivery =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content.recipient, "recipient-key");
        assert_eq!(content.message_box, "results_inbox");
        assert_eq!(content.retry_count, 0);
        assert!(content.last_error.is_none());
    }

    #[test]
    fn test_due_items_returns_elapsed_only() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());
        std::fs::create_dir_all(dir.path().join("delivery_queue")).unwrap();

        // Item 1: due now (next_retry_at in the past)
        let past = (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        let d1 = PendingDelivery {
            id: "due-1".to_string(),
            recipient: "r".to_string(),
            message_box: "inbox".to_string(),
            body: serde_json::json!({}),
            created_at: past.clone(),
            retry_count: 0,
            next_retry_at: past,
            last_error: None,
        };
        std::fs::write(
            dir.path().join("delivery_queue/due-1.json"),
            serde_json::to_string(&d1).unwrap(),
        )
        .unwrap();

        // Item 2: not due (next_retry_at in the future)
        let future = (chrono::Utc::now() + chrono::Duration::seconds(3600)).to_rfc3339();
        let d2 = PendingDelivery {
            id: "future-1".to_string(),
            recipient: "r".to_string(),
            message_box: "inbox".to_string(),
            body: serde_json::json!({}),
            created_at: chrono::Utc::now().to_rfc3339(),
            retry_count: 0,
            next_retry_at: future,
            last_error: None,
        };
        std::fs::write(
            dir.path().join("delivery_queue/future-1.json"),
            serde_json::to_string(&d2).unwrap(),
        )
        .unwrap();

        let due = queue.due_items();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "due-1");
    }

    #[test]
    fn test_mark_success_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());

        let id = queue.enqueue("r", "inbox", serde_json::json!({})).unwrap();
        assert_eq!(queue.pending_count(), 1);

        queue.mark_success(&id);
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn test_mark_failed_applies_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());

        let id = queue.enqueue("r", "inbox", serde_json::json!({})).unwrap();

        // Fail once — should schedule retry at +25s (BACKOFF_SECS[1])
        queue.mark_failed(&id, "network error");

        let path = dir.path().join("delivery_queue").join(format!("{id}.json"));
        let content: PendingDelivery =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content.retry_count, 1);
        assert_eq!(content.last_error.as_deref(), Some("network error"));

        // The next_retry_at should be in the future (backoff applied)
        let next = chrono::DateTime::parse_from_rfc3339(&content.next_retry_at).unwrap();
        assert!(next > chrono::Utc::now());
    }

    #[test]
    fn test_mark_failed_moves_to_failed_after_max_retries() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());

        let id = queue.enqueue("r", "inbox", serde_json::json!({})).unwrap();

        // Fail MAX_RETRIES times
        for i in 0..5 {
            queue.mark_failed(&id, &format!("error {i}"));
        }

        // Original file should be gone
        let path = dir.path().join("delivery_queue").join(format!("{id}.json"));
        assert!(!path.exists());

        // Should be in failed/ subdirectory
        let failed_path = dir
            .path()
            .join("delivery_queue/failed")
            .join(format!("{id}.json"));
        assert!(failed_path.exists());

        let content: PendingDelivery =
            serde_json::from_str(&std::fs::read_to_string(&failed_path).unwrap()).unwrap();
        assert_eq!(content.retry_count, 5);
    }

    #[test]
    fn test_scan_on_startup_counts_pending() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());

        assert_eq!(queue.scan_on_startup(), 0);

        queue.enqueue("r1", "inbox", serde_json::json!({})).unwrap();
        queue.enqueue("r2", "inbox", serde_json::json!({})).unwrap();

        assert_eq!(queue.scan_on_startup(), 2);
    }

    #[test]
    fn test_due_items_skips_nonexistent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DeliveryQueue::new(dir.path());
        // delivery_queue/ doesn't exist yet — should return empty, not error
        assert!(queue.due_items().is_empty());
    }

    #[test]
    fn test_pending_delivery_serde_roundtrip() {
        let delivery = PendingDelivery {
            id: "test-id".to_string(),
            recipient: "key123".to_string(),
            message_box: "results_inbox".to_string(),
            body: serde_json::json!({"task_id": "t-1", "result": "done"}),
            created_at: "2026-03-01T00:00:00Z".to_string(),
            retry_count: 2,
            next_retry_at: "2026-03-01T00:02:00Z".to_string(),
            last_error: Some("timeout".to_string()),
        };

        let json = serde_json::to_string(&delivery).unwrap();
        let parsed: PendingDelivery = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "test-id");
        assert_eq!(parsed.retry_count, 2);
        assert_eq!(parsed.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn test_pending_delivery_last_error_omitted_when_none() {
        let delivery = PendingDelivery {
            id: "id".to_string(),
            recipient: "r".to_string(),
            message_box: "box".to_string(),
            body: serde_json::json!(null),
            created_at: "2026-03-01T00:00:00Z".to_string(),
            retry_count: 0,
            next_retry_at: "2026-03-01T00:00:05Z".to_string(),
            last_error: None,
        };

        let json = serde_json::to_string(&delivery).unwrap();
        assert!(!json.contains("last_error"));
    }

    #[test]
    fn test_backoff_schedule_values() {
        assert_eq!(BACKOFF_SECS, &[5, 25, 120, 600, 600]);
        assert_eq!(MAX_RETRIES, 5);
    }
}
