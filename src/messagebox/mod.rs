//! BRC-33 MessageBox client — peer-to-peer messaging for BSV agents.
//!
//! Provides send, list, acknowledge, quote, and permission operations
//! via the Babbage MessageBox relay server.

pub mod client;
pub mod types;

pub use client::compute_message_hash;
pub use client::MessageBoxClient;
pub use types::*;
