use crate::{ConnectionContext, DisconnectContext, InboundContext, OutboundContext};
use anyhow::Result;
use async_trait::async_trait;

/// The Middleware trait allows you to hook into different stages of a WebSocket connection lifecycle.
/// It has default implementations for inbound and outbound processing as well as connection setup/teardown.
#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    type State: Send + Sync + 'static;
    type IncomingMessage: Send + Sync + 'static;
    type OutgoingMessage: Send + Sync + 'static;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    /// Called exactly once when a connection ends, regardless of how it ended
    /// (graceful shutdown, error, or client disconnect).
    ///
    /// This hook allows middleware to perform any final cleanup actions and to react to the final state of the connection.
    ///
    /// The connection state passed into this method has been built up over the entire lifecycle of the connection through:
    /// - Normal message processing,
    /// - Token-based graceful shutdown,
    /// - Client-initiated close,
    /// - Error conditions,
    /// - Or termination without a closing handshake.
    ///
    /// Any error returned from this method will be wrapped as a HandlerError, but the preserved connection state
    /// is always forwarded back. This guarantees that downstream components receive the final connection state for
    /// consistent cleanup or post-processing.
    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }
}
