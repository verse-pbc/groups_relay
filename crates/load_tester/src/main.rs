// This is the main file for the load_tester crate.
// It serves as a skeleton for implementing a load testing tool against the Groups Relay.
// The goal is to simulate multiple WebSocket clients, generate test events, collect metrics,
// and provide reporting. This template includes minimal code and detailed comments to guide further implementation.

use anyhow::Result;
use clap::Parser; // For command-line argument parsing
use nostr_sdk::prelude::*;
use rand::Rng;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{error, info};

// NIP-29 event kinds (matching the frontend definitions)
#[derive(Debug, Clone, Copy)]
pub enum GroupEventKind {
    JoinRequest = 9021,
    LeaveRequest = 9022,
    PutUser = 9000,
    RemoveUser = 9001,
    EditMetadata = 9002,
    DeleteEvent = 9005,
    CreateGroup = 9007,
    DeleteGroup = 9008,
    CreateInvite = 9009,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientRole {
    Admin,
    Member,
    NonMember,
}

/// Metrics for tracking load test results
#[derive(Debug, Default)]
struct Metrics {
    events_sent: usize,
    events_received: usize,
    errors: usize,
    total_latency: Duration,
}

impl Metrics {
    fn new() -> Self {
        Self::default()
    }

    fn average_latency(&self) -> Option<Duration> {
        if self.events_received == 0 {
            None
        } else {
            Some(self.total_latency / self.events_received as u32)
        }
    }
}

/// Command-line arguments for configuring the load test.
///
/// - `clients`: Number of concurrent simulated clients.
/// - `url`: The WebSocket endpoint of the relay to test.
/// - `duration`: How long (in seconds) the test should run.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of concurrent clients to simulate
    #[arg(short, long, default_value = "10")]
    clients: usize,

    /// WebSocket URL of the relay to test
    #[arg(short, long, default_value = "ws://127.0.0.1:8080")]
    url: String,

    /// Duration of the test in seconds
    #[arg(short, long, default_value = "60")]
    duration: u64,
}

/// Client configuration for the load test
#[derive(Debug, Clone)]
struct ClientConfig {
    url: String,
    keys: Keys,
    group_id: Option<String>,
    role: ClientRole,
    invite_code: Option<String>,
}

impl ClientConfig {
    fn new(url: String, keys: Keys, role: ClientRole) -> Self {
        Self {
            url,
            keys,
            group_id: None,
            role,
            invite_code: None,
        }
    }
}

/// Generate a random invite code
fn generate_invite_code() -> String {
    format!("INVITE{}", rand::thread_rng().gen::<u32>())
}

