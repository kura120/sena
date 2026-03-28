//! memory-engine — Sena's memory subsystem process.
//!
//! This is the process entry point. The boot sequence is ordered and any
//! failure in steps 1–8 is fatal: log the error code + subsystem name,
//! signal failure to daemon-bus (best-effort), and exit with a non-zero code.
//!
//! ## Boot Sequence
//!
//! 1. Load config from `memory-engine.toml`
//! 2. Initialize tracing subscriber
//! 3. Construct `LlamaEmbedder` and `DegradedExtractor`
//! 4. Initialize ech0 `Store` with `ProfileDerivedConfig::without_profile()`
//!    (conservative defaults — does not block on model-probe)
//! 5. Initialize `MemoryEngine`
//! 6. Start gRPC server (`MemoryService`)
//! 7. Signal `MEMORY_ENGINE_READY` to daemon-bus
//! 8. Subscribe to `MODEL_PROFILE_READY` event in background — store
//!    reconfiguration from the real profile is deferred to Phase 2
//! 9. Await shutdown signal from daemon-bus
//!
//! Steps 3–7 must not block on model-probe or inference. Per PRD §3.2,
//! memory-engine depends only on `DAEMON_BUS_READY` and must signal its own
//! readiness before model-probe can start.
//!
//! No `println!` or `eprintln!` except for the single pre-tracing fatal path
//! where the config file cannot be loaded and tracing is not yet initialized.

// ─────────────────────────────────────────────────────────────────────────────
// Module declarations — named modules only, no mod.rs
// ─────────────────────────────────────────────────────────────────────────────

pub mod capabilities;
pub mod config;
pub mod embedder;
pub mod engine;
pub mod error;
pub mod extractor;
pub mod grpc;
pub mod profile;
pub mod queue;
pub mod tier;

/// Proto-generated types for daemon-bus gRPC communication.
/// In a full build, `tonic-build` overwrites `src/generated/sena.daemonbus.v1.rs`
/// from the proto definition. The placeholder file committed to the repo keeps
/// the crate compilable before the first `cargo build` runs codegen.
pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::embedder::LlamaEmbedder;
use crate::engine::{DaemonBusClient, MemoryEngine};
use crate::error::{ErrorCode, SenaError};
use crate::extractor::{DegradedExtractor, LlamaExtractor};
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient, event_bus_service_client::EventBusServiceClient,
    BootSignal, BootSignalRequest, BusEvent, EventTopic, SubscribeRequest,
};
use crate::profile::ModelCapabilityProfile;
use crate::queue::WriteQueue;
use crate::tier::{EpisodicTier, LongTermTier, ShortTermTier};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

const SUBSYSTEM_ID: &str = "memory_engine";

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    // Build the tokio runtime explicitly so construction failures are caught
    // before any async work begins.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("memory-engine-worker")
        .build()
        .expect("failed to build tokio runtime — cannot proceed without an async executor");

    let exit_code = runtime.block_on(async_main());
    std::process::exit(exit_code);
}

