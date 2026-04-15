//! Newtype ID wrappers for compile-time safety.
//!
//! Plain `String` IDs are interchangeable at the type level, making it easy to
//! accidentally pass a task ID where a session ID is expected.  These newtypes
//! provide zero-cost wrappers that the compiler can distinguish.
//!
//! All types serialize/deserialize as plain JSON strings (`#[serde(transparent)]`)
//! so they are fully backward-compatible with existing wire formats.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Macro to generate newtype ID wrappers with standard traits.
macro_rules! newtype_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Create a new ID from anything that converts to `String`.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Create from an owned `String` (avoids extra allocation).
            pub fn from_string(s: String) -> Self {
                Self(s)
            }

            /// Borrow the inner string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the wrapper and return the inner `String`.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }
    };
}

newtype_id!(TaskId, "Unique identifier for an agent task.");
newtype_id!(SessionId, "Unique identifier for an agent session.");
newtype_id!(
    ConversationId,
    "Unique identifier for a multi-turn conversation."
);
newtype_id!(ProofTxid, "Transaction ID for an on-chain proof.");
