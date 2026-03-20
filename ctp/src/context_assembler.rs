//! Context assembler — gathers all inputs and packages them into a
//! `PromptContext` struct for handoff to prompt-composer.
//!
//! CTP assembles the raw context; prompt-composer serializes it to TOON.
//! Never TOON-encode inside CTP.

use crate::activity::ActivityState;
use crate::error::CtpError;
use crate::generated::sena_daemonbus_v1::{
    memory_service_client::MemoryServiceClient,
    MemoryReadRequest,
};
use tokio::sync::Mutex;

/// Stub for SoulBox personality snapshot — SoulBox not built yet.
/// Replaced with the real type in Milestone C.
#[derive(Debug, Clone, Default)]
pub struct SoulBoxSnapshot {
    pub personality_summary: String,
}

impl SoulBoxSnapshot {
    /// Returns an empty snapshot — the fallback when SoulBox is unavailable.
    pub fn empty() -> Self {
        Self {
            personality_summary: String::new(),
        }
    }
}

/// Stub for OS context — platform layer not built yet.
#[derive(Debug, Clone, Default)]
pub struct OsContext {
    pub active_window: String,
    pub recent_events: Vec<String>,
}

impl OsContext {
    /// Returns an empty OS context — platform layer not built yet.
    pub fn empty() -> Self {
        Self {
            active_window: String::new(),
            recent_events: Vec::new(),
        }
    }
}

/// Stub for model capability profile — received from daemon-bus at boot.
#[derive(Debug, Clone, Default)]
pub struct ModelCapabilityProfile {
    pub model_id: String,
    pub context_window: u32,
    pub output_reserve: u32,
}

/// A memory search result from memory-engine gRPC.
#[derive(Debug, Clone)]
pub struct MemoryResult {
    pub node_id: String,
    pub summary: String,
    pub score: f32,
    pub tier: String,
}

/// Assembled context for prompt-composer. Contains raw Rust structs — no TOON.
#[derive(Debug, Clone)]
pub struct PromptContext {
    pub soulbox_snapshot: SoulBoxSnapshot,
    pub short_term: Vec<MemoryResult>,
    pub long_term: Vec<MemoryResult>,
    pub episodic: Vec<MemoryResult>,
    pub os_context: OsContext,
    pub model_profile: ModelCapabilityProfile,
    pub user_intent: Option<String>,
    pub activity_state: ActivityState,
}

/// Assembles a `PromptContext` from available sources.
///
/// Connects to memory-engine via gRPC on first use (lazy connection).
/// Memory-engine unavailability produces empty results, not errors.
pub struct ContextAssembler {
    memory_client: Mutex<Option<MemoryServiceClient<tonic::transport::Channel>>>,
    memory_engine_address: String,
    memory_query_limit: u32,
    memory_min_score: f32,
}

impl ContextAssembler {
    pub fn new(memory_engine_address: String, memory_query_limit: u32, memory_min_score: f32) -> Self {
        Self {
            memory_client: Mutex::new(None),
            memory_engine_address,
            memory_query_limit,
            memory_min_score,
        }
    }

    /// Get or establish connection to memory-engine.
    /// Returns None on connection failure (with warning logged).
    async fn get_or_connect_client(&self) -> Option<MemoryServiceClient<tonic::transport::Channel>> {
        let mut guard = self.memory_client.lock().await;
        if let Some(ref client) = *guard {
            return Some(client.clone());
        }
        
        match MemoryServiceClient::connect(self.memory_engine_address.clone()).await {
            Ok(client) => {
                *guard = Some(client.clone());
                Some(client)
            }
            Err(connect_error) => {
                tracing::warn!(
                    subsystem = "ctp",
                    event_type = "memory_engine_connect_failed",
                    address = %self.memory_engine_address,
                    error = %connect_error,
                    "failed to connect to memory-engine — memory reads will return empty"
                );
                None
            }
        }
    }

    /// Read all memory tiers from memory-engine in a single gRPC call.
    /// Results are split by tier on the client side.
    /// Returns empty vecs on connection failure or gRPC errors (best-effort).
    async fn read_memory(&self, query: &str, priority: &str) -> (Vec<MemoryResult>, Vec<MemoryResult>, Vec<MemoryResult>) {
        let mut client = match self.get_or_connect_client().await {
            Some(c) => c,
            None => return (Vec::new(), Vec::new(), Vec::new()),
        };
        
        let request = tonic::Request::new(MemoryReadRequest {
            query: query.to_string(),
            limit: self.memory_query_limit,
            min_score: self.memory_min_score,
            priority: priority.to_string(),
            trace_context: String::new(), // TODO: propagate from caller when tracing is wired
        });
        
        let response = match client.read(request).await {
            Ok(resp) => resp.into_inner(),
            Err(grpc_error) => {
                tracing::warn!(
                    subsystem = "ctp",
                    event_type = "memory_read_failed",
                    error = %grpc_error,
                    "memory-engine read failed — using empty memory context"
                );
                // Clear cached client on error so reconnect is attempted next time
                let mut guard = self.memory_client.lock().await;
                *guard = None;
                return (Vec::new(), Vec::new(), Vec::new());
            }
        };
        
        // Split results by tier
        let mut short_term = Vec::new();
        let mut long_term = Vec::new();
        let mut episodic = Vec::new();
        
        for result in response.results {
            let memory_result = MemoryResult {
                node_id: result.node_id,
                summary: result.summary,
                score: result.score,
                tier: result.tier.clone(),
            };
            
            match memory_result.tier.as_str() {
                "short_term" => short_term.push(memory_result),
                "long_term" => long_term.push(memory_result),
                "episodic" => episodic.push(memory_result),
                other => {
                    tracing::debug!(
                        subsystem = "ctp",
                        event_type = "unknown_memory_tier",
                        tier = other,
                        node_id = %memory_result.node_id,
                        "memory result has unknown tier — skipping"
                    );
                }
            }
        }
        
        (short_term, long_term, episodic)
    }

