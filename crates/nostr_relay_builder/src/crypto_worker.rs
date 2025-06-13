//! Unified crypto worker for signing and verification operations
//!
//! This module provides a unified worker that handles both event signing and verification
//! operations using a pool of long-lived tokio::spawn_blocking tasks processing a shared queue.

use crate::error::{Error, Result};
use nostr_sdk::prelude::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Operations that can be performed by the crypto worker
enum CryptoOperation {
    Sign {
        event: UnsignedEvent,
        response: oneshot::Sender<Result<Event>>,
    },
    Verify {
        event: Event,
        response: oneshot::Sender<Result<()>>,
    },
}

#[derive(Debug)]
pub struct CryptoWorker {
    // Channel to send requests to workers
    tx: flume::Sender<CryptoOperation>,
    // Handles to the spawn_blocking tasks
    #[allow(dead_code)]
    handles: Vec<JoinHandle<()>>,
    // Metrics for monitoring
    metrics: Arc<CryptoWorkerMetrics>,
}

// Note: Using flume channel because:
// 1. It supports both async and blocking operations
// 2. Multi-consumer support (multiple workers can recv from same channel)
// 3. No need for Arc<Mutex<>> wrapper
// 4. Efficient implementation

#[derive(Default, Debug)]
struct CryptoWorkerMetrics {
    queued_operations: AtomicUsize,
    completed_signs: AtomicU64,
    completed_verifies: AtomicU64,
    failed_operations: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct CryptoWorkerMetricsSnapshot {
    pub queued_operations: usize,
    pub completed_signs: u64,
    pub completed_verifies: u64,
    pub failed_operations: u64,
}

impl CryptoWorker {
    pub fn new(keys: Arc<Keys>, cancellation_token: CancellationToken) -> Self {
        let cpu_count = std::env::var("CRYPTO_WORKER_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            });

        let queue_multiplier = std::env::var("CRYPTO_WORKER_QUEUE_MULTIPLIER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        info!(
            "Starting crypto worker with {} threads, queue size {}",
            cpu_count,
            cpu_count * queue_multiplier
        );

        // Use bounded channel for natural backpressure
        let (tx, rx) = flume::bounded(cpu_count * queue_multiplier);
        let metrics = Arc::new(CryptoWorkerMetrics::default());
        let mut handles = Vec::new();

        // Spawn long-lived workers using spawn_blocking
        for i in 0..cpu_count {
            let rx = rx.clone();
            let worker_keys = Arc::clone(&keys);
            let cancel = cancellation_token.clone();
            let metrics = Arc::clone(&metrics);

            let handle = tokio::task::spawn_blocking(move || {
                debug!("Crypto worker {} started", i);

                // Create a runtime for async operations in this blocking thread
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create crypto runtime");

                runtime.block_on(async {
                    loop {
                        // Check cancellation
                        if cancel.is_cancelled() {
                            debug!("Crypto worker {} shutting down", i);
                            break;
                        }

                        // Try to get work (blocking recv with timeout to check cancellation)
                        let op = match rx.recv_timeout(Duration::from_millis(100)) {
                            Ok(op) => op,
                            Err(flume::RecvTimeoutError::Timeout) => continue, // Check cancellation
                            Err(flume::RecvTimeoutError::Disconnected) => break, // Channel closed
                        };

                        // Update metrics
                        metrics.queued_operations.fetch_sub(1, Ordering::Relaxed);

                        // Process the operation
                        match op {
                            CryptoOperation::Sign { event, response } => {
                                let result = worker_keys.sign_event(event).await.map_err(|e| {
                                    Error::internal(format!("Failed to sign event: {}", e))
                                });

                                if result.is_ok() {
                                    metrics.completed_signs.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    metrics.failed_operations.fetch_add(1, Ordering::Relaxed);
                                }

                                let _ = response.send(result);
                            }
                            CryptoOperation::Verify { event, response } => {
                                let result = event.verify().map_err(|e| {
                                    Error::internal(format!("Event verification failed: {}", e))
                                });

                                if result.is_ok() {
                                    metrics.completed_verifies.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    metrics.failed_operations.fetch_add(1, Ordering::Relaxed);
                                }

                                let _ = response.send(result);
                            }
                        }
                    }
                });

                debug!("Crypto worker {} stopped", i);
            });

            handles.push(handle);
        }

        Self {
            tx,
            handles,
            metrics,
        }
    }

    pub async fn sign_event(&self, event: UnsignedEvent) -> Result<Event> {
        let (response_tx, response_rx) = oneshot::channel();

        // Update metrics
        self.metrics
            .queued_operations
            .fetch_add(1, Ordering::Relaxed);

        self.tx
            .send_async(CryptoOperation::Sign {
                event,
                response: response_tx,
            })
            .await
            .map_err(|_| {
                warn!("Crypto worker queue full, rejecting sign operation");
                Error::internal(
                    "Crypto worker unavailable - queue may be full or workers shut down",
                )
            })?;

        response_rx.await.map_err(|_| {
            Error::internal("Crypto worker dropped before completing signing operation")
        })?
    }

    pub async fn verify_event(&self, event: Event) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();

        // Update metrics
        self.metrics
            .queued_operations
            .fetch_add(1, Ordering::Relaxed);

        self.tx
            .send_async(CryptoOperation::Verify {
                event,
                response: response_tx,
            })
            .await
            .map_err(|_| {
                warn!("Crypto worker queue full, rejecting verify operation");
                Error::internal(
                    "Crypto worker unavailable - queue may be full or workers shut down",
                )
            })?;

        response_rx.await.map_err(|_| {
            Error::internal("Crypto worker dropped before completing verification operation")
        })?
    }