async fn async_main() -> i32 {
    // ── Step 1: Load configuration ──────────────────────────────────────
    let config_path = std::env::var("MEMORY_ENGINE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config/memory-engine.toml"));

    let config = match Config::load(&config_path) {
        Ok(loaded_config) => loaded_config,
        Err(config_error) => {
            // Cannot use tracing yet — subscriber is not initialized.
            // This is the one place where eprintln is acceptable: tracing
            // is not available and the process must communicate the failure.
            eprintln!(
                "[FATAL] failed to load memory-engine config from '{}': {}",
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
        subsystem = SUBSYSTEM_ID,
        event_type = "startup",
        config_path = %config_path.display(),
        daemon_bus_address = %config.grpc.daemon_bus_address,
        "memory-engine starting"
    );

    let config = Arc::new(config);

    // ── Step 3: Derive boot-time ProfileDerivedConfig ────────────────────
    //
    // PRD §3.2: memory-engine depends only on DAEMON_BUS_READY. It must not
    // block on ModelCapabilityProfile — that is published by model-probe,
    // which starts after inference, which starts after memory-engine is
    // already ready. Use conservative defaults for boot; the real profile
    // arrives via a background subscriber after we signal readiness.
    let profile_derived = profile::ProfileDerivedConfig::without_profile();

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        component = "profile",
        "booting with default ProfileDerivedConfig — ModelCapabilityProfile will arrive after MEMORY_ENGINE_READY"
    );

    // ── Step 3: Construct Embedder and Extractor ────────────────────────
    //
    // LlamaBackend must be initialized once and shared. Model loading is
    // blocking I/O — run in spawn_blocking to avoid stalling the async runtime.

    // Initialize the llama backend. This must happen once per process.
    let llama_backend = match llama_cpp_2::llama_backend::LlamaBackend::init() {
        Ok(backend) => Arc::new(backend),
        Err(backend_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                error_code = %ErrorCode::EmbedderFailure,
                error = %backend_error,
                "failed to initialize LlamaBackend"
            );
            best_effort_signal_failure(&config).await;
            return 1;
        }
    };

    let embedding_dimensions: usize = config.embedder.embedding_dim;

    let embedder_config = config.embedder.clone();
    let embedder_backend = Arc::clone(&llama_backend);
    let embedder_result = tokio::task::spawn_blocking(move || {
        LlamaEmbedder::new(&embedder_config, embedder_backend, embedding_dimensions)
    })
    .await;

    let embedder = match embedder_result {
        Ok(Ok(llama_embedder)) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                component = "embedder",
                "LlamaEmbedder initialized"
            );
            llama_embedder
        }
        Ok(Err(echo_error)) => {
            let sena_error: SenaError = echo_error.into();
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                error_code = %sena_error.code,
                "failed to initialize LlamaEmbedder"
            );
            best_effort_signal_failure(&config).await;
            return 1;
        }
        Err(join_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                error_code = %ErrorCode::EmbedderFailure,
                error = %join_error,
                "embedder initialization task panicked"
            );
            best_effort_signal_failure(&config).await;
            return 1;
        }
    };

    // Branch on degraded extractor vs full LlamaExtractor. Because Store<E, X>
    // is generic, we must pick a concrete type at compile time. We use a helper
    // function for each branch to keep the code clean.
    if profile_derived.degraded_extractor {
        tracing::warn!(
            subsystem = SUBSYSTEM_ID,
            component = "extractor",
            "using DegradedExtractor — model cannot produce structured output"
        );
        let extractor = DegradedExtractor::new();
        run_with_store(
            config,
            profile_derived,
            embedder,
            extractor,
            embedding_dimensions,
        )
        .await
    } else {
        let extractor_config = config.extractor.clone();
        let extractor_backend = Arc::clone(&llama_backend);
        let extractor_result = tokio::task::spawn_blocking(move || {
            LlamaExtractor::new(&extractor_config, extractor_backend)
        })
        .await;

        let extractor = match extractor_result {
            Ok(Ok(llama_extractor)) => {
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    component = "extractor",
                    "LlamaExtractor initialized"
                );
                llama_extractor
            }
            Ok(Err(echo_error)) => {
                let sena_error: SenaError = echo_error.into();
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    error_code = %sena_error.code,
                    "failed to initialize LlamaExtractor"
                );
                best_effort_signal_failure(&config).await;
                return 1;
            }
            Err(join_error) => {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    error_code = %ErrorCode::ExtractorFailure,
                    error = %join_error,
                    "extractor initialization task panicked"
                );
                best_effort_signal_failure(&config).await;
                return 1;
            }
        };
        run_with_store(
            config,
            profile_derived,
            embedder,
            extractor,
            embedding_dimensions,
        )
        .await
    }
}

