#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_safety_doc)]
#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::default_trait_access)]
#![warn(clippy::explicit_deref_methods)]
#![warn(clippy::explicit_iter_loop)]
#![warn(clippy::implicit_clone)]
#![warn(clippy::unnecessary_to_owned)]
#![warn(clippy::redundant_clone)]
#![warn(clippy::needless_collect)]
#![warn(clippy::missing_const_for_fn)]
#![warn(clippy::module_name_repetitions)]

use anyhow::{Context, Result};
use clap::Parser;
use groups_relay::{config, groups::Groups, server, RelayDatabase};
use nostr_relay_builder::crypto_worker::CryptoWorker;
use nostr_sdk::RelayUrl;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

#[derive(Parser, Debug)]
#[command(
    name = "Nip 29",
    version = "0.1.0",
    about = "Adds nip 29 functionality to the provided Nostr relay"
)]
struct Args {
    /// Path to config directory
    #[arg(short, long, default_value = "config")]
    config_dir: String,

    /// Override target WebSocket URL
    #[arg(short, long)]
    relay_url: Option<String>,

    /// Override source address
    #[arg(short, long)]
    local_addr: Option<String>,
}

fn setup_tracing() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,groups_relay=debug,websocket_builder=debug"));

    // Create non-blocking stdout writer
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());

    fmt()
        .with_writer(non_blocking)
        .with_env_filter(env_filter)
        .with_timer(fmt::time::SystemTime)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_level(true)
        .init();

    guard // Return the guard to keep it alive
}

#[tokio::main]
async fn main() -> Result<()> {
    // Keep the guard alive for the entire program duration
    let _guard = setup_tracing();

    let args = Args::parse();
    let config = config::Config::new(&args.config_dir).context("Failed to load configuration")?;
    let relay_settings = config
        .get_settings()
        .context("Failed to get relay settings")?;

    let mut settings = config::Settings {
        relay_url: relay_settings.relay_url.clone(),
        local_addr: relay_settings.local_addr.clone(),
        admin_keys: vec![],
        websocket: relay_settings.websocket.clone(),
        db_path: relay_settings.db_path.clone(),
        max_limit: relay_settings.max_limit,
        max_subscriptions: relay_settings.max_subscriptions,
    };

    if let Some(target_url) = args.relay_url {
        settings.relay_url = target_url;
    }

    if let Some(local_addr) = args.local_addr {
        settings.local_addr = local_addr;
    }

    // Validate URL
    let _relay_url = RelayUrl::parse(&settings.relay_url)
        .unwrap_or_else(|_| panic!("Invalid relay_url scheme: {}", settings.relay_url));

    let relay_keys = relay_settings.relay_keys()?;
    let _cancellation_token = CancellationToken::new();
    // Create task tracker for managing background tasks
    let task_tracker = TaskTracker::new();

    // Spawn crypto workers
    let crypto_sender = CryptoWorker::spawn(Arc::new(relay_keys.clone()), &task_tracker);

    // Create database with crypto sender
    let (database, db_sender) = RelayDatabase::new(settings.db_path.clone(), crypto_sender)?;
    let database = Arc::new(database);
    let groups = Arc::new(
        Groups::load_groups(
            Arc::clone(&database),
            relay_keys.public_key(),
            settings.relay_url.clone(),
        )
        .await?,
    );

    server::run_server(settings, relay_keys, database, db_sender, groups).await?;

    Ok(())
}
