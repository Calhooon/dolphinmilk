//! Tool approval gate — file-based manual approval workflow for high-risk tools.
//!
//! Extracted from `step.rs` (#208). All methods remain `impl DmLoop`.

use std::collections::HashMap;

use serde_json::Value;

use crate::events::StepEvent;

use super::step::StepContext;
use super::{emit_event, DmLoop};

impl DmLoop {
    /// Wait for manual approval of a tool call.
    ///
    /// Stages the tool call, emits an `ApprovalRequired` SSE event, records
    /// a transcript entry, then polls the file system for an approval or
    /// abort signal until the timeout elapses.
    ///
    /// Approval is granted by creating `pending_approval/{call_id}-approved.json`.
    /// Abort is signaled by creating `pending_approval/{call_id}-aborted.json`.
    /// The server's `POST /staged/{ref}/approve` and `POST /staged/{ref}/abort`
    /// handlers write these files when the staged_ref starts with `tool-`.
    ///
    /// Returns `(output, success)`:
    /// - `("", true)` if approved — caller should proceed with execution.
    /// - `("Error: ...", false)` if aborted or timed out.
    pub(crate) async fn wait_for_approval(
        &mut self,
        name: &str,
        call_id: &str,
        arguments_str: &str,
        ctx: &StepContext,
    ) -> (String, bool) {
        let staged_ref = format!("tool-{}-{}", name, call_id);
        let timeout_secs = self.approval_timeout_secs;

        tracing::info!(
            "Tool '{}' requires approval — staging as '{}' (timeout: {}s)",
            name,
            staged_ref,
            timeout_secs
        );

        // Record in transcript
        let mut data = HashMap::new();
        data.insert(
            "type".to_string(),
            Value::String("approval_required".to_string()),
        );
        data.insert("tool".to_string(), Value::String(name.to_string()));
        data.insert("call_id".to_string(), Value::String(call_id.to_string()));
        data.insert("staged_ref".to_string(), Value::String(staged_ref.clone()));
        data.insert(
            "arguments".to_string(),
            Value::String(arguments_str.to_string()),
        );
        data.insert(
            "timeout_secs".to_string(),
            Value::Number(serde_json::Number::from(timeout_secs)),
        );
        self.transcript.record("system", data);

        // Emit SSE event
        emit_event(
            ctx,
            StepEvent::ApprovalRequired {
                iteration: self.state.exec.iteration,
                call_id: call_id.to_string(),
                name: name.to_string(),
                arguments: arguments_str.to_string(),
                staged_ref: staged_ref.clone(),
                timeout_secs,
            },
        );

        // Insert into staged_transactions map for /staged API visibility
        if let Some(ref staged_map) = self.approval_tx {
            let entry = crate::server::StagedTransaction {
                reference: staged_ref.clone(),
                amount_sats: 0,
                service: format!("tool:{}", name),
                created_at: chrono::Utc::now().to_rfc3339(),
                status: "pending".to_string(),
                task_id: Some(self.state.storage.task_id.clone()),
                description: Some(format!(
                    "Approval required for tool '{}' with args: {}",
                    name,
                    &arguments_str[..arguments_str.len().min(200)]
                )),
            };
            staged_map.lock().await.insert(staged_ref.clone(), entry);
        }

        // Write approval request file
        let approval_dir = self.workspace.join("pending_approval");
        let _ = std::fs::create_dir_all(&approval_dir);
        let approval_file = approval_dir.join(format!("{}.json", call_id));
        let approval_data = serde_json::json!({
            "staged_ref": staged_ref,
            "tool": name,
            "call_id": call_id,
            "arguments": arguments_str,
            "timeout_secs": timeout_secs,
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        let _ = std::fs::write(
            &approval_file,
            serde_json::to_string_pretty(&approval_data).unwrap_or_default(),
        );

        // Poll for approval/abort/timeout
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

        loop {
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    "Approval timeout for tool '{}' (staged_ref={})",
                    name,
                    staged_ref
                );
                // Update staged map status
                if let Some(ref staged_map) = self.approval_tx {
                    let mut map = staged_map.lock().await;
                    if let Some(entry) = map.get_mut(&staged_ref) {
                        entry.status = "timeout".to_string();
                    }
                }
                let _ = std::fs::remove_file(&approval_file);
                return (format!(
                    "Error: Tool '{}' approval timed out after {}s. The tool call was auto-aborted. \
                     Use POST /staged/{}/approve before the timeout to authorize future calls.",
                    name, timeout_secs, staged_ref
                ), false);
            }

            // Check file-based approval signals
            let approved_file = approval_dir.join(format!("{}-approved.json", call_id));
            let aborted_file = approval_dir.join(format!("{}-aborted.json", call_id));
            if approved_file.exists() {
                let _ = std::fs::remove_file(&approval_file);
                let _ = std::fs::remove_file(&approved_file);
                tracing::info!("Tool '{}' approved (staged_ref={})", name, staged_ref);
                return (String::new(), true);
            }
            if aborted_file.exists() {
                let _ = std::fs::remove_file(&approval_file);
                let _ = std::fs::remove_file(&aborted_file);
                tracing::info!("Tool '{}' aborted (staged_ref={})", name, staged_ref);
                return (
                    format!(
                        "Error: Tool '{}' was manually aborted. The tool call was not executed.",
                        name
                    ),
                    false,
                );
            }

            // Sleep briefly before next poll
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}
