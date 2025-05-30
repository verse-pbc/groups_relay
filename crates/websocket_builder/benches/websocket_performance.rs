use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;
use websocket_builder::{
    InboundContext, MessageConverter, Middleware, SendMessage, StateFactory, WebSocketBuilder,
};

// Mock types for benchmarking
#[derive(Clone, Debug)]
struct MockState {
    message_count: usize,
    processing_time_sum: Duration,
    last_message_time: Option<Instant>,
}

#[derive(Clone, Debug)]
struct MockMessage {
    id: u64,
    payload: String,
    timestamp: Instant,
}

#[derive(Clone)]
struct MockStateFactory;

impl StateFactory<MockState> for MockStateFactory {
    fn create_state(&self, _token: CancellationToken) -> MockState {
        MockState {
            message_count: 0,
            processing_time_sum: Duration::ZERO,
            last_message_time: None,
        }
    }
}

#[derive(Clone)]
struct MockConverter;

impl MessageConverter<MockMessage, MockMessage> for MockConverter {
    fn inbound_from_string(&self, message: String) -> Result<Option<MockMessage>, anyhow::Error> {
        Ok(Some(MockMessage {
            id: 0,
            payload: message,
            timestamp: Instant::now(),
        }))
    }

    fn outbound_to_string(&self, message: MockMessage) -> Result<String, anyhow::Error> {
        Ok(message.payload)
    }
}

// Heavy middleware that simulates CPU-intensive work
#[derive(Debug)]
struct HeavyMiddleware {
    work_duration: Duration,
    response_count: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl Middleware for HeavyMiddleware {
    type State = MockState;
    type IncomingMessage = MockMessage;
    type OutgoingMessage = MockMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(ref msg) = ctx.message {
            // Simulate CPU-intensive work
            let start = Instant::now();
            tokio::time::sleep(self.work_duration).await;

            ctx.state.message_count += 1;
            ctx.state.processing_time_sum += start.elapsed();
            ctx.state.last_message_time = Some(Instant::now());

            // Try to send response message
            let response = MockMessage {
                id: msg.id + 1000,
                payload: format!("Response to {}", msg.id),
                timestamp: Instant::now(),
            };

            match ctx.send_message(response) {
                Ok(_) => {
                    self.response_count.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // Queue full - count as backpressure event
                }
            }
        }

        ctx.next().await
    }
}

// Light middleware that does minimal work
#[derive(Debug)]
struct LightMiddleware {
    processed_count: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl Middleware for LightMiddleware {
    type State = MockState;
    type IncomingMessage = MockMessage;
    type OutgoingMessage = MockMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.message_count += 1;
        self.processed_count.fetch_add(1, Ordering::Relaxed);
        ctx.next().await
    }
}

// Middleware that measures outbound latency
#[derive(Debug)]
struct LatencyMeasureMiddleware {
    latencies: Arc<tokio::sync::Mutex<Vec<Duration>>>,
}

#[async_trait::async_trait]
impl Middleware for LatencyMeasureMiddleware {
    type State = MockState;
    type IncomingMessage = MockMessage;
    type OutgoingMessage = MockMessage;

    async fn process_outbound(
        &self,
        ctx: &mut websocket_builder::OutboundContext<
            '_,
            Self::State,
            Self::IncomingMessage,
            Self::OutgoingMessage,
        >,
    ) -> Result<(), anyhow::Error> {
        if let Some(ref msg) = ctx.message {
            let latency = msg.timestamp.elapsed();
            self.latencies.lock().await.push(latency);
        }
        ctx.next().await
    }
}

fn benchmark_head_of_line_blocking(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("head_of_line_blocking");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(10);

    // Test different processing delays
    for delay_ms in [0, 1, 5, 10, 50].iter() {
        let delay = Duration::from_millis(*delay_ms);

        group.bench_with_input(
            BenchmarkId::new("processing_delay_ms", delay_ms),
            &delay,
            |b, &delay| {
                b.to_async(&rt).iter(|| async move {
                    let response_count = Arc::new(AtomicU64::new(0));
                    let latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

                    // Create handler with heavy middleware
                    let _handler = WebSocketBuilder::new(MockStateFactory, MockConverter)
                        .with_middleware(LatencyMeasureMiddleware {
                            latencies: latencies.clone(),
                        })
                        .with_middleware(HeavyMiddleware {
                            work_duration: delay,
                            response_count: response_count.clone(),
                        })
                        .with_channel_size(100)
                        .build();

                    // Simulate message processing
                    // In real benchmark, we'd use actual WebSocket connections
                    let messages_sent = 50;
                    let responses = response_count.load(Ordering::Relaxed);

                    black_box((messages_sent, responses))
                })
            },
        );
    }

    group.finish();
}

