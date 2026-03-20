//! gRPC EventBusService handler for daemon-bus.
//!
//! Bridges the internal tokio broadcast event bus to gRPC so that child
//! subsystems can:
//! - Publish events into the bus via `EventBusService.Publish`
//! - Subscribe to events streamed from the bus via `EventBusService.Subscribe`
//!
//! The Subscribe RPC returns a server-streaming response. Each matching
//! `InternalBusEvent` is converted to the proto `BusEvent` type and sent to
//! the subscriber over the open gRPC stream. The stream terminates when the
//! bus shuts down or the client disconnects.

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::bus::{EventBus, InternalBusEvent};
use crate::generated::sena_daemonbus_v1::{
    event_bus_service_server::EventBusService, BusEvent, EventTopic, PublishRequest,
    PublishResponse, SubscribeRequest,
};

/// The gRPC service handler for EventBusService.
///
/// Holds a cloneable `EventBus` handle (Clone via inner Arc) and routes
/// gRPC publish/subscribe requests to the internal bus.
#[derive(Clone)]
pub struct EventBusServiceHandler {
    event_bus: EventBus,
}

impl EventBusServiceHandler {
    pub fn new(event_bus: EventBus) -> Self {
        Self { event_bus }
    }
}

#[tonic::async_trait]
impl EventBusService for EventBusServiceHandler {
    /// Publish an event to the internal bus.
    ///
    /// The caller provides a complete `BusEvent` proto. It is converted to an
    /// `InternalBusEvent` and published to the broadcast channel. Fire-and-forget
    /// from the caller's perspective — the response indicates acceptance, not delivery.
    async fn publish(
        &self,
        request: Request<PublishRequest>,
    ) -> Result<Response<PublishResponse>, Status> {
        let req = request.into_inner();
        let bus_event_proto = req.event.ok_or_else(|| {
            Status::invalid_argument("PublishRequest.event must not be empty")
        })?;

        // Convert the topic i32 to the EventTopic enum.
        let topic = EventTopic::try_from(bus_event_proto.topic).map_err(|_| {
            Status::invalid_argument(format!(
                "unknown EventTopic value: {}",
                bus_event_proto.topic
            ))
        })?;

        let internal_event = InternalBusEvent {
            event_id: bus_event_proto.event_id,
            topic,
            source_subsystem: bus_event_proto.source_subsystem,
            payload: bus_event_proto.payload,
            trace_context: bus_event_proto.trace_context,
            timestamp: bus_event_proto.timestamp,
        };

        tracing::debug!(
            subsystem = "daemon_bus",
            event_type = "event_bus_grpc_publish",
            topic = ?topic,
            source = %internal_event.source_subsystem,
            "received publish request via gRPC"
        );

        let _receiver_count = self.event_bus.publish(internal_event).map_err(|publish_err| {
            tracing::error!(
                subsystem = "daemon_bus",
                event_type = "event_bus_publish_failed",
                error = %publish_err.message,
                "failed to publish event to internal bus"
            );
            Status::internal(format!("publish failed: {}", publish_err.message))
        })?;

        Ok(Response::new(PublishResponse { accepted: true }))
    }

    /// Server-streaming type alias required by the generated trait.
    type SubscribeStream = ReceiverStream<Result<BusEvent, Status>>;

    /// Subscribe to events matching the requested topics.
    ///
    /// Opens a subscription on the internal broadcast event bus and streams
    /// matching events to the gRPC caller. The stream ends when:
    /// - The internal event bus closes (daemon-bus is shutting down), or
    /// - The client disconnects (tonic drops the sink when the connection is gone).
    ///
    /// Topic filtering matches any event whose topic is in the subscriber's
    /// `topics` list. An empty list means all topics.
    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let req = request.into_inner();
        let subscriber_id = req.subscriber_id.clone();

