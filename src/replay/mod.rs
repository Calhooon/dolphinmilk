//! Replay infrastructure — interactive replay and fork execution from transcripts.
//!
//! Two main components:
//! - `ReplayViewer`: Reads a session JSONL transcript and yields events step-by-step
//!   with rich metadata (cost, tool name, iteration number, cumulative spending).
//! - `ForkExecutor`: Reconstructs conversation state from a transcript up to a given
//!   event index, producing the `prior_messages` needed to start a new agent run
//!   from that point.

pub mod fork;
pub mod viewer;

pub use fork::{ForkExecutor, ForkParams, ForkResult};
pub use viewer::{CostTimelinePoint, ReplayEvent, ReplayTimeline, ReplayViewer, ToolUsageEntry};
