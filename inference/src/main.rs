//! inference — Sena's inference subsystem process.
//!
//! This is the process entry point. The boot sequence is ordered and any
//! failure in steps 1–8 is fatal: log the error, signal failure to daemon-bus
//! (best-effort), and exit with a non-zero code.
//!
//! ## Boot Sequence
//!
//! 1. Load config from `inference.toml`
//! 2. Initialize tracing subscriber
//! 3. Connect to daemon-bus
//! 4. Subscribe to boot signals, wait for `DAEMON_BUS_READY`
//! 5. Initialize LlamaBackend
//! 6. Initialize InferenceEngine, load model
//! 7. Spawn worker task
//! 8. Start gRPC server (InferenceService)
//! 9. Signal INFERENCE_READY to daemon-bus
//! 10. Await shutdown signal
//!
//! No `println!` or `eprintln!` except for the single pre-tracing fatal path.

pub mod config;
pub mod error;
pub mod grpc;
pub mod inference_engine;
pub mod model_loader;
pub mod model_registry;
pub mod request_queue;

pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::error::InferenceError;
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient, event_bus_service_client::EventBusServiceClient,
    inference_service_server::InferenceServiceServer, BootSignal, BootSignalRequest, EventTopic,
    SubscribeRequest,
};
use crate::grpc::InferenceGrpcService;
use crate::inference_engine::InferenceEngine;

const SUBSYSTEM_ID: &str = "inference";

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("inference-worker")
        .build()
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    let exit_code = runtime.block_on(async_main());
    std::process::exit(exit_code);
}

