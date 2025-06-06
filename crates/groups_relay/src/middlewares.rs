// Groups-specific middlewares only
// Generic protocol middlewares have been moved to nostr_relay_builder
mod validation_middleware;

pub use validation_middleware::ValidationMiddleware;