/// Generate a Nostr event for testing based on client role
async fn generate_test_event(config: &ClientConfig, kind: GroupEventKind) -> Result<EventBuilder> {
    // Validate event permissions based on role
    match (config.role, kind) {
        // Admin-only events
        (ClientRole::Admin, GroupEventKind::CreateGroup)
        | (ClientRole::Admin, GroupEventKind::EditMetadata)
        | (ClientRole::Admin, GroupEventKind::CreateInvite)
        | (ClientRole::Admin, GroupEventKind::PutUser)
        | (ClientRole::Admin, GroupEventKind::RemoveUser)
        | (ClientRole::Admin, GroupEventKind::DeleteEvent)
        | (ClientRole::Admin, GroupEventKind::DeleteGroup) => (),

        // Member events
        (ClientRole::Member, GroupEventKind::LeaveRequest) => (),

        // Non-member events
        (ClientRole::NonMember, GroupEventKind::JoinRequest) => (),

        // Invalid role/event combination
        _ => anyhow::bail!("Invalid event kind {:?} for role {:?}", kind, config.role),
    }

    let content = match kind {
        GroupEventKind::CreateGroup => json!({
            "name": "Test Group",
            "about": "A test group for load testing",
            "picture": "https://example.com/pic.jpg",
            "id": config.group_id
        })
        .to_string(),

        GroupEventKind::EditMetadata => json!({
            "name": format!("Updated Group {}", rand::thread_rng().gen::<u32>()),
            "about": "Updated group description",
            "picture": "https://example.com/updated.jpg"
        })
        .to_string(),

        GroupEventKind::CreateInvite => {
            let invite_code = config
                .invite_code
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Invite code is required for CreateInvite event"))?;
            json!({
                "type": "invite",
                "group_id": config.group_id,
                "code": invite_code,
                "roles": ["member"],
                "expires_at": Timestamp::now().as_u64() + 86400 // 24 hours
            })
            .to_string()
        }

        GroupEventKind::JoinRequest => {
            if let Some(code) = &config.invite_code {
                json!({
                    "type": "join_request",
                    "group_id": config.group_id,
                    "code": code,
                    "message": "Request to join with invite code"
                })
                .to_string()
            } else {
                json!({
                    "type": "join_request",
                    "group_id": config.group_id,
                    "message": "Manual request to join"
                })
                .to_string()
            }
        }

        GroupEventKind::LeaveRequest => json!({
            "type": "leave_request",
            "group_id": config.group_id,
            "message": "Leaving the group"
        })
        .to_string(),

        GroupEventKind::PutUser => {
            // Randomly select a user to add from pending join requests
            json!({
                "type": "add_user",
                "group_id": config.group_id,
                "roles": ["member"]
            })
            .to_string()
        }

        GroupEventKind::RemoveUser => json!({
            "type": "remove_user",
            "group_id": config.group_id
        })
        .to_string(),

        GroupEventKind::DeleteEvent => json!({
            "type": "delete_event",
            "group_id": config.group_id
        })
        .to_string(),

        GroupEventKind::DeleteGroup => json!({
            "type": "delete_group",
            "group_id": config.group_id
        })
        .to_string(),
    };

    let mut builder = EventBuilder::new(Kind::Custom(kind as u16), content);

    // Add tags based on event kind
    if let Some(group_id) = &config.group_id {
        // Add h tag for all group events
        builder = builder.tag(Tag::custom(TagKind::h(), [group_id]));

        // Add additional tags based on event kind
        match kind {
            GroupEventKind::PutUser => {
                // Add p tag for user being added with role
                builder = builder.tag(Tag::custom(TagKind::p(), ["role=member"]));
            }
            GroupEventKind::RemoveUser => {
                // Add p tag for user being removed - we need to specify which user
                // For testing, we'll remove the event author
                builder = builder.tag(Tag::custom(
                    TagKind::p(),
                    [config.keys.public_key().to_string()],
                ));
            }
            GroupEventKind::DeleteEvent => {
                // Add e tag for event being deleted - we need to specify which event
                // For testing, we'll need to track an event ID to delete
                // TODO: Track an event ID to delete
                anyhow::bail!("DeleteEvent requires an event ID to delete");
            }
            GroupEventKind::EditMetadata => {
                // Add metadata tags
                builder = builder
                    .tag(Tag::custom(TagKind::custom("public"), &[] as &[String]))
                    .tag(Tag::custom(TagKind::custom("open"), &[] as &[String]));
            }
            GroupEventKind::CreateInvite => {
                // Add code tag for invite
                if let Some(code) = &config.invite_code {
                    builder = builder.tag(Tag::custom(TagKind::custom("code"), [code]));
                }
            }
            GroupEventKind::JoinRequest => {
                // Add code tag for join request if invite code is present
                if let Some(code) = &config.invite_code {
                    builder = builder.tag(Tag::custom(TagKind::custom("code"), [code]));
                }
            }
            _ => {}
        }
    } else {
        anyhow::bail!("Group ID is required for group events");
    }

    Ok(builder)
}

/// Generate appropriate event kinds based on client role
fn generate_event_kind_for_role(role: ClientRole) -> GroupEventKind {
    match role {
        ClientRole::Admin => {
            let kinds = [
                GroupEventKind::EditMetadata,
                GroupEventKind::CreateInvite,
                GroupEventKind::PutUser,
                GroupEventKind::RemoveUser,
            ];
            kinds[rand::thread_rng().gen_range(0..kinds.len())]
        }
        ClientRole::Member => GroupEventKind::LeaveRequest,
        ClientRole::NonMember => GroupEventKind::JoinRequest,
    }
}

