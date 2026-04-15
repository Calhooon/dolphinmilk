//! Security module -- compound command analysis, classification, and blocking.
//!
//! The `command_analyzer` submodule decomposes shell commands into subcommands,
//! classifies each by risk, and detects dangerous combinations like data
//! exfiltration patterns.

pub mod command_analyzer;

pub use command_analyzer::{
    analyze_command, CommandAnalysis, CommandClassification, Connector, RiskLevel, SubCommand,
};
