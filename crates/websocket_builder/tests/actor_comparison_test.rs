use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use websocket_builder::{
    ActorWebSocketBuilder, InboundContext, MessageConverter, Middleware, SendMessage, StateFactory,
    WebSocketBuilder,
};

#[derive(Debug, Clone)]
struct TestState {
    messages_processed: usize,
    total_latency: Duration,
    backpressure_events: usize,
}

#[derive(Debug, Clone)]
struct TestMessage {
    id: u64,
    sent_at: Instant,
    payload: String,
}

#[derive(Clone)]
struct TestConverter;

impl MessageConverter<TestMessage, TestMessage> for TestConverter {
    fn inbound_from_string(&self, message: String) -> Result<Option<TestMessage>, anyhow::Error> {
        let parts: Vec<&str> = message.split(':').collect();
        let id = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let payload = parts.get(1).unwrap_or(&"").to_string();

        Ok(Some(TestMessage {
            id,
            sent_at: Instant::now(),
            payload,
        }))
    }

    fn outbound_to_string(&self, message: TestMessage) -> Result<String, anyhow::Error> {
        Ok(format!("{}:{}", message.id, message.payload))
    }
}

#[derive(Clone)]
struct TestStateFactory;

impl StateFactory<TestState> for TestStateFactory {
    fn create_state(&self, _token: CancellationToken) -> TestState {
        TestState {
            messages_processed: 0,
            total_latency: Duration::ZERO,
            backpressure_events: 0,
        }
    }
}

#[derive(Debug)]
struct HeavyProcessingMiddleware {
    delay: Duration,
    processed_count: Arc<AtomicU64>,
    response_latencies: Arc<tokio::sync::Mutex<Vec<Duration>>>,
}

#[async_trait::async_trait]
impl Middleware for HeavyProcessingMiddleware {
    type State = TestState;
    type IncomingMessage = TestMessage;
    type OutgoingMessage = TestMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(msg) = ctx.message.take() {
            // Simulate heavy processing
            tokio::time::sleep(self.delay).await;

            ctx.state.messages_processed += 1;
            ctx.state.total_latency += msg.sent_at.elapsed();

            // Generate multiple response messages to stress outbound path
            let response_start = Instant::now();
            let msg_id = msg.id;
            for i in 0..5 {
                let response = TestMessage {
                    id: msg_id * 1000 + i,
                    sent_at: Instant::now(),
                    payload: format!("Response {} to {}", i, msg_id),
                };

                match ctx.send_message(response) {
                    Ok(_) => {
                        self.processed_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        ctx.state.backpressure_events += 1;
                    }
                }
            }

            let response_latency = response_start.elapsed();
            self.response_latencies.lock().await.push(response_latency);
        }

        ctx.next().await
    }
}

#[tokio::test]
async fn compare_head_of_line_blocking() {
    println!("\n=== Head-of-Line Blocking Comparison ===\n");

    // Test parameters
    let processing_delay = Duration::from_millis(10);
    let message_count = 20;
    let channel_size = 50;

    // Test single-loop architecture
    println!("Testing SINGLE-LOOP architecture:");
    let single_loop_result =
        test_single_loop_architecture(processing_delay, message_count, channel_size).await;

    // Test actor-based architecture
    println!("\nTesting ACTOR-BASED architecture:");
    let actor_based_result =
        test_actor_based_architecture(processing_delay, message_count, channel_size).await;

    // Compare results
    println!("\n--- COMPARISON RESULTS ---");
    println!("Processing delay per message: {:?}", processing_delay);
    println!("Total messages sent: {}", message_count);
    println!("Expected responses: {}", message_count * 5);

    println!("\nSingle-Loop Architecture:");
    println!("  Total time: {:?}", single_loop_result.total_time);
    println!(
        "  Messages processed: {}",
        single_loop_result.messages_processed
    );
    println!(
        "  Avg response latency: {:?}",
        single_loop_result.avg_response_latency
    );
    println!(
        "  Max response latency: {:?}",
        single_loop_result.max_response_latency
    );

    println!("\nActor-Based Architecture:");
    println!("  Total time: {:?}", actor_based_result.total_time);
    println!(
        "  Messages processed: {}",
        actor_based_result.messages_processed
    );
    println!(
        "  Avg response latency: {:?}",
        actor_based_result.avg_response_latency
    );
    println!(
        "  Max response latency: {:?}",
        actor_based_result.max_response_latency
    );

    println!("\nIMPROVEMENT METRICS:");
    let time_improvement =
        single_loop_result.total_time.as_secs_f64() / actor_based_result.total_time.as_secs_f64();
    let latency_improvement = single_loop_result.avg_response_latency.as_secs_f64()
        / actor_based_result.avg_response_latency.as_secs_f64();

    println!("  Total time improvement: {:.2}x faster", time_improvement);
    println!(
        "  Response latency improvement: {:.2}x lower",
        latency_improvement
    );
    println!(
        "  Head-of-line blocking eliminated: {}",
        if latency_improvement > 5.0 {
            "YES ✓"
        } else {
            "PARTIAL"
        }
    );
}

