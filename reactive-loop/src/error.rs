//! Error types for the reactive-loop subsystem.

use thiserror::Error;

/// Core error type for reactive-loop operations.
#[derive(Debug, Error)]
pub enum ReactiveLoopError {
    #[error("inference unavailable: {reason}")]
    InferenceUnavailable { reason: String },

    #[error("prompt composer unavailable: {reason}")]
    PromptComposerUnavailable { reason: String },

    #[error("config load error: {reason}")]
    ConfigLoad { reason: String },

    #[error("config validation failed on field '{field}': {reason}")]
    ConfigValidation { field: String, reason: String },

    #[error("daemon-bus connection failed: {reason}")]
    DaemonBusConnection { reason: String },

    #[error("gRPC error: {0}")]
    Grpc(String),

    #[error("event publishing failed: {reason}")]
    EventPublishFailed { reason: String },

    #[error("request timeout: {reason}")]
    RequestTimeout { reason: String },
}

/// Convert ReactiveLoopError to tonic::Status for gRPC responses.
impl From<ReactiveLoopError> for tonic::Status {
    fn from(error: ReactiveLoopError) -> Self {
        match error {
            ReactiveLoopError::InferenceUnavailable { reason } => {
                tonic::Status::unavailable(format!("inference service unavailable: {}", reason))
            }
            ReactiveLoopError::PromptComposerUnavailable { reason } => {
                tonic::Status::unavailable(format!(
                    "prompt composer service unavailable: {}",
                    reason
                ))
            }
            ReactiveLoopError::ConfigLoad { reason } => {
                tonic::Status::internal(format!("configuration error: {}", reason))
            }
            ReactiveLoopError::ConfigValidation { field, reason } => {
                tonic::Status::internal(format!("invalid config field '{}': {}", field, reason))
            }
            ReactiveLoopError::DaemonBusConnection { reason } => {
                tonic::Status::unavailable(format!("daemon-bus connection failed: {}", reason))
            }
            ReactiveLoopError::Grpc(message) => {
                tonic::Status::internal(format!("gRPC error: {}", message))
            }
            ReactiveLoopError::EventPublishFailed { reason } => {
                tonic::Status::internal(format!("event publishing failed: {}", reason))
            }
            ReactiveLoopError::RequestTimeout { reason } => {
                tonic::Status::deadline_exceeded(format!("request timeout: {}", reason))
            }
        }
    }
}

/// Convert tonic transport errors to ReactiveLoopError.
impl From<tonic::transport::Error> for ReactiveLoopError {
    fn from(error: tonic::transport::Error) -> Self {
        ReactiveLoopError::Grpc(error.to_string())
    }
}

/// Convert tonic Status errors to ReactiveLoopError.
impl From<tonic::Status> for ReactiveLoopError {
    fn from(error: tonic::Status) -> Self {
        ReactiveLoopError::Grpc(format!("status: {} - {}", error.code(), error.message()))
    }
}

/// Convert I/O errors to ReactiveLoopError (for config loading).
impl From<std::io::Error> for ReactiveLoopError {
    fn from(error: std::io::Error) -> Self {
        ReactiveLoopError::ConfigLoad {
            reason: error.to_string(),
        }
    }
}

/// Convert TOML deserialization errors to ReactiveLoopError.
impl From<toml::de::Error> for ReactiveLoopError {
    fn from(error: toml::de::Error) -> Self {
        ReactiveLoopError::ConfigLoad {
            reason: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_to_status_inference_unavailable() {
        let error = ReactiveLoopError::InferenceUnavailable {
            reason: "connection refused".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(status.message().contains("inference service unavailable"));
        assert!(status.message().contains("connection refused"));
    }

    #[test]
    fn test_error_to_status_prompt_composer_unavailable() {
        let error = ReactiveLoopError::PromptComposerUnavailable {
            reason: "timeout".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(status
            .message()
            .contains("prompt composer service unavailable"));
    }

    #[test]
    fn test_error_to_status_config_load() {
        let error = ReactiveLoopError::ConfigLoad {
            reason: "file not found".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("configuration error"));
    }

    #[test]
    fn test_error_to_status_config_validation() {
        let error = ReactiveLoopError::ConfigValidation {
            field: "grpc.listen_port".into(),
            reason: "must be greater than 1024".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("invalid config field"));
        assert!(status.message().contains("grpc.listen_port"));
    }

    #[test]
    fn test_error_to_status_daemon_bus_connection() {
        let error = ReactiveLoopError::DaemonBusConnection {
            reason: "connection refused".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(status.message().contains("daemon-bus connection failed"));
    }

    #[test]
    fn test_error_to_status_request_timeout() {
        let error = ReactiveLoopError::RequestTimeout {
            reason: "inference took too long".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
        assert!(status.message().contains("request timeout"));
    }

    #[test]
    fn test_from_io_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = ReactiveLoopError::from(io_error);
        match error {
            ReactiveLoopError::ConfigLoad { reason } => {
                assert!(reason.contains("file not found"));
            }
            _ => panic!("Expected ConfigLoad error"),
        }
    }

    #[test]
    fn test_from_toml_error() {
        let toml_error = toml::from_str::<toml::Value>("invalid toml {").unwrap_err();
        let error = ReactiveLoopError::from(toml_error);
        match error {
            ReactiveLoopError::ConfigLoad { reason } => {
                assert!(!reason.is_empty());
            }
            _ => panic!("Expected ConfigLoad error"),
        }
    }
}
