//! Skill hooks — PreToolUse handlers that fire before tool execution.
//!
//! Skills register hooks during activation. Before each tool execution, the
//! `HookRegistry` iterates hooks by priority (lower fires first). If any hook
//! returns `Deny`, the tool call is aborted. If a hook returns `Modify`, the
//! tool input is updated in place.

use serde::{Deserialize, Serialize};

/// A hook registered by a skill to intercept tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreToolUseHook {
    /// Name of the skill that registered this hook.
    pub skill_name: String,
    /// Glob pattern matching tool names (e.g. "execute_*", "wallet_call", "*").
    pub tool_pattern: String,
    /// Priority — lower values fire first. Default is 0.
    pub priority: i32,
}

/// Result of a hook evaluation.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Allow the tool call to proceed unchanged.
    Allow,
    /// Deny the tool call with a reason.
    Deny(String),
    /// Modify the tool input JSON before execution.
    Modify(serde_json::Value),
}

/// Callback type for hook evaluation.
///
/// Takes the tool name and current input, returns a `HookResult`.
/// This is a simple synchronous function pointer — hooks should be fast
/// and side-effect-free (no I/O, no network calls).
pub type HookCallback = Box<dyn Fn(&str, &serde_json::Value) -> HookResult + Send + Sync>;

/// A registered hook with its metadata and callback.
pub struct RegisteredHook {
    /// Hook metadata (skill name, pattern, priority).
    pub hook: PreToolUseHook,
    /// The callback that evaluates the hook.
    pub callback: HookCallback,
}

impl std::fmt::Debug for RegisteredHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredHook")
            .field("hook", &self.hook)
            .field("callback", &"<fn>")
            .finish()
    }
}

/// Registry of PreToolUse hooks, sorted by priority.
#[derive(Default)]
pub struct HookRegistry {
    hooks: Vec<RegisteredHook>,
}

impl std::fmt::Debug for HookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookRegistry")
            .field("hooks_count", &self.hooks.len())
            .finish()
    }
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Register a new hook. Hooks are kept sorted by priority (ascending).
    pub fn register(&mut self, hook: PreToolUseHook, callback: HookCallback) {
        let registered = RegisteredHook { hook, callback };
        self.hooks.push(registered);
        // Re-sort by priority (stable sort preserves insertion order for equal priorities)
        self.hooks.sort_by_key(|h| h.hook.priority);
    }

    /// Remove all hooks registered by a given skill.
    pub fn remove_by_skill(&mut self, skill_name: &str) {
        self.hooks.retain(|h| h.hook.skill_name != skill_name);
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Whether the registry has no hooks.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// List all registered hook metadata (without callbacks).
    pub fn list_hooks(&self) -> Vec<&PreToolUseHook> {
        self.hooks.iter().map(|h| &h.hook).collect()
    }

    /// Evaluate all matching hooks for a tool call.
    ///
    /// Hooks are evaluated in priority order (lowest first). Returns:
    /// - `HookResult::Deny(reason)` if any hook denies — stops immediately
    /// - `HookResult::Modify(new_input)` with the last modified input if any hook modified
    /// - `HookResult::Allow` if all hooks allowed without modification
    ///
    /// When multiple hooks modify the input, each successive hook sees the
    /// previously modified input (chained modifications).
    pub fn evaluate(&self, tool_name: &str, input: &serde_json::Value) -> HookResult {
        let mut current_input = input.clone();
        let mut was_modified = false;

        for registered in &self.hooks {
            if !matches_tool_pattern(&registered.hook.tool_pattern, tool_name) {
                continue;
            }

            match (registered.callback)(tool_name, &current_input) {
                HookResult::Allow => {
                    // Continue to next hook
                }
                HookResult::Deny(reason) => {
                    tracing::info!(
                        skill = %registered.hook.skill_name,
                        tool = %tool_name,
                        reason = %reason,
                        "PreToolUse hook denied tool call"
                    );
                    return HookResult::Deny(reason);
                }
                HookResult::Modify(new_input) => {
                    tracing::debug!(
                        skill = %registered.hook.skill_name,
                        tool = %tool_name,
                        "PreToolUse hook modified tool input"
                    );
                    current_input = new_input;
                    was_modified = true;
                }
            }
        }

        if was_modified {
            HookResult::Modify(current_input)
        } else {
            HookResult::Allow
        }
    }
}

