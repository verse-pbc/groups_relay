//! Actor-based WebSocket handler that eliminates head-of-line blocking.
//!
//! # Architecture
//!
//! Each WebSocket connection is managed by three independent actors:
//!
//! - **Reader Actor**: Reads messages from the WebSocket and forwards them to the State Actor
//! - **Writer Actor**: Receives messages from the queue and writes them to the WebSocket
//! - **State Actor**: Owns the connection state and processes all middleware sequentially
//!
//! This separation allows the reader and writer to operate concurrently while maintaining
//! sequential state mutations. The State Actor ensures that all state changes happen in a
//! predictable order, eliminating the need for locks or synchronization primitives.
//!
//! ## Benefits
//!
//! - **No head-of-line blocking**: Outbound messages don't wait for inbound processing
//! - **Concurrent I/O**: Read and write operations happen simultaneously
//! - **Owned state**: No Arc<Mutex<State>> required, state is exclusively owned by one actor
//! - **Sequential consistency**: All state mutations happen in order
//!

use crate::{
    ConnectionContext, DisconnectContext, InboundContext, MessageConverter,
    Middleware, OutboundContext, StateFactory,
};
use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

// Type aliases to simplify complex types
type StateQueryFn<S> = Box<dyn FnOnce(&S) -> Box<dyn std::any::Any + Send> + Send>;
type StateMutateFn<S> = Box<dyn FnOnce(&mut S) + Send>;

/// Commands that can be sent to the state actor
pub enum StateCommand<S, I, O> {
    /// Process an inbound message through middleware
    ProcessInbound {
        connection_id: String,
        message: I,
        response: oneshot::Sender<Result<Vec<O>>>,
    },
    /// Process an outbound message through middleware
    ProcessOutbound {
        connection_id: String,
        message: O,
        middleware_index: usize,
        response: oneshot::Sender<Result<Option<O>>>,
    },
    /// Handle connection event
    OnConnect {
        connection_id: String,
        response: oneshot::Sender<Result<Vec<O>>>,
    },
    /// Handle disconnection event
    OnDisconnect {
        connection_id: String,
        response: oneshot::Sender<Result<Vec<O>>>,
    },
    /// Apply a mutation to the state
    Mutate {
        mutation: StateMutateFn<S>,
    },
    /// Query the state
    Query {
        query: StateQueryFn<S>,
        response: oneshot::Sender<Box<dyn std::any::Any + Send>>,
    },
    /// Shutdown the actor
    Shutdown,
}

// Type alias for middleware collection
type MiddlewareCollection<S, I, O> = Arc<Vec<Arc<dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>>>>;

/// The state actor that owns the connection state
struct StateActor<S, I, O> {
    state: S,
    middlewares: MiddlewareCollection<S, I, O>,
    receiver: mpsc::UnboundedReceiver<StateCommand<S, I, O>>,
    outbound_sender: mpsc::Sender<(O, usize)>,
    _connection_id: String,
}