/// Wait for client to be connected to relay
async fn wait_for_connection(client: &Client) -> Result<()> {
    let mut attempts = 0;
    while attempts < 5 {
        let relays = client.relays().await;
        let mut connected = false;
        for (_, relay) in relays.iter() {
            if relay.status() == RelayStatus::Connected {
                connected = true;
                break;
            }
        }
        if connected {
            return Ok(());
        }
        sleep(Duration::from_secs(2u64.pow(attempts))).await;
        attempts += 1;
    }
    anyhow::bail!("Failed to connect to relay after 5 attempts")
}

/// Authenticate a client before sending events
async fn authenticate_client(client: &Client) -> Result<()> {
    // First ensure we're connected
    wait_for_connection(client).await?;

    // Wait for auth to complete
    sleep(Duration::from_secs(2)).await;

    // Verify we're authenticated
    let relays = client.relays().await;
    for (_, relay) in relays.iter() {
        if relay.status() != RelayStatus::Connected {
            anyhow::bail!("Not connected to relay after authentication");
        }
    }

    Ok(())
}

/// Create a new group and return its ID
async fn create_group(config: &ClientConfig) -> Result<String> {
    let client = Client::new(config.keys.clone());
    client.add_relay(&config.url).await?;
    client.connect().await;

    // Wait for connection and authenticate
    authenticate_client(&client).await?;

    // Generate a unique group ID using a random hex string
    let group_id = format!("{:x}", rand::thread_rng().gen::<u64>());
    info!("Generated group ID: {}", group_id);

    let metadata = json!({
        "name": "Test Group",
        "about": "A test group for load testing",
        "picture": "https://example.com/pic.jpg",
        "id": group_id
    });

    // First create the group
    let builder = EventBuilder::new(
        Kind::Custom(GroupEventKind::CreateGroup as u16),
        metadata.to_string(),
    )
    .tag(Tag::parse(vec!["d", &group_id])?)
    .tag(Tag::parse(vec!["h", &group_id])?);

    let output = client.send_event_builder(builder).await?;
    info!("Group creation sent to: {:?}", output.success);
    if !output.failed.is_empty() {
        info!("Group creation failed for: {:?}", output.failed);
    }

    // Wait for group creation to be processed
    sleep(Duration::from_secs(5)).await;

    // Now set the metadata to make the group public and open
    let metadata_builder = EventBuilder::new(
        Kind::Custom(GroupEventKind::EditMetadata as u16),
        json!({
            "name": "Test Group",
            "about": "A test group for load testing",
            "picture": "https://example.com/pic.jpg"
        })
        .to_string(),
    )
    .tag(Tag::parse(vec!["d", &group_id])?)
    .tag(Tag::parse(vec!["h", &group_id])?)
    .tag(Tag::parse(vec!["public", ""])?)
    .tag(Tag::parse(vec!["open", ""])?);

    let metadata_output = client.send_event_builder(metadata_builder).await?;
    info!(
        "Group metadata update sent to: {:?}",
        metadata_output.success
    );
    if !metadata_output.failed.is_empty() {
        info!(
            "Group metadata update failed for: {:?}",
            metadata_output.failed
        );
    }

    // Wait for metadata update to be processed
    sleep(Duration::from_secs(5)).await;

    // Verify the group state is persisted by doing a fresh subscription
    let verify_client = Client::new(config.keys.clone());
    verify_client.add_relay(&config.url).await?;
    verify_client.connect().await;
    wait_for_connection(&verify_client).await?;
    authenticate_client(&verify_client).await?;

    // First try to fetch the group state event directly
    let verify_filter = Filter::new()
        .kinds(vec![Kind::Custom(39000)]) // Group state event
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec![&group_id]);

    let Output {
        val: verify_sub_id, ..
    } = verify_client
        .subscribe(vec![verify_filter.clone()], None)
        .await?;
    info!("Verifying group state persistence...");

    // Create a channel to receive the verification event
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let sub_id_clone = verify_sub_id.clone();

    // Handle notifications in a separate task
    let _notification_handle = tokio::spawn({
        let client = verify_client.clone();
        let tx = tx.clone();
        async move {
            client
                .handle_notifications(move |notification| {
                    let tx = tx.clone();
                    let sub_id = sub_id_clone.clone();
                    async move {
                        if let RelayPoolNotification::Event {
                            subscription_id,
                            event,
                            ..
                        } = notification
                        {
                            if subscription_id == sub_id {
                                info!(
                                    "Received group state verification event: {} from pubkey {}",
                                    event.id, event.pubkey
                                );
                                let _ = tx.send(event).await;
                                return Ok(true); // Stop handling notifications
                            }
                        }
                        Ok(false)
                    }
                })
                .await
        }
    });

    // Wait for verification event with longer timeouts
    let mut verified = false;
    for i in 0..5 {
        info!("Waiting for group state verification, attempt {}", i + 1);
        match tokio::time::timeout(Duration::from_secs(5u64.pow(i)), rx.recv()).await {
            Ok(Some(event)) => {
                info!(
                    "Received group state event: {} from pubkey {}",
                    event.id, event.pubkey
                );
                // Check if this event has our group ID in its tags
                let has_group = event.tags.iter().any(|tag| {
                    if let Ok(expected_tag) = Tag::parse(vec!["d", &group_id]) {
                        tag == &expected_tag
                    } else {
                        false
                    }
                });

                if has_group {
                    info!("Found our group in state event tags");
                    verified = true;
                    // Wait additional time for the event to be fully processed
                    sleep(Duration::from_secs(5)).await;
                    info!("Group state verified after {} attempts", i + 1);
                    break;
                } else {
                    info!("Our group not found in state event tags");
                }
            }
            Ok(None) => {
                info!("Channel closed without receiving verification");
                break;
            }
            Err(_) => {
                info!("Timeout waiting for verification");
                continue;
            }
        }
    }

    if !verified {
        anyhow::bail!("Failed to verify group state persistence");
    }

    // Wait longer for group state to be fully processed
    sleep(Duration::from_secs(10)).await;

    // Unsubscribe from verification subscription
    verify_client.unsubscribe(verify_sub_id).await;

    Ok(group_id)
}