async fn async_main() -> i32 {
    // ── Step 1: Load configuration ──────────────────────────────────────
    let config_path = std::env::var("INFERENCE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config/inference.toml"));

    let config = match Config::load(&config_path) {
        Ok(loaded) => loaded,
        Err(config_error) => {
            // Cannot use tracing — subscriber not yet initialized.
            eprintln!(
                "[FATAL] failed to load inference config from '{}': {}",
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
        "inference starting"
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

    // ── Step 5: Initialize LlamaBackend ─────────────────────────────────
    let llama_backend = match llama_cpp_2::llama_backend::LlamaBackend::init() {
        Ok(backend) => Arc::new(backend),
        Err(backend_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "llama_backend_init_failed",
                error = %backend_error,
                "failed to initialize LlamaBackend"
            );
            best_effort_signal(&mut boot_client, BootSignal::InferenceUnavailable).await;
            return 1;
        }
    };

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "llama_backend_ready",
        "LlamaBackend initialized"
    );

    // ── Step 6: Initialize engine and load model ────────────────────────
    let engine = Arc::new(InferenceEngine::new(Arc::clone(&config)));

    let model_id = config.model.model_id.clone();
    let model_path = config.model.model_path.clone();

    match engine
        .load_model_with_oom_retry(&model_id, &model_path, Arc::clone(&llama_backend))
        .await
    {
        Ok(()) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "model_ready",
                model_id = %model_id,
                "model loaded successfully"
            );
        }
        Err(InferenceError::InsufficientVram {
            required_mb,
            available_mb,
        }) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "model_load_degraded",
                model_id = %model_id,
                required_vram_mb = required_mb,
                available_vram_mb = available_mb,
                "model load failed after OOM retry — signaling INFERENCE_DEGRADED"
            );
            best_effort_signal(&mut boot_client, BootSignal::InferenceDegraded).await;
            return 1;
        }
        Err(load_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "model_load_failed",
                model_id = %model_id,
                error = %load_error,
                "failed to load model"
            );
            best_effort_signal(&mut boot_client, BootSignal::InferenceUnavailable).await;
            return 1;
        }
    }

    // ── Step 7: Spawn worker task ───────────────────────────────────────
    let worker_engine = Arc::clone(&engine);
    let worker_handle = tokio::spawn(async move {
        if let Err(worker_error) = worker_engine.run_worker().await {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "worker_error",
                error = %worker_error,
                "inference worker exited with error"
            );
        }
    });

    // ── Step 8: Start gRPC server ───────────────────────────────────────
    let grpc_service = InferenceGrpcService::new(Arc::clone(&engine));
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
            best_effort_signal(&mut boot_client, BootSignal::InferenceUnavailable).await;
            return 1;
        }
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let grpc_handle = tokio::spawn(async move {
        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "grpc_server_starting",
            listen_addr = %listen_addr,
            "InferenceService gRPC server starting"
        );

        let serve_result = tonic::transport::Server::builder()
            .add_service(InferenceServiceServer::new(grpc_service))
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

    // ── Step 9: Signal INFERENCE_READY ───────────────────────────────────
    // Get display_name and VRAM info from registry
    let display_name = engine
        .registry()
        .get_display_name(&model_id)
        .await
        .unwrap_or_else(|_| model_id.clone());

    let vram_used_mb = engine.registry().total_vram_allocated_mb().await;

    let ready_request = tonic::Request::new(BootSignalRequest {
        subsystem_id: SUBSYSTEM_ID.to_owned(),
        signal: BootSignal::InferenceReady.into(),
    });

    match boot_client.signal_ready(ready_request).await {
        Ok(_) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_sent",
                signal = "INFERENCE_READY",
                model_display_name = %display_name,
                vram_used_mb = vram_used_mb,
                "INFERENCE_READY signaled to daemon-bus"
            );

            // Publish VRAM and model info to the event bus for UI consumption
            // This allows the debug UI to display VRAM usage and model display name
            let vram_total_mb = config.model.vram_budget_mb;
            let model_info_payload = serde_json::json!({
                "model_id": &model_id,
                "model_display_name": &display_name,
                "vram_used_mb": vram_used_mb,
                "vram_total_mb": vram_total_mb,
            });

            // Connect to event bus and publish the model info
            match EventBusServiceClient::connect(daemon_bus_address.clone()).await {
                Ok(mut event_client) => {
                    let bus_event = crate::generated::sena_daemonbus_v1::BusEvent {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        topic: EventTopic::TopicInferenceModelSwitching.into(),
                        source_subsystem: SUBSYSTEM_ID.to_string(),
                        payload: model_info_payload.to_string().into_bytes(),
                        trace_context: String::new(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };

                    let publish_request = tonic::Request::new(
                        crate::generated::sena_daemonbus_v1::PublishRequest {
                            event: Some(bus_event),
                        },
                    );

                    if let Err(publish_error) = event_client.publish(publish_request).await {
                        tracing::warn!(
                            subsystem = SUBSYSTEM_ID,
                            event_type = "model_info_publish_failed",
                            error = %publish_error,
                            "failed to publish model info to event bus (non-fatal)"
                        );
                    } else {
                        tracing::info!(
                            subsystem = SUBSYSTEM_ID,
                            event_type = "model_info_published",
                            "published model info to event bus for UI"
                        );
                    }
                }
                Err(connect_error) => {
                    tracing::warn!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "event_bus_connect_failed",
                        error = %connect_error,
                        "failed to connect to event bus to publish model info (non-fatal)"
                    );
                }
            }
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_failed",
                error = %signal_error,
                "failed to signal INFERENCE_READY to daemon-bus"
            );
            // Engine is running — log failure but continue.
            // daemon-bus detects via boot timeout.
        }
    }

    // ── Model Switching Event Subscription ──────────────────────────────
    // Spawn a task to subscribe to TOPIC_INFERENCE_MODEL_SWITCHING events
    // and trigger model hot-swap when requested
    #[derive(serde::Deserialize)]
    struct ModelSwitchPayload {
        model_id: String,
        model_path: String,
    }

    let event_engine = Arc::clone(&engine);
    let event_backend = Arc::clone(&llama_backend);
    let event_daemon_bus_address = daemon_bus_address.clone();

    tokio::spawn(async move {
        // Create a new BootServiceClient for this task
        let mut event_boot_client =
            match BootServiceClient::connect(event_daemon_bus_address.clone()).await {
                Ok(client) => client,
                Err(connect_error) => {
                    tracing::error!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "event_boot_client_connect_failed",
                        error = %connect_error,
                        "failed to connect boot client for model switching events"
                    );
                    return;
                }
            };

        // Create EventBusServiceClient
        let mut event_client = match EventBusServiceClient::connect(event_daemon_bus_address).await
        {
            Ok(client) => client,
            Err(connect_error) => {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_bus_connect_failed",
                    error = %connect_error,
                    "failed to connect to daemon-bus event bus for model switching"
                );
                return;
            }
        };

        // Subscribe to TOPIC_INFERENCE_MODEL_SWITCHING
        let subscribe_request = tonic::Request::new(SubscribeRequest {
            topics: vec![EventTopic::TopicInferenceModelSwitching.into()],
            subscriber_id: SUBSYSTEM_ID.to_owned(),
        });

        let mut event_stream = match event_client.subscribe(subscribe_request).await {
            Ok(response) => response.into_inner(),
            Err(subscribe_error) => {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_subscribe_failed",
                    error = %subscribe_error,
                    "failed to subscribe to TOPIC_INFERENCE_MODEL_SWITCHING"
                );
                return;
            }
        };

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "model_switching_subscription_active",
            "subscribed to TOPIC_INFERENCE_MODEL_SWITCHING"
        );

        // Process events as they arrive
        loop {
            match event_stream.message().await {
                Ok(Some(bus_event)) => {
                    tracing::debug!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "model_switching_event_received",
                        event_id = %bus_event.event_id,
                        "received model switching event"
                    );

                    // Parse the payload as JSON
                    let payload =
                        match serde_json::from_slice::<ModelSwitchPayload>(&bus_event.payload) {
                            Ok(p) => p,
                            Err(parse_error) => {
                                tracing::error!(
                                    subsystem = SUBSYSTEM_ID,
                                    event_type = "model_switch_payload_parse_failed",
                                    event_id = %bus_event.event_id,
                                    error = %parse_error,
                                    "failed to parse model switch payload"
                                );
                                continue;
                            }
                        };

                    // Call swap_model
                    tracing::info!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "model_swap_requested",
                        model_id = %payload.model_id,
                        model_path = %payload.model_path,
                        "initiating model swap"
                    );

                    let swap_result = event_engine
                        .swap_model(
                            &payload.model_id,
                            &payload.model_path,
                            Arc::clone(&event_backend),
                            &mut event_boot_client,
                        )
                        .await;

                    match swap_result {
                        Ok(()) => {
                            tracing::info!(
                                subsystem = SUBSYSTEM_ID,
                                event_type = "model_swap_success",
                                model_id = %payload.model_id,
                                "model swap completed successfully"
                            );
                        }
                        Err(swap_error) => {
                            tracing::error!(
                                subsystem = SUBSYSTEM_ID,
                                event_type = "model_swap_error",
                                model_id = %payload.model_id,
                                error = %swap_error,
                                "model swap failed"
                            );
                        }
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "event_stream_ended",
                        "model switching event stream ended"
                    );
                    break;
                }
                Err(stream_error) => {
                    tracing::error!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "event_stream_error",
                        error = %stream_error,
                        "error reading model switching event stream"
                    );
                    break;
                }
            }
        }
    });

    // ── Step 10: Await shutdown ─────────────────────────────────────────
    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "running",
        "inference running — awaiting shutdown signal"
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

    // Signal INFERENCE_UNAVAILABLE before stopping
    best_effort_signal(&mut boot_client, BootSignal::InferenceUnavailable).await;

    // Stop gRPC server — if the receiver is already dropped the server
    // already exited, so the send failure is harmless during shutdown.
    let _send_result = shutdown_tx.send(());

    // Abort the worker
    worker_handle.abort();

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
        "inference shut down cleanly"
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
) -> Result<(), InferenceError> {
    let mut event_client = EventBusServiceClient::connect(daemon_bus_address.to_owned())
        .await
        .map_err(|e| InferenceError::DaemonBusConnection {
            reason: format!("event bus connect failed: {}", e),
        })?;

    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics: vec![EventTopic::TopicBootSignal.into()],
        subscriber_id: SUBSYSTEM_ID.to_owned(),
    });

    let mut stream = event_client
        .subscribe(subscribe_request)
        .await
        .map_err(|e| InferenceError::Grpc(format!("subscribe failed: {}", e)))?
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
                    return Err(InferenceError::DaemonBusConnection {
                        reason: "event stream ended before DAEMON_BUS_READY".into(),
                    });
                }
                Err(stream_error) => {
                    return Err(InferenceError::Grpc(format!(
                        "stream error waiting for DAEMON_BUS_READY: {}",
                        stream_error
                    )));
                }
            }
        }
    };

    tokio::time::timeout(timeout, wait_future)
        .await
        .map_err(|_| InferenceError::DaemonBusConnection {
            reason: "timed out waiting for DAEMON_BUS_READY".into(),
        })?
}

