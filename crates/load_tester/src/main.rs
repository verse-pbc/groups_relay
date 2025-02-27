use anyhow::Result;
use clap::Parser;
use nostr_sdk::prelude::*;
use rand::Rng;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
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
    GroupMessage = 9,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientRole {
    Admin,
    Member,
    NonMember,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ClientState {
    Initial,
    Joined,
    MessagesComplete,
    Left,
}

#[derive(Debug, Default)]
struct Metrics {
    events_sent: usize,
    errors: usize,
    active_clients: usize,
    finished_clients: usize,
    event_latencies: HashMap<GroupEventKind, (Duration, usize)>,
}

impl Metrics {
    fn new() -> Self {
        Self::default()
    }

    fn record_event_latency(&mut self, event_kind: GroupEventKind, latency: Duration) {
        let entry = self
            .event_latencies
            .entry(event_kind)
            .or_insert((Duration::default(), 0));
        entry.0 += latency;
        entry.1 += 1;
    }

    fn average_latency_by_event(&self, event_kind: GroupEventKind) -> Option<Duration> {
        self.event_latencies.get(&event_kind).map(|(total, count)| {
            if *count == 0 {
                Duration::default()
            } else {
                *total / *count as u32
            }
        })
    }

    fn mark_client_started(&mut self) {
        self.active_clients += 1;
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(short, long, default_value = "2")]
    clients: usize,
    #[arg(short, long, default_value = "ws://0.0.0.0:8080")]
    url: String,
    #[arg(short, long, default_value = "10")]
    duration: u64,
    #[arg(short, long, default_value = "1")]
    groups: usize,
}

#[derive(Debug, Clone)]
struct ClientConfig {
    url: String,
    keys: Keys,
    group_id: Option<String>,
    role: ClientRole,
    invite_code: Option<String>,
    state: ClientState,
    messages_to_send: usize,
}

impl ClientConfig {
    fn new(url: String, keys: Keys, role: ClientRole) -> Self {
        Self {
            url,
            keys,
            group_id: None,
            role,
            invite_code: None,
            state: ClientState::Initial,
            messages_to_send: 3,
        }
    }
}

async fn generate_test_event(config: &ClientConfig, kind: GroupEventKind) -> Result<EventBuilder> {
    match (config.role, kind, config.state) {
        (ClientRole::Admin, GroupEventKind::CreateGroup, _)
        | (ClientRole::Admin, GroupEventKind::EditMetadata, _)
        | (ClientRole::Admin, GroupEventKind::CreateInvite, _)
        | (ClientRole::Admin, GroupEventKind::PutUser, _)
        | (ClientRole::Admin, GroupEventKind::RemoveUser, _)
        | (ClientRole::Admin, GroupEventKind::DeleteEvent, _)
        | (ClientRole::Admin, GroupEventKind::DeleteGroup, _) => {}
        (_, GroupEventKind::LeaveRequest, ClientState::MessagesComplete) => {}
        (_, GroupEventKind::GroupMessage, ClientState::Joined) => {}
        (ClientRole::NonMember, GroupEventKind::JoinRequest, ClientState::Initial) => {}
        _ => anyhow::bail!("Invalid event kind"),
    }

    let content = match kind {
        GroupEventKind::CreateGroup => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "name": "Test Group",
                "about": "A test group",
                "picture": "https://example.com/pic.jpg",
                "id": group_id
            })
            .to_string()
        }
        GroupEventKind::GroupMessage => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "content": format!("Test message {} from {}", rand::thread_rng().gen::<u32>(), config.keys.public_key()),
                "group_id": group_id
            }).to_string()
        }
        GroupEventKind::EditMetadata => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "name": "Test Group",
                "about": "A test group",
                "picture": "https://example.com/pic.jpg",
                "private": false,
                "closed": false,
                "group_id": group_id
            })
            .to_string()
        }
        GroupEventKind::CreateInvite => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            let invite_code = config
                .invite_code
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Invite code required"))?;
            json!({
                "type": "invite",
                "group_id": group_id,
                "code": invite_code,
                "roles": ["member"],
                "expires_at": Timestamp::now().as_u64() + 86400
            })
            .to_string()
        }
        GroupEventKind::JoinRequest => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            if let Some(code) = &config.invite_code {
                json!({
                    "type": "join_request",
                    "group_id": group_id,
                    "code": code,
                    "message": "Request to join"
                })
                .to_string()
            } else {
                json!({
                    "type": "join_request",
                    "group_id": group_id,
                    "message": "Manual join request"
                })
                .to_string()
            }
        }
        GroupEventKind::LeaveRequest => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "type": "leave_request",
                "group_id": group_id,
                "message": "Leaving group"
            })
            .to_string()
        }
        GroupEventKind::PutUser => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "type": "add_user",
                "group_id": group_id,
                "roles": ["member"]
            })
            .to_string()
        }
        GroupEventKind::RemoveUser => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "type": "remove_user",
                "group_id": group_id
            })
            .to_string()
        }
        GroupEventKind::DeleteEvent => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "type": "delete_event",
                "group_id": group_id
            })
            .to_string()
        }
        GroupEventKind::DeleteGroup => {
            let group_id = config
                .group_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Group ID required"))?;
            json!({
                "type": "delete_group",
                "group_id": group_id
            })
            .to_string()
        }
    };

    let mut builder = EventBuilder::new(Kind::Custom(kind as u16), content);
    if let Some(group_id) = &config.group_id {
        builder = builder.tag(Tag::custom(TagKind::custom("h"), [group_id]));
        match kind {
            GroupEventKind::CreateGroup | GroupEventKind::EditMetadata => {
                builder = builder
                    .tag(Tag::custom(TagKind::custom("d"), [group_id]))
                    .tag(Tag::custom(TagKind::custom("public"), &[] as &[String]))
                    .tag(Tag::custom(TagKind::custom("open"), &[] as &[String]));
            }
            GroupEventKind::CreateInvite | GroupEventKind::JoinRequest => {
                if let Some(code) = &config.invite_code {
                    builder = builder.tag(Tag::custom(TagKind::custom("code"), [code]));
                }
            }
            _ => {}
        }
    }
    Ok(builder)
}

