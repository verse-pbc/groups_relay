use crate::event_store_connection::EventStoreConnection;
use crate::nostr_database::NostrDatabase;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, MessageConverter, MessageSender,
    Middleware, OutboundContext, SendMessage,
};

pub struct NostrMessageConverter;

impl MessageConverter<ClientMessage, RelayMessage> for NostrMessageConverter {
    fn outbound_to_string(&self, message: RelayMessage) -> Result<String> {
        Ok(message.as_json())
    }

    fn inbound_from_string(&self, message: String) -> Result<Option<ClientMessage>> {
        if let Ok(client_message) = ClientMessage::from_json(&message) {
            Ok(Some(client_message))
        } else {
            warn!("Ignoring invalid inbound message: {}", message);
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct EventStoreMiddleware {
    database: Arc<NostrDatabase>,
    _token: CancellationToken,
}

impl EventStoreMiddleware {
    pub fn new(database: Arc<NostrDatabase>) -> Self {
        let token = CancellationToken::new();

        Self {
            database,
            _token: token,
        }
    }

    pub async fn add_connection(
        &self,
        connection_id: String,
        relay_url: String,
        sender: MessageSender<RelayMessage>,
        cancellation_token: CancellationToken,
    ) -> Result<EventStoreConnection> {
        let connection = EventStoreConnection::new(
            connection_id,
            self.database.clone(),
            relay_url,
            cancellation_token,
            sender,
        )
        .await?;

        Ok(connection)
    }

    async fn fetch_historical_events(
        &self,
        _connection: &EventStoreConnection,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<()> {
        // Fetch historical events from the database directly
        let events = match self.database.fetch_events(filters.to_vec()).await {
            Ok(events) => events,
            Err(e) => {
                error!("Failed to fetch historical events: {:?}", e);
                return Err(e);
            }
        };

        // Send each event
        let len = events.len();
        for event in events {
            if let Err(e) = sender
                .send(RelayMessage::Event {
                    subscription_id: subscription_id.clone(),
                    event: Box::new(event),
                })
                .await
            {
                error!(
                    "Failed to send historical event to subscription {}: {:?}",
                    subscription_id, e
                );
            }
        }

        debug!(
            "Sending EOSE for subscription {} after {} historical events",
            subscription_id, len
        );

        // Send EOSE
        if let Err(e) = sender
            .send(RelayMessage::EndOfStoredEvents(subscription_id.clone()))
            .await
        {
            error!(
                "Failed to send EOSE to subscription {}: {:?}",
                subscription_id, e
            );
        }

        Ok(())
    }
}

#[async_trait]
impl Middleware for EventStoreMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let connection_id = ctx.connection_id.as_str();

        let Some(connection) = ctx.state.relay_connection.as_mut() else {
            return Err(anyhow::anyhow!("No connection found"));
        };

        match &ctx.message {
            ClientMessage::Req {
                subscription_id,
                filters,
            } => {
                debug!(
                    target: "event_store",
                    "[{}] Processing REQ message for subscription {} with filters {:?}",
                    connection_id, subscription_id, filters
                );

                // Add subscription to the connection
                let Some(sender) = ctx.sender.clone() else {
                    panic!("Sender is None");
                };

                debug!(
                    target: "event_store",
                    "[{}] Adding subscription {} to connection",
                    connection_id,
                    subscription_id
                );
                connection
                    .handle_subscription(subscription_id.clone(), filters.clone())
                    .await?;

                // Fetch historical events
                if let Err(e) = self
                    .fetch_historical_events(connection, subscription_id, filters, sender.clone())
                    .await
                {
                    error!(
                        target: "event_store",
                        "[{}] Error fetching historical events: {:?}", connection_id, e
                    );
                    debug!(
                        target: "event_store",
                        "[{}] Removing subscription {} due to error",
                        connection_id,
                        subscription_id
                    );
                    connection
                        .handle_unsubscribe(subscription_id.clone())
                        .await?;
                    return ctx
                        .send_message(RelayMessage::Closed {
                            subscription_id: subscription_id.clone(),
                            message: "Error fetching historical events".to_string(),
                        })
                        .await;
                }

                return ctx.next().await;
            }
            ClientMessage::Close(subscription_id) => {
                debug!(
                    target: "event_store",
                    "[{}] Received CLOSE message with subscription_id: {}",
                    connection_id, subscription_id
                );

                // Remove subscription from the connection
                debug!(
                    target: "event_store",
                    "[{}] Removing subscription {} due to CLOSE message",
                    connection_id,
                    subscription_id
                );
                connection
                    .handle_unsubscribe(subscription_id.clone())
                    .await?;

                ctx.send_message(RelayMessage::Closed {
                    subscription_id: subscription_id.clone(),
                    message: "".to_string(),
                })
                .await?;
                return ctx.next().await;
            }
            ClientMessage::Event(event) => {
                let event_id = event.id;
                debug!(
                    target: "event_store",
                    "[{}] Received EVENT message with id: {}", connection_id, event_id
                );

                if let Err(e) = connection.handle_event(*event.clone()).await {
                    error!(
                        target: "event_store",
                        "[{}] Error sending event to relay: {:?}", connection_id, e
                    );
                    ctx.send_message(RelayMessage::Ok {
                        event_id,
                        status: false,
                        message: "Error saving event".to_string(),
                    })
                    .await?;
                    return ctx.next().await;
                }

                ctx.send_message(RelayMessage::Ok {
                    event_id,
                    status: true,
                    message: "".to_string(),
                })
                .await?;
                return ctx.next().await;
            }
            ClientMessage::Auth(_event) => {
                debug!(
                    target: "event_store",
                    "[{}] Received AUTH message", connection_id
                );
                return ctx.next().await;
            }
            _ => {
                return ctx.next().await;
            }
        }
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    async fn on_connect<'a>(
        &'a self,
        ctx: &mut ConnectionContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let cancellation_token = ctx.state.connection_token.child_token();

        let Some(sender) = ctx.sender.clone() else {
            panic!("Sender is None");
        };

        debug!(
            target: "event_store",
            "[{}] Creating new connection to relay {}",
            ctx.connection_id,
            ctx.state.relay_url
        );

        let connection = match self
            .add_connection(
                ctx.connection_id.clone(),
                ctx.state.relay_url.clone(),
                sender,
                cancellation_token,
            )
            .await
        {
            Ok(connection) => connection,
            Err(e) => {
                error!(
                    target: "event_store",
                    "[{}] Error adding connection to relay {}: {}",
                    ctx.connection_id,
                    ctx.state.relay_url,
                    e
                );
                return Err(e.context("Error adding connection"));
            }
        };

        debug!(
            target: "event_store",
            "[{}] Successfully created connection to relay {}",
            ctx.connection_id,
            ctx.state.relay_url
        );

        ctx.state.relay_connection = Some(connection);

        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        debug!(
            target: "event_store",
            "[{}] Connection disconnected",
            ctx.connection_id
        );
        ctx.next().await
    }
}