/// Continue the boot sequence from step 4 onward with concrete Embedder and
/// Extractor types. This avoids dynamic dispatch (`Box<dyn>`) which ech0's
/// Store does not support.
async fn run_with_store<E, X>(
    config: Arc<Config>,
    profile_derived: profile::ProfileDerivedConfig,
    embedder: E,
    extractor: X,
    vector_dimensions: usize,
) -> i32
where
    E: ech0::Embedder + 'static,
    X: ech0::Extractor + 'static,
{
    // ── Step 4: Initialize ech0 Store ───────────────────────────────────
    let store_config = build_store_config(&profile_derived, &config, vector_dimensions);

    let store = match ech0::Store::new(store_config, embedder, extractor).await {
        Ok(initialized_store) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                component = "store",
                "ech0 Store initialized"
            );
            Arc::new(initialized_store)
        }
        Err(echo_error) => {
            let sena_error: SenaError = echo_error.into();

            // Surface a specific, actionable message when the redb file lock
            // is held by a previous memory-engine process that was not fully
            // shut down. The generic "failed to initialize ech0 Store" message
            // does not tell the operator what to do.
            let debug_str = sena_error
                .debug_context
                .as_ref()
                .map(|ctx| ctx.detail.as_str())
                .unwrap_or("");
            let is_lock_conflict = debug_str.contains("already open")
                || debug_str.contains("Cannot acquire lock")
                || sena_error.message.contains("already open")
                || sena_error.message.contains("Cannot acquire lock");

            if is_lock_conflict {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    component = "store",
                    error_code = %sena_error.code,
                    db_path = %config.store.graph_path,
                    "FATAL: redb database lock held by another process \
                     — ensure no other memory-engine instance is running"
                );
            } else {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    error_code = %sena_error.code,
                    error_message = %sena_error.message,
                    debug_context = ?sena_error.debug_context,
                    "failed to initialize ech0 Store"
                );
            }

            best_effort_signal_failure(&config).await;
            return 1;
        }
    };

    // ── Step 5: Initialize MemoryEngine ─────────────────────────────────
    let write_queue = Arc::new(WriteQueue::new(Arc::clone(&store), config.queue.clone()));

    let bus_client = match connect_to_daemon_bus(&config).await {
        Ok(client) => Arc::new(client),
        Err(bus_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                error_code = %bus_error.code,
                "failed to connect to daemon-bus event bus"
            );
            best_effort_signal_failure(&config).await;
            return 1;
        }
    };

    let short_term = ShortTermTier::new(config.tier.short_term.clone());
    let long_term = LongTermTier::new(config.tier.long_term.clone());
    let episodic = EpisodicTier::new(config.tier.episodic.clone());

    let engine = Arc::new(MemoryEngine::new(
        short_term,
        long_term,
        episodic,
        Arc::clone(&store),
        Arc::clone(&write_queue),
        Arc::clone(&bus_client),
        Arc::clone(&config),
    ));

    tracing::info!(subsystem = SUBSYSTEM_ID, "MemoryEngine initialized");

    // ── Step 6: Start gRPC server ────────────────────────────────────────────
    use crate::generated::sena_daemonbus_v1::memory_service_server::MemoryServiceServer;

    let memory_service = grpc::MemoryServiceImpl::new(Arc::clone(&engine));
    let listen_addr: std::net::SocketAddr = format!("{}:{}", config.grpc.listen_address, config.grpc.listen_port)
        .parse()
        .expect("listen address must be valid — config.grpc.listen_address is validated at load and listen_port is a u16");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let grpc_handle = tokio::spawn(async move {
        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            component = "grpc",
            listen_addr = %listen_addr,
            "MemoryService gRPC server starting"
        );

        let serve_result = tonic::transport::Server::builder()
            .add_service(MemoryServiceServer::new(memory_service))
            .serve_with_shutdown(listen_addr, async {
                let _ = shutdown_rx.await;
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    component = "grpc",
                    "gRPC server received shutdown signal"
                );
            })
            .await;

        if let Err(serve_error) = serve_result {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                component = "grpc",
                error = %serve_error,
                "gRPC server exited with error"
            );
        } else {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                component = "grpc",
                "gRPC server stopped cleanly"
            );
        }
    });

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        listen_port = config.grpc.listen_port,
        "gRPC server started"
    );

    // ── Step 7: Signal MEMORY_ENGINE_READY to daemon-bus ─────────────────
    if let Err(signal_error) = signal_ready(&config).await {
        tracing::error!(
            subsystem = SUBSYSTEM_ID,
            error_code = %signal_error.code,
            "failed to signal MEMORY_ENGINE_READY to daemon-bus"
        );
        // The engine is running — we log the failure but do not exit.
        // daemon-bus will detect the missing signal via its boot timeout.
    } else {
        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            "MEMORY_ENGINE_READY signaled to daemon-bus"
        );
    }

    // ── Step 8: Subscribe to MODEL_PROFILE_READY in background ───────────
    //
    // Now that we are ready, subscribe to the event that model-probe will
    // publish once inference is also up. When it arrives we log the profile
    // fields. Full store reconfiguration (context budget, dynamic linking,
    // contradiction sensitivity) is deferred to Phase 2.
    let config_for_profile = Arc::clone(&config);
    tokio::spawn(async move {
        match receive_model_profile(&config_for_profile).await {
            Ok(model_profile) => {
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    component = "profile",
                    event_type = "model_profile_received",
                    model_id = %model_profile.model_id,
                    context_window = model_profile.context_window,
                    pre_rot_threshold = model_profile.pre_rot_threshold,
                    lora_compatible = model_profile.lora_compatible,
                    "ModelCapabilityProfile received — store reconfiguration deferred to Phase 2"
                );
            }
            Err(profile_error) => {
                // Not fatal — memory-engine is already running with defaults.
                // Log as warn so operators know the profile never arrived.
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    component = "profile",
                    error_code = %profile_error.code,
                    "ModelCapabilityProfile did not arrive — store continues with default profile"
                );
            }
        }
    });

    // ── Subscribe to TOPIC_MEMORY_CONSOLIDATION_REQUESTED in background ──
    //
    // CTP publishes this event during deep idle. When it arrives, trigger
    // the consolidation routine. The subscription runs for the lifetime of
    // the process — it never terminates on its own, only via shutdown.
    let config_for_consolidation = Arc::clone(&config);
    let engine_for_consolidation = Arc::clone(&engine);
    tokio::spawn(async move {
        if let Err(error) =
            receive_consolidation_requests(&config_for_consolidation, engine_for_consolidation)
                .await
        {
            // Consolidation subscription is a best-effort optimization.
            // If it fails to connect or the stream ends, log it but do not
            // exit the process — memory-engine continues running without
            // automatic consolidation.
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                component = "consolidation",
                error_code = %error.code,
                "consolidation subscription ended — no automatic consolidation until restart"
            );
        }
    });

    // ── Step 9: Await shutdown signal ──────────────────────────────────
    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        "memory-engine running — awaiting shutdown signal"
    );

    // Wait for SIGTERM / SIGINT (or equivalent on Windows).
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!(subsystem = SUBSYSTEM_ID, "shutdown signal received");
        }
        Err(signal_error) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                error = %signal_error,
                "failed to listen for shutdown signal — shutting down immediately"
            );
        }
    }

    // ── Graceful shutdown ───────────────────────────────────────────────
    tracing::info!(subsystem = SUBSYSTEM_ID, "initiating graceful shutdown");

    // Signal the gRPC server to stop accepting new connections.
    let _ = shutdown_tx.send(());

    // Drain the write queue and shut down the engine.
    engine.shutdown().await;

    // Wait for the gRPC server task to complete.
    if let Err(join_error) = grpc_handle.await {
        tracing::error!(
            subsystem = SUBSYSTEM_ID,
            error = %join_error,
            "gRPC server task panicked during shutdown"
        );
    }

    tracing::info!(subsystem = SUBSYSTEM_ID, "memory-engine shut down cleanly");

    0
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Receive the `ModelCapabilityProfile` from daemon-bus by subscribing to
/// the event bus and waiting for the `MODEL_PROFILE_READY` boot signal.
///
/// The profile is delivered as a JSON payload inside a `BusEvent`.
async fn receive_model_profile(config: &Config) -> Result<ModelCapabilityProfile, SenaError> {
    let timeout = Duration::from_millis(config.boot.ready_signal_timeout_ms);

    let mut event_client = EventBusServiceClient::connect(config.grpc.daemon_bus_address.clone())
        .await
        .map_err(|connect_error| {
            SenaError::new(
                ErrorCode::GrpcFailure,
                "failed to connect to daemon-bus event bus for profile subscription",
            )
            .with_debug_context(format!("connect error: {}", connect_error))
        })?;

    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics: vec![EventTopic::TopicBootSignal.into()],
        subscriber_id: SUBSYSTEM_ID.to_owned(),
    });

    let mut event_stream = event_client
        .subscribe(subscribe_request)
        .await
        .map_err(|subscribe_error| {
            SenaError::new(
                ErrorCode::GrpcFailure,
                "failed to subscribe to daemon-bus event bus",
            )
            .with_debug_context(format!("subscribe error: {}", subscribe_error))
        })?
        .into_inner();

    // Wait for a BusEvent whose payload contains the ModelCapabilityProfile.
    // The event topic is TOPIC_BOOT_SIGNAL and the source subsystem is
    // "model_probe". Timeout if the profile does not arrive in time.
    let profile_future = async {
        loop {
            match event_stream.message().await {
                Ok(Some(bus_event)) => {
                    // Check if this is the MODEL_PROFILE_READY signal.
                    if bus_event.topic == i32::from(EventTopic::TopicBootSignal)
                        && bus_event.source_subsystem == "model_probe"
                        && !bus_event.payload.is_empty()
                    {
                        let profile: ModelCapabilityProfile =
                            serde_json::from_slice(&bus_event.payload).map_err(|parse_error| {
                                SenaError::new(
                                ErrorCode::ProfileInvalid,
                                "failed to deserialize ModelCapabilityProfile from event payload",
                            )
                            .with_debug_context(format!("json parse error: {}", parse_error))
                            })?;
                        return Ok(profile);
                    }
                }
                Ok(None) => {
                    return Err(SenaError::new(
                        ErrorCode::ProfileMissing,
                        "event stream ended before ModelCapabilityProfile was received",
                    ));
                }
                Err(stream_error) => {
                    return Err(SenaError::new(
                        ErrorCode::GrpcFailure,
                        "event stream error while waiting for ModelCapabilityProfile",
                    )
                    .with_debug_context(format!("stream error: {}", stream_error)));
                }
            }
        }
    };

    tokio::time::timeout(timeout, profile_future)
        .await
        .map_err(|_elapsed| {
            SenaError::new(
                ErrorCode::BootTimeout,
                "timed out waiting for ModelCapabilityProfile from daemon-bus",
            )
        })?
}

