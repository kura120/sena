use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tonic::transport::Channel;
use tonic::Request;

use crate::config::ReconnectConfig;
use crate::debug_state::{
    BusEventEntry, DebugState, SubsystemHealthStatus, ThoughtEvent,
};
use crate::proto::event_bus_service_client::EventBusServiceClient;
use crate::proto::{EventTopic, SubscribeRequest};

/// Connect to daemon-bus and subscribe to the event stream, updating DebugState on each event.
/// Reconnects on disconnect with exponential backoff.
pub async fn run_event_stream(
    address: String,
    connection_timeout_ms: u64,
    reconnect_config: ReconnectConfig,
    debug_state: Arc<RwLock<DebugState>>,
) {
    let backoff_delays = [
        Duration::from_millis(reconnect_config.initial_delay_ms),
        Duration::from_millis(reconnect_config.second_delay_ms),
        Duration::from_millis(reconnect_config.third_delay_ms),
        Duration::from_millis(reconnect_config.steady_state_delay_ms),
    ];

    let mut attempt: usize = 0;

    loop {
        tracing::info!(
            component = "grpc",
            event_type = "connecting",
            address = %address,
            attempt = attempt,
        );

        match connect_and_stream(
            &address,
            connection_timeout_ms,
            Arc::clone(&debug_state),
        )
        .await
        {
            Ok(()) => {
                // Stream ended normally (server shut down).
                tracing::info!(
                    component = "grpc",
                    event_type = "stream_ended",
                );
            }
            Err(grpc_error) => {
                tracing::warn!(
                    component = "grpc",
                    event_type = "stream_disconnected",
                    error = %grpc_error,
                );
            }
        }

        // Mark as disconnected.
        {
            let mut state = debug_state.write().await;
            state.connected = false;
        }

        // Backoff before reconnecting.
        let delay_index = attempt.min(backoff_delays.len() - 1);
        let delay = backoff_delays[delay_index];
        if delay > Duration::ZERO {
            tracing::info!(
                component = "grpc",
                event_type = "reconnect_backoff",
                delay_ms = delay.as_millis() as u64,
            );
            tokio::time::sleep(delay).await;
        }
        attempt += 1;
    }
}

async fn connect_and_stream(
    address: &str,
    connection_timeout_ms: u64,
    debug_state: Arc<RwLock<DebugState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let channel = Channel::from_shared(address.to_string())?
        .timeout(Duration::from_millis(connection_timeout_ms))
        .connect()
        .await?;

    let mut client = EventBusServiceClient::new(channel);

    // Subscribe to all topics (empty list = all).
    let request = Request::new(SubscribeRequest {
        topics: Vec::new(),
        subscriber_id: "ui-debug-panel".to_string(),
    });

    let mut stream = client.subscribe(request).await?.into_inner();

    // Mark as connected.
    {
        let mut state = debug_state.write().await;
        state.connected = true;
    }

    tracing::info!(
        component = "grpc",
        event_type = "stream_connected",
    );

    while let Some(event) = stream.message().await? {
        let topic_value = event.topic;
        let timestamp = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        let payload_str = String::from_utf8_lossy(&event.payload).to_string();

        let mut state = debug_state.write().await;

        // Push every event to the raw event feed.
        state.push_event(BusEventEntry {
            topic: crate::debug_state::format_topic_name(topic_value).to_string(),
            source_subsystem: event.source_subsystem.clone(),
            payload_summary: crate::debug_state::truncate_with_ellipsis(&payload_str, 60),
            timestamp,
        });

        // Handle specific topic-based state updates.
        match topic_value {
            // Boot signals — subsystem ready/degraded/unavailable.
            t if t == EventTopic::TopicBootSignal as i32 => {
                handle_boot_signal(&mut state, &event.source_subsystem, &payload_str);
            }
            t if t == EventTopic::TopicSubsystemDegraded as i32 => {
                state.set_subsystem_status(
                    &event.source_subsystem,
                    SubsystemHealthStatus::Degraded,
                );
            }
            t if t == EventTopic::TopicSubsystemCrashed as i32 => {
                state.set_subsystem_status(
                    &event.source_subsystem,
                    SubsystemHealthStatus::Unavailable,
                );
            }
            t if t == EventTopic::TopicSubsystemStarted as i32 => {
                state.set_subsystem_status(
                    &event.source_subsystem,
                    SubsystemHealthStatus::Ready,
                );
            }
            // CTP thought surfaced.
            t if t == EventTopic::TopicThoughtSurfaced as i32 => {
                if let Ok(thought_data) =
                    serde_json::from_str::<serde_json::Value>(&payload_str)
                {
                    let content = thought_data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let relevance_score = thought_data
                        .get("relevance_score")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as f32;
                    state.push_thought(ThoughtEvent {
                        content,
                        relevance_score,
                        timestamp,
                    });
                }
            }
            // Memory events — update counts from payload if available.
            t if t == EventTopic::TopicMemoryWriteCompleted as i32
                || t == EventTopic::TopicMemoryUpdated as i32 =>
            {
                state.memory_stats.last_write = Some(timestamp);
                if let Ok(mem_data) =
                    serde_json::from_str::<serde_json::Value>(&payload_str)
                {
                    if let Some(tier) = mem_data.get("tier").and_then(|v| v.as_str()) {
                        let count_field = match tier {
                            "short_term" => Some(&mut state.memory_stats.short_term_count),
                            "long_term" => Some(&mut state.memory_stats.long_term_count),
                            "episodic" => Some(&mut state.memory_stats.episodic_count),
                            _ => None,
                        };
                        if let Some(count) = count_field {
                            if let Some(new_count) =
                                mem_data.get("count").and_then(|v| v.as_u64())
                            {
                                *count = new_count as u32;
                            }
                        }
                    }
                }
            }
            _ => {
                // Other topics only appear in the event feed (already pushed above).
            }
        }
    }

    Ok(())
}

/// Parse a boot signal payload and update subsystem health.
fn handle_boot_signal(
    state: &mut DebugState,
    source_subsystem: &str,
    payload_str: &str,
) {
    // Boot signal payloads include the signal name; map to health status.
    let payload_lower = payload_str.to_lowercase();
    if payload_lower.contains("ready") {
        state.set_subsystem_status(source_subsystem, SubsystemHealthStatus::Ready);
    } else if payload_lower.contains("degraded") {
        state.set_subsystem_status(source_subsystem, SubsystemHealthStatus::Degraded);
    } else if payload_lower.contains("unavailable") || payload_lower.contains("failed") {
        state.set_subsystem_status(source_subsystem, SubsystemHealthStatus::Unavailable);
    }
}