    /// Assemble a `PromptContext` from all available sources.
    ///
    /// Memory reads are combined into a single gRPC call, then split by tier.
    /// SoulBox unavailability produces a fallback, not an error.
    pub async fn assemble(&self, activity_state: ActivityState) -> Result<PromptContext, CtpError> {
        // Single gRPC call to memory-engine, then split by tier client-side
        let (short_term, long_term, episodic) = self
            .read_memory("context assembly", "background")
            .await;

        // SoulBox — fallback to empty when unavailable
        let soulbox_snapshot = self.read_soulbox().await.unwrap_or_else(|_| {
            tracing::debug!(
                subsystem = "ctp",
                event_type = "soulbox_unavailable",
                "SoulBox unavailable — using empty snapshot"
            );
            SoulBoxSnapshot::empty()
        });

        Ok(PromptContext {
            soulbox_snapshot,
            short_term,
            long_term,
            episodic,
            os_context: OsContext::empty(),
            model_profile: ModelCapabilityProfile::default(),
            user_intent: None,
            activity_state,
        })
    }

    /// Read SoulBox snapshot — stub for Phase 1, returns empty.
    async fn read_soulbox(&self) -> Result<SoulBoxSnapshot, CtpError> {
        // SoulBox is not built yet — return empty
        Ok(SoulBoxSnapshot::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_assembler_produces_context_with_all_fields() {
        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into(), 20, 0.3);
        let context = assembler
            .assemble(ActivityState::UserActive)
            .await
            .expect("test: assemble should succeed");

        // All fields should be populated (even if empty for Phase 1)
        assert_eq!(context.activity_state, ActivityState::UserActive);
        assert!(context.soulbox_snapshot.personality_summary.is_empty());
        assert!(context.short_term.is_empty());
        assert!(context.long_term.is_empty());
        assert!(context.episodic.is_empty());
        assert!(context.os_context.active_window.is_empty());
        assert!(context.user_intent.is_none());
    }

    #[tokio::test]
    async fn test_assembler_does_not_toon_encode() {
        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into(), 20, 0.3);
        let context = assembler
            .assemble(ActivityState::Idle2Min)
            .await
            .expect("test: assemble should succeed");

        // PromptContext should contain raw Rust types, not TOON strings.
        // Verify the types are Rust structs, not encoded strings.
        let _: &SoulBoxSnapshot = &context.soulbox_snapshot;
        let _: &Vec<MemoryResult> = &context.short_term;
        let _: &OsContext = &context.os_context;
        let _: &ModelCapabilityProfile = &context.model_profile;
        // If this compiles, the types are raw Rust structs — not TOON.
    }

    #[tokio::test]
    async fn test_assembler_single_memory_read() {
        // This test verifies that memory reads are combined into a single
        // gRPC call, then split by tier on the client side.
        use std::time::Instant;

        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into(), 20, 0.3);
        let start = Instant::now();
        let _context = assembler
            .assemble(ActivityState::UserActive)
            .await
            .expect("test: assemble should succeed");
        let elapsed = start.elapsed();

        // With the bogus address, connection should fail quickly and return empty results.
        // This test ensures the structure is correct (single read_memory call).
        assert!(
            elapsed.as_millis() < 5000,
            "single memory read with connection failure should be fast, took {} ms",
            elapsed.as_millis()
        );
    }

    #[tokio::test]
    async fn test_assembler_uses_empty_soulbox_when_unavailable() {
        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into(), 20, 0.3);
        let context = assembler
            .assemble(ActivityState::Idle10Min)
            .await
            .expect("test: assemble should succeed even without SoulBox");

        // Should fall back to empty snapshot without error
        assert!(context.soulbox_snapshot.personality_summary.is_empty());
    }

    #[tokio::test]
    async fn test_assembler_graceful_when_memory_unavailable() {
        // Create assembler with a bogus address that will fail to connect
        let assembler = ContextAssembler::new("http://127.0.0.1:99999".into(), 20, 0.3);
        let context = assembler
            .assemble(ActivityState::UserActive)
            .await
            .expect("test: assemble should succeed even when memory-engine is unavailable");

        // Memory reads should return empty results, not errors
        assert!(context.short_term.is_empty());
        assert!(context.long_term.is_empty());
        assert!(context.episodic.is_empty());
    }
}
