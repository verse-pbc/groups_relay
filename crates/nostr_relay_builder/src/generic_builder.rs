//! RelayBuilder for constructing Nostr relays with custom state

use crate::config::RelayConfig;
use crate::database::RelayDatabase;
use crate::error::Error;
use crate::logic::EventProcessor;
use crate::message_converter::NostrMessageConverter;
use crate::middleware::RelayMiddleware;
use crate::state::{GenericNostrConnectionFactory, NostrConnectionState};
use nostr_sdk::prelude::*;
use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use websocket_builder::{Middleware, WebSocketBuilder};

/// Builder for constructing Nostr relays with custom state.
///
/// This builder allows creating relays that maintain custom per-connection state
/// in addition to the standard framework state. This enables sophisticated features
/// like rate limiting, reputation systems, session tracking, and more.
///
/// # Type Parameters
/// - `T`: The custom state type. Must implement `Clone + Send + Sync + Debug + Default`.
///
/// # Example
/// ```rust,no_run
/// use nostr_relay_builder::{RelayBuilder, EventProcessor, EventContext, RelayConfig};
/// use nostr_sdk::prelude::*;
///
/// #[derive(Debug, Clone, Default)]
/// struct MyState {
///     request_count: u32,
/// }
///
/// # #[derive(Debug)]
/// # struct MyProcessor;
/// # #[async_trait::async_trait]
/// # impl EventProcessor<MyState> for MyProcessor {
/// #     async fn handle_event(
/// #         &self,
/// #         event: nostr_sdk::Event,
/// #         custom_state: &mut MyState,
/// #         context: EventContext<'_>,
/// #     ) -> Result<Vec<nostr_relay_builder::StoreCommand>, nostr_relay_builder::Error> {
/// #         Ok(vec![])
/// #     }
/// # }
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let keys = Keys::generate();
/// # let config = RelayConfig::new("ws://localhost:8080", "./data", keys);
/// let builder = RelayBuilder::<MyState>::new(config)
///     .with_state_factory(|| MyState::default());
///
/// let handler = builder.build_handler(MyProcessor).await?;
/// # Ok(())
/// # }
/// ```
pub struct RelayBuilder<T = ()> {
    config: RelayConfig,
    /// Middlewares to add to the relay
    middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState<T>,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    >,
    /// State factory for creating initial state for each connection
    state_factory: Option<Arc<dyn Fn() -> T + Send + Sync>>,
    /// Optional cancellation token for graceful shutdown
    cancellation_token: Option<CancellationToken>,
    /// Optional connection counter for metrics
    connection_counter: Option<Arc<AtomicUsize>>,
    _phantom: PhantomData<T>,
}