        // Convert the repeated i32 topic values to EventTopic enums.
        // Unknown topic values are silently skipped — subscribers should only
        // request topics they understand.
        let topics_of_interest: Vec<EventTopic> = req
            .topics
            .iter()
            .filter_map(|topic_i32| EventTopic::try_from(*topic_i32).ok())
            .collect();

        tracing::info!(
            subsystem = "daemon_bus",
            event_type = "event_bus_grpc_subscribe",
            subscriber_id = %subscriber_id,
            topic_count = topics_of_interest.len(),
            "new gRPC event bus subscriber"
        );

        // Subscribe to the internal bus. This is an async operation that
        // updates the subscriber_counts diagnostic map.
        let mut bus_subscriber = self
            .event_bus
            .subscribe(&subscriber_id, &topics_of_interest)
            .await;

        // We bridge the bus subscriber into a tokio mpsc channel so we can
        // return a ReceiverStream. The mpsc channel is bounded to 64 messages
        // — if the gRPC sink is slow, the bridge task will block on send and
        // will not drop messages. The bridge task runs until the bus closes or
        // the mpsc sender detects the receiver (gRPC sink) has disconnected.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<BusEvent, Status>>(64);

        // If this subscriber is interested in boot signals (or all topics),
        // replay every boot signal that fired before this subscription was
        // created. The broadcast receiver was created above — any events
        // published AFTER that subscribe() call are already queued there.
        // Events published BEFORE (e.g. DAEMON_BUS_READY) are in the replay
        // cache and are sent here so the subscriber never misses them.
        let wants_boot_signals = topics_of_interest.is_empty()
            || topics_of_interest.contains(&EventTopic::TopicBootSignal);

        if wants_boot_signals {
            let past_signals = self.event_bus.boot_signal_snapshot();
            for past_event in past_signals {
                tracing::debug!(
                    subsystem = "daemon_bus",
                    event_type = "event_bus_grpc_boot_signal_replayed",
                    subscriber_id = %subscriber_id,
                    topic = ?past_event.topic,
                    source = %past_event.source_subsystem,
                    "replaying already-fired boot signal to new subscriber"
                );
                let proto_event = internal_event_to_proto(past_event);
                if tx.send(Ok(proto_event)).await.is_err() {
                    // Subscriber disconnected during replay — stop immediately.
                    return Ok(Response::new(ReceiverStream::new(rx)));
                }
            }
        }

        let subscriber_id_clone = subscriber_id.clone();
        tokio::spawn(async move {
            loop {
                match bus_subscriber.recv().await {
                    Ok(internal_event) => {
                        let proto_event = internal_event_to_proto(internal_event);
                        // If the receiver (gRPC sink) has dropped, stop bridging.
                        if tx.send(Ok(proto_event)).await.is_err() {
                            tracing::debug!(
                                subsystem = "daemon_bus",
                                event_type = "event_bus_grpc_subscriber_disconnected",
                                subscriber_id = %subscriber_id_clone,
                                "gRPC event bus subscriber disconnected — bridge task stopping"
                            );
                            break;
                        }
                    }
                    Err(bus_err) => {
                        // Bus shut down or unrecoverable error.
                        tracing::info!(
                            subsystem = "daemon_bus",
                            event_type = "event_bus_closed",
                            subscriber_id = %subscriber_id_clone,
                            error = %bus_err.message,
                            "internal event bus closed — ending subscriber stream"
                        );
                        // Send a terminal error to the client so it knows the stream ended.
                        // Ignore send errors — the receiver may already be gone.
                        let _ = tx
                            .send(Err(Status::unavailable("daemon-bus event bus closed")))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Convert an `InternalBusEvent` to the proto `BusEvent` type.
fn internal_event_to_proto(event: InternalBusEvent) -> BusEvent {
    BusEvent {
        event_id: event.event_id,
        topic: event.topic as i32,
        source_subsystem: event.source_subsystem,
        payload: event.payload,
        trace_context: event.trace_context,
        timestamp: event.timestamp,
    }
}
