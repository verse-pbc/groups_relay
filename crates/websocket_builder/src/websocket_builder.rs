use crate::{MessageConverter, MessageHandler, Middleware};
use axum::extract::ws::{Message, WebSocket};
use axum::Error as AxumError;
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc::Receiver as MpscReceiver;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

/// Errors that can occur during WebSocket handling.
///
/// This enum represents all possible errors that can occur during the lifecycle
/// of a WebSocket connection, including connection establishment, message processing,
/// and connection termination.
///
/// Each variant includes the connection state at the time of the error, allowing
/// for proper cleanup and error recovery.
///
/// # Type Parameters
/// * `TapState` - The type of state maintained for each connection
#[derive(Error, Debug)]
pub enum WebsocketError<TapState: Send + Sync + 'static> {
    #[error("IO error: {0}")]
    IoError(std::io::Error, TapState),

    #[error("Invalid target URL: missing host")]
    InvalidTargetUrl(TapState),

    #[error("DNS resolution failed: {0}")]
    ResolveError(hickory_resolver::error::ResolveError, TapState),

    #[error("No addresses found for host: {0}")]
    NoAddressesFound(String, TapState),

    #[error("Task join error: {0}")]
    JoinError(tokio::task::JoinError, TapState),

    #[error("WebSocket error: {0}")]
    WebsocketError(AxumError, TapState),

    #[error("No closing handshake")]
    NoClosingHandshake(AxumError, TapState),

    #[error("Handler error: {0}")]
    HandlerError(Box<dyn std::error::Error + Send + Sync>, TapState),

    #[error("Missing middleware")]
    MissingMiddleware(TapState),

    #[error("Inbound message conversion error: {0}")]
    InboundMessageConversionError(String, TapState),

    #[error("Outbound message conversion error: {0}")]
    OutboundMessageConversionError(String, TapState),

    #[error("Maximum concurrent connections limit reached")]
    MaxConnectionsExceeded(TapState),

    #[error("Binary messages are not supported by this server")]
    UnsupportedBinaryMessage(TapState),
}

impl<TapState: Send + Sync + 'static> WebsocketError<TapState> {
    pub fn get_state(self) -> TapState {
        match self {
            Self::HandlerError(_, state) => state,
            Self::IoError(_, state) => state,
            Self::ResolveError(_, state) => state,
            Self::NoAddressesFound(_, state) => state,
            Self::JoinError(_, state) => state,
            Self::WebsocketError(_, state) => state,
            Self::NoClosingHandshake(_, state) => state,
            Self::MissingMiddleware(state) => state,
            Self::InvalidTargetUrl(state) => state,
            Self::MaxConnectionsExceeded(state) => state,
            Self::UnsupportedBinaryMessage(state) => state,
            Self::InboundMessageConversionError(_, state)
            | Self::OutboundMessageConversionError(_, state) => state,
        }
    }
}

/// A type alias for a vector of middleware instances.
///
/// This type represents the chain of middleware that processes messages.
/// Each middleware in the vector is wrapped in an Arc for thread-safe sharing.
///
/// # Type Parameters
/// * `S` - The type of state maintained for each connection
/// * `I` - The type of incoming messages
/// * `O` - The type of outgoing messages
pub type MiddlewareVec<S, I, O> =
    Vec<Arc<dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>>>;

/// A trait for creating per-connection state objects.
///
/// This trait is used to create new state instances for each WebSocket connection.
/// The state instance is passed through the middleware chain and can be used to
/// store connection-specific data.
///
/// # Type Parameters
/// * `State` - The type of state to create for each connection
pub trait StateFactory<State> {
    /// Creates a new state instance for each WebSocket connection.
    ///
    /// This method is called when a new WebSocket connection is established.
    /// The returned state instance will be passed through the middleware chain
    /// and can be used to store connection-specific data.
    ///
    /// # Arguments
    /// * `token` - A cancellation token that will be cancelled when the connection ends.
    ///   This token can be used to clean up resources when the connection is closed.
    fn create_state(&self, token: CancellationToken) -> State;
}

