use crate::error::Error;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use tokio::sync::mpsc::{channel, Sender};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};
use websocket_builder::{
    ConnectionContext, InboundContext, MessageConverter, MessageSender, Middleware,
    OutboundContext, SendMessage,
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

pub enum EventToSave {
    UnsignedEvent(UnsignedEvent),
    Event(Event),
}

impl EventToSave {
    pub fn is_replaceable(&self) -> bool {
        match self {
            EventToSave::UnsignedEvent(event) => event.kind.is_replaceable(),
            EventToSave::Event(event) => event.kind.is_replaceable(),
        }
    }
}

#[derive(Debug)]
pub struct RelayForwarder {
    pub relay_secret: Keys,
    #[allow(unused)]
    pub subscriptions: HashMap<SubscriptionId, Vec<Filter>>,
}

impl RelayForwarder {
    pub fn new(relay_secret: Keys) -> Self {
        Self {
            relay_secret,
            subscriptions: HashMap::new(),
        }
    }

    pub async fn add_connection(
        &self,
        relay_url: String,
        sender: MessageSender<RelayMessage>,
        cancellation_token: CancellationToken,
    ) -> Result<RelayClientConnection> {
        let connection = RelayClientConnection::new(
            relay_url.clone(),
            self.relay_secret.clone(),
            cancellation_token.clone(),
        )
        .await?;
        // let relay_client = connection.client.clone();
        // let token = cancellation_token.clone();
        // tokio::spawn(async move {
        //     token.cancelled().await;
        //     if let Err(e) = relay_client.shutdown().await {
        //         error!("Error shutting down relay client: {:?}", e);
        //     }
        // });

        let relay_client = connection.client.clone();

        // Box::pin used to avoid filling the stack with the captured variables
        tokio::spawn(Box::pin(async move {
            if let Err(e) = relay_client
                .handle_notifications(move |notification| {
                    let mut sender = sender.clone();
                    let cancellation_token = cancellation_token.clone();

                    async move {
                        if cancellation_token.is_cancelled() {
                            return Ok(true);
                        }

                        match notification {
                            RelayPoolNotification::Message { message, .. } => match message {
                                RelayMessage::Event { .. }
                                | RelayMessage::Notice { .. }
                                | RelayMessage::EndOfStoredEvents { .. }
                                | RelayMessage::Closed { .. } => {
                                    // Pipe them to the middlewares
                                    if let Err(e) = sender.send(message).await {
                                        error!("Failed to send nostr RelayMessage: {}", e);
                                    }
                                }
                                RelayMessage::Ok {
                                    event_id,
                                    status,
                                    message,
                                } => {
                                    if !status {
                                        warn!(
                                            "Received unsuccessful nostr Ok message for event {}: {}",
                                            event_id, message
                                        );
                                    }
                                }
                                RelayMessage::Auth { .. }
                                | RelayMessage::Count { .. }
                                | RelayMessage::NegErr { .. }
                                | RelayMessage::NegMsg { .. } => {
                                    // Ignored
                                }
                            },
                            RelayPoolNotification::Shutdown => {
                                debug!("Received shutdown notification");
                                return Ok(true);
                            }
                            RelayPoolNotification::Authenticated { relay_url } => {
                                debug!(
                                    "Received authenticated notification for relay: {}",
                                    relay_url
                                );
                            }
                            RelayPoolNotification::Event { .. } => {
                                // Covered by the message variant
                            }
                            #[allow(deprecated)]
                            RelayPoolNotification::RelayStatus { .. } => {
                                // Deprecated so we ignore
                            }
                        }

                        Ok(false)
                    }
                })
                .await
            {
                error!("Error handling notifications: {:?}", e);
            }

            debug!("Nostr connection closed");
        }));

        Ok(connection)
    }
}

struct ReplaceableEventsBuffer {
    buffer: HashMap<(PublicKey, Kind), UnsignedEvent>,
}

impl ReplaceableEventsBuffer {
    pub fn new() -> Self {
        Self {
            buffer: HashMap::new(),
        }
    }

    pub fn insert(&mut self, event: UnsignedEvent) {
        self.buffer.insert((event.pubkey, event.kind), event);
    }

    pub async fn flush(&mut self, client: &Client) {
        if self.buffer.is_empty() {
            return;
        }

        let Ok(signer) = client.signer().await else {
            error!("Error getting signer");
            return;
        };

        for (_, event) in self.buffer.drain() {
            match signer.sign_event(event).await {
                Ok(event) => {
                    debug!("Saving relay generated event: {:?}", event.as_json());
                    if let Err(e) = client.send_event(event).await {
                        error!("Error sending replaceable event: {:?}", e);
                    }
                }
                Err(e) => {
                    error!("Error signing event: {:?}", e);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelayClientConnection {
    pub client: Client,
    pub connection_token: CancellationToken,
    pub active_subscriptions: HashMap<SubscriptionId, Vec<Filter>>,
    replaceable_event_queue: Sender<UnsignedEvent>,
}

impl RelayClientConnection {
    async fn new(
        relay_url: String,
        relay_keys: Keys,
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        let client = create_client(&relay_url, relay_keys).await?;
        let (sender, mut receiver) = channel::<UnsignedEvent>(10);
        let client_clone = client.clone();

        let connection_token = cancellation_token.child_token();
        let token = connection_token.clone();

        // Box::pin used to avoid filling the stack with the captured variables
        tokio::spawn(Box::pin(async move {
            let mut buffer = ReplaceableEventsBuffer::new();

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        buffer.flush(&client_clone).await;
                        return;
                    }

                    event = receiver.recv() => {
                        if let Some(event) = event {
                            buffer.insert(event);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        buffer.flush(&client_clone).await;
                    }
                }
            }
        }));

        Ok(Self {
            client,
            connection_token,
            active_subscriptions: HashMap::new(),
            replaceable_event_queue: sender,
        })
    }

    pub async fn save_event(&self, event_builder: EventToSave) -> Result<(), Error> {
        match event_builder {
            EventToSave::UnsignedEvent(event) => {
                if let Err(e) = self.replaceable_event_queue.send(event).await {
                    error!("Error sending event to replaceable events sender: {:?}", e);
                }
            }
            EventToSave::Event(event) => {
                debug!("Saving event: {:?}", event.as_json());
                self.client.send_event(event).await?;
            }
        }
        Ok(())
    }

    pub fn get_subscription(&self, subscription_id: &SubscriptionId) -> Option<&[Filter]> {
        self.active_subscriptions
            .get(subscription_id)
            .map(|f| f.as_slice())
    }

    pub fn insert_subscription(&mut self, subscription_id: SubscriptionId, filters: Vec<Filter>) {
        self.active_subscriptions.insert(subscription_id, filters);
    }

    pub fn remove_subscription(&mut self, subscription_id: &SubscriptionId) {
        self.active_subscriptions.remove(subscription_id);
    }
}

pub async fn create_client(relay_url: &str, relay_keys: Keys) -> Result<Client> {
    let opts = Options::default()
        .autoconnect(true)
        .timeout(Duration::from_secs(5));

    let client = ClientBuilder::default()
        .opts(opts)
        .signer(relay_keys)
        .build();

    client.add_relay(relay_url).await?;
    Ok(client)
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
                debug!(
                    "[{}] Received REQ message with subscription_id: {} and filters: {:?}",
                    connection_id, subscription_id, filters
                );

                let opts = filters
                    .iter()
                    .find(|filter| filter.limit.is_some())
                    .map(|_| {
                        let filter_options = FilterOptions::ExitOnEOSE;
                        SubscribeAutoCloseOptions::default().filter(filter_options)
                    });

                connection.insert_subscription(subscription_id.clone(), filters.clone());

                if let Err(e) = connection
                    .client
                    .subscribe_with_id(subscription_id.clone(), filters.clone(), opts)
                    .await
                {
                    connection.remove_subscription(subscription_id);
                    error!("Error subscribing: {:?}", e);
                }

                return ctx.next().await;
            }
            ClientMessage::Event(event) => {
                let event_id = event.id;
                if let Err(e) = connection.client.send_event(*event.clone()).await {
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
            ClientMessage::Close(subscription_id) => {
                debug!(
                    "[{}] Received CLOSE message with subscription_id: {}",
                    connection_id, subscription_id
                );
                connection.client.unsubscribe(subscription_id.clone()).await;

                connection.remove_subscription(subscription_id);

                ctx.send_message(RelayMessage::Closed {
                    subscription_id: subscription_id.clone(),
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
            ctx.state.remove_subscription(subscription_id);
        }

        ctx.next().await
    }

    async fn on_connect<'a>(
        &'a self,
        ctx: &mut ConnectionContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let cancellation_token = ctx.state.connection_token.child_token();

        let connection = match self
            .add_connection(
                ctx.state.relay_url.clone(),
                ctx.sender.clone().unwrap(),
                cancellation_token,
            )
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
}
