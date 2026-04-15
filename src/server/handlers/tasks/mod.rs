//! Task-related route handlers, split by domain.

mod audit;
mod events;
mod lifecycle;
mod proofs;

pub(crate) use audit::*;
pub(crate) use events::*;
pub(crate) use lifecycle::*;
pub use proofs::*;
