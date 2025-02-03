use crate::Middleware;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct MessageSender<O> {
    pub sender: Sender<(O, usize)>,
    pub index: usize,
}

impl<O> MessageSender<O> {
    pub fn new(sender: Sender<(O, usize)>, index: usize) -> Self {
        Self { sender, index }
    }

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

    pub fn capacity(&self) -> usize {
        self.sender.capacity()
    }
}

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

impl<'a, S: Send + Sync + 'static, I: Send + Sync + 'static, O: Send + Sync + 'static>
    ConnectionContext<'a, S, I, O>
{
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
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

#[async_trait]
pub trait SendMessage<O> {
    async fn send_message(&mut self, message: O) -> Result<()>;

    /// Returns the number of available slots in the channel
    fn capacity(&self) -> usize;
}

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

impl<'a, S: Send + Sync + 'static, I: Send + Sync + 'static, O: Send + Sync + 'static>
    DisconnectContext<'a, S, I, O>
{
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

#[derive(Debug)]
pub struct InboundContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    pub connection_id: String,
    pub message: M,
    pub state: &'a mut S,
    pub sender: Option<MessageSender<O>>,
    pub(crate) index: usize,
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
}

impl<'a, S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static>
    InboundContext<'a, S, M, O>
{
    pub fn new(
        connection_id: String,
        message: M,
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

    pub async fn next(&mut self) -> Result<()> {
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
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}

#[derive(Debug)]
pub struct OutboundContext<'a, S, M, O>
where
    S: Send + Sync + 'static,
    M: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    pub connection_id: String,
    pub message: Option<O>,
    pub state: &'a mut S,
    pub sender: Option<MessageSender<O>>,
    pub(crate) index: usize,
    pub(crate) middlewares:
        &'a [Arc<dyn Middleware<State = S, IncomingMessage = M, OutgoingMessage = O>>],
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
            message: Some(message),
            sender: sender.map(|sender| MessageSender::new(sender, index)),
            state,
            middlewares,
            index,
        }
    }

    pub async fn next(&mut self) -> Result<()> {
        if self.index == 0 {
            return Ok(());
        }

        self.index -= 1;
        if let Some(sender) = &mut self.sender {
            sender.index -= 1;
        }

        self.middlewares[self.index].process_outbound(self).await
    }
}

#[async_trait]
impl<S: Send + Sync + 'static, M: Send + Sync + 'static, O: Send + Sync + 'static> SendMessage<O>
    for OutboundContext<'_, S, M, O>
{
    async fn send_message(&mut self, message: O) -> Result<()> {
        if let Some(sender) = &mut self.sender {
            sender.send(message).await?;
        }
        Ok(())
    }

    /// Returns the number of available slots in the channel
    fn capacity(&self) -> usize {
        self.sender.as_ref().map_or(0, |s| s.capacity())
    }
}
