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

/// Returns the list of capabilities the model-probe subsystem provides,
/// with `:degraded` suffix for probes that ran in degraded mode (no inference).
pub fn get_capabilities(degraded_probes: &[String]) -> Vec<String> {
    let all = [
        CONTEXT_WINDOW_PROBE,
        GRAPH_EXTRACTION_PROBE,
        INSTRUCTION_FOLLOWING_PROBE,
        STRUCTURED_OUTPUT_PROBE,
    ];

    all.iter()
        .map(|&cap| {
            // Strip the "_probe" suffix to match degraded_probes naming (e.g. "context_window")
            let probe_name = cap.strip_suffix("_probe").unwrap_or(cap);
            if degraded_probes.iter().any(|d| d == probe_name) {
                format!("{cap}:degraded")
            } else {
                cap.to_string()
            }
        })
        .collect()
}


