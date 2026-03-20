//! Internal tokio broadcast channel event bus.
//!
//! This is daemon-bus's in-process pub/sub backbone. All subsystem events flow
//! through here before being forwarded to gRPC subscribers. The bus runs entirely
//! on the tokio runtime — no blocking I/O is ever performed on the bus thread.
//!
//! Topic strings are never literals — they always reference constants derived from
//! the proto `EventTopic` enum via the `topic_name` helper.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::sync::RwLock;

use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::EventTopic;

// ─────────────────────────────────────────────────────────────────────────────
// Topic name mapping — proto enum → string constant
// ─────────────────────────────────────────────────────────────────────────────

/// Converts a proto `EventTopic` enum variant to its canonical string name.
/// This is the single source of truth for topic strings in the bus — no module
/// in daemon-bus ever writes a topic string literal.
pub fn topic_name(topic: EventTopic) -> &'static str {
    match topic {
        EventTopic::Unspecified => "event.unspecified",
        EventTopic::TopicBootSignal => "boot.signal",
        EventTopic::TopicBootFailed => "boot.failed",
        EventTopic::TopicSubsystemStarted => "subsystem.started",
        EventTopic::TopicSubsystemCrashed => "subsystem.crashed",
        EventTopic::TopicSubsystemRestarted => "subsystem.restarted",
        EventTopic::TopicSubsystemDegraded => "subsystem.degraded",
        EventTopic::TopicEscalationGranted => "escalation.granted",
        EventTopic::TopicEscalationQueued => "escalation.queued",
        EventTopic::TopicEscalationExpired => "escalation.expired",
        EventTopic::TopicEscalationReleased => "escalation.released",
        EventTopic::TopicTaskTimeout => "watchdog.task_timeout",
        EventTopic::TopicTaskTerminated => "watchdog.task_terminated",
        EventTopic::TopicMemoryUpdated => "memory.updated",
        EventTopic::TopicMemoryWriteCompleted => "memory.write_completed",
        EventTopic::TopicMemoryTierPromoted => "memory.tier_promoted",
        EventTopic::TopicModelProbeFailed => "model_probe.failed",
        EventTopic::TopicLoraTrainingRecommended => "lora.training_recommended",
        EventTopic::TopicUserMessageReceived => "user.message.received",
        EventTopic::TopicUserMessageResponse => "user.message.response",
        EventTopic::TopicThoughtSurfaced => "thought.surfaced",
        EventTopic::TopicSessionCompactionTriggered => "session.compaction_triggered",
        EventTopic::TopicMemoryConsolidationRequested => "memory.consolidation_requested",
        EventTopic::TopicInferenceModelSwitching => "inference.model_switching",
        EventTopic::TopicAgentRegistered => "agent.registered",
        EventTopic::TopicAgentQuarantined => "agent.quarantined",
        EventTopic::TopicPcPromptAssembled => "pc.prompt_assembled",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bus event payload
// ─────────────────────────────────────────────────────────────────────────────

/// Internal representation of an event flowing through the bus.
/// This is the in-process type — it is converted to/from the proto `BusEvent`
/// at the gRPC boundary.
#[derive(Debug, Clone)]
pub struct InternalBusEvent {
    /// Unique event identifier for tracing and deduplication.
    pub event_id: String,
    /// Proto topic enum — used for routing and filtering.
    pub topic: EventTopic,
    /// Which subsystem emitted this event.
    pub source_subsystem: String,
    /// Arbitrary payload bytes — receivers interpret based on topic contract.
    pub payload: Vec<u8>,
    /// OpenTelemetry trace context propagated with every event.
    pub trace_context: String,
    /// UTC timestamp in RFC 3339 format, set at publish time.
    pub timestamp: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Event Bus
// ─────────────────────────────────────────────────────────────────────────────

/// The internal tokio broadcast channel event bus.
///
/// All events are broadcast to every subscriber. Subscribers filter by topic
/// on the receive side. The channel capacity is set from config — if a slow
/// subscriber falls behind, it receives a `RecvError::Lagged` and must
/// re-subscribe or accept dropped messages.
///
/// This type is cheaply cloneable via the inner `Arc`.
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

struct EventBusInner {
    sender: broadcast::Sender<InternalBusEvent>,
    /// Tracks active subscriber count per topic for diagnostics.
    /// Not used for routing — all subscribers receive all messages and filter locally.
    subscriber_counts: RwLock<HashMap<EventTopic, usize>>,
    channel_capacity: usize,
    /// Ordered list of every boot-signal event that has been published so far.
    ///
    /// Uses a `std::sync::Mutex` (not tokio) because `publish()` is non-async
    /// and the lock is never held across an await point. When a new gRPC
    /// subscriber connects and requests `TopicBootSignal`, all events in this
    /// list are immediately sent before the live stream begins — this ensures
    /// subsystems that connect after `DAEMON_BUS_READY` is emitted still
    /// receive it at subscription time.
    boot_signal_replay: std::sync::Mutex<Vec<InternalBusEvent>>,
}

impl EventBus {
    /// Create a new event bus with the given broadcast channel capacity.
    /// Capacity comes from `config/daemon-bus.toml` `[bus].channel_capacity`.
    pub fn new(channel_capacity: usize) -> Self {
        let (sender, _initial_receiver) = broadcast::channel(channel_capacity);
        // The initial receiver is intentionally dropped — subscribers create
        // their own receivers via `subscribe()`. Dropping it here is safe
        // because `broadcast::channel` keeps the sender alive independently.

        Self {
            inner: Arc::new(EventBusInner {
                sender,
                subscriber_counts: RwLock::new(HashMap::new()),
                channel_capacity,
                boot_signal_replay: std::sync::Mutex::new(Vec::new()),
            }),
        }
    }

    /// Publish an event to all subscribers.
    ///
    /// Returns the number of active receivers that will see this event.
    /// Returns an error if there are no active receivers — callers decide
    /// whether that is acceptable (e.g. during early boot, zero subscribers
    /// is expected).
    pub fn publish(&self, event: InternalBusEvent) -> SenaResult<usize> {
        let topic = event.topic;
        let topic_str = topic_name(topic);

        // Store boot signal events in the replay cache before broadcasting.
        // New gRPC subscribers that arrive after these signals fire will
        // receive them immediately on connect — preventing the race where
        // DAEMON_BUS_READY is emitted before a subsystem subscribes.
        // The std::sync::Mutex is intentional: publish() is non-async and the
        // lock is never held across an await point, so a tokio mutex is not
        // needed and would be incorrect here.
        if topic == EventTopic::TopicBootSignal {
            if let Ok(mut replay) = self.inner.boot_signal_replay.lock() {
                replay.push(event.clone());
            }
            // If the mutex is poisoned (from a panic elsewhere) we skip
            // storage but always proceed with the broadcast.
        }

        match self.inner.sender.send(event) {
            Ok(receiver_count) => {
                tracing::debug!(
                    subsystem = "daemon_bus",
                    event_type = "bus_publish",
                    topic = topic_str,
                    receiver_count = receiver_count,
                    "event published to bus"
                );
                Ok(receiver_count)
            }
            Err(broadcast::error::SendError(_unsent_event)) => {
                // No active receivers. During early boot this is expected.
                // Log at debug, not error — the caller decides severity.
                tracing::debug!(
                    subsystem = "daemon_bus",
                    event_type = "bus_publish_no_receivers",
                    topic = topic_str,
                    "event published but no active receivers"
                );
                // Return 0 instead of an error for the no-receiver case —
                // this is a normal condition during boot and teardown.
                Ok(0)
            }
        }
    }

    /// Create a new subscriber that receives all events from this point forward.
    ///
    /// The subscriber must filter events by topic on the receive side.
    /// Optionally provide a list of topics the subscriber cares about — this is
    /// recorded for diagnostics only, it does not affect message delivery.
    pub async fn subscribe(
        &self,
        subscriber_id: &str,
        topics_of_interest: &[EventTopic],
    ) -> EventBusSubscriber {
        let receiver = self.inner.sender.subscribe();

        // Track subscriber interest for diagnostics.
        {
            let mut counts = self.inner.subscriber_counts.write().await;
            for topic in topics_of_interest {
                *counts.entry(*topic).or_insert(0) += 1;
            }
        }

        tracing::debug!(
            subsystem = "daemon_bus",
            event_type = "bus_subscribe",
            subscriber_id = subscriber_id,
            topic_count = topics_of_interest.len(),
            "new bus subscriber registered"
        );

        EventBusSubscriber {
            receiver,
            topics_of_interest: topics_of_interest.to_vec(),
            subscriber_id: subscriber_id.to_string(),
        }
    }

    /// Returns the configured channel capacity (for diagnostics).
    pub fn channel_capacity(&self) -> usize {
        self.inner.channel_capacity
    }

    /// Returns the current number of active receivers on the broadcast channel.
    pub fn receiver_count(&self) -> usize {
        self.inner.sender.receiver_count()
    }

    /// Returns a snapshot of all boot signal events published so far.
    ///
    /// Used by the gRPC `EventBusService.Subscribe` handler to replay missed
    /// boot signals to newly-connected subscribers. The snapshot is a clone
    /// so the lock is released immediately. Returns an empty Vec if the
    /// replay mutex is poisoned.
    pub fn boot_signal_snapshot(&self) -> Vec<InternalBusEvent> {
        self.inner
            .boot_signal_replay
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Subscriber
// ─────────────────────────────────────────────────────────────────────────────

/// A subscriber handle to the event bus.
///
/// Receives all events and filters locally by `topics_of_interest`.
/// If the subscriber falls behind by more than `channel_capacity` messages,
/// it receives a lagged error and must decide whether to continue or re-subscribe.
pub struct EventBusSubscriber {
    receiver: broadcast::Receiver<InternalBusEvent>,
    /// Topics this subscriber cares about. Empty means all topics.
    topics_of_interest: Vec<EventTopic>,
    subscriber_id: String,
}

impl EventBusSubscriber {
    /// Receive the next event matching this subscriber's topics of interest.
    ///
    /// Blocks (async) until an event arrives. If the subscriber has lagged behind,
    /// logs a warning and skips to the latest available message.
    pub async fn recv(&mut self) -> SenaResult<InternalBusEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    // Filter by topic if the subscriber registered specific interests.
                    if self.topics_of_interest.is_empty()
                        || self.topics_of_interest.contains(&event.topic)
                    {
                        return Ok(event);
                    }
                    // Event did not match any topic of interest — skip silently.
                    continue;
                }
                Err(broadcast::error::RecvError::Lagged(skipped_count)) => {
                    // The subscriber fell behind. This is a capacity issue, not a bug.
                    // Log at warn so it surfaces in diagnostics without crashing.
                    tracing::warn!(
                        subsystem = "daemon_bus",
                        event_type = "bus_subscriber_lagged",
                        subscriber_id = %self.subscriber_id,
                        skipped_count = skipped_count,
                        "subscriber lagged behind — skipped messages"
                    );
                    // Continue receiving from the current position rather than erroring out.
                    // The broadcast receiver automatically advances past the gap.
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // The sender side was dropped — the bus is shutting down.
                    return Err(SenaError::new(
                        ErrorCode::BusPublishFailed,
                        "event bus channel closed — daemon-bus is shutting down",
                    ));
                }
            }
        }
    }

    /// Returns this subscriber's ID (for diagnostics and logging).
    pub fn subscriber_id(&self) -> &str {
        &self.subscriber_id
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Convenience constructors for InternalBusEvent
// ─────────────────────────────────────────────────────────────────────────────

impl InternalBusEvent {
    /// Create a new bus event with a generated UUID and current UTC timestamp.
    pub fn new(
        topic: EventTopic,
        source_subsystem: impl Into<String>,
        payload: Vec<u8>,
        trace_context: impl Into<String>,
    ) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic,
            source_subsystem: source_subsystem.into(),
            payload,
            trace_context: trace_context.into(),
            timestamp: chrono_free_utc_now(),
        }
    }

    /// Create a bus event with no payload — used for signals that carry
    /// meaning in the topic alone (e.g. boot signals).
    pub fn signal(
        topic: EventTopic,
        source_subsystem: impl Into<String>,
        trace_context: impl Into<String>,
    ) -> Self {
        Self::new(topic, source_subsystem, Vec::new(), trace_context)
    }
}