impl<S, I, O> StateActor<S, I, O>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    fn spawn(
        initial_state: S,
        middlewares: MiddlewareCollection<S, I, O>,
        outbound_sender: mpsc::Sender<(O, usize)>,
        connection_id: String,
    ) -> mpsc::UnboundedSender<StateCommand<S, I, O>> {
        let (sender, receiver) = mpsc::unbounded_channel();
        
        tokio::spawn(async move {
            let mut actor = StateActor {
                state: initial_state,
                middlewares,
                receiver,
                outbound_sender,
                _connection_id: connection_id,
            };
            actor.run().await;
        });
        
        sender
    }
    
    async fn run(&mut self) {
        while let Some(command) = self.receiver.recv().await {
            match command {
                StateCommand::ProcessInbound { connection_id, message, response } => {
                    let result = self.process_inbound(connection_id, message).await;
                    let _ = response.send(result);
                }
                StateCommand::ProcessOutbound { connection_id, message, middleware_index, response } => {
                    let result = self.process_outbound(connection_id, message, middleware_index).await;
                    let _ = response.send(result);
                }
                StateCommand::OnConnect { connection_id, response } => {
                    let result = self.on_connect(connection_id).await;
                    let _ = response.send(result);
                }
                StateCommand::OnDisconnect { connection_id, response } => {
                    let result = self.on_disconnect(connection_id).await;
                    let _ = response.send(result);
                }
                StateCommand::Mutate { mutation } => {
                    mutation(&mut self.state);
                }
                StateCommand::Query { query, response } => {
                    let result = query(&self.state);
                    let _ = response.send(result);
                }
                StateCommand::Shutdown => {
                    debug!("State actor shutting down");
                    break;
                }
            }
        }
    }
    
    async fn process_inbound(&mut self, connection_id: String, message: I) -> Result<Vec<O>> {
        // Create the context for middleware processing
        // Using the main outbound sender directly
        let mut ctx = InboundContext::new(
            connection_id,
            Some(message),
            Some(self.outbound_sender.clone()),
            &mut self.state,
            &self.middlewares,
            0,
        );
        
        // Process through the first middleware (which will chain to others)
        if !self.middlewares.is_empty() {
            if let Err(e) = self.middlewares[0].process_inbound(&mut ctx).await {
                error!("Error in inbound middleware processing: {}", e);
                return Err(e);
            }
        }
        
        // Since messages are sent directly to the outbound channel,
        // we don't need to collect them here
        Ok(Vec::new())
    }
    
    async fn process_outbound(&mut self, connection_id: String, message: O, middleware_index: usize) -> Result<Option<O>> {
        // Create the context starting at the specified middleware index
        let mut ctx = OutboundContext::new(
            connection_id,
            message,
            Some(self.outbound_sender.clone()),
            &mut self.state,
            &self.middlewares,
            middleware_index,
        );
        
        // Process through middleware in reverse order
        if middleware_index < self.middlewares.len() {
            if let Err(e) = self.middlewares[middleware_index].process_outbound(&mut ctx).await {
                error!("Error in outbound middleware processing: {}", e);
                return Err(e);
            }
        }
        
        // Return the potentially modified message
        Ok(ctx.message)
    }
    
    async fn on_connect(&mut self, connection_id: String) -> Result<Vec<O>> {
        // Create the context
        let mut ctx = ConnectionContext::new(
            connection_id,
            Some(self.outbound_sender.clone()),
            &mut self.state,
            &self.middlewares,
            0,
        );
        
        // Process through middleware
        if !self.middlewares.is_empty() {
            if let Err(e) = self.middlewares[0].on_connect(&mut ctx).await {
                error!("Error in on_connect middleware processing: {}", e);
                return Err(e);
            }
        }
        
        // Messages are sent directly to outbound channel
        Ok(Vec::new())
    }
    
    async fn on_disconnect(&mut self, connection_id: String) -> Result<Vec<O>> {
        // Create the context
        let mut ctx = DisconnectContext::new(
            connection_id,
            Some(self.outbound_sender.clone()),
            &mut self.state,
            &self.middlewares,
            0,
        );
        
        // Process through middleware
        if !self.middlewares.is_empty() {
            if let Err(e) = self.middlewares[0].on_disconnect(&mut ctx).await {
                error!("Error in on_disconnect middleware processing: {}", e);
                return Err(e);
            }
        }
        
        // Messages are sent directly to outbound channel
        Ok(Vec::new())
    }
}

/// Actor-based WebSocket handler that separates read/write/state concerns
pub struct ActorWebSocketHandler<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    F: StateFactory<S> + Send + Sync + Clone + 'static,
{
    middlewares: MiddlewareCollection<S, I, O>,
    message_converter: Arc<C>,
    state_factory: F,
    channel_size: usize,
}

impl<S, I, O, C, F> Clone for ActorWebSocketHandler<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    F: StateFactory<S> + Send + Sync + Clone + 'static,
{
    fn clone(&self) -> Self {
        Self {
            middlewares: self.middlewares.clone(),
            message_converter: self.message_converter.clone(),
            state_factory: self.state_factory.clone(),
            channel_size: self.channel_size,
        }
    }
}

