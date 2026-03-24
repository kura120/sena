use crate::config::Config;
use crate::grpc;
use crate::state::DebugState;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
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
    pub pre_thought_text: Option<String>,
    pub thought_content: Option<String>,
    pub chain_of_thought_supported: bool,
}

/// Snapshot of current debug state for frontend initialization
#[derive(Debug, Clone, Serialize)]
pub struct DebugSnapshot {
    pub subsystems: Vec<SubsystemEntry>,
    pub events: Vec<BusEventSnapshot>,
    pub connected: bool,
    pub vram: VramSnapshot,
    pub thoughts: Vec<ThoughtSnapshot>,
    pub memory_stats: MemoryStatsSnapshot,
    pub inference_stats: InferenceStatsSnapshot,
    pub prompt_traces: Vec<PromptTraceSnapshot>,
    pub conversation_turns: Vec<ConversationTurnSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubsystemEntry {
    pub name: String,
    pub status: String,
    pub timestamp: Option<String>,
    pub boot_signal_name: Option<String>,
    pub capabilities: Option<CapabilityBreakdownSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityBreakdownSnapshot {
    pub granted: Vec<CapabilityItemSnapshot>,
    pub degraded: Vec<CapabilityItemSnapshot>,
    pub denied: Vec<CapabilityItemSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityItemSnapshot {
    pub label: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BusEventSnapshot {
    pub topic: String,
    pub source: String,
    pub payload: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThoughtSnapshot {
    pub content: String,
    pub relevance_score: f32,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStatsSnapshot {
    pub short_term_count: u32,
    pub long_term_count: u32,
    pub episodic_count: u32,
    pub last_write: Option<String>,
    pub short_term_last_write: Option<String>,
    pub long_term_last_write: Option<String>,
    pub episodic_last_write: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InferenceStatsSnapshot {
    pub active_model: String,
    pub model_display_name: String,
    pub tokens_per_second: f32,
    pub total_completions: u64,
    pub last_completion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VramSnapshot {
    pub used_mb: u32,
    pub total_mb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptTraceSnapshot {
    pub sections: Vec<String>,
    pub toon_output_preview: String,
    pub token_count: u32,
    pub token_budget: u32,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationTurnSnapshot {
    pub role: String,
    pub content_preview: String,
    pub model_id: String,
    pub latency_ms: u64,
    pub tokens_prompt: u32,
    pub tokens_generated: u32,
    pub timestamp: String,
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

    let address = state.config.grpc.reactive_loop_address.clone();
    let connection_timeout_ms = state.config.grpc.connection_timeout_ms;

    let response = grpc::send_chat_message(&address, content, connection_timeout_ms)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to send message");
            if e.code() == tonic::Code::Unavailable {
                "reactive-loop is not reachable. Make sure daemon-bus has started it.".to_string()
            } else {
                format!("Failed to send message: {}", e)
            }
        })?;

    // Update inference stats from the direct gRPC response
    if let Ok(mut guard) = state.debug_state.lock() {
        if !response.model_id.is_empty() {
            guard.inference_stats.active_model = response.model_id.clone();
        }
        if response.latency_ms > 0 && response.tokens_generated > 0 {
            // Calculate tokens/s from response metadata
            guard.inference_stats.tokens_per_second =
                (response.tokens_generated as f32 / response.latency_ms as f32) * 1000.0;
        }
        guard.inference_stats.total_completions += 1;
        guard.inference_stats.last_completion = Some(chrono::Utc::now());
    }

    Ok(SendMessageResponse {
        response: response.response,
        model_id: response.model_id,
        latency_ms: response.latency_ms,
        pre_thought_text: response.pre_thought_text,
        thought_content: response.thought_content,
        chain_of_thought_supported: response.chain_of_thought_supported,
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

/// Return current debug state so frontends can hydrate on mount.
/// Solves the race where gRPC events arrive before JS listeners are ready.
#[tauri::command]
pub async fn get_debug_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<DebugSnapshot, String> {
    let guard = state.debug_state.lock().map_err(|e| format!("Lock poisoned: {}", e))?;

    let subsystems = guard
        .subsystem_health
        .iter()
        .map(|(name, entry)| {
            let status_str = match entry.status {
                crate::state::SubsystemHealthStatus::Ready => "Ready",
                crate::state::SubsystemHealthStatus::Degraded => "Degraded",
                crate::state::SubsystemHealthStatus::Unavailable => "Unavailable",
                crate::state::SubsystemHealthStatus::Unknown => "Unknown",
            };

            let capabilities = entry.capabilities.as_ref().map(|caps| {
                CapabilityBreakdownSnapshot {
                    granted: caps.granted.iter()
                        .map(|item| CapabilityItemSnapshot {
                            label: item.label.clone(),
                            reason: item.reason.clone(),
                        })
                        .collect(),
                    degraded: caps.degraded.iter()
                        .map(|item| CapabilityItemSnapshot {
                            label: item.label.clone(),
                            reason: item.reason.clone(),
                        })
                        .collect(),
                    denied: caps.denied.iter()
                        .map(|item| CapabilityItemSnapshot {
                            label: item.label.clone(),
                            reason: item.reason.clone(),
                        })
                        .collect(),
                }
            });

            SubsystemEntry {
                name: name.clone(),
                status: status_str.to_string(),
                timestamp: entry.last_change.map(|t| t.to_rfc3339()),
                boot_signal_name: entry.boot_signal_name.clone(),
                capabilities,
            }
        })
        .collect();

    let events = guard
        .event_feed
        .iter()
        .map(|e| BusEventSnapshot {
            topic: e.topic.clone(),
            source: e.source_subsystem.clone(),
            payload: e.payload_summary.clone(),
            timestamp: e.timestamp.to_rfc3339(),
        })
        .collect();

    let vram = VramSnapshot {
        used_mb: guard.vram_used_mb,
        total_mb: guard.vram_total_mb,
    };

    let thoughts: Vec<ThoughtSnapshot> = guard
        .thought_feed
        .iter()
        .map(|t| ThoughtSnapshot {
            content: t.content.clone(),
            relevance_score: t.relevance_score,
            timestamp: t.timestamp.to_rfc3339(),
        })
        .collect();

    let memory_stats = MemoryStatsSnapshot {
        short_term_count: guard.memory_stats.short_term_count,
        long_term_count: guard.memory_stats.long_term_count,
        episodic_count: guard.memory_stats.episodic_count,
        last_write: guard.memory_stats.last_write.map(|t| t.to_rfc3339()),
        short_term_last_write: guard.memory_stats.short_term_last_write.map(|t| t.to_rfc3339()),
        long_term_last_write: guard.memory_stats.long_term_last_write.map(|t| t.to_rfc3339()),
        episodic_last_write: guard.memory_stats.episodic_last_write.map(|t| t.to_rfc3339()),
    };

    let inference_stats = InferenceStatsSnapshot {
        active_model: guard.inference_stats.active_model.clone(),
        model_display_name: guard.inference_stats.model_display_name.clone(),
        tokens_per_second: guard.inference_stats.tokens_per_second,
        total_completions: guard.inference_stats.total_completions,
        last_completion: guard.inference_stats.last_completion.map(|t| t.to_rfc3339()),
    };

    let prompt_traces: Vec<PromptTraceSnapshot> = guard
        .prompt_traces
        .iter()
        .map(|p| PromptTraceSnapshot {
            sections: p.sections.clone(),
            toon_output_preview: p.toon_output_preview.clone(),
            token_count: p.token_count,
            token_budget: p.token_budget,
            timestamp: p.timestamp.to_rfc3339(),
        })
        .collect();

    let conversation_turns: Vec<ConversationTurnSnapshot> = guard
        .conversation_turns
        .iter()
        .map(|c| ConversationTurnSnapshot {
            role: c.role.clone(),
            content_preview: c.content_preview.clone(),
            model_id: c.model_id.clone(),
            latency_ms: c.latency_ms,
            tokens_prompt: c.tokens_prompt,
            tokens_generated: c.tokens_generated,
            timestamp: c.timestamp.to_rfc3339(),
        })
        .collect();

    Ok(DebugSnapshot {
        subsystems,
        events,
        connected: guard.connected,
        vram,
        thoughts,
        memory_stats,
        inference_stats,
        prompt_traces,
        conversation_turns,
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

/// Reboot daemon-bus: kill the existing process by port, wait, and respawn.
/// Emits "subsystems-reset" event so all UI panels clear their state.
#[tauri::command]
pub async fn reboot_daemon_bus(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Reset debug state before delegating to daemon_launcher
    if let Ok(mut debug_state) = state.debug_state.lock() {
        for entry in debug_state.subsystem_health.values_mut() {
            entry.status = crate::state::SubsystemHealthStatus::Unknown;
            entry.last_change = None;
            entry.boot_signal_name = None;
            entry.capabilities = None;
        }
        debug_state.boot_signal_history.clear();
    }

    let config = state.config.daemon_bus.clone();
    crate::daemon_launcher::reboot_daemon_bus(&app_handle, &config).await
}

/// Show the notification history window (lazy-create if needed).
#[tauri::command]
pub async fn show_notification_history(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::overlay::create_notification_history_window(&app_handle)
        .map_err(|e| format!("Failed to show notification history: {}", e))
}

/// Get all panel open/close states from the persistent store.
/// Returns a JSON object mapping panel labels to boolean states.
/// Panels not in the store default to true (open).
#[tauri::command]
pub async fn get_panel_states(
    app_handle: tauri::AppHandle,
) -> Result<std::collections::HashMap<String, bool>, String> {
    use tauri_plugin_store::StoreExt;

    let store = app_handle
        .store("panel-states.json")
        .map_err(|e| format!("Failed to access panel state store: {}", e))?;

    let mut panel_states = std::collections::HashMap::new();
    for &panel_label in crate::overlay::ALL_PANELS {
        let default_open = !matches!(panel_label, "model-panel" | "settings");
        let is_open = store
            .get(panel_label)
            .and_then(|val| val.as_bool())
            .unwrap_or(default_open);
        panel_states.insert(panel_label.to_string(), is_open);
    }

    Ok(panel_states)
}

/// Set a single panel's open/close state in persistent store.
#[tauri::command]
pub async fn set_panel_state(
    label: String,
    is_open: bool,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    info!(label = %label, is_open, "Setting panel state");

    let store = app_handle
        .store("panel-states.json")
        .map_err(|e| format!("Failed to access panel state store: {}", e))?;

    store.set(label.clone(), serde_json::json!(is_open));
    store
        .save()
        .map_err(|e| format!("Failed to persist panel state: {}", e))?;

    // Emit event so widget bar can update its active states (non-critical broadcast)
    let _ = app_handle.emit("panel-state-changed", serde_json::json!({
        "label": label,
        "is_open": is_open,
    }));

    Ok(())
}

/// Get an overlay setting value from persistent store.
/// Supported keys: "reopen_panels_on_toggle"
/// Returns the value or a default.
#[tauri::command]
pub async fn get_overlay_setting(
    key: String,
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_store::StoreExt;

    let store = app_handle
        .store("overlay-settings.json")
        .map_err(|e| format!("Failed to access overlay settings store: {}", e))?;

    let value = store.get(&key).unwrap_or({
        // Return defaults for known settings
        match key.as_str() {
            "reopen_panels_on_toggle" => serde_json::json!(true),
            _ => serde_json::json!(null),
        }
    });

    Ok(value)
}

/// Set an overlay setting in persistent store.
#[tauri::command]
pub async fn set_overlay_setting(
    key: String,
    value: serde_json::Value,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    info!(key = %key, "Setting overlay setting");

    let store = app_handle
        .store("overlay-settings.json")
        .map_err(|e| format!("Failed to access overlay settings store: {}", e))?;

    store.set(key, value);
    store
        .save()
        .map_err(|e| format!("Failed to persist overlay setting: {}", e))?;

    Ok(())
}

/// Show a single panel window and set its state to open.
#[tauri::command]
pub async fn show_panel(
    label: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    crate::overlay::show_single_panel(&app_handle, &label)
}

/// Hide a single panel window and set its state to closed.
#[tauri::command]
pub async fn hide_panel(
    label: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    crate::overlay::hide_single_panel(&app_handle, &label)
}

/// Show the settings panel (lazy-create if needed).
#[tauri::command]
pub async fn show_settings_panel(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::overlay::show_single_panel(&app_handle, crate::overlay::PANEL_SETTINGS)
}

/// Read a subsystem's TOML config file and return it as a JSON value.
/// The JSON structure mirrors the TOML structure (sections become objects).
#[tauri::command]
pub async fn read_subsystem_config(
    subsystem: String,
) -> Result<serde_json::Value, String> {
    let path = crate::config::resolve_subsystem_config_path(&subsystem)?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    
    // Parse TOML to serde_json::Value
    let toml_value: toml::Value = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse TOML: {}", e))?;
    
    // Convert toml::Value to serde_json::Value
    let json_value = serde_json::to_value(toml_value)
        .map_err(|e| format!("Failed to convert to JSON: {}", e))?;
    
    Ok(json_value)
}

/// Write values to a subsystem's TOML config file, preserving comments and formatting.
/// `values` is a flat JSON object where keys use dot notation: "section.key" = value
#[tauri::command]
pub async fn write_subsystem_config(
    subsystem: String,
    values: serde_json::Value,
) -> Result<(), String> {
    use toml_edit::DocumentMut;
    
    let path = crate::config::resolve_subsystem_config_path(&subsystem)?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    
    let mut doc: DocumentMut = content.parse::<DocumentMut>()
        .map_err(|e| format!("Failed to parse TOML document: {}", e))?;
    
    if let Some(obj) = values.as_object() {
        for (dotted_key, val) in obj {
            let parts: Vec<&str> = dotted_key.split('.').collect();
            
            match parts.len() {
                1 => {
                    // Top-level key
                    if let Some(toml_val) = json_to_toml_value(val) {
                        doc[parts[0]] = toml_val;
                    }
                }
                2 => {
                    // section.key
                    if let Some(toml_val) = json_to_toml_value(val) {
                        doc[parts[0]][parts[1]] = toml_val;
                    }
                }
                3 => {
                    // section.subsection.key (e.g., tier.short_term.max_entries)
                    if let Some(toml_val) = json_to_toml_value(val) {
                        doc[parts[0]][parts[1]][parts[2]] = toml_val;
                    }
                }
                _ => {
                    return Err(format!("Key path too deep: {}", dotted_key));
                }
            }
        }
    } else {
        return Err("values must be a JSON object".to_string());
    }
    
    std::fs::write(&path, doc.to_string())
        .map_err(|e| format!("Failed to write config: {}", e))?;
    
    info!(subsystem = %subsystem, "Config updated");
    Ok(())
}

/// Convert a JSON value to a toml_edit value item.
fn json_to_toml_value(val: &serde_json::Value) -> Option<toml_edit::Item> {
    use toml_edit::{Array, Item, value};
    
    match val {
        serde_json::Value::Bool(b) => Some(value(*b)),
        serde_json::Value::Number(n) => n.as_i64().map(&value).or_else(|| n.as_f64().map(value)),
        serde_json::Value::String(s) => Some(value(s.as_str())),
        serde_json::Value::Array(arr) => {
            let mut toml_arr = Array::new();
            for item in arr {
                match item {
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            toml_arr.push(i);
                        } else if let Some(f) = n.as_f64() {
                            toml_arr.push(f);
                        }
                    }
                    serde_json::Value::String(s) => {
                        toml_arr.push(s.as_str());
                    }
                    serde_json::Value::Bool(b) => {
                        toml_arr.push(*b);
                    }
                    _ => {}
                }
            }
            Some(Item::Value(toml_edit::Value::Array(toml_arr)))
        }
        _ => None,
    }
}

/// Validate config values for a subsystem before saving.
/// Returns a list of validation errors (empty = valid).
#[tauri::command]
pub async fn validate_subsystem_config(
    subsystem: String,
    values: serde_json::Value,
) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    
    if let Some(obj) = values.as_object() {
        match subsystem.as_str() {
            "daemon-bus" => {
                // channel_capacity must be power of two
                if let Some(cap) = obj.get("bus.channel_capacity").and_then(|v| v.as_u64()) {
                    if cap == 0 || (cap & (cap - 1)) != 0 {
                        errors.push(format!("channel_capacity must be a power of two (64, 128, 256...), got {}", cap));
                    }
                }
                // max_escalation >= default_escalation
                let max_esc = obj.get("arbitration.max_escalation_duration_ms").and_then(|v| v.as_u64());
                let def_esc = obj.get("arbitration.default_escalation_duration_ms").and_then(|v| v.as_u64());
                if let (Some(max), Some(def)) = (max_esc, def_esc) {
                    if max < def {
                        errors.push(format!("max_escalation_duration_ms ({}) must be ≥ default_escalation_duration_ms ({})", max, def));
                    }
                }
                // max_task_timeout >= default_task_timeout
                let max_task = obj.get("watchdog.max_task_timeout_ms").and_then(|v| v.as_u64());
                let def_task = obj.get("watchdog.default_task_timeout_ms").and_then(|v| v.as_u64());
                if let (Some(max), Some(def)) = (max_task, def_task) {
                    if max < def {
                        errors.push(format!("max_task_timeout_ms ({}) must be ≥ default_task_timeout_ms ({})", max, def));
                    }
                }
                // backoff_ms length must equal max_retries
                let max_retries = obj.get("supervisor.max_retries").and_then(|v| v.as_u64());
                let backoff = obj.get("supervisor.backoff_ms").and_then(|v| v.as_array());
                if let (Some(retries), Some(arr)) = (max_retries, backoff) {
                    if arr.len() as u64 != retries {
                        errors.push(format!("backoff_ms must have exactly {} entries, got {}", retries, arr.len()));
                    }
                }
            }
            "ctp" => {
                // idle_2min <= active threshold
                let active = obj.get("surface_thresholds.user_active").and_then(|v| v.as_f64());
                let idle_2 = obj.get("surface_thresholds.idle_2min").and_then(|v| v.as_f64());
                let idle_10 = obj.get("surface_thresholds.idle_10min").and_then(|v| v.as_f64());
                if let (Some(a), Some(i2)) = (active, idle_2) {
                    if i2 > a {
                        errors.push(format!("idle_2min_threshold ({}) must be ≤ active_threshold ({})", i2, a));
                    }
                }
                if let (Some(i2), Some(i10)) = (idle_2, idle_10) {
                    if i10 > i2 {
                        errors.push(format!("idle_10min_threshold ({}) must be ≤ idle_2min_threshold ({})", i10, i2));
                    }
                }
            }
            _ => {} // Other subsystems have no cross-field validation
        }
    }
    
    Ok(errors)
}

/// Restart a subsystem via daemon-bus supervisor.
/// Emits a toast notification and returns.
#[tauri::command]
pub async fn restart_subsystem(
    subsystem: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _address = state.config.grpc.daemon_bus_address.clone();
    
    // Emit toast
    crate::toast::emit_toast(
        &app_handle,
        "info",
        "Restarting",
        &format!("Restarting {}...", subsystem),
    );
    
    // Try to call daemon-bus supervisor restart via gRPC
    // For now, this is a best-effort call — daemon-bus supervisor 
    // may not have a restart RPC yet, so we log and proceed
    info!(subsystem = %subsystem, "Subsystem restart requested");
    
    // Mark subsystem as Unknown during restart
    if let Ok(mut debug_state) = state.debug_state.lock() {
        debug_state.set_subsystem_status(&subsystem, crate::state::SubsystemHealthStatus::Unknown);
    }
    
    let _ = app_handle.emit(
        "subsystem-status-updated",
        crate::grpc::SubsystemStatusPayload {
            subsystem: subsystem.clone(),
            status: "Unknown".to_string(),
        },
    );
    
    Ok(())
}

/// A local GGUF model metadata entry.
#[derive(Debug, Clone, Serialize)]
pub struct LocalModel {
    pub path: String,
    pub display_name: String,
    pub filename: String,
    pub size_gb: f64,
    pub architecture: String,
    pub quantization: String,
    pub is_active: bool,
}

/// An Ollama model discovered on the system.
#[derive(Debug, Clone, Serialize)]
pub struct OllamaModel {
    pub name: String,
    pub tag: String,
    pub size_gb: f64,
    pub architecture: String,
    pub blob_digest: String,
    pub is_extracted: bool,
    pub lora_compatible: bool,
    pub chain_of_thought_support: bool,
}

/// Normalize a model path to canonical relative form "models/<filename>".
fn normalize_model_path(path: &str) -> String {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(path);
    format!("models/{}", filename)
}

/// List all local GGUF models in inference/models/.
#[tauri::command]
pub async fn list_local_models(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<LocalModel>, String> {
    let workspace = crate::config::resolve_workspace_root()?;
    let models_dir = workspace.join("inference").join("models");
    
    if !models_dir.exists() {
        return Ok(Vec::new());
    }
    
    let active_model = state.debug_state.lock()
        .ok()
        .map(|s| normalize_model_path(&s.inference_stats.active_model))
        .unwrap_or_default();
    
    let models_dir_clone = models_dir.clone();
    let entries: Vec<_> = tokio::task::spawn_blocking(move || {
        let mut results = Vec::new();
        let dir = match std::fs::read_dir(&models_dir_clone) {
            Ok(d) => d,
            Err(e) => return Err(format!("Failed to read models directory: {}", e)),
        };
        for entry in dir {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable dir entry");
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("gguf") {
                continue;
            }
            let filename = path.file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("")
                .to_string();
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            // Parse GGUF metadata — log failures instead of silently dropping
            let metadata = match crate::gguf::parse_gguf_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(file = %filename, error = %e, "Failed to parse GGUF metadata");
                    crate::gguf::GgufMetadata::default()
                }
            };
            results.push((filename, size_bytes, metadata));
        }
        Ok(results)
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e: String| e)?;
    
    let mut models = Vec::new();
    for (filename, size_bytes, metadata) in entries {
        let size_gb = size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let display_name = metadata.name
            .unwrap_or_else(|| filename.trim_end_matches(".gguf").to_string());
        let architecture = metadata.architecture.unwrap_or_else(|| "unknown".to_string());
        let quantization = metadata.file_type
            .map(crate::gguf::file_type_to_quantization)
            .unwrap_or("Unknown")
            .to_string();
        let relative_path = format!("models/{}", filename);
        let is_active = active_model == relative_path;
        
        models.push(LocalModel {
            path: relative_path,
            display_name,
            filename,
            size_gb: (size_gb * 100.0).round() / 100.0,
            architecture,
            quantization,
            is_active,
        });
    }
    
    Ok(models)
}

/// List Ollama models by scanning the manifests directory.
#[tauri::command]
pub async fn list_ollama_models(manifests_dir: Option<String>) -> Result<Vec<OllamaModel>, String> {
    let base_manifests_dir = match manifests_dir {
        Some(ref dir) => std::path::PathBuf::from(dir),
        None => detect_ollama_directory(),
    };
    
    let manifests_dir = base_manifests_dir
        .join("registry.ollama.ai")
        .join("library");
    
    if !manifests_dir.exists() {
        return Ok(Vec::new());
    }
    
    // Check which models are already extracted locally
    let workspace = crate::config::resolve_workspace_root().ok();
    let local_models_dir = workspace.as_ref().map(|w| w.join("inference").join("models"));
    
    let lora_compatible_archs = ["llama", "mistral", "qwen", "gemma", "phi"];
    let cot_models = ["deepseek-r1", "qwen-thinking", "qwq"];
    
    let manifests_dir_clone = manifests_dir.clone();
    let local_models_dir_clone = local_models_dir.clone();
    
    let models = tokio::task::spawn_blocking(move || {
        let mut models = Vec::new();
        let model_families = std::fs::read_dir(&manifests_dir_clone)
            .map_err(|e| format!("Failed to read Ollama manifests: {}", e))?;
        
        for family_entry in model_families {
            let family_entry = match family_entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable Ollama family entry");
                    continue;
                }
            };
            if !family_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            
            let family_name = family_entry.file_name().to_string_lossy().to_string();
            
            let tags = match std::fs::read_dir(family_entry.path()) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(family = %family_name, error = %e, "Failed to read Ollama tags");
                    continue;
                }
            };
            
            for tag_entry in tags {
                let tag_entry = match tag_entry {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(error = %e, "Skipping unreadable tag entry");
                        continue;
                    }
                };
                let tag_name = tag_entry.file_name().to_string_lossy().to_string();
                
                // Read manifest JSON
                let manifest_content = match std::fs::read_to_string(tag_entry.path()) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(family = %family_name, tag = %tag_name, error = %e, "Failed to read manifest");
                        continue;
                    }
                };
                
                let manifest: serde_json::Value = match serde_json::from_str(&manifest_content) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(family = %family_name, tag = %tag_name, error = %e, "Failed to parse manifest JSON");
                        continue;
                    }
                };
                
                // Find the model layer (mediaType containing "model")
                let layers = manifest.get("layers").and_then(|l| l.as_array());
                let model_layer = layers.and_then(|layers| {
                    layers.iter().find(|l| {
                        l.get("mediaType")
                            .and_then(|m| m.as_str())
                            .map(|m| m.contains("model"))
                            .unwrap_or(false)
                    })
                });
                
                let (blob_digest, size_bytes) = match model_layer {
                    Some(layer) => {
                        let digest = layer.get("digest")
                            .and_then(|d| d.as_str())
                            .unwrap_or("")
                            .to_string();
                        let size = layer.get("size")
                            .and_then(|s| s.as_u64())
                            .unwrap_or(0);
                        (digest, size)
                    }
                    None => continue,
                };
                
                let size_gb = size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                let size_gb = (size_gb * 100.0).round() / 100.0;
                
                // Determine architecture from family name
                let architecture = family_name.clone();
                
                let lora_compatible = lora_compatible_archs.iter()
                    .any(|a| family_name.to_lowercase().contains(a));
                
                let chain_of_thought_support = cot_models.iter()
                    .any(|m| family_name.to_lowercase().contains(m));
                
                // Check if already extracted
                let is_extracted = local_models_dir_clone.as_ref()
                    .map(|dir| {
                        let expected_name = format!("{}-{}.gguf", family_name, tag_name);
                        dir.join(&expected_name).exists()
                    })
                    .unwrap_or(false);
                
                models.push(OllamaModel {
                    name: family_name.clone(),
                    tag: tag_name,
                    size_gb,
                    architecture,
                    blob_digest,
                    is_extracted,
                    lora_compatible,
                    chain_of_thought_support,
                });
            }
        }
        Ok::<_, String>(models)
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e: String| e)?;
    
    Ok(models)
}

/// Validate that a digest string is in the expected format (e.g. "sha256:<hex>").
fn validate_blob_digest(digest: &str) -> Result<(), String> {
    let parts: Vec<&str> = digest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err("Invalid digest format: expected 'algorithm:hex'".to_string());
    }
    if !parts[0].chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid digest algorithm".to_string());
    }
    if !parts[1].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid digest hash: must be hex characters only".to_string());
    }
    Ok(())
}

