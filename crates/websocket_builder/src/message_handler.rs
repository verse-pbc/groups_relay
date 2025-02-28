use crate::{
    ConnectionContext, DisconnectContext, InboundContext, MiddlewareVec, OutboundContext,
    WebsocketError,
};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver as MpscReceiver, Sender as MpscSender};
use tokio_util::sync::CancellationToken;
use tracing::error;

/// A trait for converting between wire format (string) messages and application types.
///
/// This trait handles the conversion of messages between the wire format (strings)
/// used by the WebSocket protocol and the application-specific types used by
/// the middleware chain.
///
/// # Type Parameters
/// * `I` - The type of incoming messages after conversion
/// * `O` - The type of outgoing messages before conversion
pub trait MessageConverter<I, O>: Send + Sync {
    /// Converts an incoming string message to the application type.
    ///
    /// # Arguments
    /// * `message` - The string message received from the WebSocket
    ///
    /// # Returns
    /// * `Ok(Some(I))` - Successfully converted message
    /// * `Ok(None)` - Message should be ignored
    /// * `Err` - Conversion failed
    fn inbound_from_string(&self, message: String) -> Result<Option<I>, anyhow::Error>;

    /// Converts an outgoing application message to a string.
    ///
    /// # Arguments
    /// * `message` - The application message to convert
    ///
    /// # Returns
    /// * `Ok(String)` - Successfully converted message
    /// * `Err` - Conversion failed
    fn outbound_to_string(&self, message: O) -> Result<String, anyhow::Error>;
}

/// Handles message processing through a middleware chain.
///
/// This struct manages the lifecycle of a WebSocket connection, including:
/// * Connection establishment and cleanup
/// * Message conversion between wire format and application types
/// * Message routing through the middleware chain
/// * State management
///
/// # Type Parameters
/// * `TapState` - The type of state maintained for each connection
/// * `I` - The type of incoming messages after conversion
/// * `O` - The type of outgoing messages before conversion
/// * `Converter` - The type that handles message conversion
pub struct MessageHandler<
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
> {
    middlewares: Arc<MiddlewareVec<TapState, I, O>>,
    message_converter: Arc<Converter>,
    sender: Option<MpscSender<(O, usize)>>,
    cancellation_token: CancellationToken,
    channel_size: usize,
}

