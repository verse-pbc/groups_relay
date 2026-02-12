//! Deadlock torture test
//!
//! This binary stress tests the components suspected of causing deadlocks:
//! 1. LMDB with slow queries via spawn_blocking
//! 2. Concurrent subscription operations with RwLock
//! 3. Diagnostics collection during high load
//!
//! Run with: cargo run --bin deadlock_torture

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use tokio::sync::RwLock;
use tokio::task::JoinSet;

/// Simulates the subscription registry's RwLock pattern
struct FakeSubscriptionRegistry {
    /// RwLock for subscriptions (like in relay_builder)
    subscriptions: RwLock<Vec<String>>,
    /// Counter for operations
    ops_count: AtomicUsize,
}

impl FakeSubscriptionRegistry {
    fn new() -> Self {
        Self {
            subscriptions: RwLock::new(Vec::new()),
            ops_count: AtomicUsize::new(0),
        }
    }

    async fn add_subscription(&self, id: String) {
        let mut subs = self.subscriptions.write().await;
        subs.push(id);
        self.ops_count.fetch_add(1, Ordering::Relaxed);
    }

    async fn remove_subscription(&self) {
        let mut subs = self.subscriptions.write().await;
        subs.pop();
        self.ops_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Simulates get_diagnostics - reads while iterating
    async fn collect_diagnostics(&self) -> usize {
        let subs = self.subscriptions.read().await;
        let count = subs.len();
        // Simulate some work while holding the lock
        tokio::time::sleep(Duration::from_micros(100)).await;
        self.ops_count.fetch_add(1, Ordering::Relaxed);
        count
    }
}

/// Test 1: Pure spawn_blocking saturation
async fn test_blocking_pool_saturation(running: Arc<AtomicBool>) {
    println!("\n=== Test 1: spawn_blocking pool saturation ===");

    let completed = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let mut handles = JoinSet::new();

    // Spawn many blocking tasks that simulate slow LMDB operations
    for i in 0..100 {
        let completed = Arc::clone(&completed);
        let started = Arc::clone(&started);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            while running.load(Ordering::Relaxed) {
                started.fetch_add(1, Ordering::Relaxed);

                // Simulate slow LMDB query (like the 3-4 second ones we saw)
                let result = tokio::task::spawn_blocking(move || {
                    std::thread::sleep(Duration::from_millis(100 + (i % 50) as u64));
                    42
                })
                .await;

                if result.is_ok() {
                    completed.fetch_add(1, Ordering::Relaxed);
                }

                // Small yield
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }

    // Monitor progress
    let start = Instant::now();
    while running.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let s = started.load(Ordering::Relaxed);
        let c = completed.load(Ordering::Relaxed);
        println!(
            "  [{:>3}s] started: {}, completed: {}, pending: {}",
            start.elapsed().as_secs(),
            s,
            c,
            s.saturating_sub(c)
        );

        // Check for stall
        if s > 0 && c == 0 && start.elapsed() > Duration::from_secs(5) {
            println!("  WARNING: Possible stall detected - tasks started but none completed!");
        }
    }

    handles.abort_all();
    println!("  Test 1 complete");
}

/// Test 2: RwLock contention (subscription registry pattern)
async fn test_rwlock_contention(running: Arc<AtomicBool>) {
    println!("\n=== Test 2: RwLock contention (subscription registry pattern) ===");

    let registry = Arc::new(FakeSubscriptionRegistry::new());
    let mut handles = JoinSet::new();

    // Writers - add/remove subscriptions
    for i in 0..20 {
        let registry = Arc::clone(&registry);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            let mut count = 0;
            while running.load(Ordering::Relaxed) {
                registry.add_subscription(format!("sub_{i}_{count}")).await;
                tokio::time::sleep(Duration::from_micros(100)).await;
                registry.remove_subscription().await;
                count += 1;
            }
        });
    }

    // Readers - collect diagnostics (like the diagnostics task)
    for _ in 0..5 {
        let registry = Arc::clone(&registry);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            while running.load(Ordering::Relaxed) {
                let _count = registry.collect_diagnostics().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }

    // Monitor
    let start = Instant::now();
    while running.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let ops = registry.ops_count.load(Ordering::Relaxed);
        println!(
            "  [{:>3}s] operations: {}, rate: {}/s",
            start.elapsed().as_secs(),
            ops,
            ops / start.elapsed().as_secs().max(1) as usize
        );
    }

    handles.abort_all();
    println!("  Test 2 complete");
}