/// Sanitize a model name to a safe filename (alphanumeric, hyphens, underscores, dots).
fn sanitize_model_name(name: &str) -> Result<String, String> {
    let sanitized: String = name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err("Invalid model name".to_string());
    }
    // Ensure no path separators survived
    if sanitized.contains('/') || sanitized.contains('\\') {
        return Err("Model name must not contain path separators".to_string());
    }
    Ok(sanitized)
}

/// Extract an Ollama model blob to inference/models/.
#[tauri::command]
pub async fn extract_ollama_model(
    blob_digest: String,
    model_name: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    // Validate inputs to prevent path traversal
    validate_blob_digest(&blob_digest)?;
    let safe_model_name = sanitize_model_name(&model_name)?;
    
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Cannot determine user home directory".to_string())?;
    
    // Blob path: ~/.ollama/models/blobs/<digest>
    let digest_filename = blob_digest.replace(':', "-");
    let blob_path = std::path::PathBuf::from(&home)
        .join(".ollama")
        .join("models")
        .join("blobs")
        .join(&digest_filename);
    
    if !blob_path.exists() {
        return Err(format!("Blob not found: {}", blob_path.display()));
    }
    
    // Verify blob path is within the expected Ollama directory
    let expected_blobs_dir = std::path::PathBuf::from(&home)
        .join(".ollama")
        .join("models")
        .join("blobs");
    let canonical_blob = blob_path.canonicalize()
        .map_err(|e| format!("Failed to resolve blob path: {}", e))?;
    let canonical_blobs_dir = expected_blobs_dir.canonicalize()
        .map_err(|e| format!("Failed to resolve blobs directory: {}", e))?;
    if !canonical_blob.starts_with(&canonical_blobs_dir) {
        return Err("Blob path traversal not allowed".to_string());
    }
    
    // Verify it's a valid GGUF file (check magic bytes) in a blocking context
    let blob_path_verify = blob_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&blob_path_verify)
            .map_err(|e| format!("Failed to open blob: {}", e))?;
        let mut magic = [0u8; 4];
        std::io::Read::read_exact(&mut file, &mut magic)
            .map_err(|e| format!("Failed to read blob header: {}", e))?;
        if &magic != b"GGUF" {
            return Err("Blob is not a valid GGUF file".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e: String| e)?;
    
    let workspace = crate::config::resolve_workspace_root()?;
    let models_dir = workspace.join("inference").join("models");
    
    // Create models directory if it doesn't exist
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;
    
    let dest_filename = format!("{}.gguf", safe_model_name);
    let dest_path = models_dir.join(&dest_filename);
    
    // Emit extraction start toast
    crate::toast::emit_toast(
        &app_handle,
        "info",
        "Extracting Model",
        &format!("Extracting {} to local library...", model_name),
    );
    
    // Copy file with progress events
    let src_size = std::fs::metadata(&blob_path)
        .map(|m| m.len())
        .map_err(|e| format!("Failed to get blob size: {}", e))?;
    
    let blob_path_clone = blob_path.clone();
    let dest_path_clone = dest_path.clone();
    let app_clone = app_handle.clone();
    let model_name_clone = model_name.clone();
    
    // Run the copy in a blocking task to avoid blocking async runtime
    tokio::task::spawn_blocking(move || {
        use std::io::{BufReader, BufWriter, Read, Write};
        
        let src = std::fs::File::open(&blob_path_clone)
            .map_err(|e| format!("Failed to open source: {}", e))?;
        let dst = std::fs::File::create(&dest_path_clone)
            .map_err(|e| format!("Failed to create destination: {}", e))?;
        
        let mut reader = BufReader::with_capacity(8 * 1024 * 1024, src);
        let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, dst);
        
        let mut copied = 0u64;
        let mut buf = vec![0u8; 1024 * 1024]; // 1MB chunks
        let mut last_progress = 0u32;
        
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("Read error: {}", e))?;
            if n == 0 { break; }
            writer.write_all(&buf[..n]).map_err(|e| format!("Write error: {}", e))?;
            copied += n as u64;
            
            let progress = if src_size > 0 { ((copied as f64 / src_size as f64) * 100.0) as u32 } else { 100 };
            if progress != last_progress {
                last_progress = progress;
                let _ = app_clone.emit("model-extract-progress", serde_json::json!({
                    "model_name": model_name_clone,
                    "progress": progress,
                    "copied_bytes": copied,
                    "total_bytes": src_size,
                }));
            }
        }
        
        writer.flush().map_err(|e| format!("Flush error: {}", e))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e: String| e)?;
    
    // Emit completion toast
    crate::toast::emit_toast(
        &app_handle,
        "success",
        "Model Ready",
        &format!("{} added to local library", model_name),
    );
    
    let relative_path = format!("models/{}", dest_filename);
    Ok(relative_path)
}

