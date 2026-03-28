//! Instruction following probe — multi-step instruction compliance test.
//!
//! Sends a structured multi-step instruction prompt to the model and scores
//! how accurately the response matches the expected output format. The score
//! is a float 0.0–1.0 representing the fraction of instruction steps followed
//! correctly, then mapped to a `CapabilityLevel` using config thresholds.

use std::time::Instant;

use crate::config::InstructionFollowingProbeConfig;
use crate::error::SenaResult;
use crate::generated::sena_daemonbus_v1::inference_service_client::InferenceServiceClient;
use crate::generated::sena_daemonbus_v1::CompleteRequest;
use crate::probes::{CapabilityLevel, ProbeResult};
use tonic::transport::Channel;

/// Run the instruction following probe against the active model.
///
/// Sends a multi-step instruction prompt via gRPC to InferenceService and
/// compares the response line-by-line against the expected output.
///
/// # Graceful Degradation
/// When `inference_client` is None or the inference call fails:
/// 1. Logs a warning
/// 2. Returns score 0.0 with `degraded: true`
/// 3. Does NOT return an error (error would abort the battery)
pub async fn run(
    config: &InstructionFollowingProbeConfig,
    model_id: &str,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<ProbeResult> {
    let start = Instant::now();

    let degraded = inference_client.is_none();

    if degraded {
        tracing::warn!(
            subsystem = "model_probe",
            probe_name = "instruction_following",
            reason = "inference_unavailable",
            "probe degraded — InferenceService client not available"
        );
    }

    let model_response = if let Some(mut client) = inference_client {
        match run_with_inference(&mut client, &config.probe_prompt).await {
            Ok(response) => response,
            Err(inference_error) => {
                tracing::warn!(
                    subsystem = "model_probe",
                    probe_name = "instruction_following",
                    event_type = "inference_failed",
                    error = %inference_error,
                    "inference call failed — returning degraded result"
                );
                String::new()
            }
        }
    } else {
        String::new()
    };

    let raw_score = score_instruction_compliance(
        &model_response,
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
        degraded = degraded || model_response.is_empty(),
        "probe completed"
    );

    Ok(ProbeResult {
        probe_name: "instruction_following".to_string(),
        raw_score,
        capability_level: Some(capability_level),
        duration,
        degraded: degraded || model_response.is_empty(),
    })
}

/// Send the instruction prompt to the model via InferenceService gRPC.
async fn run_with_inference(
    client: &mut InferenceServiceClient<Channel>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let request = CompleteRequest {
        prompt: prompt.to_string(),
        model_id: String::new(),
        max_tokens: 256,
        temperature: 0.0,
        priority: 3, // Standard priority for probes
        request_id: format!("probe-instruction-{}", uuid_v4_simple()),
    };

    let response = client.complete(request).await?;
    let inner = response.into_inner();
    Ok(inner.text)
}

/// Generate a simple pseudo-random request ID (no external crate dependency).
fn uuid_v4_simple() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
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