impl<S, I, O, C, F> ActorWebSocketHandler<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    F: StateFactory<S> + Send + Sync + Clone + 'static,
{
    pub fn new(
        middlewares: Vec<Arc<dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>>>,
        message_converter: C,
        state_factory: F,
        channel_size: usize,
    ) -> Self {
        Self {
            middlewares: Arc::new(middlewares),
            message_converter: Arc::new(message_converter),
            state_factory,
            channel_size,
        }
    }
    
    pub async fn start(
        &self,
        socket: WebSocket,
        connection_id: String,
        cancellation_token: CancellationToken,
    ) -> Result<()> {
        let connection_token = cancellation_token.child_token();
        
        // Create initial state
        let initial_state = self.state_factory.create_state(connection_token.clone());
        
        // Create channels for read/write coordination
        let (outbound_tx, outbound_rx) = mpsc::channel::<(O, usize)>(self.channel_size);
        
        // Spawn state actor with connection ID
        let state_sender = StateActor::spawn(
            initial_state, 
            self.middlewares.clone(), 
            outbound_tx.clone(),
            connection_id.clone(),
        );
        
        // Process on_connect event
        let (connect_tx, connect_rx) = oneshot::channel();
        if state_sender.send(StateCommand::OnConnect {
            connection_id: connection_id.clone(),
            response: connect_tx,
        }).is_err() {
            return Err(anyhow::anyhow!("Failed to send on_connect command"));
        }
        
        // Wait for on_connect response
        match connect_rx.await {
            Ok(Ok(_)) => {
                // Messages are sent directly by middleware
            }
            Ok(Err(e)) => {
                error!("Error during on_connect: {}", e);
                return Err(e);
            }
            Err(_) => {
                return Err(anyhow::anyhow!("State actor closed during on_connect"));
            }
        }
        
        // Split the WebSocket
        let (ws_sink, ws_stream) = socket.split();
        
        // Spawn reader task
        let reader_handle = self.spawn_reader(
            connection_id.clone(),
            ws_stream,
            state_sender.clone(),
            outbound_tx.clone(),
            connection_token.clone(),
        );
        
        // Spawn writer task
        let writer_handle = self.spawn_writer(
            connection_id.clone(),
            ws_sink,
            outbound_rx,
            state_sender.clone(),
            connection_token.clone(),
        );
        
        // Wait for tasks to complete
        tokio::select! {
            _ = reader_handle => {
                debug!("Reader task completed");
            }
            _ = writer_handle => {
                debug!("Writer task completed");
            }
            _ = connection_token.cancelled() => {
                debug!("Connection cancelled");
            }
        }
        
        // Process on_disconnect event
        let (disconnect_tx, disconnect_rx) = oneshot::channel();
        let _ = state_sender.send(StateCommand::OnDisconnect {
            connection_id: connection_id.clone(),
            response: disconnect_tx,
        });
        
        // Wait for disconnect processing (ignore errors as connection is closing)
        let _ = disconnect_rx.await;
        
        // Shutdown state actor
        let _ = state_sender.send(StateCommand::Shutdown);
        
        Ok(())
    }
    