/// Switch the active inference model.
#[tauri::command]
pub async fn switch_model(
    model_path: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use toml_edit::DocumentMut;
    
    // Write new model_path to inference.toml in a blocking context
    let config_path = crate::config::resolve_subsystem_config_path("inference")?;
    let model_path_clone = model_path.clone();
    tokio::task::spawn_blocking(move || {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read inference config: {}", e))?;
        
        let mut doc: DocumentMut = content.parse::<DocumentMut>()
            .map_err(|e| format!("Failed to parse config: {}", e))?;
        
        doc["model"]["model_path"] = toml_edit::value(&model_path_clone);
        
        std::fs::write(&config_path, doc.to_string())
            .map_err(|e| format!("Failed to write config: {}", e))?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e: String| e)?;
    
    // Emit toast
    crate::toast::emit_toast(
        &app_handle,
        "info",
        "Switching Model",
        &format!("Loading {}...", model_path),
    );
    
    // Mark inference as reloading
    if let Ok(mut debug_state) = state.debug_state.lock() {
        debug_state.set_subsystem_status("inference", crate::state::SubsystemHealthStatus::Unknown);
    }
    
    let _ = app_handle.emit(
        "subsystem-status-updated",
        crate::grpc::SubsystemStatusPayload {
            subsystem: "inference".to_string(),
            status: "Unknown".to_string(),
        },
    );
    
    info!(model_path = %model_path, "Model switch initiated");
    Ok(())
}

/// Get agent model assignments from persistent store.
#[tauri::command]
pub async fn get_agent_model_assignments(
    app_handle: tauri::AppHandle,
) -> Result<std::collections::HashMap<String, String>, String> {
    use tauri_plugin_store::StoreExt;
    
    let store = app_handle
        .store("agent-model-assignments.json")
        .map_err(|e| format!("Failed to access store: {}", e))?;
    
    let agents = [
        "reactive-loop", "reasoning", "memory", "file", "screen",
        "process", "browser", "peripheral", "tacet",
    ];
    
    let mut assignments = std::collections::HashMap::new();
    for agent in agents {
        let model_id = store
            .get(agent)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "auto".to_string());
        assignments.insert(agent.to_string(), model_id);
    }
    
    Ok(assignments)
}

