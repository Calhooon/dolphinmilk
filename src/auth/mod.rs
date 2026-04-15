//! BRC-31 Authrite — mutual authentication for BSV services.
//!
//! Client-side: authenticated HTTP requests for MessageBox, NanoStore,
//! and any BRC-31 service.
//!
//! Server-side: handshake handling, session store, and request verification
//! for the parent console and future public API.

pub mod client;
pub mod serialization;
pub mod server;
pub mod session;

pub use client::AuthriteClient;
pub use server::{
    Brc31AuthContext, Brc31AuthParams, Brc31ServerSession, Brc31SessionStore, HandshakeRequest,
    HandshakeResponse,
};
pub use session::AuthSession;
