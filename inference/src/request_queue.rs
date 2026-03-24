//! Priority request queue for serializing inference calls.
//!
//! Reactive (tier 1) pops before Standard (tier 3) which pops before
//! Background (tier 4). Enforces max depth and expires stale requests.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, Mutex, Notify};

use crate::error::InferenceError;
use crate::generated::sena_daemonbus_v1::{CompleteResponse, StreamCompleteChunk};

/// Priority tiers matching the global daemon-bus priority system.
/// Lower numeric value = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Tier 1 — user-facing reactive requests.
    Reactive = 1,
    /// Tier 3 — normal agent operation.
    Standard = 3,
    /// Tier 4 — CTP default, background work.
    Background = 4,
}

impl Priority {
    /// Convert from the proto `priority` i32 field.
    pub fn from_proto(value: i32) -> Self {
        match value {
            1 => Priority::Reactive,
            3 => Priority::Standard,
            4 => Priority::Background,
            // Default unknown priorities to standard
            _ => Priority::Standard,
        }
    }
}

/// Response channel type — either complete or streaming.
pub enum ResponseChannel {
    Complete(oneshot::Sender<Result<CompleteResponse, InferenceError>>),
    Stream(mpsc::Sender<Result<StreamCompleteChunk, InferenceError>>),
}

impl ResponseChannel {
    /// Try to send an error, consuming the channel.
    pub fn try_send_error(self, error: InferenceError) {
        match self {
            ResponseChannel::Complete(tx) => {
                let _send_result = tx.send(Err(error));
            }
            ResponseChannel::Stream(tx) => {
                let _send_result = tx.try_send(Err(error));
            }
        }
    }
}

/// A request waiting in the queue with its response channel.
pub struct QueuedRequest {
    /// The TOON-encoded prompt to complete.
    pub prompt: String,
    /// Target model ID (empty = active model).
    pub model_id: String,
    /// Max tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// Request correlation ID.
    pub request_id: String,
    /// Priority tier for ordering.
    pub priority: Priority,
    /// When this request was enqueued — for timeout checks.
    pub enqueued_at: Instant,
    /// Channel to send the result back to the gRPC handler.
    pub response_channel: ResponseChannel,
}

// BinaryHeap is a max-heap, so we want higher-priority (lower tier number) to be "greater"
impl Eq for QueuedRequest {}
impl PartialEq for QueuedRequest {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.enqueued_at == other.enqueued_at
    }
}
impl Ord for QueuedRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower tier number = higher priority = should come first (be "greater" in max-heap)
        // So reverse the comparison on tier number
        let priority_ord = (other.priority as i32).cmp(&(self.priority as i32));
        match priority_ord {
            Ordering::Equal => {
                // Same priority: older requests first (FIFO within tier)
                other.enqueued_at.cmp(&self.enqueued_at)
            }
            other => other,
        }
    }
}
impl PartialOrd for QueuedRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Priority-aware async request queue.
pub struct RequestQueue {
    inner: Mutex<BinaryHeap<QueuedRequest>>,
    notify: Notify,
    max_depth: usize,
    request_timeout_ms: u64,
}

impl RequestQueue {
    pub fn new(max_depth: usize, request_timeout_ms: u64) -> Self {
        Self {
            inner: Mutex::new(BinaryHeap::new()),
            notify: Notify::new(),
            max_depth,
            request_timeout_ms,
        }
    }

    /// Push a request. Returns RequestQueueFull if at max depth.
    pub async fn push(&self, request: QueuedRequest) -> Result<(), InferenceError> {
        let mut queue = self.inner.lock().await;
        if queue.len() >= self.max_depth {
            let max_depth = self.max_depth;
            // Send error to the caller before rejecting
            request
                .response_channel
                .try_send_error(InferenceError::RequestQueueFull { max_depth });
            return Err(InferenceError::RequestQueueFull { max_depth });
        }
        queue.push(request);
        drop(queue); // release lock before notify
        self.notify.notify_one();
        Ok(())
    }

    /// Pop the highest-priority non-expired request. Blocks until one is available.
    /// Expired requests have timeout errors sent on their response_channel.
    pub async fn pop(&self) -> Option<QueuedRequest> {
        loop {
            {
                let mut queue = self.inner.lock().await;
                while let Some(request) = queue.pop() {
                    let elapsed_ms = request.enqueued_at.elapsed().as_millis() as u64;
                    if elapsed_ms > self.request_timeout_ms {
                        let timeout_ms = self.request_timeout_ms;
                        // Send timeout error — if receiver dropped, that's fine
                        request
                            .response_channel
                            .try_send_error(InferenceError::RequestTimeout { timeout_ms });
                        continue; // try next request
                    }
                    return Some(request);
                }
            }
            // Queue empty — wait for notification
            self.notify.notified().await;
        }
    }

