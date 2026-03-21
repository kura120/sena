//! Reactive-loop subsystem — Sena's user-facing message handler.
//!
//! Priority Tier 1 (Reactive) — orchestrates the full conversation flow:
//! 1. Receive user message via UserMessageService.SendMessage
//! 2. Publish TOPIC_USER_MESSAGE_RECEIVED
//! 3. Build PromptContext (minimal, since CTP doesn't exist yet)
//! 4. Call prompt-composer to assemble prompt (with fallback)
//! 5. Call inference to generate response (with fallback)
//! 6. Publish TOPIC_USER_MESSAGE_RESPONSE
//! 7. Return response to caller

mod config;
mod error;
mod generated;
mod grpc;
mod handler;

use crate::config::Config;
use crate::error::ReactiveLoopError;
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient, event_bus_service_client::EventBusServiceClient,
    inference_service_client::InferenceServiceClient,
    prompt_composer_service_client::PromptComposerServiceClient,
    user_message_service_server::UserMessageServiceServer, BootSignal, BootSignalRequest,
    EventTopic, SubscribeRequest,
};
use crate::grpc::UserMessageGrpcService;
use crate::handler::MessageHandler;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

const SUBSYSTEM_ID: &str = "reactive_loop";

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("reactive-loop-worker")
        .build()
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    let exit_code = runtime.block_on(async_main());
    std::process::exit(exit_code);
}

async fn async_main() -> i32 {
    // ========== STEP 1: Load Configuration ==========
    let config_path = std::env::var("REACTIVE_LOOP_CONFIG")
        .unwrap_or_else(|_| "config/reactive-loop.toml".to_string());
    let config_path = Path::new(&config_path);

    let config = match Config::load(config_path) {
        Ok(loaded) => loaded,
        Err(config_error) => {
            // Cannot use tracing — subscriber not yet initialized.
            eprintln!(
                "[FATAL] failed to load {} config from '{}': {}",
                SUBSYSTEM_ID,
                config_path.display(),
                config_error
            );
            return 1;
        }
    };

    // ========== STEP 2: Initialize Tracing ==========
    initialize_tracing(&config.logging);

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "startup",
        config_path = %config_path.display(),
        daemon_bus_address = %config.grpc.daemon_bus_address,
        inference_address = %config.grpc.inference_address,
        prompt_composer_address = %config.grpc.prompt_composer_address,
        listen_address = %config.grpc.listen_address,
        listen_port = config.grpc.listen_port,
        "reactive-loop starting"
    );

    let config = Arc::new(config);

    // ========== STEP 3: Connect to daemon-bus ==========
    let daemon_bus_address = config.grpc.daemon_bus_address.clone();
    let connect_timeout = Duration::from_millis(config.grpc.connection_timeout_ms);

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "daemon_bus_connecting",
        address = %daemon_bus_address,
        timeout_ms = config.grpc.connection_timeout_ms,
        "connecting to daemon-bus"
    );

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
                event_type = "daemon_bus_connection_failed",
                address = %daemon_bus_address,
                error = %connect_error,
                "failed to connect to daemon-bus"
            );
            return 1;
        }
        Err(_elapsed) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "daemon_bus_connection_timeout",
                address = %daemon_bus_address,
                timeout_ms = config.grpc.connection_timeout_ms,
                "daemon-bus connection timed out"
            );
            return 1;
        }
    };

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "daemon_bus_connected",
        "connected to daemon-bus"
    );

    // ========== STEP 4: Wait for DAEMON_BUS_READY ==========
    let ready_timeout = Duration::from_millis(config.boot.ready_signal_timeout_ms);
    if let Err(wait_error) = wait_for_daemon_bus_ready(&daemon_bus_address, ready_timeout).await {
        tracing::error!(
            subsystem = SUBSYSTEM_ID,
            event_type = "daemon_bus_ready_wait_failed",
            error = %wait_error,
            "failed to wait for DAEMON_BUS_READY signal"
        );
        // Note: Proto doesn't define ReactiveLoopUnavailable signal.
        return 1;
    }

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "daemon_bus_ready_received",
        "DAEMON_BUS_READY signal received"
    );

    // ========== STEP 5: Connect to inference and prompt-composer ==========
    let event_bus_client = match EventBusServiceClient::connect(daemon_bus_address.clone()).await {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "event_bus_connection_failed",
                error = %e,
                "failed to connect to event bus"
            );
            // Note: Proto doesn't define ReactiveLoopUnavailable signal.
            return 1;
        }
    };

    let inference_address = config.grpc.inference_address.clone();
    let inference_client = match tokio::time::timeout(
        connect_timeout,
        InferenceServiceClient::connect(inference_address.clone()),
    )
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                event_type = "inference_connection_failed",
                address = %inference_address,
                error = %e,
                "failed to connect to inference (will use fallback responses)"
            );
            // Create a lazy client that won't actually connect
            InferenceServiceClient::new(
                tonic::transport::Channel::from_shared(inference_address)
                    .expect("valid inference address") // Safe: validated in config
                    .connect_lazy(),
            )
        }
        Err(_) => {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                event_type = "inference_connection_timeout",
                address = %inference_address,
                "inference connection timed out (will use fallback responses)"
            );
            InferenceServiceClient::new(
                tonic::transport::Channel::from_shared(inference_address)
                    .expect("valid inference address") // Safe: validated in config
                    .connect_lazy(),
            )
        }
    };

    let prompt_composer_address = config.grpc.prompt_composer_address.clone();
    let prompt_composer_client = match tokio::time::timeout(
        connect_timeout,
        PromptComposerServiceClient::connect(prompt_composer_address.clone()),
    )
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                event_type = "prompt_composer_connection_failed",
                address = %prompt_composer_address,
                error = %e,
                "failed to connect to prompt-composer (will use minimal context mode)"
            );
            PromptComposerServiceClient::new(
                tonic::transport::Channel::from_shared(prompt_composer_address)
                    .expect("valid prompt composer address") // Safe: validated in config
                    .connect_lazy(),
            )
        }
        Err(_) => {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                event_type = "prompt_composer_connection_timeout",
                address = %prompt_composer_address,
                "prompt-composer connection timed out (will use minimal context mode)"
            );
            PromptComposerServiceClient::new(
                tonic::transport::Channel::from_shared(prompt_composer_address)
                    .expect("valid prompt composer address") // Safe: validated in config
                    .connect_lazy(),
            )
        }
    };

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "service_clients_initialized",
        "all service clients initialized"
    );

    // ========== STEP 6: Initialize Handler ==========
    let handler = Arc::new(MessageHandler::new(
        Arc::clone(&config),
        event_bus_client,
        inference_client,
        prompt_composer_client,
    ));

    // ========== STEP 7: Start gRPC Server ==========
    let grpc_service = UserMessageGrpcService::new(Arc::clone(&handler));
    let listen_addr = format!(
        "{}:{}",
        config.grpc.listen_address, config.grpc.listen_port
    )
    .parse()
    .expect("valid listen address from config"); // Safe: validated in config

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let grpc_handle = tokio::spawn(async move {
        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "grpc_server_starting",
            listen_addr = %listen_addr,
            "starting gRPC server"
        );

        let serve_result = tonic::transport::Server::builder()
            .add_service(UserMessageServiceServer::new(grpc_service))
            .serve_with_shutdown(listen_addr, async {
                let _ = shutdown_rx.await;
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "grpc_server_shutdown",
                    "gRPC server shutting down"
                );
            })
            .await;

        if let Err(serve_error) = serve_result {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "grpc_server_error",
                error = %serve_error,
                "gRPC server error"
            );
        }
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ========== STEP 8: Signal REACTIVE_LOOP_READY ==========
    let ready_request = tonic::Request::new(BootSignalRequest {
        subsystem_id: SUBSYSTEM_ID.to_owned(),
        signal: BootSignal::ReactiveLoopReady.into(),
    });

    match boot_client.signal_ready(ready_request).await {
        Ok(_) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_sent",
                signal = "REACTIVE_LOOP_READY",
                "signaled ready to daemon-bus"
            );
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_failed",
                signal = "REACTIVE_LOOP_READY",
                error = %signal_error,
                "failed to signal ready — daemon-bus will detect via boot timeout"
            );
            // Continue anyway — daemon-bus will detect via timeout
        }
    }

    // ========== STEP 9: Await Shutdown Signal ==========
    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "ready",
        "reactive-loop is ready and serving"
    );

    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "shutdown_signal_received",
                "received ctrl-c, initiating graceful shutdown"
            );
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "signal_handler_error",
                error = %signal_error,
                "failed to listen for ctrl-c"
            );
        }
    }

    // ========== STEP 10: Graceful Shutdown ==========
    // Note: Proto doesn't define ReactiveLoopUnavailable signal — daemon-bus tracks liveness via health checks.

    // Stop gRPC server
    let _ = shutdown_tx.send(());

    if let Err(join_error) = grpc_handle.await {
        if !join_error.is_cancelled() {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "grpc_shutdown_error",
                error = %join_error,
                "error while shutting down gRPC server"
            );
        }
    }

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "shutdown_complete",
        "reactive-loop shutdown complete"
    );

    0 // Success
}

