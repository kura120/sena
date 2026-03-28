//! Capability constants for the prompt-composer subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what features this subsystem provides.

/// Assemble prompts from context and template
pub const PROMPT_ASSEMBLY: &str = "prompt_assembly";

/// Encode prompts in TOON format
pub const TOON_ENCODING: &str = "toon_encoding";

/// Manage context window budget and prioritization
pub const CONTEXT_BUDGET_MANAGEMENT: &str = "context_budget_management";

/// Simplified TOON encoding (for early development or degraded mode)
pub const TOON_ENCODING_SIMPLIFIED: &str = "toon_encoding:simplified";

/// Returns the list of capabilities the prompt-composer subsystem currently provides.
///
/// This is called when signaling PROMPT_COMPOSER_READY to daemon-bus.
pub fn get_capabilities() -> Vec<String> {
    vec![
        PROMPT_ASSEMBLY.to_string(),
        TOON_ENCODING.to_string(),
        CONTEXT_BUDGET_MANAGEMENT.to_string(),
    ]
}

/// Returns capabilities when using simplified TOON encoding.
///
/// This can be used during early development or when the full TOON
/// specification is not yet implemented.
#[allow(dead_code)]
pub fn get_capabilities_simplified() -> Vec<String> {
    vec![
        PROMPT_ASSEMBLY.to_string(),
        TOON_ENCODING_SIMPLIFIED.to_string(),
        CONTEXT_BUDGET_MANAGEMENT.to_string(),
    ]
}
