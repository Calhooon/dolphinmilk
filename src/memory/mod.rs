//! Memory system — persistent knowledge store, BM25 search, session summaries,
//! encrypted storage, and UHRP sync.
//!
//! The memory module provides five capabilities:
//!   1. File-based memory store (markdown + YAML frontmatter)
//!   2. Tantivy BM25 full-text search over stored memories
//!   3. Session summarization from transcript events
//!   4. BRC-78-like AES-256-GCM encryption for memory files
//!   5. UHRP/NanoStore client for warm persistent storage

pub mod encrypt;
pub mod processing;
pub mod search;
pub mod session;
pub mod store;
pub mod sync;
pub mod uhrp;

// Re-export key types
pub use processing::{post_process, ProcessingResult};
pub use search::{MemoryIndex, MemorySnippet, TamperWarning};
pub use session::SessionSummarizer;
pub use store::{MemoryCategory, MemoryEntry, MemoryStore, IDENTITY_TAG};
pub use sync::MemorySync;
