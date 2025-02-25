use crate::error::Error;
use crate::groups::{
    Group, ADDRESSABLE_EVENT_KINDS, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022, NON_GROUP_ALLOWED_KINDS,
};
use crate::metrics;
use crate::nostr_database::RelayDatabase;
use crate::nostr_session_state::NostrConnectionState;
use crate::Groups;
use crate::StoreCommand;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tracing::error;
use websocket_builder::{
    ConnectionContext, InboundContext, Middleware, OutboundContext, SendMessage,
};

#[derive(Debug)]
pub struct Nip29Middleware {
    groups: Arc<Groups>,
    relay_pubkey: PublicKey,
    database: Arc<RelayDatabase>,
}

#[derive(Debug, thiserror::Error)]
pub enum MiddlewareError {
    #[error("Group not found: {0}")]
    GroupNotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Authentication required: {0}")]
    AuthRequired(String),
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl From<MiddlewareError> for Error {
    fn from(err: MiddlewareError) -> Self {
        match err {
            MiddlewareError::GroupNotFound(msg) => Error::notice(msg),
            MiddlewareError::PermissionDenied(msg) => Error::restricted(msg),
            MiddlewareError::AuthRequired(msg) => Error::auth_required(msg),
            MiddlewareError::Internal(err) => Error::notice(err.to_string()),
        }
    }
}

impl Nip29Middleware {
    pub fn new(groups: Arc<Groups>, relay_pubkey: PublicKey, database: Arc<RelayDatabase>) -> Self {
        Self {
            groups,
            relay_pubkey,
            database,
        }
    }