/// A builder for configuring and creating WebSocket handlers.
///
/// This builder provides a fluent interface for configuring WebSocket handlers
/// with middleware, connection limits, timeouts, and other settings.
///
/// # Type Parameters
/// * `TapState` - The type of state maintained for each connection
/// * `I` - The type of incoming messages after conversion
/// * `O` - The type of outgoing messages before conversion
/// * `Converter` - The type that handles message conversion
/// * `Factory` - The type that creates new state instances
///
/// # Example
/// ```
/// use websocket_builder::{WebSocketBuilder, StateFactory, MessageConverter, Middleware};
/// use tokio_util::sync::CancellationToken;
/// use anyhow::Result;
/// use async_trait::async_trait;
///
/// #[derive(Debug)]
/// struct MyState;
///
/// #[derive(Clone)]
/// struct MyStateFactory;
///
/// impl StateFactory<MyState> for MyStateFactory {
///     fn create_state(&self, _token: CancellationToken) -> MyState {
///         MyState
///     }
/// }
///
/// #[derive(Clone)]
/// struct JsonConverter;
///
/// impl MessageConverter<String, String> for JsonConverter {
///     fn inbound_from_string(&self, msg: String) -> Result<Option<String>> {
///         Ok(Some(msg))
///     }
///     fn outbound_to_string(&self, msg: String) -> Result<String> {
///         Ok(msg)
///     }
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
/// }
///
/// let handler = WebSocketBuilder::new(MyStateFactory, JsonConverter)
///     .with_middleware(LoggerMiddleware)
///     .with_channel_size(100)
///     .build();
/// ```
pub struct WebSocketBuilder<
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    Factory: StateFactory<TapState> + Send + Sync + Clone + 'static,
> {
    state_factory: Factory,
    middlewares:
        Vec<Arc<dyn Middleware<State = TapState, IncomingMessage = I, OutgoingMessage = O>>>,
    message_converter: Converter,
    channel_size: usize,
    max_connection_time: Option<Duration>,
    max_connections: Option<usize>,
}

impl<
        TapState: std::fmt::Debug + Send + Sync + 'static,
        I: Send + Sync + 'static,
        O: Send + Sync + 'static,
        Converter: MessageConverter<I, O> + Send + Sync + Clone + 'static,
        Factory: StateFactory<TapState> + Send + Sync + Clone + 'static,
    > WebSocketBuilder<TapState, I, O, Converter, Factory>
{
    /// Creates a new WebSocket builder with the given state factory and message converter.
    ///
    /// # Arguments
    /// * `state_factory` - Factory for creating per-connection state
    /// * `message_converter` - Converter for transforming between wire format and application types
    ///
    /// # Returns
    /// A new builder instance with default settings:
    /// * No middleware
    /// * Channel size of 100 messages
    /// * No connection time limit
    /// * No connection count limit
    pub fn new(state_factory: Factory, message_converter: Converter) -> Self {
        Self {
            state_factory,
            middlewares: Vec::new(),
            message_converter,
            channel_size: 100, // Default size
            max_connection_time: None,
            max_connections: None,
        }
    }

    /// Adds a middleware to the processing chain.
    ///
    /// Middleware are executed in the order they are added for inbound messages,
    /// and in reverse order for outbound messages.
    ///
    /// # Arguments
    /// * `middleware` - The middleware instance to add
    ///
    /// # Returns
    /// The builder instance for method chaining
    #[must_use]
    pub fn with_middleware<
        M: Middleware<State = TapState, IncomingMessage = I, OutgoingMessage = O> + 'static,
    >(
        mut self,
        middleware: M,
    ) -> Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    /// Adds a middleware that's already wrapped in an Arc to the processing chain.
    ///
    /// This method is useful when you need to add middlewares that are already
    /// type-erased as trait objects.
    ///
    /// # Arguments
    /// * `middleware` - The middleware instance wrapped in Arc
    ///
    /// # Returns
    /// The builder instance for method chaining
    #[must_use]
    pub fn with_arc_middleware(
        mut self,
        middleware: Arc<dyn Middleware<State = TapState, IncomingMessage = I, OutgoingMessage = O>>,
    ) -> Self {
        self.middlewares.push(middleware);
        self
    }

    /// Sets the maximum duration for a connection.
    ///
    /// After this duration, the connection will be gracefully closed.
    /// This can be used to implement connection rotation or to prevent
    /// resource leaks from long-lived connections.
    ///
    /// # Arguments
    /// * `duration` - The maximum duration for a connection
    ///
    /// # Returns
    /// The builder instance for method chaining
    #[must_use]
    pub fn with_max_connection_time(mut self, duration: Duration) -> Self {
        self.max_connection_time = Some(duration);
        self
    }

    /// Sets the maximum number of concurrent connections.
    ///
    /// When this limit is reached, new connection attempts will be
    /// rejected with a `MaxConnectionsExceeded` error.
    ///
    /// # Arguments
    /// * `max` - The maximum number of concurrent connections
    ///
    /// # Returns
    /// The builder instance for method chaining
    #[must_use]
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Sets the size of the channel used for message passing.
    ///
    /// This controls the buffer size for outbound messages. When the
    /// buffer is full, backpressure will be applied to senders.
    ///
    /// # Arguments
    /// * `size` - The size of the channel buffer
    ///
    /// # Returns
    /// The builder instance for method chaining
    #[must_use]
    pub const fn with_channel_size(mut self, size: usize) -> Self {
        self.channel_size = size;
        self
    }

    /// Builds the WebSocket handler with the configured settings.
    ///
    /// # Returns
    /// A new WebSocket handler instance ready to process connections
    pub fn build(self) -> WebSocketHandler<TapState, I, O, Converter, Factory> {
        WebSocketHandler {
            middlewares: Arc::new(self.middlewares),
            message_converter: Arc::new(self.message_converter),
            state_factory: self.state_factory,
            channel_size: self.channel_size,
            max_connection_time: self.max_connection_time,
            connection_semaphore: self
                .max_connections
                .map(|cap| Arc::new(Semaphore::new(cap))),
        }
    }
}