/// Subscribe to TOPIC_MEMORY_CONSOLIDATION_REQUESTED and trigger consolidation
/// when events arrive from CTP.
///
/// This function runs indefinitely until the event stream ends or a
/// connection error occurs. It is spawned as a background task and does not
/// block the main execution flow.
async fn receive_consolidation_requests<E, X>(
    config: &Config,
    engine: Arc<MemoryEngine<E, X>>,
) -> Result<(), SenaError>
where
    E: ech0::Embedder + 'static,
    X: ech0::Extractor + 'static,
{
    let mut event_client = EventBusServiceClient::connect(config.grpc.daemon_bus_address.clone())
        .await
        .map_err(|connect_error| {
            SenaError::new(
                ErrorCode::GrpcFailure,
                "failed to connect to daemon-bus event bus for consolidation subscription",
            )
            .with_debug_context(format!("connect error: {}", connect_error))
        })?;

    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics: vec![EventTopic::TopicMemoryConsolidationRequested.into()],
        subscriber_id: SUBSYSTEM_ID.to_owned(),
    });

    let mut event_stream = event_client
        .subscribe(subscribe_request)
        .await
        .map_err(|subscribe_error| {
            SenaError::new(
                ErrorCode::GrpcFailure,
                "failed to subscribe to TOPIC_MEMORY_CONSOLIDATION_REQUESTED",
            )
            .with_debug_context(format!("subscribe error: {}", subscribe_error))
        })?
        .into_inner();

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        component = "consolidation",
        "subscribed to TOPIC_MEMORY_CONSOLIDATION_REQUESTED"
    );

    // Process events until the stream ends or an error occurs.
    loop {
        match event_stream.message().await {
            Ok(Some(bus_event)) => {
                // Verify this is a consolidation request from CTP.
                if bus_event.topic == i32::from(EventTopic::TopicMemoryConsolidationRequested) {
                    tracing::debug!(
                        subsystem = SUBSYSTEM_ID,
                        component = "consolidation",
                        event_id = %bus_event.event_id,
                        source_subsystem = %bus_event.source_subsystem,
                        "received TOPIC_MEMORY_CONSOLIDATION_REQUESTED event"
                    );

                    // Trigger consolidation. If it fails, log the error but
                    // do not terminate the subscription — the next event
                    // will trigger another attempt.
                    if let Err(consolidation_error) = engine.consolidate().await {
                        tracing::warn!(
                            subsystem = SUBSYSTEM_ID,
                            component = "consolidation",
                            error_code = %consolidation_error.code,
                            "consolidation failed"
                        );
                    }
                }
            }
            Ok(None) => {
                // Event stream ended cleanly.
                return Err(SenaError::new(
                    ErrorCode::GrpcFailure,
                    "consolidation event stream ended unexpectedly",
                ));
            }
            Err(stream_error) => {
                return Err(SenaError::new(
                    ErrorCode::GrpcFailure,
                    "consolidation event stream error",
                )
                .with_debug_context(format!("stream error: {}", stream_error)));
            }
        }
    }
}

