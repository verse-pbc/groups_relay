use crate::{ConnectionContext, DisconnectContext, InboundContext, OutboundContext};
use anyhow::Result;
use async_trait::async_trait;

/// A trait for implementing WebSocket connection middleware.
///
/// This trait allows you to hook into different stages of a WebSocket connection's lifecycle:
/// * Connection establishment
/// * Message processing (both inbound and outbound)
/// * Connection termination
///
/// Each middleware can:
/// * Modify or transform messages
/// * Maintain per-connection state
/// * Send messages back through the WebSocket
/// * Perform cleanup when connections end
///
/// # Type Parameters
/// * `State` - The type of state maintained for each connection
/// * `IncomingMessage` - The type of messages received from clients
/// * `OutgoingMessage` - The type of messages sent to clients
///
/// # Example
/// ```
/// use websocket_builder::{Middleware, InboundContext, OutboundContext};
/// use async_trait::async_trait;
/// use anyhow::Result;
///
/// #[derive(Debug)]
/// struct MyState {
///     counter: usize,
/// }
///
/// #[derive(Debug)]
/// struct LoggerMiddleware;
///
/// #[async_trait]
/// impl Middleware for LoggerMiddleware {
///     type State = MyState;
///     type IncomingMessage = String;
///     type OutgoingMessage = String;
///
///     async fn process_inbound(
///         &self,
///         ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
///     ) -> Result<()> {
///         if let Some(message) = &ctx.message {
///             println!("Received message: {}", message);
///         }
///         ctx.next().await
///     }
/// }
/// ```
#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    /// The type of state maintained for each connection.
    type State: Send + Sync + 'static;

    /// The type of messages received from clients.
    type IncomingMessage: Send + Sync + 'static;

    /// The type of messages sent to clients.
    type OutgoingMessage: Send + Sync + 'static;

    /// Processes an incoming message from a client.
    ///
    /// This method is called for each message received from a client. The middleware can:
    /// * Inspect or modify the message
    /// * Update the connection state
    /// * Send messages back to the client
    /// * Forward the message to the next middleware
    ///
    /// # Arguments
    /// * `ctx` - Context containing the message and connection state
    ///
    /// # Returns
    /// * `Ok(())` - Message processed successfully
    /// * `Err` - Processing failed
    ///
    /// The default implementation forwards the message to the next middleware.
    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    /// Processes an outgoing message to a client.
    ///
    /// This method is called for each message sent to a client. The middleware can:
    /// * Inspect or modify the message
    /// * Update the connection state
    /// * Send additional messages
    /// * Forward the message to the next middleware
    ///
    /// # Arguments
    /// * `ctx` - Context containing the message and connection state
    ///
    /// # Returns
    /// * `Ok(())` - Message processed successfully
    /// * `Err` - Processing failed
    ///
    /// The default implementation forwards the message to the next middleware.
    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    /// Called when a new WebSocket connection is established.
    ///
    /// This method allows middleware to:
    /// * Initialize per-connection state
    /// * Send initial messages to the client
    /// * Perform connection setup tasks
    ///
    /// # Arguments
    /// * `ctx` - Context containing the connection state
    ///
    /// # Returns
    /// * `Ok(())` - Connection setup successful
    /// * `Err` - Setup failed
    ///
    /// The default implementation forwards the connection event to the next middleware.
    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    /// Called when a WebSocket connection ends.
    ///
    /// This method is called exactly once when a connection ends, regardless of how it ended
    /// (graceful shutdown, error, or client disconnect). It allows middleware to:
    /// * Perform cleanup tasks
    /// * Save final connection state
    /// * Send final messages
    /// * Log connection statistics
    ///
    /// The connection state passed to this method reflects the entire connection lifecycle,
    /// including:
    /// * Normal message processing
    /// * Token-based graceful shutdown
    /// * Client-initiated close
    /// * Error conditions
    /// * Termination without closing handshake
    ///
    /// # Arguments
    /// * `ctx` - Context containing the final connection state
    ///
    /// # Returns
    /// * `Ok(())` - Cleanup successful
    /// * `Err` - Cleanup failed
    ///
    /// Any error returned from this method will be wrapped as a HandlerError, but the
    /// connection state is always preserved and forwarded back. This ensures that
    /// downstream components receive the final state for consistent cleanup.
    ///
    /// The default implementation forwards the disconnect event to the next middleware.
    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }
}
