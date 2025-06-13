//! Error types for the relay builder framework

use snafu::{Backtrace, Snafu};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Internal error: {message}"))]
    Internal {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("WebSocket error: {message}"))]
    WebSocket {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Database error: {message}"))]
    Database {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Protocol error: {message}"))]
    Protocol {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Auth required: {message}"))]
    AuthRequired {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Restricted: {message}"))]
    Restricted {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Notice: {message}"))]
    Notice {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Event error: {message}"))]
    EventError {
        message: String,
        event_id: nostr_sdk::EventId,
        backtrace: Backtrace,
    },

    #[snafu(display("Subscription error: {message}"))]
    SubscriptionError {
        message: String,
        subscription_id: String,
        backtrace: Backtrace,
    },
}

impl Error {
    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Create a database error
    pub fn database(message: impl Into<String>) -> Self {
        Self::Database {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Create a protocol error
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Create an auth required error
    pub fn auth_required(message: impl Into<String>) -> Self {
        Self::AuthRequired {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Create a restricted error
    pub fn restricted(message: impl Into<String>) -> Self {
        Self::Restricted {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Create a notice error
    pub fn notice(message: impl Into<String>) -> Self {
        Self::Notice {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Create an event error
    pub fn event_error(message: impl Into<String>, event_id: nostr_sdk::EventId) -> Self {
        Self::EventError {
            message: message.into(),
            event_id,
            backtrace: Backtrace::capture(),
        }
    }

    /// Create a subscription error
    pub fn subscription_error(
        message: impl Into<String>,
        subscription_id: impl Into<String>,
    ) -> Self {
        Self::SubscriptionError {
            message: message.into(),
            subscription_id: subscription_id.into(),
            backtrace: Backtrace::capture(),
        }
    }
}

// Conversion to anyhow is done by anyhow's blanket implementation
// since Error implements std::error::Error through snafu

pub type Result<T> = std::result::Result<T, Error>;