/// Connect to the daemon-bus event bus service (for publishing events).
async fn connect_to_daemon_bus(config: &Config) -> Result<DaemonBusClient, SenaError> {
    let timeout = Duration::from_millis(config.grpc.connect_timeout_ms);

    let channel = tokio::time::timeout(
        timeout,
        tonic::transport::Channel::from_shared(config.grpc.daemon_bus_address.clone())
            .map_err(|uri_error| {
                SenaError::new(ErrorCode::GrpcFailure, "invalid daemon-bus address")
                    .with_debug_context(format!("uri error: {}", uri_error))
            })?
            .connect(),
    )
    .await
    .map_err(|_elapsed| {
        SenaError::new(ErrorCode::BootTimeout, "timed out connecting to daemon-bus")
    })?
    .map_err(|connect_error| {
        SenaError::new(ErrorCode::GrpcFailure, "failed to connect to daemon-bus")
            .with_debug_context(format!("connect error: {}", connect_error))
    })?;

    Ok(EventBusServiceClient::new(channel))
}

/// Signal `MEMORY_ENGINE_READY` to daemon-bus via the boot service.
async fn signal_ready(config: &Config) -> Result<(), SenaError> {
    let timeout = Duration::from_millis(config.grpc.connect_timeout_ms);

    let mut boot_client = tokio::time::timeout(
        timeout,
        BootServiceClient::connect(config.grpc.daemon_bus_address.clone()),
    )
    .await
    .map_err(|_elapsed| {
        SenaError::new(
            ErrorCode::BootTimeout,
            "timed out connecting to daemon-bus boot service",
        )
    })?
    .map_err(|connect_error| {
        SenaError::new(
            ErrorCode::GrpcFailure,
            "failed to connect to daemon-bus boot service",
        )
        .with_debug_context(format!("connect error: {}", connect_error))
    })?;

    let request = tonic::Request::new(BootSignalRequest {
        subsystem_id: SUBSYSTEM_ID.to_owned(),
        signal: BootSignal::MemoryEngineReady.into(),
        capabilities: capabilities::get_capabilities(),
    });

    boot_client
        .signal_ready(request)
        .await
        .map_err(|grpc_error| {
            SenaError::new(
                ErrorCode::GrpcFailure,
                "failed to signal MEMORY_ENGINE_READY",
            )
            .with_debug_context(format!("gRPC error: {}", grpc_error))
        })?;

    Ok(())
}

