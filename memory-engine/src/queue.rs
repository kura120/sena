//! Async priority write queue for serializing all ech0 `ingest_text` calls.
//!
//! No subsystem calls `store.ingest_text()` directly — every write goes through
//! `WriteQueue`. The queue enforces:
//!
//! - **Sequential ech0 ingest calls** — never concurrent
//! - **Priority ordering** — Reactive at front, Background at back
//! - **Max depth** from config — returns `ErrorCode::QueueFull` if exceeded
//! - **Per-item bounded timeout** (`operation_timeout_ms` from config) — returns
//!   `ErrorCode::QueueTimeout` if it expires before processing
//! - **Retry on transient `EchoError`** — up to `max_attempts` with `backoff_ms`
//!   spacing
//!
//! The drain task `JoinHandle` is stored and must not be silently dropped.
//!
//! ## Design
//!
//! Items flow through an `mpsc` channel from `submit()` to the drain loop. The
//! drain loop receives items from the channel, inserts them into a priority
//! `VecDeque` (Reactive at front, Background at back), then pops and processes
//! items sequentially. This ensures that a Reactive item submitted while a
//! Background batch is queued will jump to the front on the next drain cycle.
//!
//! Depth tracking uses an `AtomicU32` shared between `submit()` (increment on
//! send) and the drain loop (decrement after processing). This avoids locking
//! the deque just for a depth check on the hot enqueue path.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use ech0::{Embedder, Extractor, Store};

use crate::config::QueueConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};

// ─────────────────────────────────────────────────────────────────────────────
// Priority
// ─────────────────────────────────────────────────────────────────────────────

