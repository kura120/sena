use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("model not found: {model_id}")]
    ModelNotFound { model_id: String },

    #[error("model load failed for '{model_id}': {reason}")]
    ModelLoad { model_id: String, reason: String },

    #[error("insufficient VRAM: required {required_mb} MB, available {available_mb} MB")]
    InsufficientVram {
        required_mb: u32,
        available_mb: u32,
    },

    #[error("request queue full (max depth: {max_depth})")]
    RequestQueueFull { max_depth: usize },

    #[error("request timed out after {timeout_ms} ms")]
    RequestTimeout { timeout_ms: u64 },

    #[error("OOM during inference for model '{model_id}'")]
    OomDuringInference { model_id: String },

    #[error("model is currently switching — unavailable")]
    ModelSwitching,

    #[error("inference execution failed: {reason}")]
    InferenceExecution { reason: String },

    #[error("spawn_blocking task failed: {0}")]
    SpawnBlocking(#[from] tokio::task::JoinError),

    #[error("config load failed: {reason}")]
    ConfigLoad { reason: String },

    #[error("config validation failed: {field}: {reason}")]
    ConfigValidation { field: String, reason: String },

    #[error("gRPC error: {0}")]
    Grpc(String),

    #[error("daemon-bus connection failed: {reason}")]
    DaemonBusConnection { reason: String },
}

impl From<InferenceError> for tonic::Status {
    fn from(error: InferenceError) -> Self {
        match error {
            InferenceError::ModelNotFound { .. } => {
                tonic::Status::not_found(error.to_string())
            }
            InferenceError::ModelLoad { .. } => tonic::Status::internal(error.to_string()),
            InferenceError::InsufficientVram { .. } => {
                tonic::Status::resource_exhausted(error.to_string())
            }
            InferenceError::RequestQueueFull { .. } => {
                tonic::Status::resource_exhausted(error.to_string())
            }
            InferenceError::RequestTimeout { .. } => {
                tonic::Status::deadline_exceeded(error.to_string())
            }
            InferenceError::OomDuringInference { .. } => {
                tonic::Status::unavailable(error.to_string())
            }
            InferenceError::ModelSwitching => tonic::Status::unavailable(error.to_string()),
            InferenceError::InferenceExecution { .. } => {
                tonic::Status::internal(error.to_string())
            }
            InferenceError::SpawnBlocking(_) => tonic::Status::internal(error.to_string()),
            InferenceError::ConfigLoad { .. } => tonic::Status::internal(error.to_string()),
            InferenceError::ConfigValidation { .. } => tonic::Status::internal(error.to_string()),
            InferenceError::Grpc(_) => tonic::Status::internal(error.to_string()),
            InferenceError::DaemonBusConnection { .. } => {
                tonic::Status::unavailable(error.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_to_status_model_not_found() {
        let error = InferenceError::ModelNotFound {
            model_id: "test-model".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn test_error_to_status_queue_full() {
        let error = InferenceError::RequestQueueFull { max_depth: 100 };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn test_error_to_status_timeout() {
        let error = InferenceError::RequestTimeout { timeout_ms: 5000 };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[test]
    fn test_error_to_status_oom() {
        let error = InferenceError::OomDuringInference {
            model_id: "test".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Unavailable);
    }
}
