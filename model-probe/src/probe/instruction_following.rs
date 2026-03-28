//! Instruction following probe — multi-step instruction compliance test.
//!
//! Sends a structured multi-step instruction prompt to the model and scores
//! how accurately the response matches the expected output format. The score
//! is a float 0.0–1.0 representing the fraction of instruction steps followed
//! correctly, then mapped to a `CapabilityLevel` using config thresholds.

use std::time::Instant;

use crate::config::InstructionFollowingProbeConfig;
use crate::error::SenaResult;
use crate::probes::{CapabilityLevel, ProbeResult};

/// Run the instruction following probe against the active model.
///
/// The probe sends a multi-step instruction prompt with a precise expected
/// output format. Each line of the response is compared against the expected
/// output to compute a compliance score.
///
/// # Scoring
/// - Each matching line contributes equally to the score.
/// - Lines are compared after trimming whitespace.
/// - The raw score is the fraction of expected lines that match.
/// - Score is mapped to `CapabilityLevel` via config thresholds.
pub async fn run(
    config: &InstructionFollowingProbeConfig,
    model_id: &str,
) -> SenaResult<ProbeResult> {
    let start = Instant::now();

    // TODO(implementation): Send config.probe_prompt to the model via llama-cpp-rs
    // with temperature=0, max_tokens from global probe config, then compare
    // the response against config.expected_output line by line.
    //
    // Stub: return a placeholder result indicating the probe did not run.

    let raw_score = score_instruction_compliance(
        "", // model response — empty until inference is wired
        &config.expected_output,
    );

    let capability_level = derive_capability_level(
        raw_score,
        config.partial_threshold,
        config.full_threshold,
    );

    let duration = start.elapsed();

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "instruction_following",
        model_id = model_id,
        result = %capability_level,
        score = raw_score,
        duration_ms = duration.as_millis() as u64,
        "probe completed"
    );

    Ok(ProbeResult {
        probe_name: "instruction_following".to_string(),
        raw_score,
        capability_level: Some(capability_level),
        duration,
        degraded: true, // TODO: Implement real inference probe
    })
}

/// Score instruction compliance by comparing response lines against expected lines.
///
/// Each expected line that has a matching response line (after trimming) scores
/// equally. Returns a fraction in [0.0, 1.0].
fn score_instruction_compliance(response: &str, expected_output: &str) -> f64 {
    let expected_lines: Vec<&str> = expected_output.lines().collect();
    if expected_lines.is_empty() {
        return 0.0;
    }

    let response_lines: Vec<&str> = response.lines().collect();

    let mut matched_count: usize = 0;
    for (index, expected_line) in expected_lines.iter().enumerate() {
        if let Some(response_line) = response_lines.get(index) {
            if response_line.trim() == expected_line.trim() {
                matched_count += 1;
            }
        }
    }

    matched_count as f64 / expected_lines.len() as f64
}

/// Map a raw score to a `CapabilityLevel` using config-driven thresholds.
fn derive_capability_level(
    score: f64,
    partial_threshold: f64,
    full_threshold: f64,
) -> CapabilityLevel {
    if score >= full_threshold {
        CapabilityLevel::Full
    } else if score >= partial_threshold {
        CapabilityLevel::Partial
    } else {
        CapabilityLevel::None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_match_scores_one() {
        let expected = "ALPHA\n7\nEND";
        let response = "ALPHA\n7\nEND";
        let score = score_instruction_compliance(response, expected);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn partial_match_scores_fraction() {
        let expected = "ALPHA\n7\nEND";
        let response = "ALPHA\n7\nWRONG";
        let score = score_instruction_compliance(response, expected);
        // 2 out of 3 lines match
        assert!((score - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn no_match_scores_zero() {
        let expected = "ALPHA\n7\nEND";
        let response = "wrong\nwrong\nwrong";
        let score = score_instruction_compliance(response, expected);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_response_scores_zero() {
        let expected = "ALPHA\n7\nEND";
        let response = "";
        let score = score_instruction_compliance(response, expected);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_expected_scores_zero() {
        let score = score_instruction_compliance("anything", "");
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn whitespace_trimming_applied() {
        let expected = "ALPHA\n7\nEND";
        let response = "  ALPHA  \n  7  \n  END  ";
        let score = score_instruction_compliance(response, expected);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn capability_level_full_at_threshold() {
        assert_eq!(
            derive_capability_level(0.90, 0.50, 0.90),
            CapabilityLevel::Full
        );
    }

    #[test]
    fn capability_level_partial_between_thresholds() {
        assert_eq!(
            derive_capability_level(0.60, 0.50, 0.90),
            CapabilityLevel::Partial
        );
    }

    #[test]
    fn capability_level_none_below_partial() {
        assert_eq!(
            derive_capability_level(0.30, 0.50, 0.90),
            CapabilityLevel::None
        );
    }

    #[test]
    fn extra_response_lines_ignored() {
        let expected = "ALPHA\n7";
        let response = "ALPHA\n7\nEXTRA\nMORE";
        let score = score_instruction_compliance(response, expected);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }
}
