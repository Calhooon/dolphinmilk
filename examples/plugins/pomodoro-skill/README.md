# Pomodoro Skill

Example bsv-worm skill plugin demonstrating the `Skill` trait.

## What it does

When activated, injects Pomodoro productivity instructions into the agent's system prompt. The agent works in focused intervals, tracks progress, and takes breaks.

## Usage

```rust
use pomodoro_skill::PomodoroSkill;
use bsv_worm_sdk::skill::Skill;

let skill = PomodoroSkill::new();
skill.activate().unwrap();
assert!(skill.is_active());

// Instructions are injected into the system prompt
println!("{}", skill.instructions());
```

## Configuration

Default intervals: 25min work, 5min short break, 15min long break.

Custom intervals:
```rust
let skill = PomodoroSkill::with_intervals(50, 10, 30).unwrap();
```

## Testing

```bash
cargo test -p pomodoro-skill
```