/// Set the model assignment for a specific agent.
#[tauri::command]
pub async fn set_agent_model_assignment(
    agent: String,
    model_id: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;
    
    let store = app_handle
        .store("agent-model-assignments.json")
        .map_err(|e| format!("Failed to access store: {}", e))?;
    
    store.set(agent.clone(), serde_json::json!(model_id));
    store.save().map_err(|e| format!("Failed to persist assignment: {}", e))?;
    
    info!(agent = %agent, model_id = %model_id, "Agent model assignment updated");
    Ok(())
}

/// Delete a local model from inference/models/.
#[tauri::command]
pub async fn delete_local_model(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Normalize both paths before comparing to prevent alternate-representation bypass
    let normalized_path = normalize_model_path(&path);
    let active_model = state.debug_state.lock()
        .ok()
        .map(|s| normalize_model_path(&s.inference_stats.active_model))
        .unwrap_or_default();
    
    if active_model == normalized_path {
        return Err("Cannot delete the currently active model".to_string());
    }
    
    let workspace = crate::config::resolve_workspace_root()?;
    let full_path = workspace.join("inference").join(&path);
    
    if !full_path.exists() {
        return Err(format!("Model file not found: {}", path));
    }
    
    // Security: ensure the path is within inference/models/
    let models_dir = workspace.join("inference").join("models");
    let canonical_path = full_path.canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let canonical_models = models_dir.canonicalize()
        .map_err(|e| format!("Failed to resolve models dir: {}", e))?;
    
    if !canonical_path.starts_with(&canonical_models) {
        return Err("Path traversal not allowed".to_string());
    }
    
    // Run deletion in blocking context
    let full_path_clone = full_path.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::remove_file(&full_path_clone)
            .map_err(|e| format!("Failed to delete model: {}", e))
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e: String| e)?;
    
    info!(path = %path, "Local model deleted");
    Ok(())
}

