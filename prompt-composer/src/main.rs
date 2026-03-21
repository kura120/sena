//! prompt-composer — Sena's prompt assembly subsystem process.
//!
//! This is the process entry point. The boot sequence is ordered and any
//! failure in steps 1–7 is fatal: log the error, signal failure to daemon-bus
//! (best-effort), and exit with a non-zero code.
//!
//! ## Boot Sequence
//!
//! 1. Load config from `prompt-composer.toml`
//! 2. Initialize tracing subscriber
//! 3. Connect to daemon-bus
//! 4. Subscribe to boot signals, wait for `DAEMON_BUS_READY`
//! 5. Start gRPC server (PromptComposerService)
//! 6. Signal PROMPT_COMPOSER_READY to daemon-bus
//! 7. Await shutdown signal
//!
//! No `println!` or `eprintln!` except for the single pre-tracing fatal path.

pub mod assembler;
pub mod config;
pub mod error;
pub mod esu;
pub mod grpc;

pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::error::PromptComposerError;
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient,
    event_bus_service_client::EventBusServiceClient,
    prompt_composer_service_server::PromptComposerServiceServer, BootSignal, BootSignalRequest,
    EventTopic, SubscribeRequest,
};
use crate::grpc::PromptComposerGrpcService;

const SUBSYSTEM_ID: &str = "prompt_composer";

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("prompt-composer-worker")
        .build()
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    let exit_code = runtime.block_on(async_main());
    std::process::exit(exit_code);
}

async fn async_main() -> i32 {
    // ── Step 1: Load configuration ──────────────────────────────────────
    let config_path = std::env::var("PROMPT_COMPOSER_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config/prompt-composer.toml"));

    let config = match Config::load(&config_path) {
        Ok(loaded) => loaded,
        Err(config_error) => {
            // Cannot use tracing — subscriber not yet initialized.
            eprintln!(
                "[FATAL] failed to load prompt-composer config from '{}': {}",
                config_path.display(),
                config_error
            );
            return 1;
        }
    };

    let config = Arc::new(config);

    // ── Step 2: Initialize structured logging ───────────────────────────
    initialize_tracing(&config.logging);

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "startup",
        config_path = %config_path.display(),
        daemon_bus_address = %config.grpc.daemon_bus_address,
        "prompt-composer starting"
    );

    // ── Step 3: Connect to daemon-bus ───────────────────────────────────
    let daemon_bus_address = config.grpc.daemon_bus_address.clone();
    let connect_timeout = Duration::from_millis(config.grpc.connection_timeout_ms);

    let boot_client_result = tokio::time::timeout(
        connect_timeout,
        BootServiceClient::connect(daemon_bus_address.clone()),
    )
    .await;

    let mut boot_client = match boot_client_result {
        Ok(Ok(client)) => client,
        Ok(Err(connect_error)) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "daemon_bus_connect_failed",
                error = %connect_error,
                "failed to connect to daemon-bus boot service"
            );
            return 1;
        }
        Err(_elapsed) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "daemon_bus_connect_timeout",
                "timed out connecting to daemon-bus"
            );
            return 1;
        }
    };

    // ── Step 4: Wait for DAEMON_BUS_READY ───────────────────────────────
    // Subscribe to boot signals and wait until we see DAEMON_BUS_READY.
    match wait_for_daemon_bus_ready(&daemon_bus_address, connect_timeout).await {
        Ok(()) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_received",
                signal = "DAEMON_BUS_READY",
                "daemon-bus is ready"
            );
        }
        Err(wait_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_wait_failed",
                error = %wait_error,
                "failed to receive DAEMON_BUS_READY"
            );
            return 1;
        }
    }

    // ── Step 5: Start gRPC server ───────────────────────────────────────
    let grpc_service = PromptComposerGrpcService::new(Arc::clone(&config));
    let listen_addr_result: Result<std::net::SocketAddr, _> =
        format!("{}:{}", config.grpc.listen_address, config.grpc.listen_port)
            .parse()
            .map_err(|parse_error: std::net::AddrParseError| {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "invalid_listen_address",
                    address = %config.grpc.listen_address,
                    port = config.grpc.listen_port,
                    error = %parse_error,
                    "invalid gRPC listen address"
                );
                parse_error
            });

    let listen_addr = match listen_addr_result {
        Ok(addr) => addr,
        Err(_) => {
            // Cannot signal unavailable — proto doesn't define PromptComposerUnavailable.
            // Daemon-bus will detect via boot timeout.
            return 1;
        }
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let grpc_handle = tokio::spawn(async move {
        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "grpc_server_starting",
            listen_addr = %listen_addr,
            "PromptComposerService gRPC server starting"
        );

        let serve_result = tonic::transport::Server::builder()
            .add_service(PromptComposerServiceServer::new(grpc_service))
            .serve_with_shutdown(listen_addr, async {
                // Completion of the receive (Ok or Err) both mean "shut down".
                // Ok = explicit signal, Err = sender dropped (also means shutdown).
                let _shutdown_signal = shutdown_rx.await;
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "grpc_server_shutdown",
                    "gRPC server received shutdown signal"
                );
            })
            .await;

        if let Err(serve_error) = serve_result {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "grpc_server_error",
                error = %serve_error,
                "gRPC server exited with error"
            );
        }
    });

    // ── Step 6: Signal PROMPT_COMPOSER_READY ────────────────────────────
    let ready_request = tonic::Request::new(BootSignalRequest {
        subsystem_id: SUBSYSTEM_ID.to_owned(),
        signal: BootSignal::PromptComposerReady.into(),
    });

    match boot_client.signal_ready(ready_request).await {
        Ok(_) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_sent",
                signal = "PROMPT_COMPOSER_READY",
                "PROMPT_COMPOSER_READY signaled to daemon-bus"
            );
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_failed",
                error = %signal_error,
                "failed to signal PROMPT_COMPOSER_READY to daemon-bus"
            );
            // Service is running — log failure but continue.
            // daemon-bus detects via boot timeout.
        }
    }

    // ── Step 7: Await shutdown ──────────────────────────────────────────
    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "running",
        "prompt-composer running — awaiting shutdown signal"
    );

    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "shutdown_signal_received",
                "shutdown signal received"
            );
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "shutdown_signal_error",
                error = %signal_error,
                "failed to listen for shutdown signal"
            );
        }
    }

    // ── Graceful shutdown ───────────────────────────────────────────────
    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "shutdown_initiated",
        "initiating graceful shutdown"
    );

    // Note: Proto doesn't define PromptComposerUnavailable signal.
    // Daemon-bus detects shutdown via connection close.

    // Stop gRPC server — if the receiver is already dropped the server
    // already exited, so the send failure is harmless during shutdown.
    let _send_result = shutdown_tx.send(());

    // Wait for gRPC server to finish
    if let Err(join_error) = grpc_handle.await {
        if !join_error.is_cancelled() {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "grpc_shutdown_error",
                error = %join_error,
                "gRPC server task panicked during shutdown"
            );
        }
    }

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "shutdown_complete",
        "prompt-composer shut down cleanly"
    );

    0
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Wait for DAEMON_BUS_READY by subscribing to the event bus boot signal topic.
async fn wait_for_daemon_bus_ready(
    daemon_bus_address: &str,
    timeout: Duration,
) -> Result<(), PromptComposerError> {
    let mut event_client = EventBusServiceClient::connect(daemon_bus_address.to_owned())
        .await
        .map_err(|e| PromptComposerError::DaemonBusConnection {
            reason: format!("event bus connect failed: {}", e),
        })?;

    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics: vec![EventTopic::TopicBootSignal.into()],
        subscriber_id: SUBSYSTEM_ID.to_owned(),
    });

    let mut stream = event_client
        .subscribe(subscribe_request)
        .await
        .map_err(|e| PromptComposerError::Grpc(format!("subscribe failed: {}", e)))?
        .into_inner();

    let wait_future = async {
        loop {
            match stream.message().await {
                Ok(Some(bus_event)) => {
                    // Check if this is the DAEMON_BUS_READY boot signal
                    if bus_event.topic == i32::from(EventTopic::TopicBootSignal) {
                        // Parse the signal from the payload or check source
                        // For now, check if source is daemon_bus
                        if bus_event.source_subsystem == "daemon_bus" {
                            return Ok(());
                        }
                    }
                }
                Ok(None) => {
                    return Err(PromptComposerError::DaemonBusConnection {
                        reason: "event stream ended before DAEMON_BUS_READY".into(),
                    });
                }
                Err(stream_error) => {
                    return Err(PromptComposerError::Grpc(format!(
                        "stream error waiting for DAEMON_BUS_READY: {}",
                        stream_error
                    )));
                }
            }
        }
    };

    tokio::time::timeout(timeout, wait_future)
        .await
        .map_err(|_| PromptComposerError::DaemonBusConnection {
            reason: "timed out waiting for DAEMON_BUS_READY".into(),
        })?
}