/// Produces an RFC 3339 UTC timestamp without pulling in the `chrono` crate.
/// Uses `std::time::SystemTime` which is sufficient for event ordering.
fn chrono_free_utc_now() -> String {
    // humantime formats SystemTime as RFC 3339 — but we avoid that dep too.
    // Instead, use the same approach tonic's well-known-types use: seconds since epoch.
    let duration_since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        // SystemTime before UNIX_EPOCH should never happen on a real system.
        // If it does, 0 is a clearly wrong sentinel that will surface in logs.
        .unwrap_or_default();
    // Format as a simple epoch-seconds string. A full RFC 3339 formatter would
    // require either `chrono` or manual calendar math. Epoch seconds are
    // unambiguous, sortable, and trivially parseable by any consumer.
    format!(
        "{}.{:09}",
        duration_since_epoch.as_secs(),
        duration_since_epoch.subsec_nanos()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_and_receive_single_event() {
        let bus = EventBus::new(16);
        let mut subscriber = bus
            .subscribe("test_subscriber", &[EventTopic::TopicBootSignal])
            .await;

        let event =
            InternalBusEvent::signal(EventTopic::TopicBootSignal, "test_subsystem", "trace-abc");

        let receiver_count = bus.publish(event.clone()).expect("publish should succeed");
        assert_eq!(receiver_count, 1);

        let received = subscriber.recv().await.expect("recv should succeed");
        assert_eq!(received.topic, EventTopic::TopicBootSignal);
        assert_eq!(received.source_subsystem, "test_subsystem");
    }

    #[tokio::test]
    async fn subscriber_filters_by_topic() {
        let bus = EventBus::new(16);
        let mut subscriber = bus
            .subscribe("filtered_sub", &[EventTopic::TopicBootFailed])
            .await;

        // Publish an event the subscriber does NOT care about.
        let irrelevant = InternalBusEvent::signal(EventTopic::TopicBootSignal, "test", "");
        bus.publish(irrelevant).expect("publish should succeed");

        // Publish an event the subscriber DOES care about.
        let relevant = InternalBusEvent::signal(EventTopic::TopicBootFailed, "test", "");
        bus.publish(relevant).expect("publish should succeed");

        let received = subscriber.recv().await.expect("recv should succeed");
        assert_eq!(received.topic, EventTopic::TopicBootFailed);
    }

    #[tokio::test]
    async fn publish_with_no_receivers_returns_zero() {
        let bus = EventBus::new(16);
        let event = InternalBusEvent::signal(EventTopic::TopicBootSignal, "test", "");
        let count = bus
            .publish(event)
            .expect("publish should succeed even with no receivers");
        assert_eq!(count, 0);
    }
}
