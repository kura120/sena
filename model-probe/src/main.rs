//! model-probe — Sena's runtime model capability detection subsystem.
//!
//! Runs once at boot (step 5.5 — after the model is loaded, before CTP starts).
//! Detects the active model's capabilities via a battery of lightweight probes,
//! detects hardware capabilities, publishes `ModelCapabilityProfile` and
//! `HardwareProfile` to daemon-bus via gRPC, signals `MODEL_PROFILE_READY`,
//! and exits cleanly.
//!
//! model-probe is stateless — it holds no data between runs.

// ─────────────────────────────────────────────────────────────────────────────
// Module declarations — named modules only, no mod.rs
// ─────────────────────────────────────────────────────────────────────────────

pub mod capabilities;
pub mod config;
pub mod error;
pub mod hardware;
pub mod probes;

/// Individual probe implementations.
pub mod probe {
    pub mod context_window;
    pub mod graph_extraction;
    pub mod instruction_following;
    pub mod lora_compat;
    pub mod memory_fidelity;
    pub mod reasoning;
    pub mod structured_output;
}

/// Proto-generated types for daemon-bus gRPC communication.
/// In a full build, `tonic-build` overwrites `src/generated/sena.daemonbus.v1.rs`
/// from the proto definition. The placeholder file committed to the repo keeps
/// the crate compilable before the first `cargo build` runs codegen.
pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

use std::path::PathBuf;
use std::time::Duration;

use crate::config::ModelProbeConfig;
use crate::error::{ErrorCode, SenaError};
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient,
    event_bus_service_client::EventBusServiceClient,
    inference_service_client::InferenceServiceClient,
    BootSignal, BootSignalRequest, BusEvent, EventTopic, PublishRequest,
};

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    // Build the tokio runtime explicitly so construction failures are caught
    // before any async work begins.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("model-probe-worker")
        .build()
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    let exit_code = runtime.block_on(async_main());
    std::process::exit(exit_code);
}

