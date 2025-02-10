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
use tracing::{debug, error, info};
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
        // Parse synchronously since JSON parsing doesn't need to be async

        // Return immediately to maintain message order
        if let Ok(client_message) = ClientMessage::from_json(&message) {
            debug!("Successfully parsed inbound message: {}", message);
            Ok(Some(client_message))
        } else {
            error!("Ignoring invalid inbound message: {}", message);
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
                info!(
                    target: "event_store",
                    "[{}] Processing REQ message for subscription {}",
                    connection_id,
                    subscription_id
                );

                let connection = ctx.state.relay_connection.as_ref();
                if let Some(connection) = connection {
                    debug!(
                        target: "event_store",
                        "[{}] Adding subscription {} with filters: {:?}",
                        connection_id,
                        subscription_id,
                        filters
                    );

                    if let Err(e) = connection
                        .handle_subscription(subscription_id.clone(), filters.clone())
                        .await
                    {
                        error!(
                            target: "event_store",
                            "[{}] Failed to add subscription {}: {}",
                            connection_id,
                            subscription_id,
                            e
                        );
                        return Err(e.into());
                    }

                    debug!(
                        target: "event_store",
                        "[{}] Successfully added subscription {}",
                        connection_id,
                        subscription_id
                    );

                    // Fetch and send historical events before EOSE
                    if let Some(sender) = &mut ctx.sender {
                        debug!(
                            target: "event_store",
                            "[{}] Fetching historical events for subscription {}",
                            connection_id,
                            subscription_id
                        );

                        if let Err(e) = self
                            .fetch_historical_events(
                                connection,
                                subscription_id,
                                filters,
                                sender.clone(),
                            )
                            .await
                        {
                            error!(
                                target: "event_store",
                                "[{}] Failed to fetch historical events for subscription {}: {}",
                                connection_id,
                                subscription_id,
                                e
                            );
                            return Err(e.into());
                        }

                        debug!(
                            target: "event_store",
                            "[{}] Successfully sent historical events for subscription {}",
                            connection_id,
                            subscription_id
                        );
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
                info!(
                    target: "event_store",
                    "[{}] Processing CLOSE message for subscription {}",
                    connection_id,
                    subscription_id
                );

                let connection = ctx.state.relay_connection.as_ref();
                if let Some(connection) = connection {
                    if let Err(e) = connection.handle_unsubscribe(subscription_id.clone()).await {
                        error!(
                            target: "event_store",
                            "[{}] Failed to unsubscribe {}: {}",
                            connection_id,
                            subscription_id,
                            e
                        );
                        return Err(e.into());
                    }

                    debug!(
                        target: "event_store",
                        "[{}] Successfully unsubscribed {}",
                        connection_id,
                        subscription_id
                    );
                }

                ctx.send_message(RelayMessage::Closed {
                    subscription_id: subscription_id.clone(),
                    message: "".to_string(),
                })
                .await
            }
            ClientMessage::Event(event) => {
                info!(
                    target: "event_store",
                    "[{}] Processing EVENT message: {}",
                    connection_id,
                    event.id
                );

                let event_id = event.id;
                let connection = ctx.state.relay_connection.as_ref();
                if let Some(connection) = connection {
                    if let Err(e) = connection.save_and_broadcast(*event.clone()).await {
                        error!(
                            target: "event_store",
                            "[{}] Failed to save event {}: {}",
                            connection_id,
                            event_id,
                            e
                        );
                        return Ok(());
                    }
                    debug!(
                        target: "event_store",
                        "[{}] Successfully saved and broadcast event {}",
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
                    "[{}] Processing AUTH message",
                    connection_id
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
        debug!(
            target: "event_store",
            "[{}] Processing outbound message: {:?}",
            ctx.connection_id,
            ctx.message
        );
        let result = ctx.next().await;
        if let Err(ref e) = result {
            error!(
                target: "event_store",
                "[{}] Error processing outbound message: {}",
                ctx.connection_id,
                e
            );
        }
        result
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

        // Clean up any remaining subscriptions from the global metrics
        if let Some(connection) = &ctx.state.relay_connection {
            let remaining_subs = connection.get_local_subscription_count();
            if remaining_subs > 0 {
                debug!(
                    target: "event_store",
                    "[{}] Cleaning up {} remaining subscriptions from metrics",
                    ctx.connection_id,
                    remaining_subs
                );
                // Decrement the global metrics by the number of subscriptions that weren't explicitly closed
                metrics::active_subscriptions().decrement(remaining_subs as f64);
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
    use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
    use tokio_util::sync::CancellationToken;
    use tracing::{debug, error, warn};
    use websocket_builder::{StateFactory, WebSocketBuilder, WebSocketHandler};

    struct TestClient {
        write: futures_util::stream::SplitSink<
            WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
            Message,
        >,
        read: futures_util::stream::SplitStream<
            WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        >,
    }

    #[derive(Clone)]
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

    impl TestClient {
        async fn connect(url: &str) -> Self {
            debug!(target: "test_client", "Connecting to {}", url);
            let (ws_stream, _) = connect_async(url).await.unwrap();
            let (write, read) = ws_stream.split();
            debug!(target: "test_client", "Connected successfully to {}", url);
            Self { write, read }
        }

        async fn send_message(&mut self, msg: &ClientMessage) {
            let message = Message::Text(msg.as_json().into());
            debug!(target: "test_client", "Sending message: {:?}", message);
            self.write.send(message).await.unwrap();
        }

        async fn expect_message(&mut self) -> RelayMessage {
            debug!(target: "test_client", "Waiting for message");
            match self.read.next().await {
                Some(Ok(msg)) => {
                    debug!(target: "test_client", "Received message: {:?}", msg);
                    match msg {
                        Message::Text(text) => RelayMessage::from_json(&*text).unwrap(),
                        Message::Close(_) => {
                            debug!(target: "test_client", "Received close frame, sending close response");
                            // Send close frame in response if we haven't already
                            let _ = self.write.send(Message::Close(None)).await;
                            panic!("Unexpected close frame");
                        }
                        _ => panic!("Unexpected message type: {:?}", msg),
                    }
                }
                Some(Err(e)) => {
                    error!(target: "test_client", "WebSocket error: {}", e);
                    panic!("WebSocket error: {}", e);
                }
                None => {
                    error!(target: "test_client", "Connection closed unexpectedly");
                    panic!("Connection closed unexpectedly");
                }
            }
        }

        async fn expect_event(&mut self, subscription_id: &SubscriptionId, expected_event: &Event) {
            debug!(
                target: "test_client",
                "Expecting event for subscription {}", subscription_id
            );
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
                    debug!(
                        target: "test_client",
                        "Successfully received expected event for subscription {}", subscription_id
                    );
                }
                msg => panic!(
                    "Expected Event message for subscription {}, got: {:?}",
                    subscription_id, msg
                ),
            }
        }

        async fn expect_ok(&mut self, event_id: &EventId) {
            debug!(target: "test_client", "Expecting OK for event {}", event_id);
            match self.expect_message().await {
                RelayMessage::Ok {
                    event_id: received_id,
                    status,
                    ..
                } => {
                    assert_eq!(received_id, *event_id, "OK message event ID mismatch");
                    assert!(status, "Event {} was not accepted by the relay", event_id);
                    debug!(target: "test_client", "Successfully received OK for event {}", event_id);
                }
                msg => panic!("Expected OK message for event {}, got: {:?}", event_id, msg),
            }
        }

        async fn expect_eose(&mut self, subscription_id: &SubscriptionId) {
            debug!(
                target: "test_client",
                "Expecting EOSE for subscription {}", subscription_id
            );
            match self.expect_message().await {
                RelayMessage::EndOfStoredEvents(sub_id) => {
                    assert_eq!(sub_id, *subscription_id, "EOSE subscription ID mismatch");
                    debug!(
                        target: "test_client",
                        "Successfully received EOSE for subscription {}", subscription_id
                    );
                }
                msg => panic!(
                    "Expected EOSE message for subscription {}, got: {:?}",
                    subscription_id, msg
                ),
            }
        }

        async fn close(mut self) {
            debug!(target: "test_client", "Initiating graceful close");
            // Send close frame
            if let Err(e) = self.write.send(Message::Close(None)).await {
                warn!(target: "test_client", "Failed to send close frame: {}", e);
            }

            // Wait for close frame response or timeout after 1 second
            let timeout = tokio::time::sleep(Duration::from_secs(1));
            tokio::pin!(timeout);

            loop {
                tokio::select! {
                    msg = self.read.next() => {
                        match msg {
                            Some(Ok(Message::Close(_))) => {
                                debug!(target: "test_client", "Received close frame response");
                                break;
                            }
                            Some(Ok(msg)) => {
                                debug!(target: "test_client", "Ignoring message during close: {:?}", msg);
                                continue;
                            }
                            Some(Err(e)) => {
                                warn!(target: "test_client", "Error during close: {}", e);
                                break;
                            }
                            None => {
                                debug!(target: "test_client", "Connection closed by server");
                                break;
                            }
                        }
                    }
                    _ = &mut timeout => {
                        warn!(target: "test_client", "Close handshake timed out");
                        break;
                    }
                }
            }

            debug!(target: "test_client", "Close complete");
        }
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

        // Verify historical event
        subscriber
            .expect_event(&subscription_id, &historical_event)
            .await;
        subscriber.expect_eose(&subscription_id).await;

        // Publish new event
        let (_, new_event) = create_signed_event("New event").await;
        publisher
            .send_message(&ClientMessage::Event(Box::new(new_event.clone())))
            .await;

        // Verify subscriber receives both events
        publisher.expect_ok(&new_event.id).await;
        subscriber.expect_event(&subscription_id, &new_event).await;

        // Clean up
        subscriber.close().await;
        publisher.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_all_event_kinds() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save events of different kinds
        let keys = Keys::generate();
        let text_note =
            EventBuilder::text_note("Text note").build_with_ctx(&Instant::now(), keys.public_key());
        let text_note = keys.sign_event(text_note).await.unwrap();

        let mut metadata = Metadata::new();
        metadata.name = Some("Test User".to_string());
        metadata.about = Some("about me".to_string());
        metadata.picture = Some("https://example.com/pic.jpg".to_string());
        let metadata_event =
            EventBuilder::metadata(&metadata).build_with_ctx(&Instant::now(), keys.public_key());
        let metadata_event = keys.sign_event(metadata_event).await.unwrap();

        let recommend_relay = EventBuilder::new(Kind::RelayList, "wss://relay.example.com")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let recommend_relay = keys.sign_event(recommend_relay).await.unwrap();

        // Save all events
        database.save_signed_event(&text_note).await.unwrap();
        database.save_signed_event(&metadata_event).await.unwrap();
        database.save_signed_event(&recommend_relay).await.unwrap();

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

        // We should receive all events (order may vary)
        let mut received_kinds = vec![];
        for _ in 0..3 {
            match client.expect_message().await {
                RelayMessage::Event { event, .. } => {
                    received_kinds.push(event.kind);
                }
                msg => panic!("Expected Event message, got: {:?}", msg),
            }
        }

        client.expect_eose(&subscription_id).await;

        // Verify we received all kinds
        assert!(received_kinds.contains(&Kind::TextNote));
        assert!(received_kinds.contains(&Kind::Metadata));
        assert!(received_kinds.contains(&Kind::RelayList));

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_limit_filter_returns_events_in_reverse_chronological_order() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save events with different timestamps
        let keys = Keys::generate();
        let mut events = vec![];

        // Create events with increasing timestamps
        for i in 0..5 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key());
            let event = keys.sign_event(event).await.unwrap();
            database.save_signed_event(&event).await.unwrap();
            events.push(event);
        }

        // Start server and connect client
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Subscribe with limit filter
        let subscription_id = SubscriptionId::new("limited_events");
        let limit_filter = vec![Filter::new().limit(3)]; // Only get last 3 events
        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filters: limit_filter,
            })
            .await;

        // Collect received events
        let mut received_events = vec![];
        for _ in 0..3 {
            match client.expect_message().await {
                RelayMessage::Event { event, .. } => {
                    received_events.push(*event);
                }
                msg => panic!("Expected Event message, got: {:?}", msg),
            }
        }

        client.expect_eose(&subscription_id).await;

        // Verify events are in reverse chronological order
        for i in 0..received_events.len() - 1 {
            assert!(
                received_events[i].created_at >= received_events[i + 1].created_at,
                "Events not in reverse chronological order"
            );
        }

        // Verify we got the most recent events (last 3 from our 5 events)
        assert_eq!(received_events.len(), 3);
        assert_eq!(received_events[0].created_at, events[4].created_at);
        assert_eq!(received_events[1].created_at, events[3].created_at);
        assert_eq!(received_events[2].created_at, events[2].created_at);

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_metadata_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a metadata event
        let keys = Keys::generate();
        let mut metadata = Metadata::new();
        metadata.name = Some("Test User".to_string());
        let metadata_event =
            EventBuilder::metadata(&metadata).build_with_ctx(&Instant::now(), keys.public_key());
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
}
