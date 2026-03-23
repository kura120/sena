#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod daemon_launcher;
mod generated;
mod gguf;
mod grpc;
mod hotkey;
mod overlay;
mod state;
mod toast;
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
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::start_event_stream,
            commands::send_message,
            commands::get_overlay_config,
            commands::get_debug_snapshot,
            commands::toggle_overlay_cmd,
            commands::save_window_position,
            commands::reboot_daemon_bus,
            commands::show_notification_history,
            commands::get_panel_states,
            commands::set_panel_state,
            commands::get_overlay_setting,
            commands::set_overlay_setting,
            commands::show_panel,
            commands::hide_panel,
            commands::show_settings_panel,
            commands::read_subsystem_config,
            commands::write_subsystem_config,
            commands::validate_subsystem_config,
            commands::restart_subsystem,
            commands::list_local_models,
            commands::list_ollama_models,
            commands::extract_ollama_model,
            commands::switch_model,
            commands::get_agent_model_assignments,
            commands::set_agent_model_assignment,
            commands::delete_local_model,
            commands::open_models_folder,
            commands::get_ollama_directory,
            commands::select_ollama_directory,
            commands::show_model_panel,
        ])
        .setup(|app| {
            // Create all overlay panel windows
            overlay::create_panel_windows(app)
                .map_err(|e| format!("Failed to create panel windows: {}", e))?;

            // Register the overlay toggle hotkey
            let toggle_key = app.state::<AppState>().config.overlay.toggle_key.clone();
            if let Err(e) = hotkey::register_overlay_hotkey(app, &toggle_key) {
                // Log error but don't fail startup
                tracing::error!(error = %e, "Failed to register overlay hotkey");
            }

            // Auto-launch daemon-bus and then start event stream
            let daemon_handle = app.handle().clone();
            let daemon_config = app.state::<AppState>().config.daemon_bus.clone();

            // Grab event stream params before spawning
            let stream_started = &app.state::<AppState>().stream_started;
            let should_start_stream = stream_started
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok();

            let es_app_handle = app.handle().clone();
            let es_address = app.state::<AppState>().config.grpc.daemon_bus_address.clone();
            let es_timeout = app.state::<AppState>().config.grpc.connection_timeout_ms;
            let es_reconnect = app.state::<AppState>().config.reconnect.clone();
            let es_debug_state = std::sync::Arc::clone(&app.state::<AppState>().debug_state);

            tauri::async_runtime::spawn(async move {
                // Wait for daemon-bus to be available
                daemon_launcher::ensure_daemon_bus_running(&daemon_handle, &daemon_config).await;

                // Allow boot signals to propagate before subscribing to event stream
                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

                // Start event stream after daemon is confirmed running
                if should_start_stream {
                    grpc::run_event_stream(
                        es_app_handle,
                        es_address,
                        es_timeout,
                        es_reconnect,
                        es_debug_state,
                    )
                    .await;
                }
            });

            // Setup system tray
            tray::setup_system_tray(app)
                .map_err(|e| format!("Failed to setup system tray: {}", e))?;

            // Emit initial hotkey toast after a short delay so windows have time to render
            let toast_handle = app.handle().clone();
            let hotkey_display = toggle_key.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                toast::emit_toast(
                    &toast_handle,
                    "info",
                    "Sena Debug Overlay",
                    &format!("Press {} to toggle the debug overlay", hotkey_display),
                );
            });

            // Signal UI_READY to daemon-bus after all windows are initialized.
            // UI is not required for SENA_READY (spawn_at_boot = false in supervisor
            // config) so a failure here must never prevent startup — just log and
            // continue. daemon-bus may not be running in standalone / dev mode.
            let signal_address = app
                .state::<AppState>()
                .config
                .grpc
                .daemon_bus_address
                .clone();
            tauri::async_runtime::spawn(async move {
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
        .build(tauri::generate_context!())
        .expect("failed to build Tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = &event {
                // Kill daemon-bus if the UI spawned it
                let config = app_handle.state::<AppState>();
                let address = config.config.daemon_bus.address.clone();
                let port = address
                    .rsplit(':')
                    .next()
                    .and_then(|p| p.parse::<u16>().ok())
                    .unwrap_or(50051);

                // Use a blocking runtime to run the async kill
                // We're in the exit handler so blocking is acceptable
                let rt = tokio::runtime::Runtime::new();
                if let Ok(rt) = rt {
                    let _ = rt.block_on(daemon_launcher::kill_process_on_port(port));
                    tracing::info!("daemon-bus killed on UI exit");
                }
            }
        });
}
