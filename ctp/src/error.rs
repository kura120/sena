use thiserror::Error;

#[derive(Debug, Error)]
pub enum CtpError {
    #[error("config error: {0}")]
    Config(String),

    #[error("config validation failed: {field}: {reason}")]
    ConfigValidation { field: String, reason: String },

    #[error("daemon-bus error: {0}")]
    DaemonBus(String),

    #[error("memory-engine error: {0}")]
    MemoryEngine(String),

    #[error("thought queue full")]
    QueueFull,

    #[error("activity detection error: {0}")]
    ActivityDetection(String),

    #[error("gRPC transport error: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),
}

impl From<CtpError> for tonic::Status {
    fn from(error: CtpError) -> Self {
        match error {
            CtpError::Config(_) | CtpError::ConfigValidation { .. } => {
                tonic::Status::internal(error.to_string())
            }
            CtpError::DaemonBus(_) => tonic::Status::unavailable(error.to_string()),
            CtpError::MemoryEngine(_) => tonic::Status::unavailable(error.to_string()),
            CtpError::QueueFull => tonic::Status::resource_exhausted(error.to_string()),
            CtpError::ActivityDetection(_) => tonic::Status::internal(error.to_string()),
            CtpError::GrpcTransport(_) => tonic::Status::unavailable(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_messages() {
        let errors: Vec<CtpError> = vec![
            CtpError::Config("test config error".into()),
            CtpError::ConfigValidation {
                field: "test_field".into(),
                reason: "test reason".into(),
            },
            CtpError::DaemonBus("test daemon bus error".into()),
            CtpError::MemoryEngine("test memory engine error".into()),
            CtpError::QueueFull,
            CtpError::ActivityDetection("test activity error".into()),
        ];

        for error in &errors {
            let display = error.to_string();
            assert!(!display.is_empty(), "error display must not be empty: {:?}", error);
        }
    }

    #[test]
    fn test_error_to_status_queue_full() {
        let error = CtpError::QueueFull;
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn test_error_to_status_daemon_bus() {
        let error = CtpError::DaemonBus("connection lost".into());
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Unavailable);
    }

    #[test]
    fn test_error_to_status_config() {
        let error = CtpError::Config("bad config".into());
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Internal);
    }
}
