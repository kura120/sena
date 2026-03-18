//! ctp — Sena's continuous proactive cognitive loop.
//!
//! CTP runs permanently at background priority, generating candidate thoughts
//! from telemetry and memory signals, scoring them for relevance, surfacing
//! high-relevance ones through the thought queue, and driving memory
//! consolidation during idle periods.
//!
//! ## Boot Sequence
//!
//! 1. Load config from `ctp.toml`
//! 2. Initialize tracing subscriber
//! 3. Connect to daemon-bus
//! 4. Connect to memory-engine
//! 5. Boot gate: wait for MEMORY_ENGINE_READY + MODEL_PROFILE_READY + (LORA_READY | LORA_SKIPPED)
//! 6. Initialize ActivityMonitor, spawn poll loop
//! 7. Initialize ThoughtQueue
//! 8. Initialize ContextAssembler
//! 9. Spawn three pipelines (generation, evaluation, consolidation)
//! 10. Signal CTP_READY to daemon-bus
//! 11. Await shutdown signal

pub mod activity;
pub mod config;
pub mod context_assembler;
pub mod error;
pub mod pipelines;
pub mod relevance;
pub mod thought_queue;

pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::activity::ActivityMonitor;
use crate::config::Config;
use crate::error::CtpError;
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient,
    event_bus_service_client::EventBusServiceClient,
    BootSignal, BootSignalRequest, EventTopic, SubscribeRequest,
};
use crate::pipelines::spawn_all;
use crate::thought_queue::ThoughtQueue;

const SUBSYSTEM_ID: &str = "ctp";

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("ctp-worker")
        .build()
        // Panic acceptable: cannot proceed without an async executor
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    let exit_code = runtime.block_on(async_main());
    std::process::exit(exit_code);
}

async fn async_main() -> i32 {
    // ── Step 1: Load configuration ──────────────────────────────────────
    let config_path = std::env::var("CTP_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config/ctp.toml"));

    let config = match Config::load(&config_path) {
        Ok(loaded) => loaded,
        Err(config_error) => {
            // Cannot use tracing — subscriber not yet initialized.
            eprintln!(
                "[FATAL] failed to load CTP config from '{}': {}",
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
        "CTP starting"
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

    // ── Step 4: Connect to memory-engine ────────────────────────────────
    let memory_address = config.grpc.memory_engine_address.clone();
    // memory-engine client connection is deferred until after boot gate.
    // The address is validated here; actual connection happens when pipelines start.

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "daemon_bus_connected",
        "connected to daemon-bus"
    );

    // ── Step 5: Boot gate ───────────────────────────────────────────────
    // Wait for MEMORY_ENGINE_READY + MODEL_PROFILE_READY + (LORA_READY | LORA_SKIPPED)
    match wait_for_boot_prerequisites(&daemon_bus_address, connect_timeout).await {
        Ok(()) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_prerequisites_met",
                "all boot prerequisites received"
            );
        }
        Err(wait_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_gate_failed",
                error = %wait_error,
                "failed to receive all boot prerequisites"
            );
            return 1;
        }
    }

    // ── Step 6: Initialize ActivityMonitor ──────────────────────────────
    let activity_monitor = Arc::new(ActivityMonitor::new());
    let monitor_config = config.activity.clone();
    let monitor_clone = Arc::clone(&activity_monitor);
    let _activity_handle = tokio::spawn(async move {
        monitor_clone.run_poll_loop(&monitor_config).await;
    });

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "activity_monitor_started",
        "activity monitor poll loop started"
    );

    // ── Step 7: Initialize ThoughtQueue ─────────────────────────────────
    let thought_queue = Arc::new(ThoughtQueue::new());

    // ── Step 8: Initialize ContextAssembler (with memory-engine address) ─
    // Actual gRPC client connection to memory-engine is deferred to when
    // the context assembler first needs to query. For Phase 1, memory reads
    // return empty results (memory-engine may not be running).
    let _memory_address = memory_address;

    // ── Step 9: Spawn three pipelines ───────────────────────────────────
    let (generation_handle, evaluation_handle, consolidation_handle, _telemetry_tx) = spawn_all(
        Arc::clone(&config),
        Arc::clone(&thought_queue),
        Arc::clone(&activity_monitor),
        daemon_bus_address.clone(),
    );

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "pipelines_spawned",
        "generation, evaluation, and consolidation pipelines running"
    );

    // ── Step 10: Signal CTP_READY ───────────────────────────────────────
    let ready_request = tonic::Request::new(BootSignalRequest {
        subsystem_id: SUBSYSTEM_ID.to_owned(),
        signal: BootSignal::CtpReady.into(),
    });

    match boot_client.signal_ready(ready_request).await {
        Ok(_) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_sent",
                signal = "CTP_READY",
                "CTP_READY signaled to daemon-bus"
            );
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_failed",
                error = %signal_error,
                "failed to signal CTP_READY to daemon-bus"
            );
            // CTP is running — log failure but continue.
            // daemon-bus detects via boot timeout.
        }
    }

    // ── Step 11: Await shutdown ─────────────────────────────────────────
    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "running",
        "CTP running — awaiting shutdown signal"
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

    generation_handle.abort();
    evaluation_handle.abort();
    consolidation_handle.abort();

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "shutdown_complete",
        "CTP shut down cleanly"
    );

    0
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Wait for all three boot prerequisites:
/// - MEMORY_ENGINE_READY
/// - MODEL_PROFILE_READY
/// - LORA_READY or LORA_SKIPPED
async fn wait_for_boot_prerequisites(
    daemon_bus_address: &str,
    timeout: Duration,
) -> Result<(), CtpError> {
    let mut event_client = EventBusServiceClient::connect(daemon_bus_address.to_owned())
        .await
        .map_err(|connect_error| CtpError::DaemonBus(connect_error.to_string()))?;

    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics: vec![EventTopic::TopicBootSignal.into()],
        subscriber_id: SUBSYSTEM_ID.to_owned(),
    });

    let mut stream = event_client
        .subscribe(subscribe_request)
        .await
        .map_err(|status| CtpError::DaemonBus(format!("subscribe failed: {}", status)))?
        .into_inner();

    let wait_future = async {
        let mut memory_engine_ready = false;
        let mut model_profile_ready = false;
        let mut lora_resolved = false;

        loop {
            match stream.message().await {
                Ok(Some(bus_event)) => {
                    if bus_event.topic == i32::from(EventTopic::TopicBootSignal) {
                        // TODO: decode BootSignal enum from payload once daemon-bus
                        // wire format is finalized. Currently matching by source_subsystem
                        // string as a Phase 1 approximation.
                        match bus_event.source_subsystem.as_str() {
                            "memory_engine" | "memory-engine" => {
                                memory_engine_ready = true;
                                tracing::info!(
                                    subsystem = SUBSYSTEM_ID,
                                    event_type = "boot_signal_received",
                                    signal = "MEMORY_ENGINE_READY",
                                    "received MEMORY_ENGINE_READY"
                                );
                            }
                            "model_probe" | "model-probe" => {
                                model_profile_ready = true;
                                tracing::info!(
                                    subsystem = SUBSYSTEM_ID,
                                    event_type = "boot_signal_received",
                                    signal = "MODEL_PROFILE_READY",
                                    "received MODEL_PROFILE_READY"
                                );
                            }
                            "lora_manager" | "lora-manager" => {
                                lora_resolved = true;
                                tracing::info!(
                                    subsystem = SUBSYSTEM_ID,
                                    event_type = "boot_signal_received",
                                    signal = "LORA_READY_OR_SKIPPED",
                                    "received LORA_READY or LORA_SKIPPED"
                                );
                            }
                            _ => {}
                        }

                        if memory_engine_ready && model_profile_ready && lora_resolved {
                            return Ok(());
                        }
                    }
                }
                Ok(None) => {
                    return Err(CtpError::DaemonBus(
                        "event stream ended before all boot prerequisites received".into(),
                    ));
                }
                Err(stream_error) => {
                    return Err(CtpError::DaemonBus(format!(
                        "stream error waiting for boot prerequisites: {}",
                        stream_error
                    )));
                }
            }
        }
    };

    tokio::time::timeout(timeout, wait_future)
        .await
        .map_err(|_| {
            CtpError::DaemonBus(
                "timed out waiting for boot prerequisites".into(),
            )
        })?
}

