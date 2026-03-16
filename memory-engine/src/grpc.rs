//! gRPC service implementation for memory-engine.
//!
//! Delegates all calls to `MemoryEngine`. No business logic here.
//! `debug_context` is always stripped from `SenaError` before any response.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::engine::{MemoryEngine, MemoryEntry, TargetTier};
use crate::error::ErrorCode;
use crate::generated::sena_daemonbus_v1::{
    memory_service_server::MemoryService, MemoryPromoteRequest, MemoryPromoteResponse,
    MemoryReadRequest, MemoryReadResponse, MemorySearchResult, MemoryWriteRequest,
    MemoryWriteResponse,
};
use crate::queue::Priority;
use crate::tier::EntryId;
use ech0::SearchOptions;

pub struct MemoryServiceImpl<E: ech0::Embedder, X: ech0::Extractor> {
    engine: Arc<MemoryEngine<E, X>>,
}

impl<E: ech0::Embedder + 'static, X: ech0::Extractor + 'static> MemoryServiceImpl<E, X> {
    pub fn new(engine: Arc<MemoryEngine<E, X>>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl<E, X> MemoryService for MemoryServiceImpl<E, X>
where
    E: ech0::Embedder + 'static,
    X: ech0::Extractor + 'static,
{
    async fn write(
        &self,
        request: Request<MemoryWriteRequest>,
    ) -> Result<Response<MemoryWriteResponse>, Status> {
        let req = request.into_inner();

        let target_tier = parse_tier(&req.target_tier)?;
        let priority = parse_priority(&req.priority)?;

        let entry = MemoryEntry {
            text: req.text,
            target_tier,
        };

        let entry_id = self
            .engine
            .write(entry, priority)
            .await
            .map_err(|sena_error| {
                // debug_context stripped — never crosses gRPC boundary
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "grpc",
                    operation = "write",
                    error_code = %sena_error.code,
                    "write RPC failed"
                );
                error_code_to_status(&sena_error.code, &sena_error.message)
            })?;

        Ok(Response::new(MemoryWriteResponse {
            accepted: true,
            entry_id,
        }))
    }

    async fn read(
        &self,
        request: Request<MemoryReadRequest>,
    ) -> Result<Response<MemoryReadResponse>, Status> {
        let req = request.into_inner();

        let priority = parse_priority(&req.priority)?;

        let options = SearchOptions {
            limit: req.limit as usize,
            min_importance: req.min_score,
            ..Default::default()
        };

        let search_result = self
            .engine
            .read(&req.query, options, priority)
            .await
            .map_err(|sena_error| {
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "grpc",
                    operation = "read",
                    error_code = %sena_error.code,
                    "read RPC failed"
                );
                error_code_to_status(&sena_error.code, &sena_error.message)
            })?;

        let results = search_result
            .nodes
            .into_iter()
            .map(|scored_node| {
                // Node does not have a `summary` field — use `kind` as a
                // concise descriptor. `source_text` may contain the original
                // ingested text but is potentially large and contains user
                // content which must never appear in logs; `kind` is safe.
                let summary = scored_node.node.kind.clone();

                // Node does not have a `tier` field. Infer tier from metadata
                // if present, otherwise fall back to the node kind.
                let tier = scored_node
                    .node
                    .metadata
                    .get("tier")
                    .and_then(|value| value.as_str())
                    .unwrap_or_else(|| match scored_node.node.kind.as_str() {
                        "working" | "short_term" | "session" => "short_term",
                        "event" | "episode" => "episodic",
                        _ => "long_term",
                    })
                    .to_owned();

                MemorySearchResult {
                    node_id: scored_node.node.id.to_string(),
                    summary,
                    score: scored_node.score,
                    tier,
                }
            })
            .collect();

        Ok(Response::new(MemoryReadResponse { results }))
    }

