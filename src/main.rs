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
use nostr_sdk::RelayUrl;
use std::sync::Arc;
use std::time::Duration;
use tokio_metrics::RuntimeMonitor;
use tokio_util::sync::CancellationToken;
use tokio_util_watchdog::Watchdog;

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
    #[cfg(feature = "console")]
    {
        use std::time::Duration;
        console_subscriber::ConsoleLayer::builder()
            .server_addr(([0, 0, 0, 0], 6669))
            .retention(Duration::from_secs(3600)) // Keep task history for 1 hour
            .init();
        let (_non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
        return guard;
    }

    #[cfg(not(feature = "console"))]
    {
        use tracing_subscriber::{fmt, EnvFilter};

        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,groups_relay=debug,relay_builder=debug"));

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
}

fn main() -> Result<()> {
    // Keep the guard alive for the entire program duration
    let _guard = setup_tracing();

    // Build runtime with explicit worker thread count to prevent deadlock
    // on low-CPU machines. Default is num_cpus, but with only 2 workers,
    // any race in park/wake coordination can freeze the entire runtime.
    // Also increased blocking thread pool for heavy LMDB load.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .max_blocking_threads(2048)
        .thread_keep_alive(Duration::from_secs(60))
        .build()
        .expect("Failed to create Tokio runtime");

    runtime.block_on(async_main())
}

/// Spawns a background task that periodically logs runtime metrics.
/// This provides visibility into runtime health evolution before any deadlock.
fn spawn_runtime_metrics_logger(handle: tokio::runtime::Handle) {
    let runtime_monitor = RuntimeMonitor::new(&handle);

    tokio::spawn(async move {
        // Log metrics every 60 seconds
        let mut intervals = runtime_monitor.intervals();
        let mut iteration = 0u64;

        loop {
            // Wait 60 seconds between metrics snapshots
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            // Get metrics delta since last call (returns None if runtime is shutting down)
            let Some(metrics) = intervals.next() else {
                tracing::info!(target: "groups_relay", "Runtime shutting down, stopping metrics logger");
                break;
            };
            iteration += 1;

            tracing::info!(
                target: "groups_relay",
                iteration = iteration,
                workers_count = metrics.workers_count,
                total_park_count = metrics.total_park_count,
                total_noop_count = metrics.total_noop_count,
                total_steal_count = metrics.total_steal_count,
                total_steal_operations = metrics.total_steal_operations,
                total_polls_count = metrics.total_polls_count,
                total_busy_duration_ms = metrics.total_busy_duration.as_millis() as u64,
                total_local_schedule_count = metrics.total_local_schedule_count,
                total_overflow_count = metrics.total_overflow_count,
                budget_forced_yield_count = metrics.budget_forced_yield_count,
                io_driver_ready_count = metrics.io_driver_ready_count,
                "Runtime metrics (60s interval)"
            );

            // Log warnings if we see concerning patterns
            if metrics.total_overflow_count > 0 {
                tracing::warn!(
                    target: "groups_relay",
                    overflow_count = metrics.total_overflow_count,
                    "Task queue overflow detected - tasks being pushed to global queue"
                );
            }

            if metrics.budget_forced_yield_count > 100 {
                tracing::warn!(
                    target: "groups_relay",
                    yield_count = metrics.budget_forced_yield_count,
                    "High budget forced yield count - tasks may be CPU-bound"
                );
            }
        }
    });
}

async fn async_main() -> Result<()> {
    // Initialize watchdog to detect runtime stalls
    // With panic(false), it logs diagnostics but doesn't crash
    let _watchdog = Watchdog::builder()
        .heartbeat_period(Duration::from_secs(1))
        .watchdog_timeout(Duration::from_secs(10))
        .triggered_metrics_duration(Duration::from_secs(3))
        .triggered_metrics_collections(30)
        .task_dump_deadline(Duration::from_secs(10))
        .panic(false)
        .thread_name("groups-relay-watchdog")
        .build();

    tracing::info!("Watchdog initialized with 10s timeout");

    // Start periodic runtime metrics logging for pre-deadlock diagnostics
    spawn_runtime_metrics_logger(tokio::runtime::Handle::current());
    tracing::info!("Runtime metrics logger started (logs every 60s)");

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

    // Create database (CryptoHelper is created internally)
    let database = RelayDatabase::new(settings.db_path.clone()).await?;
    let database = Arc::new(database);
    let groups = Arc::new(
        Groups::load_groups(
            Arc::clone(&database),
            relay_keys.public_key(),
            settings.relay_url.clone(),
        )
        .await?,
    );

    server::run_server(settings, relay_keys, database, groups).await?;

    Ok(())
}
