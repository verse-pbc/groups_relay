use anyhow::Result;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nostr_sdk::{Event, EventBuilder, Keys};
use std::time::Duration;
use tokio::task::spawn_blocking;
use tokio::time::sleep;

// # Event Verification CPU Hogging Benchmark
//
// This benchmark demonstrates how CPU-intensive cryptographic verification operations
// can block the async runtime if not properly offloaded to a dedicated thread pool.
//
// It compares two approaches to handling event verification in an async context:
//
// 1. **Proper offloading** using `spawn_blocking` to move CPU-intensive work to a dedicated thread pool
// 2. **Direct execution** on the async runtime thread, which can block the thread and prevent other tasks from progressing
//
// The benchmark measures how many concurrent I/O-like tasks can complete while verification is running.
// A higher number indicates better async runtime responsiveness and throughput.

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

/// Processes event verification using the recommended approach:
/// Offloading CPU-intensive work to a dedicated thread pool.
///
/// This approach:
/// - Clones the event to make it owned by the blocking task
/// - Uses `spawn_blocking` to run verification on a separate thread pool
/// - Keeps the async runtime thread free to process other tasks
async fn process_with_cloning(event: Box<Event>) -> Result<()> {
    let event_owned = event.clone();
    let result = spawn_blocking(move || event_owned.verify()).await?;
    let _ = black_box(result);
    Ok(())
}

/// Processes event verification directly on the async runtime thread.
///
/// This approach:
/// - Runs CPU-intensive verification directly on the async runtime thread
/// - Can block the thread, preventing other tasks from making progress
/// - Is simpler but can significantly reduce overall throughput
async fn process_direct(event: Box<Event>) -> Result<()> {
    let result = event.verify();
    black_box(result?);
    Ok(())
}

fn benchmark(c: &mut Criterion) {
    // Create a test event for verification
    let keys = Keys::generate();
    let event = Box::new(
        EventBuilder::text_note("Hello, benchmarking!")
            .sign_with_keys(&keys)
            .expect("Failed to create event"),
    );

    // Create a multi-threaded runtime with limited threads to better
    // demonstrate the impact of blocking operations
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2) // Dual-core setup to simulate resource constraints
        .build()
        .unwrap();

    let mut group = c.benchmark_group("event_verification_cpu_hogging");

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
                // Create a shared channel to signal when all verifications are done
                let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

                // Clone the event before moving it into the closure
                let event_clone = event.clone();

                // Spawn a task to handle all verifications
                tokio::spawn(async move {
                    let mut handles = Vec::with_capacity(100);

                    // Spawn 100 verification tasks using spawn_blocking
                    for _ in 0..100 {
                        handles.push(tokio::spawn(process_with_cloning(event_clone.clone())));
                    }

                    // Wait for all verification tasks
                    for handle in handles {
                        handle.await.unwrap().unwrap();
                    }

                    // Signal completion
                    let _ = tx.send(()).await;
                });

                // Count how many simulation tasks complete before verifications finish
                // This is our key metric - higher is better
                let mut completed_simulations = 0;

                loop {
                    tokio::select! {
                        // Check if verifications are done
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

    // Benchmark 2: Running CPU-intensive work directly on the async runtime
    group.bench_with_input(
        BenchmarkId::new("direct_blocking_approach", 100),
        &100,
        |b, &_i| {
            b.to_async(&runtime).iter(|| async {
                // Create a shared channel to signal when all verifications are done
                let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

                // Clone the event before moving it into the closure
                let event_clone = event.clone();

                // Spawn a task to handle all verifications
                tokio::spawn(async move {
                    // Run 100 verification tasks directly (will block the thread)
                    for _ in 0..100 {
                        process_direct(event_clone.clone()).await.unwrap();
                    }

                    // Signal completion
                    let _ = tx.send(()).await;
                });

                // Count how many simulation tasks complete before verifications finish
                // This is our key metric - higher is better
                let mut completed_simulations = 0;

                loop {
                    tokio::select! {
                        // Check if verifications are done
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