/// Setup signal handler for graceful shutdown
async fn setup_signal_handler() -> Result<broadcast::Sender<()>> {
    let (shutdown_tx, _) = broadcast::channel(1);
    let shutdown_tx_clone = shutdown_tx.clone();

    tokio::spawn(async move {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal");
            }
            _ = sigint.recv() => {
                info!("Received SIGINT signal");
            }
        }

        info!("Initiating graceful shutdown...");
        let _ = shutdown_tx_clone.send(());
        Ok::<_, anyhow::Error>(())
    });

    Ok(shutdown_tx)
}

/// The main function initializes logging, parses arguments,
/// and spawns multiple asynchronous tasks to simulate load.
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging
    tracing_subscriber::fmt::init();

    // Setup signal handler
    let shutdown_tx = setup_signal_handler().await?;

    // Parse command-line arguments
    let args = Args::parse();
    info!(
        "Starting load test with {} clients for {} seconds",
        args.clients, args.duration
    );

    // Create shared metrics
    let metrics = Arc::new(Mutex::new(Metrics::new()));
    let mut handles = Vec::new();

    // Create admin client and group
    let admin_keys = Keys::generate();
    let mut admin_config =
        ClientConfig::new(args.url.clone(), admin_keys.clone(), ClientRole::Admin);
    let group_id = create_group(&admin_config).await?;
    admin_config.group_id = Some(group_id.clone());
    info!("Created group with ID: {} by admin", group_id);

    // Wait longer for group creation and internal events to complete
    sleep(Duration::from_secs(5)).await;

    // Subscribe to verify group exists and admin has proper permissions
    let verify_client = Client::new(admin_config.keys.clone());
    verify_client.add_relay(&admin_config.url).await?;
    verify_client.connect().await;
    sleep(Duration::from_secs(2)).await;
    authenticate_client(&verify_client).await?;

    let group_filter = Filter::new()
        .kinds(vec![Kind::Custom(39000)]) // Group state event
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec![&group_id]);

    let Output {
        val: _verify_sub_id,
        ..
    } = verify_client.subscribe(vec![group_filter], None).await?;
    info!("Waiting for group state verification...");

    // Wait longer for group state to be initialized
    sleep(Duration::from_secs(10)).await;

    // Create invite for half of the clients
    let invite_code = generate_invite_code();
    let mut admin_config_with_invite = admin_config.clone();
    admin_config_with_invite.invite_code = Some(invite_code.clone());
    let invite_builder =
        generate_test_event(&admin_config_with_invite, GroupEventKind::CreateInvite).await?;

    let invite_client = Client::new(admin_config.keys.clone());
    invite_client.add_relay(&admin_config.url).await?;
    invite_client.connect().await;

    // Wait for connection and authenticate
    wait_for_connection(&invite_client).await?;
    authenticate_client(&invite_client).await?;

    // Verify group exists before creating invite
    let verify_filter = Filter::new()
        .kinds(vec![Kind::Custom(39000)]) // Group state event
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec![&group_id]);

    let Output {
        val: verify_sub_id, ..
    } = invite_client.subscribe(vec![verify_filter], None).await?;
    info!("Verifying group state before creating invite...");

    // Create a channel to receive the verification event
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let sub_id_clone = verify_sub_id.clone();

    // Handle notifications in a separate task
    let _notification_handle = tokio::spawn({
        let client = invite_client.clone();
        let tx = tx.clone();
        async move {
            client
                .handle_notifications(move |notification| {
                    let tx = tx.clone();
                    let sub_id = sub_id_clone.clone();
                    async move {
                        if let RelayPoolNotification::Event {
                            subscription_id,
                            event,
                            ..
                        } = notification
                        {
                            if subscription_id == sub_id {
                                info!("Received group state verification event: {}", event.id);
                                let _ = tx.send(event).await;
                                return Ok(true); // Stop handling notifications
                            }
                        }
                        Ok(false)
                    }
                })
                .await
        }
    });

    // Wait for verification event
    let mut verified = false;
    for i in 0..5 {
        info!("Waiting for group state verification, attempt {}", i + 1);
        match tokio::time::timeout(Duration::from_secs(2u64.pow(i)), rx.recv()).await {
            Ok(Some(_)) => {
                verified = true;
                info!("Group state verified after {} attempts", i + 1);
                break;
            }
            Ok(None) => {
                info!("Channel closed without receiving verification");
                break;
            }
            Err(_) => {
                info!("Timeout waiting for verification");
                continue;
            }
        }
    }

    if !verified {
        anyhow::bail!("Failed to verify group state before creating invite");
    }

    // Wait a bit longer for all internal events to be processed
    sleep(Duration::from_secs(5)).await;

    // Unsubscribe from verification subscription
    invite_client.unsubscribe(verify_sub_id).await;

    // Subscribe to verify the invite creation
    let invite_filter = Filter::new()
        .kinds(vec![Kind::Custom(GroupEventKind::CreateInvite as u16)])
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec![&group_id]);

    let Output {
        val: invite_sub_id, ..
    } = invite_client.subscribe(vec![invite_filter], None).await?;
    info!("Subscribed to invite creation events");

    // Create a channel to receive the invite event
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let sub_id_clone = invite_sub_id.clone();

    // Handle notifications in a separate task
    let _notification_handle = tokio::spawn({
        let client = invite_client.clone();
        let tx = tx.clone();
        async move {
            client
                .handle_notifications(move |notification| {
                    let tx = tx.clone();
                    let sub_id = sub_id_clone.clone();
                    async move {
                        if let RelayPoolNotification::Event {
                            subscription_id,
                            event,
                            ..
                        } = notification
                        {
                            if subscription_id == sub_id {
                                info!("Received invite creation event: {}", event.id);
                                let _ = tx.send(event).await;
                                return Ok(true); // Stop handling notifications
                            }
                        }
                        Ok(false)
                    }
                })
                .await
        }
    });

    let _invite_output = invite_client.send_event_builder(invite_builder).await?;
    info!("Created invite with code: {}", invite_code);

    // Wait for invite event confirmation with timeout
    let mut received_invite = false;
    for i in 0..5 {
        info!(
            "Waiting for invite creation confirmation, attempt {}",
            i + 1
        );
        match tokio::time::timeout(Duration::from_secs(2u64.pow(i)), rx.recv()).await {
            Ok(Some(_)) => {
                received_invite = true;
                info!("Received invite creation event after {} attempts", i + 1);
                break;
            }
            Ok(None) => {
                info!("Channel closed without receiving invite confirmation");
                break;
            }
            Err(_) => {
                info!("Timeout waiting for invite confirmation");
                continue;
            }
        }
    }

    if !received_invite {
        anyhow::bail!("Timeout waiting for invite creation event");
    }

    // Wait a bit longer for all internal events to be processed
    sleep(Duration::from_secs(5)).await;

    // Unsubscribe from invite events
    invite_client.unsubscribe(invite_sub_id).await;

    // Split clients between invite-based and manual joins
    let num_invite_clients = args.clients / 2;

    // Spawn non-member clients that will join with invite
    for i in 0..num_invite_clients {
        let client_metrics = metrics.clone();
        let mut config =
            ClientConfig::new(args.url.clone(), Keys::generate(), ClientRole::NonMember);
        config.group_id = Some(group_id.clone());
        config.invite_code = Some(invite_code.clone());
        let mut shutdown_rx = shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            tokio::select! {
                result = run_client(config, client_metrics) => {
                    if let Err(e) = result {
                        error!("Invite client {} error: {}", i, e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Invite client {} received shutdown signal", i);
                }
            }
        });
        handles.push(handle);
    }

    // Spawn non-member clients that will request manual join
    for i in num_invite_clients..args.clients {
        let client_metrics = metrics.clone();
        let mut config =
            ClientConfig::new(args.url.clone(), Keys::generate(), ClientRole::NonMember);
        config.group_id = Some(group_id.clone());
        let mut shutdown_rx = shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            tokio::select! {
                result = run_client(config, client_metrics) => {
                    if let Err(e) = result {
                        error!("Manual join client {} error: {}", i, e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Manual join client {} received shutdown signal", i);
                }
            }
        });
        handles.push(handle);
    }

    // Spawn admin task to handle join requests and group management
    let admin_metrics = metrics.clone();
    let mut admin_shutdown_rx = shutdown_tx.subscribe();
    let admin_handle = tokio::spawn(async move {
        tokio::select! {
            result = run_client(admin_config, admin_metrics) => {
                if let Err(e) = result {
                    error!("Admin client error: {}", e);
                }
            }
            _ = admin_shutdown_rx.recv() => {
                info!("Admin client received shutdown signal");
            }
        }
    });
    handles.push(admin_handle);

    // Wait for either test duration or shutdown signal
    let mut shutdown_rx = shutdown_tx.subscribe();
    tokio::select! {
        _ = sleep(Duration::from_secs(args.duration)) => {
            info!("Test duration completed");
        }
        _ = shutdown_rx.recv() => {
            info!("Received shutdown signal in main");
        }
    }

    // Print final metrics
    let final_metrics = metrics.lock().await;
    info!("Load test completed. Final metrics:");
    info!("Events sent: {}", final_metrics.events_sent);
    info!("Events received: {}", final_metrics.events_received);
    info!("Errors: {}", final_metrics.errors);
    if let Some(avg_latency) = final_metrics.average_latency() {
        info!("Average latency: {:?}", avg_latency);
    }

    // Wait for all clients to finish
    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

