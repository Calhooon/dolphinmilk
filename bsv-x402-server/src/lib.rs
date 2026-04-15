//! bsv-x402-server — x402 payment verification server.
//!
//! BRC-29 payment flow, service discovery, circuit breaker, rate limiting,
//! and response caching for paid API endpoints.
//!
//! This crate is protocol-agnostic: it defines traits (`WalletApi`, `AuthClient`)
//! for wallet and auth operations, allowing any BRC-100 wallet implementation
//! to plug in.

pub mod cache;
pub mod circuit_breaker;
pub mod discovery;
pub mod error;
pub mod payment;
pub mod rate_limit;
pub mod refund;
pub mod registry;
pub mod schema;
pub mod traits;
