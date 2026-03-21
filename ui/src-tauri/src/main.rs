#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod generated;
mod grpc;
mod hotkey;
mod overlay;
mod state;
mod tray;

use commands::AppState;
use config::Config;
use state::DebugState;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tracing::info;

fn main() {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    // Load configuration
    let config = Config::load().expect("Failed to load UI configuration");
    info!("UI configuration loaded successfully");

    // Initialize debug state
    let debug_state = Arc::new(Mutex::new(DebugState::new(
        config.debug_panel.thought_feed_max,
        config.debug_panel.event_feed_max,
    )));

    // Create app state
    let app_state = AppState {
        debug_state,
        config,
        stream_started: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::start_event_stream,
            commands::send_message,
            commands::get_overlay_config,
            commands::toggle_overlay_cmd,
            commands::save_window_position,
        ])
        .setup(|app| {
            // Create all overlay panel windows
            overlay::create_panel_windows(app)
                .map_err(|e| format!("Failed to create panel windows: {}", e))?;

            // Register the overlay toggle hotkey
            let toggle_key = app
                .state::<AppState>()
                .config
                .overlay
                .toggle_key
                .clone();
            if let Err(e) = hotkey::register_overlay_hotkey(app, &toggle_key) {
                // Log error but don't fail startup
                tracing::error!(error = %e, "Failed to register overlay hotkey");
            }

            // Setup system tray
            tray::setup_system_tray(app)
                .map_err(|e| format!("Failed to setup system tray: {}", e))?;

            // Start the gRPC event stream
            let app_handle = app.handle().clone();
            let app_state = app.state::<AppState>();
            if app_state
                .stream_started
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok()
            {
                let address = app_state.config.grpc.daemon_bus_address.clone();
                let connection_timeout_ms = app_state.config.grpc.connection_timeout_ms;
                let reconnect_config = app_state.config.reconnect.clone();
                let debug_state = std::sync::Arc::clone(&app_state.debug_state);

                tauri::async_runtime::spawn(async move {
                    grpc::run_event_stream(
                        app_handle,
                        address,
                        connection_timeout_ms,
                        reconnect_config,
                        debug_state,
                    )
                    .await;
                });
            }

            // Signal UI_READY to daemon-bus after all windows are initialized.
            // UI is not required for SENA_READY (spawn_at_boot = false in supervisor
            // config) so a failure here must never prevent startup — just log and
            // continue. daemon-bus may not be running in standalone / dev mode.
            let signal_address = app.state::<AppState>().config.grpc.daemon_bus_address.clone();
            tokio::spawn(async move {
                match grpc::signal_ui_ready(&signal_address).await {
                    Ok(()) => info!("UI_READY signal acknowledged by daemon-bus"),
                    Err(e) => tracing::warn!(
                        error = %e,
                        "Failed to signal UI_READY — running in standalone mode"
                    ),
                }
            });

            info!("Tauri application setup complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Tauri application");
}