/// Open the inference/models/ directory in the system file explorer.
#[tauri::command]
pub fn open_models_folder() -> Result<(), String> {
    let workspace = crate::config::resolve_workspace_root()?;
    let models_path = workspace.join("inference").join("models");
    
    // Create the directory if it doesn't exist
    if !models_path.exists() {
        std::fs::create_dir_all(&models_path)
            .map_err(|e| format!("Failed to create models directory: {}", e))?;
    }
    
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(models_path.as_os_str())
            .spawn()
            .map_err(|e| format!("Failed to open explorer: {}", e))?;
    }
    
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(models_path.as_os_str())
            .spawn()
            .map_err(|e| format!("Failed to open finder: {}", e))?;
    }
    
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(models_path.as_os_str())
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {}", e))?;
    }
    
    info!(path = %models_path.display(), "Opened models folder");
    Ok(())
}

/// Detect the default Ollama manifests directory.
fn detect_ollama_directory() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        let user_profile = std::env::var("USERPROFILE")
            .unwrap_or_else(|_| "C:\\Users\\Default".to_string());
        std::path::PathBuf::from(user_profile)
            .join(".ollama").join("models").join("manifests")
    }
    #[cfg(not(target_os = "windows"))]
    {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".ollama").join("models").join("manifests")
    }
}

