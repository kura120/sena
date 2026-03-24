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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilityBreakdownPayload>,
}

/// Capability item payload for frontend
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityItemPayload {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Capability breakdown payload for frontend
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityBreakdownPayload {
    pub granted: Vec<CapabilityItemPayload>,
    pub degraded: Vec<CapabilityItemPayload>,
    pub denied: Vec<CapabilityItemPayload>,
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

/// Extract capability breakdown from MODEL_PROFILE_READY boot signal payload
pub fn extract_capabilities_from_payload(
    payload: &[u8],
    signal_name: &str,
) -> Option<crate::state::CapabilityBreakdownEntry> {
    // Only attempt to extract capabilities for MODEL_PROFILE_READY signals
    if signal_name != "MODEL_PROFILE_READY" {
        return None;
    }

    // Try to parse as JSON
    let json_str = std::str::from_utf8(payload).ok()?;
    let json_value: serde_json::Value = serde_json::from_str(json_str).ok()?;

    // Extract capabilities object
    let capabilities_obj = json_value.get("capabilities")?.as_object()?;

    // Helper to parse capability array
    let parse_capability_array = |arr_value: Option<&serde_json::Value>| -> Vec<crate::state::CapabilityItemEntry> {
        arr_value
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let obj = item.as_object()?;
                        let label = obj.get("label")?.as_str()?.to_string();
                        let reason = obj.get("reason").and_then(|r| r.as_str()).map(|s| s.to_string());
                        Some(crate::state::CapabilityItemEntry { label, reason })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let granted = parse_capability_array(capabilities_obj.get("granted"));
    let degraded = parse_capability_array(capabilities_obj.get("degraded"));
    let denied = parse_capability_array(capabilities_obj.get("denied"));

    Some(crate::state::CapabilityBreakdownEntry {
        granted,
        degraded,
        denied,
    })
}

/// Normalize subsystem name (replace underscores with dashes)
pub fn normalize_subsystem_name(name: &str) -> String {
    name.replace('_', "-")
}

/// Derive event category from topic for UI filtering
fn derive_event_category(topic: i32) -> &'static str {
    match topic {
        1 | 2 => "boot",     // Boot signals
        10..=13 => "error",  // Subsystem lifecycle errors
        40..=42 => "memory", // Memory events
        62..=64 => "ctp",    // CTP events
        60 | 61 => "user",   // User messages
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
        match connect_and_stream(&app_handle, &address, connection_timeout_ms, &debug_state).await {
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

    // Mark UI as Ready — the UI is definitely running if we reached this point.
    // This fixes the race where UI_READY is signaled before the event stream connects.
    if let Ok(mut state) = debug_state.lock() {
        state.set_subsystem_status("ui", SubsystemHealthStatus::Ready);
    }
    let _ = app_handle.emit(
        "subsystem-status-updated",
        SubsystemStatusPayload {
            subsystem: "ui".to_string(),
            status: "Ready".to_string(),
        },
    );

    info!("Event stream connected");

    // After the stream connects, daemon-bus replays historical boot signals
    // as the first messages. Schedule a sync event after a short delay to give
    // frontend webviews a chance to receive the replayed state via
    // get_debug_snapshot. This covers the race where JS mounts before the
    // stream connects.
    let sync_handle = app_handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        let _ = sync_handle.emit("debug-state-sync", ());
    });

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
    tracing::info!(topic = %topic_str, source = %event.source_subsystem, "emitted bus-event to frontend");

    // Handle specific topic types
    match EventTopic::try_from(event.topic) {
        Ok(EventTopic::TopicBootSignal) => {
            handle_boot_signal(
                app_handle,
                debug_state,
                &event.payload,
                &event.source_subsystem,
            )
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
        Ok(EventTopic::TopicPcPromptAssembled) => {
            handle_prompt_assembled(debug_state, &event.payload).await;
        }
        Ok(EventTopic::TopicUserMessageReceived) => {
            handle_user_message_received(debug_state, &event.payload).await;
        }
        Ok(EventTopic::TopicUserMessageResponse) => {
            handle_user_message_response(debug_state, &event.payload).await;
        }
        Ok(EventTopic::TopicInferenceModelSwitching) => {
            handle_inference_model_switching(debug_state, &event.payload).await;
        }
        _ => {
            tracing::trace!(event_type = "unknown_topic", "unhandled event topic in UI bridge");
        }
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

    // Extract capabilities if present
    let capabilities = extract_capabilities_from_payload(payload, &signal_name);

    // Convert capabilities for payload emission
    let capabilities_payload = capabilities.as_ref().map(|caps| CapabilityBreakdownPayload {
        granted: caps
            .granted
            .iter()
            .map(|item| CapabilityItemPayload {
                label: item.label.clone(),
                reason: item.reason.clone(),
            })
            .collect(),
        degraded: caps
            .degraded
            .iter()
            .map(|item| CapabilityItemPayload {
                label: item.label.clone(),
                reason: item.reason.clone(),
            })
            .collect(),
        denied: caps
            .denied
            .iter()
            .map(|item| CapabilityItemPayload {
                label: item.label.clone(),
                reason: item.reason.clone(),
            })
            .collect(),
    });

    // Clone for later emit (first emit moves these values)
    let signal_name_clone = signal_name.clone();
    let subsystem_clone = subsystem.clone();

    // Update state
    if let Ok(mut state) = debug_state.lock() {
        // Clone capabilities before moving into boot_signal_history
        let capabilities_for_subsystem = capabilities.clone();
        
        state.push_boot_signal(BootSignalEntry {
            signal_name: signal_name.clone(),
            source_subsystem: subsystem.clone(),
            required,
            timestamp: Utc::now(),
            capabilities,
        });

        // Update subsystem health based on signal type, storing signal name and capabilities
        match signal_name.as_str() {
            "INFERENCE_UNAVAILABLE" => {
                state.set_subsystem_with_signal(
                    "inference",
                    SubsystemHealthStatus::Unavailable,
                    signal_name.clone(),
                    capabilities_for_subsystem,
                );
            }
            "INFERENCE_DEGRADED" => {
                state.set_subsystem_with_signal(
                    "inference",
                    SubsystemHealthStatus::Degraded,
                    signal_name.clone(),
                    capabilities_for_subsystem,
                );
            }
            _ => {
                state.set_subsystem_with_signal(
                    &subsystem,
                    SubsystemHealthStatus::Ready,
                    signal_name.clone(),
                    capabilities_for_subsystem,
                );
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
            capabilities: capabilities_payload,
        },
    );

    // Emit subsystem status update so the health panel reflects boot signals
    let status_str = match signal_name_clone.as_str() {
        "INFERENCE_UNAVAILABLE" => "Unavailable",
        "INFERENCE_DEGRADED" => "Degraded",
        _ => "Ready",
    };

    let _ = app_handle.emit(
        "subsystem-status-updated",
        SubsystemStatusPayload {
            subsystem: subsystem_clone.clone(),
            status: status_str.to_string(),
        },
    );
    tracing::info!(signal = %signal_name_clone, subsystem = %subsystem_clone, status = %status_str, "emitted subsystem-status-updated from boot signal");
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
        SubsystemHealthStatus::Ready => "Ready",
        SubsystemHealthStatus::Degraded => "Degraded",
        SubsystemHealthStatus::Unavailable => "Unavailable",
        SubsystemHealthStatus::Unknown => "Unknown",
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

async fn handle_prompt_assembled(
    debug_state: &Arc<Mutex<DebugState>>,
    payload: &[u8],
) {
    // Parse payload as JSON with best-effort extraction
    let (sections, toon_preview, token_count, token_budget) = if let Ok(json_str) = std::str::from_utf8(payload) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            let sections = json.get("sections")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let toon_preview = json.get("toon_output")
                .and_then(|v| v.as_str())
                .map(|s| crate::state::truncate_with_ellipsis(s, 200))
                .unwrap_or_default();
            let token_count = json.get("token_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let token_budget = json.get("token_budget")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            (sections, toon_preview, token_count, token_budget)
        } else {
            (vec![], String::new(), 0, 0)
        }
    } else {
        (vec![], String::new(), 0, 0)
    };

    if let Ok(mut state) = debug_state.lock() {
        state.push_prompt_trace(crate::state::PromptTraceEntry {
            sections,
            toon_output_preview: toon_preview,
            token_count,
            token_budget,
            timestamp: chrono::Utc::now(),
        });
    }
}

async fn handle_user_message_received(
    debug_state: &Arc<Mutex<DebugState>>,
    payload: &[u8],
) {
    let content_preview = if let Ok(json_str) = std::str::from_utf8(payload) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            json.get("content")
                .and_then(|v| v.as_str())
                .map(|s| crate::state::truncate_with_ellipsis(s, 100))
                .unwrap_or_default()
        } else {
            crate::state::truncate_with_ellipsis(json_str, 100)
        }
    } else {
        String::new()
    };

    if let Ok(mut state) = debug_state.lock() {
        state.push_conversation_turn(crate::state::ConversationTurn {
            role: "user".to_string(),
            content_preview,
            model_id: String::new(),
            latency_ms: 0,
            tokens_prompt: 0,
            tokens_generated: 0,
            timestamp: chrono::Utc::now(),
        });
    }
}

async fn handle_user_message_response(
    debug_state: &Arc<Mutex<DebugState>>,
    payload: &[u8],
) {
    let (content_preview, model_id, latency_ms, tokens_prompt, tokens_generated) =
        if let Ok(json_str) = std::str::from_utf8(payload) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                let content = json.get("response")
                    .and_then(|v| v.as_str())
                    .map(|s| crate::state::truncate_with_ellipsis(s, 100))
                    .unwrap_or_default();
                let model = json.get("model_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let latency = json.get("latency_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let tp = json.get("tokens_prompt")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let tg = json.get("tokens_generated")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                (content, model, latency, tp, tg)
            } else {
                (String::new(), String::new(), 0, 0, 0)
            }
        } else {
            (String::new(), String::new(), 0, 0, 0)
        };

    if let Ok(mut state) = debug_state.lock() {
        // Update inference stats from response metadata
        if tokens_generated > 0 && latency_ms > 0 {
            let tps = (tokens_generated as f32 / latency_ms as f32) * 1000.0;
            state.inference_stats.tokens_per_second = tps;
            state.inference_stats.total_completions += 1;
            state.inference_stats.last_completion = Some(chrono::Utc::now());
            if !model_id.is_empty() {
                state.inference_stats.active_model = model_id.clone();
            }
        }

        state.push_conversation_turn(crate::state::ConversationTurn {
            role: "assistant".to_string(),
            content_preview,
            model_id,
            latency_ms,
            tokens_prompt,
            tokens_generated,
            timestamp: chrono::Utc::now(),
        });
    }
}

async fn handle_inference_model_switching(
    debug_state: &Arc<Mutex<DebugState>>,
    payload: &[u8],
) {
    if let Ok(json_str) = std::str::from_utf8(payload) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            let model_id = json.get("model_id")
                .and_then(|v| v.as_str())
                .unwrap_or("switching...")
                .to_string();
            let display_name = json.get("model_display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let vram_used = json.get("vram_used_mb")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let vram_total = json.get("vram_total_mb")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;

            if let Ok(mut state) = debug_state.lock() {
                state.inference_stats.active_model = model_id;
                if !display_name.is_empty() {
                    state.inference_stats.model_display_name = display_name.to_string();
                }
                if vram_used > 0 {
                    state.vram_used_mb = vram_used;
                }
                if vram_total > 0 {
                    state.vram_total_mb = vram_total;
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

    let channel = Channel::from_shared(address.to_string())?.connect().await?;

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

    #[test]
    fn test_extract_capabilities_valid_json() {
        let json = r#"{
            "signal": "MODEL_PROFILE_READY",
            "capabilities": {
                "granted": [
                    {"label": "Text Generation"},
                    {"label": "Context Window (8K)"}
                ],
                "degraded": [
                    {"label": "Chain-of-Thought", "reason": "Model does not support think tags"}
                ],
                "denied": [
                    {"label": "Vision", "reason": "No vision adapter loaded"}
                ]
            }
        }"#;

        let result = extract_capabilities_from_payload(json.as_bytes(), "MODEL_PROFILE_READY");
        assert!(result.is_some());

        let caps = result.unwrap();
        assert_eq!(caps.granted.len(), 2);
        assert_eq!(caps.granted[0].label, "Text Generation");
        assert_eq!(caps.granted[0].reason, None);
        assert_eq!(caps.granted[1].label, "Context Window (8K)");

        assert_eq!(caps.degraded.len(), 1);
        assert_eq!(caps.degraded[0].label, "Chain-of-Thought");
        assert_eq!(
            caps.degraded[0].reason,
            Some("Model does not support think tags".to_string())
        );

        assert_eq!(caps.denied.len(), 1);
        assert_eq!(caps.denied[0].label, "Vision");
        assert_eq!(
            caps.denied[0].reason,
            Some("No vision adapter loaded".to_string())
        );
    }

    #[test]
    fn test_extract_capabilities_no_capabilities_field() {
        let json = r#"{"signal": "MODEL_PROFILE_READY"}"#;
        let result = extract_capabilities_from_payload(json.as_bytes(), "MODEL_PROFILE_READY");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_capabilities_non_json() {
        // Test with binary/protobuf-like payload
        let binary_payload = b"\x08\x01\x12\x04test";
        let result = extract_capabilities_from_payload(binary_payload, "MODEL_PROFILE_READY");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_capabilities_wrong_signal() {
        let json = r#"{
            "signal": "INFERENCE_READY",
            "capabilities": {
                "granted": [{"label": "Test"}]
            }
        }"#;

        // Should return None for non-MODEL_PROFILE_READY signals
        let result = extract_capabilities_from_payload(json.as_bytes(), "INFERENCE_READY");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_capabilities_empty_arrays() {
        let json = r#"{
            "signal": "MODEL_PROFILE_READY",
            "capabilities": {
                "granted": [],
                "degraded": [],
                "denied": []
            }
        }"#;

        let result = extract_capabilities_from_payload(json.as_bytes(), "MODEL_PROFILE_READY");
        assert!(result.is_some());

        let caps = result.unwrap();
        assert_eq!(caps.granted.len(), 0);
        assert_eq!(caps.degraded.len(), 0);
        assert_eq!(caps.denied.len(), 0);
    }
}
