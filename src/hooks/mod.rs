//! Lifecycle hook system — observe, modify, or block agent behavior.
//!
//! The hook system provides extensibility through 4 handler modes
//! (Command, Prompt, Http, Agent) that can react to 20+ lifecycle events
//! across 6 categories (Tool, Permission, Session, Context, Task, Config).
//!
//! ## Design
//!
//! - **Non-intrusive**: When no hooks are registered, `fire()` returns immediately
//!   after a single empty-vec check. Zero overhead on the hot path.
//! - **Priority-ordered**: Hooks fire in ascending priority order (lower = first).
//! - **Blocking vs non-blocking**: Blocking hooks can halt an operation by returning
//!   `HookResult::Block`. Non-blocking hooks log errors but never stop execution.
//! - **Timeout-enforced**: Every handler has a configurable timeout. Commands that
//!   exceed their timeout are killed.
//!
//! ## Configuration
//!
//! ```toml
//! [[hooks]]
//! event = "pre_tool_execution"
//! handler = { type = "command", cmd = "echo '{tool_name}'" }
//! priority = 10
//! blocking = true
//! timeout_ms = 5000
//! ```

pub mod config;
pub mod events;
pub mod executor;

use std::time::Duration;

pub use config::{parse_hook_configs, HookConfigEntry};
pub use events::{HookEvent, HookEventType};
pub use executor::{HookHandler, HookResult};

/// A registered hook binding an event type to a handler.
#[derive(Debug, Clone)]
pub struct HookRegistration {
    /// Unique identifier for this hook.
    pub id: String,
    /// Which event type triggers this hook.
    pub event_type: HookEventType,
    /// How to handle the event.
    pub handler: HookHandler,
    /// Execution priority — lower numbers fire first.
    pub priority: i32,
    /// If true, a `Block` result prevents the operation.
    pub blocking: bool,
    /// Whether this hook is currently active.
    pub enabled: bool,
    /// Timeout in milliseconds for handler execution.
    pub timeout_ms: u64,
    /// Optional matcher pattern for selective event matching.
    /// Empty/None = match all events of this type.
    /// "Write|Edit" = pipe-separated exact match on tool_name.
    /// "^git.*" = regex match on tool_name.
    pub matcher: Option<String>,
    /// If true, this hook is removed after first successful execution.
    pub once: bool,
}

/// Registry of lifecycle hooks.
///
/// Hooks are stored in registration order. `fire()` filters by event type,
/// sorts by priority, and executes handlers sequentially.
pub struct HookRegistry {
    hooks: Vec<HookRegistration>,
}

