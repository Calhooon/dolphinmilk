//! Hook configuration parsing from `dolphin-milk.toml`.
//!
//! Hooks are defined as an array of `[[hooks]]` entries in the config file.
//! Each entry specifies an event type, handler, priority, and behavior flags.

use serde::Deserialize;

use super::events::HookEventType;
use super::executor::HookHandler;
use super::HookRegistration;

/// Raw TOML representation of a hook entry.
///
/// ```toml
/// [[hooks]]
/// event = "pre_tool_execution"
/// handler = { type = "command", cmd = "echo 'Tool: {tool_name}'" }
/// priority = 10
/// blocking = true
/// timeout_ms = 5000
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct HookConfigEntry {
    /// Event type string (e.g., "pre_tool_execution").
    pub event: String,
    /// Handler configuration (tagged union via `type` field).
    pub handler: HookHandler,
    /// Priority — lower numbers fire first. Default: 100.
    #[serde(default = "default_priority")]
    pub priority: i32,
    /// Whether this hook blocks the operation on failure. Default: false.
    #[serde(default)]
    pub blocking: bool,
    /// Whether this hook is enabled. Default: true.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Timeout in milliseconds for handler execution. Default: 5000.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Optional human-readable ID. Auto-generated if not provided.
    pub id: Option<String>,
    /// Optional matcher pattern for selective event matching.
    /// Empty = match all events of this type.
    /// "Write|Edit|Read" = pipe-separated exact match on tool_name.
    /// "^git.*" = regex match on tool_name.
    pub matcher: Option<String>,
    /// If true, this hook self-removes after first successful fire.
    #[serde(default)]
    pub once: bool,
}

fn default_priority() -> i32 {
    100
}

fn default_enabled() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    5000
}

impl HookConfigEntry {
    /// Convert this config entry into a [`HookRegistration`].
    ///
    /// Returns `Err` if the event type string is not recognized.
    pub fn into_registration(self, index: usize) -> Result<HookRegistration, String> {
        let event_type: HookEventType = self
            .event
            .parse()
            .map_err(|e: String| format!("hook[{index}]: {e}"))?;

        let id = self
            .id
            .unwrap_or_else(|| format!("hook-{index}-{}", self.event));

        Ok(HookRegistration {
            id,
            event_type,
            handler: self.handler,
            priority: self.priority,
            blocking: self.blocking,
            enabled: self.enabled,
            timeout_ms: self.timeout_ms,
            matcher: self.matcher,
            once: self.once,
        })
    }
}

/// Parse hook registrations from a list of config entries.
///
/// Invalid entries are logged and skipped — they do not prevent
/// valid hooks from loading.
pub fn parse_hook_configs(entries: &[HookConfigEntry]) -> Vec<HookRegistration> {
    let mut registrations = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        match entry.clone().into_registration(i) {
            Ok(reg) => {
                tracing::info!(
                    "Loaded hook '{}': event={}, priority={}, blocking={}",
                    reg.id,
                    reg.event_type,
                    reg.priority,
                    reg.blocking
                );
                registrations.push(reg);
            }
            Err(e) => {
                tracing::warn!("Skipping invalid hook config: {e}");
            }
        }
    }
    registrations
}