impl<
        TapState: Send + Sync + 'static,
        I: Send + Sync + 'static,
        O: Send + Sync + 'static,
        Converter: MessageConverter<I, O> + Send + Sync + 'static,
    > MessageHandler<TapState, I, O, Converter>
{
    /// Creates a new message handler.
    ///
    /// # Arguments
    /// * `middlewares` - The chain of middleware to process messages
    /// * `message_converter` - Converter for message formats
    /// * `sender` - Channel for sending outbound messages
    /// * `cancellation_token` - Token for cancelling the handler
    /// * `channel_size` - Size of the message channel buffer
    pub fn new(
        middlewares: Arc<MiddlewareVec<TapState, I, O>>,
        message_converter: Arc<Converter>,
        sender: Option<MpscSender<(O, usize)>>,
        cancellation_token: CancellationToken,
        channel_size: usize,
    ) -> Self {
        Self {
            middlewares,
            message_converter,
            sender,
            cancellation_token,
            channel_size,
        }
    }

    /// Processes an incoming message through the middleware chain.
    ///
    /// This method:
    /// 1. Converts the wire format message to the application type
    /// 2. Creates a context for the message
    /// 3. Passes the message through each middleware in sequence
    ///
    /// # Arguments
    /// * `connection_id` - Identifier for logging and tracking
    /// * `payload` - The raw message received from the WebSocket
    /// * `state` - Current connection state
    ///
    /// # Returns
    /// * `Ok(TapState)` - Updated state after processing
    /// * `Err(WebsocketError)` - If an error occurred during processing
    pub async fn handle_incoming_message(
        &self,
        connection_id: String,
        payload: String,
        mut state: TapState,
    ) -> Result<TapState, WebsocketError<TapState>> {
        let Ok(inbound_message) = self.message_converter.inbound_from_string(payload) else {
            return Err(WebsocketError::InboundMessageConversionError(
                "Failed to convert inbound message".to_string(),
                state,
            ));
        };

        if inbound_message.is_none() {
            return Ok(state);
        };

        let mut ctx = InboundContext::new(
            connection_id.clone(),
            inbound_message,
            self.sender.clone(),
            &mut state,
            &self.middlewares,
            0,
        );

        // Process through first middleware only - it will handle chain progression
        if let Err(e) = self.middlewares[0].process_inbound(&mut ctx).await {
            error!("Error in first middleware: {:?}", e);
            return Err(WebsocketError::HandlerError(e.into(), state));
        }

        Ok(state)
    }

    /// Processes an outbound message through the middleware chain.
    ///
    /// This method:
    /// 1. Creates a context for the message
    /// 2. Passes the message through remaining middleware
    /// 3. Converts the processed message to wire format
    ///
    /// # Arguments
    /// * `connection_id` - Identifier for logging and tracking
    /// * `message` - The message to process
    /// * `middleware_index` - Index of the middleware that sent the message
    /// * `state` - Current connection state
    ///
    /// # Returns
    /// * `Ok((TapState, Option<String>))` - Updated state and converted message
    /// * `Err(WebsocketError)` - If an error occurred during processing
    pub async fn handle_outbound_message(
        &self,
        connection_id: String,
        message: O,
        middleware_index: usize,
        mut state: TapState,
    ) -> Result<(TapState, Option<String>), WebsocketError<TapState>> {
        let message = if middleware_index > 0 {
            let mut ctx = OutboundContext::new(
                connection_id.clone(),
                message,
                self.sender.clone(),
                &mut state,
                &self.middlewares,
                middleware_index,
            );

            // Start with the current middleware and work backward
            if let Err(e) = self.middlewares[middleware_index]
                .process_outbound(&mut ctx)
                .await
            {
                error!(
                    "Error processing outbound message in middleware {}: {:?}",
                    middleware_index, e
                );
                return Err(WebsocketError::HandlerError(e.into(), state));
            };

            ctx.message
        } else {
            Some(message)
        };

        let Some(message) = message else {
            return Ok((state, None));
        };

        let Ok(string_message) = self.message_converter.outbound_to_string(message) else {
            error!("Failed to convert outbound message to string");
            return Err(WebsocketError::OutboundMessageConversionError(
                "Failed to convert outbound message to string".to_string(),
                state,
            ));
        };
        Ok((state, Some(string_message)))
    }

    /// Handles connection establishment.
    ///
    /// This method:
    /// 1. Creates a message channel for outbound messages
    /// 2. Notifies middleware of the new connection
    /// 3. Sets up the initial connection state
    ///
    /// # Arguments
    /// * `connection_id` - Identifier for logging and tracking
    /// * `state` - Initial connection state
    ///
    /// # Returns
    /// * `Ok((TapState, MpscReceiver))` - Updated state and message receiver
    /// * `Err(WebsocketError)` - If an error occurred during setup
    pub async fn on_connect(
        &mut self,
        connection_id: String,
        mut state: TapState,
    ) -> Result<(TapState, MpscReceiver<(O, usize)>), WebsocketError<TapState>> {
        let (sender, receiver) = tokio::sync::mpsc::channel(self.channel_size);
        self.sender = Some(sender);

        let mut ctx = ConnectionContext::new(
            connection_id,
            self.sender.clone(),
            &mut state,
            &self.middlewares,
            0,
        );

        if let Err(e) = self.middlewares[0].on_connect(&mut ctx).await {
            return Err(WebsocketError::HandlerError(e.into(), state));
        };

        Ok((state, receiver))
    }

    /// Handles connection termination.
    ///
    /// This method:
    /// 1. Notifies middleware of the disconnection
    /// 2. Cancels the connection token
    /// 3. Cleans up connection resources
    ///
    /// # Arguments
    /// * `connection_id` - Identifier for logging and tracking
    /// * `state` - Final connection state
    ///
    /// # Returns
    /// * `Ok(TapState)` - Final state after cleanup
    /// * `Err(WebsocketError)` - If an error occurred during cleanup
    pub async fn on_disconnect(
        &self,
        connection_id: String,
        mut state: TapState,
    ) -> Result<TapState, WebsocketError<TapState>> {
        let mut ctx = DisconnectContext::new(
            connection_id,
            self.sender.clone(),
            &mut state,
            &self.middlewares,
            0,
        );

        if let Err(e) = self.middlewares[0].on_disconnect(&mut ctx).await {
            return Err(WebsocketError::HandlerError(e.into(), state));
        };

        self.cancellation_token.cancel();
        Ok(state)
    }
}