    async fn promote(
        &self,
        request: Request<MemoryPromoteRequest>,
    ) -> Result<Response<MemoryPromoteResponse>, Status> {
        let req = request.into_inner();

        let priority = parse_priority(&req.priority)?;
        let entry_id = EntryId::new(req.entry_id);

        self.engine
            .promote(entry_id, priority)
            .await
            .map_err(|sena_error| {
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "grpc",
                    operation = "promote",
                    error_code = %sena_error.code,
                    "promote RPC failed"
                );
                error_code_to_status(&sena_error.code, &sena_error.message)
            })?;

        Ok(Response::new(MemoryPromoteResponse { promoted: true }))
    }
}

// ── Parse helpers ─────────────────────────────────────────────────────────────

fn parse_tier(tier_str: &str) -> Result<TargetTier, Status> {
    match tier_str {
        "short_term" => Ok(TargetTier::ShortTerm),
        "long_term" => Ok(TargetTier::LongTerm),
        "episodic" => Ok(TargetTier::Episodic),
        other => Err(Status::invalid_argument(format!(
            "unknown target_tier '{}' — expected short_term | long_term | episodic",
            other
        ))),
    }
}

fn parse_priority(priority_str: &str) -> Result<Priority, Status> {
    match priority_str {
        "reactive" => Ok(Priority::Reactive),
        "background" => Ok(Priority::Background),
        other => Err(Status::invalid_argument(format!(
            "unknown priority '{}' — expected reactive | background",
            other
        ))),
    }
}

fn error_code_to_status(code: &ErrorCode, message: &str) -> Status {
    match code {
        ErrorCode::QueueFull => Status::resource_exhausted(message),
        ErrorCode::QueueTimeout => Status::deadline_exceeded(message),
        ErrorCode::StorageFailure | ErrorCode::EmbedderFailure | ErrorCode::ExtractorFailure => {
            Status::internal(message)
        }
        ErrorCode::ProfileMissing | ErrorCode::ProfileInvalid => {
            Status::failed_precondition(message)
        }
        _ => Status::internal(message),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tier_valid() {
        assert_eq!(parse_tier("short_term").unwrap(), TargetTier::ShortTerm);
        assert_eq!(parse_tier("long_term").unwrap(), TargetTier::LongTerm);
        assert_eq!(parse_tier("episodic").unwrap(), TargetTier::Episodic);
    }

    #[test]
    fn parse_tier_invalid() {
        let err = parse_tier("unknown").unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn parse_priority_valid() {
        assert_eq!(parse_priority("reactive").unwrap(), Priority::Reactive);
        assert_eq!(parse_priority("background").unwrap(), Priority::Background);
    }

    #[test]
    fn parse_priority_invalid() {
        let err = parse_priority("urgent").unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn parse_tier_empty_string_is_invalid() {
        let err = parse_tier("").unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn parse_priority_empty_string_is_invalid() {
        let err = parse_priority("").unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn queue_full_maps_to_resource_exhausted_in_grpc() {
        let status = error_code_to_status(&ErrorCode::QueueFull, "at capacity");
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn queue_timeout_maps_to_deadline_exceeded_in_grpc() {
        let status = error_code_to_status(&ErrorCode::QueueTimeout, "timed out");
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[test]
    fn storage_failure_maps_to_internal_in_grpc() {
        let status = error_code_to_status(&ErrorCode::StorageFailure, "store error");
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn profile_missing_maps_to_failed_precondition_in_grpc() {
        let status = error_code_to_status(&ErrorCode::ProfileMissing, "no profile");
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn embedder_failure_maps_to_internal_in_grpc() {
        let status = error_code_to_status(&ErrorCode::EmbedderFailure, "embed error");
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn extractor_failure_maps_to_internal_in_grpc() {
        let status = error_code_to_status(&ErrorCode::ExtractorFailure, "extract error");
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn profile_invalid_maps_to_failed_precondition_in_grpc() {
        let status = error_code_to_status(&ErrorCode::ProfileInvalid, "bad profile");
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}
