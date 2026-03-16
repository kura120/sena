//! gRPC service implementation for memory-engine.
//!
//! Exposes `MemoryService` to daemon-bus (and other subsystems routed through
//! daemon-bus). Every method delegates to `MemoryEngine` — no business logic
//! lives here.
//!
//! # Responsibilities
//!
//! - Translate incoming gRPC requests into `MemoryEngine` method calls.
//! - Translate `SenaError` results into `tonic::Status` responses.
//! - **Always strip `debug_context`** from `SenaError` before any gRPC response.
//! - Log request metadata (operation, priority) but never memory content.
//!
//! # Non-responsibilities
//!
//! - No tier management, no lock acquisition, no ech0 calls.
//! - No relevance scoring, no prompt assembly, no reasoning.
//! - No event broadcasting — `MemoryEngine` handles that internally.

use std::sync::Arc;

use ech0::{Embedder, Extractor};
use tonic::{Request, Response, Status};

use crate::engine::{MemoryEngine, MemoryEntry, TargetTier};
use crate::error::SenaError;
use crate::queue::Priority;

// ─────────────────────────────────────────────────────────────────────────────
// Request / response types
// ─────────────────────────────────────────────────────────────────────────────
//
// MemoryService is not yet defined in the daemon-bus proto. These types serve
// as the gRPC contract until a formal proto definition is added. Once the
// proto exists, these are replaced by the generated types.
//
// TODO(proto): Add MemoryService to sena.daemonbus.v1.proto and replace these
// manual definitions with the generated types from tonic-build.

/// Priority level as transmitted over gRPC.
/// Maps to the internal `queue::Priority` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GrpcPriority {
    Reactive = 1,
    Background = 2,
}

impl GrpcPriority {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            1 => Some(GrpcPriority::Reactive),
            2 => Some(GrpcPriority::Background),
            _ => None,
        }
    }

    pub fn to_internal(self) -> Priority {
        match self {
            GrpcPriority::Reactive => Priority::Reactive,
            GrpcPriority::Background => Priority::Background,
        }
    }
}

/// Target tier as transmitted over gRPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GrpcTargetTier {
    ShortTerm = 1,
    LongTerm = 2,
    Episodic = 3,
}

