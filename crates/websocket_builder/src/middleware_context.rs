use crate::Middleware;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error};

/// A wrapper for sending messages through the middleware chain.
///
/// This type provides a way to send messages back through the WebSocket connection
/// while maintaining the middleware chain's order and state.
///
/// # Type Parameters
/// * `O` - The type of outgoing messages
#[derive(Debug, Clone)]
pub struct MessageSender<O> {
    /// The channel sender for outgoing messages
    pub sender: Sender<(O, usize)>,
    /// The index of the middleware that sent the message
    pub index: usize,
}

impl<O> MessageSender<O> {
    /// Creates a new message sender.
    ///
    /// # Arguments
    /// * `sender` - The channel sender for outgoing messages
    /// * `index` - The index of the middleware that will use this sender
    pub fn new(sender: Sender<(O, usize)>, index: usize) -> Self {
        Self { sender, index }
    }

    /// Sends a message through the channel.
    ///
    /// This method attempts to send a message without blocking. If the channel
    /// is full, it will return an error immediately.
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err(TrySendError)` - Channel is full or closed
    pub async fn send(&mut self, message: O) -> Result<(), TrySendError<(O, usize)>> {
        debug!(
            "MessageSender sending message from middleware index: {}",
            self.index
        );

        if let Err(e) = self.sender.try_send((message, self.index)) {
            error!(
                "Failed to send message. Current capacity: {}. Error: {}",
                self.capacity(),
                e
            );
            return Err(e);
        }

        debug!("MessageSender successfully sent message");
        Ok(())
    }

    /// Returns the number of available slots in the channel.
    pub fn capacity(&self) -> usize {
        self.sender.capacity()
    }
}

/// A trait for sending messages through the middleware chain.
///
/// This trait provides a common interface for sending messages back through
/// the WebSocket connection, regardless of the context type.
///
/// # Type Parameters
/// * `O` - The type of outgoing messages
#[async_trait]
pub trait SendMessage<O> {
    /// Sends a message through the channel.
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err` - Channel is full or closed
    async fn send_message(&mut self, message: O) -> Result<()>;

    /// Returns the number of available slots in the channel.
    fn capacity(&self) -> usize;
}

