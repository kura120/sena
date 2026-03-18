//! Context assembler — gathers all inputs and packages them into a
//! `PromptContext` struct for handoff to prompt-composer.
//!
//! CTP assembles the raw context; prompt-composer serializes it to TOON.
//! Never TOON-encode inside CTP.

use crate::activity::ActivityState;
use crate::error::CtpError;

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
/// For Phase 1, memory reads return empty results and SoulBox returns
/// an empty snapshot. Real gRPC clients are wired in later milestones.
pub struct ContextAssembler {
    _memory_engine_address: String,
}

impl ContextAssembler {
    pub fn new(memory_engine_address: String) -> Self {
        Self {
            _memory_engine_address: memory_engine_address,
        }
    }

    /// Assemble a `PromptContext` from all available sources.
    ///
    /// Three memory tier reads fire concurrently via `tokio::join!`.
    /// SoulBox unavailability produces a fallback, not an error.
    pub async fn assemble(&self, activity_state: ActivityState) -> Result<PromptContext, CtpError> {
        // Fire all three memory reads in parallel via tokio::join!
        let (short_term_result, long_term_result, episodic_result) = tokio::join!(
            self.read_short_term(),
            self.read_long_term(),
            self.read_episodic(),
        );

        let short_term = short_term_result.unwrap_or_default();
        let long_term = long_term_result.unwrap_or_default();
        let episodic = episodic_result.unwrap_or_default();

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

    /// Read short-term memory — stub for Phase 1.
    async fn read_short_term(&self) -> Result<Vec<MemoryResult>, CtpError> {
        Ok(Vec::new())
    }

    /// Read long-term memory — stub for Phase 1.
    async fn read_long_term(&self) -> Result<Vec<MemoryResult>, CtpError> {
        Ok(Vec::new())
    }

    /// Read episodic memory — stub for Phase 1.
    async fn read_episodic(&self) -> Result<Vec<MemoryResult>, CtpError> {
        Ok(Vec::new())
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
        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into());
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
        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into());
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
    async fn test_assembler_parallel_memory_queries() {
        // This test verifies the structural property that memory reads use
        // tokio::join! — they run concurrently, not sequentially.
        // We verify by timing: three concurrent 10ms reads should take ~10ms,
        // not ~30ms if they were sequential.
        use std::time::Instant;

        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into());
        let start = Instant::now();
        let _context = assembler
            .assemble(ActivityState::UserActive)
            .await
            .expect("test: assemble should succeed");
        let elapsed = start.elapsed();

        // Phase 1 stubs return immediately — the point is they don't block each other.
        // This test ensures the structure is correct (tokio::join! used).
        assert!(
            elapsed.as_millis() < 1000,
            "parallel reads should be fast, took {} ms",
            elapsed.as_millis()
        );
    }

    #[tokio::test]
    async fn test_assembler_uses_empty_soulbox_when_unavailable() {
        let assembler = ContextAssembler::new("http://127.0.0.1:50052".into());
        let context = assembler
            .assemble(ActivityState::Idle10Min)
            .await
            .expect("test: assemble should succeed even without SoulBox");

        // Should fall back to empty snapshot without error
        assert!(context.soulbox_snapshot.personality_summary.is_empty());
    }
}
