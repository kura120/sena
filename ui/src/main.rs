//! Sena UI subsystem — Freya-based debug panel and future chat interface.
//!
//! Launches a Freya window, connects to daemon-bus via gRPC, and renders
//! live debug state from the event stream. The debug panel toggles via F12
//! or the Inspect button.

use std::path::Path;
use std::sync::Arc;

use freya::prelude::*;
use tokio::sync::RwLock;

mod app;
mod components;
mod config;
mod debug_state;
mod grpc;
mod state;
mod theme;

mod proto {
    include!("generated/sena.daemonbus.v1.rs");
}

fn main() {
    // Load config.
    let config_path = std::env::var("UI_CONFIG")
        .unwrap_or_else(|_| "config/ui.toml".to_string());
    let config = config::Config::load(Path::new(&config_path)).unwrap_or_else(|err| {
        eprintln!("fatal: failed to load UI config: {err}");
        std::process::exit(1);
    });

    // Init structured logging.
    let log_level = config.logging.level.clone();
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .init();

    tracing::info!(
        event_type = "ui_starting",
        component = "main",
    );

    // Create shared debug state.
    let debug_state = Arc::new(RwLock::new(debug_state::DebugState::new(
        config.debug_panel.thought_feed_max,
        config.debug_panel.event_feed_max,
    )));

    // Spawn gRPC event stream task in the background.
    let grpc_state = Arc::clone(&debug_state);
    let grpc_address = config.grpc.daemon_bus_address.clone();
    let grpc_timeout = config.grpc.connection_timeout_ms;
    let reconnect_config = config.reconnect.clone();

    // We need to spawn the gRPC task on the tokio runtime that Freya will use.
    // Freya runs its own tokio runtime internally, so we spawn via std::thread
    // to avoid conflicting runtimes.
    let grpc_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for gRPC"); // Panic acceptable: fatal boot failure
        rt.block_on(grpc::run_event_stream(
            grpc_address,
            grpc_timeout,
            reconnect_config,
            grpc_state,
        ));
    });

    // Capture config values for the closure.
    let panel_width = config.debug_panel.width;
    // Intentional leak — title must be 'static for Freya's WindowConfig closure bound.
    let window_title: &'static str = Box::leak(config.window.title.clone().into_boxed_str());

    let debug_state_for_app = Arc::clone(&debug_state);
    let app_fn = move || {
        app::app(
            debug_state_for_app.clone(),
            panel_width,
            "Debug Panel",
            "Inspect",
            "No thoughts surfaced yet",
            "No events yet",
            "daemon-bus offline",
        )
    };

    // Launch Freya window.
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(app_fn)
                .with_title(window_title)
                .with_size(config.window.width, config.window.height),
        ),
    );

    tracing::info!(
        event_type = "ui_shutting_down",
        component = "main",
    );

    // gRPC thread will be cleaned up when the process exits.
    drop(grpc_handle);
}