impl GrpcTargetTier {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            1 => Some(GrpcTargetTier::ShortTerm),
            2 => Some(GrpcTargetTier::LongTerm),
            3 => Some(GrpcTargetTier::Episodic),
            _ => None,
        }
    }

    pub fn to_internal(self) -> TargetTier {
        match self {
            GrpcTargetTier::ShortTerm => TargetTier::ShortTerm,
            GrpcTargetTier::LongTerm => TargetTier::LongTerm,
            GrpcTargetTier::Episodic => TargetTier::Episodic,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Manual gRPC message types
// ─────────────────────────────────────────────────────────────────────────────
//
// These will be replaced by proto-generated types once MemoryService is added
// to the proto definition.

/// Request to write a memory entry.
#[derive(Debug, Clone)]
pub struct WriteMemoryRequest {
    /// The text content to ingest. Never logged by the gRPC layer.
    pub text: String,
    /// Target tier (1 = ShortTerm, 2 = LongTerm, 3 = Episodic).
    pub target_tier: i32,
    /// Priority (1 = Reactive, 2 = Background).
    pub priority: i32,
}

/// Response to a write request.
#[derive(Debug, Clone)]
pub struct WriteMemoryResponse {
    /// Whether the write was accepted and committed.
    pub success: bool,
}

/// Request to search/read memory.
#[derive(Debug, Clone)]
pub struct ReadMemoryRequest {
    /// The query string for semantic search. Never logged by the gRPC layer.
    pub query: String,
    /// Maximum number of results to return.
    pub max_results: u32,
    /// Priority (1 = Reactive, 2 = Background).
    pub priority: i32,
}

/// Response containing search results.
#[derive(Debug, Clone)]
pub struct ReadMemoryResponse {
    /// Serialized search results from ech0. The caller deserializes based
    /// on the agreed-upon contract.
    pub results_json: String,
}

/// Request to promote an entry from short-term to long-term.
#[derive(Debug, Clone)]
pub struct PromoteMemoryRequest {
    /// The entry ID to promote.
    pub entry_id: String,
    /// Priority (1 = Reactive, 2 = Background).
    pub priority: i32,
}

/// Response to a promote request.
#[derive(Debug, Clone)]
pub struct PromoteMemoryResponse {
    /// Whether the promotion succeeded.
    pub success: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// MemoryServiceImpl
// ─────────────────────────────────────────────────────────────────────────────

/// gRPC service implementation for memory-engine.
///
/// Holds an `Arc<MemoryEngine<E, X>>` and delegates all calls to it. Contains
/// zero business logic — only request validation, delegation, and response
/// translation.
///
/// Generic over `E` (embedder) and `X` (extractor) because `MemoryEngine`
/// is generic over these types.
pub struct MemoryServiceImpl<E: Embedder, X: Extractor> {
    engine: Arc<MemoryEngine<E, X>>,
}

impl<E: Embedder + 'static, X: Extractor + 'static> MemoryServiceImpl<E, X> {
    /// Create a new gRPC service backed by the given `MemoryEngine`.
    pub fn new(engine: Arc<MemoryEngine<E, X>>) -> Self {
        Self { engine }
    }

    /// Handle a write request.
    ///
    /// Validates the request fields, delegates to `MemoryEngine::write()`,
    /// and translates the result into a gRPC response. `debug_context` is
    /// always stripped before returning any error.
    pub async fn write_memory(
        &self,
        request: Request<WriteMemoryRequest>,
    ) -> Result<Response<WriteMemoryResponse>, Status> {
        let inner = request.into_inner();

        let target_tier = GrpcTargetTier::from_i32(inner.target_tier)
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "invalid target_tier value: {}",
                    inner.target_tier
                ))
            })?
            .to_internal();

        let priority = GrpcPriority::from_i32(inner.priority)
            .ok_or_else(|| {
                Status::invalid_argument(format!("invalid priority value: {}", inner.priority))
            })?
            .to_internal();

        tracing::debug!(
            subsystem = "memory_engine",
            component = "grpc",
            operation = "write",
            tier = target_tier.as_str(),
            priority = %priority,
            "write_memory request received"
        );

        let entry = MemoryEntry {
            text: inner.text,
            target_tier,
        };

        self.engine
            .write(entry, priority)
            .await
            .map(|()| Response::new(WriteMemoryResponse { success: true }))
            .map_err(|sena_error| strip_and_convert(sena_error))
    }

    /// Handle a read/search request.
    ///
    /// Validates the request fields, delegates to `MemoryEngine::read()`,
    /// and translates the result into a gRPC response. `debug_context` is
    /// always stripped before returning any error.
    pub async fn read_memory(
        &self,
        request: Request<ReadMemoryRequest>,
    ) -> Result<Response<ReadMemoryResponse>, Status> {
        let inner = request.into_inner();

        let priority = GrpcPriority::from_i32(inner.priority)
            .ok_or_else(|| {
                Status::invalid_argument(format!("invalid priority value: {}", inner.priority))
            })?
            .to_internal();

        tracing::debug!(
            subsystem = "memory_engine",
            component = "grpc",
            operation = "read",
            priority = %priority,
            max_results = inner.max_results,
            "read_memory request received"
        );

        // Construct ech0 SearchOptions from the request.
        // The exact SearchOptions fields depend on ech0's API — we pass
        // through what we can and let ech0 handle defaults for the rest.
        let search_options = ech0::SearchOptions {
            limit: inner.max_results as usize,
            ..Default::default()
        };

        let search_result = self
            .engine
            .read(&inner.query, search_options, priority)
            .await
            .map_err(|sena_error| strip_and_convert(sena_error))?;

        // Return a summary of the search results for the gRPC response.
        // SearchResult does not implement Serialize, so we return node
        // count information. A proper proto message for SearchResult
        // should be defined in a follow-up.
        let results_json = format!(
            "{{\"nodes_count\":{},\"edges_count\":{}}}",
            search_result.nodes.len(),
            search_result.edges.len()
        );

        Ok(Response::new(ReadMemoryResponse { results_json }))
    }

    /// Handle a promote request (short-term → long-term).
    ///
    /// Validates the request fields, delegates to `MemoryEngine::promote()`,
    /// and translates the result into a gRPC response. `debug_context` is
    /// always stripped before returning any error.
    pub async fn promote_memory(
        &self,
        request: Request<PromoteMemoryRequest>,
    ) -> Result<Response<PromoteMemoryResponse>, Status> {
        let inner = request.into_inner();

        let priority = GrpcPriority::from_i32(inner.priority)
            .ok_or_else(|| {
                Status::invalid_argument(format!("invalid priority value: {}", inner.priority))
            })?
            .to_internal();

        if inner.entry_id.is_empty() {
            return Err(Status::invalid_argument("entry_id must not be empty"));
        }

        tracing::debug!(
            subsystem = "memory_engine",
            component = "grpc",
            operation = "promote",
            priority = %priority,
            "promote_memory request received"
        );

        let entry_id = crate::tier::EntryId::new(inner.entry_id);

        self.engine
            .promote(entry_id, priority)
            .await
            .map(|()| Response::new(PromoteMemoryResponse { success: true }))
            .map_err(|sena_error| strip_and_convert(sena_error))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error conversion — always strips debug_context
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a `SenaError` into a `tonic::Status`, stripping `debug_context`
/// before crossing the gRPC boundary.
///
/// `debug_context` is logged locally at the point of origin (engine.rs,
/// queue.rs, etc.) and must **never** appear in any gRPC response.
fn strip_and_convert(error: SenaError) -> Status {
    // Log the full error locally (including debug_context) before stripping.
    if let Some(ref debug_ctx) = error.debug_context {
        tracing::debug!(
            subsystem = "memory_engine",
            component = "grpc",
            error_code = %error.code,
            debug_context = %debug_ctx,
            "stripping debug_context before gRPC response"
        );
    }

    let stripped = error.into_cross_process();
    stripped.into()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{DebugContext, ErrorCode};

    #[test]
    fn strip_and_convert_removes_debug_context() {
        let error = SenaError {
            code: ErrorCode::StorageFailure,
            message: "ingest_text failed".to_owned(),
            debug_context: Some(DebugContext::new(
                "internal ech0 detail that must not cross gRPC",
            )),
        };

        let status = strip_and_convert(error);
        let status_message = status.message().to_string();

        assert!(
            !status_message.contains("internal ech0 detail"),
            "debug_context must not appear in gRPC status message"
        );
        assert!(status_message.contains("STORAGE_FAILURE"));
        assert!(status_message.contains("ingest_text failed"));
    }

    #[test]
    fn strip_and_convert_handles_no_debug_context() {
        let error = SenaError::new(ErrorCode::QueueFull, "write queue at capacity");

        let status = strip_and_convert(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
        assert!(status.message().contains("QUEUE_FULL"));
    }

    #[test]
    fn grpc_priority_from_i32_valid() {
        assert_eq!(GrpcPriority::from_i32(1), Some(GrpcPriority::Reactive));
        assert_eq!(GrpcPriority::from_i32(2), Some(GrpcPriority::Background));
    }

    #[test]
    fn grpc_priority_from_i32_invalid() {
        assert_eq!(GrpcPriority::from_i32(0), None);
        assert_eq!(GrpcPriority::from_i32(3), None);
        assert_eq!(GrpcPriority::from_i32(-1), None);
    }

    #[test]
    fn grpc_priority_to_internal() {
        assert_eq!(GrpcPriority::Reactive.to_internal(), Priority::Reactive);
        assert_eq!(GrpcPriority::Background.to_internal(), Priority::Background);
    }

    #[test]
    fn grpc_target_tier_from_i32_valid() {
        assert_eq!(GrpcTargetTier::from_i32(1), Some(GrpcTargetTier::ShortTerm));
        assert_eq!(GrpcTargetTier::from_i32(2), Some(GrpcTargetTier::LongTerm));
        assert_eq!(GrpcTargetTier::from_i32(3), Some(GrpcTargetTier::Episodic));
    }

    #[test]
    fn grpc_target_tier_from_i32_invalid() {
        assert_eq!(GrpcTargetTier::from_i32(0), None);
        assert_eq!(GrpcTargetTier::from_i32(4), None);
        assert_eq!(GrpcTargetTier::from_i32(-1), None);
    }

    #[test]
    fn grpc_target_tier_to_internal() {
        assert_eq!(
            GrpcTargetTier::ShortTerm.to_internal(),
            TargetTier::ShortTerm
        );
        assert_eq!(GrpcTargetTier::LongTerm.to_internal(), TargetTier::LongTerm);
        assert_eq!(GrpcTargetTier::Episodic.to_internal(), TargetTier::Episodic);
    }

    #[test]
    fn queue_full_maps_to_resource_exhausted_in_grpc() {
        let error = SenaError::new(ErrorCode::QueueFull, "at capacity");
        let status = strip_and_convert(error);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn queue_timeout_maps_to_deadline_exceeded_in_grpc() {
        let error = SenaError::new(ErrorCode::QueueTimeout, "timed out");
        let status = strip_and_convert(error);
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[test]
    fn storage_failure_maps_to_internal_in_grpc() {
        let error = SenaError::new(ErrorCode::StorageFailure, "store error");
        let status = strip_and_convert(error);
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn profile_missing_maps_to_failed_precondition_in_grpc() {
        let error = SenaError::new(ErrorCode::ProfileMissing, "no profile");
        let status = strip_and_convert(error);
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}