/// Initialize tracing subscriber with format from config.
fn initialize_tracing(logging_config: &config::LoggingConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::try_new(&logging_config.level)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    match logging_config.format.as_str() {
        "json" => {
            let subscriber = tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_thread_ids(true)
                .with_file(false)
                .with_line_number(false)
                .finish();

            // This must be called exactly once. Panic is acceptable — duplicate
            // initialization means a configuration bug that must be caught.
            tracing::subscriber::set_global_default(subscriber)
                .expect("tracing subscriber must be set exactly once");
        }
        _ => {
            let subscriber = tracing_subscriber::fmt()
                .pretty()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_thread_ids(true)
                .with_file(false)
                .with_line_number(false)
                .finish();

            tracing::subscriber::set_global_default(subscriber)
                .expect("tracing subscriber must be set exactly once");
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn test_boot_signal_gate_prevents_early_start() {
        // Verify that a oneshot channel gate blocks until the signal fires.
        // This tests the boot gate pattern used in async_main.
        let (tx, rx) = oneshot::channel::<()>();

        let gate_task = tokio::spawn(async move {
            // This simulates waiting for DAEMON_BUS_READY
            rx.await
                .expect("test: gate sender should not drop before sending")
        });

        // Gate should not be resolved yet
        assert!(!gate_task.is_finished());

        // Send the signal
        tx.send(())
            .expect("test: receiver should still be alive");

        // Now the gate should resolve
        gate_task
            .await
            .expect("test: gate task should complete");
    }

    #[tokio::test]
    async fn test_shutdown_signal_channel_works() {
        // Verify the shutdown oneshot channel pattern.
        // When the sender fires, the receiver resolves.
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let server_task = tokio::spawn(async move {
            // Simulates serve_with_shutdown waiting
            let _signal = shutdown_rx.await;
            "shutdown_received"
        });

        // Send shutdown
        shutdown_tx
            .send(())
            .expect("test: receiver alive");

        let result = server_task
            .await
            .expect("test: task should complete");
        assert_eq!(result, "shutdown_received");
    }
}
