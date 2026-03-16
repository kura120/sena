//! Structured error type for memory-engine.
//!
//! Mirrors the daemon-bus SenaError pattern: every error carries a machine-readable
//! code, a human-readable message, and an optional debug_context that is **never**
//! propagated cross-process.
//!
//! `message` must never contain raw text, user messages, memory entry content,
//! model output, or SoulBox values — only operation names and error codes.

use std::fmt;

// ─────────────────────────────────────────────────────────────────────────────
// ErrorCode
// ─────────────────────────────────────────────────────────────────────────────

/// Machine-readable error codes for memory-engine operations.
/// Safe to propagate across process boundaries via gRPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    /// ech0 Store operation failed (read, search, or internal store error).
    StorageFailure,
    /// Embedding generation via llama-cpp-rs failed.
    EmbedderFailure,
    /// Graph/entity extraction via llama-cpp-rs failed.
    ExtractorFailure,
    /// ModelCapabilityProfile was not received from daemon-bus during boot.
    ProfileMissing,
    /// ModelCapabilityProfile was received but contained invalid or
    /// unprocessable values.
    ProfileInvalid,
    /// The write queue has reached max_depth — no more operations can be
    /// enqueued until the queue drains.
    QueueFull,
    /// A queued write operation exceeded its operation_timeout_ms before
    /// processing completed.
    QueueTimeout,
    /// The boot sequence did not complete within ready_signal_timeout_ms.
    BootTimeout,
    /// A gRPC call to daemon-bus or from a client failed.
    GrpcFailure,
    /// Failed to load or parse memory-engine.toml.
    ConfigLoadFailure,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code_str = match self {
            ErrorCode::StorageFailure => "STORAGE_FAILURE",
            ErrorCode::EmbedderFailure => "EMBEDDER_FAILURE",
            ErrorCode::ExtractorFailure => "EXTRACTOR_FAILURE",
            ErrorCode::ProfileMissing => "PROFILE_MISSING",
            ErrorCode::ProfileInvalid => "PROFILE_INVALID",
            ErrorCode::QueueFull => "QUEUE_FULL",
            ErrorCode::QueueTimeout => "QUEUE_TIMEOUT",
            ErrorCode::BootTimeout => "BOOT_TIMEOUT",
            ErrorCode::GrpcFailure => "GRPC_FAILURE",
            ErrorCode::ConfigLoadFailure => "CONFIG_LOAD_FAILURE",
        };
        write!(formatter, "{}", code_str)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DebugContext
// ─────────────────────────────────────────────────────────────────────────────

/// Internal-only debug information attached to a SenaError.
/// Written to structured logs locally, **never** sent over gRPC.
#[derive(Debug, Clone)]
pub struct DebugContext {
    /// Free-form debug detail. Must not contain user content or memory entries.
    pub detail: String,
}

impl DebugContext {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for DebugContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.detail)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SenaError
// ─────────────────────────────────────────────────────────────────────────────

/// The structured error type used throughout memory-engine.
///
/// Separates two concerns:
/// - `code` + `message`: safe to propagate cross-process via gRPC.
/// - `debug_context`: local-only, written to structured logs, **never** sent
///   over the wire.
#[derive(Debug, Clone)]
pub struct SenaError {
    /// Machine-readable error code. Safe to propagate anywhere.
    pub code: ErrorCode,
    /// Human-readable description. Safe to log and surface cross-process.
    /// Must never contain raw text, user messages, memory entry content,
    /// model output, or SoulBox values — only operation names and error codes.
    pub message: String,
    /// Internal-only debug information. Populated locally for tracing,
    /// included in local structured logs, stripped before crossing any
    /// process boundary.
    pub debug_context: Option<DebugContext>,
}