/// Match a tool name against a glob-like pattern.
///
/// Supports:
/// - `*` — matches everything
/// - `prefix*` — starts-with matching
/// - `*suffix` — ends-with matching
/// - `prefix*suffix` — starts-with AND ends-with
/// - exact match (no wildcards)
fn matches_tool_pattern(pattern: &str, tool_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    match pattern.find('*') {
        None => {
            // Exact match
            pattern == tool_name
        }
        Some(star_pos) => {
            let prefix = &pattern[..star_pos];
            let suffix = &pattern[star_pos + 1..];

            if suffix.is_empty() {
                // "prefix*"
                tool_name.starts_with(prefix)
            } else if prefix.is_empty() {
                // "*suffix"
                tool_name.ends_with(suffix)
            } else {
                // "prefix*suffix"
                tool_name.starts_with(prefix)
                    && tool_name.ends_with(suffix)
                    && tool_name.len() >= prefix.len() + suffix.len()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_matches_tool_pattern_exact() {
        assert!(matches_tool_pattern("execute_bash", "execute_bash"));
        assert!(!matches_tool_pattern("execute_bash", "file_read"));
    }

    #[test]
    fn test_matches_tool_pattern_wildcard_all() {
        assert!(matches_tool_pattern("*", "anything"));
        assert!(matches_tool_pattern("*", ""));
    }

    #[test]
    fn test_matches_tool_pattern_prefix() {
        assert!(matches_tool_pattern("execute_*", "execute_bash"));
        assert!(matches_tool_pattern("execute_*", "execute_python"));
        assert!(!matches_tool_pattern("execute_*", "file_read"));
    }

    #[test]
    fn test_matches_tool_pattern_suffix() {
        assert!(matches_tool_pattern("*_call", "wallet_call"));
        assert!(matches_tool_pattern("*_call", "x402_call"));
        assert!(!matches_tool_pattern("*_call", "execute_bash"));
    }

    #[test]
    fn test_matches_tool_pattern_prefix_and_suffix() {
        assert!(matches_tool_pattern("wallet_*_call", "wallet_some_call"));
        assert!(!matches_tool_pattern("wallet_*_call", "wallet_call"));
    }

    #[test]
    fn test_hook_registry_register_and_priority_order() {
        let mut registry = HookRegistry::new();

        registry.register(
            PreToolUseHook {
                skill_name: "high".to_string(),
                tool_pattern: "*".to_string(),
                priority: 10,
            },
            Box::new(|_, _| HookResult::Allow),
        );

        registry.register(
            PreToolUseHook {
                skill_name: "low".to_string(),
                tool_pattern: "*".to_string(),
                priority: -5,
            },
            Box::new(|_, _| HookResult::Allow),
        );

        registry.register(
            PreToolUseHook {
                skill_name: "mid".to_string(),
                tool_pattern: "*".to_string(),
                priority: 0,
            },
            Box::new(|_, _| HookResult::Allow),
        );

        assert_eq!(registry.len(), 3);
        let hooks = registry.list_hooks();
        assert_eq!(hooks[0].skill_name, "low");
        assert_eq!(hooks[1].skill_name, "mid");
        assert_eq!(hooks[2].skill_name, "high");
    }

    #[test]
    fn test_hook_deny_blocks_execution() {
        let mut registry = HookRegistry::new();

        registry.register(
            PreToolUseHook {
                skill_name: "guard".to_string(),
                tool_pattern: "execute_*".to_string(),
                priority: 0,
            },
            Box::new(|tool_name, _| {
                if tool_name == "execute_bash" {
                    HookResult::Deny("bash execution disabled by guard skill".to_string())
                } else {
                    HookResult::Allow
                }
            }),
        );

        let result = registry.evaluate("execute_bash", &json!({"command": "rm -rf /"}));
        match result {
            HookResult::Deny(reason) => {
                assert!(reason.contains("bash execution disabled"));
            }
            _ => panic!("Expected Deny, got {:?}", result),
        }

        // Non-matching tool should pass
        let result = registry.evaluate("file_read", &json!({"path": "/tmp/test"}));
        assert!(matches!(result, HookResult::Allow));
    }

    #[test]
    fn test_hook_modify_changes_input() {
        let mut registry = HookRegistry::new();

        registry.register(
            PreToolUseHook {
                skill_name: "sanitizer".to_string(),
                tool_pattern: "wallet_call".to_string(),
                priority: 0,
            },
            Box::new(|_, input| {
                let mut modified = input.clone();
                if let Some(obj) = modified.as_object_mut() {
                    obj.insert("injected".to_string(), json!(true));
                }
                HookResult::Modify(modified)
            }),
        );

        let input = json!({"method": "getPublicKey"});
        let result = registry.evaluate("wallet_call", &input);
        match result {
            HookResult::Modify(new_input) => {
                assert_eq!(new_input["method"], "getPublicKey");
                assert_eq!(new_input["injected"], true);
            }
            _ => panic!("Expected Modify, got {:?}", result),
        }
    }

    #[test]
    fn test_deny_stops_chain() {
        let mut registry = HookRegistry::new();

        // First hook (priority -1) denies
        registry.register(
            PreToolUseHook {
                skill_name: "blocker".to_string(),
                tool_pattern: "*".to_string(),
                priority: -1,
            },
            Box::new(|_, _| HookResult::Deny("blocked".to_string())),
        );

        // Second hook (priority 0) would modify — but should never fire
        registry.register(
            PreToolUseHook {
                skill_name: "modifier".to_string(),
                tool_pattern: "*".to_string(),
                priority: 0,
            },
            Box::new(|_, _| HookResult::Modify(json!({"should_not": "appear"}))),
        );

        let result = registry.evaluate("any_tool", &json!({}));
        match result {
            HookResult::Deny(reason) => assert_eq!(reason, "blocked"),
            _ => panic!("Expected Deny from first hook"),
        }
    }

    #[test]
    fn test_chained_modifications() {
        let mut registry = HookRegistry::new();

        // First modifier adds field_a
        registry.register(
            PreToolUseHook {
                skill_name: "mod_a".to_string(),
                tool_pattern: "*".to_string(),
                priority: 0,
            },
            Box::new(|_, input| {
                let mut m = input.clone();
                m.as_object_mut()
                    .unwrap()
                    .insert("field_a".to_string(), json!(1));
                HookResult::Modify(m)
            }),
        );

        // Second modifier adds field_b (sees field_a from previous)
        registry.register(
            PreToolUseHook {
                skill_name: "mod_b".to_string(),
                tool_pattern: "*".to_string(),
                priority: 1,
            },
            Box::new(|_, input| {
                let mut m = input.clone();
                // Verify we can see the previous modification
                assert_eq!(input["field_a"], 1);
                m.as_object_mut()
                    .unwrap()
                    .insert("field_b".to_string(), json!(2));
                HookResult::Modify(m)
            }),
        );

        let result = registry.evaluate("any_tool", &json!({}));
        match result {
            HookResult::Modify(v) => {
                assert_eq!(v["field_a"], 1);
                assert_eq!(v["field_b"], 2);
            }
            _ => panic!("Expected Modify with both fields"),
        }
    }

    #[test]
    fn test_remove_by_skill() {
        let mut registry = HookRegistry::new();

        registry.register(
            PreToolUseHook {
                skill_name: "keep".to_string(),
                tool_pattern: "*".to_string(),
                priority: 0,
            },
            Box::new(|_, _| HookResult::Allow),
        );
        registry.register(
            PreToolUseHook {
                skill_name: "remove_me".to_string(),
                tool_pattern: "*".to_string(),
                priority: 0,
            },
            Box::new(|_, _| HookResult::Deny("should be removed".to_string())),
        );

        assert_eq!(registry.len(), 2);
        registry.remove_by_skill("remove_me");
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.list_hooks()[0].skill_name, "keep");
    }

    #[test]
    fn test_no_hooks_allows() {
        let registry = HookRegistry::new();
        let result = registry.evaluate("any_tool", &json!({}));
        assert!(matches!(result, HookResult::Allow));
    }

    #[test]
    fn test_non_matching_hooks_allows() {
        let mut registry = HookRegistry::new();
        registry.register(
            PreToolUseHook {
                skill_name: "specific".to_string(),
                tool_pattern: "wallet_call".to_string(),
                priority: 0,
            },
            Box::new(|_, _| HookResult::Deny("should not fire".to_string())),
        );

        // Different tool name — hook should not match
        let result = registry.evaluate("execute_bash", &json!({}));
        assert!(matches!(result, HookResult::Allow));
    }
}
