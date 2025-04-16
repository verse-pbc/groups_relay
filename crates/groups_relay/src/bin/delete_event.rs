use anyhow::{Context, Result};
use clap::Parser;
use nostr_database::{NostrEventsDatabase};
use nostr_sdk::prelude::*;
use nostr_lmdb::NostrLMDB;
use std::path::PathBuf;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(
    name = "delete-event",
    version = "0.1.0",
    about = "Deletes a specific Nostr event from the LMDB database."
)]
struct Args {
    /// Hex-encoded ID of the Nostr event to delete.
    #[arg(short, long)]
    event_id: String,

    /// Path to the LMDB database directory.
    #[arg(short, long)]
    db: PathBuf,
}

fn setup_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,delete_event=debug"));

    fmt()
        .with_env_filter(env_filter)
        .with_timer(fmt::time::SystemTime)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_level(true)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing();
    let args = Args::parse();

    // Parse the event ID
    let event_id = EventId::from_hex(&args.event_id)
        .with_context(|| format!("Invalid event ID format: {}", args.event_id))?;

    info!(
        "Attempting to delete event ID {} from database at {:?}",
        event_id.to_hex(),
        args.db
    );

    // Open the database
    let db = NostrLMDB::open(&args.db)
        .with_context(|| format!("Failed to open database at {:?}", args.db))?;

    // Create the filter
    let filter = Filter::new().id(event_id);

    // Delete the event
    match db.delete(filter).await {
        Ok(_) => {
            info!(
                "Successfully deleted event {} from database {:?}.",
                event_id.to_hex(),
                args.db
            );
            Ok(())
        }
        Err(e) => {
            error!("Failed to delete event {}: {:?}", event_id.to_hex(), e);
            Err(e.into())
        }
    }
}
