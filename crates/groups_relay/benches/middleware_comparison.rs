use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use groups_relay::groups::Groups;
use groups_relay::groups_event_processor::GroupsRelayProcessor;
use groups_relay::RelayDatabase;
use nostr_relay_builder::{EventProcessor, EventContext, RelayConfig};
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Create a test database and groups instance
async fn setup_bench() -> (tempfile::TempDir, Arc<RelayDatabase>, Arc<Groups>, Keys) {
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("bench_db");

    let admin_keys = Keys::generate();
    let database =
        Arc::new(RelayDatabase::new(db_path.to_str().unwrap(), admin_keys.clone()).unwrap());

    let groups = Arc::new(
        Groups::load_groups(database.clone(), admin_keys.public_key())
            .await
            .unwrap(),
    );

    (tmp_dir, database, groups, admin_keys)
}

/// Create test event
fn create_test_event(keys: &Keys, kind: u16, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::from(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

/// Create test groups and events for benchmarking
async fn create_test_data(
    groups: &Arc<Groups>,
    database: &Arc<RelayDatabase>,
    admin_keys: &Keys,
    num_groups: usize,
    members_per_group: usize,
) -> Vec<Event> {
    let mut events = Vec::new();

    // Create GroupsRelayProcessor for handling events
    let _config = RelayConfig::new("ws://bench", database.clone(), admin_keys.clone());
    let processor = Arc::new(GroupsRelayProcessor::new(
        groups.clone(),
        admin_keys.public_key(),
    ));

    // Create groups
    for i in 0..num_groups {
        let group_id = format!("bench_group_{}", i);
        let create_event = create_test_event(
            admin_keys,
            9007, // Group creation
            vec![
                Tag::custom(TagKind::h(), [&group_id]),
                Tag::custom(TagKind::d(), [&group_id]),
                Tag::custom(
                    TagKind::Custom("name".into()),
                    [&format!("Benchmark Group {}", i)],
                ),
                if i % 2 == 0 {
                    Tag::custom(TagKind::Custom("private".into()), [""])
                } else {
                    Tag::custom(TagKind::Custom("public".into()), [""])
                },
            ],
        );

        let admin_pk = admin_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(&admin_pk),
            subdomain: &nostr_lmdb::Scope::Default,
            relay_pubkey: &admin_pk,
        };

        processor
            .handle_event(create_event.clone(), &mut (), context)
            .await
            .unwrap();

        // Add members
        for j in 0..members_per_group {
            let member_keys = Keys::generate();
            let add_event = create_test_event(
                admin_keys,
                9000, // Add user
                vec![
                    Tag::custom(TagKind::h(), [&group_id]),
                    Tag::public_key(member_keys.public_key()),
                ],
            );

            processor.handle_event(add_event, &mut (), context).await.unwrap();

            // Create some messages from members
            if j < 5 {
                let msg_event = create_test_event(
                    &member_keys,
                    Kind::TextNote.as_u16(),
                    vec![
                        Tag::custom(TagKind::h(), [&group_id]),
                        Tag::custom(
                            TagKind::custom("content"),
                            [&format!("Message {} from member {}", i, j)],
                        ),
                    ],
                );
                events.push(msg_event);
            }
        }
    }

    events
}

/// Benchmark visibility checks - direct comparison
fn bench_visibility_direct(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_tmp_dir, database, groups, admin_keys) = rt.block_on(setup_bench());

    // Create test data
    let test_events = rt.block_on(create_test_data(&groups, &database, &admin_keys, 5, 10));

    let mut group = c.benchmark_group("visibility_direct");

    // Test different authentication states
    let test_cases = vec![
        ("authenticated_admin", Some(admin_keys.public_key())),
        ("authenticated_user", Some(Keys::generate().public_key())),
        ("unauthenticated", None),
    ];

    for (name, auth_pubkey) in test_cases {
        for (i, event) in test_events.iter().enumerate() {
            group.bench_with_input(
                BenchmarkId::new(format!("groups_logic_{}", name), i),
                event,
                |b, event| {
                    let processor = GroupsRelayProcessor::new(groups.clone(), admin_keys.public_key());

                    b.to_async(&rt).iter(|| async {
                        let context = EventContext {
                            authed_pubkey: auth_pubkey.as_ref(),
                            subdomain: &nostr_lmdb::Scope::Default,
                            relay_pubkey: &admin_keys.public_key(),
                        };

                        black_box(processor.can_see_event(event, &(), context))
                    });
                },
            );
        }
    }

    group.finish();
}

