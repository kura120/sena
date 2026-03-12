//! Structured error type for model-probe.
//!
//! Mirrors the daemon-bus SenaError pattern per PRD §12.1: every error carries
//! a machine-readable code, a human-readable message, and an optional
//! debug_context that is **never** propagated cross-process.

use std::fmt;

/// Machine-readable error codes for model-probe operations.
/// These are safe to propagate across process boundaries via gRPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    /// Failed to load or parse the model-probe config TOML.
    ConfigLoadFailed,
    /// Failed to connect to daemon-bus gRPC endpoint.
    DaemonBusConnectionFailed,
    /// Failed to publish a boot signal or event to daemon-bus.
    DaemonBusPublishFailed,
    /// Hardware detection failed — could not query GPU or system info.
    HardwareDetectionFailed,
    /// Model loading or initialization failed via llama-cpp-rs.
    ModelLoadFailed,
    /// A probe timed out before producing a result.
    ProbeTimeout,
    /// A probe failed during execution (inference error, parse error, etc.).
    ProbeFailed,
    /// The entire probe battery failed — no usable profile could be built.
    ProbeBatteryFailed,
    /// Serialization or deserialization of profile/event payloads failed.
    SerializationFailed,
    /// Internal — catch-all for unexpected conditions that should not happen.
    Internal,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code_str = match self {
            ErrorCode::ConfigLoadFailed => "CONFIG_LOAD_FAILED",
            ErrorCode::DaemonBusConnectionFailed => "DAEMON_BUS_CONNECTION_FAILED",
            ErrorCode::DaemonBusPublishFailed => "DAEMON_BUS_PUBLISH_FAILED",
            ErrorCode::HardwareDetectionFailed => "HARDWARE_DETECTION_FAILED",
            ErrorCode::ModelLoadFailed => "MODEL_LOAD_FAILED",
            ErrorCode::ProbeTimeout => "PROBE_TIMEOUT",
            ErrorCode::ProbeFailed => "PROBE_FAILED",
            ErrorCode::ProbeBatteryFailed => "PROBE_BATTERY_FAILED",
            ErrorCode::SerializationFailed => "SERIALIZATION_FAILED",
            ErrorCode::Internal => "INTERNAL",
        };
        write!(formatter, "{}", code_str)
    }
}

/// The structured error type used throughout model-probe.
///
/// Separates two concerns per PRD §12.1:
/// - `code` + `message`: safe to propagate cross-process via gRPC.
/// - `debug_context`: local-only, written to structured logs, **never** sent over the wire.
#[derive(Debug, Clone)]
pub struct SenaError {
    /// Machine-readable error code. Safe to propagate anywhere.
    pub code: ErrorCode,
    /// Human-readable description. Safe to log and surface to the user.
    /// Must never contain prompt content, model responses, SoulBox values,
    /// file content, memory entry content, or stack traces with user data.
    pub message: String,
    /// Internal-only debug information. Populated locally for tracing,
    /// included in local structured logs, stripped before crossing
    /// any process boundary.
    pub debug_context: Option<String>,
}

impl SenaError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            debug_context: None,
        }
    }

    pub fn with_debug_context(mut self, context: impl Into<String>) -> Self {
        self.debug_context = Some(context.into());
        self
    }

    /// Produce a copy of this error with debug_context stripped.
    /// Used before sending errors across process boundaries via gRPC.
    pub fn into_cross_process(self) -> Self {
        Self {
            code: self.code,
            message: self.message,
            debug_context: None,
        }
    }
}

impl fmt::Display for SenaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for SenaError {}

