use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use nostr_lmdb::NostrLMDB;
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(
    name = "delete-event",
    version = "0.1.0",
    about = "Deletes a specific Nostr event or prunes groups from the LMDB database that are either inactive (no activity in 1+ month) or empty (no members)."
)]
struct Args {
    /// Hex-encoded ID of the Nostr event to delete.
    #[arg(short, long)]
    event_id: Option<String>,

    /// Path to the LMDB database directory.
    #[arg(short, long)]
    db: PathBuf,

    /// If set, prunes all groups that haven't had activity in the last 1+ month or have no members
    #[arg(long)]
    prune_inactive_groups: bool,

    /// Skip confirmation prompt
    #[arg(long)]
    yes: bool,
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

#[derive(Debug)]
struct PruneStats {
    inactive_groups: usize,
    empty_groups: usize,
    events_deleted: usize,
}

#[derive(Debug)]
struct GroupInfo {
    name: Option<String>,
    reason: String,
}

async fn analyze_groups(db: &NostrLMDB) -> Result<(HashMap<String, GroupInfo>, PruneStats)> {
    let one_month_ago = Timestamp::now() - Duration::from_secs(30 * 24 * 60 * 60);
    let mut stats = PruneStats {
        inactive_groups: 0,
        empty_groups: 0,
        events_deleted: 0,
    };
    let mut groups_to_delete = HashMap::new();

    // Step 1: Get all group IDs and metadata
    info!("Fetching all group metadata events...");
    let metadata_filter = Filter::new().kinds(vec![
        Kind::Custom(39000), // KIND_GROUP_METADATA_39000
        Kind::Custom(39001), // KIND_GROUP_ADMINS_39001
        Kind::Custom(39002), // KIND_GROUP_MEMBERS_39002
    ]);

    let metadata_events = db.query(metadata_filter).await?;
    let mut group_ids = HashSet::new();
    let mut group_names = HashMap::new();

    // First pass: collect group IDs and names
    for event in metadata_events.iter() {
        if let Some(group_id) = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::h() || t.kind() == TagKind::d())
            .and_then(|t| t.content())
        {
            group_ids.insert(group_id.to_string());

            // If this is a metadata event, try to get the group name
            if event.kind == Kind::Custom(39000) {
                if let Some(name) = event.tags.find(TagKind::Name).and_then(|t| t.content()) {
                    group_names.insert(group_id.to_string(), name.to_string());
                }
            }
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
        // First check if the group has any members
        let members_filter = Filter::new()
            .kind(Kind::Custom(39002))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), &group_id)
            .limit(1);

        let members_events = db.query(members_filter).await?;
        let is_empty = if let Some(members_event) = members_events.first() {
            // Count p tags which represent members
            members_event.tags.filter(TagKind::p()).count() == 0
        } else {
            // No members event means empty group
            true
        };

        let mut should_delete = false;
        let mut delete_reason = String::new();

        if is_empty {
            should_delete = true;
            delete_reason = "empty (no members)".to_string();
            stats.empty_groups += 1;
        } else {
            // Check for inactivity only if group is not empty
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
                if latest_event.created_at < one_month_ago {
                    should_delete = true;
                    delete_reason = "inactive (no activity in 1+ month)".to_string();
                    stats.inactive_groups += 1;
                }
            }
        }

        if should_delete {
            groups_to_delete.insert(
                group_id.clone(),
                GroupInfo {
                    name: group_names.get(&group_id).cloned(),
                    reason: delete_reason,
                },
            );
        }

        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Analysis complete");
    Ok((groups_to_delete, stats))
}

async fn delete_groups(db: &NostrLMDB, groups: &HashMap<String, GroupInfo>) -> Result<usize> {
    let mut events_deleted = 0;
    let progress_bar = ProgressBar::new(groups.len() as u64);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} deletions ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    for (group_id, info) in groups {
        info!("Deleting group {}: {}", group_id, info.reason);

        // Delete all events with h tag
        let h_deletion_filter =
            Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::H), group_id);
        db.delete(h_deletion_filter).await?;

        // Delete all events with d tag
        let d_deletion_filter =
            Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id);
        db.delete(d_deletion_filter).await?;

        events_deleted += 2; // We can't get exact count, increment by 2 for h and d tag deletions
        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Deletion complete");
    Ok(events_deleted)
}

fn prompt_for_confirmation(
    groups: &HashMap<String, GroupInfo>,
    stats: &PruneStats,
) -> Result<bool> {
    println!("\nGroups to be deleted:");
    println!("=====================");

    for (group_id, info) in groups {
        let name_display = info.name.as_deref().unwrap_or("(unnamed)");
        println!("- {} ({}): {}", name_display, group_id, info.reason);
    }

    println!("\nSummary:");
    println!("- {} groups will be deleted:", groups.len());
    println!("  • {} empty groups", stats.empty_groups);
    println!("  • {} inactive groups", stats.inactive_groups);

    print!("\nDo you want to proceed with deletion? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().eq_ignore_ascii_case("y"))
}

async fn prune_inactive_groups(db: &NostrLMDB, skip_confirmation: bool) -> Result<()> {
    info!("Analyzing groups...");
    let (groups_to_delete, mut stats) = analyze_groups(db).await?;

    if groups_to_delete.is_empty() {
        info!("No groups found that need to be deleted.");
        return Ok(());
    }

    if !skip_confirmation && !prompt_for_confirmation(&groups_to_delete, &stats)? {
        info!("Deletion cancelled by user.");
        return Ok(());
    }

    info!("Starting deletion...");
    stats.events_deleted = delete_groups(db, &groups_to_delete).await?;

    info!(
        "Pruning complete. Deleted {} inactive groups, {} empty groups, at least {} events total",
        stats.inactive_groups, stats.empty_groups, stats.events_deleted
    );
    info!("\x1b[1;33mIMPORTANT: You must restart the relay server for these changes to take effect!\x1b[0m");

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
        info!("Starting groups analysis...");
        prune_inactive_groups(&db, args.yes).await
    } else if let Some(event_id) = args.event_id {
        // Handle single event deletion
        let event_id = EventId::from_hex(&event_id)
            .with_context(|| format!("Invalid event ID format: {event_id}"))?;

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
