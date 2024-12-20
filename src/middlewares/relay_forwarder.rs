use crate::nostr_session_state::NostrConnectionState;
use crate::relay_client_connection::RelayClientConnection;
use crate::Error;
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use nostr_sdk::prelude::*;
use std::time::Duration;
use tokio::sync::mpsc::Sender;
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

#[derive(Debug, Clone)]
struct Subscription {
    filters: Vec<Filter>,
    sender: MessageSender<RelayMessage>,
    connection_id: String,
}

impl Subscription {
    pub fn new(
        filters: Vec<Filter>,
        sender: MessageSender<RelayMessage>,
        connection_id: String,
    ) -> Self {
        Self {
            filters,
            sender,
            connection_id,
        }
    }

    pub fn matches(&self, event: &Event) -> bool {
        self.filters.iter().any(|filter| filter.match_event(event))
    }
}

#[derive(Debug)]
pub struct RelayForwarder {
    pub relay_secret: Keys,
    broadcast_sender: Sender<Event>,
    sub_update_sender: Sender<SubUpdateMessage>,
    _token: CancellationToken,
}

//These are basically actor messages but for the moment it's not worth to use an actor library for this
// TODO: Refactor for ractor?
#[derive(Debug)]
enum SubUpdateMessage {
    Add(SubscriptionId, Subscription),
    Remove(SubscriptionId),
    RemoveConnection(String),
}

