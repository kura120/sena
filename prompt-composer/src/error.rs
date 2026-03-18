use thiserror::Error;

#[derive(Debug, Error)]
pub enum PcError {
    #[error("config error: {0}")]
    Config(String),

    #[error("TOON encoding failed: {0}")]
    ToonEncodingFailed(String),

    #[error("TOON encoding produced invalid output")]
    ToonEncodingInvalid,

    #[error("sacred content overflow: required {required_tokens} tokens, budget {budget} tokens")]
    SacredContentOverflow { required_tokens: u32, budget: u32 },

    #[error("token budget exceeded")]
    BudgetExceeded,

    #[error("gRPC transport error: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),

    #[error("spawn_blocking failed: {0}")]
    SpawnBlocking(String),
}

impl From<PcError> for tonic::Status {
    fn from(error: PcError) -> Self {
        match error {
            PcError::Config(_) => tonic::Status::internal(error.to_string()),
            PcError::ToonEncodingFailed(_) => tonic::Status::internal(error.to_string()),
            PcError::ToonEncodingInvalid => tonic::Status::internal(error.to_string()),
            PcError::SacredContentOverflow { .. } => {
                tonic::Status::resource_exhausted(error.to_string())
            }
            PcError::BudgetExceeded => tonic::Status::resource_exhausted(error.to_string()),
            PcError::GrpcTransport(_) => tonic::Status::unavailable(error.to_string()),
            PcError::SpawnBlocking(_) => tonic::Status::internal(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_sacred_overflow_to_status() {
        let error = PcError::SacredContentOverflow {
            required_tokens: 5000,
            budget: 4000,
        };
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn test_error_toon_invalid_to_status() {
        let error = PcError::ToonEncodingInvalid;
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn test_error_budget_exceeded_to_status() {
        let error = PcError::BudgetExceeded;
        let status = tonic::Status::from(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }
}