impl SenaError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            debug_context: None,
        }
    }

    pub fn with_debug_context(mut self, detail: impl Into<String>) -> Self {
        self.debug_context = Some(DebugContext::new(detail));
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

// ─────────────────────────────────────────────────────────────────────────────
// From<EchoError>
// ─────────────────────────────────────────────────────────────────────────────

/// Maps ech0's `EchoError` to memory-engine's `SenaError`.
///
/// This conversion happens at the engine.rs call site — never inside trait impls
/// (Embedder/Extractor return `EchoError` per ech0's trait contract).
///
/// The mapping inspects `EchoError.code` to select the most specific
/// `ErrorCode`. The `EchoError.context` is captured in `debug_context` so it
/// appears in local structured logs but never crosses a gRPC boundary.
impl From<ech0::EchoError> for SenaError {
    fn from(echo_error: ech0::EchoError) -> Self {
        let (code, message) = match echo_error.code {
            ech0::ErrorCode::EmbedderFailure => (
                ErrorCode::EmbedderFailure,
                "ech0 embedding operation failed".to_owned(),
            ),
            ech0::ErrorCode::ExtractorFailure => (
                ErrorCode::ExtractorFailure,
                "ech0 extraction operation failed".to_owned(),
            ),
            ech0::ErrorCode::InvalidInput => {
                (ErrorCode::StorageFailure, "ech0 invalid input".to_owned())
            }
            ech0::ErrorCode::CapacityExceeded => (
                ErrorCode::StorageFailure,
                "ech0 capacity exceeded".to_owned(),
            ),
            // StorageFailure, ConsistencyError, ConflictUnresolved all map to
            // StorageFailure — they represent internal store, graph, or
            // persistence errors that memory-engine cannot recover from at the
            // operation level.
            _ => (
                ErrorCode::StorageFailure,
                format!("ech0 store operation failed: {:?}", echo_error.code),
            ),
        };

        let debug_detail = match &echo_error.context {
            Some(ctx) => format!(
                "EchoError {{ code: {:?}, message: {}, context_location: {} }}",
                echo_error.code, echo_error.message, ctx.location
            ),
            None => format!(
                "EchoError {{ code: {:?}, message: {} }}",
                echo_error.code, echo_error.message
            ),
        };

        SenaError {
            code,
            message,
            debug_context: Some(DebugContext::new(debug_detail)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// tonic::Status conversions
// ─────────────────────────────────────────────────────────────────────────────

/// Convert SenaError into a tonic::Status for gRPC error propagation.
/// `debug_context` is always stripped — only code and message cross the wire.
impl From<SenaError> for tonic::Status {
    fn from(error: SenaError) -> Self {
        let grpc_code = match &error.code {
            ErrorCode::StorageFailure => tonic::Code::Internal,
            ErrorCode::EmbedderFailure => tonic::Code::Internal,
            ErrorCode::ExtractorFailure => tonic::Code::Internal,
            ErrorCode::ProfileMissing => tonic::Code::FailedPrecondition,
            ErrorCode::ProfileInvalid => tonic::Code::InvalidArgument,
            ErrorCode::QueueFull => tonic::Code::ResourceExhausted,
            ErrorCode::QueueTimeout => tonic::Code::DeadlineExceeded,
            ErrorCode::BootTimeout => tonic::Code::DeadlineExceeded,
            ErrorCode::GrpcFailure => tonic::Code::Unavailable,
            ErrorCode::ConfigLoadFailure => tonic::Code::FailedPrecondition,
        };

        let safe_message = format!("[{}] {}", error.code, error.message);
        tonic::Status::new(grpc_code, safe_message)
    }
}

/// Convert a tonic::Status (received from daemon-bus) into a SenaError.
impl From<tonic::Status> for SenaError {
    fn from(status: tonic::Status) -> Self {
        SenaError::new(
            ErrorCode::GrpcFailure,
            format!("gRPC call failed: {}", status.message()),
        )
        .with_debug_context(format!("gRPC code: {:?}", status.code()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Additional From impls for common error sources
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a serde_json error into a SenaError.
/// Used when deserializing ModelCapabilityProfile from daemon-bus event payload.
impl From<serde_json::Error> for SenaError {
    fn from(error: serde_json::Error) -> Self {
        SenaError::new(
            ErrorCode::ProfileInvalid,
            "failed to deserialize profile payload",
        )
        .with_debug_context(format!("serde_json error: {error}"))
    }
}

/// Convert a toml deserialization error into a SenaError.
/// Used when loading memory-engine.toml.
impl From<toml::de::Error> for SenaError {
    fn from(error: toml::de::Error) -> Self {
        SenaError::new(
            ErrorCode::ConfigLoadFailure,
            "failed to parse memory-engine.toml",
        )
        .with_debug_context(format!("toml error: {error}"))
    }
}

/// Convert a std::io::Error into a SenaError.
/// Used when reading config files from disk.
impl From<std::io::Error> for SenaError {
    fn from(error: std::io::Error) -> Self {
        SenaError::new(
            ErrorCode::ConfigLoadFailure,
            "failed to read configuration file",
        )
        .with_debug_context(format!("io error: {error}"))
    }
}

/// Convenience alias used throughout memory-engine.
pub type SenaResult<T> = Result<T, SenaError>;

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_includes_code_and_message() {
        let error = SenaError::new(ErrorCode::StorageFailure, "ingest_text failed");
        let displayed = format!("{}", error);
        assert_eq!(displayed, "[STORAGE_FAILURE] ingest_text failed");
    }

    #[test]
    fn cross_process_strips_debug_context() {
        let error = SenaError::new(ErrorCode::EmbedderFailure, "embedding call failed")
            .with_debug_context("llama-cpp returned CUDA OOM at layer 12");

        assert!(error.debug_context.is_some());

        let safe_error = error.into_cross_process();
        assert!(safe_error.debug_context.is_none());
        assert_eq!(safe_error.code, ErrorCode::EmbedderFailure);
        assert_eq!(safe_error.message, "embedding call failed");
    }

    #[test]
    fn into_tonic_status_never_leaks_debug_context() {
        let error = SenaError::new(ErrorCode::StorageFailure, "ingest_text failed")
            .with_debug_context("internal ech0 stack: Node::insert panicked at graph.rs:42");

        let status: tonic::Status = error.into();
        let status_message = status.message().to_string();

        assert!(
            !status_message.contains("graph.rs:42"),
            "debug_context must not leak into tonic::Status message"
        );
        assert!(status_message.contains("STORAGE_FAILURE"));
        assert!(status_message.contains("ingest_text failed"));
    }

    #[test]
    fn error_code_display_is_screaming_snake_case() {
        assert_eq!(format!("{}", ErrorCode::StorageFailure), "STORAGE_FAILURE");
        assert_eq!(
            format!("{}", ErrorCode::EmbedderFailure),
            "EMBEDDER_FAILURE"
        );
        assert_eq!(
            format!("{}", ErrorCode::ExtractorFailure),
            "EXTRACTOR_FAILURE"
        );
        assert_eq!(format!("{}", ErrorCode::ProfileMissing), "PROFILE_MISSING");
        assert_eq!(format!("{}", ErrorCode::ProfileInvalid), "PROFILE_INVALID");
        assert_eq!(format!("{}", ErrorCode::QueueFull), "QUEUE_FULL");
        assert_eq!(format!("{}", ErrorCode::QueueTimeout), "QUEUE_TIMEOUT");
        assert_eq!(format!("{}", ErrorCode::BootTimeout), "BOOT_TIMEOUT");
        assert_eq!(format!("{}", ErrorCode::GrpcFailure), "GRPC_FAILURE");
        assert_eq!(
            format!("{}", ErrorCode::ConfigLoadFailure),
            "CONFIG_LOAD_FAILURE"
        );
    }

    #[test]
    fn from_tonic_status_preserves_message() {
        let status = tonic::Status::unavailable("daemon-bus unreachable");
        let error: SenaError = status.into();

        assert_eq!(error.code, ErrorCode::GrpcFailure);
        assert!(error.message.contains("daemon-bus unreachable"));
        assert!(error.debug_context.is_some());
    }

    #[test]
    fn queue_full_maps_to_resource_exhausted() {
        let error = SenaError::new(ErrorCode::QueueFull, "write queue at capacity");
        let status: tonic::Status = error.into();
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn queue_timeout_maps_to_deadline_exceeded() {
        let error = SenaError::new(ErrorCode::QueueTimeout, "operation timed out");
        let status: tonic::Status = error.into();
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[test]
    fn profile_missing_maps_to_failed_precondition() {
        let error = SenaError::new(
            ErrorCode::ProfileMissing,
            "ModelCapabilityProfile not received",
        );
        let status: tonic::Status = error.into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}
