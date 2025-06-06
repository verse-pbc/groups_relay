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
    pub fn send(&mut self, message: O) -> Result<(), TrySendError<(O, usize)>> {
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

    /// Sends a message through the channel, bypassing the current middleware.
    ///
    /// This method is useful when the current middleware has already processed
    /// the message (e.g., already applied filtering/validation) and you want to
    /// skip re-processing in the current middleware while still allowing
    /// other middleware in the chain to process the message.
    ///
    /// The outbound processing will start from the previous middleware in the chain
    /// (current index - 1), or index 0 if already at the first middleware.
    ///
    /// # When to use send_bypass() vs send():
    ///
    /// **Use `send_bypass()` when:**
    /// - You've already applied the current middleware's logic (e.g., filtering, validation)
    /// - You want to avoid duplicate processing in the current middleware
    /// - Example: Historical events that were already filtered during database query
    ///
    /// **Use `send()` when:**
    /// - You want the message to go through the normal middleware chain processing
    /// - The current middleware should also process this outbound message
    /// - Example: Broadcasting new events that need full filtering/validation
    ///
    /// # Arguments
    /// * `message` - The message to send
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err(TrySendError)` - Channel is full or closed
    pub fn send_bypass(&mut self, message: O) -> Result<(), TrySendError<(O, usize)>> {
        let bypass_index = if self.index > 0 { self.index - 1 } else { 0 };

        debug!(
            "MessageSender bypassing current middleware (index {}) and starting from index {}",
            self.index, bypass_index
        );

        if let Err(e) = self.sender.try_send((message, bypass_index)) {
            error!(
                "Failed to send bypass message. Current capacity: {}. Error: {}",
                self.capacity(),
                e
            );
            return Err(e);
        }

        debug!("MessageSender successfully sent bypass message");
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
    /// Queue `message` for the client via the bounded, non-blocking channel.
    ///
    /// • Non-blocking (`try_send`) → avoids dead-locks
    /// • Preserves outbound-middleware order (continues after current middleware)
    ///
    /// Returns `Err(TrySendError::Full | Closed)` as the back-pressure signal.
    fn send_message(&mut self, message: O) -> Result<()>;

    /// Returns the number of available slots in the channel.
    fn capacity(&self) -> usize;
}

/// Context for handling connection establishment.
#[derive(Debug)]
pub struct ConnectionContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    pub connection_id: String,
    pub state: &'a mut S,
    pub sender: Option<MessageSender<O>>,
    pub(crate) index: usize,
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    ConnectionContext<'a, S, M, O>
{
    pub fn new(
        connection_id: String,
        sender: Option<Sender<(O, usize)>>,
        state: &'a mut S,
        middlewares: &'a [Arc<
            dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>,
        >],
        index: usize,
    ) -> Self {
        Self {
            connection_id,
            sender: sender.map(|s| MessageSender::new(s, index)),
            state,
            middlewares,
            index,
        }
    }

    pub async fn next(&mut self) -> Result<()> {
        if self.index >= self.middlewares.len() - 1 {
            return Ok(());
        }
        self.index += 1;
        if let Some(sender) = &mut self.sender {
            sender.index = self.index;
        }
        let middleware = &self.middlewares[self.index];
        middleware.on_connect(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for ConnectionContext<'_, S, M, O>
{
    fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message)?;
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

/// Context for handling connection termination.
#[derive(Debug)]
pub struct DisconnectContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    pub connection_id: String,
    pub state: &'a mut S,
    pub sender: Option<MessageSender<O>>,
    pub(crate) index: usize,
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    DisconnectContext<'a, S, M, O>
{
    pub fn new(
        connection_id: String,
        sender: Option<Sender<(O, usize)>>,
        state: &'a mut S,
        middlewares: &'a [Arc<
            dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>,
        >],
        index: usize,
    ) -> Self {
        Self {
            connection_id,
            sender: sender.map(|s| MessageSender::new(s, index)),
            state,
            middlewares,
            index,
        }
    }

    pub async fn next(&mut self) -> Result<()> {
        if self.index >= self.middlewares.len() - 1 {
            return Ok(());
        }
        self.index += 1;
        if let Some(sender) = &mut self.sender {
            sender.index = self.index;
        }
        let middleware = &self.middlewares[self.index];
        middleware.on_disconnect(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for DisconnectContext<'_, S, M, O>
{
    fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message)?;
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

/// Context for handling incoming messages.
#[derive(Debug)]
pub struct InboundContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    pub connection_id: String,
    pub state: &'a mut S,
    pub sender: Option<MessageSender<O>>,
    pub(crate) index: usize,
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
    pub message: Option<M>,
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    InboundContext<'a, S, M, O>
{
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
            sender: sender.map(|s| MessageSender::new(s, index)),
            state,
            middlewares,
            index,
            message,
        }
    }

    pub async fn next(&mut self) -> Result<()> {
        if self.message.is_none() {
            debug!("Inbound message is empty, stopping middleware chain");
            return Ok(());
        }
        if self.index >= self.middlewares.len() - 1 {
            return Ok(());
        }
        self.index += 1;
        if let Some(sender) = &mut self.sender {
            sender.index = self.index;
        }
        let middleware = &self.middlewares[self.index];
        middleware.process_inbound(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for InboundContext<'_, S, M, O>
{
    fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message)?;
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

/// Context for handling outgoing messages.
#[derive(Debug)]
pub struct OutboundContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    pub connection_id: String,
    pub state: &'a mut S,
    pub sender: Option<MessageSender<O>>,
    pub(crate) index: usize,
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
    pub message: Option<O>,
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    OutboundContext<'a, S, M, O>
{
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
            sender: sender.map(|s| MessageSender::new(s, index)),
            state,
            middlewares,
            index,
            message: Some(message),
        }
    }

    pub async fn next(&mut self) -> Result<()> {
        if self.index == 0 {
            return Ok(());
        }
        self.index -= 1;
        if let Some(sender) = &mut self.sender {
            sender.index = self.index;
        }
        let middleware = &self.middlewares[self.index];
        middleware.process_outbound(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for OutboundContext<'_, S, M, O>
{
    fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message)?;
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}
