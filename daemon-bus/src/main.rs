//! daemon-bus — Sena's root process.
//!
//! This is the entry point for the entire Sena system. daemon-bus is the only
//! process that is never restarted by anything else — if it goes down, everything
//! goes down. It is pure infrastructure: no business logic lives here.
//!
//! Responsibilities:
//! - Load config from `config/daemon-bus.toml`
//! - Initialize structured logging via `tracing`
//! - Stand up the internal tokio broadcast event bus
//! - Initialize the process supervisor, watchdog, and priority arbiter
//! - Run the boot sequence orchestrator
//! - Serve gRPC on the configured address
//!
//! Every tunable value comes from config — nothing is hardcoded.

// ─────────────────────────────────────────────────────────────────────────────
// Module declarations
// ─────────────────────────────────────────────────────────────────────────────

pub mod arbitration;
pub mod boot;
pub mod bus;
pub mod config;
pub mod error;
pub mod grpc;
pub mod supervisor;
pub mod watchdog;

/// Proto-generated types. In a full build, `tonic-build` overwrites
/// `src/generated/sena.daemonbus.v1.rs` from the proto definition.
/// The placeholder file committed to the repo keeps the crate compilable
/// before the first `cargo build` runs codegen.
pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

use std::path::PathBuf;

use crate::arbitration::Arbiter;
use crate::boot::{BootOrchestrator, BootPhase};
use crate::bus::EventBus;
use crate::config::DaemonBusConfig;

use crate::supervisor::Supervisor;
use crate::watchdog::Watchdog;

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    // Build the tokio runtime explicitly rather than using `#[tokio::main]` so
    // that any runtime construction failure is caught and reported before any
    // async work begins. Multi-thread scheduler per PRD §13.2.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("daemon-bus-worker")
        .build()
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    runtime.block_on(async_main());
}