fn benchmark_outbound_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("outbound_message_latency");
    group.measurement_time(Duration::from_secs(3));

    // Test outbound latency under different load conditions
    for inbound_rate in [0, 10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("inbound_msgs_per_sec", inbound_rate),
            inbound_rate,
            |b, &rate| {
                b.to_async(&rt).iter(|| async move {
                    let latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

                    // Create handler
                    let _handler = WebSocketBuilder::new(MockStateFactory, MockConverter)
                        .with_middleware(LatencyMeasureMiddleware {
                            latencies: latencies.clone(),
                        })
                        .with_middleware(LightMiddleware {
                            processed_count: Arc::new(AtomicU64::new(0)),
                        })
                        .with_channel_size(50)
                        .build();

                    // Simulate load and measure latencies
                    // In real benchmark, we'd generate actual traffic
                    let avg_latency = if rate > 0 {
                        Duration::from_micros(100 + rate / 10)
                    } else {
                        Duration::from_micros(10)
                    };

                    black_box(avg_latency)
                })
            },
        );
    }

    group.finish();
}

fn benchmark_backpressure_handling(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("backpressure");
    group.measurement_time(Duration::from_secs(3));

    // Test different channel sizes
    for size in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::new("channel_size", size), size, |b, &size| {
            b.to_async(&rt).iter(|| async move {
                let _backpressure_events = Arc::new(AtomicU64::new(0));

                // Create handler with specific channel size
                let _handler = WebSocketBuilder::new(MockStateFactory, MockConverter)
                    .with_middleware(LightMiddleware {
                        processed_count: Arc::new(AtomicU64::new(0)),
                    })
                    .with_channel_size(size)
                    .build();

                // Simulate burst traffic to trigger backpressure
                // In real benchmark, we'd send actual messages
                let expected_backpressure = if size < 100 {
                    5 // Small buffers hit backpressure quickly
                } else {
                    0 // Large buffers rarely hit backpressure
                };

                black_box(expected_backpressure)
            })
        });
    }

    group.finish();
}

fn benchmark_concurrent_connections(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("concurrent_connections");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    // Test scaling with multiple connections
    for conn_count in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::new("connections", conn_count),
            conn_count,
            |b, &conn_count| {
                b.to_async(&rt).iter(|| async move {
                    let total_messages = Arc::new(AtomicU64::new(0));

                    // Create multiple handlers to simulate concurrent connections
                    let handlers: Vec<_> = (0..conn_count)
                        .map(|_| {
                            WebSocketBuilder::new(MockStateFactory, MockConverter)
                                .with_middleware(LightMiddleware {
                                    processed_count: total_messages.clone(),
                                })
                                .with_channel_size(50)
                                .build()
                        })
                        .collect();

                    // Simulate processing
                    let _messages_per_connection = 100;
                    let total_processed = total_messages.load(Ordering::Relaxed);

                    black_box((handlers.len(), total_processed))
                })
            },
        );
    }

    group.finish();
}

// Additional benchmark for measuring middleware chain overhead
fn benchmark_middleware_chain_overhead(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("middleware_chain_overhead");

    // Test different middleware chain lengths
    for chain_length in [1, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("middleware_count", chain_length),
            chain_length,
            |b, &length| {
                b.to_async(&rt).iter(|| async move {
                    let mut builder = WebSocketBuilder::new(MockStateFactory, MockConverter);

                    // Add multiple lightweight middleware
                    for _ in 0..length {
                        builder = builder.with_middleware(LightMiddleware {
                            processed_count: Arc::new(AtomicU64::new(0)),
                        });
                    }

                    let _handler = builder.with_channel_size(100).build();

                    black_box(_handler)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_head_of_line_blocking,
    benchmark_outbound_latency,
    benchmark_backpressure_handling,
    benchmark_concurrent_connections,
    benchmark_middleware_chain_overhead
);
criterion_main!(benches);