async fn wait_for_connection(client: &Client) -> Result<()> {
    let mut attempts = 0;
    while attempts < 5 {
        if client
            .relays()
            .await
            .iter()
            .any(|(_, relay)| relay.status() == RelayStatus::Connected)
        {
            return Ok(());
        }
        sleep(Duration::from_millis(100 * 2u64.pow(attempts))).await;
        attempts += 1;
    }
    anyhow::bail!("Failed to connect after 5 attempts")
}

async fn authenticate_client(client: &Client) -> Result<()> {
    wait_for_connection(client).await?;
    if client
        .relays()
        .await
        .iter()
        .any(|(_, relay)| relay.status() != RelayStatus::Connected)
    {
        anyhow::bail!("Not connected after authentication");
    }
    Ok(())
}

async fn wait_for_event2<F>(
    client: &Client,
    filter: Filter,
    timeout_dur: Duration,
    predicate: F,
) -> Result<Option<Event>>
where
    F: Fn(&Event) -> bool,
{
    let result = client.subscribe(filter.clone(), None).await?;
    let sub_id = result.val;
    let mut notifications = client.notifications();
    let timeout_fut = sleep(timeout_dur);
    tokio::pin!(timeout_fut);
    loop {
        tokio::select! {
            Ok(notification) = notifications.recv() => {
                if let RelayPoolNotification::Message { message, .. } = notification {
                    let RelayMessage::Event { event, subscription_id, .. } = message else { continue };
                    if subscription_id != sub_id { continue };
                    if predicate(&event) {
                        client.unsubscribe(sub_id).await;
                        return Ok(Some(*event));
                    }
                }
            }
            _ = &mut timeout_fut => {
                client.unsubscribe(sub_id).await;
                return Ok(None);
            }
        }
    }
}

async fn join_group(
    client: &Client,
    config: &mut ClientConfig,
    metrics: &Arc<Mutex<Metrics>>,
    _group_id: &str,
    membership_filter: &Filter,
) -> Result<()> {
    if config.state != ClientState::Initial {
        return Ok(());
    }
    let join_request = generate_test_event(config, GroupEventKind::JoinRequest).await?;
    let start = Instant::now();
    let _output = client.send_event_builder(join_request).await?;
    let latency = start.elapsed();
    metrics.lock().await.events_sent += 1;
    metrics
        .lock()
        .await
        .record_event_latency(GroupEventKind::JoinRequest, latency);

    let event = wait_for_event2(
        client,
        membership_filter.clone(),
        Duration::from_secs(30),
        |event| {
            event.tags.iter().any(|tag| {
                tag.kind() == TagKind::p()
                    && tag
                        .as_slice()
                        .get(1)
                        .map(|v| v == &config.keys.public_key().to_string())
                        .unwrap_or(false)
            })
        },
    )
    .await?;

    if event.is_some() {
        config.state = ClientState::Joined;
    } else {
        metrics.lock().await.errors += 1;
        return Err(anyhow::anyhow!("Failed to join group"));
    }
    Ok(())
}

