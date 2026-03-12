//! Structured error type for daemon-bus.
//!
//! Every error carries a machine-readable code, a human-readable message,
//! and an optional debug_context that is **never** propagated cross-process.
//! debug_context is stripped before any gRPC response is sent.

use std::fmt;

/// Machine-readable error codes for daemon-bus operations.
/// These are safe to propagate across process boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    /// Boot sequence failed — a required subsystem did not signal ready in time.
    BootTimeout,
    /// Boot sequence halted — a required subsystem crashed during boot.
    BootSubsystemFailed,
    /// Process supervision — subsystem exceeded max retries, entering degraded mode.
    SupervisionRetriesExhausted,
    /// Process supervision — failed to spawn a subsystem process.
    SupervisionSpawnFailed,
    /// Priority arbitration — escalation request denied (queue full).
    EscalationDenied,
    /// Priority arbitration — invalid escalation ID on release.
    EscalationNotFound,
    /// Watchdog — a task exceeded its wall-clock timeout.
    TaskTimeout,
    /// Watchdog — task registration failed (capacity reached).
    WatchdogCapacityExceeded,
    /// Event bus — publish failed.
    BusPublishFailed,
    /// Configuration — failed to load or parse config file.
    ConfigLoadFailed,
    /// gRPC server — failed to bind or serve.
    GrpcServerFailed,
    /// Internal — catch-all for unexpected conditions that should not happen.
    Internal,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code_str = match self {
            ErrorCode::BootTimeout => "BOOT_TIMEOUT",
            ErrorCode::BootSubsystemFailed => "BOOT_SUBSYSTEM_FAILED",
            ErrorCode::SupervisionRetriesExhausted => "SUPERVISION_RETRIES_EXHAUSTED",
            ErrorCode::SupervisionSpawnFailed => "SUPERVISION_SPAWN_FAILED",
            ErrorCode::EscalationDenied => "ESCALATION_DENIED",
            ErrorCode::EscalationNotFound => "ESCALATION_NOT_FOUND",
            ErrorCode::TaskTimeout => "TASK_TIMEOUT",
            ErrorCode::WatchdogCapacityExceeded => "WATCHDOG_CAPACITY_EXCEEDED",
            ErrorCode::BusPublishFailed => "BUS_PUBLISH_FAILED",
            ErrorCode::ConfigLoadFailed => "CONFIG_LOAD_FAILED",
            ErrorCode::GrpcServerFailed => "GRPC_SERVER_FAILED",
            ErrorCode::Internal => "INTERNAL",
        };
        write!(formatter, "{}", code_str)
    }
}

/// The structured error type used throughout daemon-bus.
///
/// Separates two concerns per PRD §12.1:
/// - `code` + `message`: safe to propagate cross-process via gRPC.
/// - `debug_context`: local-only, written to structured logs, **never** sent over the wire.
#[derive(Debug, Clone)]
pub struct SenaError {
    /// Machine-readable error code. Safe to propagate anywhere.
    pub code: ErrorCode,
    /// Human-readable description. Safe to log and surface to the user.
    /// Must never contain prompt content, SoulBox values, file content,
    /// memory entry content, or stack traces with user data.
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

/// Convert SenaError into a tonic::Status for gRPC responses.
/// debug_context is always stripped — only code and message cross the wire.
impl From<SenaError> for tonic::Status {
    fn from(error: SenaError) -> Self {
        let grpc_code = match &error.code {
            ErrorCode::BootTimeout | ErrorCode::TaskTimeout => tonic::Code::DeadlineExceeded,
            ErrorCode::EscalationNotFound => tonic::Code::NotFound,
            ErrorCode::EscalationDenied | ErrorCode::WatchdogCapacityExceeded => {
                tonic::Code::ResourceExhausted
            }
            ErrorCode::ConfigLoadFailed => tonic::Code::FailedPrecondition,
            ErrorCode::Internal => tonic::Code::Internal,
            ErrorCode::BootSubsystemFailed
            | ErrorCode::SupervisionRetriesExhausted
            | ErrorCode::SupervisionSpawnFailed
            | ErrorCode::BusPublishFailed
            | ErrorCode::GrpcServerFailed => tonic::Code::Internal,
        };

        // Only code and message cross the wire — debug_context is deliberately dropped.
        let safe_message = format!("[{}] {}", error.code, error.message);
        tonic::Status::new(grpc_code, safe_message)
    }
}

/// Convenience alias used throughout daemon-bus.
pub type SenaResult<T> = Result<T, SenaError>;
