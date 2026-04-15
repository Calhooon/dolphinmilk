//! Pomodoro Skill — example bsv-worm skill plugin.
//!
//! Demonstrates how to implement the `Skill` trait from `bsv-worm-sdk`.
//! When active, injects Pomodoro productivity instructions into the agent's
//! system prompt, encouraging focused work in timed intervals.

use bsv_worm_sdk::skill::{Skill, SkillError};
use std::sync::atomic::{AtomicBool, Ordering};

/// Default Pomodoro work interval in minutes.
pub const DEFAULT_WORK_MINUTES: u32 = 25;
/// Default short break in minutes.
pub const DEFAULT_SHORT_BREAK_MINUTES: u32 = 5;
/// Default long break in minutes (after 4 pomodoros).
pub const DEFAULT_LONG_BREAK_MINUTES: u32 = 15;

/// A Pomodoro focus/productivity skill.
///
/// When active, the agent receives instructions to work in focused intervals,
/// take breaks, and track completed pomodoros.
pub struct PomodoroSkill {
    active: AtomicBool,
    work_minutes: u32,
    short_break_minutes: u32,
    long_break_minutes: u32,
}

impl PomodoroSkill {
    /// Create a new PomodoroSkill with default intervals.
    pub fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            work_minutes: DEFAULT_WORK_MINUTES,
            short_break_minutes: DEFAULT_SHORT_BREAK_MINUTES,
            long_break_minutes: DEFAULT_LONG_BREAK_MINUTES,
        }
    }

    /// Create a PomodoroSkill with custom intervals.
    pub fn with_intervals(work: u32, short_break: u32, long_break: u32) -> Result<Self, SkillError> {
        if work == 0 {
            return Err(SkillError::InvalidConfig(
                "work interval must be > 0".into(),
            ));
        }
        if short_break == 0 {
            return Err(SkillError::InvalidConfig(
                "short break must be > 0".into(),
            ));
        }
        if long_break == 0 {
            return Err(SkillError::InvalidConfig(
                "long break must be > 0".into(),
            ));
        }
        Ok(Self {
            active: AtomicBool::new(false),
            work_minutes: work,
            short_break_minutes: short_break,
            long_break_minutes: long_break,
        })
    }

    /// Check if the skill is currently active.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Work interval duration in minutes.
    pub fn work_minutes(&self) -> u32 {
        self.work_minutes
    }

    /// Short break duration in minutes.
    pub fn short_break_minutes(&self) -> u32 {
        self.short_break_minutes
    }

    /// Long break duration in minutes.
    pub fn long_break_minutes(&self) -> u32 {
        self.long_break_minutes
    }
}

impl Default for PomodoroSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for PomodoroSkill {
    fn name(&self) -> &str {
        "pomodoro"
    }

    fn description(&self) -> &str {
        "Pomodoro focus timer — work in focused intervals with short breaks"
    }

    fn instructions(&self) -> &str {
        "## Pomodoro Focus Mode\n\
         \n\
         You are operating in Pomodoro focus mode. Follow these rules:\n\
         \n\
         1. **Work in focused intervals.** Each work session should target a single, \
         well-defined objective. Do not context-switch mid-interval.\n\
         2. **Track progress.** At the start of each interval, state the objective. \
         At the end, summarize what was accomplished.\n\
         3. **Take breaks.** After each work interval, pause briefly. After 4 intervals, \
         take a longer break.\n\
         4. **Minimize distractions.** During a work interval, only respond to the \
         current task. Queue non-urgent items for the break.\n\
         5. **Budget awareness.** Each pomodoro has an implicit sat budget. If the task \
         will exceed budget, break it into smaller pomodoros.\n"
    }

    fn auto_activate(&self) -> bool {
        false
    }

    fn required_tools(&self) -> Vec<String> {
        vec!["memory_store".into()]
    }

    fn activate(&self) -> Result<(), SkillError> {
        self.active.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn deactivate(&self) -> Result<(), SkillError> {
        self.active.store(false, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pomodoro_defaults() {
        let skill = PomodoroSkill::new();
        assert_eq!(skill.name(), "pomodoro");
        assert!(!skill.auto_activate());
        assert!(!skill.is_active());
        assert_eq!(skill.work_minutes, DEFAULT_WORK_MINUTES);
        assert_eq!(skill.short_break_minutes, DEFAULT_SHORT_BREAK_MINUTES);
        assert_eq!(skill.long_break_minutes, DEFAULT_LONG_BREAK_MINUTES);
    }

    #[test]
    fn test_pomodoro_lifecycle() {
        let skill = PomodoroSkill::new();
        assert!(!skill.is_active());

        skill.activate().unwrap();
        assert!(skill.is_active());

        skill.deactivate().unwrap();
        assert!(!skill.is_active());
    }

    #[test]
    fn test_pomodoro_instructions_not_empty() {
        let skill = PomodoroSkill::new();
        let instructions = skill.instructions();
        assert!(!instructions.is_empty());
        assert!(instructions.contains("Pomodoro"));
        assert!(instructions.contains("focused intervals"));
    }

    #[test]
    fn test_pomodoro_required_tools() {
        let skill = PomodoroSkill::new();
        let tools = skill.required_tools();
        assert!(tools.contains(&"memory_store".to_string()));
    }

    #[test]
    fn test_custom_intervals() {
        let skill = PomodoroSkill::with_intervals(50, 10, 30).unwrap();
        assert_eq!(skill.work_minutes, 50);
        assert_eq!(skill.short_break_minutes, 10);
        assert_eq!(skill.long_break_minutes, 30);
    }

    #[test]
    fn test_invalid_intervals() {
        let result = PomodoroSkill::with_intervals(0, 5, 15);
        assert!(result.is_err());

        let result = PomodoroSkill::with_intervals(25, 0, 15);
        assert!(result.is_err());

        let result = PomodoroSkill::with_intervals(25, 5, 0);
        assert!(result.is_err());
    }
}