async fn async_main() -> i32 {
    // ── Step 1: Load configuration ──────────────────────────────────────
    let config_path = std::env::var("MODEL_PROBE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config/model-probe.toml"));

    let config = match ModelProbeConfig::load(&config_path).await {
        Ok(loaded_config) => loaded_config,
        Err(config_error) => {
            // Cannot use tracing yet — subscriber is not initialized.
            // This is the one place where eprintln is acceptable.
            eprintln!(
                "[FATAL] failed to load model-probe config from '{}': {}",
                config_path.display(),
                config_error
            );
            if let Some(ref debug_ctx) = config_error.debug_context {
                eprintln!("[FATAL] debug context: {}", debug_ctx);
            }
            return 1;
        }
    };

    // ── Step 2: Initialize structured logging ───────────────────────────
    initialize_tracing(&config.logging);

    tracing::info!(
        subsystem = "model_probe",
        event_type = "startup",
        config_path = %config_path.display(),
        daemon_bus_address = %config.grpc.daemon_bus_address,
        "model-probe starting"
    );

    // ── Step 3: Detect hardware capabilities ────────────────────────────
    let hardware_profile = match hardware::detect_hardware(&config.hardware).await {
        Ok(profile) => profile,
        Err(hardware_error) => {
            tracing::error!(
                subsystem = "model_probe",
                event_type = "hardware_detection_failed",
                error_code = %hardware_error.code,
                error_message = %hardware_error.message,
                "hardware detection failed — cannot build profile"
            );
            // Hardware detection failure is not fatal for the probe battery,
            // but we still need a profile. Build a minimal one.
            hardware::HardwareProfile {
                vram_total_mb: 0,
                vram_available_mb: 0,
                ram_total_mb: 0,
                cuda_compute: String::new(),
                tier: hardware::HardwareTier::Low,
            }
        }
    };

    tracing::info!(
        subsystem = "model_probe",
        event_type = "hardware_profile_ready",
        tier = %hardware_profile.tier,
        vram_total_mb = hardware_profile.vram_total_mb,
        vram_available_mb = hardware_profile.vram_available_mb,
        ram_total_mb = hardware_profile.ram_total_mb,
        cuda_compute = %hardware_profile.cuda_compute,
        "hardware profile detected"
    );

    // ── Step 4: Connect to daemon-bus gRPC ──────────────────────────────
    let daemon_bus_address = config.grpc.daemon_bus_address.clone();
    let connect_timeout = Duration::from_millis(config.grpc.connect_timeout_ms);

    let endpoint = match tonic::transport::Endpoint::from_shared(daemon_bus_address.clone()) {
        Ok(ep) => ep,
        Err(invalid_uri_error) => {
            tracing::error!(
                subsystem = "model_probe",
                event_type = "grpc_invalid_uri",
                address = %daemon_bus_address,
                error = %invalid_uri_error,
                "invalid daemon-bus gRPC address"
            );
            // Cannot connect — run probes anyway but skip publishing
            return run_probes_without_grpc(&config, &hardware_profile).await;
        }
    };

    let grpc_channel = match tokio::time::timeout(
        connect_timeout,
        endpoint.connect(),
    )
    .await
    {
        Ok(Ok(channel)) => {
            tracing::info!(
                subsystem = "model_probe",
                event_type = "grpc_connected",
                address = %daemon_bus_address,
                "connected to daemon-bus gRPC"
            );
            Some(channel)
        }
        Ok(Err(transport_error)) => {
            tracing::error!(
                subsystem = "model_probe",
                event_type = "grpc_connection_failed",
                address = %daemon_bus_address,
                error = %transport_error,
                "failed to connect to daemon-bus — will run probes but cannot publish results"
            );
            None
        }
        Err(_timeout) => {
            tracing::error!(
                subsystem = "model_probe",
                event_type = "grpc_connection_timeout",
                address = %daemon_bus_address,
                timeout_ms = config.grpc.connect_timeout_ms,
                "daemon-bus gRPC connection timed out — will run probes but cannot publish results"
            );
            None
        }
    };

    // ── Step 5: Derive model_id from config ─────────────────────────────
    //
    // The model_id is derived from the model file path. In a full implementation
    // this would come from model metadata extracted via llama-cpp-rs.
    let model_id = derive_model_id(&config.model.model_path);

    tracing::info!(
        subsystem = "model_probe",
        event_type = "model_identified",
        model_id = %model_id,
        model_path = %config.model.model_path,
        "model identified for probing"
    );

    // ── Step 6: Run the probe battery ───────────────────────────────────
    //
    // last_lora_training_score is None on first boot — no prior LoRA training
    // has occurred. In a full implementation this would be fetched from
    // memory-engine via gRPC.
    let last_lora_training_score: Option<f64> = None;

    // Create InferenceService client if gRPC channel is available
    let inference_client = grpc_channel.as_ref().map(|channel| {
        InferenceServiceClient::new(channel.clone())
    });

    let battery_result = match probes::run_probe_battery(
        &config,
        &model_id,
        last_lora_training_score,
        inference_client,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(battery_error) => {
            tracing::error!(
                subsystem = "model_probe",
                event_type = "probe_battery_failed",
                error_code = %battery_error.code,
                error_message = %battery_error.message,
                "probe battery failed — publishing failure event and exiting"
            );

            // Publish MODEL_PROBE_FAILED to daemon-bus if connected
            if let Some(channel) = grpc_channel {
                let publish_result = publish_probe_failed_event(
                    channel.clone(),
                    &model_id,
                    &battery_error.message,
                )
                .await;

                if let Err(publish_error) = publish_result {
                    tracing::error!(
                        subsystem = "model_probe",
                        event_type = "publish_failed",
                        error_code = %publish_error.code,
                        error_message = %publish_error.message,
                        "failed to publish MODEL_PROBE_FAILED event"
                    );
                }
            }

            return 1;
        }
    };

    tracing::info!(
        subsystem = "model_probe",
        event_type = "probe_battery_complete",
        model_id = %model_id,
        total_duration_ms = battery_result.total_duration_ms,
        lora_training_recommended = battery_result.lora_training_recommended,
        "probe battery completed successfully"
    );

    // ── Step 7: Publish profiles and signals to daemon-bus ──────────────
    if let Some(channel) = grpc_channel {
        // Publish LORA_TRAINING_RECOMMENDED if reasoning gap detection triggered it
        if battery_result.lora_training_recommended {
            let lora_publish_result = publish_lora_training_recommended(
                channel.clone(),
                &model_id,
                &battery_result.model_profile,
            )
            .await;

            if let Err(publish_error) = lora_publish_result {
                tracing::error!(
                    subsystem = "model_probe",
                    event_type = "publish_failed",
                    error_code = %publish_error.code,
                    error_message = %publish_error.message,
                    "failed to publish LORA_TRAINING_RECOMMENDED event"
                );
                // Non-fatal — continue to signal MODEL_PROFILE_READY
            }
        }

        // Signal MODEL_PROFILE_READY — this unblocks CTP in the boot sequence.
        //
        // The profile payload is serialized as JSON bytes in the BusEvent payload.
        // Receivers (PC, CTP, agents) deserialize based on topic contract.
        let signal_result = signal_model_profile_ready(
            channel.clone(),
            &model_id,
            &battery_result.model_profile,
            &hardware_profile,
        )
        .await;

        match signal_result {
            Ok(()) => {
                tracing::info!(
                    subsystem = "model_probe",
                    event_type = "model_profile_ready_signaled",
                    model_id = %model_id,
                    "MODEL_PROFILE_READY signaled to daemon-bus — boot can proceed"
                );
            }
            Err(signal_error) => {
                tracing::error!(
                    subsystem = "model_probe",
                    event_type = "signal_failed",
                    error_code = %signal_error.code,
                    error_message = %signal_error.message,
                    "failed to signal MODEL_PROFILE_READY — boot may stall"
                );
                return 1;
            }
        }
    } else {
        tracing::warn!(
            subsystem = "model_probe",
            event_type = "no_grpc_connection",
            "no gRPC connection to daemon-bus — profiles computed but not published"
        );
    }

    // ── Step 8: Exit cleanly ────────────────────────────────────────────
    tracing::info!(
        subsystem = "model_probe",
        event_type = "shutdown",
        "model-probe completed successfully — exiting"
    );

    0
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: run probes without gRPC (fallback when connection fails at URI level)
// ─────────────────────────────────────────────────────────────────────────────