impl HookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Create a registry pre-populated from config entries.
    pub fn from_config(entries: &[HookConfigEntry]) -> Self {
        let hooks = parse_hook_configs(entries);
        Self { hooks }
    }

    /// Register a new hook.
    pub fn register(&mut self, hook: HookRegistration) {
        tracing::debug!(
            "Registered hook '{}' for event {} (priority={}, blocking={})",
            hook.id,
            hook.event_type,
            hook.priority,
            hook.blocking
        );
        self.hooks.push(hook);
    }

    /// Remove a hook by ID. Returns `true` if a hook was removed.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.hooks.len();
        self.hooks.retain(|h| h.id != id);
        let removed = self.hooks.len() < before;
        if removed {
            tracing::debug!("Unregistered hook '{id}'");
        }
        removed
    }

    /// Fire an event and collect results from all matching handlers.
    ///
    /// Hooks are filtered by event type and matcher pattern, sorted by
    /// priority (ascending), and executed sequentially. For blocking hooks,
    /// a `Block` result stops execution immediately. Non-blocking hook
    /// failures are logged but do not stop processing.
    ///
    /// Hooks with `once: true` are removed after first successful execution.
    ///
    /// Returns an empty `Vec` immediately if no hooks are registered
    /// (zero overhead on the hot path).
    pub async fn fire(&mut self, event: HookEvent) -> Vec<HookResult> {
        // Fast path: no hooks registered at all
        if self.hooks.is_empty() {
            return Vec::new();
        }

        let event_type = event.event_type();
        let match_target = event.matcher_target();

        // Collect matching enabled hooks, sorted by priority
        let mut matching_indices: Vec<usize> = self
            .hooks
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                h.enabled
                    && h.event_type == event_type
                    && matches_pattern(&h.matcher, &match_target)
            })
            .map(|(i, _)| i)
            .collect();

        if matching_indices.is_empty() {
            return Vec::new();
        }

        // Sort by priority
        matching_indices.sort_by_key(|&i| self.hooks[i].priority);

        let mut results = Vec::with_capacity(matching_indices.len());
        let mut once_to_remove: Vec<String> = Vec::new();

        for &idx in &matching_indices {
            let hook = &self.hooks[idx];
            let timeout = Duration::from_millis(hook.timeout_ms);
            let result = hook.handler.execute(&event, timeout).await;

            tracing::debug!(
                "Hook '{}' for {} returned {:?}",
                hook.id,
                event_type,
                result
            );

            // Track once: true hooks that succeeded
            if hook.once && matches!(result, HookResult::Allow | HookResult::Modify(_)) {
                once_to_remove.push(hook.id.clone());
            }

            if hook.blocking {
                if let HookResult::Block(ref reason) = result {
                    tracing::info!(
                        "Blocking hook '{}' blocked {}: {}",
                        hook.id,
                        event_type,
                        reason
                    );
                    results.push(result);
                    // Remove once hooks before returning
                    self.remove_hooks_by_id(&once_to_remove);
                    return results;
                }
            }

            results.push(result);
        }

        // Remove once hooks that fired successfully
        self.remove_hooks_by_id(&once_to_remove);

        results
    }

    /// Remove hooks by their IDs (used for `once: true` cleanup).
    fn remove_hooks_by_id(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        for id in ids {
            tracing::debug!("Removing once-hook '{id}' after successful fire");
        }
        self.hooks.retain(|h| !ids.contains(&h.id));
    }

    /// Return all registered hooks.
    pub fn list(&self) -> &[HookRegistration] {
        &self.hooks
    }

    /// Count hooks registered for a specific event type.
    pub fn count_for_event(&self, event_type: &HookEventType) -> usize {
        self.hooks
            .iter()
            .filter(|h| h.enabled && h.event_type == *event_type)
            .count()
    }

    /// Return total number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Return `true` if no hooks are registered.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a matcher pattern matches the target string.
///
/// Patterns:
/// - `None` or empty string → matches everything
/// - `"foo|bar|baz"` → pipe-separated exact match (case-insensitive)
/// - `"^pattern.*"` → regex match (if starts with `^`)
/// - Other → substring match (case-insensitive)
fn matches_pattern(matcher: &Option<String>, target: &Option<String>) -> bool {
    let pattern = match matcher {
        None => return true,
        Some(p) if p.is_empty() => return true,
        Some(p) => p,
    };

    let target = match target {
        None => return true, // No target → match all
        Some(t) => t,
    };

    // Pipe-separated exact match
    if pattern.contains('|') && !pattern.starts_with('^') {
        return pattern
            .split('|')
            .any(|p| p.trim().eq_ignore_ascii_case(target));
    }

    // Regex match (starts with ^)
    if pattern.starts_with('^') {
        return regex::Regex::new(pattern)
            .map(|re| re.is_match(target))
            .unwrap_or(false);
    }

    // Substring match (case-insensitive)
    target.to_lowercase().contains(&pattern.to_lowercase())
}

/// Check hook results for any `Block` result.
///
/// Convenience function for callers that need to know if an operation
/// should be blocked without inspecting results manually.
pub fn is_blocked(results: &[HookResult]) -> Option<&str> {
    for result in results {
        if let HookResult::Block(reason) = result {
            return Some(reason);
        }
    }
    None
}
