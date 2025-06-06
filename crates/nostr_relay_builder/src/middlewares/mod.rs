//! Protocol and utility middlewares for Nostr relays

mod error_handling;
mod event_verifier;
mod generic_error_handling;
mod generic_event_verifier;
mod generic_logger;
mod logger;
mod nip09_deletion;
mod nip40_expiration;
mod nip42_auth;
mod nip70_protected;

pub use error_handling::{ClientMessageId, ErrorHandlingMiddleware};
pub use event_verifier::EventVerifierMiddleware;
pub use generic_error_handling::GenericErrorHandlingMiddleware;
pub use generic_event_verifier::GenericEventVerifierMiddleware;
pub use generic_logger::GenericLoggerMiddleware;
pub use logger::{LoggerMetricsHandler, LoggerMiddleware};
pub use nip09_deletion::Nip09Middleware;
pub use nip40_expiration::Nip40ExpirationMiddleware;
pub use nip42_auth::{AuthConfig, Nip42Middleware};
pub use nip70_protected::Nip70Middleware;