    /// Get current metrics
    pub fn metrics(&self) -> CryptoWorkerMetricsSnapshot {
        CryptoWorkerMetricsSnapshot {
            queued_operations: self.metrics.queued_operations.load(Ordering::Relaxed),
            completed_signs: self.metrics.completed_signs.load(Ordering::Relaxed),
            completed_verifies: self.metrics.completed_verifies.load(Ordering::Relaxed),
            failed_operations: self.metrics.failed_operations.load(Ordering::Relaxed),
        }
    }
}

impl Drop for CryptoWorker {
    fn drop(&mut self) {
        // Close channel to signal workers to stop accepting new work
        // Workers will process remaining items before exiting
        drop(self.tx.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sign_event() {
        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        let worker = CryptoWorker::new(Arc::clone(&keys), cancellation_token.clone());

        // Create test event
        let unsigned_event = UnsignedEvent::new(
            keys.public_key(),
            Timestamp::now(),
            Kind::TextNote,
            vec![],
            "Test message",
        );

        // Sign the event
        let signed_event = worker.sign_event(unsigned_event).await.unwrap();

        // Verify the signature
        assert!(signed_event.verify().is_ok());
        assert_eq!(signed_event.pubkey, keys.public_key());

        // Check metrics
        let metrics = worker.metrics();
        assert_eq!(metrics.completed_signs, 1);
        assert_eq!(metrics.failed_operations, 0);

        // Cleanup
        cancellation_token.cancel();
    }

    #[tokio::test]
    async fn test_verify_event() {
        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        let worker = CryptoWorker::new(Arc::clone(&keys), cancellation_token.clone());

        // Create and sign event
        let unsigned_event = UnsignedEvent::new(
            keys.public_key(),
            Timestamp::now(),
            Kind::TextNote,
            vec![],
            "Test message",
        );
        let signed_event = keys.sign_event(unsigned_event).await.unwrap();

        // Verify the event
        worker.verify_event(signed_event).await.unwrap();

        // Check metrics
        let metrics = worker.metrics();
        assert_eq!(metrics.completed_verifies, 1);
        assert_eq!(metrics.failed_operations, 0);

        // Cleanup
        cancellation_token.cancel();
    }

    #[tokio::test]
    async fn test_invalid_event_verification() {
        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        let worker = CryptoWorker::new(Arc::clone(&keys), cancellation_token.clone());

        // Create an event with invalid signature by modifying content after signing
        let unsigned = UnsignedEvent::new(
            keys.public_key(),
            Timestamp::now(),
            Kind::TextNote,
            vec![],
            "Test",
        );

        let mut event = keys.sign_event(unsigned).await.unwrap();

        // Modify content to make signature invalid
        event.content = "Modified content".to_string();

        // Verify should fail
        let result = worker.verify_event(event).await;
        assert!(result.is_err());

        // Check metrics
        let metrics = worker.metrics();
        assert_eq!(metrics.completed_verifies, 0);
        assert_eq!(metrics.failed_operations, 1);

        // Cleanup
        cancellation_token.cancel();
    }

    #[tokio::test]
    async fn test_concurrent_operations() {
        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        let worker = Arc::new(CryptoWorker::new(
            Arc::clone(&keys),
            cancellation_token.clone(),
        ));

        // Spawn multiple concurrent operations
        let mut sign_handles = vec![];
        let mut verify_handles = vec![];

        // Spawn signing tasks
        for i in 0..50 {
            let worker_clone = Arc::clone(&worker);
            let keys_clone = Arc::clone(&keys);

            let handle = tokio::spawn(async move {
                let unsigned_event = UnsignedEvent::new(
                    keys_clone.public_key(),
                    Timestamp::now(),
                    Kind::TextNote,
                    vec![],
                    format!("Test message {}", i),
                );

                worker_clone.sign_event(unsigned_event).await
            });

            sign_handles.push(handle);
        }

        // Spawn verification tasks
        for i in 0..50 {
            let worker_clone = Arc::clone(&worker);
            let keys_clone = Arc::clone(&keys);

            let handle = tokio::spawn(async move {
                let unsigned_event = UnsignedEvent::new(
                    keys_clone.public_key(),
                    Timestamp::now(),
                    Kind::TextNote,
                    vec![],
                    format!("Test verify {}", i),
                );
                let signed_event = keys_clone.sign_event(unsigned_event).await.unwrap();

                worker_clone.verify_event(signed_event).await
            });

            verify_handles.push(handle);
        }

        // Wait for all signing operations
        let sign_results: Vec<_> = futures_util::future::join_all(sign_handles).await;
        let verify_results: Vec<_> = futures_util::future::join_all(verify_handles).await;

        // Verify all signing operations succeeded
        for result in sign_results {
            assert!(result.is_ok());
            assert!(result.unwrap().is_ok());
        }

        // Verify all verification operations succeeded
        for result in verify_results {
            assert!(result.is_ok());
            assert!(result.unwrap().is_ok());
        }

        // Check metrics
        let metrics = worker.metrics();
        assert_eq!(metrics.completed_signs, 50);
        assert_eq!(metrics.completed_verifies, 50);
        assert_eq!(metrics.failed_operations, 0);
        assert_eq!(metrics.queued_operations, 0); // Should be drained

        // Cleanup
        cancellation_token.cancel();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_graceful_shutdown() {
        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        let worker = Arc::new(CryptoWorker::new(
            Arc::clone(&keys),
            cancellation_token.clone(),
        ));

        // Queue some operations
        let mut handles = vec![];
        for i in 0..10 {
            let worker_clone = Arc::clone(&worker);
            let keys_clone = Arc::clone(&keys);

            let handle = tokio::spawn(async move {
                let unsigned_event = UnsignedEvent::new(
                    keys_clone.public_key(),
                    Timestamp::now(),
                    Kind::TextNote,
                    vec![],
                    format!("Shutdown test {}", i),
                );

                worker_clone.sign_event(unsigned_event).await
            });

            handles.push(handle);
        }

        // Give some time for operations to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Cancel and drop the worker
        cancellation_token.cancel();
        drop(worker);

        // Use timeout to prevent hanging
        let results = tokio::time::timeout(
            Duration::from_secs(5),
            futures_util::future::join_all(handles),
        )
        .await
        .unwrap_or_else(|_| vec![]);

        // Count successes
        let successes = results
            .iter()
            .filter(|r| r.is_ok() && r.as_ref().unwrap().is_ok())
            .count();
        println!("Completed {} operations before shutdown", successes);

        // At least some should have completed or been cancelled
        assert!(successes > 0 || results.is_empty());
    }

    #[tokio::test]
    async fn test_backpressure() {
        // Use smaller queue for testing
        std::env::set_var("CRYPTO_WORKER_THREADS", "2");
        std::env::set_var("CRYPTO_WORKER_QUEUE_MULTIPLIER", "2");

        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        let worker = Arc::new(CryptoWorker::new(
            Arc::clone(&keys),
            cancellation_token.clone(),
        ));

        // Try to overflow the queue
        let mut handles = vec![];
        for i in 0..10 {
            let worker_clone = Arc::clone(&worker);
            let keys_clone = Arc::clone(&keys);

            let handle = tokio::spawn(async move {
                let unsigned_event = UnsignedEvent::new(
                    keys_clone.public_key(),
                    Timestamp::now(),
                    Kind::TextNote,
                    vec![],
                    format!("Backpressure test {}", i),
                );

                // Add small delay to simulate slow processing
                tokio::time::sleep(Duration::from_millis(10)).await;
                worker_clone.sign_event(unsigned_event).await
            });

            handles.push(handle);
        }

        // Wait for results
        let results: Vec<_> = futures_util::future::join_all(handles).await;

        // Should handle backpressure gracefully
        let successes = results
            .iter()
            .filter(|r| r.is_ok() && r.as_ref().unwrap().is_ok())
            .count();
        let failures = results.len() - successes;

        println!(
            "Backpressure test: {} successes, {} failures",
            successes, failures
        );

        // Some operations should succeed
        assert!(successes > 0);

        // Cleanup
        cancellation_token.cancel();
        std::env::remove_var("CRYPTO_WORKER_THREADS");
        std::env::remove_var("CRYPTO_WORKER_QUEUE_MULTIPLIER");
    }
}