/// Best-effort signal to daemon-bus. Logs on failure but does not propagate.
async fn best_effort_signal(
    boot_client: &mut BootServiceClient<tonic::transport::Channel>,
    signal: BootSignal,
) {
    let request = tonic::Request::new(BootSignalRequest {
        subsystem_id: SUBSYSTEM_ID.to_owned(),
        signal: signal.into(),
    });

    match boot_client.signal_ready(request).await {
        Ok(_) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_sent",
                signal = ?signal,
                "boot signal sent to daemon-bus"
            );
        }
        Err(grpc_error) => {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_failed",
                signal = ?signal,
                error = %grpc_error,
                "failed to send boot signal to daemon-bus (best-effort)"
            );
        }
    }
}

/// Initialize tracing subscriber with format from config.
fn initialize_tracing(logging_config: &config::LoggingConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter =
        EnvFilter::try_new(&logging_config.level).unwrap_or_else(|_| EnvFilter::new("info"));

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
        tx.send(()).expect("test: receiver should still be alive");

        // Now the gate should resolve
        gate_task.await.expect("test: gate task should complete");
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
        shutdown_tx.send(()).expect("test: receiver alive");

        let result = server_task.await.expect("test: task should complete");
        assert_eq!(result, "shutdown_received");
    }
}