/// Run the probe battery and log results even when daemon-bus is unreachable.
///
/// This path is only taken when the daemon-bus address itself is invalid (URI
/// parse failure). The probes still run so local logs capture the profile, but
/// no boot signal is published, which means daemon-bus will eventually time out
/// this subsystem and Sena starts in minimal capability mode.
async fn run_probes_without_grpc(
    config: &ModelProbeConfig,
    _hardware_profile: &hardware::HardwareProfile,
) -> i32 {
    let model_id = derive_model_id(&config.model.model_path);

    let battery_result = probes::run_probe_battery(config, &model_id, None, None).await;

    match battery_result {
        Ok(outcome) => {
            tracing::warn!(
                subsystem = "model_probe",
                event_type = "probe_battery_complete_no_grpc",
                model_id = %model_id,
                total_duration_ms = outcome.total_duration_ms,
                "probe battery completed but cannot publish — no gRPC connection"
            );
            // Exit with success code — probes ran, results are in logs.
            // daemon-bus will handle the missing boot signal via its timeout.
            0
        }
        Err(battery_error) => {
            tracing::error!(
                subsystem = "model_probe",
                event_type = "probe_battery_failed",
                error_code = %battery_error.code,
                error_message = %battery_error.message,
                "probe battery failed and no gRPC connection to report failure"
            );
            1
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: derive model_id from model file path
// ─────────────────────────────────────────────────────────────────────────────

/// Derive a model identifier from the model file path.
///
/// Strips the directory and file extension to produce a human-readable ID.
/// In a full implementation this would come from GGUF metadata via llama-cpp-rs.
fn derive_model_id(model_path: &str) -> String {
    std::path::Path::new(model_path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown-model".to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: signal MODEL_PROFILE_READY to daemon-bus
// ─────────────────────────────────────────────────────────────────────────────

/// Signal MODEL_PROFILE_READY to daemon-bus via BootService.SignalReady,
/// then publish the full profile payload via EventBusService.Publish.
async fn signal_model_profile_ready(
    channel: tonic::transport::Channel,
    model_id: &str,
    model_profile: &probes::ModelCapabilityProfile,
    hardware_profile: &hardware::HardwareProfile,
) -> error::SenaResult<()> {
    // Serialize the combined payload — both profiles together so downstream
    // consumers get everything in one event.
    let payload = CombinedProfilePayload {
        model_profile: model_profile.clone(),
        hardware_profile: hardware_profile.clone(),
    };
    let payload_bytes = serde_json::to_vec(&payload)?;

    // Publish the profile payload on the event bus first, so subscribers
    // can read it as soon as they see the boot signal.
    let mut event_bus_client = EventBusServiceClient::new(channel.clone());

    let bus_event = BusEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        topic: EventTopic::TopicBootSignal.into(),
        source_subsystem: "model-probe".to_string(),
        payload: payload_bytes,
        trace_context: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let publish_response = event_bus_client
        .publish(PublishRequest {
            event: Some(bus_event),
        })
        .await
        .map_err(|status| {
            SenaError::new(
                ErrorCode::DaemonBusPublishFailed,
                "failed to publish profile payload to event bus",
            )
            .with_debug_context(format!("gRPC status: {status}"))
        })?;

    if !publish_response.into_inner().accepted {
        return Err(SenaError::new(
            ErrorCode::DaemonBusPublishFailed,
            "daemon-bus rejected profile payload publish",
        ));
    }

    // Signal boot readiness via BootService
    let mut boot_client = BootServiceClient::new(channel);

    let signal_response = boot_client
        .signal_ready(BootSignalRequest {
            subsystem_id: "model-probe".to_string(),
            signal: BootSignal::ModelProfileReady.into(),
            capabilities: capabilities::get_capabilities(&model_profile.degraded_probes),
        })
        .await
        .map_err(|status| {
            SenaError::new(
                ErrorCode::DaemonBusPublishFailed,
                "failed to signal MODEL_PROFILE_READY via BootService",
            )
            .with_debug_context(format!("gRPC status: {status}"))
        })?;

    if !signal_response.into_inner().acknowledged {
        return Err(SenaError::new(
            ErrorCode::DaemonBusPublishFailed,
            "daemon-bus did not acknowledge MODEL_PROFILE_READY signal",
        ));
    }

    tracing::info!(
        subsystem = "model_probe",
        event_type = "boot_signal_sent",
        model_id = model_id,
        signal = "MODEL_PROFILE_READY",
        "boot readiness signal acknowledged by daemon-bus"
    );

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: publish LORA_TRAINING_RECOMMENDED
// ─────────────────────────────────────────────────────────────────────────────

/// Publish LORA_TRAINING_RECOMMENDED event to daemon-bus when reasoning gap
/// detection exceeds the configured threshold and the model is LoRA-compatible.
async fn publish_lora_training_recommended(
    channel: tonic::transport::Channel,
    model_id: &str,
    model_profile: &probes::ModelCapabilityProfile,
) -> error::SenaResult<()> {
    let mut event_bus_client = EventBusServiceClient::new(channel);

    let payload = serde_json::json!({
        "model_id": model_id,
        "reasoning_quality": model_profile.reasoning_quality,
        "lora_compatible": model_profile.lora_compatible,
    });
    let payload_bytes = serde_json::to_vec(&payload)?;

    let bus_event = BusEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        topic: EventTopic::TopicLoraTrainingRecommended.into(),
        source_subsystem: "model-probe".to_string(),
        payload: payload_bytes,
        trace_context: String::new(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let publish_response = event_bus_client
        .publish(PublishRequest {
            event: Some(bus_event),
        })
        .await
        .map_err(|status| {
            SenaError::new(
                ErrorCode::DaemonBusPublishFailed,
                "failed to publish LORA_TRAINING_RECOMMENDED event",
            )
            .with_debug_context(format!("gRPC status: {status}"))
        })?;

    if !publish_response.into_inner().accepted {
        return Err(SenaError::new(
            ErrorCode::DaemonBusPublishFailed,
            "daemon-bus rejected LORA_TRAINING_RECOMMENDED publish",
        ));
    }

    tracing::info!(
        subsystem = "model_probe",
        event_type = "lora_training_recommended_published",
        model_id = model_id,
        reasoning_quality = model_profile.reasoning_quality,
        "LORA_TRAINING_RECOMMENDED event published to daemon-bus"
    );

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: publish MODEL_PROBE_FAILED
// ─────────────────────────────────────────────────────────────────────────────

/// Publish MODEL_PROBE_FAILED event to daemon-bus when the probe battery
/// fails in an unrecoverable way.
async fn publish_probe_failed_event(
    channel: tonic::transport::Channel,
    model_id: &str,
    reason: &str,
) -> error::SenaResult<()> {
    let mut event_bus_client = EventBusServiceClient::new(channel);

    let payload = serde_json::json!({
        "model_id": model_id,
        "reason": reason,
    });
    let payload_bytes = serde_json::to_vec(&payload)?;

    let bus_event = BusEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        topic: EventTopic::TopicModelProbeFailed.into(),
        source_subsystem: "model-probe".to_string(),
        payload: payload_bytes,
        trace_context: String::new(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let publish_response = event_bus_client
        .publish(PublishRequest {
            event: Some(bus_event),
        })
        .await
        .map_err(|status| {
            SenaError::new(
                ErrorCode::DaemonBusPublishFailed,
                "failed to publish MODEL_PROBE_FAILED event",
            )
            .with_debug_context(format!("gRPC status: {status}"))
        })?;

    if !publish_response.into_inner().accepted {
        return Err(SenaError::new(
            ErrorCode::DaemonBusPublishFailed,
            "daemon-bus rejected MODEL_PROBE_FAILED publish",
        ));
    }

    tracing::info!(
        subsystem = "model_probe",
        event_type = "probe_failed_published",
        model_id = model_id,
        reason = reason,
        "MODEL_PROBE_FAILED event published to daemon-bus"
    );

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Combined payload for MODEL_PROFILE_READY event
// ─────────────────────────────────────────────────────────────────────────────

/// Combined payload serialized into the BusEvent for MODEL_PROFILE_READY.
/// Downstream consumers (PC, CTP, agents) deserialize this from the event payload.
#[derive(serde::Serialize, serde::Deserialize)]
struct CombinedProfilePayload {
    model_profile: probes::ModelCapabilityProfile,
    hardware_profile: hardware::HardwareProfile,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tracing initialization
// ─────────────────────────────────────────────────────────────────────────────

/// Initialize the `tracing` subscriber based on config.
///
/// Uses `tracing-subscriber` with either a JSON layer (for production) or a
/// pretty-printed layer (for development). The log level filter is set from
/// config.
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

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_model_id_from_path_with_extension() {
        let model_id = derive_model_id("models/llama-3-8b-q4_K_M.gguf");
        assert_eq!(model_id, "llama-3-8b-q4_K_M");
    }

    #[test]
    fn derive_model_id_from_path_without_extension() {
        let model_id = derive_model_id("models/my-model");
        assert_eq!(model_id, "my-model");
    }

    #[test]
    fn derive_model_id_from_bare_filename() {
        let model_id = derive_model_id("default.gguf");
        assert_eq!(model_id, "default");
    }

    #[test]
    fn derive_model_id_empty_path_returns_unknown() {
        let model_id = derive_model_id("");
        assert_eq!(model_id, "unknown-model");
    }

    #[test]
    fn derive_model_id_from_deeply_nested_path() {
        let model_id = derive_model_id("/opt/models/v2/quantized/model-7b.gguf");
        assert_eq!(model_id, "model-7b");
    }

    #[test]
    fn derive_model_id_windows_path() {
        let model_id = derive_model_id("C:\\models\\my-model.gguf");
        assert_eq!(model_id, "my-model");
    }

    #[test]
    fn combined_profile_payload_serialization_round_trip() {
        let payload = CombinedProfilePayload {
            model_profile: probes::ModelCapabilityProfile {
                model_id: "test-model".to_string(),
                context_window: 8192,
                pre_rot_threshold: 5529,
                structured_output: probes::CapabilityLevel::Full,
                instruction_following: probes::CapabilityLevel::Partial,
                reasoning_quality: 0.75,
                lora_compatible: true,
                memory_injection_fidelity: 0.85,
                graph_extraction: probes::CapabilityLevel::None,
                lora_training_recommended: false,
                degraded_probes: vec![],
            },
            hardware_profile: hardware::HardwareProfile {
                vram_total_mb: 16384,
                vram_available_mb: 12000,
                ram_total_mb: 32768,
                cuda_compute: "8.6".to_string(),
                tier: hardware::HardwareTier::High,
            },
        };

        let json_bytes = serde_json::to_vec(&payload)
            .expect("CombinedProfilePayload should serialize to JSON");

        let deserialized: CombinedProfilePayload = serde_json::from_slice(&json_bytes)
            .expect("CombinedProfilePayload should deserialize from JSON");

        assert_eq!(deserialized.model_profile.model_id, "test-model");
        assert_eq!(deserialized.hardware_profile.vram_total_mb, 16384);
        assert_eq!(deserialized.hardware_profile.tier, hardware::HardwareTier::High);
    }
}
