use crate::config::ReconnectConfig;
use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient,
    user_message_service_client::UserMessageServiceClient, BootSignal, BootSignalRequest,
    EventTopic, SubscribeRequest, UserMessageRequest, UserMessageResponse,
};
use crate::state::{BootSignalEntry, BusEventEntry, DebugState, SubsystemHealthStatus};
use chrono::{DateTime, Utc};
use prost::Message as ProstMessage;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tonic::transport::Channel;
use tracing::{error, info};

/// Payload for boot-signal-received Tauri event
#[derive(Debug, Clone, Serialize)]
pub struct BootSignalPayload {
    pub signal: String,
    pub required: bool,
    pub timestamp: String,
    pub subsystem: String,
}

/// Payload for subsystem-status-updated Tauri event
#[derive(Debug, Clone, Serialize)]
pub struct SubsystemStatusPayload {
    pub subsystem: String,
    pub status: String,
}

/// Payload for bus-event Tauri event
#[derive(Debug, Clone, Serialize)]
pub struct BusEventPayload {
    pub topic: String,
    pub source: String,
    pub payload: String,
    pub category: String,
    pub timestamp: String,
}

/// Map boot signal enum to subsystem name
pub fn signal_name_to_subsystem(signal_name: &str) -> Option<&str> {
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
        "REACTIVE_LOOP_READY" => Some("reactive-loop"),
        _ => None,
    }
}

/// Extract boot signal name from payload bytes
pub fn extract_boot_signal_name(payload: &[u8]) -> String {
    // Try protobuf decode first
    if let Ok(request) = BootSignalRequest::decode(payload) {
        if let Ok(signal) = BootSignal::try_from(request.signal) {
            return signal.as_str_name().to_string();
        }
    }

    // Try JSON decode
    if let Ok(json_str) = std::str::from_utf8(payload) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            // Try {"signal": "NAME"}
            if let Some(signal) = json.get("signal").and_then(|v| v.as_str()) {
                return signal.to_string();
            }
            // Try {"signal": int}
            if let Some(signal_int) = json.get("signal").and_then(|v| v.as_i64()) {
                if let Ok(signal) = BootSignal::try_from(signal_int as i32) {
                    return signal.as_str_name().to_string();
                }
            }
        }
    }

    "UNKNOWN".to_string()
}

/// Normalize subsystem name (replace underscores with dashes)
pub fn normalize_subsystem_name(name: &str) -> String {
    name.replace('_', "-")
}

/// Derive event category from topic for UI filtering
fn derive_event_category(topic: i32) -> &'static str {
    match topic {
        1 | 2 => "boot",                  // Boot signals
        10..=13 => "error",               // Subsystem lifecycle errors
        40..=42 => "memory",              // Memory events
        62..=64 => "ctp",                 // CTP events
        60 | 61 => "user",                // User messages
        _ => "default",
    }
}

/// Run the event stream, processing events and emitting Tauri events
pub async fn run_event_stream(
    app_handle: tauri::AppHandle,
    address: String,
    connection_timeout_ms: u64,
    reconnect_config: ReconnectConfig,
    debug_state: Arc<Mutex<DebugState>>,
) {
    let mut attempt = 0u32;

    loop {
        match connect_and_stream(
            &app_handle,
            &address,
            connection_timeout_ms,
            &debug_state,
        )
        .await
        {
            Ok(_) => {
                info!("Event stream ended gracefully");
                attempt = 0; // Reset on clean disconnect
            }
            Err(e) => {
                error!(error = %e, "Event stream error");

                // Update connection state
                if let Ok(mut state) = debug_state.lock() {
                    state.connected = false;
                }
            }
        }

        // Calculate reconnection delay with exponential backoff
        let delay_ms = match attempt {
            0 => reconnect_config.initial_delay_ms,
            1 => reconnect_config.second_delay_ms,
            2 => reconnect_config.third_delay_ms,
            _ => reconnect_config.steady_state_delay_ms,
        };

        if delay_ms > 0 {
            info!(delay_ms, attempt, "Reconnecting to event bus");
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        }

        attempt = attempt.saturating_add(1);
    }
}

async fn connect_and_stream(
    app_handle: &tauri::AppHandle,
    address: &str,
    connection_timeout_ms: u64,
    debug_state: &Arc<Mutex<DebugState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(%address, "Connecting to event bus");

    let channel = Channel::from_shared(address.to_string())?
        .connect_timeout(std::time::Duration::from_millis(connection_timeout_ms))
        .connect()
        .await?;

    let mut client = EventBusServiceClient::new(channel);

    let request = tonic::Request::new(SubscribeRequest {
        topics: vec![], // Empty means all topics
        subscriber_id: "ui-debug-panel".to_string(),
    });

    let mut stream = client.subscribe(request).await?.into_inner();

    // Mark as connected
    if let Ok(mut state) = debug_state.lock() {
        state.connected = true;
        if state.started_at.is_none() {
            state.started_at = Some(Utc::now());
        }
    }

    info!("Event stream connected");

    // Process events
    while let Some(event) = stream.message().await? {
        handle_bus_event(app_handle, debug_state, event).await;
    }

    Ok(())
}

