use crate::{
    ConnectionContext, DisconnectContext, InboundContext, MiddlewareVec, OutboundContext,
    WebsocketError,
};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver as MpscReceiver, Sender as MpscSender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

pub trait MessageConverter<I, O>: Send + Sync {
    fn inbound_from_string(&self, message: String) -> Result<Option<I>, anyhow::Error>;
    fn outbound_to_string(&self, message: O) -> Result<String, anyhow::Error>;
}

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

        let Some(inbound_message) = inbound_message else {
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

        debug!(
            "[{}] Starting inbound message processing through middleware chain",
            connection_id
        );

        // Process through first middleware
        if let Err(e) = self.middlewares[0].process_inbound(&mut ctx).await {
            error!("[{}] Error in first middleware: {:?}", connection_id, e);
            return Err(WebsocketError::HandlerError(e.into(), state));
        }

        // Continue processing through the rest of the chain
        if let Err(e) = ctx.next().await {
            error!("[{}] Error in middleware chain: {:?}", connection_id, e);
            return Err(WebsocketError::HandlerError(e.into(), state));
        }

        debug!(
            "[{}] Completed inbound message processing through middleware chain",
            connection_id
        );

        Ok(state)
    }

    pub async fn handle_outbound_message(
        &self,
        connection_id: String,
        message: O,
        middleware_index: usize,
        mut state: TapState,
    ) -> Result<(TapState, Option<String>), WebsocketError<TapState>> {
        debug!(
            "[{}] Starting outbound message processing from middleware {}",
            connection_id, middleware_index
        );

        let message = if middleware_index > 0 {
            let mut ctx = OutboundContext::new(
                connection_id.clone(),
                message,
                self.sender.clone(),
                &mut state,
                &self.middlewares,
                middleware_index,
            );

            debug!(
                "[{}] Processing through remaining middlewares starting at {}",
                connection_id, middleware_index
            );

            if let Err(e) = self.middlewares[middleware_index]
                .process_outbound(&mut ctx)
                .await
            {
                error!(
                    "[{}] Error processing outbound message in middleware {}: {:?}",
                    connection_id, middleware_index, e
                );
                return Err(WebsocketError::HandlerError(e.into(), state));
            };

            debug!(
                "[{}] Middleware processing complete, message present: {:?}",
                connection_id,
                ctx.message.is_some()
            );
            ctx.message
        } else {
            debug!(
                "[{}] No middleware processing needed (index 0)",
                connection_id
            );
            Some(message)
        };

        match message {
            Some(message) => {
                debug!("[{}] Converting outbound message to string", connection_id);
                let Ok(string_message) = self.message_converter.outbound_to_string(message) else {
                    error!(
                        "[{}] Failed to convert outbound message to string",
                        connection_id
                    );
                    return Err(WebsocketError::OutboundMessageConversionError(
                        "Failed to convert outbound message to string".to_string(),
                        state,
                    ));
                };
                debug!(
                    "[{}] Successfully converted message to string",
                    connection_id
                );
                Ok((state, Some(string_message)))
            }
            None => {
                debug!("[{}] No message to send after processing", connection_id);
                Ok((state, None))
            }
        }
    }

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