async fn async_main() {
    // ── Step 1: Load configuration ──────────────────────────────────────
    //
    // The config path is resolved relative to the daemon-bus crate root.
    // An override via the DAEMON_BUS_CONFIG environment variable is supported
    // for development and CI, but the default is always the checked-in file.
    // Default config path is relative to the workspace root so daemon-bus can
    // be launched from `C:\dev\Sena` without setting DAEMON_BUS_CONFIG.
    // When running from the individual crate directory (e.g. during development),
    // set the env var: DAEMON_BUS_CONFIG=config/daemon-bus.toml
    let config_path = std::env::var("DAEMON_BUS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("daemon-bus/config/daemon-bus.toml"));

    let config = match DaemonBusConfig::load(&config_path).await {
        Ok(loaded_config) => loaded_config,
        Err(config_error) => {
            // Cannot use tracing yet — subscriber is not initialized.
            // This is the one place where eprintln is acceptable: logging
            // infrastructure depends on config, so a config failure must be
            // surfaced via stderr.
            eprintln!(
                "[FATAL] failed to load daemon-bus config from '{}': {}",
                config_path.display(),
                config_error
            );
            if let Some(ref debug_ctx) = config_error.debug_context {
                eprintln!("[FATAL] debug context: {}", debug_ctx);
            }
            std::process::exit(1);
        }
    };

    // ── Step 2: Initialize structured logging ───────────────────────────
    //
    // tracing crate exclusively, structured fields only — per
    // copilot-instructions.md. OpenTelemetry integration is deferred to a
    // follow-up PR; for now the subscriber outputs JSON or pretty-printed
    // structured logs to stdout.
    initialize_tracing(&config.logging);

    tracing::info!(
        subsystem = "daemon_bus",
        event_type = "startup",
        config_path = %config_path.display(),
        grpc_address = %config.grpc.socket_addr(),
        "daemon-bus starting"
    );

    // ── Step 3: Create the internal event bus ────────────────────────────
    let event_bus = EventBus::new(config.bus.channel_capacity);

    tracing::info!(
        subsystem = "daemon_bus",
        event_type = "bus_created",
        channel_capacity = config.bus.channel_capacity,
        "internal event bus created"
    );

    // ── Step 4: Create the process supervisor ────────────────────────────
    let supervisor = Supervisor::new(config.supervisor.clone(), event_bus.clone());

    // ── Step 5: Create the watchdog ──────────────────────────────────────
    let watchdog = Watchdog::new(config.watchdog.clone(), event_bus.clone());
    // Start the sweep loop and store the handle so it is never silently
    // dropped — a dropped handle would cancel the sweep task.
    let _watchdog_sweep_handle = watchdog.start_sweep_loop();

    // ── Step 6: Create the priority arbiter ──────────────────────────────
    let _arbiter = Arbiter::new(config.arbitration.clone(), event_bus.clone());

    // ── Step 7: Create boot orchestrator and start gRPC server ──────────
    //
    // The boot orchestrator is created first so the gRPC BootService can
    // hold a handle to it. The gRPC server is started *before* the boot
    // sequence runs so that child subsystems can call SignalReady over gRPC
    // as soon as they are spawned.
    let boot_orchestrator =
        BootOrchestrator::new(&config.boot, event_bus.clone(), supervisor.clone());

    let _grpc_server_handle = match grpc::start_grpc_server(
        &config.grpc,
        boot_orchestrator.clone(),
        event_bus.clone(),
    )
    .await
    {
        Ok(handle) => handle,
        Err(grpc_error) => {
            tracing::error!(
                subsystem = "daemon_bus",
                event_type = "grpc_server_start_failed",
                error_code = %grpc_error.code,
                error_message = %grpc_error.message,
                "failed to start gRPC server — child subsystems will not be able to signal readiness"
            );
            std::process::exit(1);
        }
    };

    // ── Step 8: Run the boot sequence ────────────────────────────────────
    //
    // The gRPC server is now accepting connections. The boot orchestrator
    // spawns child subsystems which connect back to gRPC to signal readiness.
    let mut boot_phase_receiver = match boot_orchestrator.run().await {
        Ok(receiver) => receiver,
        Err(boot_error) => {
            tracing::error!(
                subsystem = "daemon_bus",
                event_type = "boot_start_failed",
                error_code = %boot_error.code,
                error_message = %boot_error.message,
                "failed to start boot sequence"
            );
            std::process::exit(1);
        }
    };

    // Wait for boot to reach a terminal state (Ready or Failed).
    // The gRPC server is already running so child subsystems can call
    // SignalReady while this loop waits.
    loop {
        // wait for changes; the initial value is InProgress
        if boot_phase_receiver.changed().await.is_err() {
            // Sender dropped — should not happen during boot.
            tracing::error!(
                subsystem = "daemon_bus",
                event_type = "boot_phase_sender_dropped",
                "boot phase watch sender dropped unexpectedly"
            );
            break;
        }

        let phase = boot_phase_receiver.borrow().clone();
        match phase {
            BootPhase::Ready => {
                tracing::info!(
                    subsystem = "daemon_bus",
                    event_type = "sena_ready",
                    "SENA_READY — all required subsystems signaled"
                );
                break;
            }
            BootPhase::Failed { ref reason } => {
                tracing::error!(
                    subsystem = "daemon_bus",
                    event_type = "boot_failed",
                    reason = %reason,
                    "boot sequence failed — Sena is not ready"
                );
                // PRD §10.5: Sena does not enter a partially-ready state.
                // A minimal system notification would be shown here in the
                // full implementation. For the scaffold, we exit.
                supervisor.shutdown_all().await;
                std::process::exit(1);
            }
            BootPhase::InProgress => {
                // Still waiting — loop back to await the next change.
                continue;
            }
        }
    }

    // ── Step 9: Keep daemon-bus alive ────────────────────────────────────
    //
    // The gRPC server is running in a background task. Hold the main task
    // open until a shutdown signal is received.
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!(
                subsystem = "daemon_bus",
                event_type = "shutdown_requested",
                "received shutdown signal — initiating graceful shutdown"
            );
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = "daemon_bus",
                event_type = "shutdown_signal_error",
                error = %signal_error,
                "failed to listen for shutdown signal"
            );
        }
    }

    // ── Graceful shutdown ────────────────────────────────────────────────
    tracing::info!(
        subsystem = "daemon_bus",
        event_type = "shutdown_started",
        "daemon-bus shutting down"
    );

    supervisor.shutdown_all().await;

    tracing::info!(
        subsystem = "daemon_bus",
        event_type = "shutdown_complete",
        "daemon-bus shutdown complete"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tracing initialization
// ─────────────────────────────────────────────────────────────────────────────

/// Initialize the `tracing` subscriber based on config.
///
/// Uses `tracing-subscriber` with either a JSON layer (for production) or a
/// pretty-printed layer (for development). The log level filter is set from
/// `config.logging.level`.
fn initialize_tracing(logging_config: &config::LoggingConfig) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let env_filter =
        EnvFilter::try_new(&logging_config.level).unwrap_or_else(|_| EnvFilter::new("info"));

    match logging_config.format.as_str() {
        "json" => {
            let subscriber = fmt::Subscriber::builder()
                .with_env_filter(env_filter)
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false)
                .with_target(true)
                .with_thread_ids(true)
                .finish();

            tracing::subscriber::set_global_default(subscriber)
                .expect("failed to set tracing subscriber — logging will not work");
        }
        _ => {
            // "pretty" or any unrecognized format defaults to human-readable output.
            let subscriber = fmt::Subscriber::builder()
                .with_env_filter(env_filter)
                .pretty()
                .with_target(true)
                .with_thread_ids(true)
                .finish();

            tracing::subscriber::set_global_default(subscriber)
                .expect("failed to set tracing subscriber — logging will not work");
        }
    }
}
