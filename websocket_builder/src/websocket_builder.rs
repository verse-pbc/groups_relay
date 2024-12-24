use crate::{MessageConverter, MessageHandler, Middleware};
use axum::extract::ws::{Message, WebSocket};
use axum::Error as AxumError;
use futures_util::StreamExt;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::Receiver as MpscReceiver;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

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
}

impl<TapState: Send + Sync + 'static> WebsocketError<TapState> {
    pub fn state(self) -> TapState {
        match self {
            WebsocketError::HandlerError(_, state) => state,
            WebsocketError::IoError(_, state) => state,
            WebsocketError::ResolveError(_, state) => state,
            WebsocketError::NoAddressesFound(_, state) => state,
            WebsocketError::JoinError(_, state) => state,
            WebsocketError::WebsocketError(_, state) => state,
            WebsocketError::NoClosingHandshake(_, state) => state,
            WebsocketError::MissingMiddleware(state) => state,
            WebsocketError::InvalidTargetUrl(state) => state,
            WebsocketError::InboundMessageConversionError(_, state) => state,
            WebsocketError::OutboundMessageConversionError(_, state) => state,
        }
    }
}

/// Factory trait for creating per-connection state objects
pub trait StateFactory<State> {
    /// Creates a new state instance for each WebSocket connection
    ///
    /// # Arguments
    /// * `token` - A cancellation token that will be cancelled when the connection ends
    fn create_state(&self, token: CancellationToken) -> State;
}

pub struct WebSocketBuilder<
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
    Factory: StateFactory<TapState>,
> {
    state_factory: Factory,
    middlewares:
        Vec<Arc<dyn Middleware<State = TapState, IncomingMessage = I, OutgoingMessage = O>>>,
    message_converter: Converter,
    channel_size: usize,
}

impl<
        TapState: std::fmt::Debug + Send + Sync + 'static,
        I: Send + Sync + 'static,
        O: Send + Sync + 'static,
        Converter: MessageConverter<I, O> + Send + Sync + 'static,
        Factory: StateFactory<TapState> + Send + Sync + 'static,
    > WebSocketBuilder<TapState, I, O, Converter, Factory>
{
    pub fn new(state_factory: Factory, message_converter: Converter) -> Self {
        Self {
            state_factory,
            middlewares: Vec::new(),
            message_converter,
            channel_size: 100, // Default size
        }
    }

    /// The passed middleware will be used to wrap the existing middleware.
    pub fn with_middleware<
        M: Middleware<State = TapState, IncomingMessage = I, OutgoingMessage = O> + 'static,
    >(
        mut self,
        middleware: M,
    ) -> Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    pub fn with_channel_size(mut self, size: usize) -> Self {
        self.channel_size = size;
        self
    }

    pub fn build(self) -> WebSocketHandler<TapState, I, O, Converter, Factory> {
        WebSocketHandler {
            middlewares: Arc::new(self.middlewares),
            message_converter: Arc::new(self.message_converter),
            state_factory: self.state_factory,
            channel_size: self.channel_size,
        }
    }
}

pub type MiddlewareVec<S, I, O> =
    Vec<Arc<dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>>>;

pub struct WebSocketHandler<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + 'static,
    F: StateFactory<S> + Send + Sync + 'static,
{
    middlewares: Arc<MiddlewareVec<S, I, O>>,
    message_converter: Arc<C>,
    state_factory: F,
    channel_size: usize,
}

impl<TapState, I, O, Converter, Factory> WebSocketHandler<TapState, I, O, Converter, Factory>
where
    TapState: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
    Factory: StateFactory<TapState> + Send + Sync + 'static,
{
    pub async fn start(
        &self,
        socket: WebSocket,
        connection_id: String,
        cancellation_token: CancellationToken,
    ) -> Result<(), WebsocketError<TapState>> {
        let connection_token = cancellation_token.child_token();
        let state = self.state_factory.create_state(connection_token.clone());
        let middlewares = self.middlewares.clone();
        let message_converter = self.message_converter.clone();
        info!("[{}] New connection", connection_id);

        let mut session_handler = MessageHandler::new(
            middlewares,
            message_converter,
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
            Ok(state) => state,
            Err(e) => e.state(),
        };

        if let Err(e) = session_handler
            .on_disconnect(connection_id.clone(), state)
            .await
        {
            error!("Disconnect error: {}", e);
        }

        info!("[{}] Connection closed", connection_id);
        Ok(())
    }
}

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
    let (state, server_receiver) = session_handler
        .on_connect(connection_id.clone(), state)
        .await?;

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
            WebsocketError::NoClosingHandshake(e, state) => {
                debug!("Client closed without closing handshake: {}", e);
                return Ok(state);
            }
            _ => {
                error!("Client error: {}", e);
                return Err(e);
            }
        },
    };

    Ok(state)
}

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
    debug!("[{}] Starting message loop", connection_id);

    // Helper function to handle a single message
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
        let log_prefix = if is_flush { "Flushing" } else { "Processing" };
        debug!(
            "[{}] {} outbound message from middleware {}",
            connection_id, log_prefix, middleware_index
        );

        let (new_state, message) = match handler
            .handle_outbound_message(connection_id.to_string(), message, middleware_index, state)
            .await
        {
            Ok((new_state, message)) => (new_state, message),
            Err(e) => {
                error!(
                    "[{}] Error handling outbound message{}: {}",
                    connection_id,
                    if is_flush { " during flush" } else { "" },
                    e
                );
                return Err(e);
            }
        };

        if let Some(message) = message {
            debug!(
                "[{}] Sending{} message to websocket",
                connection_id,
                if is_flush { " final" } else { "" }
            );
            if let Err(e) = socket.send(Message::Text(message)).await {
                error!(
                    "[{}] Failed to send{} message to websocket: {}",
                    connection_id,
                    if is_flush { " final" } else { "" },
                    e
                );
                return Err(WebsocketError::WebsocketError(e, new_state));
            }
            debug!(
                "[{}] Successfully sent{} message to websocket",
                connection_id,
                if is_flush { " final" } else { "" }
            );
        } else {
            debug!("[{}] No message to send", connection_id);
        }

        Ok(new_state)
    }

    loop {
        debug!("[{}] Message loop iteration starting", connection_id);
        tokio::select! {
            biased;

            _ = cancellation_token.cancelled() => {
                debug!("[{}] Cancellation token triggered, flushing pending messages", connection_id);

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
                    warn!("[{}] Failed to send close frame: {}", connection_id, e);
                }

                debug!("[{}] Finished flushing messages", connection_id);
                return Ok(state);
            }

            server_message = server_receiver.recv() => {
                debug!("[{}] Server receiver got message", connection_id);
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
                        debug!("[{}] Receiver closed", connection_id);
                        return Ok(state);
                    }
                }
            }

            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        state = handler
                            .handle_incoming_message(connection_id.to_string(), text, state)
                            .await?
                    }
                    Some(Ok(Message::Binary(_))) => {
                        todo!("handle binary message")
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(e) = socket.send(Message::Pong(payload)).await {
                            warn!("Pong failed: {}", e);
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {

                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Client closed");
                        return Ok(state);
                    }
                    Some(Err(e)) => {
                        if e.to_string().contains("without closing handshake") {
                            return Err(WebsocketError::NoClosingHandshake(e, state));
                        }
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