/// Best-effort attempt to signal boot failure to daemon-bus.
///
/// If this fails, we only log — the process is about to exit anyway.
async fn best_effort_signal_failure(config: &Config) {
    let timeout = Duration::from_millis(config.grpc.connect_timeout_ms);

    // Attempt to publish a TOPIC_BOOT_FAILED event as a courtesy.
    // daemon-bus detects failure via boot timeout regardless.
    let event_result = tokio::time::timeout(timeout, async {
        let mut event_client =
            match EventBusServiceClient::connect(config.grpc.daemon_bus_address.clone()).await {
                Ok(client) => client,
                Err(connect_error) => {
                    tracing::warn!(
                        subsystem = SUBSYSTEM_ID,
                        error = %connect_error,
                        "could not connect to daemon-bus to signal boot failure"
                    );
                    return Err(connect_error.to_string());
                }
            };

        let event = BusEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic: EventTopic::TopicBootFailed.into(),
            source_subsystem: SUBSYSTEM_ID.to_owned(),
            payload: Vec::new(),
            trace_context: String::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        event_client
            .publish(tonic::Request::new(
                crate::generated::sena_daemonbus_v1::PublishRequest { event: Some(event) },
            ))
            .await
            .map_err(|grpc_error| grpc_error.to_string())?;

        Ok(())
    })
    .await;

    match event_result {
        Ok(Ok(())) => {
            tracing::info!(
                subsystem = SUBSYSTEM_ID,
                "boot failure event published to daemon-bus"
            );
        }
        _ => {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                "could not publish boot failure event to daemon-bus"
            );
        }
    }
}