async fn handle_bus_event(
    app_handle: &tauri::AppHandle,
    debug_state: &Arc<Mutex<DebugState>>,
    event: crate::generated::sena_daemonbus_v1::BusEvent,
) {
    let timestamp = event
        .timestamp
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());

    // Parse topic name
    let topic_str = crate::state::format_topic_name(event.topic);

    // Truncate payload for display
    let payload_summary = if let Ok(s) = std::str::from_utf8(&event.payload) {
        crate::state::truncate_with_ellipsis(s, 120)
    } else {
        format!("<binary {} bytes>", event.payload.len())
    };

    // Push to event feed
    if let Ok(mut state) = debug_state.lock() {
        state.total_events_received += 1;
        state.push_event(BusEventEntry {
            topic: topic_str.to_string(),
            source_subsystem: event.source_subsystem.clone(),
            payload_summary: payload_summary.clone(),
            timestamp,
        });
    }

    // Emit bus-event to frontend
    let category = derive_event_category(event.topic);
    let _ = app_handle.emit(
        "bus-event",
        BusEventPayload {
            topic: topic_str.to_string(),
            source: event.source_subsystem.clone(),
            payload: payload_summary.clone(),
            category: category.to_string(),
            timestamp: timestamp.to_rfc3339(),
        },
    );

    // Handle specific topic types
    match EventTopic::try_from(event.topic) {
        Ok(EventTopic::TopicBootSignal) => {
            handle_boot_signal(app_handle, debug_state, &event.payload, &event.source_subsystem)
                .await;
        }
        Ok(EventTopic::TopicSubsystemDegraded) => {
            let subsystem = normalize_subsystem_name(&event.source_subsystem);
            update_subsystem_status(
                app_handle,
                debug_state,
                &subsystem,
                SubsystemHealthStatus::Degraded,
            )
            .await;
        }
        Ok(EventTopic::TopicSubsystemCrashed) => {
            // Design choice: crashed subsystems are marked Degraded, not Unavailable
            let subsystem = normalize_subsystem_name(&event.source_subsystem);
            update_subsystem_status(
                app_handle,
                debug_state,
                &subsystem,
                SubsystemHealthStatus::Degraded,
            )
            .await;
        }
        Ok(EventTopic::TopicSubsystemStarted) => {
            let subsystem = normalize_subsystem_name(&event.source_subsystem);
            update_subsystem_status(
                app_handle,
                debug_state,
                &subsystem,
                SubsystemHealthStatus::Ready,
            )
            .await;
        }
        Ok(EventTopic::TopicThoughtSurfaced) => {
            handle_thought_surfaced(debug_state, &event.payload).await;
        }
        Ok(EventTopic::TopicMemoryWriteCompleted) | Ok(EventTopic::TopicMemoryUpdated) => {
            handle_memory_event(debug_state, &event.payload).await;
        }
        _ => {}
    }
}

async fn handle_boot_signal(
    app_handle: &tauri::AppHandle,
    debug_state: &Arc<Mutex<DebugState>>,
    payload: &[u8],
    source_subsystem: &str,
) {
    let signal_name = extract_boot_signal_name(payload);
    let subsystem = signal_name_to_subsystem(&signal_name)
        .map(|s| s.to_string())
        .unwrap_or_else(|| normalize_subsystem_name(source_subsystem));

    let required = crate::state::REQUIRED_BOOT_SIGNALS.contains(&signal_name.as_str());

    // Update state
    if let Ok(mut state) = debug_state.lock() {
        state.push_boot_signal(BootSignalEntry {
            signal_name: signal_name.clone(),
            source_subsystem: subsystem.clone(),
            required,
            timestamp: Utc::now(),
        });

        // Update subsystem health based on signal type
        match signal_name.as_str() {
            "INFERENCE_UNAVAILABLE" => {
                state.set_subsystem_status("inference", SubsystemHealthStatus::Unavailable);
            }
            "INFERENCE_DEGRADED" => {
                state.set_subsystem_status("inference", SubsystemHealthStatus::Degraded);
            }
            _ => {
                state.set_subsystem_status(&subsystem, SubsystemHealthStatus::Ready);
            }
        }
    }

    // Emit Tauri event
    let _ = app_handle.emit(
        "boot-signal-received",
        BootSignalPayload {
            signal: signal_name,
            required,
            timestamp: Utc::now().to_rfc3339(),
            subsystem,
        },
    );
}