    /// Checks if a filter is querying group-related data
    fn is_group_query(&self, filter: &Filter) -> bool {
        filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::H))
            || filter
                .generic_tags
                .contains_key(&SingleLetterTag::lowercase(Alphabet::D))
    }

    /// Checks if a filter is querying addressable event kinds
    fn is_addressable_query(&self, filter: &Filter) -> bool {
        filter
            .kinds
            .as_ref()
            .is_some_and(|kinds| kinds.iter().any(|k| ADDRESSABLE_EVENT_KINDS.contains(k)))
    }

    /// Gets all group tags from a filter
    fn get_group_tags<'a>(&self, filter: &'a Filter) -> impl Iterator<Item = String> + 'a {
        filter
            .generic_tags
            .iter()
            .filter(|(k, _)| k == &&SingleLetterTag::lowercase(Alphabet::H))
            .flat_map(|(_, tag_set)| tag_set.iter())
            .cloned()
    }
    async fn handle_event(
        &self,
        event: &Event,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        if event.kind == KIND_GROUP_CREATE_9007 {
            return self.groups.handle_group_create(event).await;
        }

        // Allow events through for unmanaged groups (groups not in relay state)
        // Per NIP-29: In unmanaged groups, everyone is considered a member
        // These groups can later be converted to managed groups by the relay admin
        if event.tags.find(TagKind::h()).is_some()
            && !Group::is_group_management_kind(event.kind)
            && self.groups.find_group_from_event(event).is_none()
        {
            return Ok(vec![StoreCommand::SaveSignedEvent(event.clone())]);
        }

        let events_to_save = match event.kind {
            k if k == KIND_GROUP_EDIT_METADATA_9002 => self.groups.handle_edit_metadata(event)?,

            k if k == KIND_GROUP_USER_JOIN_REQUEST_9021 => {
                self.groups.handle_join_request(event)?
            }

            k if k == KIND_GROUP_USER_LEAVE_REQUEST_9022 => {
                self.groups.handle_leave_request(event)?
            }

            k if k == KIND_GROUP_SET_ROLES_9006 => self.groups.handle_set_roles(event)?,

            k if k == KIND_GROUP_ADD_USER_9000 => self.groups.handle_put_user(event)?,

            k if k == KIND_GROUP_REMOVE_USER_9001 => self.groups.handle_remove_user(event)?,

            k if k == KIND_GROUP_DELETE_9008 => {
                self.groups.handle_delete_group(event, authed_pubkey)?
            }

            k if k == KIND_GROUP_DELETE_EVENT_9005 => {
                self.groups.handle_delete_event(event, authed_pubkey)?
            }

            k if k == KIND_GROUP_CREATE_INVITE_9009 => self.groups.handle_create_invite(event)?,

            k if !NON_GROUP_ALLOWED_KINDS.contains(&k) => {
                self.groups.handle_group_content(event)?
            }

            _ => vec![StoreCommand::SaveSignedEvent(event.clone())],
        };

        Ok(events_to_save)
    }

    /// Verifies if a filter has access to the requested groups.
    ///
    /// The verification follows these rules:
    /// 1. Non-group queries (no 'h' or 'd' tags) are always allowed
    /// 2. Addressable event kinds (39xxx) are always allowed
    /// 3. For unmanaged groups (groups that don't exist in relay state):
    ///    - Access is always granted as everyone is considered a member
    ///    - Groups can form organically before becoming managed
    ///    - Can be converted to managed groups by relay admin later
    /// 4. For managed groups:
    ///    - Public groups are accessible to everyone
    ///    - Private groups require authentication and membership
    ///    - The relay pubkey always has access
    fn verify_filter(
        &self,
        authed_pubkey: Option<PublicKey>,
        filter: &Filter,
    ) -> Result<(), Error> {
        if !self.is_group_query(filter) {
            return Ok(());
        }
        if self.is_addressable_query(filter) {
            return Ok(());
        }

        for tag in self.get_group_tags(filter) {
            if let Some(group_ref) = self.groups.get_group(&tag) {
                self.groups.verify_group_access(&group_ref, authed_pubkey)?;
            }
        }
        Ok(())
    }

    /// Verifies filters and handles subscription requests for a given subscription ID.
    async fn handle_subscription(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
        authed_pubkey: Option<PublicKey>,
        connection: Option<&NostrConnectionState>,
    ) -> Result<(), Error> {
        for filter in &filters {
            self.verify_filter(authed_pubkey, filter)?;
        }

        let Some(conn) = connection else {
            error!(
                "No connection available for subscription {}",
                subscription_id
            );
            return Ok(());
        };

        let Some(relay_conn) = &conn.relay_connection else {
            error!(
                "No relay connection available for subscription {}",
                subscription_id
            );
            return Ok(());
        };

        relay_conn
            .handle_subscription_request(subscription_id.clone(), filters)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl Middleware for Nip29Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        match &ctx.message {
            ClientMessage::Event(event) => {
                metrics::inbound_events_processed().increment(1);
                match self.handle_event(event, &ctx.state.authed_pubkey).await {
                    Ok(events_to_save) => {
                        let event_id = event.id;
                        ctx.state.save_events(events_to_save).await?;
                        ctx.send_message(RelayMessage::ok(
                            event_id,
                            true,
                            "Event processed successfully",
                        ))
                        .await?;
                        Ok(())
                    }
                    Err(e) => Err(e.into()),
                }
            }
            ClientMessage::Req {
                subscription_id,
                filter,
            } => {
                self.handle_subscription(
                    subscription_id.clone(),
                    vec![filter.as_ref().clone()],
                    ctx.state.authed_pubkey,
                    Some(ctx.state),
                )
                .await?;
                Ok(())
            }
            ClientMessage::ReqMultiFilter {
                subscription_id,
                filters,
            } => {
                self.handle_subscription(
                    subscription_id.clone(),
                    filters.clone(),
                    ctx.state.authed_pubkey,
                    Some(ctx.state),
                )
                .await?;
                Ok(())
            }
            ClientMessage::Close(subscription_id) => {
                let subscription_id = subscription_id.clone();

                let Some(connection) = ctx.state.relay_connection.as_ref() else {
                    error!(
                        "No connection available for unsubscribing {}",
                        subscription_id
                    );
                    // Send CLOSED message even without connection
                    ctx.send_message(RelayMessage::closed(subscription_id.clone(), ""))
                        .await?;
                    return Ok(());
                };

                match connection.handle_unsubscribe(subscription_id.clone()).await {
                    Ok(()) => {
                        ctx.send_message(RelayMessage::closed(subscription_id.clone(), ""))
                            .await?;
                    }
                    Err(e) => {
                        error!("Failed to unsubscribe {}: {}", subscription_id, e);
                        return Err(e.into());
                    }
                }

                Ok(())
            }
            _ => ctx.next().await,
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(RelayMessage::Event { event, .. }) = &ctx.message else {
            return ctx.next().await;
        };

        let Some(group) = self.groups.find_group_from_event(event) else {
            return ctx.next().await;
        };

        let Ok(can_see) = group.can_see_event(&ctx.state.authed_pubkey, &self.relay_pubkey, event)
        else {
            ctx.message = None;
            return ctx.next().await;
        };

        if !can_see {
            ctx.message = None;
        }

        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(sender) = ctx.sender.clone() else {
            error!("No sender available for connection setup in Nip29Middleware");
            return Ok(());
        };

        ctx.state
            .setup_connection(ctx.connection_id.clone(), self.database.clone(), sender)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, create_test_keys, create_test_state, setup_test};
    use axum::{
        extract::{ConnectInfo, State, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::{SinkExt, StreamExt};
    use std::{net::SocketAddr, time::Duration};
    use tokio::net::TcpListener;
    use tokio::time::sleep;
    use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
    use tokio_util::sync::CancellationToken;
    use tracing::{debug, error, warn};
    use websocket_builder::{
        InboundContext, MessageConverter, MessageSender, OutboundContext, StateFactory,
        WebSocketBuilder, WebSocketHandler,
    };

    #[derive(Clone)]
    struct NostrMessageConverter;

    impl MessageConverter<ClientMessage, RelayMessage> for NostrMessageConverter {
        fn outbound_to_string(&self, message: RelayMessage) -> anyhow::Result<String> {
            debug!("Converting outbound message to string: {:?}", message);
            Ok(message.as_json())
        }

        fn inbound_from_string(&self, message: String) -> anyhow::Result<Option<ClientMessage>> {
            // Parse synchronously since JSON parsing doesn't need to be async
            if let Ok(client_message) = ClientMessage::from_json(&message) {
                debug!("Successfully parsed inbound message: {}", message);
                Ok(Some(client_message))
            } else {
                error!("Ignoring invalid inbound message: {}", message);
                Ok(None)
            }
        }
    }

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
                challenge: None,
                authed_pubkey: None,
                relay_url: RelayUrl::parse("ws://test.relay").expect("Invalid test relay URL"),
                relay_connection: None,
                connection_token: token.clone(),
                event_start_time: None,
                event_kind: None,
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

    async fn start_test_server(database: Arc<RelayDatabase>) -> (SocketAddr, CancellationToken) {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let cancellation_token = CancellationToken::new();
        let token = cancellation_token.clone();

        let ws_handler = WebSocketBuilder::new(TestStateFactory, NostrMessageConverter)
            .with_middleware(Nip29Middleware::new(
                Arc::new(
                    Groups::load_groups(database.clone(), Keys::generate().public_key())
                        .await
                        .unwrap(),
                ),
                Keys::generate().public_key(),
                database,
            ))
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

        async fn expect_closed(&mut self, subscription_id: &SubscriptionId) {
            debug!(
                target: "test_client",
                "Expecting CLOSED for subscription {}", subscription_id
            );
            match self.expect_message().await {
                RelayMessage::Closed {
                    subscription_id: sub_id,
                    ..
                } => {
                    assert_eq!(sub_id, *subscription_id, "CLOSED subscription ID mismatch");
                    debug!(
                        target: "test_client",
                        "Successfully received CLOSED for subscription {}", subscription_id
                    );
                }
                msg => panic!(
                    "Expected CLOSED message for subscription {}, got: {:?}",
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
    async fn test_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups, admin_keys.public_key(), database);

        // Create a content event for a non-existent group
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(TagKind::h(), ["non_existent_group"])],
        )
        .await;

        // Should allow the event through since it's an unmanaged group
        let result = middleware.handle_event(&event, &None).await;
        assert!(result.is_ok());
        if let Ok(commands) = result {
            assert_eq!(commands.len(), 1);
            match &commands[0] {
                StoreCommand::SaveSignedEvent(saved_event) => assert_eq!(saved_event.id, event.id),
                _ => panic!("Expected SaveSignedEvent command"),
            }
        }
    }

    #[tokio::test]
    async fn test_allowed_non_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups, admin_keys.public_key(), database);

        // Create a content event for a non-existent group
        let event = create_test_event(
            &member_keys,
            10009, // This event doesn't need an 'h' tag
            vec![],
        )
        .await;

        // Should not return an error because group is not needed here
        let result = middleware.handle_event(&event, &None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_outbound_visibility_member_can_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        // Create a group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Add member to group
        let add_member_event = create_test_event(
            &admin_keys,
            9008,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(&add_member_event).unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Test member can see event
        let mut state = create_test_state(Some(member_keys.public_key()));
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: SubscriptionId::new("test"),
                event: Box::new(content_event),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_some());
    }

    #[tokio::test]
    async fn test_process_outbound_visibility_non_member_cannot_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        // Create a group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Test non-member cannot see event
        let mut state = create_test_state(Some(non_member_keys.public_key()));
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: SubscriptionId::new("test"),
                event: Box::new(content_event),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_none());
    }

    #[tokio::test]
    async fn test_process_outbound_visibility_relay_can_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        // Create a group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Test relay pubkey can see event
        let mut state = create_test_state(Some(admin_keys.public_key()));
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: SubscriptionId::new("test"),
                event: Box::new(content_event),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_some());
    }

    #[tokio::test]
    async fn test_filter_verification_normal_filter_with_h_tag() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let normal_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), "test_group");
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &normal_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_d_tag() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let meta_filter = Filter::new()
            .kind(Kind::Custom(9007))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group");
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &meta_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_reference_filter_with_e_tag() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let ref_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::E), "test_id");
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &ref_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_non_group_query() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        // Create a filter with no 'h' tag but with other tags
        let non_group_filter = Filter::new()
            .kind(Kind::Custom(443))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::T), "test-tag");

        // Should pass verification since it's not a group query
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &non_group_filter)
            .is_ok());

        // Same filter but with multiple tags should also work
        let multi_tag_filter = Filter::new()
            .kind(Kind::Custom(443))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::T), "test-tag-1")
            .custom_tag(SingleLetterTag::lowercase(Alphabet::T), "test-tag-2");
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &multi_tag_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_non_existing_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let non_existing_group_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            "non_existing_group",
        );
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &non_existing_group_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_addressable_kind() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        // Create a test group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        let meta_filter = Filter::new()
            .kinds(vec![Kind::Custom(39000)]) // Just the addressable kind
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &meta_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_member_access() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let private_group_id = "private_group";
        let private_create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::custom(TagKind::p(), ["true"]),
            ],
        )
        .await;
        groups
            .handle_group_create(&private_create_event)
            .await
            .unwrap();

        let add_to_private_event = create_test_event(
            &admin_keys,
            9008,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(&add_to_private_event).unwrap();

        let private_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), private_group_id);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &private_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_non_member_access() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, _, non_member_keys) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let private_group_id = "private_group";
        let private_create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::custom(TagKind::p(), ["true"]),
            ],
        )
        .await;
        groups
            .handle_group_create(&private_create_event)
            .await
            .unwrap();

        let private_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), private_group_id);
        assert!(middleware
            .verify_filter(Some(non_member_keys.public_key()), &private_filter)
            .is_err());
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_no_auth() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let private_group_id = "private_group";
        let private_create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::custom(TagKind::p(), ["true"]),
            ],
        )
        .await;
        groups
            .handle_group_create(&private_create_event)
            .await
            .unwrap();

        let private_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), private_group_id);
        assert!(middleware.verify_filter(None, &private_filter).is_err());
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_relay_access() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        let private_group_id = "private_group";
        let private_create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::custom(TagKind::p(), ["true"]),
            ],
        )
        .await;
        groups
            .handle_group_create(&private_create_event)
            .await
            .unwrap();

        let private_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), private_group_id);
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &private_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_group_not_found_single_ok_message() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups, admin_keys.public_key(), database.clone());

        // Create a content event for a non-existent group
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(TagKind::h(), ["non_existent_group"])],
        )
        .await;

        // Create a test context with a connection
        let mut state = create_test_state(None);
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        state
            .setup_connection("test_conn".to_string(), database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Event(Box::new(event.clone())),
            None,
            &mut state,
            &[],
            0,
        );

        // Process the event - should succeed since it's an unmanaged group
        let result = middleware.process_inbound(&mut ctx).await;
        assert!(result.is_ok());

        // Verify that no OK message was sent since we're letting EventStoreMiddleware handle it
        assert!(ctx.sender.is_none() || ctx.capacity() == 0);
    }

    #[tokio::test]
    async fn test_close_subscription() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Start the test server
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect client
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Send CLOSE message
        let subscription_id = SubscriptionId::new("test_sub");
        client
            .send_message(&ClientMessage::Close(subscription_id.clone()))
            .await;

        // Verify we receive a CLOSED message
        client.expect_closed(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_close_active_subscription() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;

        // Start the test server
        let (addr, token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect client
        let url = format!("ws://{}", addr);
        let mut client = TestClient::connect(&url).await;

        // Create a subscription first
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            admin_keys.public_key().to_string(),
        );

        client
            .send_message(&ClientMessage::Req {
                subscription_id: subscription_id.clone(),
                filter: Box::new(filter),
            })
            .await;

        // Wait for EOSE since there are no historical events
        match client.expect_message().await {
            RelayMessage::EndOfStoredEvents(sub_id) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            msg => panic!("Expected EOSE message, got: {:?}", msg),
        }

        // Now close the subscription
        client
            .send_message(&ClientMessage::Close(subscription_id.clone()))
            .await;

        // Verify we receive a CLOSED message
        client.expect_closed(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }

    fn create_test_context(
        state: &mut NostrConnectionState,
        message: RelayMessage,
    ) -> OutboundContext<'_, NostrConnectionState, ClientMessage, RelayMessage> {
        OutboundContext::new("test_conn".to_string(), message, None, state, &[], 0)
    }

    #[tokio::test]
    async fn test_group_create_with_existing_events_requires_relay_admin() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware =
            Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database.clone());

        // First create an unmanaged event for a group
        let group_id = "test_group";
        let unmanaged_event = create_test_event(
            &member_keys,
            11, // Regular content event
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Save the unmanaged event
        database
            .save_signed_event(unmanaged_event.clone())
            .await
            .unwrap();

        sleep(Duration::from_millis(30)).await;

        // Try to create a managed group with non-admin key - should fail
        let create_event_non_admin = create_test_event(
            &member_keys,
            9007, // KIND_GROUP_CREATE_9007
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        let result = middleware
            .handle_event(&create_event_non_admin, &None)
            .await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Only relay admin can create a managed group from an unmanaged one"
        );

        // Try to create a managed group with relay admin key - should succeed
        let create_event_admin = create_test_event(
            &admin_keys,
            9007, // KIND_GROUP_CREATE_9007
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        let result = middleware.handle_event(&create_event_admin, &None).await;
        assert!(result.is_ok());
        if let Ok(commands) = result {
            // Should have 6 commands: save create event + 5 metadata events
            assert_eq!(commands.len(), 6);
            match &commands[0] {
                StoreCommand::SaveSignedEvent(saved_event) => {
                    assert_eq!(saved_event.id, create_event_admin.id)
                }
                _ => panic!("Expected SaveSignedEvent command"),
            }
        }

        // Verify the group was created and is managed
        let group = groups.get_group(group_id).unwrap();
        assert!(group.is_admin(&admin_keys.public_key()));
    }

    #[tokio::test]
    async fn test_filter_verification_p_tag_without_reference_tags() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database);

        // Create a filter with just a 'p' tag and kind
        let p_tag_filter = Filter::new().kind(Kind::Custom(1059)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            admin_keys.public_key().to_string(),
        );

        // Should pass verification since 'p' tags don't need reference tags
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &p_tag_filter)
            .is_ok());

        // Same filter but with multiple 'p' tags should also work
        let multi_p_filter = Filter::new().kind(Kind::Custom(1059)).custom_tags(
            SingleLetterTag::lowercase(Alphabet::P),
            vec![
                admin_keys.public_key().to_string(),
                "another_pubkey".to_string(),
            ],
        );
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &multi_p_filter)
            .is_ok());

        // Filter with 'p' tag and other non-reference tags should also work
        let mixed_filter = Filter::new()
            .kind(Kind::Custom(1059))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::P),
                admin_keys.public_key().to_string(),
            )
            .custom_tag(SingleLetterTag::lowercase(Alphabet::T), "test");
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &mixed_filter)
            .is_ok());
    }
}