impl<T> RelayBuilder<T>
where
    T: Clone + Send + Sync + std::fmt::Debug + 'static,
{
    /// Create a new relay builder with the given configuration
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            middlewares: Vec::new(),
            state_factory: None,
            cancellation_token: None,
            connection_counter: None,
            _phantom: PhantomData,
        }
    }

    /// Set the state factory for creating initial custom state
    #[must_use]
    pub fn with_state_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        self.state_factory = Some(Arc::new(factory));
        self
    }

    /// Set a cancellation token for graceful shutdown
    #[must_use]
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    /// Set a connection counter for metrics
    #[must_use]
    pub fn with_connection_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
        self.connection_counter = Some(counter);
        self
    }

    /// Transform the builder to use a different state type
    pub fn with_custom_state<U>(self) -> RelayBuilder<U>
    where
        U: Clone + Send + Sync + std::fmt::Debug + 'static,
    {
        RelayBuilder {
            config: self.config,
            middlewares: Vec::new(),
            state_factory: None,
            cancellation_token: self.cancellation_token,
            connection_counter: self.connection_counter,
            _phantom: PhantomData,
        }
    }

    /// Add a middleware to the relay
    ///
    /// Middleware are executed in the order they are added for inbound messages,
    /// and in reverse order for outbound messages.
    #[must_use]
    pub fn with_middleware<M>(mut self, middleware: M) -> Self
    where
        M: Middleware<
                State = NostrConnectionState<T>,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            > + 'static,
    {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    /// Build the connection factory
    fn build_connection_factory(
        &self,
        database: Arc<RelayDatabase>,
    ) -> Result<GenericNostrConnectionFactory<T>, Error>
    where
        T: Default,
    {
        if let Some(ref state_factory) = self.state_factory {
            GenericNostrConnectionFactory::new_with_factory(
                self.config.relay_url.clone(),
                database,
                self.config.scope_config.clone(),
                state_factory.clone() as Arc<dyn Fn() -> T + Send + Sync>,
            )
        } else {
            GenericNostrConnectionFactory::new(
                self.config.relay_url.clone(),
                database,
                self.config.scope_config.clone(),
            )
        }
    }

    /// Build a WebSocket server with all configured middlewares
    pub async fn build_server<L: EventProcessor<T>>(
        mut self,
        processor: L,
    ) -> Result<RelayWebSocketHandler<T>, Error>
    where
        T: Default,
    {
        let websocket_config = self.config.websocket_config.clone();

        // Set the global query limit
        crate::global_config::set_query_limit(self.config.query_limit);

        let database = self.config.create_database()?;
        let custom_middlewares = std::mem::take(&mut self.middlewares);
        let connection_factory = self.build_connection_factory(database.clone())?;

        let relay_middleware =
            RelayMiddleware::new(processor, self.config.keys.public_key(), database);

        let mut builder = WebSocketBuilder::new(connection_factory, NostrMessageConverter);

        builder = builder.with_channel_size(websocket_config.channel_size);
        if let Some(max_connections) = websocket_config.max_connections {
            builder = builder.with_max_connections(max_connections);
        }
        if let Some(max_time) = websocket_config.max_connection_time {
            builder = builder.with_max_connection_time(Duration::from_secs(max_time));
        }

        // Add standard middlewares that should always be present
        builder = builder.with_middleware(crate::middlewares::LoggerMiddleware::new());
        builder = builder.with_middleware(crate::middlewares::ErrorHandlingMiddleware::new());

        // Add NIP-42 authentication middleware if enabled
        if self.config.enable_auth {
            let auth_config =
                self.config
                    .auth_config
                    .clone()
                    .unwrap_or_else(|| crate::middlewares::AuthConfig {
                        auth_url: self.config.relay_url.clone(),
                        base_domain_parts: 2,
                        validate_subdomains: matches!(
                            self.config.scope_config,
                            crate::config::ScopeConfig::Subdomain { .. }
                        ),
                    });
            builder =
                builder.with_middleware(crate::middlewares::Nip42Middleware::new(auth_config));
        }

        // Add event verification middleware
        builder = builder.with_middleware(crate::middlewares::EventVerifierMiddleware::new());

        // Add custom middlewares
        for middleware in custom_middlewares {
            builder = builder.with_arc_middleware(middleware);
        }

        // Relay middleware must be last to process messages after all validation
        builder = builder.with_middleware(relay_middleware);

        Ok(builder.build())
    }

    /// Alias for build_server - builds a WebSocket handler
    pub async fn build_handler<L: EventProcessor<T>>(
        self,
        processor: L,
    ) -> Result<RelayWebSocketHandler<T>, Error>
    where
        T: Default,
    {
        self.build_server(processor).await
    }

    /// Build handlers for framework integration
    #[cfg(feature = "axum")]
    pub async fn build_handlers<L: EventProcessor<T>>(
        self,
        processor: L,
        relay_info: crate::handlers::RelayInfo,
    ) -> Result<crate::handlers::RelayHandlers<T>, Error>
    where
        T: Default,
    {
        let cancellation_token = self.cancellation_token.clone();
        let connection_counter = self.connection_counter.clone();
        let scope_config = self.config.scope_config.clone();
        let handler = self.build_handler(processor).await?;
        Ok(crate::handlers::RelayHandlers::new(
            handler,
            relay_info,
            cancellation_token,
            connection_counter,
            scope_config,
        ))
    }

    /// Build handlers with an Axum root handler
    #[cfg(feature = "axum")]
    pub async fn build_axum_handler<L: EventProcessor<T>>(
        self,
        processor: L,
        relay_info: crate::handlers::RelayInfo,
    ) -> Result<
        impl Fn(
                Option<axum::extract::ws::WebSocketUpgrade>,
                axum::extract::ConnectInfo<std::net::SocketAddr>,
                axum::http::HeaderMap,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = axum::response::Response> + Send>,
            > + Clone
            + Send
            + 'static,
        Error,
    >
    where
        T: Default,
    {
        let handlers = Arc::new(self.build_handlers(processor, relay_info).await?);
        Ok(handlers.axum_root_handler())
    }
}

/// Type alias for the complete WebSocket handler type used by the relay
pub type RelayWebSocketHandler<T> = websocket_builder::WebSocketHandler<
    NostrConnectionState<T>,
    ClientMessage<'static>,
    RelayMessage<'static>,
    NostrMessageConverter,
    GenericNostrConnectionFactory<T>,
>;

/// Type alias for the default relay handler (with unit state for backward compatibility)
pub type DefaultRelayWebSocketHandler = RelayWebSocketHandler<()>;
