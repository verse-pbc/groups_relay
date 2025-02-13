use anyhow::{Context, Result};
use clap::Parser;
use groups_relay::{config, groups::Groups, nostr_database::RelayDatabase, server};
use nostr_sdk::RelayUrl;
use std::sync::Arc;

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

    /// Override authentication URL
    #[arg(short, long)]
    auth_url: Option<String>,
}

fn setup_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,groups_relay=debug,websocket_builder=debug"));

    fmt()
        .with_env_filter(env_filter)
        .with_timer(fmt::time::SystemTime)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_level(true)
        .with_span_events(fmt::format::FmtSpan::NEW | fmt::format::FmtSpan::CLOSE)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing();

    let args = Args::parse();
    let config = config::Config::new(&args.config_dir).context("Failed to load configuration")?;
    let relay_settings = config
        .get_settings()
        .context("Failed to get relay settings")?;

    let mut settings = config::Settings {
        relay_url: relay_settings.relay_url.clone(),
        local_addr: relay_settings.local_addr.clone(),
        auth_url: relay_settings.auth_url.clone(),
        admin_keys: vec![],
        websocket: relay_settings.websocket.clone(),
        db_path: relay_settings.db_path.clone(),
    };

    if let Some(target_url) = args.relay_url {
        settings.relay_url = target_url;
    }

    if let Some(local_addr) = args.local_addr {
        settings.local_addr = local_addr;
    }

    if let Some(auth_url) = args.auth_url {
        settings.auth_url = auth_url;
    }

    // Validate URLs
    let _relay_url = RelayUrl::parse(&settings.relay_url)?;
    let _auth_url = RelayUrl::parse(&settings.auth_url)?;

    let relay_keys = relay_settings.relay_keys()?;
    let database = Arc::new(RelayDatabase::new(
        settings.db_path.clone(),
        relay_keys.clone(),
    )?);
    let groups = Arc::new(Groups::load_groups(database.clone(), relay_keys.public_key()).await?);

    server::run_server(settings, relay_keys, database, groups).await?;

    Ok(())
}