/// Convert SenaError into a tonic::Status for gRPC error propagation.
/// debug_context is always stripped — only code and message cross the wire.
impl From<SenaError> for tonic::Status {
    fn from(error: SenaError) -> Self {
        let grpc_code = match &error.code {
            ErrorCode::ConfigLoadFailed => tonic::Code::FailedPrecondition,
            ErrorCode::DaemonBusConnectionFailed => tonic::Code::Unavailable,
            ErrorCode::DaemonBusPublishFailed => tonic::Code::Internal,
            ErrorCode::HardwareDetectionFailed => tonic::Code::Internal,
            ErrorCode::ModelLoadFailed => tonic::Code::FailedPrecondition,
            ErrorCode::ProbeTimeout => tonic::Code::DeadlineExceeded,
            ErrorCode::ProbeFailed => tonic::Code::Internal,
            ErrorCode::ProbeBatteryFailed => tonic::Code::Internal,
            ErrorCode::SerializationFailed => tonic::Code::Internal,
            ErrorCode::Internal => tonic::Code::Internal,
        };

        let safe_message = format!("[{}] {}", error.code, error.message);
        tonic::Status::new(grpc_code, safe_message)
    }
}

/// Convert a tonic::Status (received from daemon-bus) into a SenaError.
impl From<tonic::Status> for SenaError {
    fn from(status: tonic::Status) -> Self {
        SenaError::new(
            ErrorCode::DaemonBusPublishFailed,
            format!("gRPC call failed: {}", status.message()),
        )
        .with_debug_context(format!("gRPC code: {:?}", status.code()))
    }
}

/// Convert a serde_json serialization error into a SenaError.
impl From<serde_json::Error> for SenaError {
    fn from(error: serde_json::Error) -> Self {
        SenaError::new(
            ErrorCode::SerializationFailed,
            "JSON serialization/deserialization failed",
        )
        .with_debug_context(format!("serde_json error: {error}"))
    }
}

/// Convenience alias used throughout model-probe.
pub type SenaResult<T> = Result<T, SenaError>;

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_includes_code_and_message() {
        let error = SenaError::new(ErrorCode::ProbeFailed, "reasoning probe returned empty");
        let displayed = format!("{}", error);
        assert_eq!(displayed, "[PROBE_FAILED] reasoning probe returned empty");
    }

    #[test]
    fn cross_process_strips_debug_context() {
        let error = SenaError::new(ErrorCode::ProbeTimeout, "probe timed out")
            .with_debug_context("internal stack trace details");

        assert!(error.debug_context.is_some());

        let safe_error = error.into_cross_process();
        assert!(safe_error.debug_context.is_none());
        assert_eq!(safe_error.code, ErrorCode::ProbeTimeout);
        assert_eq!(safe_error.message, "probe timed out");
    }

    #[test]
    fn into_tonic_status_never_leaks_debug_context() {
        let error = SenaError::new(ErrorCode::HardwareDetectionFailed, "NVML unavailable")
            .with_debug_context("NvmlError: driver version mismatch at 0xDEAD");

        let status: tonic::Status = error.into();
        let status_message = status.message().to_string();

        assert!(
            !status_message.contains("0xDEAD"),
            "debug_context must not leak into tonic::Status message"
        );
        assert!(status_message.contains("HARDWARE_DETECTION_FAILED"));
        assert!(status_message.contains("NVML unavailable"));
    }

    #[test]
    fn error_code_display_is_screaming_snake_case() {
        assert_eq!(
            format!("{}", ErrorCode::ConfigLoadFailed),
            "CONFIG_LOAD_FAILED"
        );
        assert_eq!(
            format!("{}", ErrorCode::DaemonBusConnectionFailed),
            "DAEMON_BUS_CONNECTION_FAILED"
        );
        assert_eq!(format!("{}", ErrorCode::ProbeTimeout), "PROBE_TIMEOUT");
        assert_eq!(format!("{}", ErrorCode::Internal), "INTERNAL");
    }

    #[test]
    fn from_tonic_status_preserves_message() {
        let status = tonic::Status::unavailable("daemon-bus unreachable");
        let error: SenaError = status.into();

        assert_eq!(error.code, ErrorCode::DaemonBusPublishFailed);
        assert!(error.message.contains("daemon-bus unreachable"));
        assert!(error.debug_context.is_some());
    }

    #[test]
    fn from_serde_json_error_maps_correctly() {
        let bad_json = "not valid json {{{";
        let serde_error: Result<serde_json::Value, _> = serde_json::from_str(bad_json);
        let error: SenaError = serde_error.unwrap_err().into();

        assert_eq!(error.code, ErrorCode::SerializationFailed);
        assert!(error.debug_context.is_some());
    }
}