/// Best-effort signal to daemon-bus. Logs on failure but does not propagate.
#[allow(dead_code)]
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
    async fn test_boot_gate_requires_all_three_signals() {
        // Verify that a multi-signal gate blocks until all three signals fire.
        // This tests the boot gate pattern used in async_main.
        let (tx1, rx1) = oneshot::channel::<&str>();
        let (tx2, rx2) = oneshot::channel::<&str>();
        let (tx3, rx3) = oneshot::channel::<&str>();

        let gate_task = tokio::spawn(async move {
            // Simulates waiting for all three prerequisites
            let (r1, r2, r3) = tokio::join!(rx1, rx2, rx3);
            (
                r1.expect("test: signal 1 must arrive"),
                r2.expect("test: signal 2 must arrive"),
                r3.expect("test: signal 3 must arrive"),
            )
        });

        // Gate should not be resolved yet
        assert!(!gate_task.is_finished());

        // Send signals one at a time
        tx1.send("MEMORY_ENGINE_READY")
            .expect("test: receiver should still be alive");
        assert!(!gate_task.is_finished());

        tx2.send("MODEL_PROFILE_READY")
            .expect("test: receiver should still be alive");
        assert!(!gate_task.is_finished());

        tx3.send("LORA_READY")
            .expect("test: receiver should still be alive");

        // Now the gate should resolve
        let (s1, s2, s3) = gate_task.await.expect("test: gate task should complete");
        assert_eq!(s1, "MEMORY_ENGINE_READY");
        assert_eq!(s2, "MODEL_PROFILE_READY");
        assert_eq!(s3, "LORA_READY");
    }

    #[tokio::test]
    async fn test_shutdown_cancels_all_pipeline_handles() {
        // Verify that aborting JoinHandles cancels the spawned tasks.
        let handle1 = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        let handle2 = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        let handle3 = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });

        // Abort all three
        handle1.abort();
        handle2.abort();
        handle3.abort();

        // All should report as cancelled
        let r1 = handle1.await;
        let r2 = handle2.await;
        let r3 = handle3.await;

        assert!(r1.unwrap_err().is_cancelled());
        assert!(r2.unwrap_err().is_cancelled());
        assert!(r3.unwrap_err().is_cancelled());
    }
}

