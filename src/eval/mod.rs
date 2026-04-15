//! Agent evaluation framework.
//!
//! Provides trajectory recording, grading rubrics, and report generation
//! for evaluating agent behavior from recorded JSONL session transcripts.
//!
//! Three submodules:
//!   - `trajectory`: Parse and replay JSONL transcripts into structured trajectories
//!   - `grader`: Evaluate trajectories against rubrics (task completion, efficiency, safety, cost)
//!   - `report`: Generate evaluation reports with regression detection

pub mod grader;
pub mod report;
pub mod trajectory;

pub use grader::{BuiltinRubric, GradingCriterion, Rubric, Score};
pub use report::{EvalReport, RegressionFlag};
pub use trajectory::{StepType, Trajectory, TrajectoryMetadata, TrajectoryStep};
