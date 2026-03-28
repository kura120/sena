//! Capability constants for the ctp subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what features this subsystem provides.

/// Generate candidate thoughts from telemetry and memory signals
pub const THOUGHT_GENERATION: &str = "thought_generation";

/// Evaluate thought quality and coherence
pub const THOUGHT_EVALUATION: &str = "thought_evaluation";

/// Score thoughts for relevance to user context
pub const RELEVANCE_SCORING: &str = "relevance_scoring";

/// Synthetic telemetry mode (for testing without real OS telemetry)
pub const TELEMETRY_SYNTHETIC: &str = "telemetry:synthetic";

/// Returns the list of capabilities the ctp subsystem currently provides.
///
/// This is called when signaling CTP_READY to daemon-bus.
pub fn get_capabilities() -> Vec<String> {
    // CTP currently uses synthetic telemetry (no real OS telemetry until
    // platform layer integration in Milestone D). Report this so downstream
    // consumers know the thought pipeline is not driven by real user activity.
    vec![
        THOUGHT_GENERATION.to_string(),
        THOUGHT_EVALUATION.to_string(),
        RELEVANCE_SCORING.to_string(),
        TELEMETRY_SYNTHETIC.to_string(),
    ]
}
