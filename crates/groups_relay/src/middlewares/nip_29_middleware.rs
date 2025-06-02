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
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tracing::{debug, error};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext, SendMessage,
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
        event: Box<Event>,
        authed_pubkey: &Option<PublicKey>,
        subdomain: Scope,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Allow events through for unmanaged groups (groups not in relay state)
        // Per NIP-29: In unmanaged groups, everyone is considered a member
        // These groups can later be converted to managed groups by the relay admin
        if event.tags.find(TagKind::h()).is_some()
            && !Group::is_group_management_kind(event.kind)
            && self
                .groups
                .find_group_from_event(&event, &subdomain)
                .is_none()
        {
            debug!(target: "nip29", "Processing unmanaged group event: kind={}, id={}", event.kind, event.id);
            return Ok(vec![StoreCommand::SaveSignedEvent(event, subdomain)]);
        }

        let events_to_save = match event.kind {
            k if k == KIND_GROUP_CREATE_9007 => {
                debug!(target: "nip29", "Processing group create event: id={}", event.id);
                self.groups.handle_group_create(event, &subdomain).await?
            }

            k if k == KIND_GROUP_EDIT_METADATA_9002 => {
                debug!(target: "nip29", "Processing group edit metadata event: id={}", event.id);
                self.groups.handle_edit_metadata(event, &subdomain)?
            }

            k if k == KIND_GROUP_USER_JOIN_REQUEST_9021 => {
                debug!(target: "nip29", "Processing group join request: id={}", event.id);
                self.groups.handle_join_request(event, &subdomain)?
            }

            k if k == KIND_GROUP_USER_LEAVE_REQUEST_9022 => {
                debug!(target: "nip29", "Processing group leave request: id={}", event.id);
                self.groups.handle_leave_request(event, &subdomain)?
            }

            k if k == KIND_GROUP_SET_ROLES_9006 => {
                debug!(target: "nip29", "Processing group set roles event: id={}", event.id);
                self.groups.handle_set_roles(event, &subdomain)?
            }

            k if k == KIND_GROUP_ADD_USER_9000 => {
                debug!(target: "nip29", "Processing group add user event: id={}", event.id);
                self.groups.handle_put_user(event, &subdomain)?
            }

            k if k == KIND_GROUP_REMOVE_USER_9001 => {
                debug!(target: "nip29", "Processing group remove user event: id={}", event.id);
                self.groups.handle_remove_user(event, &subdomain)?
            }

            k if k == KIND_GROUP_DELETE_9008 => {
                debug!(target: "nip29", "Processing group deletion event: id={}", event.id);
                self.groups
                    .handle_delete_group(event, authed_pubkey, &subdomain)?
            }

            k if k == KIND_GROUP_DELETE_EVENT_9005 => {
                debug!(target: "nip29", "Processing group content event deletion: id={}", event.id);
                self.groups
                    .handle_delete_event(event, authed_pubkey, &subdomain)?
            }

            k if k == KIND_GROUP_CREATE_INVITE_9009 => {
                debug!(target: "nip29", "Processing group create invite event: id={}", event.id);
                self.groups.handle_create_invite(event, &subdomain)?
            }

            k if !NON_GROUP_ALLOWED_KINDS.contains(&k) => {
                debug!(target: "nip29", "Processing group content event: kind={}, id={}", event.kind, event.id);
                self.groups.handle_group_content(event, &subdomain)?
            }

            _ => {
                debug!(target: "nip29", "Processing non-group event: kind={}, id={}", event.kind, event.id);
                vec![StoreCommand::SaveSignedEvent(event, subdomain)]
            }
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

        // Use the default scope for filters - in production the scope comes from the subdomain
        let scope = Scope::Default;

        for tag in self.get_group_tags(filter) {
            if let Some(group_ref) = self.groups.get_group(&scope, &tag) {
                self.groups
                    .verify_group_access(group_ref.value(), authed_pubkey)?;
            }
        }
        Ok(())
    }

    /// Verifies filters and handles subscription requests with fill-buffer pagination.
    async fn handle_subscription(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
        authed_pubkey: Option<PublicKey>,
        connection_state: Option<&NostrConnectionState>,
    ) -> Result<(), Error> {
        // First verify all filters
        for filter in &filters {
            self.verify_filter(authed_pubkey, filter)?;
        }

        // Create the visibility checker closure
        let groups = Arc::clone(&self.groups);
        let subdomain = connection_state
            .map(|cs| cs.subdomain().clone())
            .unwrap_or(Scope::Default);

        let visibility_checker = move |event: &Event,
                                       authed_pubkey: &Option<PublicKey>,
                                       relay_pk: &PublicKey|
              -> Result<bool, Error> {
            // Check if this is a group event
            if let Some(group_ref) = groups.find_group_from_event(event, &subdomain) {
                // Group event - check access control using the group's can_see_event method
                group_ref
                    .value()
                    .can_see_event(authed_pubkey, relay_pk, event)
                    .map_err(|e| Error::internal(format!("Group access check failed: {}", e)))
            } else {
                // Not a group event or unmanaged group - allow it through
                Ok(true)
            }
        };

        // Delegate to the extracted subscription handler with the closure
        super::subscription_handler::handle_subscription(
            visibility_checker,
            &self.relay_pubkey,
            subscription_id,
            filters,
            authed_pubkey,
            connection_state,
        )
        .await
    }
}