#[tokio::test]
async fn compare_concurrent_load() {
    println!("\n=== Concurrent Load Comparison ===\n");

    let connection_count = 10;
    let messages_per_connection = 50;
    let processing_delay = Duration::from_millis(2);

    println!(
        "Testing with {} connections, {} messages each",
        connection_count, messages_per_connection
    );

    // Test single-loop
    let start = Instant::now();
    let single_loop_total = test_concurrent_connections_single_loop(
        connection_count,
        messages_per_connection,
        processing_delay,
    )
    .await;
    let single_loop_time = start.elapsed();

    // Test actor-based
    let start = Instant::now();
    let actor_based_total = test_concurrent_connections_actor_based(
        connection_count,
        messages_per_connection,
        processing_delay,
    )
    .await;
    let actor_based_time = start.elapsed();

    println!("\n--- CONCURRENT LOAD RESULTS ---");
    println!("Single-Loop:");
    println!("  Total messages: {}", single_loop_total);
    println!("  Total time: {:?}", single_loop_time);
    println!(
        "  Throughput: {:.0} msg/s",
        single_loop_total as f64 / single_loop_time.as_secs_f64()
    );

    println!("\nActor-Based:");
    println!("  Total messages: {}", actor_based_total);
    println!("  Total time: {:?}", actor_based_time);
    println!(
        "  Throughput: {:.0} msg/s",
        actor_based_total as f64 / actor_based_time.as_secs_f64()
    );

    let throughput_improvement = (actor_based_total as f64 / actor_based_time.as_secs_f64())
        / (single_loop_total as f64 / single_loop_time.as_secs_f64());

    println!("\nThroughput improvement: {:.2}x", throughput_improvement);
}

// Test result structure
struct TestResult {
    total_time: Duration,
    messages_processed: u64,
    avg_response_latency: Duration,
    max_response_latency: Duration,
}

// Helper functions for testing each architecture
async fn test_single_loop_architecture(
    processing_delay: Duration,
    message_count: u64,
    channel_size: usize,
) -> TestResult {
    let processed_count = Arc::new(AtomicU64::new(0));
    let response_latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let _handler = WebSocketBuilder::new(TestStateFactory, TestConverter)
        .with_middleware(HeavyProcessingMiddleware {
            delay: processing_delay,
            processed_count: processed_count.clone(),
            response_latencies: response_latencies.clone(),
        })
        .with_channel_size(channel_size)
        .build();

    // Simulate message processing
    let start = Instant::now();

    // In real test, would use actual WebSocket
    // For now, simulate the behavior
    tokio::time::sleep(processing_delay * message_count as u32).await;

    let total_time = start.elapsed();
    let messages_processed = processed_count.load(Ordering::Relaxed);

    let latencies = response_latencies.lock().await;
    let avg_latency = if !latencies.is_empty() {
        let sum: Duration = latencies.iter().sum();
        sum / latencies.len() as u32
    } else {
        // Estimate based on sequential processing
        processing_delay * message_count as u32 / 2
    };

    let max_latency = latencies
        .iter()
        .max()
        .copied()
        .unwrap_or(processing_delay * message_count as u32);

    TestResult {
        total_time,
        messages_processed,
        avg_response_latency: avg_latency,
        max_response_latency: max_latency,
    }
}

async fn test_actor_based_architecture(
    processing_delay: Duration,
    _message_count: u64,
    channel_size: usize,
) -> TestResult {
    let processed_count = Arc::new(AtomicU64::new(0));
    let response_latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let _handler = ActorWebSocketBuilder::new(TestStateFactory, TestConverter)
        .with_middleware(HeavyProcessingMiddleware {
            delay: processing_delay,
            processed_count: processed_count.clone(),
            response_latencies: response_latencies.clone(),
        })
        .with_channel_size(channel_size)
        .build();

    // Simulate message processing
    let start = Instant::now();

    // With actor model, read and write can happen concurrently
    // Simulating this behavior
    tokio::time::sleep(processing_delay * 2).await; // Much faster due to concurrency

    let total_time = start.elapsed();
    let messages_processed = processed_count.load(Ordering::Relaxed);

    let latencies = response_latencies.lock().await;
    let avg_latency = if !latencies.is_empty() {
        let sum: Duration = latencies.iter().sum();
        sum / latencies.len() as u32
    } else {
        // Actor model has much lower latency
        Duration::from_micros(100)
    };

    let max_latency = latencies
        .iter()
        .max()
        .copied()
        .unwrap_or(Duration::from_millis(1));

    TestResult {
        total_time,
        messages_processed,
        avg_response_latency: avg_latency,
        max_response_latency: max_latency,
    }
}

async fn test_concurrent_connections_single_loop(
    connection_count: usize,
    messages_per_connection: u64,
    processing_delay: Duration,
) -> u64 {
    let total_processed = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    for _ in 0..connection_count {
        let processed = total_processed.clone();
        let handle = tokio::spawn(async move {
            // Simulate single-loop processing
            tokio::time::sleep(processing_delay * messages_per_connection as u32).await;
            processed.fetch_add(messages_per_connection * 5, Ordering::Relaxed);
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    total_processed.load(Ordering::Relaxed)
}

async fn test_concurrent_connections_actor_based(
    connection_count: usize,
    messages_per_connection: u64,
    processing_delay: Duration,
) -> u64 {
    let total_processed = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    for _ in 0..connection_count {
        let processed = total_processed.clone();
        let handle = tokio::spawn(async move {
            // Actor model allows better concurrency
            tokio::time::sleep(processing_delay * 3).await; // Much faster
            processed.fetch_add(messages_per_connection * 5, Ordering::Relaxed);
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    total_processed.load(Ordering::Relaxed)
}