/// Get the currently configured Ollama manifests directory.
/// Checks the persistent store first, then falls back to auto-detection.
#[tauri::command]
pub async fn get_ollama_directory(
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use tauri_plugin_store::StoreExt;
    
    let store = app_handle
        .store("ollama-settings.json") 
        .map_err(|e| format!("Failed to access store: {}", e))?;
    
    if let Some(val) = store.get("ollama_manifests_dir") {
        if let Some(dir) = val.as_str() {
            if std::path::Path::new(dir).exists() {
                return Ok(dir.to_string());
            }
        }
    }
    
    let detected = detect_ollama_directory();
    Ok(detected.to_string_lossy().to_string())
}

/// Open a folder picker to select the Ollama manifests directory.
/// Saves the selection to persistent store.
#[tauri::command]
pub async fn select_ollama_directory(
    app_handle: tauri::AppHandle,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    use tauri_plugin_store::StoreExt;
    
    let handle = app_handle.clone();
    let picked = tokio::task::spawn_blocking(move || {
        handle.dialog()
            .file()
            .set_title("Select Ollama Manifests Directory")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?;
    
    match picked {
        Some(path) => {
            let path_str = path.to_string();
            
            // Persist to store
            let store = app_handle
                .store("ollama-settings.json")
                .map_err(|e| format!("Failed to access store: {}", e))?;
            
            store.set("ollama_manifests_dir".to_string(), serde_json::json!(path_str));
            store.save().map_err(|e| format!("Failed to persist: {}", e))?;
            
            info!(path = %path_str, "Ollama directory changed");
            Ok(Some(path_str))
        }
        None => Ok(None), // User cancelled
    }
}

/// Show the model panel window.
#[tauri::command]
pub async fn show_model_panel(app_handle: tauri::AppHandle) -> Result<(), String> {
    crate::overlay::show_single_panel(&app_handle, crate::overlay::PANEL_MODEL)
}