/// A handler for WebSocket connections with middleware support.
///
/// This handler processes incoming and outgoing messages through a chain of middleware,
/// maintains per-connection state, and handles connection lifecycle events.
///
/// # Type Parameters
/// * `S` - The type of state maintained for each connection
/// * `I` - The type of incoming messages after conversion
/// * `O` - The type of outgoing messages before conversion
/// * `C` - The type that handles message conversion
/// * `F` - The type that creates new state instances
///
/// # Features
/// * Bidirectional middleware pipeline for message processing
/// * Per-connection state management
/// * Automatic connection cleanup
/// * Connection limits and timeouts
/// * Backpressure handling via channel size limits
#[derive(Clone)]
pub struct WebSocketHandler<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    F: StateFactory<S> + Send + Sync + Clone + 'static,
{
    pub(crate) middlewares: Arc<MiddlewareVec<S, I, O>>,
    pub(crate) message_converter: Arc<C>,
    pub(crate) state_factory: F,
    pub(crate) channel_size: usize,
    pub(crate) max_connection_time: Option<Duration>,
    pub(crate) connection_semaphore: Option<Arc<Semaphore>>,
}

impl<TapState, I, O, Converter, Factory> WebSocketHandler<TapState, I, O, Converter, Factory>
where
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    Factory: StateFactory<TapState> + Send + Sync + Clone + 'static,
{
    /// Starts handling a WebSocket connection.
    ///
    /// This method processes the lifecycle of a WebSocket connection, including:
    /// * Connection setup and state initialization
    /// * Message processing through the middleware chain
    /// * Connection cleanup and resource release
    ///
    /// The connection will be processed until one of the following occurs:
    /// * The client closes the connection
    /// * The cancellation token is triggered
    /// * The maximum connection time is reached (if configured)
    /// * An error occurs during processing
    ///
    /// # Returns
    /// * `Ok(())` if the connection was processed successfully
    /// * `Err(WebsocketError)` if an error occurred during processing
    ///
    /// # Errors
    /// Returns a `WebsocketError` if:
    /// * The WebSocket connection fails
    /// * Message conversion fails
    /// * Middleware processing fails
    /// * The handler encounters an IO error
    /// * The maximum connections limit is reached
    pub async fn start(
        &self,
        socket: WebSocket,
        connection_id: String,
        cancellation_token: CancellationToken,
    ) -> Result<(), WebsocketError<TapState>> {
        // Create a child cancellation token to isolate connection termination from
        // server-wide shutdowns, ensuring only this specific connection is affected
        let connection_token = cancellation_token.child_token();

        let state = self.state_factory.create_state(connection_token.clone());

        let (state, _connection_permit) = self.try_acquire_connection_permit(state).await?;

        self.spawn_timeout_task(connection_token.clone());

        let mut session_handler = MessageHandler::new(
            self.middlewares.clone(),
            self.message_converter.clone(),
            None,
            connection_token.clone(),
            self.channel_size,
        );

        let state = match handle_connection_lifecycle(
            connection_id.clone(),
            socket,
            &mut session_handler,
            connection_token,
            state,
        )
        .await
        {
            Ok(final_state) => final_state,
            Err(e) => e.get_state(),
        };

        if let Err(e) = session_handler
            .on_disconnect(connection_id.clone(), state)
            .await
        {
            error!("Error during connection disconnect handler: {}", e);
        }

        Ok(())
    }

    /// Handles connection permit acquisition
    async fn try_acquire_connection_permit(
        &self,
        state: TapState,
    ) -> Result<(TapState, Option<OwnedSemaphorePermit>), WebsocketError<TapState>> {
        if let Some(semaphore) = &self.connection_semaphore {
            match semaphore.clone().try_acquire_owned() {
                Ok(permit) => Ok((state, Some(permit))),
                Err(_) => {
                    warn!("Maximum connections limit reached, rejecting connection");
                    Err(WebsocketError::MaxConnectionsExceeded(state))
                }
            }
        } else {
            Ok((state, None))
        }
    }

    /// Spawns timeout task if max_connection_time is configured
    fn spawn_timeout_task(&self, connection_token: CancellationToken) {
        if let Some(max_time) = self.max_connection_time {
            let token = connection_token.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(max_time) => {
                        if !token.is_cancelled() {
                            warn!(
                                "Max connection time ({:?}) exceeded, initiating graceful shutdown",
                                max_time
                            );
                            token.cancel();
                        }
                    }
                    _ = token.cancelled() => {} // Connection already cancelled.
                }
            });
        }
    }
}