/// Context for handling connection establishment.
///
/// This context is passed to middleware when a new WebSocket connection
/// is established. It provides access to:
/// * Connection state
/// * Message sending capabilities
/// * Connection identifier
///
/// # Type Parameters
/// * `S` - The type of state maintained for each connection
/// * `M` - The type of incoming messages
/// * `O` - The type of outgoing messages
#[derive(Debug)]
pub struct ConnectionContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    /// Unique identifier for the connection
    pub connection_id: String,
    /// Mutable reference to the connection state
    pub state: &'a mut S,
    /// Optional sender for outgoing messages
    pub sender: Option<MessageSender<O>>,
    /// Current position in the middleware chain
    pub(crate) index: usize,
    /// Reference to the middleware chain
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, I: Send + Sync + 'static, O: Send + Sync + 'static>
    ConnectionContext<'a, S, I, O>
{
    /// Creates a new connection context.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `sender` - Optional channel for sending outgoing messages
    /// * `state` - Mutable reference to the connection state
    /// * `middlewares` - Reference to the middleware chain
    /// * `index` - Current position in the middleware chain
    pub fn new(
        connection_id: String,
        sender: Option<Sender<(O, usize)>>,
        state: &'a mut S,
        middlewares: &'a [Arc<
            dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>,
        >],
        index: usize,
    ) -> Self {
        Self {
            connection_id,
            sender: sender.map(|sender| MessageSender::new(sender, index)),
            state,
            middlewares,
            index,
        }
    }

    /// Advances to the next middleware in the chain.
    ///
    /// This method:
    /// 1. Increments the middleware index
    /// 2. Updates the message sender's index
    /// 3. Calls the next middleware's `on_connect` method
    ///
    /// # Returns
    /// * `Ok(())` - Successfully processed by next middleware
    /// * `Err` - Processing failed
    pub async fn next(&mut self) -> Result<()> {
        if self.index >= self.middlewares.len() - 1 {
            return Ok(());
        }

        self.index += 1;
        if let Some(sender) = &mut self.sender {
            sender.index += 1;
        }

        let middleware = &self.middlewares[self.index];
        middleware.on_connect(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, I: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for ConnectionContext<'_, S, I, O>
{
    /// Sends a message through the channel.
    ///
    /// This method sends a message back through the WebSocket connection
    /// during connection establishment. This can be used to send initial
    /// messages like welcome messages or configuration data.
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err` - Channel is full or closed
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    /// Returns the number of available slots in the channel.
    ///
    /// This can be used to implement backpressure by checking if there's
    /// room in the channel before sending messages.
    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

/// Context for handling connection termination.
///
/// This context is passed to middleware when a WebSocket connection
/// is terminated. It provides access to:
/// * Final connection state
/// * Message sending capabilities (for cleanup messages)
/// * Connection identifier
///
/// # Type Parameters
/// * `S` - The type of state maintained for each connection
/// * `M` - The type of incoming messages
/// * `O` - The type of outgoing messages
#[derive(Debug)]
pub struct DisconnectContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    /// Unique identifier for the connection
    pub connection_id: String,
    /// Mutable reference to the connection state
    pub state: &'a mut S,
    /// Optional sender for outgoing messages
    pub sender: Option<MessageSender<O>>,
    /// Current position in the middleware chain
    pub(crate) index: usize,
    /// Reference to the middleware chain
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, I: Send + Sync + 'static, O: Send + Sync + 'static>
    DisconnectContext<'a, S, I, O>
{
    /// Creates a new disconnect context.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `sender` - Optional channel for sending outgoing messages
    /// * `state` - Mutable reference to the connection state
    /// * `middlewares` - Reference to the middleware chain
    /// * `index` - Current position in the middleware chain
    pub fn new(
        connection_id: String,
        sender: Option<Sender<(O, usize)>>,
        state: &'a mut S,
        middlewares: &'a [Arc<
            dyn Middleware<State = S, IncomingMessage = I, OutgoingMessage = O>,
        >],
        index: usize,
    ) -> Self {
        Self {
            connection_id,
            sender: sender.map(|sender| MessageSender::new(sender, index)),
            state,
            middlewares,
            index,
        }
    }

    /// Advances to the next middleware in the chain.
    ///
    /// This method:
    /// 1. Increments the middleware index
    /// 2. Updates the message sender's index
    /// 3. Calls the next middleware's `on_disconnect` method
    ///
    /// # Returns
    /// * `Ok(())` - Successfully processed by next middleware
    /// * `Err` - Processing failed
    pub async fn next(&mut self) -> Result<()> {
        if self.index >= self.middlewares.len() - 1 {
            return Ok(());
        }

        self.index += 1;
        if let Some(sender) = &mut self.sender {
            sender.index += 1;
        }

        let middleware = &self.middlewares[self.index];
        middleware.on_disconnect(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, I: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for DisconnectContext<'_, S, I, O>
{
    /// Sends a message through the channel.
    ///
    /// This method sends a message back through the WebSocket connection
    /// during connection termination. This can be used to send final
    /// messages like goodbye messages or cleanup notifications.
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err` - Channel is full or closed
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    /// Returns the number of available slots in the channel.
    ///
    /// This can be used to implement backpressure by checking if there's
    /// room in the channel before sending messages.
    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

/// Context for handling incoming messages.
///
/// This context is passed to middleware when a message is received
/// from a client. It provides access to:
/// * The received message
/// * Connection state
/// * Message sending capabilities
/// * Connection identifier
///
/// # Type Parameters
/// * `S` - The type of state maintained for each connection
/// * `M` - The type of incoming messages
/// * `O` - The type of outgoing messages
#[derive(Debug)]
pub struct InboundContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    /// Unique identifier for the connection
    pub connection_id: String,
    /// The received message, it's an option because you may want to own the
    /// message in the last inbound middleware
    pub message: Option<M>,
    /// Mutable reference to the connection state
    pub state: &'a mut S,
    /// Optional sender for outgoing messages
    pub sender: Option<MessageSender<O>>,
    /// Current position in the middleware chain
    pub(crate) index: usize,
    /// Reference to the middleware chain
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    InboundContext<'a, S, M, O>
{
    /// Creates a new inbound context.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `message` - The received message
    /// * `sender` - Optional channel for sending outgoing messages
    /// * `state` - Mutable reference to the connection state
    /// * `middlewares` - Reference to the middleware chain
    /// * `index` - Current position in the middleware chain
    pub fn new(
        connection_id: String,
        message: Option<M>,
        sender: Option<Sender<(O, usize)>>,
        state: &'a mut S,
        middlewares: &'a [Arc<
            dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>,
        >],
        index: usize,
    ) -> Self {
        Self {
            connection_id,
            message,
            sender: sender.map(|sender| MessageSender::new(sender, index)),
            state,
            middlewares,
            index,
        }
    }

    /// Advances to the next middleware in the chain.
    ///
    /// This method:
    /// 1. Increments the middleware index
    /// 2. Updates the message sender's index
    /// 3. Calls the next middleware's `process_inbound` method
    ///
    /// # Returns
    /// * `Ok(())` - Successfully processed by next middleware
    /// * `Err` - Processing failed
    pub async fn next(&mut self) -> Result<()> {
        if self.message.is_none() {
            // If the current middleware consumed the message we just stop
            // the middleware chain
            debug!("Inbound message is empty, stopping middleware chain");
            return Ok(());
        }

        if self.index >= self.middlewares.len() - 1 {
            return Ok(());
        }

        self.index += 1;
        if let Some(sender) = &mut self.sender {
            sender.index += 1;
        }

        let middleware = &self.middlewares[self.index];
        middleware.process_inbound(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for InboundContext<'_, S, M, O>
{
    /// Sends a message through the channel.
    ///
    /// This method sends a message back through the WebSocket connection
    /// in response to an incoming message. This can be used to send
    /// immediate responses, acknowledgments, or error messages.
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err` - Channel is full or closed
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    /// Returns the number of available slots in the channel.
    ///
    /// This can be used to implement backpressure by checking if there's
    /// room in the channel before sending messages.
    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

/// Context for handling outgoing messages.
///
/// This context is passed to middleware when a message is being sent
/// to a client. It provides access to:
/// * The message to send
/// * Connection state
/// * Message sending capabilities
/// * Connection identifier
///
/// # Type Parameters
/// * `S` - The type of state maintained for each connection
/// * `M` - The type of incoming messages
/// * `O` - The type of outgoing messages
#[derive(Debug)]
pub struct OutboundContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    /// Unique identifier for the connection
    pub connection_id: String,
    /// The message to send (can be modified by middleware)
    pub message: Option<O>,
    /// Mutable reference to the connection state
    pub state: &'a mut S,
    /// Optional sender for additional messages
    pub sender: Option<MessageSender<O>>,
    /// Current position in the middleware chain
    pub(crate) index: usize,
    /// Reference to the middleware chain
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    OutboundContext<'a, S, M, O>
{
    /// Creates a new outbound context.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `message` - The message to send
    /// * `sender` - Optional channel for sending additional messages
    /// * `state` - Mutable reference to the connection state
    /// * `middlewares` - Reference to the middleware chain
    /// * `index` - Current position in the middleware chain
    pub fn new(
        connection_id: String,
        message: O,
        sender: Option<Sender<(O, usize)>>,
        state: &'a mut S,
        middlewares: &'a [Arc<
            dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>,
        >],
        index: usize,
    ) -> Self {
        Self {
            connection_id,
            message: Some(message),
            sender: sender.map(|sender| MessageSender::new(sender, index)),
            state,
            middlewares,
            index,
        }
    }

    /// Advances to the next middleware in the chain.
    ///
    /// This method:
    /// 1. Decrements the middleware index
    /// 2. Updates the message sender's index
    /// 3. Calls the next middleware's `process_outbound` method
    ///
    /// # Returns
    /// * `Ok(())` - Successfully processed by next middleware
    /// * `Err` - Processing failed
    pub async fn next(&mut self) -> Result<()> {
        if self.index == 0 {
            return Ok(());
        }

        self.index -= 1;
        if let Some(sender) = &mut self.sender {
            sender.index -= 1;
        }

        let middleware = &self.middlewares[self.index];
        middleware.process_outbound(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for OutboundContext<'_, S, M, O>
{
    /// Sends a message through the channel.
    ///
    /// This method sends an additional message through the WebSocket connection
    /// while processing an outgoing message. This can be used to send related
    /// messages, metadata, or split large messages into smaller chunks.
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err` - Channel is full or closed
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    /// Returns the number of available slots in the channel.
    ///
    /// This can be used to implement backpressure by checking if there's
    /// room in the channel before sending messages.
    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}