    /// Current number of items in the queue.
    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    /// Check if the queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_request(
        priority: Priority,
        request_id: &str,
    ) -> (
        QueuedRequest,
        oneshot::Receiver<Result<CompleteResponse, InferenceError>>,
    ) {
        let (tx, rx) = oneshot::channel();
        let req = QueuedRequest {
            prompt: "test prompt".into(),
            model_id: String::new(),
            max_tokens: 100,
            temperature: 0.7,
            request_id: request_id.into(),
            priority,
            enqueued_at: Instant::now(),
            response_channel: ResponseChannel::Complete(tx),
        };
        (req, rx)
    }

    #[tokio::test]
    async fn test_push_and_pop_returns_item() {
        let queue = RequestQueue::new(10, 30000);
        let (req, _rx) = make_request(Priority::Standard, "r1");
        queue.push(req).await.expect("test: push should succeed");
        let popped = queue.pop().await;
        assert!(popped.is_some());
        assert_eq!(popped.expect("test: just checked is_some").request_id, "r1");
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let queue = RequestQueue::new(10, 30000);
        let (bg_req, _rx1) = make_request(Priority::Background, "bg");
        let (reactive_req, _rx2) = make_request(Priority::Reactive, "reactive");

        queue.push(bg_req).await.expect("test: push bg");
        queue.push(reactive_req).await.expect("test: push reactive");

        // Reactive should pop first despite being pushed second
        let first = queue.pop().await.expect("test: should have item");
        assert_eq!(first.request_id, "reactive");
        let second = queue.pop().await.expect("test: should have item");
        assert_eq!(second.request_id, "bg");
    }

    #[tokio::test]
    async fn test_queue_full_returns_error() {
        let queue = RequestQueue::new(1, 30000);
        let (req1, _rx1) = make_request(Priority::Standard, "r1");
        let (req2, _rx2) = make_request(Priority::Standard, "r2");

        queue
            .push(req1)
            .await
            .expect("test: first push should succeed");
        let result = queue.push(req2).await;
        assert!(result.is_err());
        match result.expect_err("test: should be queue full") {
            InferenceError::RequestQueueFull { max_depth } => assert_eq!(max_depth, 1),
            other => panic!("Expected RequestQueueFull, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_expired_request_discarded_at_pop() {
        let queue = RequestQueue::new(10, 1); // 1ms timeout
        let (tx, rx) = oneshot::channel();
        let req = QueuedRequest {
            prompt: "test".into(),
            model_id: String::new(),
            max_tokens: 10,
            temperature: 0.0,
            request_id: "expired".into(),
            priority: Priority::Standard,
            enqueued_at: Instant::now() - Duration::from_secs(1), // already expired
            response_channel: ResponseChannel::Complete(tx),
        };

        // Also push a non-expired request so pop doesn't block forever
        let (fresh_req, _rx2) = make_request(Priority::Standard, "fresh");

        queue.push(req).await.expect("test: push expired");
        queue.push(fresh_req).await.expect("test: push fresh");

        // Should skip expired, return fresh
        let popped = queue.pop().await.expect("test: should get fresh");
        assert_eq!(popped.request_id, "fresh");

        // The expired request's receiver should have the timeout error
        let expired_result = rx.await.expect("test: should receive timeout error");
        assert!(expired_result.is_err());
    }

    #[tokio::test]
    async fn test_len_accurate() {
        let queue = RequestQueue::new(10, 30000);
        assert_eq!(queue.len().await, 0);

        let (req1, _rx1) = make_request(Priority::Standard, "r1");
        let (req2, _rx2) = make_request(Priority::Standard, "r2");
        queue.push(req1).await.expect("test: push");
        queue.push(req2).await.expect("test: push");
        assert_eq!(queue.len().await, 2);
    }

    #[tokio::test]
    async fn test_pop_blocks_until_item() {
        let queue = std::sync::Arc::new(RequestQueue::new(10, 30000));
        let queue_clone = std::sync::Arc::clone(&queue);

        let handle = tokio::spawn(async move { queue_clone.pop().await });

        // Small delay to let pop() start waiting
        tokio::time::sleep(Duration::from_millis(50)).await;

        let (req, _rx) = make_request(Priority::Standard, "delayed");
        queue.push(req).await.expect("test: push");

        let result = handle.await.expect("test: join handle");
        assert!(result.is_some());
        assert_eq!(result.expect("test: just checked").request_id, "delayed");
    }
}
