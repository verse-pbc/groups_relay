use crate::database::NostrDatabase;
use crate::event_store_connection::{EventStoreConfig, EventStoreConnection};
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
pub struct EventStore {
    database: Arc<NostrDatabase>,
    _token: CancellationToken,
}

impl EventStore {
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
        let config = EventStoreConfig {
            id: connection_id,
            database: self.database.clone(),
            relay_url,
            cancellation_token,
        };

        let connection = EventStoreConnection::new(config, sender).await?;

        Ok(connection)
    }

    async fn fetch_historical_events(
        &self,
        connection: &EventStoreConnection,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<()> {
        // Fetch historical events
        let events = match connection.fetch_events(filters.to_vec()).await {
            Ok(events) => events,
            Err(e) => {
                error!("Failed to fetch historical events: {:?}", e);
                return Err(e.into());
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
impl Middleware for EventStore {
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
                    target: "relay_forwarder",
                    "[{}] Processing REQ message for subscription {} with filters {:?}",
                    connection_id, subscription_id, filters
                );

                // Add subscription to the connection
                let Some(sender) = ctx.sender.clone() else {
                    panic!("Sender is None");
                };

                // Set the message sender for the connection if not already set
                if connection.message_sender.is_none() {
                    debug!(
                        target: "relay_forwarder",
                        "[{}] Setting message sender for connection",
                        connection_id
                    );
                    connection.set_message_sender(sender.clone());
                }

                debug!(
                    target: "relay_forwarder",
                    "[{}] Adding subscription {} to connection",
                    connection_id,
                    subscription_id
                );
                connection.add_subscription(subscription_id.clone(), filters.clone());

                // Fetch historical events
                if let Err(e) = self
                    .fetch_historical_events(connection, subscription_id, filters, sender.clone())
                    .await
                {
                    error!(
                        target: "relay_forwarder",
                        "[{}] Error fetching historical events: {:?}", connection_id, e
                    );
                    debug!(
                        target: "relay_forwarder",
                        "[{}] Removing subscription {} due to error",
                        connection_id,
                        subscription_id
                    );
                    connection.remove_subscription(subscription_id);
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
                    target: "relay_forwarder",
                    "[{}] Received CLOSE message with subscription_id: {}",
                    connection_id, subscription_id
                );

                // Remove subscription from the connection
                debug!(
                    target: "relay_forwarder",
                    "[{}] Removing subscription {} due to CLOSE message",
                    connection_id,
                    subscription_id
                );
                connection.remove_subscription(subscription_id);

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
                    target: "relay_forwarder",
                    "[{}] Received EVENT message with id: {}", connection_id, event_id
                );

                if let Err(e) = connection.save_signed_event(*event.clone()).await {
                    error!(
                        target: "relay_forwarder",
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
                    target: "relay_forwarder",
                    "[{}] Processing AUTH message",
                    connection_id
                );
                return ctx.next().await;
            }
            ClientMessage::Count { .. } => {
                debug!(
                    target: "relay_forwarder",
                    "[{}] Not implemented: COUNT message",
                    connection_id
                );
                ctx.send_message(RelayMessage::Notice {
                    message: "COUNT not implemented".to_string(),
                })
                .await?;
                return ctx.next().await;
            }
            _ => {
                debug!(
                    target: "relay_forwarder",
                    "[{}] Not implemented client message: {:?}",
                    connection_id, ctx.message
                );

                ctx.send_message(RelayMessage::Notice {
                    message: "Not implemented".to_string(),
                })
                .await?;
                return ctx.next().await;
            }
        }
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let message = match ctx.message.as_ref() {
            Some(msg) => msg,
            None => return ctx.next().await,
        };

        if let RelayMessage::Closed {
            subscription_id, ..
        } = message
        {
            debug!(
                target: "relay_forwarder",
                "[{}] Subscription {} closed via CLOSED message",
                ctx.connection_id,
                subscription_id
            );
        }

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
            target: "relay_forwarder",
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
                    target: "relay_forwarder",
                    "[{}] Error adding connection to relay {}: {}",
                    ctx.connection_id,
                    ctx.state.relay_url,
                    e
                );
                return Err(e.context("Error adding connection"));
            }
        };

        debug!(
            target: "relay_forwarder",
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
            target: "relay_forwarder",
            "[{}] Connection disconnected",
            ctx.connection_id
        );
        ctx.next().await
    }
}
