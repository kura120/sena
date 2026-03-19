//! gRPC BootService handler implementation.
//!
//! Exposes the boot sequence orchestrator over gRPC so child subsystems
//! can call SignalReady to report their readiness during the boot sequence.
//! Also provides GetBootStatus for diagnostics and UI.
//!
//! This handler is pure infrastructure — all business logic lives in
//! `crate::boot::BootOrchestrator`. The handler validates inputs, converts
//! proto types to internal types, and forwards to the orchestrator.

use std::collections::HashMap;

use tonic::{Request, Response, Status};

use crate::boot::BootOrchestrator;
use crate::error::SenaError;
use crate::generated::sena_daemonbus_v1::{
    boot_service_server::BootService, BootSignal, BootSignalRequest, BootSignalResponse,
    BootStatusRequest, BootStatusResponse,
};

/// The gRPC service handler for BootService.
///
/// Holds a cloneable `BootOrchestrator` handle (Clone via inner Arc) and
/// forwards gRPC requests to it after validation and type conversion.
#[derive(Clone)]
pub struct BootServiceHandler {
    boot_orchestrator: BootOrchestrator,
}

impl BootServiceHandler {
    pub fn new(boot_orchestrator: BootOrchestrator) -> Self {
        Self { boot_orchestrator }
    }
}

#[tonic::async_trait]
impl BootService for BootServiceHandler {
    /// Called by each subsystem when it is truly ready.
    ///
    /// Validates that the signal is not BOOT_SIGNAL_UNSPECIFIED, converts
    /// the signal i32 to its string name, and forwards to the orchestrator.
    async fn signal_ready(
        &self,
        request: Request<BootSignalRequest>,
    ) -> Result<Response<BootSignalResponse>, Status> {
        let req = request.into_inner();
        let subsystem_id = req.subsystem_id;
        let signal_i32 = req.signal;

        // Validate that signal is not BOOT_SIGNAL_UNSPECIFIED (0).
        if signal_i32 == 0 {
            tracing::warn!(
                subsystem = "daemon_bus",
                event_type = "boot_signal_invalid",
                subsystem_id = %subsystem_id,
                signal = signal_i32,
                "received BOOT_SIGNAL_UNSPECIFIED — invalid signal"
            );

            return Err(Status::invalid_argument(
                "signal must not be BOOT_SIGNAL_UNSPECIFIED",
            ));
        }

        // Convert i32 to BootSignal enum.
        let boot_signal = BootSignal::try_from(signal_i32).map_err(|_| {
            tracing::warn!(
                subsystem = "daemon_bus",
                event_type = "boot_signal_unknown_value",
                subsystem_id = %subsystem_id,
                signal = signal_i32,
                "received unknown boot signal value"
            );

            Status::invalid_argument(format!("unknown boot signal value: {}", signal_i32))
        })?;

        // Get the signal's string name (e.g. "MEMORY_ENGINE_READY").
        let signal_name = boot_signal.as_str_name();

        tracing::debug!(
            subsystem = "daemon_bus",
            event_type = "boot_signal_grpc_received",
            subsystem_id = %subsystem_id,
            signal = %signal_name,
            "received boot signal via gRPC"
        );

        // Forward to the orchestrator. The orchestrator returns Ok(true) if
        // the signal was accepted, Ok(false) if boot has already failed.
        let acknowledged = self
            .boot_orchestrator
            .signal_ready(&subsystem_id, signal_name)
            .await
            .map_err(|err: SenaError| {
                tracing::error!(
                    subsystem = "daemon_bus",
                    event_type = "boot_signal_error",
                    subsystem_id = %subsystem_id,
                    signal = %signal_name,
                    error_code = %err.code,
                    error_message = %err.message,
                    "boot signal processing failed"
                );

                // Convert SenaError to tonic::Status using the From impl.
                Status::from(err)
            })?;

        Ok(Response::new(BootSignalResponse { acknowledged }))
    }

    /// Query current boot status — useful for UI and diagnostics.
    ///
    /// Retrieves the current boot status snapshot from the orchestrator
    /// and converts it to the gRPC response format.
    async fn get_boot_status(
        &self,
        _request: Request<BootStatusRequest>,
    ) -> Result<Response<BootStatusResponse>, Status> {
        tracing::debug!(
            subsystem = "daemon_bus",
            event_type = "boot_status_query",
            "boot status query received via gRPC"
        );

        let snapshot = self.boot_orchestrator.get_boot_status().await;

        // Convert BootStatusSnapshot to BootStatusResponse.
        // The snapshot's subsystem_signals is HashMap<String, bool> where
        // the key is the signal name and the value is whether it was received.
        //
        // The proto BootStatusResponse.subsystem_signals is
        // HashMap<String, i32> where the value is a BootSignal enum value.
        //
        // Conversion rule:
        // - If received=true, look up the BootSignal enum from the signal name
        //   and use its i32 value.
        // - If received=false, use BootSignal::Unspecified (0) to indicate
        //   not-yet-received.
        let subsystem_signals: HashMap<String, i32> = snapshot
            .subsystem_signals
            .into_iter()
            .map(|(signal_name, received)| {
                let signal_value = if received {
                    // Signal has been received — convert the signal name back to its i32 value.
                    BootSignal::from_str_name(&signal_name)
                        .map(|s| s as i32)
                        .unwrap_or_else(|| {
                            // Signal name not recognized — should not happen in correct operation.
                            // Log a warning and fall back to Unspecified.
                            tracing::warn!(
                                subsystem = "daemon_bus",
                                event_type = "boot_status_unknown_signal",
                                signal_name = %signal_name,
                                "boot status snapshot contains unknown signal name"
                            );
                            BootSignal::Unspecified as i32
                        })
                } else {
                    // Signal not received yet — use Unspecified (0).
                    BootSignal::Unspecified as i32
                };

                (signal_name, signal_value)
            })
            .collect();

        let response = BootStatusResponse {
            subsystem_signals,
            sena_ready: snapshot.sena_ready,
        };

        tracing::debug!(
            subsystem = "daemon_bus",
            event_type = "boot_status_response",
            sena_ready = snapshot.sena_ready,
            signal_count = response.subsystem_signals.len(),
            "boot status query completed"
        );

        Ok(Response::new(response))
    }
}
