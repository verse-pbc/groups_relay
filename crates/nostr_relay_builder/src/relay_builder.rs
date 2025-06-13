//! RelayBuilder for constructing Nostr relays with custom state

use crate::config::RelayConfig;
use crate::crypto_worker::CryptoWorker;
use crate::database::RelayDatabase;
use crate::error::Error;
use crate::event_processor::EventProcessor;
use crate::message_converter::NostrMessageConverter;
use crate::metrics::SubscriptionMetricsHandler;
use crate::middleware::RelayMiddleware;
use crate::middlewares::MetricsHandler;
use crate::state::{GenericNostrConnectionFactory, NostrConnectionState};
use nostr_sdk::prelude::*;
use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use websocket_builder::{Middleware, WebSocketBuilder};

/// HTML rendering options for the relay
#[cfg(feature = "axum")]
#[derive(Clone, Default)]
pub enum HtmlOption {
    /// No HTML - returns 404 for non-WebSocket requests
    None,
    /// Default HTML page that renders RelayInfo
    #[default]
    Default,
    /// Custom HTML provider function
    Custom(Arc<dyn Fn(&crate::handlers::RelayInfo) -> String + Send + Sync>),
}

#[cfg(feature = "axum")]
impl std::fmt::Debug for HtmlOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlOption::None => write!(f, "HtmlOption::None"),
            HtmlOption::Default => write!(f, "HtmlOption::Default"),
            HtmlOption::Custom(_) => write!(f, "HtmlOption::Custom(<function>)"),
        }
    }
}

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
    /// Optional metrics handler for the relay
    metrics_handler: Option<Arc<dyn MetricsHandler>>,
    /// Optional subscription metrics handler
    subscription_metrics_handler: Option<Arc<dyn SubscriptionMetricsHandler>>,
    /// HTML rendering option for browser requests
    #[cfg(feature = "axum")]
    html_option: HtmlOption,
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
            metrics_handler: None,
            subscription_metrics_handler: None,
            #[cfg(feature = "axum")]
            html_option: HtmlOption::Default,
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

    /// Set a metrics handler for the relay
    #[must_use]
    pub fn with_metrics<M>(mut self, handler: M) -> Self
    where
        M: MetricsHandler + 'static,
    {
        self.metrics_handler = Some(Arc::new(handler));
        self
    }

    /// Set a subscription metrics handler
    #[must_use]
    pub fn with_subscription_metrics<M>(mut self, handler: M) -> Self
    where
        M: SubscriptionMetricsHandler + 'static,
    {
        self.subscription_metrics_handler = Some(Arc::new(handler));
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
            metrics_handler: None,
            subscription_metrics_handler: None,
            #[cfg(feature = "axum")]
            html_option: self.html_option,
            _phantom: PhantomData,
        }
    }

    /// Set HTML rendering option
    #[cfg(feature = "axum")]
    #[must_use]
    pub fn with_html(mut self, html_option: HtmlOption) -> Self {
        self.html_option = html_option;
        self
    }

    /// Disable HTML rendering (return 404 for browser requests)
    #[cfg(feature = "axum")]
    #[must_use]
    pub fn without_html(mut self) -> Self {
        self.html_option = HtmlOption::None;
        self
    }

    /// Set custom HTML provider
    #[cfg(feature = "axum")]
    #[must_use]
    pub fn with_custom_html<F>(mut self, html_provider: F) -> Self
    where
        F: Fn(&crate::handlers::RelayInfo) -> String + Send + Sync + 'static,
    {
        self.html_option = HtmlOption::Custom(Arc::new(html_provider));
        self
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
        // Always calculate channel_size based on subscription limits
        let mut websocket_config = self.config.websocket_config.clone();
        websocket_config.channel_size =
            self.config.max_subscriptions * self.config.max_limit * 110 / 100;

        // Set the global subscription limits
        crate::global_config::set_max_subscriptions(self.config.max_subscriptions);
        crate::global_config::set_max_limit(self.config.max_limit);

        // Set the global subscription metrics handler if provided
        if let Some(handler) = self.subscription_metrics_handler.clone() {
            crate::global_metrics::set_subscription_metrics_handler(handler);
        }

        // Create the crypto worker for signing and verification
        let cancellation_token = self.cancellation_token.clone().unwrap_or_default();
        let crypto_worker = Arc::new(CryptoWorker::new(
            Arc::new(self.config.keys.clone()),
            cancellation_token.clone(),
        ));

        let database = self.config.create_database(crypto_worker.clone())?;
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

        // Add metrics middleware if handler is provided
        if let Some(metrics_handler) = self.metrics_handler.clone() {
            builder = builder.with_arc_middleware(Arc::new(
                crate::middlewares::MetricsMiddleware::with_arc_handler(metrics_handler),
            ));
        }

        // Add NIP-42 authentication middleware if enabled
        if self.config.enable_auth {
            let auth_config =
                self.config
                    .auth_config
                    .clone()
                    .unwrap_or_else(|| crate::middlewares::AuthConfig {
                        relay_url: self.config.relay_url.clone(),
                        validate_subdomains: matches!(
                            self.config.scope_config,
                            crate::config::ScopeConfig::Subdomain { .. }
                        ),
                    });
            builder =
                builder.with_middleware(crate::middlewares::Nip42Middleware::new(auth_config));
        }

        // Add event verification middleware with crypto worker
        builder = builder.with_middleware(crate::middlewares::EventVerifierMiddleware::new(
            crypto_worker.clone(),
        ));

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
        let html_option = self.html_option.clone();
        let handlers = Arc::new(self.build_handlers(processor, relay_info).await?);

        Ok(
            move |ws: Option<axum::extract::ws::WebSocketUpgrade>,
                  connect_info: axum::extract::ConnectInfo<std::net::SocketAddr>,
                  headers: axum::http::HeaderMap| {
                let handlers = handlers.clone();
                let html_option = html_option.clone();
                Box::pin(async move {
                    // Check if this is a WebSocket or NIP-11 request
                    if ws.is_some()
                        || headers
                            .get("accept")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s == "application/nostr+json")
                            .unwrap_or(false)
                    {
                        // Handle WebSocket/NIP-11 with the built handler
                        handlers.axum_root_handler()(ws, connect_info, headers).await
                    } else {
                        // Handle HTML based on configuration
                        use axum::response::{Html, IntoResponse};

                        match &html_option {
                            HtmlOption::None => axum::http::StatusCode::NOT_FOUND.into_response(),
                            HtmlOption::Default => {
                                Html(crate::handlers::default_relay_html(handlers.relay_info()))
                                    .into_response()
                            }
                            HtmlOption::Custom(provider) => {
                                Html(provider(handlers.relay_info())).into_response()
                            }
                        }
                    }
                })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = axum::response::Response> + Send>,
                    >
            },
        )
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