async fn send_messages(
    client: &Client,
    config: &mut ClientConfig,
    metrics: &Arc<Mutex<Metrics>>,
    _group_id: &str,
) -> Result<()> {
    if config.state != ClientState::Joined {
        return Ok(());
    }
    for _ in 0..config.messages_to_send {
        let message_event = generate_test_event(config, GroupEventKind::GroupMessage).await?;
        let start = Instant::now();
        client.send_event_builder(message_event).await?;
        let latency = start.elapsed();
        metrics.lock().await.events_sent += 1;
        metrics
            .lock()
            .await
            .record_event_latency(GroupEventKind::GroupMessage, latency);
    }
    config.state = ClientState::MessagesComplete;
    Ok(())
}

async fn leave_group(
    client: &Client,
    config: &mut ClientConfig,
    metrics: &Arc<Mutex<Metrics>>,
    _group_id: &str,
    membership_filter: &Filter,
) -> Result<()> {
    if config.state != ClientState::MessagesComplete {
        return Ok(());
    }
    let leave_request = generate_test_event(config, GroupEventKind::LeaveRequest).await?;
    let start = Instant::now();
    client.send_event_builder(leave_request).await?;
    let latency = start.elapsed();
    metrics.lock().await.events_sent += 1;
    metrics
        .lock()
        .await
        .record_event_latency(GroupEventKind::LeaveRequest, latency);

    let event = wait_for_event2(
        client,
        membership_filter.clone(),
        Duration::from_secs(30),
        |event| {
            !event.tags.iter().any(|tag| {
                tag.kind() == TagKind::p()
                    && tag
                        .as_slice()
                        .get(1)
                        .map(|v| v == &config.keys.public_key().to_string())
                        .unwrap_or(false)
            })
        },
    )
    .await?;

    if event.is_some() {
        config.state = ClientState::Left;
    } else {
        metrics.lock().await.errors += 1;
        return Err(anyhow::anyhow!("Failed to leave group"));
    }
    Ok(())
}

async fn run_client(mut config: ClientConfig, metrics: Arc<Mutex<Metrics>>) -> Result<()> {
    let client = Client::new(config.keys.clone());
    client.add_relay(&config.url).await?;
    client.connect().await;
    authenticate_client(&client).await?;
    metrics.lock().await.mark_client_started();

    let group_id = config
        .group_id
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Group ID required"))?
        .clone();
    let membership_filter = Filter::new()
        .kinds(vec![Kind::Custom(39002)])
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), &group_id);
    client.subscribe(membership_filter.clone(), None).await?;

    join_group(
        &client,
        &mut config,
        &metrics,
        &group_id,
        &membership_filter,
    )
    .await?;
    send_messages(&client, &mut config, &metrics, &group_id).await?;
    leave_group(
        &client,
        &mut config,
        &metrics,
        &group_id,
        &membership_filter,
    )
    .await?;

    client.disconnect().await;
    Ok(())
}

async fn create_groups(args: &Args) -> Result<Vec<ClientConfig>> {
    let mut group_configs = Vec::new();
    for i in 0..args.groups {
        let mut admin_config =
            ClientConfig::new(args.url.clone(), Keys::generate(), ClientRole::Admin);
        admin_config.group_id = Some(format!("group_{:x}", rand::thread_rng().gen::<u64>()));
        let group_id = admin_config.group_id.as_ref().unwrap().clone();
        let admin_client = Client::new(admin_config.keys.clone());
        admin_client.add_relay(&admin_config.url).await?;
        admin_client.connect().await;
        authenticate_client(&admin_client).await?;
        let filter = Filter::new()
            .kinds(vec![Kind::Custom(39002)])
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), &group_id);
        admin_client.subscribe(filter.clone(), None).await?;
        sleep(Duration::from_millis(100)).await;
        let create_event = generate_test_event(&admin_config, GroupEventKind::CreateGroup).await?;
        let output = admin_client.send_event_builder(create_event).await?;
        info!("Group {} creation sent: {:?}", i + 1, output.success);
        let event = wait_for_event2(&admin_client, filter, Duration::from_secs(30), |event| {
            event.tags.iter().any(|tag| {
                tag.kind() == TagKind::p()
                    && tag
                        .as_slice()
                        .get(1)
                        .map(|v| v == &admin_config.keys.public_key().to_string())
                        .unwrap_or(false)
            })
        })
        .await?;
        if event.is_none() {
            return Err(anyhow::anyhow!(
                "Failed to create group {} within timeout",
                i + 1
            ));
        }
        info!("Group {} confirmed", i + 1);
        let metadata_event =
            generate_test_event(&admin_config, GroupEventKind::EditMetadata).await?;
        let output = admin_client.send_event_builder(metadata_event).await?;
        info!("Group {} metadata update sent: {:?}", i + 1, output.success);
        group_configs.push(admin_config);
    }
    Ok(group_configs)
}