    fn spawn_reader(
        &self,
        connection_id: String,
        mut ws_stream: futures_util::stream::SplitStream<WebSocket>,
        state_sender: mpsc::UnboundedSender<StateCommand<S, I, O>>,
        _outbound_tx: mpsc::Sender<(O, usize)>,
        cancellation_token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let message_converter = self.message_converter.clone();
        
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        break;
                    }
                    
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                // Convert message
                                match message_converter.inbound_from_string(text) {
                                    Ok(Some(msg)) => {
                                        // Send to state actor for processing
                                        let (response_tx, response_rx) = oneshot::channel();
                                        let command = StateCommand::ProcessInbound {
                                            connection_id: connection_id.clone(),
                                            message: msg,
                                            response: response_tx,
                                        };
                                        
                                        if state_sender.send(command).is_err() {
                                            break;
                                        }
                                        
                                        // Wait for processing (messages are sent directly to outbound channel)
                                        match response_rx.await {
                                            Ok(Ok(_messages)) => {
                                                // Messages already sent to outbound channel by state actor
                                            }
                                            Ok(Err(e)) => {
                                                error!("Error processing inbound message: {}", e);
                                            }
                                            Err(_) => {
                                                error!("State actor response channel closed");
                                                break;
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        // Message filtered out
                                    }
                                    Err(e) => {
                                        error!("Failed to convert inbound message: {}", e);
                                    }
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                debug!("WebSocket closed by client");
                                break;
                            }
                            Some(Err(e)) => {
                                error!("WebSocket error: {}", e);
                                break;
                            }
                            None => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            
            debug!("Reader task for {} exiting", connection_id);
        })
    }
    
    fn spawn_writer(
        &self,
        connection_id: String,
        mut ws_sink: futures_util::stream::SplitSink<WebSocket, Message>,
        mut outbound_rx: mpsc::Receiver<(O, usize)>,
        state_sender: mpsc::UnboundedSender<StateCommand<S, I, O>>,
        cancellation_token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let message_converter = self.message_converter.clone();
        
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        // Send close frame
                        let _ = ws_sink.send(Message::Close(None)).await;
                        break;
                    }
                    
                    message = outbound_rx.recv() => {
                        match message {
                            Some((msg, middleware_index)) => {
                                // Process through outbound middleware
                                let (response_tx, response_rx) = oneshot::channel();
                                let command = StateCommand::ProcessOutbound {
                                    connection_id: connection_id.clone(),
                                    message: msg,
                                    middleware_index,
                                    response: response_tx,
                                };
                                
                                if state_sender.send(command).is_err() {
                                    break;
                                }
                                
                                // Wait for processed message
                                match response_rx.await {
                                    Ok(Ok(Some(processed_msg))) => {
                                        // Convert to string
                                        match message_converter.outbound_to_string(processed_msg) {
                                            Ok(text) => {
                                                if let Err(e) = ws_sink.send(Message::Text(text)).await {
                                                    error!("Failed to send message: {}", e);
                                                    break;
                                                }
                                            }
                                            Err(e) => {
                                                error!("Failed to convert outbound message: {}", e);
                                            }
                                        }
                                    }
                                    Ok(Ok(None)) => {
                                        // Message filtered out by middleware
                                    }
                                    Ok(Err(e)) => {
                                        error!("Error processing outbound message: {}", e);
                                    }
                                    Err(_) => {
                                        error!("State actor response channel closed");
                                        break;
                                    }
                                }
                            }
                            None => {
                                // Channel closed
                                break;
                            }
                        }
                    }
                }
            }
            
            debug!("Writer task for {} exiting", connection_id);
        })
    }
}

/// Builder for the actor-based WebSocket handler
pub struct ActorWebSocketBuilder<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    F: StateFactory<S> + Send + Sync + Clone + 'static,
{
    state_factory: F,
    middlewares: Vec<Arc<dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>>>,
    message_converter: C,
    channel_size: usize,
}

impl<S, I, O, C, F> ActorWebSocketBuilder<S, I, O, C, F>
where
    S: Send + Sync + 'static,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    C: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    F: StateFactory<S> + Send + Sync + Clone + 'static,
{
    pub fn new(state_factory: F, message_converter: C) -> Self {
        Self {
            state_factory,
            middlewares: Vec::new(),
            message_converter,
            channel_size: 100,
        }
    }
    
    pub fn with_middleware<M>(mut self, middleware: M) -> Self
    where
        M: Middleware<State = S, IncomingMessage = I, OutgoingMessage = O> + 'static,
    {
        self.middlewares.push(Arc::new(middleware));
        self
    }
    
    pub fn with_channel_size(mut self, size: usize) -> Self {
        self.channel_size = size;
        self
    }
    
    pub fn build(self) -> ActorWebSocketHandler<S, I, O, C, F> {
        ActorWebSocketHandler::new(
            self.middlewares,
            self.message_converter,
            self.state_factory,
            self.channel_size,
        )
    }
}