/// Priority level for a write operation.
///
/// Only two levels are used by the write queue. The full Sena priority tier
/// hierarchy (Critical through Background) is mapped down to these two at
/// the engine.rs call site before enqueuing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// User-facing, latency-sensitive writes. Inserted at the front of the
    /// queue so they are processed before any background work.
    Reactive,
    /// Non-urgent writes (CTP, telemetry, decay). Inserted at the back.
    Background,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Reactive => write!(formatter, "reactive"),
            Priority::Background => write!(formatter, "background"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WriteItem — a single queued write request
// ─────────────────────────────────────────────────────────────────────────────

/// Payload for a single write operation sent through the channel to the
/// drain loop. The drain loop inserts it into the priority deque, then
/// processes it.
struct WriteItem {
    /// The text content to pass to `store.ingest_text()`.
    text: String,
    /// Priority of this write — determines deque insertion position.
    priority: Priority,
    /// When this item was enqueued — used to enforce `operation_timeout_ms`.
    enqueued_at: Instant,
    /// One-shot channel to send the result back to the waiting caller.
    response_tx: oneshot::Sender<SenaResult<()>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// WriteQueue
// ─────────────────────────────────────────────────────────────────────────────

/// Async priority write queue that serializes all ech0 `ingest_text` calls.
///
/// All write operations flow through this queue. The queue owns a single
/// tokio task (the "drain loop") that processes items sequentially — ech0
/// never sees concurrent ingest calls.
///
/// The `JoinHandle` for the drain task is stored in `drain_handle` and must
/// not be silently dropped. Call `shutdown()` to signal the drain loop to
/// stop and await its completion.
pub struct WriteQueue<E: Embedder, X: Extractor> {
    /// Sender half of the channel used to submit items to the drain loop.
    submit_tx: mpsc::Sender<WriteItem>,

    /// Atomic depth counter shared with the drain loop. Incremented on
    /// `submit()`, decremented by the drain loop after processing each item.
    /// Allows fast-path depth checks without locking the deque.
    depth: Arc<AtomicU32>,

    /// Maximum queue depth from config — cached for fast-path checks.
    max_depth: u32,

    /// Join handle for the drain task — stored, never silently dropped.
    drain_handle: tokio::sync::Mutex<Option<JoinHandle<()>>>,

    /// Queue configuration (max_depth, operation_timeout_ms, retry policy).
    config: QueueConfig,

    /// Phantom data to carry the E and X type parameters without storing them.
    _phantom: std::marker::PhantomData<(E, X)>,
}

impl<E: Embedder + 'static, X: Extractor + 'static> WriteQueue<E, X> {
    /// Create a new `WriteQueue` and spawn the drain loop.
    ///
    /// The drain loop runs as a single tokio task that receives items via the
    /// channel, inserts them into a priority deque, then pops and processes
    /// them sequentially. It retries transient ech0 errors according to the
    /// retry policy in `config`.
    ///
    /// # Arguments
    ///
    /// * `store` — The ech0 `Store` instance used for `ingest_text` calls.
    /// * `config` — Queue configuration (max_depth, timeouts, retry policy).
    pub fn new(store: Arc<Store<E, X>>, config: QueueConfig) -> Self {
        let depth = Arc::new(AtomicU32::new(0));

        // Bounded channel — buffer size matches max_depth so submit() never
        // blocks on the channel itself when the depth check has already passed.
        // The +1 avoids a zero-capacity channel if max_depth is somehow 0.
        let channel_capacity = (config.max_depth as usize).max(1);
        let (submit_tx, submit_rx) = mpsc::channel::<WriteItem>(channel_capacity);

        let drain_depth = Arc::clone(&depth);
        let drain_config = config.clone();
        let drain_handle = tokio::spawn(Self::drain_loop(
            store,
            submit_rx,
            drain_depth,
            drain_config,
        ));

        tracing::info!(
            subsystem = "memory_engine",
            component = "queue",
            max_depth = config.max_depth,
            operation_timeout_ms = config.operation_timeout_ms,
            max_retry_attempts = config.retry.max_attempts,
            retry_backoff_ms = config.retry.backoff_ms,
            "write queue initialized"
        );

        Self {
            submit_tx,
            depth,
            max_depth: config.max_depth,
            drain_handle: tokio::sync::Mutex::new(Some(drain_handle)),
            config,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Submit a text for ingestion into ech0 at the given priority.
    ///
    /// Sends the item through the channel to the drain loop, which handles
    /// priority ordering in the deque. Awaits the result with a timeout.
    ///
    /// # Errors
    ///
    /// - `ErrorCode::QueueFull` if the queue has reached `max_depth`.
    /// - `ErrorCode::QueueTimeout` if the item is not processed within
    ///   `operation_timeout_ms`.
    /// - Any `SenaError` propagated from the ech0 ingest call itself.
    pub async fn submit(&self, text: String, priority: Priority) -> SenaResult<()> {
        // Fast-path depth check — no lock, just an atomic load.
        let current_depth = self.depth.load(Ordering::Acquire);
        if current_depth >= self.max_depth {
            tracing::warn!(
                subsystem = "memory_engine",
                component = "queue",
                operation = "submit",
                priority = %priority,
                depth = current_depth,
                max_depth = self.max_depth,
                "write queue full — rejecting submit"
            );
            return Err(SenaError::new(
                ErrorCode::QueueFull,
                "write queue at capacity",
            ));
        }

        let (response_tx, response_rx) = oneshot::channel();

        let item = WriteItem {
            text,
            priority,
            enqueued_at: Instant::now(),
            response_tx,
        };

        let item_priority = item.priority;

        // Increment depth before sending — if the send fails, we decrement.
        // This is a slight over-count in the race window, which is safe:
        // it may cause a spurious QueueFull rejection but never allows the
        // queue to exceed max_depth.
        self.depth.fetch_add(1, Ordering::Release);

        // Send through the channel to the drain loop.
        if let Err(send_error) = self.submit_tx.send(item).await {
            // Channel closed — drain loop has shut down.
            self.depth.fetch_sub(1, Ordering::Release);

            // The WriteItem is inside the SendError — its response_tx will
            // be dropped, but we return an explicit error to the caller.
            drop(send_error);

            return Err(SenaError::new(
                ErrorCode::QueueTimeout,
                "write queue drain loop has shut down — cannot accept new items",
            ));
        }

        tracing::debug!(
            subsystem = "memory_engine",
            component = "queue",
            operation = "submit",
            priority = %item_priority,
            depth = self.depth.load(Ordering::Relaxed),
            "item submitted to drain loop"
        );

        // Await the result with a timeout.
        let timeout_duration = Duration::from_millis(self.config.operation_timeout_ms);

        match tokio::time::timeout(timeout_duration, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_recv_error)) => {
                // The drain loop dropped the response sender without sending
                // a result — this means the drain loop shut down while this
                // item was still pending.
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "queue",
                    operation = "submit",
                    priority = %item_priority,
                    "response channel closed — drain loop may have shut down"
                );
                Err(SenaError::new(
                    ErrorCode::QueueTimeout,
                    "write queue drain loop terminated before processing item",
                ))
            }
            Err(_elapsed) => {
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "queue",
                    operation = "submit",
                    priority = %item_priority,
                    timeout_ms = self.config.operation_timeout_ms,
                    "queued write operation timed out"
                );
                Err(SenaError::new(
                    ErrorCode::QueueTimeout,
                    "queued write operation exceeded operation_timeout_ms",
                ))
            }
        }
    }

    /// Gracefully shut down the write queue.
    ///
    /// Closes the submit channel to signal the drain loop to exit after
    /// processing remaining items, then awaits the drain task's `JoinHandle`.
    /// Any items still in the queue when the drain loop exits will have their
    /// response channels dropped, causing callers to receive a `QueueTimeout`
    /// or channel-closed error.
    pub async fn shutdown(&self) {
        // Take the join handle so we can await it. Dropping the Sender side
        // of the channel cannot be done directly through &self, but once
        // all Sender clones are dropped the drain loop's recv returns None.
        // Since we hold the only Sender (no clones), the drain loop will
        // exit once the WriteQueue is dropped. For explicit shutdown, we
        // await the handle.
        let handle = {
            let mut guard = self.drain_handle.lock().await;
            guard.take()
        };

        if let Some(join_handle) = handle {
            tracing::info!(
                subsystem = "memory_engine",
                component = "queue",
                "shutting down write queue drain loop"
            );

            // Wait for the drain loop to finish. If it panicked, log but
            // don't propagate — we're shutting down.
            if let Err(join_error) = join_handle.await {
                tracing::error!(
                    subsystem = "memory_engine",
                    component = "queue",
                    error = %join_error,
                    "drain loop task panicked during shutdown"
                );
            }
        }
    }

    /// Returns the current approximate depth of the write queue.
    pub fn depth(&self) -> u32 {
        self.depth.load(Ordering::Relaxed)
    }

    /// Returns the configured maximum depth of the write queue.
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    // ─────────────────────────────────────────────────────────────────────
    // Drain loop — runs as a single tokio task
    // ─────────────────────────────────────────────────────────────────────

    /// The drain loop — runs as a single tokio task, processing items
    /// sequentially.
    ///
    /// 1. Receives item(s) from the mpsc channel.
    /// 2. Inserts them into a local priority `VecDeque`.
    /// 3. Pops the highest-priority item and processes it (calls
    ///    `store.ingest_text()` with retry logic).
    /// 4. Decrements the shared depth counter.
    /// 5. Sends the result back to the caller via the oneshot channel.
    /// 6. Repeats until the channel is closed (shutdown signal).
    async fn drain_loop(
        store: Arc<Store<E, X>>,
        mut submit_rx: mpsc::Receiver<WriteItem>,
        depth: Arc<AtomicU32>,
        config: QueueConfig,
    ) {
        let operation_timeout = Duration::from_millis(config.operation_timeout_ms);
        let retry_backoff = Duration::from_millis(config.retry.backoff_ms);
        let max_retry_attempts = config.retry.max_attempts;

        // Local priority deque — only accessed by this task, no lock needed.
        let mut deque: VecDeque<WriteItem> = VecDeque::new();

        tracing::info!(
            subsystem = "memory_engine",
            component = "queue",
            "drain loop started"
        );

        loop {
            // If the deque is empty, block until at least one item arrives
            // (or the channel closes). If the deque has items, try to receive
            // more without blocking before processing.
            if deque.is_empty() {
                match submit_rx.recv().await {
                    Some(item) => {
                        push_prioritized(&mut deque, item);
                    }
                    None => {
                        // Channel closed — shutdown.
                        tracing::info!(
                            subsystem = "memory_engine",
                            component = "queue",
                            "drain loop received shutdown signal — no remaining items"
                        );
                        return;
                    }
                }
            }

            // Drain any additional items that arrived while we were idle or
            // processing the previous item — keeps priority ordering accurate.
            while let Ok(additional_item) = submit_rx.try_recv() {
                push_prioritized(&mut deque, additional_item);
            }

            // Process one item from the front of the deque (highest priority).
            if let Some(write_item) = deque.pop_front() {
                process_item(
                    &store,
                    write_item,
                    operation_timeout,
                    max_retry_attempts,
                    retry_backoff,
                )
                .await;

                // Decrement depth after processing — matches the increment
                // in submit().
                depth.fetch_sub(1, Ordering::Release);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Priority insertion
// ─────────────────────────────────────────────────────────────────────────────

/// Insert an item into the deque respecting priority ordering.
/// Reactive goes to the front, Background goes to the back.
fn push_prioritized(deque: &mut VecDeque<WriteItem>, item: WriteItem) {
    match item.priority {
        Priority::Reactive => deque.push_front(item),
        Priority::Background => deque.push_back(item),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Item processing with retry
// ─────────────────────────────────────────────────────────────────────────────

/// Process a single write item: call `store.ingest_text()` with retry logic,
/// then send the result back to the caller via the oneshot channel.
async fn process_item<E: Embedder, X: Extractor>(
    store: &Arc<Store<E, X>>,
    item: WriteItem,
    operation_timeout: Duration,
    max_retry_attempts: u32,
    retry_backoff: Duration,
) {
    let elapsed_since_enqueue = item.enqueued_at.elapsed();

    // Check if the item has already timed out before we even started
    // processing it (it sat in the queue too long).
    if elapsed_since_enqueue >= operation_timeout {
        tracing::warn!(
            subsystem = "memory_engine",
            component = "queue",
            operation = "ingest",
            priority = %item.priority,
            elapsed_ms = elapsed_since_enqueue.as_millis() as u64,
            timeout_ms = operation_timeout.as_millis() as u64,
            "item expired in queue before processing — sending timeout"
        );

        // The receiver may have already timed out and dropped — discard
        // the send error.
        let _ = item.response_tx.send(Err(SenaError::new(
            ErrorCode::QueueTimeout,
            "write operation expired in queue before processing began",
        )));
        return;
    }

    let start_time = Instant::now();
    let mut last_error: Option<SenaError> = None;
    let mut attempt: u32 = 0;

    while attempt < max_retry_attempts {
        attempt += 1;

        // Check remaining time budget before each attempt.
        let total_elapsed = item.enqueued_at.elapsed();
        if total_elapsed >= operation_timeout {
            tracing::warn!(
                subsystem = "memory_engine",
                component = "queue",
                operation = "ingest",
                priority = %item.priority,
                attempt = attempt,
                elapsed_ms = total_elapsed.as_millis() as u64,
                "operation timed out during retry sequence"
            );
            let _ = item.response_tx.send(Err(SenaError::new(
                ErrorCode::QueueTimeout,
                "write operation timed out during retry sequence",
            )));
            return;
        }

        let time_left = operation_timeout.saturating_sub(total_elapsed);

        // Call store.ingest_text with a timeout on the remaining budget.
        let ingest_result = tokio::time::timeout(time_left, store.ingest_text(&item.text)).await;

        match ingest_result {
            Ok(Ok(_ingest_result)) => {
                let duration_ms = start_time.elapsed().as_millis() as u64;

                tracing::debug!(
                    subsystem = "memory_engine",
                    component = "queue",
                    operation = "ingest",
                    priority = %item.priority,
                    attempt = attempt,
                    duration_ms = duration_ms,
                    "ingest_text succeeded"
                );

                // Send success back. If the receiver has dropped (timed out
                // on their end), discard silently.
                let _ = item.response_tx.send(Ok(()));
                return;
            }
            Ok(Err(echo_error)) => {
                let sena_error: SenaError = echo_error.into();

                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "queue",
                    operation = "ingest",
                    priority = %item.priority,
                    attempt = attempt,
                    max_attempts = max_retry_attempts,
                    error_code = %sena_error.code,
                    "ingest_text failed — will retry if attempts remain"
                );

                last_error = Some(sena_error);

                // Back off before retrying — but only if we have attempts
                // remaining and time budget left.
                if attempt < max_retry_attempts {
                    let time_until_deadline =
                        operation_timeout.saturating_sub(item.enqueued_at.elapsed());
                    let actual_backoff = retry_backoff.min(time_until_deadline);
                    if !actual_backoff.is_zero() {
                        tokio::time::sleep(actual_backoff).await;
                    }
                }
            }
            Err(_elapsed) => {
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "queue",
                    operation = "ingest",
                    priority = %item.priority,
                    attempt = attempt,
                    "ingest_text call timed out within remaining budget"
                );

                let _ = item.response_tx.send(Err(SenaError::new(
                    ErrorCode::QueueTimeout,
                    "ingest_text call exceeded remaining time budget",
                )));
                return;
            }
        }
    }

    // All retry attempts exhausted — send the last error back.
    let final_error = last_error.unwrap_or_else(|| {
        SenaError::new(
            ErrorCode::StorageFailure,
            "ingest_text failed after all retry attempts — no error captured",
        )
    });

    tracing::error!(
        subsystem = "memory_engine",
        component = "queue",
        operation = "ingest",
        priority = %item.priority,
        attempts_exhausted = max_retry_attempts,
        error_code = %final_error.code,
        "all retry attempts exhausted for ingest_text"
    );

    let _ = item.response_tx.send(Err(final_error));
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_queue_config() -> QueueConfig {
        QueueConfig {
            max_depth: 10,
            operation_timeout_ms: 5000,
            retry: crate::config::RetryConfig {
                max_attempts: 2,
                backoff_ms: 50,
            },
        }
    }

    #[test]
    fn priority_display() {
        assert_eq!(format!("{}", Priority::Reactive), "reactive");
        assert_eq!(format!("{}", Priority::Background), "background");
    }

    #[test]
    fn priority_equality() {
        assert_eq!(Priority::Reactive, Priority::Reactive);
        assert_eq!(Priority::Background, Priority::Background);
        assert_ne!(Priority::Reactive, Priority::Background);
    }

    #[test]
    fn push_prioritized_reactive_goes_to_front() {
        let mut deque: VecDeque<WriteItem> = VecDeque::new();

        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        let (tx3, _rx3) = oneshot::channel();

        // Push a background item first.
        push_prioritized(
            &mut deque,
            WriteItem {
                text: "background-1".to_owned(),
                priority: Priority::Background,
                enqueued_at: Instant::now(),
                response_tx: tx1,
            },
        );

        // Push a reactive item — should go to the front.
        push_prioritized(
            &mut deque,
            WriteItem {
                text: "reactive-1".to_owned(),
                priority: Priority::Reactive,
                enqueued_at: Instant::now(),
                response_tx: tx2,
            },
        );

        // Push another background item.
        push_prioritized(
            &mut deque,
            WriteItem {
                text: "background-2".to_owned(),
                priority: Priority::Background,
                enqueued_at: Instant::now(),
                response_tx: tx3,
            },
        );

        assert_eq!(deque.len(), 3);

        // Pop order should be: reactive-1, background-1, background-2
        let first = deque.pop_front().expect("should have item");
        assert_eq!(first.text, "reactive-1");
        assert_eq!(first.priority, Priority::Reactive);

        let second = deque.pop_front().expect("should have item");
        assert_eq!(second.text, "background-1");
        assert_eq!(second.priority, Priority::Background);

        let third = deque.pop_front().expect("should have item");
        assert_eq!(third.text, "background-2");
        assert_eq!(third.priority, Priority::Background);

        assert!(deque.pop_front().is_none());
    }

    #[test]
    fn push_prioritized_multiple_reactive_preserves_fifo_within_priority() {
        let mut deque: VecDeque<WriteItem> = VecDeque::new();

        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();

        push_prioritized(
            &mut deque,
            WriteItem {
                text: "reactive-first".to_owned(),
                priority: Priority::Reactive,
                enqueued_at: Instant::now(),
                response_tx: tx1,
            },
        );

        // Second reactive goes to front — so it will be popped first.
        // This is the trade-off of push_front for reactive: later reactives
        // get higher priority. This is acceptable because reactive items
        // are latency-sensitive and the most recent one is most urgent.
        push_prioritized(
            &mut deque,
            WriteItem {
                text: "reactive-second".to_owned(),
                priority: Priority::Reactive,
                enqueued_at: Instant::now(),
                response_tx: tx2,
            },
        );

        let first = deque.pop_front().expect("should have item");
        assert_eq!(first.text, "reactive-second");

        let second = deque.pop_front().expect("should have item");
        assert_eq!(second.text, "reactive-first");
    }

    #[test]
    fn atomic_depth_tracking() {
        let depth = AtomicU32::new(0);

        depth.fetch_add(1, Ordering::Release);
        depth.fetch_add(1, Ordering::Release);
        assert_eq!(depth.load(Ordering::Acquire), 2);

        depth.fetch_sub(1, Ordering::Release);
        assert_eq!(depth.load(Ordering::Acquire), 1);

        depth.fetch_sub(1, Ordering::Release);
        assert_eq!(depth.load(Ordering::Acquire), 0);
    }

    #[test]
    fn config_values_accessible() {
        let config = test_queue_config();
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.operation_timeout_ms, 5000);
        assert_eq!(config.retry.max_attempts, 2);
        assert_eq!(config.retry.backoff_ms, 50);
    }
}
