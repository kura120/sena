use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tonic::transport::Channel;
use tonic::Request;

use crate::config::ReconnectConfig;
use crate::debug_state::{
    BootSignalEntry, BusEventEntry, DebugState, SubsystemHealthStatus, ThoughtEvent,
    REQUIRED_BOOT_SIGNALS,
};
use prost::Message as ProstMessage;
use crate::proto::event_bus_service_client::EventBusServiceClient;
use crate::proto::{BootSignal, BootSignalRequest, EventTopic, SubscribeRequest};

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

    // Mark as connected and record start time.
    {
        let mut state = debug_state.write().await;
        state.connected = true;
        if state.started_at.is_none() {
            state.started_at = Some(chrono::Utc::now());
        }
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

        // Increment total event counter.
        state.total_events_received += 1;

        // For boot signal events, show the actual signal name instead of the
        // generic "Boot Signal" topic label.
        let display_topic = if topic_value == EventTopic::TopicBootSignal as i32 {
            let sig = extract_boot_signal_name(&event.payload);
            if sig == "UNKNOWN" {
                crate::debug_state::format_topic_name(topic_value).to_string()
            } else {
                sig
            }
        } else {
            crate::debug_state::format_topic_name(topic_value).to_string()
        };

        // Push every event to the raw event feed.
        state.push_event(BusEventEntry {
            topic: display_topic,
            source_subsystem: event.source_subsystem.clone(),
            payload_summary: crate::debug_state::truncate_with_ellipsis(&payload_str, 60),
            timestamp,
        });

        // Handle specific topic-based state updates.
        match topic_value {
            // Boot signals — subsystem ready/degraded/unavailable.
            t if t == EventTopic::TopicBootSignal as i32 => {
                tracing::debug!(component = "grpc", raw_payload = %String::from_utf8_lossy(&event.payload), "boot signal raw payload");
                handle_boot_signal(&mut state, &event.source_subsystem, &event.payload, timestamp);
            }
            t if t == EventTopic::TopicSubsystemDegraded as i32 => {
                state.set_subsystem_status(
                    &normalize_subsystem_name(&event.source_subsystem),
                    SubsystemHealthStatus::Degraded,
                );
            }
            t if t == EventTopic::TopicSubsystemCrashed as i32 => {
                // Normalize name: daemon-bus uses underscores in subsystem IDs
                // (e.g. "model_probe") but our keys use hyphens.
                state.set_subsystem_status(
                    &normalize_subsystem_name(&event.source_subsystem),
                    SubsystemHealthStatus::Degraded,
                );
            }
            t if t == EventTopic::TopicSubsystemStarted as i32 => {
                state.set_subsystem_status(
                    &normalize_subsystem_name(&event.source_subsystem),
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

/// Parse a boot signal payload and update subsystem health + boot signal history.
fn handle_boot_signal(
    state: &mut DebugState,
    source_subsystem: &str,
    payload: &[u8],
    timestamp: chrono::DateTime<chrono::Utc>,
) {
    // Attempt to extract the signal name from the payload.
    let signal_name = extract_boot_signal_name(payload);
    let required = REQUIRED_BOOT_SIGNALS.contains(&signal_name.as_str());

    // Map signal name to the correct subsystem — boot signals are often
    // broadcast by daemon-bus, so source_subsystem may not match the
    // subsystem that actually became ready.
    // Normalize the fallback subsystem name: daemon-bus config uses underscores
    // (e.g. "model_probe") but our keys always use hyphens.
    let normalized_source = normalize_subsystem_name(source_subsystem);
    let target_subsystem = signal_name_to_subsystem(&signal_name)
        .unwrap_or(normalized_source.as_str());

    // Push to boot signal history.
    state.push_boot_signal(BootSignalEntry {
        signal_name: signal_name.clone(),
        source_subsystem: target_subsystem.to_string(),
        required,
        timestamp,
    });

    // Boot signal payloads include the signal name; map to health status.
    let signal_upper = signal_name.to_uppercase();
    if signal_upper.contains("READY") {
        state.set_subsystem_status(target_subsystem, SubsystemHealthStatus::Ready);
    } else if signal_upper.contains("DEGRADED") {
        state.set_subsystem_status(target_subsystem, SubsystemHealthStatus::Degraded);
    } else if signal_upper.contains("UNAVAILABLE") || signal_upper.contains("FAILED") {
        state.set_subsystem_status(target_subsystem, SubsystemHealthStatus::Unavailable);
    }
}

/// Map a boot signal name to the subsystem it represents.
fn signal_name_to_subsystem(signal_name: &str) -> Option<&str> {
    match signal_name {
        "DAEMON_BUS_READY" => Some("daemon-bus"),
        "MEMORY_ENGINE_READY" => Some("memory-engine"),
        "PLATFORM_READY" => Some("platform"),
        "AGENTS_READY" => Some("agents"),
        "MODEL_PROFILE_READY" => Some("model-probe"),
        "CTP_READY" => Some("ctp"),
        "INFERENCE_READY" | "INFERENCE_UNAVAILABLE" | "INFERENCE_DEGRADED" => Some("inference"),
        "SOULBOX_READY" => Some("soulbox"),
        "PROMPT_COMPOSER_READY" => Some("prompt-composer"),
        "UI_READY" => Some("ui"),
        "LORA_READY" | "LORA_SKIPPED" => Some("lora-manager"),
        "SENA_READY" => Some("daemon-bus"),
        _ => None,
    }
}

/// Extract the boot signal name from the payload bytes.
/// Tries protobuf decode first (canonical daemon-bus encoding), then JSON,
/// then gives up and returns "UNKNOWN".
fn extract_boot_signal_name(payload: &[u8]) -> String {
    // Primary path: payload is a prost-encoded BootSignalRequest.
    if let Ok(req) = BootSignalRequest::decode(payload) {
        if let Ok(signal) = BootSignal::try_from(req.signal) {
            if signal != BootSignal::Unspecified {
                return signal.as_str_name().to_string();
            }
        }
    }
    // Secondary path: JSON encoding {"signal": "DAEMON_BUS_READY"} or {"signal": 1}.
    let payload_str = String::from_utf8_lossy(payload);
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload_str) {
        if let Some(name) = json.get("signal").and_then(|v| v.as_str()) {
            return name.to_string();
        }
        if let Some(val) = json.get("signal").and_then(|v| v.as_i64()) {
            if let Some(signal) = BootSignal::try_from(val as i32).ok() {
                return signal.as_str_name().to_string();
            }
        }
    }
    "UNKNOWN".to_string()
}

/// Normalize a subsystem identifier to use hyphens instead of underscores.
/// daemon-bus config keys use underscores (e.g. "model_probe") but our
/// display map uses hyphens ("model-probe"). Always convert on ingress.
fn normalize_subsystem_name(name: &str) -> String {
    name.replace('_', "-")
}