async fn spawn_clients(
    config: ClientConfig,
    count: u32,
    start_index: u32,
    metrics: Arc<Mutex<Metrics>>,
    shutdown: &CancellationToken,
) -> Vec<JoinHandle<Result<()>>> {
    let mut handles = Vec::new();
    for i in 0..count {
        let client_config = config.clone();
        let metrics_clone = metrics.clone();
        let _shutdown = shutdown.clone();
        let handle = tokio::spawn(async move {
            let res = run_client(client_config, metrics_clone).await;
            if let Err(e) = &res {
                error!("Client {} failed: {}", start_index + i, e);
            }
            res
        });
        handles.push(handle);
    }
    handles
}

async fn wait_for_completion(
    metrics: Arc<Mutex<Metrics>>,
    duration: u64,
    shutdown: &CancellationToken,
) -> Result<()> {
    let timeout_fut = sleep(Duration::from_secs(duration));
    tokio::pin!(timeout_fut);
    let mut check_interval = interval(Duration::from_millis(100));
    let mut waiting_since = None;
    let max_wait = Duration::from_secs(5);
    loop {
        tokio::select! {
            _ = &mut timeout_fut => break,
            _ = shutdown.cancelled() => break,
            _ = check_interval.tick() => {
                let m = metrics.lock().await;
                if m.active_clients > 0 && m.finished_clients >= m.active_clients {
                    break;
                }
                if m.active_clients > 0 && m.events_sent > 0 {
                    if waiting_since.is_none() {
                        waiting_since = Some(Instant::now());
                    } else if waiting_since.unwrap().elapsed() > max_wait {
                        break;
                    }
                }
            }
        }
    }
    sleep(Duration::from_secs(1)).await;
    Ok(())
}

async fn print_metrics(metrics: &Mutex<Metrics>) {
    let m = metrics.lock().await;
    info!("Load test completed. Final metrics:");
    info!("Events sent: {}", m.events_sent);
    info!("Errors: {}", m.errors);
    info!("Active clients: {}", m.active_clients);
    info!("Finished clients: {}", m.finished_clients);
    if m.active_clients > m.finished_clients {
        error!(
            "{} clients did not complete",
            m.active_clients - m.finished_clients
        );
    }
    for kind in [
        GroupEventKind::CreateGroup,
        GroupEventKind::EditMetadata,
        GroupEventKind::JoinRequest,
        GroupEventKind::GroupMessage,
        GroupEventKind::LeaveRequest,
    ] {
        if let Some(latency) = m.average_latency_by_event(kind) {
            let (total, count) = m.event_latencies.get(&kind).unwrap();
            info!(
                "{:?}: {:?} avg ({} events, total: {:?})",
                kind, latency, count, total
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let metrics = Arc::new(Mutex::new(Metrics::new()));
    let mut handles = Vec::new();
    let shutdown_token = CancellationToken::new();

    let group_configs = create_groups(&args).await?;
    let clients_per_group = args.clients / args.groups;
    let remainder = args.clients % args.groups;
    for (i, admin_config) in group_configs.iter().enumerate() {
        let group_clients = if i < remainder {
            clients_per_group + 1
        } else {
            clients_per_group
        };
        if group_clients > 0 {
            info!(
                "Spawning {} clients for group {}",
                group_clients,
                admin_config.group_id.as_ref().unwrap()
            );
            for j in 0..group_clients {
                let mut client_config =
                    ClientConfig::new(args.url.clone(), Keys::generate(), ClientRole::NonMember);
                client_config.group_id = admin_config.group_id.clone();
                handles.extend(
                    spawn_clients(
                        client_config,
                        1,
                        (i as u32) * (clients_per_group as u32) + j as u32,
                        metrics.clone(),
                        &shutdown_token,
                    )
                    .await,
                );
            }
        }
    }

    wait_for_completion(metrics.clone(), args.duration, &shutdown_token).await?;
    print_metrics(&metrics).await;
    shutdown_token.cancel();

    let timeout_duration = Duration::from_secs(5);
    for handle in handles {
        match timeout(timeout_duration, handle).await {
            Ok(Ok(Ok(()))) => (),
            Ok(Ok(Err(e))) => error!("Client task failed: {}", e),
            Ok(Err(e)) => error!("Client task panicked: {}", e),
            Err(e) => error!("Client task timed out: {}", e),
        }
    }
    Ok(())
}