/// Build an ech0 `StoreConfig` from the `ProfileDerivedConfig` and
/// memory-engine `Config`.
///
/// This translates memory-engine's config model into ech0's config model.
/// The actual `StoreConfig` fields depend on ech0's API — this function
/// will be updated as ech0 stabilizes.
fn build_store_config(
    profile_derived: &profile::ProfileDerivedConfig,
    config: &Config,
    vector_dimensions: usize,
) -> ech0::StoreConfig {
    // Construct ech0 config types from our derived values.
    //
    // ech0's StoreConfig has these top-level fields:
    //   store: StorePathConfig (graph_path, vector_path, vector_dimensions)
    //   memory: MemoryConfig (short_term_capacity, decay rates, etc.)
    //   dynamic_linking: DynamicLinkingConfig (top_k_candidates, similarity_threshold, etc.)
    //   contradiction: ContradictionConfig (confidence_threshold, etc.)
    //
    // We use ech0 defaults for most fields and override the ones controlled
    // by the profile derivation.
    let mut store_config = ech0::StoreConfig::default();

    // Set storage paths from config — ech0 will create the files if they do
    // not yet exist, but the parent directory must exist before Store::new.
    store_config.store.graph_path = config.store.graph_path.clone();
    store_config.store.vector_path = config.store.vector_path.clone();

    // Set vector dimensions to match the embedder.
    store_config.store.vector_dimensions = vector_dimensions;

    // Map profile-derived dynamic linking flag.
    // When disabled, set top_k_candidates to 0 and max_links_per_ingest to 0
    // so the linking pass is effectively a no-op.
    if !profile_derived.dynamic_linking_enabled {
        store_config.dynamic_linking.top_k_candidates = 0;
        store_config.dynamic_linking.max_links_per_ingest = 0;
    }

    // Map profile-derived contradiction sensitivity to ech0's confidence_threshold.
    // Higher sensitivity → lower confidence_threshold (more contradictions flagged).
    // The profile gives us a sensitivity in [0.3, 0.8]. We invert it to map to
    // ech0's confidence_threshold (lower = more sensitive).
    store_config.contradiction.confidence_threshold =
        1.0 - profile_derived.contradiction_sensitivity;

    store_config
}

// ─────────────────────────────────────────────────────────────────────────────
// Tracing initialization
// ─────────────────────────────────────────────────────────────────────────────

/// Initialize the `tracing` subscriber based on the logging config.
///
/// Must be called exactly once, early in the boot sequence. After this,
/// all `tracing::info!`, `tracing::warn!`, etc. calls produce output.
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

            tracing::subscriber::set_global_default(subscriber).expect(
                "tracing subscriber must be set exactly once — duplicate initialization detected",
            );
        }
        // Default to "pretty" for any non-json format.
        _ => {
            let subscriber = tracing_subscriber::fmt()
                .pretty()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_thread_ids(true)
                .with_file(false)
                .with_line_number(false)
                .finish();

            tracing::subscriber::set_global_default(subscriber).expect(
                "tracing subscriber must be set exactly once — duplicate initialization detected",
            );
        }
    }
}