#[async_trait]
impl Middleware for Nip29Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(client_message) = ctx.message.take() else {
            // No message, nothing to do
            return Ok(());
        };

        match client_message {
            ClientMessage::Event(event_cow) => {
                metrics::inbound_events_processed().increment(1);
                let original_event_id = event_cow.as_ref().id; // Get ID before moving
                let subdomain = ctx.state.subdomain().clone();
                match self
                    .handle_event(
                        Box::new(event_cow.into_owned()),
                        &ctx.state.authed_pubkey,
                        subdomain,
                    )
                    .await
                {
                    Ok(commands) => {
                        // Use save_and_broadcast to properly handle replaceable events and broadcast to subscriptions
                        if let Some(subscription_manager) = &ctx.state.subscription_manager {
                            for command in commands {
                                subscription_manager.save_and_broadcast(command).await?;
                            }
                        } else {
                            // This should not happen - subscription manager should always be available
                            error!(target: "nip29", "No subscription manager available for connection");
                            return Err(Error::internal("No subscription manager available").into());
                        }
                        // If all saves were successful, send OK
                        if let Some(_sender) = ctx.sender.as_mut() {
                            // sender variable is not used
                            ctx.send_message(RelayMessage::ok(
                                original_event_id,
                                true,
                                "Event processed successfully",
                            ))?;
                        }
                    }
                    Err(e) => {
                        if ctx.sender.is_some() {
                            let notice_msg = format!("Error processing event: {}", e);
                            ctx.send_message(RelayMessage::notice(notice_msg))?;
                        }
                        error!(target: "nip29", "Error handling inbound event: {:?}", e);
                        return Err(e.into());
                    }
                }
            }
            ClientMessage::Req {
                subscription_id,
                filter,
            } => {
                match self
                    .handle_subscription(
                        subscription_id.into_owned(),
                        vec![filter.into_owned()],
                        ctx.state.authed_pubkey,
                        Some(ctx.state),
                    )
                    .await
                {
                    Ok(_) => {
                        // EOSE / Stored events are handled by NostrConnectionState/SubscriptionManager
                    }
                    Err(e) => {
                        if ctx.sender.is_some() {
                            let notice_msg = format!("Error processing REQ: {}", e);
                            ctx.send_message(RelayMessage::notice(notice_msg))?;
                        }
                        error!(target: "nip29", "Error handling REQ: {:?}", e);
                        return Err(e.into());
                    }
                }
            }
            ClientMessage::Close(sub_id_cow) => {
                if let Some(conn_state) = ctx.state.subscription_manager.as_ref() {
                    if let Err(e) = conn_state.remove_subscription(sub_id_cow.as_ref().clone()) {
                        error!(target: "nip29", "Error closing subscription {}: {:?}", sub_id_cow, e);
                        if ctx.sender.is_some() {
                            let notice_msg =
                                format!("Error processing CLOSE for {}: {}", sub_id_cow, e);
                            ctx.send_message(RelayMessage::notice(notice_msg))?;
                        }
                    } else {
                        debug!(target: "nip29", "Successfully closed subscription: {}", sub_id_cow);
                        // NIP-01: A relay MAY send a CLOSED message to confirm that a CLOSE message has been processed.
                        // Not strictly required by NIP-29, but good practice.
                        if ctx.sender.is_some() {
                            ctx.send_message(RelayMessage::closed(
                                sub_id_cow.into_owned(),
                                "Subscription closed by client request",
                            ))?;
                        }
                    }
                } else {
                    error!(target: "nip29", "No subscription manager found for CLOSE on conn: {}", ctx.connection_id);
                    if ctx.sender.is_some() {
                        ctx.send_message(RelayMessage::notice(format!(
                            "Error: No subscription manager to process CLOSE for {}",
                            sub_id_cow
                        )))?;
                    }
                }
            }
            ClientMessage::Auth(auth_event_cow) => {
                // NIP-29 does not explicitly define AUTH handling related to groups.
                // Typically, NIP-42 (AuthMiddleware) would handle this.
                // For now, acknowledge with OK as per general relay behavior if not handled by another middleware.
                if ctx.sender.is_some() {
                    ctx.send_message(RelayMessage::ok(
                        auth_event_cow.as_ref().id,
                        true,
                        "AUTH received",
                    ))?;
                }
                return Ok(());
            }
            ClientMessage::NegOpen { .. }
            | ClientMessage::NegClose { .. }
            | ClientMessage::NegMsg { .. } => {
                // NIP-45: Negotiation. Not handled by NIP-29, pass through.
                debug!(target: "nip29", "Passing through NIP-45 message: {:?}", client_message);
                // To pass through, we need to reconstruct the context's message and call next.
                // However, we took the message, so we'd need to put it back.
                // For now, as Nip29Middleware is often one of the last, we'll assume no further processing needed.
                // If it were earlier in a chain, proper pass-through would require putting it back or cloning.
                return Ok(());
            }
            ClientMessage::ReqMultiFilter {
                subscription_id,
                filters,
            } => {
                debug!(target: "nip29", "Received ReqMultiFilter for {}. NIP-29 typically handles single filter REQ. Passing through/ignoring.", subscription_id);
                // Similar to REQ, but with multiple filters. NIP-29 doesn't explicitly cover this.
                // For now, we can try to handle it like a single REQ if group logic applies broadly,
                // or ignore if group filtering is per-subscription based on the *first* filter.
                // Let's attempt to handle it similarly to REQ for now, using all filters.
                match self
                    .handle_subscription(
                        subscription_id.into_owned(),
                        filters.into_iter().map(|f| f.to_owned()).collect(),
                        ctx.state.authed_pubkey,
                        Some(ctx.state),
                    )
                    .await
                {
                    Ok(_) => {
                        // EOSE / Stored events are handled by NostrConnectionState/SubscriptionManager
                    }
                    Err(e) => {
                        if ctx.sender.is_some() {
                            let notice_msg = format!("Error processing ReqMultiFilter: {}", e);
                            ctx.send_message(RelayMessage::notice(notice_msg))?;
                        }
                        error!(target: "nip29", "Error handling ReqMultiFilter: {:?}", e);
                    }
                }
                return Ok(());
            }
            ClientMessage::Count {
                subscription_id,
                filter: _,
            } => {
                debug!(target: "nip29", "Received Count for {}. NIP-29 does not specify COUNT handling. Ignoring.", subscription_id);
                // NIP-45 COUNT. NIP-29 does not define behavior for this.
                // A relay might send a COUNT reply, but NIP29Middleware doesn't have group-specific logic for it.
                return Ok(());
            }
        }
        Ok(())
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        // Note: Historical events from REQ handlers (subscription_handler) don't go through
        // this process_outbound hook. They are sent directly using send_bypass() after
        // already applying visibility filtering during pagination processing. This hook
        // primarily handles broadcasted events from subscription_manager that need filtering.
        let Some(RelayMessage::Event { event, .. }) = &ctx.message else {
            return ctx.next().await;
        };

        let Some(group) = self
            .groups
            .find_group_from_event(event, ctx.state.subdomain())
        else {
            return ctx.next().await;
        };

        let Ok(can_see) =
            group
                .value()
                .can_see_event(&ctx.state.authed_pubkey, &self.relay_pubkey, event)
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
            return Err(Error::internal("No sender available for connection setup").into());
        };

        ctx.state
            .setup_connection(self.database.clone(), sender)
            .await?;

        Ok(())
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(connection) = &ctx.state.subscription_manager {
            debug!("Proactively cleaning up connection in on_disconnect");
            connection.cleanup();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, create_test_keys, setup_test};
    use axum::{
        extract::{ConnectInfo, State, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::{SinkExt, StreamExt};
    use std::borrow::Cow;
    use std::{net::SocketAddr, time::Duration};
    use tokio::net::TcpListener;
    use tokio::time::sleep;
    use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
    use tokio_util::sync::CancellationToken;
    use tracing::{debug, error, warn};
    use websocket_builder::{
        MessageConverter, OutboundContext as TestOutboundContext, StateFactory, WebSocketBuilder,
        WebSocketHandler,
    };

    #[derive(Clone)]
    struct NostrMessageConverter;

    impl MessageConverter<ClientMessage<'static>, RelayMessage<'static>> for NostrMessageConverter {
        fn outbound_to_string(&self, message: RelayMessage<'static>) -> anyhow::Result<String> {
            debug!("Converting outbound message to string: {:?}", message);
            Ok(message.as_json())
        }

        fn inbound_from_string(
            &self,
            message: String,
        ) -> anyhow::Result<Option<ClientMessage<'static>>> {
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
                subscription_manager: None,
                connection_token: token.clone(),
                event_start_time: None,
                event_kind: None,
                subdomain: Scope::Default,
            }
        }
    }

    struct ServerState {
        ws_handler: WebSocketHandler<
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
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
            .with_channel_size(1000) // Match production settings
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

        async fn send_message(&mut self, msg: &ClientMessage<'static>) {
            let message = Message::Text(msg.as_json().into());
            debug!(target: "test_client", "Sending message: {:?}", message);
            self.write.send(message).await.unwrap();
        }

        async fn expect_message(&mut self) -> RelayMessage<'static> {
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
                    assert_eq!(
                        sub_id.as_ref(),
                        subscription_id,
                        "CLOSED subscription ID mismatch"
                    );
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
        let event_id = event.id;
        let result = middleware
            .handle_event(Box::new(event), &None, Scope::Default)
            .await;
        assert!(result.is_ok());
        if let Ok(commands) = result {
            assert_eq!(commands.len(), 1);
            match &commands[0] {
                StoreCommand::SaveSignedEvent(saved_event, _) => {
                    assert_eq!(saved_event.id, event_id)
                }
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
        let result = middleware
            .handle_event(Box::new(event), &None, Scope::Default)
            .await;
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
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;
        groups
            .handle_group_create(Box::new(create_event), &Scope::Default)
            .await
            .unwrap();

        // Add member to group
        let add_member_event = create_test_event(
            &admin_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id.to_string()]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups
            .handle_put_user(Box::new(add_member_event), &Scope::Default)
            .unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;

        // Test member can see event
        let mut state = NostrConnectionState::new("ws://test".to_string()).unwrap();
        state.authed_pubkey = Some(member_keys.public_key());
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: Cow::Owned(SubscriptionId::new("test")),
                event: Cow::Owned(content_event),
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
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;
        groups
            .handle_group_create(Box::new(create_event), &Scope::Default)
            .await
            .unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;

        // Test non-member cannot see event
        let mut state = NostrConnectionState::new("ws://test".to_string()).unwrap();
        state.authed_pubkey = Some(non_member_keys.public_key());
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: Cow::Owned(SubscriptionId::new("test")),
                event: Cow::Owned(content_event),
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
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;
        groups
            .handle_group_create(Box::new(create_event), &Scope::Default)
            .await
            .unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;

        // Test relay pubkey can see event
        let mut state = NostrConnectionState::new("ws://test".to_string()).unwrap();
        state.authed_pubkey = Some(admin_keys.public_key());
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: Cow::Owned(SubscriptionId::new("test")),
                event: Cow::Owned(content_event),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_some());
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
        let _middleware = Nip29Middleware::new(groups, admin_keys.public_key(), database.clone());

        // Create a content event for a non-existent group
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(
                TagKind::h(),
                ["non_existent_group".to_string()],
            )],
        )
        .await;

        // Start the test server
        let (addr, shutdown_token) = start_test_server(database).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let mut client = TestClient::connect(&format!("ws://{}", addr)).await;

        // Send an event for a group that doesn't exist
        client
            .send_message(&ClientMessage::Event(Cow::Owned(event.clone())))
            .await;

        // Expect an OK message because the event is for a non-existent (unmanaged) group
        // and should be allowed through.
        match client.expect_message().await {
            RelayMessage::Ok {
                event_id,
                status,
                message: _,
            } => {
                assert_eq!(event_id, event.id);
                assert!(status); // true for success
            }
            other => panic!("Expected OK message, got {:?}", other),
        }

        client.close().await;
        shutdown_token.cancel();
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
            .send_message(&ClientMessage::Close(Cow::Owned(subscription_id.clone())))
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
                subscription_id: Cow::Owned(subscription_id.clone()),
                filter: Cow::Owned(filter),
            })
            .await;

        // Wait for EOSE since there are no historical events
        match client.expect_message().await {
            RelayMessage::EndOfStoredEvents(sub_id) => {
                assert_eq!(
                    sub_id.as_ref(),
                    &subscription_id,
                    "EOSE subscription ID mismatch"
                );
            }
            msg => panic!("Expected EOSE message, got: {:?}", msg),
        }

        // Now close the subscription
        client
            .send_message(&ClientMessage::Close(Cow::Owned(subscription_id.clone())))
            .await;

        // Verify we receive a CLOSED message
        client.expect_closed(&subscription_id).await;

        // Clean up
        client.close().await;
        token.cancel();
    }

    fn create_test_context<'a>(
        state: &'a mut NostrConnectionState,
        message: RelayMessage<'static>,
    ) -> TestOutboundContext<'a, NostrConnectionState, ClientMessage<'static>, RelayMessage<'static>>
    {
        TestOutboundContext::new(
            "test_conn".to_string(),
            message,
            None,
            state,
            &[] as &[Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage<'static>,
                    OutgoingMessage = RelayMessage<'static>,
                >,
            >],
            0,
        )
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
            .save_signed_event(unmanaged_event.clone(), Scope::Default)
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
            .handle_event(Box::new(create_event_non_admin), &None, Scope::Default)
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

        let event_id = create_event_admin.id;
        let result = middleware
            .handle_event(Box::new(create_event_admin), &None, Scope::Default)
            .await;
        assert!(result.is_ok());
        if let Ok(commands) = result {
            // Should have 6 commands: save create event + 5 metadata events
            assert_eq!(commands.len(), 6);
            match &commands[0] {
                StoreCommand::SaveSignedEvent(saved_event, _) => {
                    assert_eq!(saved_event.id, event_id)
                }
                _ => panic!("Expected SaveSignedEvent command"),
            }
        }

        // Verify the group was created and is managed
        let scope = Scope::Default; // In tests we use the default scope
        let group = groups.get_group(&scope, group_id).unwrap();
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

        let normal_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            "test_group".to_string(),
        );
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
            .kind(Kind::Custom(9007)) // KIND_GROUP_CREATE_9007
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                "test_group".to_string(),
            );
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
            .kind(Kind::Custom(11)) // Any kind
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::E),
                "test_id".to_string(),
            ); // 'e' tag for event reference
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

        let non_group_filter = Filter::new().kind(Kind::Custom(443)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::T),
            "test-tag".to_string(),
        );
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &non_group_filter)
            .is_ok());

        let multi_tag_filter = Filter::new()
            .kind(Kind::Custom(443))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::T),
                "test-tag-1".to_string(),
            )
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::T),
                "test-tag-2".to_string(),
            );
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
            "non_existing_group".to_string(),
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
        let middleware =
            Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database.clone()); // Ensure DB is cloned if Groups needs it

        let group_id = "test_group_addr_kind";
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id.to_string()])],
        )
        .await;
        groups
            .handle_group_create(Box::new(create_event), &Scope::Default)
            .await
            .unwrap();

        // Persist group creation related events if necessary for the group to be found
        // This might involve saving commands from handle_group_create to the database
        // For simplicity here, assume group is in memory. If DB interaction is key:
        // let commands = groups.handle_group_create(Box::new(create_event), &Scope::Default).await.unwrap();
        // for cmd in commands { database.save_store_command(cmd).await.unwrap(); }

        let meta_filter = Filter::new()
            .kinds(vec![Kind::Custom(39000)]) // Addressable kind
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                group_id.to_string(),
            ); // 'd' tag for group identifier
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
        let middleware =
            Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database.clone());

        let private_group_id = "private_group_member_access";
        let private_create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007.as_u16(),
            vec![
                Tag::custom(TagKind::h(), [private_group_id.to_string()]),
                Tag::custom(TagKind::Custom("private".into()), Vec::<String>::new()),
            ],
        )
        .await;
        // Ensure group is actually created and considered private by the Groups module.
        let commands = groups
            .handle_group_create(Box::new(private_create_event), &Scope::Default)
            .await
            .unwrap();
        for cmd in commands {
            middleware.database.save_store_command(cmd).await.unwrap();
        }

        let add_to_private_event = create_test_event(
            &admin_keys,
            KIND_GROUP_ADD_USER_9000.as_u16(),
            vec![
                Tag::custom(TagKind::h(), [private_group_id.to_string()]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        let commands_add = groups
            .handle_put_user(Box::new(add_to_private_event), &Scope::Default)
            .unwrap();
        for cmd in commands_add {
            middleware.database.save_store_command(cmd).await.unwrap();
        }

        let private_filter = Filter::new()
            .kind(Kind::Custom(11)) // Any kind for content
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::H),
                private_group_id.to_string(),
            );
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &private_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_non_member_access() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, _member_keys, non_member_keys) = create_test_keys().await; // Ensure non_member_keys is distinct
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware =
            Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database.clone());

        let private_group_id = "private_group_non_member";
        let private_create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007.as_u16(),
            vec![
                Tag::custom(TagKind::h(), [private_group_id.to_string()]),
                Tag::custom(TagKind::Custom("private".into()), Vec::<String>::new()),
            ],
        )
        .await;
        let commands = groups
            .handle_group_create(Box::new(private_create_event), &Scope::Default)
            .await
            .unwrap();
        for cmd in commands {
            middleware.database.save_store_command(cmd).await.unwrap();
        }

        let private_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            private_group_id.to_string(),
        );
        assert!(middleware
            .verify_filter(Some(non_member_keys.public_key()), &private_filter)
            .is_err()); // Non-member should not access
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_no_auth() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware =
            Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database.clone());

        let private_group_id = "private_group_no_auth";
        let private_create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007.as_u16(),
            vec![
                Tag::custom(TagKind::h(), [private_group_id.to_string()]),
                Tag::custom(TagKind::Custom("private".into()), Vec::<String>::new()),
            ],
        )
        .await;
        let commands = groups
            .handle_group_create(Box::new(private_create_event), &Scope::Default)
            .await
            .unwrap();
        for cmd in commands {
            middleware.database.save_store_command(cmd).await.unwrap();
        }

        let private_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            private_group_id.to_string(),
        );
        assert!(middleware
            .verify_filter(None, &private_filter) // No authenticated pubkey
            .is_err());
    }

    #[tokio::test]
    async fn test_filter_verification_private_group_relay_access() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware =
            Nip29Middleware::new(groups.clone(), admin_keys.public_key(), database.clone());

        let private_group_id = "private_group_relay_access";
        let private_create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007.as_u16(),
            vec![
                Tag::custom(TagKind::h(), [private_group_id.to_string()]),
                Tag::custom(TagKind::Custom("private".into()), Vec::<String>::new()),
            ],
        )
        .await;
        let commands = groups
            .handle_group_create(Box::new(private_create_event), &Scope::Default)
            .await
            .unwrap();
        for cmd in commands {
            middleware.database.save_store_command(cmd).await.unwrap();
        }

        let private_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            private_group_id.to_string(),
        );
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &private_filter) // Authenticated as relay admin
            .is_ok());
    }
}
