//! x402 server error types.

#[derive(Debug, thiserror::Error)]
pub enum X402Error {
    /// Payment flow failure (402 handling, BEEF construction, refund).
    #[error("x402 payment error: {0}")]
    Payment(String),

    /// Network / HTTP request failure.
    #[error("x402 request error: {0}")]
    Request(String),

    /// Service discovery failure (manifest fetch, registry lookup).
    #[error("x402 discovery error: {0}")]
    Discovery(String),
}

impl X402Error {
    /// Convenience constructor for payment errors (mirrors WormError::payment).
    pub fn payment(msg: impl Into<String>) -> Self {
        Self::Payment(msg.into())
    }

    /// Convenience constructor for request errors.
    pub fn request(msg: impl Into<String>) -> Self {
        Self::Request(msg.into())
    }

    /// Convenience constructor for discovery errors.
    pub fn discovery(msg: impl Into<String>) -> Self {
        Self::Discovery(msg.into())
    }

    /// Get the error message string.
    pub fn message(&self) -> &str {
        match self {
            Self::Payment(msg) => msg,
            Self::Request(msg) => msg,
            Self::Discovery(msg) => msg,
        }
    }
}