/// Initialize the tracing subscriber based on configuration.
fn initialize_tracing(logging_config: &config::LoggingConfig) {
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

/// Wait for DAEMON_BUS_READY signal from the event bus.
async fn wait_for_daemon_bus_ready(
    daemon_bus_address: &str,
    timeout: Duration,
) -> Result<(), ReactiveLoopError> {
    let mut event_client = EventBusServiceClient::connect(daemon_bus_address.to_owned())
        .await
        .map_err(|e| ReactiveLoopError::DaemonBusConnection {
            reason: format!("failed to connect event bus client: {}", e),
        })?;

    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics: vec![EventTopic::TopicBootSignal.into()],
        subscriber_id: SUBSYSTEM_ID.to_owned(),
    });

    let mut stream = event_client
        .subscribe(subscribe_request)
        .await
        .map_err(|e| ReactiveLoopError::DaemonBusConnection {
            reason: format!("failed to subscribe to boot signals: {}", e),
        })?
        .into_inner();

    let wait_future = async {
        loop {
            match stream.message().await {
                Ok(Some(bus_event)) => {
                    if bus_event.topic == i32::from(EventTopic::TopicBootSignal) {
                        if bus_event.source_subsystem == "daemon_bus" {
                            tracing::debug!(
                                subsystem = SUBSYSTEM_ID,
                                event_type = "daemon_bus_ready_signal",
                                "received DAEMON_BUS_READY signal"
                            );
                            return Ok(());
                        }
                    }
                }
                Ok(None) => {
                    return Err(ReactiveLoopError::DaemonBusConnection {
                        reason: "event stream ended before DAEMON_BUS_READY".into(),
                    });
                }
                Err(stream_error) => {
                    return Err(ReactiveLoopError::Grpc(format!(
                        "stream error: {}",
                        stream_error
                    )));
                }
            }
        }
    };

    tokio::time::timeout(timeout, wait_future)
        .await
        .map_err(|_| ReactiveLoopError::DaemonBusConnection {
            reason: format!("timed out waiting for DAEMON_BUS_READY after {:?}", timeout),
        })?
}
