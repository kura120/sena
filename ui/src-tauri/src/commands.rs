use crate::config::Config;
use crate::grpc;
use crate::state::DebugState;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{error, info};

/// Shared application state accessible to all Tauri commands
pub struct AppState {
    pub debug_state: Arc<Mutex<DebugState>>,
    pub config: Config,
    pub stream_started: AtomicBool,
}

/// Response for send_message command
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageResponse {
    pub response: String,
    pub model_id: String,
    pub latency_ms: u64,
}

/// Panel configuration for overlay
#[derive(Debug, Clone, Serialize)]
pub struct PanelConfig {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Response for get_overlay_config command
#[derive(Debug, Clone, Serialize)]
pub struct OverlayConfigResponse {
    pub toggle_key: String,
    pub panels: Vec<PanelConfig>,
}

/// Start the event stream in the background
#[tauri::command]
pub async fn start_event_stream(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Only start once
    if state
        .stream_started
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Event stream already started".to_string());
    }

    info!("Starting event stream");

    let address = state.config.grpc.daemon_bus_address.clone();
    let connection_timeout_ms = state.config.grpc.connection_timeout_ms;
    let reconnect_config = state.config.reconnect.clone();
    let debug_state = Arc::clone(&state.debug_state);

    // Spawn the event stream as a background task
    tokio::spawn(async move {
        grpc::run_event_stream(
            app_handle,
            address,
            connection_timeout_ms,
            reconnect_config,
            debug_state,
        )
        .await;
    });

    Ok(())
}

/// Send a chat message to the reactive loop via daemon-bus
#[tauri::command]
pub async fn send_message(
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<SendMessageResponse, String> {
    info!("Sending chat message");

    let address = state.config.grpc.daemon_bus_address.clone();
    let connection_timeout_ms = state.config.grpc.connection_timeout_ms;

    let response = grpc::send_chat_message(&address, content, connection_timeout_ms)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to send message");
            format!("Failed to send message: {}", e)
        })?;

    Ok(SendMessageResponse {
        response: response.response,
        model_id: response.model_id,
        latency_ms: response.latency_ms,
    })
}

/// Get overlay configuration
#[tauri::command]
pub async fn get_overlay_config(
    state: tauri::State<'_, AppState>,
) -> Result<OverlayConfigResponse, String> {
    let config = &state.config.overlay;

    let panels = vec![
        PanelConfig {
            name: "health".to_string(),
            x: config.health_window.x,
            y: config.health_window.y,
            width: config.health_window.width,
            height: config.health_window.height,
        },
        PanelConfig {
            name: "event_bus".to_string(),
            x: config.event_bus_window.x,
            y: config.event_bus_window.y,
            width: config.event_bus_window.width,
            height: config.event_bus_window.height,
        },
        PanelConfig {
            name: "chat".to_string(),
            x: config.chat_window.x,
            y: config.chat_window.y,
            width: config.chat_window.width,
            height: config.chat_window.height,
        },
        PanelConfig {
            name: "boot_timeline".to_string(),
            x: config.boot_timeline_window.x,
            y: config.boot_timeline_window.y,
            width: config.boot_timeline_window.width,
            height: config.boot_timeline_window.height,
        },
    ];

    Ok(OverlayConfigResponse {
        toggle_key: config.toggle_key.clone(),
        panels,
    })
}

/// Toggle overlay visibility via command (for frontend invocation)
#[tauri::command]
pub async fn toggle_overlay_cmd(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::overlay::toggle_overlay(&app_handle)
}

/// Save window position to persistent store
#[tauri::command]
pub async fn save_window_position(
    label: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    info!(
        label = %label,
        x, y, width, height,
        "Saving window position"
    );

    // Get or create the store
    let store = app_handle
        .store("window-positions.json")
        .map_err(|e| format!("Failed to access store: {}", e))?;

    // Create the position object
    let position = serde_json::json!({
        "x": x,
        "y": y,
        "width": width,
        "height": height,
    });

    // Save to store
    store.set(label.clone(), position);

    store
        .save()
        .map_err(|e| format!("Failed to persist store: {}", e))?;

    info!(label = %label, "Window position saved");
    Ok(())
}
