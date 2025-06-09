//! Protocol and utility middlewares for Nostr relays

mod error_handling;
mod event_verifier;
mod logger;
mod metrics;
mod nip09_deletion;
mod nip40_expiration;
mod nip42_auth;
mod nip70_protected;

pub use error_handling::{ClientMessageId, ErrorHandlingMiddleware};
pub use event_verifier::EventVerifierMiddleware;
pub use logger::LoggerMiddleware;
pub use metrics::{MetricsHandler, MetricsMiddleware};
pub use nip09_deletion::Nip09Middleware;
pub use nip40_expiration::Nip40ExpirationMiddleware;
pub use nip42_auth::{AuthConfig, Nip42Middleware};
pub use nip70_protected::Nip70Middleware;
