//! Dolphin Milk library crate — re-exports for testing and embedding.

// --- Core: agent loop, inference, errors ---
pub mod config;
pub mod error;
pub mod replay;
pub mod runner;
pub mod think;
pub mod types;

// --- Auth & payments ---
pub mod auth;
pub mod wallet;
pub mod x402;

// --- On-chain state (grouped directory) ---
pub mod onchain;
pub use onchain::budget;
pub use onchain::proofs;
pub use onchain::state;

// --- Session lifecycle (grouped directory) ---
pub mod session;
pub use session::conversation;
pub use session::events;
pub use session::message;
pub use session::transcript;

// --- Analytics ---
pub mod analytics;

// --- Memory & intelligence ---
pub mod context;
pub mod loop_detect;
pub mod memory;
pub mod skills;
pub mod templates;

// --- Communication ---
pub mod certificates;
pub mod delegation;
pub mod delivery;
pub mod discovery;
pub mod messagebox;
pub mod moderation;
pub mod sanitize;

// --- Hooks ---
pub mod hooks;

// --- Security ---
pub mod security;

// --- Analytics ---
pub mod time_estimate;

// --- Audit ---
pub mod audit;

// --- Evaluation ---
pub mod eval;

// --- Orchestration ---
pub mod orchestration;

// --- Overlay ---
pub mod overlay;

// --- Infrastructure: server, CLI, logging ---
pub mod banner;
pub mod cli;
pub mod heartbeat;
pub mod logging;
pub mod mcp;
pub mod metrics;
pub mod server;
pub mod tools;