/// Benchmark different NIP-29 event types
fn bench_nip29_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_tmp_dir, database, groups, admin_keys) = rt.block_on(setup_bench());

    // Create test data
    rt.block_on(create_test_data(&groups, &database, &admin_keys, 3, 5));

    let user_keys = Keys::generate();

    // Create different event types for benchmarking
    let test_events = vec![
        (
            "group_create",
            create_test_event(
                &admin_keys,
                9007,
                vec![
                    Tag::custom(TagKind::h(), ["new_group"]),
                    Tag::custom(TagKind::d(), ["new_group"]),
                    Tag::custom(TagKind::Custom("name".into()), ["New Benchmark Group"]),
                ],
            ),
        ),
        (
            "user_add",
            create_test_event(
                &admin_keys,
                9000,
                vec![
                    Tag::custom(TagKind::h(), ["bench_group_0"]),
                    Tag::public_key(user_keys.public_key()),
                ],
            ),
        ),
        (
            "chat_message",
            create_test_event(
                &user_keys,
                Kind::TextNote.as_u16(),
                vec![
                    Tag::custom(TagKind::h(), ["bench_group_0"]),
                    Tag::custom(TagKind::custom("content"), ["Hello, group!"]),
                ],
            ),
        ),
        (
            "group_edit",
            create_test_event(
                &admin_keys,
                9002,
                vec![
                    Tag::custom(TagKind::h(), ["bench_group_0"]),
                    Tag::custom(TagKind::Custom("name".into()), ["Updated Group Name"]),
                ],
            ),
        ),
        (
            "user_remove",
            create_test_event(
                &admin_keys,
                9001,
                vec![
                    Tag::custom(TagKind::h(), ["bench_group_0"]),
                    Tag::public_key(user_keys.public_key()),
                ],
            ),
        ),
    ];

    let mut group = c.benchmark_group("nip29_operations");
    group.sample_size(50);

    for (name, event) in test_events {
        // Benchmark GroupsRelayLogic handle_event
        group.bench_function(format!("groups_logic_{}", name), |b| {
            let processor = GroupsRelayProcessor::new(groups.clone(), admin_keys.public_key());

            b.to_async(&rt).iter(|| async {
                let admin_pk = admin_keys.public_key();
                let context = EventContext {
                    authed_pubkey: Some(&admin_pk),
                    subdomain: &nostr_lmdb::Scope::Default,
                    relay_pubkey: &admin_pk,
                };

                black_box(processor.handle_event(event.clone(), &mut (), context).await)
            });
        });
    }

    group.finish();
}

/// Benchmark group operations
fn bench_group_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_tmp_dir, database, groups, admin_keys) = rt.block_on(setup_bench());

    // Create test data
    rt.block_on(create_test_data(&groups, &database, &admin_keys, 10, 20));

    let mut group = c.benchmark_group("group_operations");
    group.sample_size(20);

    // Create different types of group operations
    let operations = vec![
        ("get_group", "bench_group_0"),
        ("check_membership", "bench_group_1"),
        ("check_admin", "bench_group_2"),
    ];

    for (op_name, group_id) in operations {
        group.bench_function(op_name, |b| {
            b.to_async(&rt).iter(|| async {
                match op_name {
                    "get_group" => {
                        black_box(groups.get_group(&nostr_lmdb::Scope::Default, group_id));
                    }
                    "check_membership" => {
                        let user = admin_keys.public_key();
                        if let Some(group) = groups.get_group(&nostr_lmdb::Scope::Default, group_id) {
                            black_box(group.is_member(&user));
                        }
                    }
                    "check_admin" => {
                        let user = admin_keys.public_key();
                        if let Some(group) = groups.get_group(&nostr_lmdb::Scope::Default, group_id) {
                            black_box(group.is_admin(&user));
                        }
                    }
                    _ => {}
                }
            });
        });
    }

    // Benchmark filter operations
    group.bench_function("filter_by_group", |b| {
        let filter = Filter::new()
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), "bench_group_0");

        b.to_async(&rt).iter(|| async {
            let processor = GroupsRelayProcessor::new(groups.clone(), admin_keys.public_key());

            let admin_pk = admin_keys.public_key();
            let context = EventContext {
                authed_pubkey: Some(&admin_pk),
                subdomain: &nostr_lmdb::Scope::Default,
                relay_pubkey: &admin_pk,
            };

            black_box(processor.verify_filters(&[filter.clone()], &(), context))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_visibility_direct,
    bench_nip29_operations,
    bench_group_operations
);
criterion_main!(benches);