impl RelayForwarder {
    pub fn new(relay_secret: Keys) -> Self {
        let (event_sender, mut event_receiver) = tokio::sync::mpsc::channel::<Event>(100);
        let (sub_sender, mut sub_receiver) = tokio::sync::mpsc::channel::<SubUpdateMessage>(100);
        let token = CancellationToken::new();

        let forwarder = Self {
            relay_secret,
            broadcast_sender: event_sender,
            sub_update_sender: sub_sender,
            _token: token,
        };

        let token_clone = forwarder._token.clone();
        let broadcast_subs: DashMap<SubscriptionId, Subscription> = DashMap::new();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;

                    _ = token_clone.cancelled() => {
                        debug!("Broadcast task shutting down");
                        return;
                    }

                    Some(update_msg) = sub_receiver.recv() => {
                        match update_msg {
                            SubUpdateMessage::Add(sub_id, sub) => {
                                debug!("Broadcast task: Adding subscription {}", sub_id);
                                broadcast_subs.insert(sub_id, sub);
                            }
                            SubUpdateMessage::Remove(sub_id) => {
                                debug!("Broadcast task: Removing subscription {}", sub_id);
                                broadcast_subs.remove(&sub_id);
                            }
                            SubUpdateMessage::RemoveConnection(conn_id) => {
                                debug!("Broadcast task: Removing all subscriptions for connection {}", conn_id);
                                broadcast_subs.retain(|_, sub| sub.connection_id != conn_id);
                            }
                        }
                    }

                    Some(event) = event_receiver.recv() => {
                        debug!("Broadcast task received event: kind={}", event.kind);
                        let mut matching_subscriptions = Vec::new();

                        // Log subscription summary
                        let total_subs = broadcast_subs.len();
                        debug!("Current subscription summary:");
                        debug!("Total subscriptions: {}", total_subs);
                        if total_subs > 0 {
                            broadcast_subs.iter().for_each(|entry| {
                                let sub = entry.value();
                                debug!(
                                    "  Subscription {}: connection={}, filters={:?}",
                                    entry.key(),
                                    sub.connection_id,
                                    sub.filters
                                );
                            });
                        }

                        // Collect matching subscriptions
                        broadcast_subs.iter().for_each(|entry| {
                            let subscription_id = entry.key().clone();
                            let subscription = entry.value().clone();

                            if subscription.matches(&event) {
                                debug!("Found matching subscription: {}", subscription_id);
                                matching_subscriptions.push((subscription_id, subscription.sender));
                            }
                        });

                        debug!("Broadcasting to {} matching subscriptions", matching_subscriptions.len());
                        // Send to all matching subscriptions
                        for (subscription_id, mut sender) in matching_subscriptions {
                            if let Err(e) = sender
                                .send(RelayMessage::Event {
                                    event: Box::new(event.clone()),
                                    subscription_id: subscription_id.clone(),
                                })
                                .await
                            {
                                error!(
                                    "Failed to send event to subscription {}: {:?}",
                                    subscription_id, e
                                );
                            } else {
                                debug!("Successfully sent event to subscription {}", subscription_id);
                            }
                        }
                    }
                }
            }
        });

        forwarder
    }

    pub fn remove_connection_subscriptions(&self, connection_id: &str) {
        if let Err(e) = self
            .sub_update_sender
            .try_send(SubUpdateMessage::RemoveConnection(
                connection_id.to_string(),
            ))
        {
            error!("Failed to send connection cleanup request: {:?}", e);
        } else {
            debug!("Sent cleanup request for connection {}", connection_id);
        }
    }

    pub async fn broadcast(&self, event: &Event) {
        if let Err(e) = self.broadcast_sender.send(event.clone()).await {
            error!("Failed to send event to broadcast channel: {:?}", e);
        }
    }

    pub async fn add_connection(
        &self,
        relay_url: String,
        cancellation_token: CancellationToken,
    ) -> Result<RelayClientConnection> {
        let connection = RelayClientConnection::new(
            relay_url.clone(),
            self.relay_secret.clone(),
            cancellation_token.clone(),
            self.broadcast_sender.clone(),
        )
        .await?;

        Ok(connection)
    }

    async fn fetch_historical_events(
        &self,
        connection: &RelayClientConnection,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<(), Error> {
        // Fetch historical events with a 10-second timeout
        let events = connection
            .fetch_events(filters.to_vec(), Some(Duration::from_secs(10)))
            .await?;

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
impl Middleware for RelayForwarder {
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
                // Store subscription for future custom event filtering and distribution
                if let Some(ref sender) = ctx.sender {
                    let sub = Subscription::new(
                        filters.clone(),
                        sender.clone(),
                        connection_id.to_string(),
                    );

                    if let Err(e) = self
                        .sub_update_sender
                        .send(SubUpdateMessage::Add(subscription_id.clone(), sub))
                        .await
                    {
                        error!("Failed to send subscription update: {:?}", e);
                    }

                    // Fetch historical events
                    if let Err(e) = self
                        .fetch_historical_events(
                            connection,
                            subscription_id,
                            filters,
                            sender.clone(),
                        )
                        .await
                    {
                        error!("Error fetching historical events: {:?}", e);
                        if let Err(e) = self
                            .sub_update_sender
                            .send(SubUpdateMessage::Remove(subscription_id.clone()))
                            .await
                        {
                            error!("Failed to remove subscription after error: {:?}", e);
                        }
                        return ctx
                            .send_message(RelayMessage::Closed {
                                subscription_id: subscription_id.clone(),
                                message: "".to_string(),
                            })
                            .await;
                    }
                } else {
                    error!("No sender available for subscription {}", subscription_id);
                    return ctx
                        .send_message(RelayMessage::Closed {
                            subscription_id: subscription_id.clone(),
                            message: "".to_string(),
                        })
                        .await;
                }

                return ctx.next().await;
            }
            ClientMessage::Close(subscription_id) => {
                debug!(
                    "[{}] Received CLOSE message with subscription_id: {}",
                    connection_id, subscription_id
                );

                // Remove subscription from our tracking
                if let Err(e) = self
                    .sub_update_sender
                    .send(SubUpdateMessage::Remove(subscription_id.clone()))
                    .await
                {
                    error!("Failed to send subscription removal: {:?}", e);
                }

                ctx.send_message(RelayMessage::Closed {
                    subscription_id: subscription_id.clone(),
                    message: "".to_string(),
                })
                .await?;
                return ctx.next().await;
            }
            ClientMessage::Event(event) => {
                let event_id = event.id;
                if let Err(e) = connection.send_event(*event.clone()).await {
                    error!("Error sending event to relay: {:?}", e);
                    ctx.send_message(RelayMessage::Ok {
                        event_id,
                        status: false,
                        message: "Error sending event".to_string(),
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
            _ => {
                debug!(
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
            debug!("Subscription {} closed via CLOSED message", subscription_id);
        }

        ctx.next().await
    }

    async fn on_connect<'a>(
        &'a self,
        ctx: &mut ConnectionContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let cancellation_token = ctx.state.connection_token.child_token();

        let connection = match self
            .add_connection(ctx.state.relay_url.clone(), cancellation_token)
            .await
        {
            Ok(connection) => connection,
            Err(e) => {
                error!(
                    "Error adding connection to relay {}: {}",
                    ctx.state.relay_url, e
                );
                return Err(e.context("Error adding connection"));
            }
        };

        ctx.state.relay_connection = Some(connection);

        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let connection_id = ctx.connection_id.as_str();

        self.remove_connection_subscriptions(connection_id);

        ctx.next().await
    }
}
