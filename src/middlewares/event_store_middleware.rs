use crate::error::Error;
use crate::event_store_connection::EventStoreConnection;
use crate::metrics;
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

#[derive(Clone)]
pub struct NostrMessageConverter;

impl MessageConverter<ClientMessage, RelayMessage> for NostrMessageConverter {
    fn outbound_to_string(&self, message: RelayMessage) -> Result<String> {
        debug!("Converting outbound message to string: {:?}", message);
        Ok(message.as_json())
    }

    fn inbound_from_string(&self, message: String) -> Result<Option<ClientMessage>> {
        if let Ok(client_message) = ClientMessage::from_json(&message) {
            debug!("Successfully parsed inbound message: {}", message);
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
        connection: &EventStoreConnection,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<(), Error> {
        let events = match connection.fetch_events(filters.to_vec()).await {
            Ok(events) => events,
            Err(e) => {
                error!("Failed to fetch historical events: {:?}", e);
                return Err(e);
            }
        };

        // Send each event
        let len = events.len();
        let capacity = connection.sender_capacity() / 2;

        for event in events.into_iter().take(capacity) {
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

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let connection_id = ctx.connection_id.clone();

        match &ctx.message {
            ClientMessage::Req {
                subscription_id,
                filters,
            } => {
                debug!(
                    target: "event_store",
                    "[{}] Received REQ message for subscription {}, connection present: {}",
                    connection_id,
                    subscription_id,
                    ctx.state.relay_connection.is_some()
                );

                let connection = ctx.state.relay_connection.as_ref();
                if let Some(connection) = connection {
                    connection
                        .handle_subscription(subscription_id.clone(), filters.clone())
                        .await?;

                    debug!(
                        target: "event_store",
                        "[{}] Added subscription {}",
                        connection_id,
                        subscription_id
                    );

                    // Fetch and send historical events before EOSE
                    if let Some(sender) = &mut ctx.sender {
                        self.fetch_historical_events(
                            connection,
                            subscription_id,
                            filters,
                            sender.clone(),
                        )
                        .await?;
                    }
                } else {
                    error!(
                        target: "event_store",
                        "[{}] No connection available for subscription {}",
                        connection_id,
                        subscription_id
                    );
                }

                ctx.next().await
            }
            ClientMessage::Close(subscription_id) => {
                debug!(
                    target: "event_store",
                    "[{}] Received CLOSE message for subscription {}, connection present: {}",
                    connection_id,
                    subscription_id,
                    ctx.state.relay_connection.is_some()
                );

                let connection = ctx.state.relay_connection.as_ref();
                if let Some(connection) = connection {
                    connection
                        .handle_unsubscribe(subscription_id.clone())
                        .await?;
                }

                return ctx
                    .send_message(RelayMessage::Closed {
                        subscription_id: subscription_id.clone(),
                        message: "".to_string(),
                    })
                    .await;
            }
            ClientMessage::Event(event) => {
                debug!(
                    target: "event_store",
                    "[{}] Received EVENT message: {}, connection present: {}",
                    connection_id,
                    event.id,
                    ctx.state.relay_connection.is_some()
                );

                let event_id = event.id;
                let connection = ctx.state.relay_connection.as_ref();
                if let Some(connection) = connection {
                    if let Err(e) = connection.save_and_broadcast(*event.clone()).await {
                        error!(
                            target: "event_store",
                            "[{}] Failed to save event: {:?}",
                            connection_id,
                            e
                        );
                        return Ok(());
                    }
                    debug!(
                        target: "event_store",
                        "[{}] Successfully handled event {}",
                        connection_id,
                        event_id
                    );
                } else {
                    error!(
                        target: "event_store",
                        "[{}] No connection available for event {}",
                        connection_id,
                        event_id
                    );
                }

                ctx.send_message(RelayMessage::Ok {
                    event_id,
                    status: true,
                    message: "".to_string(),
                })
                .await?;
                ctx.next().await
            }
            ClientMessage::Auth(_event) => {
                debug!(
                    target: "event_store",
                    "[{}] Received AUTH message, connection present: {}",
                    connection_id,
                    ctx.state.relay_connection.is_some()
                );
                ctx.next().await
            }
            _ => ctx.next().await,
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        debug!(
            target: "event_store",
            "[{}] Setting up connection",
            ctx.connection_id
        );

        let connection = EventStoreConnection::new(
            ctx.connection_id.clone(),
            self.database.clone(),
            ctx.connection_id.clone(),
            ctx.state.connection_token.clone(),
            ctx.sender.clone().expect("Sender must be present"),
        )
        .await?;

        ctx.state.relay_connection = Some(connection);

        debug!(
            target: "event_store",
            "[{}] Connection setup complete",
            ctx.connection_id
        );

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

        // Get the active subscription count and decrement metrics in one go
        if let Some(connection) = &ctx.state.relay_connection {
            if let Ok(count) = connection.subscription_count().await {
                if count > 0 {
                    // Decrement all subscriptions at once
                    metrics::active_subscriptions().decrement(count as f64);
                }
            }
        }

        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{ConnectInfo, State, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::{SinkExt, StreamExt};
    use nostr_sdk::{EventBuilder, Keys};
    use std::time::Instant;
    use std::{net::SocketAddr, time::Duration};
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use tokio_util::sync::CancellationToken;
    use websocket_builder::{StateFactory, WebSocketBuilder, WebSocketHandler};

    struct TestStateFactory;

    impl StateFactory<NostrConnectionState> for TestStateFactory {
        fn create_state(&self, token: CancellationToken) -> NostrConnectionState {
            NostrConnectionState {
                relay_url: "ws://test.relay".to_string(),
                challenge: None,
                authed_pubkey: None,
                relay_connection: None,
                connection_token: token,
            }
        }
    }

    struct ServerState {
        ws_handler: WebSocketHandler<
            NostrConnectionState,
            ClientMessage,
            RelayMessage,
            NostrMessageConverter,
            TestStateFactory,
        >,
        shutdown: CancellationToken,
    }

    async fn websocket_handler(
        ws: WebSocketUpgrade,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        State(state): State<Arc<ServerState>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| async move {
            state
                .ws_handler
                .start(socket, addr.to_string(), state.shutdown.clone())
                .await
                .unwrap();
        })
    }

    async fn setup_test() -> (TempDir, Arc<NostrDatabase>) {
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("test.db");
        let keys = Keys::generate();
        let database =
            Arc::new(NostrDatabase::new(db_path.to_str().unwrap().to_string(), keys).unwrap());

        (tmp_dir, database)
    }

    async fn start_test_server(database: Arc<NostrDatabase>) -> (SocketAddr, CancellationToken) {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let cancellation_token = CancellationToken::new();
        let token = cancellation_token.clone();

        let ws_handler = WebSocketBuilder::new(TestStateFactory, NostrMessageConverter)
            .with_middleware(EventStoreMiddleware::new(database))
            .build();

        let server_state = ServerState {
            ws_handler,
            shutdown: token,
        };

        let app = Router::new()
            .route("/", get(websocket_handler))
            .with_state(Arc::new(server_state));

        let listener = TcpListener::bind(addr).await.unwrap();
        let local_addr = listener.local_addr().unwrap();

        let token = cancellation_token.clone();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                token.cancelled().await;
            })
            .await
            .unwrap();
        });

        (local_addr, cancellation_token)
    }

    async fn create_signed_event(content: &str) -> (Keys, Event) {
        let keys = Keys::generate();
        let event =
            EventBuilder::text_note(content).build_with_ctx(&Instant::now(), keys.public_key());
        let event = keys.sign_event(event).await.unwrap();
        (keys, event)
    }

    struct TestClient {
        write: futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        read: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    }

    impl TestClient {
        async fn connect(url: &str) -> Self {
            let (ws_stream, _) = connect_async(url).await.unwrap();
            let (write, read) = ws_stream.split();
            Self { write, read }
        }

        async fn send_message(&mut self, msg: &ClientMessage) {
            self.write
                .send(Message::Text(msg.as_json().into()))
                .await
                .unwrap();
        }

        async fn expect_message(&mut self) -> RelayMessage {
            let msg = self.read.next().await.unwrap().unwrap();
            RelayMessage::from_json(msg.to_text().unwrap()).unwrap()
        }

        async fn expect_event(&mut self, subscription_id: &SubscriptionId, expected_event: &Event) {
            match self.expect_message().await {
                RelayMessage::Event {
                    subscription_id: sub_id,
                    event: received_event,
                } => {
                    assert_eq!(sub_id, *subscription_id, "Event subscription ID mismatch");
                    assert_eq!(
                        *received_event, *expected_event,
                        "Received event does not match expected event"
                    );
                }
                msg => panic!(
                    "Expected Event message for subscription {}, got: {:?}",
                    subscription_id, msg
                ),
            }
        }

        async fn expect_ok(&mut self, event_id: &EventId) {
            match self.expect_message().await {
                RelayMessage::Ok {
                    event_id: received_id,
                    status,
                    ..
                } => {
                    assert_eq!(received_id, *event_id, "OK message event ID mismatch");
                    assert!(status, "Event {} was not accepted by the relay", event_id);
                }
                msg => panic!("Expected OK message for event {}, got: {:?}", event_id, msg),
            }
        }

        async fn expect_eose(&mut self, subscription_id: &SubscriptionId) {
            match self.expect_message().await {
                RelayMessage::EndOfStoredEvents(sub_id) => {
                    assert_eq!(sub_id, *subscription_id, "EOSE subscription ID mismatch");
                }
                msg => panic!(
                    "Expected EOSE message for subscription {}, got: {:?}",
                    subscription_id, msg
                ),
            }
        }

        #[allow(dead_code)]
        async fn expect_closed(&mut self, subscription_id: &SubscriptionId) {
            match self.expect_message().await {
                RelayMessage::Closed {
                    subscription_id: id,
                    ..
                } => {
                    assert_eq!(&id, subscription_id);
                }
                msg => panic!("Expected Closed message, got: {:?}", msg),
            }
        }

        async fn close(mut self) {
            self.write.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_subscription_receives_historical_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a historical event
        let (_, historical_event) = create_signed_event("Historical event").await;
        database.save_signed_event(&historical_event).await.unwrap();

        // Start the test server
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect client
        let url = format!("ws://{}", addr);
        let mut subscriber = TestClient::connect(&url).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote]).limit(5)];
        subscriber
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters,
            })
            .await;

        // Verify historical event and EOSE
        subscriber
            .expect_event(&subscription_id, &historical_event)
            .await;
        subscriber.expect_eose(&subscription_id).await;

        // Clean up
        subscriber.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_subscription_receives_new_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Start the test server
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect clients
        let url = format!("ws://{}", addr);
        let mut subscriber = TestClient::connect(&url).await;
        let mut publisher = TestClient::connect(&url).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote]).limit(5)];
        subscriber
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters,
            })
            .await;

        // Wait for EOSE since there are no historical events
        subscriber.expect_eose(&subscription_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Publish new event
        let (_, event) = create_signed_event("Hello, world!").await;
        publisher
            .send_message(&ClientMessage::Event(Box::new(event.clone())))
            .await;

        // Verify subscriber receives the new event
        publisher.expect_ok(&event.id).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        subscriber.expect_event(&subscription_id, &event).await;

        // Clean up
        subscriber.close().await;
        publisher.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_subscription_receives_both_historical_and_new_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a historical event
        let (_, historical_event) = create_signed_event("Historical event").await;
        database.save_signed_event(&historical_event).await.unwrap();

        // Start the test server
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect clients
        let url = format!("ws://{}", addr);
        let mut subscriber = TestClient::connect(&url).await;
        let mut publisher = TestClient::connect(&url).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote]).limit(5)];
        subscriber
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters,
            })
            .await;

        // Verify historical event and EOSE
        subscriber
            .expect_event(&subscription_id, &historical_event)
            .await;
        subscriber.expect_eose(&subscription_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Publish new event
        let (_, event) = create_signed_event("Hello, world!").await;
        publisher
            .send_message(&ClientMessage::Event(Box::new(event.clone())))
            .await;

        // Verify subscriber receives the new event
        publisher.expect_ok(&event.id).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        subscriber.expect_event(&subscription_id, &event).await;

        // Clean up
        subscriber.close().await;
        publisher.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_text_note_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a text note event
        let (_keys, text_note) = create_signed_event("Text note event").await;
        database.save_signed_event(&text_note).await.unwrap();

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with empty filter
        let subscription_id = SubscriptionId::new("text_note_events");
        let empty_filter = vec![Filter::new()];
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: empty_filter,
            })
            .await;

        // We should receive the text note event
        match client.expect_message().await {
            RelayMessage::Event { event, .. } => {
                assert_eq!(event.kind, Kind::TextNote, "Event was not a text note");
            }
            msg => panic!("Expected Event message, got: {:?}", msg),
        }

        client.expect_eose(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_metadata_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a metadata event
        let keys = Keys::generate();
        let metadata_event = EventBuilder::new(Kind::Metadata, "{}")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let metadata_event = keys.sign_event(metadata_event).await.unwrap();
        database.save_signed_event(&metadata_event).await.unwrap();

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with empty filter
        let subscription_id = SubscriptionId::new("metadata_events");
        let empty_filter = vec![Filter::new()];
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: empty_filter,
            })
            .await;

        // We should receive the metadata event
        match client.expect_message().await {
            RelayMessage::Event { event, .. } => {
                assert_eq!(event.kind, Kind::Metadata, "Event was not a metadata event");
            }
            msg => panic!("Expected Event message, got: {:?}", msg),
        }

        client.expect_eose(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_contact_list_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a contact list event
        let keys = Keys::generate();
        let contacts_event = EventBuilder::new(Kind::ContactList, "[]")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let contacts_event = keys.sign_event(contacts_event).await.unwrap();
        database.save_signed_event(&contacts_event).await.unwrap();

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with empty filter
        let subscription_id = SubscriptionId::new("contact_list_events");
        let empty_filter = vec![Filter::new()];
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: empty_filter,
            })
            .await;

        // We should receive the contacts event
        match client.expect_message().await {
            RelayMessage::Event { event, .. } => {
                assert_eq!(
                    event.kind,
                    Kind::ContactList,
                    "Event was not a contact list event"
                );
            }
            msg => panic!("Expected Event message, got: {:?}", msg),
        }

        client.expect_eose(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_all_event_kinds() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save events of different kinds
        let (keys, text_note) = create_signed_event("Text note event").await;
        let metadata_event = EventBuilder::new(Kind::Metadata, "{}")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let metadata_event = keys.sign_event(metadata_event).await.unwrap();
        let contacts_event = EventBuilder::new(Kind::ContactList, "[]")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let contacts_event = keys.sign_event(contacts_event).await.unwrap();

        database.save_signed_event(&text_note).await.unwrap();
        database.save_signed_event(&metadata_event).await.unwrap();
        database.save_signed_event(&contacts_event).await.unwrap();

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with empty filter
        let subscription_id = SubscriptionId::new("all_events");
        let empty_filter = vec![Filter::new()];
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: empty_filter,
            })
            .await;

        // We should receive all events
        let mut received_events = Vec::new();
        for _ in 0..3 {
            match client.expect_message().await {
                RelayMessage::Event { event, .. } => received_events.push(*event),
                msg => panic!("Expected Event message, got: {:?}", msg),
            }
        }

        client.expect_eose(&subscription_id).await;

        // Verify we got all events regardless of save order
        assert_eq!(received_events.len(), 3, "Did not receive all events");
        let received_kinds: Vec<Kind> = received_events.iter().map(|e| e.kind).collect();
        assert!(
            received_kinds.contains(&Kind::TextNote),
            "Missing text note event"
        );
        assert!(
            received_kinds.contains(&Kind::Metadata),
            "Missing metadata event"
        );
        assert!(
            received_kinds.contains(&Kind::ContactList),
            "Missing contact list event"
        );

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_events_from_multiple_authors() {
        let (_tmp_dir, database) = setup_test().await;

        // Create events with different authors
        let (keys1, event1) = create_signed_event("Event from author 1").await;
        let (keys2, event2) = create_signed_event("Event from author 2").await;

        // Save events
        database.save_signed_event(&event1).await.unwrap();
        database.save_signed_event(&event2).await.unwrap();

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with empty filter
        let subscription_id = SubscriptionId::new("multi_author_events");
        let empty_filter = vec![Filter::new()];
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: empty_filter,
            })
            .await;

        // We should receive events from both authors
        let mut received_events = Vec::new();
        for _ in 0..2 {
            match client.expect_message().await {
                RelayMessage::Event { event, .. } => received_events.push(*event),
                msg => panic!("Expected Event message, got: {:?}", msg),
            }
        }

        client.expect_eose(&subscription_id).await;

        // Verify we got events from both authors
        assert!(
            received_events
                .iter()
                .any(|e| e.pubkey == keys1.public_key()),
            "No events from author 1"
        );
        assert!(
            received_events
                .iter()
                .any(|e| e.pubkey == keys2.public_key()),
            "No events from author 2"
        );

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_limit_filter_returns_events_in_reverse_chronological_order() {
        let (_tmp_dir, database) = setup_test().await;

        // Create events with different timestamps
        let now = Instant::now();
        let (keys, event1) = create_signed_event("First event").await;
        let event2 = EventBuilder::new(Kind::TextNote, "Second event")
            .build_with_ctx(&(now + Duration::from_secs(1)), keys.public_key());
        let event2 = keys.sign_event(event2).await.unwrap();
        let event3 = EventBuilder::new(Kind::TextNote, "Third event")
            .build_with_ctx(&(now + Duration::from_secs(2)), keys.public_key());
        let event3 = keys.sign_event(event3).await.unwrap();

        // Save events in random order
        database.save_signed_event(&event2).await.unwrap();
        database.save_signed_event(&event1).await.unwrap();
        database.save_signed_event(&event3).await.unwrap();

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with limit filter
        let subscription_id = SubscriptionId::new("limited_events");
        let filter = vec![Filter::new().kinds(vec![Kind::TextNote]).limit(3)];
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: filter,
            })
            .await;

        // We should receive events in reverse chronological order
        let mut received_events = Vec::new();
        for _ in 0..3 {
            match client.expect_message().await {
                RelayMessage::Event { event, .. } => received_events.push(*event),
                msg => panic!("Expected Event message, got: {:?}", msg),
            }
        }

        client.expect_eose(&subscription_id).await;

        // Verify events are in reverse chronological order
        assert_eq!(
            received_events[0].created_at, event3.created_at,
            "First event should be the newest"
        );
        assert_eq!(
            received_events[1].created_at, event2.created_at,
            "Second event should be the second newest"
        );
        assert_eq!(
            received_events[2].created_at, event1.created_at,
            "Third event should be the oldest"
        );

        // Clean up
        client.close().await;
        token.cancel();
    }
}
