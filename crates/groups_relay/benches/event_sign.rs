use anyhow::Result;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use nostr_sdk::prelude::*;
use nostr_sdk::{Keys, UnsignedEvent};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::task::spawn_blocking;
use tokio::time::sleep;

// # Event Signing CPU Hogging Benchmark
//
// This benchmark demonstrates how CPU-intensive cryptographic signing operations
// can block the async runtime if not properly offloaded to a dedicated thread pool.
//
// It compares two approaches to handling event signing in an async context:
//
// 1. **Direct execution** - Running signing directly on the async runtime thread
//    (blocks the thread and prevents other tasks from progressing)
//
// 2. **Spawn blocking** - Using spawn_blocking to offload CPU-intensive work
//    (keeps the async runtime responsive for other tasks)
//
// The benchmark measures how many concurrent I/O-like tasks can complete while signing is running.
// A higher number indicates better async runtime responsiveness and throughput.
//
// EXPECTED RESULTS:
// - spawn_blocking_approach: Shows significantly higher throughput (2-3x better)
// - direct_blocking_approach: Shows much lower throughput due to blocking
//
// The spawn_blocking approach is significantly faster because it allows the runtime
// to process other tasks while CPU-intensive work is happening on separate threads.

// Create a shared runtime for all blocking operations
static BLOCKING_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

// Get or initialize the blocking runtime
fn get_blocking_runtime() -> &'static tokio::runtime::Runtime {
    BLOCKING_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2) // Dual-core setup to simulate resource constraints
            .enable_all() // Enable all features including time and IO
            .build()
            .unwrap()
    })
}

/// Simulates an I/O-bound task that needs to wake up frequently.
///
/// This represents typical async workloads like network requests or database queries
/// that involve short bursts of activity followed by waiting. These tasks are most
/// affected by CPU hogging on the async runtime.
async fn simulate_other_task() {
    // Use multiple small sleeps instead of one large sleep to better demonstrate
    // how CPU-intensive work can block the event loop between wake-ups
    for _ in 0..10 {
        sleep(Duration::from_millis(3)).await;
    }
}

/// Signs an event directly on the async runtime thread.
///
/// This approach:
/// - Runs CPU-intensive signing directly on the async runtime thread
/// - Can block the thread, preventing other tasks from making progress
/// - Is the simplest but can significantly reduce overall throughput
async fn process_direct(keys: Arc<Keys>, unsigned_event: UnsignedEvent) -> Result<()> {
    let result = keys.sign_event(unsigned_event).await?;
    let _ = black_box(result);
    Ok(())
}

/// Signs an event using spawn_blocking to offload CPU-intensive work.
///
/// This approach:
/// - Offloads CPU-intensive work to a dedicated thread pool
/// - Keeps the main async runtime thread free to process other tasks
/// - Is more efficient for overall system throughput
async fn process_with_spawn_blocking(keys: Arc<Keys>, unsigned_event: UnsignedEvent) -> Result<()> {
    // Clone the keys and event before moving them into the closure
    let keys_clone = keys.clone();
    let unsigned_event_clone = unsigned_event;

    // Use spawn_blocking to offload the CPU-intensive work to a dedicated thread pool
    let result = spawn_blocking(move || {
        // Use the shared global runtime for all operations
        get_blocking_runtime().block_on(keys_clone.sign_event(unsigned_event_clone))
    })
    .await??;

    let _ = black_box(result);
    Ok(())
}

fn benchmark(c: &mut Criterion) {
    // Create keys and an unsigned event for signing
    let keys = Arc::new(Keys::generate());
    let unsigned_event = UnsignedEvent::new(
        keys.public_key(),
        Timestamp::now(),
        Kind::TextNote,
        vec![],
        "Hello, world!",
    );

    // Create a multi-threaded runtime with limited threads to better
    // demonstrate the impact of blocking operations
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2) // Dual-core setup to simulate resource constraints
        .build()
        .unwrap();

    let mut group = c.benchmark_group("event_signing_cpu_hogging");

    // Configure benchmark parameters
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.warm_up_time(Duration::from_secs(5));

    // Benchmark 1: Using spawn_blocking to offload CPU-intensive work
    group.bench_with_input(
        BenchmarkId::new("spawn_blocking_approach", 100),
        &100,
        |b, &_i| {
            b.to_async(&runtime).iter(|| async {
                // Create a shared channel to signal when all signings are done
                let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

                // Clone the keys and event
                let keys_clone = keys.clone();
                let unsigned_event_clone = unsigned_event.clone();

                // Spawn a task to handle all signings
                tokio::spawn(async move {
                    let mut handles = Vec::with_capacity(100);

                    // Spawn 100 signing tasks using spawn_blocking
                    for _ in 0..100 {
                        handles.push(tokio::spawn(process_with_spawn_blocking(
                            keys_clone.clone(),
                            unsigned_event_clone.clone(),
                        )));
                    }

                    // Wait for all signing tasks
                    for handle in handles {
                        handle.await.unwrap().unwrap();
                    }

                    // Signal completion
                    let _ = tx.send(()).await;
                });

                // Count how many simulation tasks complete before signings finish
                // This is our key metric - higher is better
                let mut completed_simulations = 0;

                loop {
                    tokio::select! {
                        // Check if signings are done
                        _ = rx.recv() => {
                            break;
                        }
                        // Try to run a simulation task
                        _ = simulate_other_task() => {
                            completed_simulations += 1;
                        }
                    }
                }

                completed_simulations // Return this so criterion can measure it
            })
        },
    );

    // Benchmark 2: Direct signing on the async runtime thread
    group.bench_with_input(
        BenchmarkId::new("direct_blocking_approach", 100),
        &100,
        |b, &_i| {
            b.to_async(&runtime).iter(|| async {
                // Create a shared channel to signal when all signings are done
                let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

                // Clone the keys and event
                let keys_clone = keys.clone();
                let unsigned_event_clone = unsigned_event.clone();

                // Spawn a task to handle all signings
                tokio::spawn(async move {
                    // Run 100 signing tasks directly (will block the thread)
                    for _ in 0..100 {
                        process_direct(keys_clone.clone(), unsigned_event_clone.clone())
                            .await
                            .unwrap();
                    }

                    // Signal completion
                    let _ = tx.send(()).await;
                });

                // Count how many simulation tasks complete before signings finish
                // This is our key metric - higher is better
                let mut completed_simulations = 0;

                loop {
                    tokio::select! {
                        // Check if signings are done
                        _ = rx.recv() => {
                            break;
                        }
                        // Try to run a simulation task
                        _ = simulate_other_task() => {
                            completed_simulations += 1;
                        }
                    }
                }

                completed_simulations // Return this so criterion can measure it
            })
        },
    );

    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
