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
    let capabilities = vec![
        THOUGHT_GENERATION.to_string(),
        THOUGHT_EVALUATION.to_string(),
        RELEVANCE_SCORING.to_string(),
    ];

    // TODO: Once telemetry integration is implemented, check if using
    // synthetic telemetry and add TELEMETRY_SYNTHETIC if so.
    // For now, CTP does not have telemetry integration yet.

    capabilities
}