/// Handles the lifecycle of a WebSocket connection.
///
/// This function manages the main processing loop for a connection, including:
/// * Message reception and sending
/// * State management
/// * Error handling
/// * Connection cleanup
///
/// # Arguments
/// * `connection_id` - Unique identifier for the connection
/// * `socket` - The WebSocket connection
/// * `session_handler` - Handler for processing messages
/// * `cancellation_token` - Token for cancelling the connection
/// * `state` - Initial connection state
///
/// # Returns
/// * `Ok(TapState)` - The final state if the connection closed normally
/// * `Err(WebsocketError)` - If an error occurred during processing
async fn handle_connection_lifecycle<
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
>(
    connection_id: String,
    socket: WebSocket,
    session_handler: &mut MessageHandler<TapState, I, O, Converter>,
    cancellation_token: CancellationToken,
    state: TapState,
) -> Result<TapState, WebsocketError<TapState>> {
    let (state, server_receiver) = match session_handler
        .on_connect(connection_id.clone(), state)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!("WebSocket connection setup failed: {}", e);
            return Err(e);
        }
    };

    let state = match message_loop(
        &connection_id,
        socket,
        server_receiver,
        session_handler,
        cancellation_token,
        state,
    )
    .await
    {
        Ok(state) => state,
        Err(e) => match e {
            WebsocketError::NoClosingHandshake(_e, state) => {
                return Ok(state);
            }
            _ => {
                warn!("WebSocket message loop error: {}", e);
                return Err(e);
            }
        },
    };

    Ok(state)
}

