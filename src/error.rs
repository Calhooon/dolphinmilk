//! BSV-WORM error hierarchy.
//!
//! All worm errors use the DmError enum so callers can match
//! broadly or narrowly. Each variant carries structured context
//! for logging and budget tracking.

use std::collections::HashMap;

/// Structured context attached to errors for logging and debugging.
pub type ErrorContext = HashMap<String, serde_json::Value>;

#[derive(Debug, thiserror::Error)]
pub enum DmError {
    /// Wallet connection or operation failure.
    #[error("wallet error: {message}")]
    Wallet {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// x402 payment flow failure (402 handling, BEEF construction, refund).
    #[error("payment error: {message}")]
    Payment {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// Tool execution failure.
    #[error("tool error: {message}")]
    Tool {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// Budget limit exceeded or drain detected.
    #[error("budget error: {message}")]
    Budget {
        message: String,
        context: ErrorContext,
    },

    /// Configuration loading or validation failure.
    #[error("config error: {message}")]
    Config {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// Loop detection triggered (circuit breaker).
    #[error("loop error: {message}")]
    Loop {
        message: String,
        context: ErrorContext,
    },

    /// Memory system failure (store, index, search).
    #[error("memory error: {message}")]
    Memory {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// BRC-31 Authrite authentication failure.
    #[error("auth error: {message}")]
    Auth {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// BRC-33 MessageBox communication failure.
    #[error("messagebox error: {message}")]
    MessageBox {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        context: ErrorContext,
    },

    /// Conversation storage failure.
    #[error("conversation error: {message}")]
    Conversation {
        message: String,
        context: ErrorContext,
    },
}

impl DmError {
    pub fn wallet(msg: impl Into<String>) -> Self {
        Self::Wallet {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn wallet_with(msg: impl Into<String>, ctx: ErrorContext) -> Self {
        Self::Wallet {
            message: msg.into(),
            source: None,
            context: ctx,
        }
    }

    pub fn payment(msg: impl Into<String>) -> Self {
        Self::Payment {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn payment_with(msg: impl Into<String>, ctx: ErrorContext) -> Self {
        Self::Payment {
            message: msg.into(),
            source: None,
            context: ctx,
        }
    }

    pub fn tool(msg: impl Into<String>) -> Self {
        Self::Tool {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn budget(msg: impl Into<String>) -> Self {
        Self::Budget {
            message: msg.into(),
            context: ErrorContext::new(),
        }
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn loop_err(msg: impl Into<String>) -> Self {
        Self::Loop {
            message: msg.into(),
            context: ErrorContext::new(),
        }
    }

    pub fn memory(msg: impl Into<String>) -> Self {
        Self::Memory {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn memory_with_source(
        msg: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Memory {
            message: msg.into(),
            source: Some(Box::new(source)),
            context: ErrorContext::new(),
        }
    }

    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn messagebox(msg: impl Into<String>) -> Self {
        Self::MessageBox {
            message: msg.into(),
            source: None,
            context: ErrorContext::new(),
        }
    }

    pub fn conversation(msg: impl Into<String>) -> Self {
        Self::Conversation {
            message: msg.into(),
            context: ErrorContext::new(),
        }
    }

    /// Get the error context map.
    pub fn context(&self) -> &ErrorContext {
        match self {
            Self::Wallet { context, .. } => context,
            Self::Payment { context, .. } => context,
            Self::Tool { context, .. } => context,
            Self::Budget { context, .. } => context,
            Self::Config { context, .. } => context,
            Self::Loop { context, .. } => context,
            Self::Memory { context, .. } => context,
            Self::Auth { context, .. } => context,
            Self::MessageBox { context, .. } => context,
            Self::Conversation { context, .. } => context,
        }
    }
}

pub type DmResult<T> = Result<T, DmError>;