async fn update_subsystem_status(
    app_handle: &tauri::AppHandle,
    debug_state: &Arc<Mutex<DebugState>>,
    subsystem: &str,
    status: SubsystemHealthStatus,
) {
    // Update state
    if let Ok(mut state) = debug_state.lock() {
        state.set_subsystem_status(subsystem, status);
    }

    // Emit Tauri event
    let status_str = match status {
        SubsystemHealthStatus::Ready => "ready",
        SubsystemHealthStatus::Degraded => "degraded",
        SubsystemHealthStatus::Unavailable => "unavailable",
        SubsystemHealthStatus::Unknown => "unknown",
    };

    let _ = app_handle.emit(
        "subsystem-status-updated",
        SubsystemStatusPayload {
            subsystem: subsystem.to_string(),
            status: status_str.to_string(),
        },
    );
}

async fn handle_thought_surfaced(debug_state: &Arc<Mutex<DebugState>>, payload: &[u8]) {
    if let Ok(json_str) = std::str::from_utf8(payload) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            let content = json
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let relevance_score = json
                .get("relevance_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32;

            if let Ok(mut state) = debug_state.lock() {
                state.push_thought(crate::state::ThoughtEvent {
                    content,
                    relevance_score,
                    timestamp: Utc::now(),
                });
            }
        }
    }
}

async fn handle_memory_event(debug_state: &Arc<Mutex<DebugState>>, payload: &[u8]) {
    if let Ok(json_str) = std::str::from_utf8(payload) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(tier) = json.get("tier").and_then(|v| v.as_str()) {
                let count = json.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                if let Ok(mut state) = debug_state.lock() {
                    state.update_memory_stats(tier, count);
                }
            }
        }
    }
}

/// Send a chat message to the reactive loop via daemon-bus
pub async fn send_chat_message(
    address: &str,
    message: String,
    connection_timeout_ms: u64,
) -> Result<UserMessageResponse, tonic::Status> {
    let channel = Channel::from_shared(address.to_string())
        .map_err(|e| tonic::Status::invalid_argument(format!("Invalid address: {}", e)))?
        .connect_timeout(std::time::Duration::from_millis(connection_timeout_ms))
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(format!("Failed to connect: {}", e)))?;

    let mut client = UserMessageServiceClient::new(channel);

    let request = tonic::Request::new(UserMessageRequest {
        message,
        trace_context: String::new(), // Will be populated by daemon-bus
    });

    let response: tonic::Response<UserMessageResponse> = client.send_message(request).await?;

    Ok(response.into_inner())
}

/// Signal UI_READY to daemon-bus after all windows are initialized.
pub async fn signal_ui_ready(
    address: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::generated::sena_daemonbus_v1::boot_service_client::BootServiceClient;

    let channel = Channel::from_shared(address.to_string())?
        .connect()
        .await?;

    let mut client = BootServiceClient::new(channel);

    let request = tonic::Request::new(BootSignalRequest {
        subsystem_id: "ui".to_string(),
        signal: BootSignal::UiReady as i32,
    });

    client.signal_ready(request).await?;

    info!(
        component = "grpc",
        event_type = "ui_ready_signaled",
        "UI_READY signal sent to daemon-bus"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_name_to_subsystem() {
        assert_eq!(
            signal_name_to_subsystem("DAEMON_BUS_READY"),
            Some("daemon-bus")
        );
        assert_eq!(
            signal_name_to_subsystem("MEMORY_ENGINE_READY"),
            Some("memory-engine")
        );
        assert_eq!(
            signal_name_to_subsystem("INFERENCE_READY"),
            Some("inference")
        );
        assert_eq!(
            signal_name_to_subsystem("INFERENCE_UNAVAILABLE"),
            Some("inference")
        );
        assert_eq!(
            signal_name_to_subsystem("INFERENCE_DEGRADED"),
            Some("inference")
        );
        assert_eq!(signal_name_to_subsystem("UNKNOWN_SIGNAL"), None);
    }

    #[test]
    fn test_normalize_subsystem_name() {
        assert_eq!(normalize_subsystem_name("memory_engine"), "memory-engine");
        assert_eq!(normalize_subsystem_name("daemon_bus"), "daemon-bus");
        assert_eq!(normalize_subsystem_name("inference"), "inference");
    }

    #[test]
    fn test_extract_boot_signal_name_json() {
        // Test JSON with string signal
        let json = r#"{"signal": "DAEMON_BUS_READY"}"#;
        let signal = extract_boot_signal_name(json.as_bytes());
        assert_eq!(signal, "DAEMON_BUS_READY");

        // Test JSON with int signal
        let json = r#"{"signal": 1}"#;
        let signal = extract_boot_signal_name(json.as_bytes());
        assert_eq!(signal, "DAEMON_BUS_READY");

        // Test invalid JSON
        let signal = extract_boot_signal_name(b"invalid");
        assert_eq!(signal, "UNKNOWN");
    }
}
