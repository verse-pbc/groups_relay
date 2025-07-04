use anyhow::Result;
use clap::Parser;
use groups_relay::groups::{Groups, KIND_GROUP_METADATA_39000};
use groups_relay::RelayDatabase;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "add_original_relay",
    version = "0.1.0",
    about = "Add original_relay tag to existing metadata events"
)]
struct Args {
    /// Path to the database
    #[arg(short, long)]
    db_path: String,

    /// Relay private key in hex format
    #[arg(short = 'k', long)]
    relay_private_key: String,

    /// Relay URL to add as original_relay tag
    #[arg(short = 'u', long)]
    relay_url: String,

    /// Dry run mode - don't actually save events
    #[arg(short = 'n', long, default_value = "false")]
    dry_run: bool,
}

fn setup_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

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

    // Parse relay keys
    let secret_key = SecretKey::from_hex(&args.relay_private_key)?;
    let relay_keys = Keys::new(secret_key);
    let relay_pubkey = relay_keys.public_key();

    info!("Starting add_original_relay tool");
    info!("Database path: {}", args.db_path);
    info!("Relay URL: {}", args.relay_url);
    info!("Relay public key: {}", relay_pubkey);
    info!("Dry run: {}", args.dry_run);

    // Create task tracker
    let task_tracker = TaskTracker::new();

    // Open database
    let (database, db_sender) = RelayDatabase::with_task_tracker(
        &args.db_path,
        Arc::new(relay_keys.clone()),
        task_tracker.clone(),
    )?;
    let database = Arc::new(database);
    task_tracker.close();

    // Load groups
    let groups =
        Groups::load_groups(Arc::clone(&database), relay_pubkey, args.relay_url.clone()).await?;

    // Get all scopes
    let scopes = database.list_scopes().await?;
    info!("Found {} scopes to process", scopes.len());

    let mut total_updated = 0;
    let mut total_errors = 0;

    for scope in scopes {
        info!("Processing scope: {:?}", scope);

        // Query all metadata events in this scope
        let metadata_filter = vec![Filter::new()
            .kinds(vec![KIND_GROUP_METADATA_39000])
            .since(Timestamp::from(0))];

        let metadata_events = match database.query(metadata_filter, &scope).await {
            Ok(events) => events,
            Err(e) => {
                error!(
                    "Error querying metadata events for scope {:?}: {}",
                    scope, e
                );
                total_errors += 1;
                continue;
            }
        };

        info!(
            "Found {} metadata events in scope {:?}",
            metadata_events.len(),
            scope
        );

        for event in metadata_events {
            // Check if event already has original_relay tag
            let has_original_relay = event
                .tags
                .iter()
                .any(|tag| tag.kind() == TagKind::custom("original_relay"));

            if has_original_relay {
                info!(
                    "Event {} already has original_relay tag, skipping",
                    event.id
                );
                continue;
            }

            // Extract group ID
            let group_id = match event.tags.find(TagKind::d()) {
                Some(tag) => match tag.content() {
                    Some(id) => id.to_string(),
                    None => {
                        warn!("Identifier tag has no content in event {}", event.id);
                        continue;
                    }
                },
                None => {
                    warn!("No identifier tag found in event {}", event.id);
                    continue;
                }
            };

            info!("Processing group {} metadata event {}", group_id, event.id);

            // Get the group
            let group = match groups.get_group(&scope, &group_id) {
                Some(g) => g,
                None => {
                    warn!("Group {} not found in scope {:?}", group_id, scope);
                    continue;
                }
            };

            // Generate new metadata event with original_relay tag
            let new_event = group.generate_metadata_event(&relay_pubkey, &args.relay_url);

            if args.dry_run {
                info!(
                    "DRY RUN: Would update metadata event for group {}",
                    group_id
                );
                info!("New event tags: {:?}", new_event.tags);
            } else {
                // Save the new unsigned event
                match db_sender
                    .save_unsigned_event(new_event, scope.clone())
                    .await
                {
                    Ok(_) => {
                        info!("Successfully updated metadata event for group {}", group_id);
                        total_updated += 1;
                    }
                    Err(e) => {
                        error!("Error saving metadata event for group {}: {}", group_id, e);
                        total_errors += 1;
                    }
                }
            }
        }
    }

    info!("Migration complete!");
    info!("Total events updated: {}", total_updated);
    info!("Total errors: {}", total_errors);

    if args.dry_run {
        info!("This was a dry run - no changes were made");
    }

    drop(db_sender);
    task_tracker.wait().await;

    Ok(())
}