/// Processes messages for a WebSocket connection.
///
/// This function implements the main message processing loop, handling:
/// * Incoming messages from the client
/// * Outgoing messages from the server
/// * Connection cancellation
/// * Graceful shutdown
///
/// The loop continues until one of the following occurs:
/// * The client closes the connection
/// * The cancellation token is triggered
/// * An error occurs
///
/// # Arguments
/// * `connection_id` - Identifier for logging and tracking
/// * `socket` - The WebSocket connection
/// * `server_receiver` - Channel for receiving outbound messages
/// * `handler` - Handler for processing messages
/// * `cancellation_token` - Token for cancelling the connection
/// * `state` - Current connection state
///
/// # Returns
/// * `Ok(TapState)` - The final state if the connection closed normally
/// * `Err(WebsocketError)` - If an error occurred during processing
async fn message_loop<
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
>(
    connection_id: &str,
    mut socket: WebSocket,
    mut server_receiver: MpscReceiver<(O, usize)>,
    handler: &mut MessageHandler<TapState, I, O, Converter>,
    cancellation_token: CancellationToken,
    mut state: TapState,
) -> Result<TapState, WebsocketError<TapState>> {
    loop {
        tokio::select! {
            biased;

            _ = cancellation_token.cancelled() => {
                // Flush any pending messages in the channel
                while let Ok(msg) = server_receiver.try_recv() {
                    let (message, middleware_index) = msg;
                    state = handle_outgoing_message(
                        connection_id,
                        &mut socket,
                        message,
                        middleware_index,
                        handler,
                        state,
                        true,
                    )
                    .await?;
                }

                // Send a close frame
                if let Err(e) = socket.send(Message::Close(None)).await {
                    warn!("Failed to send WebSocket close frame to client: {}", e);
                }

                return Ok(state);
            }

            server_message = server_receiver.recv() => {
                match server_message {
                    Some((message, middleware_index)) => {
                        state = handle_outgoing_message(
                            connection_id,
                            &mut socket,
                            message,
                            middleware_index,
                            handler,
                            state,
                            false,
                        )
                        .await?;
                    }
                    None => {
                        return Ok(state);
                    }
                }
            }

            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        state = handler
                            .handle_incoming_message(connection_id.to_string(), text, state)
                            .await?;
                    }
                    Some(Ok(Message::Binary(_))) => {
                        error!("Protocol violation: received binary message - terminating connection as binary messages are not supported");
                        return Err(WebsocketError::UnsupportedBinaryMessage(state));
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(e) = socket.send(Message::Pong(payload)).await {
                            warn!("Failed to send pong: {}", e);
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                    }
                    Some(Ok(Message::Close(_))) => {
                        // Send close frame in response if we haven't already
                        if let Err(_e) = socket.send(Message::Close(None)).await {
                        }
                        return Ok(state);
                    }
                    Some(Err(e)) => {
                        if e.to_string().contains("without closing handshake") {
                            return Err(WebsocketError::NoClosingHandshake(e, state));
                        }
                        error!("WebSocket error: {}", e);
                        return Err(WebsocketError::WebsocketError(e, state));
                    }
                    None => {
                        return Ok(state);
                    }
                }
            }
        }
    }
}

/// Processes an outgoing message through the middleware chain and sends it.
///
/// This helper function consolidates the logic for handling outbound messages,
/// including middleware processing, conversion, and sending over the socket.
async fn handle_outgoing_message<TapState, I, O, Converter>(
    connection_id: &str,
    socket: &mut WebSocket,
    message: O,
    middleware_index: usize,
    handler: &mut MessageHandler<TapState, I, O, Converter>,
    state: TapState,
    is_flush: bool,
) -> Result<TapState, WebsocketError<TapState>>
where
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
{
    let _log_prefix = if is_flush { "Flushing" } else { "Processing" };
    let (new_state, message) = match handler
        .handle_outbound_message(connection_id.to_string(), message, middleware_index, state)
        .await
    {
        Ok((new_state, message)) => (new_state, message),
        Err(e) => {
            error!(
                "Error handling outbound message{}: {}",
                if is_flush { " during flush" } else { "" },
                e
            );
            return Err(e);
        }
    };

    if let Some(message) = message {
        if let Err(e) = socket.send(Message::Text(message)).await {
            error!(
                "Failed to send{} message to websocket: {}",
                if is_flush { " final" } else { "" },
                e
            );
            return Err(WebsocketError::WebsocketError(e, new_state));
        }
    }

    Ok(new_state)
}
