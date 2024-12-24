use crate::error::Error;
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
        connection: &EventStoreConnection,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<(), Error> {
        // Fetch historical events from the database directly
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
impl Middleware for EventStoreMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
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
                    if let Err(e) = connection.handle_event(*event.clone()).await {
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

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    async fn on_connect<'a>(
        &'a self,
        ctx: &mut ConnectionContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
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
    use std::{net::SocketAddr, time::Instant};
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use tokio_util::sync::CancellationToken;
    use websocket_builder::{StateFactory, WebSocketBuilder, WebSocketHandler};

    struct TestStateFactory {
        database: Arc<NostrDatabase>,
    }

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

        let ws_handler = WebSocketBuilder::new(
            TestStateFactory {
                database: database.clone(),
            },
            NostrMessageConverter,
        )
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

        async fn expect_closed(&mut self, subscription_id: &SubscriptionId) {
            match self.expect_message().await {
                RelayMessage::Closed {
                    subscription_id: sub_id,
                    ..
                } => {
                    assert_eq!(
                        sub_id, *subscription_id,
                        "Closed message subscription ID mismatch"
                    );
                }
                msg => panic!(
                    "Expected Closed message for subscription {}, got: {:?}",
                    subscription_id, msg
                ),
            }
        }

        async fn close(mut self) {
            self.write.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_subscription_and_broadcast() {
        let (_tmp_dir, database) = setup_test().await;

        // Create and save a historical event
        let (_, historical_event) = create_signed_event("Historical event").await;
        database.save_signed_event(&historical_event).unwrap();

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

        // Verify OK and broadcast
        publisher.expect_ok(&event.id).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        subscriber.expect_event(&subscription_id, &event).await;

        // Clean up
        subscriber.close().await;
        publisher.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_all_events() {
        let (_tmp_dir, database) = setup_test().await;

        // Create events with different kinds and authors
        let (keys1, text_note) = create_signed_event("Text note event").await;
        let (keys2, _) = create_signed_event("unused").await; // Just to get different keys

        // Create a metadata event
        let metadata_event = EventBuilder::new(Kind::Metadata, "{}")
            .build_with_ctx(&Instant::now(), keys2.public_key());
        let metadata_event = keys2.sign_event(metadata_event).await.unwrap();

        // Create a contacts event
        let contacts_event = EventBuilder::new(Kind::ContactList, "[]")
            .build_with_ctx(&Instant::now(), keys1.public_key());
        let contacts_event = keys1.sign_event(contacts_event).await.unwrap();

        // Save all events
        database.save_signed_event(&text_note).unwrap();
        database.save_signed_event(&metadata_event).unwrap();
        database.save_signed_event(&contacts_event).unwrap();

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

        // We should receive all events in any order
        let mut received_events = Vec::new();
        for _ in 0..3 {
            match client.expect_message().await {
                RelayMessage::Event { event, .. } => received_events.push(*event),
                msg => panic!("Expected Event message, got: {:?}", msg),
            }
        }

        // Verify EOSE after all events
        client.expect_eose(&subscription_id).await;

        // Verify we got all events
        assert!(
            received_events.iter().any(|e| e.id == text_note.id),
            "Text note event not found in response"
        );
        assert!(
            received_events.iter().any(|e| e.id == metadata_event.id),
            "Metadata event not found in response"
        );
        assert!(
            received_events.iter().any(|e| e.id == contacts_event.id),
            "Contacts event not found in response"
        );

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

        // Verify we got different kinds
        assert!(
            received_events.iter().any(|e| e.kind == Kind::TextNote),
            "No text note events"
        );
        assert!(
            received_events.iter().any(|e| e.kind == Kind::Metadata),
            "No metadata events"
        );
        assert!(
            received_events.iter().any(|e| e.kind == Kind::ContactList),
            "No contact list events"
        );

        // Send Close message and verify we get a Closed response
        client
            .send_message(&ClientMessage::Close(subscription_id.clone()))
            .await;
        client.expect_closed(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }
}
