//! Skill trait and associated types for building Dolphin Milk skills.
//!
//! A skill is a behavioral module that customizes agent behavior. Skills can
//! inject instructions into the system prompt and manage lifecycle state.
//!
//! # Example
//!
//! ```rust
//! use bsv_worm_sdk::skill::{Skill, SkillError};
//!
//! struct MySkill {
//!     active: std::sync::atomic::AtomicBool,
//! }
//!
//! impl Skill for MySkill {
//!     fn name(&self) -> &str { "my_skill" }
//!     fn description(&self) -> &str { "A custom skill" }
//!     fn instructions(&self) -> &str { "Follow these instructions..." }
//!     fn activate(&self) -> Result<(), SkillError> {
//!         self.active.store(true, std::sync::atomic::Ordering::Relaxed);
//!         Ok(())
//!     }
//!     fn deactivate(&self) -> Result<(), SkillError> {
//!         self.active.store(false, std::sync::atomic::Ordering::Relaxed);
//!         Ok(())
//!     }
//! }
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during skill operations.
#[derive(Debug, Error)]
pub enum SkillError {
    /// Skill activation failed.
    #[error("activation failed: {0}")]
    ActivationFailed(String),

    /// Skill deactivation failed.
    #[error("deactivation failed: {0}")]
    DeactivationFailed(String),

    /// Skill configuration is invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// Skill state error.
    #[error("state error: {0}")]
    StateError(String),
}

/// Persistent configuration for a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillConfig {
    /// Arbitrary JSON configuration for the skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

/// Trait for implementing a Dolphin Milk skill.
///
/// Skills customize agent behavior by providing instructions and managing
/// lifecycle state. They can be auto-activated (included in every prompt)
/// or explicitly activated by the user.
pub trait Skill: Send + Sync {
    /// Unique name for this skill (e.g. "pomodoro").
    fn name(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str;

    /// Markdown instructions injected into the system prompt when active.
    fn instructions(&self) -> &str;

    /// Whether this skill should auto-activate (included in every prompt).
    fn auto_activate(&self) -> bool {
        false
    }

    /// Tool names this skill references or requires.
    fn required_tools(&self) -> Vec<String> {
        Vec::new()
    }

    /// Activate the skill. Called when the skill is enabled.
    fn activate(&self) -> Result<(), SkillError>;

    /// Deactivate the skill. Called when the skill is disabled.
    fn deactivate(&self) -> Result<(), SkillError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSkill;

    impl Skill for TestSkill {
        fn name(&self) -> &str {
            "test_skill"
        }
        fn description(&self) -> &str {
            "A test skill"
        }
        fn instructions(&self) -> &str {
            "Do the test thing."
        }
        fn activate(&self) -> Result<(), SkillError> {
            Ok(())
        }
        fn deactivate(&self) -> Result<(), SkillError> {
            Ok(())
        }
    }

    #[test]
    fn test_skill_defaults() {
        let skill = TestSkill;
        assert_eq!(skill.name(), "test_skill");
        assert!(!skill.auto_activate());
        assert!(skill.required_tools().is_empty());
    }

    #[test]
    fn test_skill_lifecycle() {
        let skill = TestSkill;
        assert!(skill.activate().is_ok());
        assert!(skill.deactivate().is_ok());
    }

    #[test]
    fn test_skill_error_display() {
        let e = SkillError::ActivationFailed("timeout".into());
        assert_eq!(e.to_string(), "activation failed: timeout");

        let e = SkillError::InvalidConfig("missing field".into());
        assert_eq!(e.to_string(), "invalid configuration: missing field");
    }

    #[test]
    fn test_skill_config_serialization() {
        let config = SkillConfig {
            config: Some(serde_json::json!({"interval": 25})),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("interval"));

        let empty = SkillConfig::default();
        let json = serde_json::to_string(&empty).unwrap();
        assert_eq!(json, "{}");
    }
}
