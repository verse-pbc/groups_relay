use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

// Simple benchmark to measure channel throughput and latency

fn benchmark_mpsc_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("mpsc_channel_throughput");

    // Test different channel sizes
    for size in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("channel_size", size), size, |b, &size| {
            b.to_async(&rt).iter(|| async move {
                let (tx, mut rx) = mpsc::channel::<u64>(size);

                // Spawn receiver
                let received = Arc::new(AtomicU64::new(0));
                let received_clone = received.clone();
                tokio::spawn(async move {
                    while let Some(_msg) = rx.recv().await {
                        received_clone.fetch_add(1, Ordering::Relaxed);
                    }
                });

                // Send messages until we hit backpressure
                let mut sent = 0u64;
                let mut backpressure_count = 0;

                for i in 0..1000 {
                    match tx.try_send(i) {
                        Ok(_) => sent += 1,
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            backpressure_count += 1;
                            // Yield to let receiver process
                            tokio::task::yield_now().await;
                        }
                        Err(_) => break,
                    }
                }

                // Wait a bit for receiver to catch up
                tokio::time::sleep(Duration::from_millis(10)).await;

                let received_count = received.load(Ordering::Relaxed);
                black_box((sent, received_count, backpressure_count))
            })
        });
    }

    group.finish();
}

fn benchmark_message_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("message_processing_latency");

    // Simulate different processing delays
    for delay_us in [0, 100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("processing_delay_us", delay_us),
            delay_us,
            |b, &delay_us| {
                b.to_async(&rt).iter(|| async move {
                    let (tx, mut rx) = mpsc::channel::<Instant>(100);

                    // Spawn processor
                    let latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));
                    let latencies_clone = latencies.clone();
                    tokio::spawn(async move {
                        while let Some(timestamp) = rx.recv().await {
                            // Simulate processing
                            if delay_us > 0 {
                                tokio::time::sleep(Duration::from_micros(delay_us)).await;
                            }
                            let latency = timestamp.elapsed();
                            latencies_clone.lock().await.push(latency);
                        }
                    });

                    // Send batch of messages
                    for _ in 0..100 {
                        let _ = tx.send(Instant::now()).await;
                    }

                    // Wait for processing
                    tokio::time::sleep(Duration::from_millis(50)).await;

                    // Calculate average latency
                    let latencies_vec = latencies.lock().await;
                    let avg_latency = if !latencies_vec.is_empty() {
                        let sum: Duration = latencies_vec.iter().sum();
                        sum.as_micros() as u64 / latencies_vec.len() as u64
                    } else {
                        0
                    };

                    black_box(avg_latency)
                })
            },
        );
    }

    group.finish();
}

fn benchmark_concurrent_tasks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("concurrent_task_overhead");

    // Test different numbers of concurrent tasks
    for task_count in [1, 10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("task_count", task_count),
            task_count,
            |b, &task_count| {
                b.to_async(&rt).iter(|| async move {
                    let start = Instant::now();
                    let counter = Arc::new(AtomicU64::new(0));

                    // Spawn tasks
                    let mut handles = Vec::new();
                    for _ in 0..task_count {
                        let counter_clone = counter.clone();
                        let handle = tokio::spawn(async move {
                            // Minimal work
                            counter_clone.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        });
                        handles.push(handle);
                    }

                    // Wait for all tasks
                    for handle in handles {
                        let _ = handle.await;
                    }

                    let elapsed = start.elapsed();
                    let completed = counter.load(Ordering::Relaxed);

                    black_box((elapsed.as_micros(), completed))
                })
            },
        );
    }

    group.finish();
}

fn benchmark_select_loop(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("select_loop_performance");

    group.bench_function("biased_select_overhead", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx1, mut rx1) = mpsc::channel::<u64>(100);
            let (tx2, mut rx2) = mpsc::channel::<u64>(100);

            // Pre-fill channels
            for i in 0..50 {
                let _ = tx1.send(i).await;
                let _ = tx2.send(i).await;
            }

            let mut count = 0;
            let start = Instant::now();

            // Simulate message loop with biased select
            for _ in 0..100 {
                tokio::select! {
                    biased;

                    Some(_) = rx1.recv() => {
                        count += 1;
                    }
                    Some(_) = rx2.recv() => {
                        count += 1;
                    }
                }
            }

            let elapsed = start.elapsed();
            black_box((count, elapsed.as_nanos()))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_mpsc_throughput,
    benchmark_message_latency,
    benchmark_concurrent_tasks,
    benchmark_select_loop
);
criterion_main!(benches);