/// Simulate a single WebSocket client with basic message sending and metrics collection
async fn run_client(config: ClientConfig, metrics: Arc<Mutex<Metrics>>) -> Result<()> {
    let client = Client::new(config.keys.clone());
    client.add_relay(&config.url).await?;
    client.connect().await;
    sleep(Duration::from_secs(2)).await;

    // Authenticate before any operations
    authenticate_client(&client).await?;

    // Subscribe to our own events with auto-generated ID
    let subscription = Filter::new()
        .pubkey(config.keys.public_key())
        .kinds(vec![
            Kind::Custom(GroupEventKind::JoinRequest as u16),
            Kind::Custom(GroupEventKind::LeaveRequest as u16),
            Kind::Custom(GroupEventKind::EditMetadata as u16),
            Kind::Custom(GroupEventKind::CreateInvite as u16),
        ])
        .since(Timestamp::now());

    let Output { val: sub_id, .. } = client.subscribe(vec![subscription], None).await?;
    info!("Subscribed with ID: {}", sub_id);

    // Keep connection alive with periodic REQ messages
    let client_clone = client.clone();
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(30)).await;
            let keep_alive_filter = Filter::new().limit(1);
            if let Err(e) = client_clone.subscribe(vec![keep_alive_filter], None).await {
                error!("Keep-alive error: {}", e);
                break;
            }
        }
    });

    // Handle notifications in a separate task
    let metrics_clone = metrics.clone();
    let client_clone = client.clone();
    let sub_id_clone = sub_id.clone();
    let _notification_handle = tokio::spawn(async move {
        if let Err(e) = client_clone
            .handle_notifications(move |notification| {
                let metrics = metrics_clone.clone();
                let sub_id = sub_id_clone.clone();
                async move {
                    if let RelayPoolNotification::Event {
                        subscription_id,
                        event,
                        ..
                    } = notification
                    {
                        // Only process events from our subscription
                        if subscription_id == sub_id {
                            let mut metrics = metrics.lock().await;
                            metrics.events_received += 1;

                            // Calculate latency using timestamp difference
                            let now = Timestamp::now().as_u64();
                            let created_at = event.created_at.as_u64();
                            let latency = now.saturating_sub(created_at);
                            metrics.total_latency += Duration::from_secs(latency);

                            match event.kind {
                                Kind::Custom(k) if k == GroupEventKind::CreateGroup as u16 => {
                                    info!("Received group creation event: {}", event.id);
                                }
                                Kind::Custom(k) if k == GroupEventKind::JoinRequest as u16 => {
                                    info!("Received join request event: {}", event.id);
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(false) // Continue the notification loop
                }
            })
            .await
        {
            error!("Notification handler error: {}", e);
        }
    });

    // Generate and send test events
    loop {
        let event_kind = generate_event_kind_for_role(config.role);
        let builder = generate_test_event(&config, event_kind).await?;
        let output = client.send_event_builder(builder).await?;

        {
            let mut metrics = metrics.lock().await;
            metrics.events_sent += 1;

            if !output.failed.is_empty() {
                metrics.errors += output.failed.len();
                error!(
                    "Failed to send {:?} event to relays: {:?}",
                    event_kind, output.failed
                );
            }

            if !output.success.is_empty() {
                info!(
                    "Successfully sent {:?} event to relays: {:?}",
                    event_kind, output.success
                );
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

// TODO: Implement these additional functions in the next iteration:
// 1. ✓ setup_signal_handler() for graceful shutdown
// 2. ✓ retry_connection() with exponential backoff (not needed for load testing)
// 3. ✓ implement_auth() for NIP-42 authentication
// 4. ✓ create_group() to set up test groups
// 5. ✓ generate_group_events() for more varied event types
//
// Additional TODOs:
// 6. Add admin approval functionality for join requests
// 7. Add group metadata update testing
// 8. Add concurrent group creation/deletion stress testing
// 9. Add metrics for event processing latency per event type
// 10. Add configurable rate limiting for event generation
// 11. Add support for testing group permissions and role changes
// 12. Add simulation of realistic user behavior patterns
