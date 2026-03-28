//! Capability constants for the model-probe subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what probes this subsystem provides.
//!
//! Format:
//! - `probe_name` for operational probes
//! - `probe_name:degraded` for probes that passed with degraded results

/// Context window size detection probe
pub const CONTEXT_WINDOW_PROBE: &str = "context_window_probe";

/// Graph/entity extraction capability probe
pub const GRAPH_EXTRACTION_PROBE: &str = "graph_extraction_probe";

/// Instruction following quality probe
pub const INSTRUCTION_FOLLOWING_PROBE: &str = "instruction_following_probe";

/// Structured output (JSON) generation probe
pub const STRUCTURED_OUTPUT_PROBE: &str = "structured_output_probe";

/// Returns the list of capabilities the model-probe subsystem will provide.
///
/// This is called when signaling MODEL_PROFILE_READY to daemon-bus.
/// In the current implementation, probes are not yet checking for degraded
/// results, so all capabilities are reported as operational.
///
/// TODO: When probe results are implemented, check each probe's outcome
/// and append `:degraded` suffix for probes that passed with degraded quality.
pub fn get_capabilities() -> Vec<String> {
    vec![
        CONTEXT_WINDOW_PROBE.to_string(),
        GRAPH_EXTRACTION_PROBE.to_string(),
        INSTRUCTION_FOLLOWING_PROBE.to_string(),
        STRUCTURED_OUTPUT_PROBE.to_string(),
    ]
}

/// Returns capabilities with degraded markers for failed or low-quality probes.
///
/// This function will be used once probe result evaluation is implemented.
#[allow(dead_code)]
pub fn get_capabilities_with_degraded(probe_results: &ProbeResults) -> Vec<String> {
    let mut capabilities = Vec::new();

    capabilities.push(if probe_results.context_window_degraded {
        format!("{}:degraded", CONTEXT_WINDOW_PROBE)
    } else {
        CONTEXT_WINDOW_PROBE.to_string()
    });

    capabilities.push(if probe_results.graph_extraction_degraded {
        format!("{}:degraded", GRAPH_EXTRACTION_PROBE)
    } else {
        GRAPH_EXTRACTION_PROBE.to_string()
    });

    capabilities.push(if probe_results.instruction_following_degraded {
        format!("{}:degraded", INSTRUCTION_FOLLOWING_PROBE)
    } else {
        INSTRUCTION_FOLLOWING_PROBE.to_string()
    });

    capabilities.push(if probe_results.structured_output_degraded {
        format!("{}:degraded", STRUCTURED_OUTPUT_PROBE)
    } else {
        STRUCTURED_OUTPUT_PROBE.to_string()
    });

    capabilities
}

/// Placeholder struct for probe results evaluation.
/// This will be replaced with actual probe result types when implemented.
#[allow(dead_code)]
pub struct ProbeResults {
    pub context_window_degraded: bool,
    pub graph_extraction_degraded: bool,
    pub instruction_following_degraded: bool,
    pub structured_output_degraded: bool,
}
