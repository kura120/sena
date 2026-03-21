use thiserror::Error;

#[derive(Debug, Error)]
pub enum PromptComposerError {
    #[error("budget exhausted: sacred tokens {sacred_tokens} exceed budget {budget}")]
    BudgetExhausted { sacred_tokens: u32, budget: u32 },

    #[error("encoding failed: {reason}")]
    EncodingFailed { reason: String },

    #[error("config load failed: {reason}")]
    ConfigLoad { reason: String },

    #[error("config validation failed: {field}: {reason}")]
    ConfigValidation { field: String, reason: String },

    #[error("daemon-bus connection failed: {reason}")]
    DaemonBusConnection { reason: String },

    #[error("gRPC error: {0}")]
    Grpc(String),

    #[error("missing required field: {field}")]
    MissingField { field: String },

    #[error("invalid model profile: {reason}")]
    InvalidModelProfile { reason: String },
}

impl From<PromptComposerError> for tonic::Status {
    fn from(error: PromptComposerError) -> Self {
        match error {
            PromptComposerError::BudgetExhausted { .. } => {
                tonic::Status::resource_exhausted(error.to_string())
            }
            PromptComposerError::EncodingFailed { .. } => {
                tonic::Status::internal(error.to_string())
            }
            PromptComposerError::ConfigLoad { .. } => tonic::Status::internal(error.to_string()),
            PromptComposerError::ConfigValidation { .. } => {
                tonic::Status::internal(error.to_string())
            }
            PromptComposerError::DaemonBusConnection { .. } => {
                tonic::Status::unavailable(error.to_string())
            }
            PromptComposerError::Grpc(_) => tonic::Status::internal(error.to_string()),
            PromptComposerError::MissingField { .. } => {
                tonic::Status::invalid_argument(error.to_string())
            }
            PromptComposerError::InvalidModelProfile { .. } => {
                tonic::Status::invalid_argument(error.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_to_status_budget_exhausted() {
        let error = PromptComposerError::BudgetExhausted {
            sacred_tokens: 5000,
            budget: 4096,
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn test_error_to_status_missing_field() {
        let error = PromptComposerError::MissingField {
            field: "model_profile".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn test_error_to_status_daemon_bus_connection() {
        let error = PromptComposerError::DaemonBusConnection {
            reason: "connection refused".into(),
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Unavailable);
    }
}