/// Test 3: Combined - spawn_blocking + RwLock (the likely culprit)
async fn test_combined_blocking_and_rwlock(running: Arc<AtomicBool>) {
    println!("\n=== Test 3: Combined spawn_blocking + RwLock contention ===");

    let registry = Arc::new(FakeSubscriptionRegistry::new());
    let blocking_completed = Arc::new(AtomicUsize::new(0));
    let mut handles = JoinSet::new();

    // Simulate slow LMDB operations in spawn_blocking
    for i in 0..50 {
        let blocking_completed = Arc::clone(&blocking_completed);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            while running.load(Ordering::Relaxed) {
                // Simulate slow DB query
                let _ = tokio::task::spawn_blocking(move || {
                    std::thread::sleep(Duration::from_millis(50 + (i % 100) as u64));
                })
                .await;
                blocking_completed.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    // Subscription writers
    for i in 0..20 {
        let registry = Arc::clone(&registry);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            let mut count = 0;
            while running.load(Ordering::Relaxed) {
                registry.add_subscription(format!("sub_{i}_{count}")).await;
                tokio::time::sleep(Duration::from_micros(500)).await;
                registry.remove_subscription().await;
                count += 1;
            }
        });
    }

    // Diagnostics collector (runs periodically like the real one)
    {
        let registry = Arc::clone(&registry);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            while running.load(Ordering::Relaxed) {
                let _count = registry.collect_diagnostics().await;
                // Every 100ms like an aggressive diagnostics interval
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    }

    // Monitor for stalls
    let start = Instant::now();
    let mut last_blocking = 0;
    let mut stall_count = 0;

    while running.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(60) {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let blocking = blocking_completed.load(Ordering::Relaxed);
        let registry_ops = registry.ops_count.load(Ordering::Relaxed);

        let blocking_delta = blocking - last_blocking;
        last_blocking = blocking;

        println!(
            "  [{:>3}s] blocking ops: {} (+{}), registry ops: {}",
            start.elapsed().as_secs(),
            blocking,
            blocking_delta,
            registry_ops
        );

        // Detect stall - if blocking operations stopped progressing
        if blocking_delta == 0 && start.elapsed() > Duration::from_secs(3) {
            stall_count += 1;
            println!(
                "  WARNING: No blocking operations completed this second! (stall count: {})",
                stall_count
            );
            if stall_count >= 5 {
                println!("  DEADLOCK DETECTED: 5 consecutive seconds with no progress!");
                break;
            }
        } else {
            stall_count = 0;
        }
    }

    handles.abort_all();
    println!("  Test 3 complete");
}

/// Test 4: Real LMDB with concurrent operations
async fn test_real_lmdb(running: Arc<AtomicBool>) {
    println!("\n=== Test 4: Real LMDB stress test ===");

    // Use a temp directory in /tmp
    let db_path = format!("/tmp/deadlock_torture_test_{}", std::process::id());

    // Clean up if exists
    let _ = std::fs::remove_dir_all(&db_path);

    // Create database
    let db: Arc<groups_relay::RelayDatabase> =
        match groups_relay::RelayDatabase::new(db_path.clone()).await {
            Ok(db) => Arc::new(db),
            Err(e) => {
                println!("  Failed to create database: {}", e);
                return;
            }
        };

    let keys = Keys::generate();
    let write_count = Arc::new(AtomicUsize::new(0));
    let read_count = Arc::new(AtomicUsize::new(0));
    let mut handles = JoinSet::new();

    // Writers
    for i in 0..10 {
        let db: Arc<groups_relay::RelayDatabase> = Arc::clone(&db);
        let keys = keys.clone();
        let write_count = Arc::clone(&write_count);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            let mut n = 0;
            while running.load(Ordering::Relaxed) {
                let event = EventBuilder::text_note(format!("Test event {} from writer {}", n, i))
                    .sign_with_keys(&keys)
                    .expect("Failed to sign");

                match db.save_event(&event, &Scope::Default).await {
                    Ok(_) => {
                        write_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        println!("  Write error: {}", e);
                    }
                }
                n += 1;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }

    // Readers with various filter patterns (simulating the slow queries we saw)
    for kind in [1, 1984, 4550, 20000, 23333, 34550] {
        let db: Arc<groups_relay::RelayDatabase> = Arc::clone(&db);
        let read_count = Arc::clone(&read_count);
        let running = Arc::clone(&running);

        handles.spawn(async move {
            while running.load(Ordering::Relaxed) {
                let filter = Filter::new().kind(Kind::from(kind)).limit(50);

                let start = Instant::now();
                match db.query(vec![filter], &Scope::Default).await {
                    Ok(_events) => {
                        let elapsed = start.elapsed();
                        read_count.fetch_add(1, Ordering::Relaxed);

                        if elapsed > Duration::from_millis(100) {
                            println!("  Slow query for kind {}: {:?}", kind, elapsed);
                        }
                    }
                    Err(e) => {
                        println!("  Read error: {}", e);
                    }
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
    }

    // Monitor
    let start = Instant::now();
    while running.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let w = write_count.load(Ordering::Relaxed);
        let r = read_count.load(Ordering::Relaxed);
        println!(
            "  [{:>3}s] writes: {}, reads: {}",
            start.elapsed().as_secs(),
            w,
            r
        );
    }

    handles.abort_all();

    // Cleanup
    let _ = std::fs::remove_dir_all(&db_path);

    println!("  Test 4 complete");
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    println!("Deadlock Torture Test");
    println!("=====================");
    println!("Runtime config: 8 worker threads (matching production)");
    println!("Press Ctrl+C to stop early\n");

    let running = Arc::new(AtomicBool::new(true));

    // Handle Ctrl+C
    let running_clone = Arc::clone(&running);
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        println!("\nShutting down...");
        running_clone.store(false, Ordering::Relaxed);
    });

    // Run tests sequentially
    test_blocking_pool_saturation(Arc::clone(&running)).await;

    if running.load(Ordering::Relaxed) {
        test_rwlock_contention(Arc::clone(&running)).await;
    }

    if running.load(Ordering::Relaxed) {
        test_combined_blocking_and_rwlock(Arc::clone(&running)).await;
    }

    if running.load(Ordering::Relaxed) {
        test_real_lmdb(Arc::clone(&running)).await;
    }

    println!("\n=== All tests complete ===");
}
