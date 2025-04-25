use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use nostr_database::NostrEventsDatabase;
use nostr_lmdb::NostrLMDB;
use nostr_sdk::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(
    name = "delete-event",
    version = "0.1.0",
    about = "Deletes a specific Nostr event or prunes inactive groups from the LMDB database."
)]
struct Args {
    /// Hex-encoded ID of the Nostr event to delete.
    #[arg(short, long)]
    event_id: Option<String>,

    /// Path to the LMDB database directory.
    #[arg(short, long)]
    db: PathBuf,

    /// If set, prunes all groups that haven't had activity in the last 3 months
    #[arg(long)]
    prune_inactive_groups: bool,
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

async fn prune_inactive_groups(db: &NostrLMDB) -> Result<()> {
    let three_months_ago = Timestamp::now() - Duration::from_secs(90 * 24 * 60 * 60);
    let mut groups_pruned = 0;
    let mut events_deleted = 0;

    // Step 1: Get all group IDs
    info!("Fetching all group metadata events...");
    let metadata_filter = Filter::new().kinds(vec![
        Kind::Custom(39000), // KIND_GROUP_METADATA_39000
        Kind::Custom(39001), // KIND_GROUP_ADMINS_39001
        Kind::Custom(39002), // KIND_GROUP_MEMBERS_39002
    ]);

    let metadata_events = db.query(metadata_filter).await?;
    let mut group_ids = HashSet::new();

    for event in metadata_events {
        if let Some(group_id) = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::h() || t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            group_ids.insert(group_id.to_string());
        }
    }

    info!("Found {} total groups to analyze", group_ids.len());

    // Create progress bar
    let progress_bar = ProgressBar::new(group_ids.len() as u64);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} groups ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    // Step 2: Process each group
    for group_id in group_ids {
        // Find latest event for this group
        let h_filter = Filter::new()
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &group_id)
            .limit(1)
            .until(Timestamp::now());
        let d_filter = Filter::new()
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), &group_id)
            .limit(1)
            .until(Timestamp::now());

        let mut latest_events = db.query(h_filter).await?;
        latest_events.extend(db.query(d_filter).await?);

        if let Some(latest_event) = latest_events.first() {
            if latest_event.created_at < three_months_ago {
                info!("Pruning inactive group: {}", group_id);

                // Delete all events for this group
                let h_deletion_filter =
                    Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::H), &group_id);
                let d_deletion_filter =
                    Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::D), &group_id);

                // Delete events with h tag
                db.delete(h_deletion_filter).await?;
                // Delete events with d tag
                db.delete(d_deletion_filter).await?;

                events_deleted += 1; // We can't get exact count, so increment by 1 for each deletion operation
                groups_pruned += 1;
                info!("Deleted events for group {}", group_id);
            }
        }

        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Pruning complete");
    info!(
        "Pruning complete. Pruned {} groups, deleted at least {} events",
        groups_pruned, events_deleted
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing();
    let args = Args::parse();

    // Open the database
    let db = NostrLMDB::open(&args.db)
        .with_context(|| format!("Failed to open database at {:?}", args.db))?;

    if args.prune_inactive_groups {
        info!("Starting inactive groups pruning...");
        prune_inactive_groups(&db).await
    } else if let Some(event_id) = args.event_id {
        // Handle single event deletion
        let event_id = EventId::from_hex(&event_id)
            .with_context(|| format!("Invalid event ID format: {}", event_id))?;

        info!(
            "Attempting to delete event ID {} from database at {:?}",
            event_id.to_hex(),
            args.db
        );

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
    } else {
        Err(anyhow::anyhow!(
            "Either --event-id or --prune-inactive-groups must be specified"
        ))
    }
}
