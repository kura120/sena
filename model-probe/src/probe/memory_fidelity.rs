//! Memory injection fidelity probe.
//!
//! Injects a known fact into the model's context and verifies the model can
//! retrieve and reason about it correctly. The fidelity score (0.0–1.0)
//! determines how much tiered context PC injects at runtime — shallow models
//! receive simplified context, deep models receive full tiered context.
//!
//! Scoring: fraction of expected keywords found in the model's response.
//! All thresholds come from config — nothing hardcoded here.

use std::time::Instant;

use crate::config::MemoryFidelityProbeConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::inference_service_client::InferenceServiceClient;
use crate::generated::sena_daemonbus_v1::CompleteRequest;
use tonic::transport::Channel;

/// Result of the memory injection fidelity probe.
#[derive(Debug, Clone)]
pub struct MemoryFidelityResult {
    /// Raw fidelity score between 0.0 and 1.0.
    pub fidelity_score: f32,
    /// Wall-clock duration of the probe in milliseconds.
    pub duration_ms: u64,
    /// Whether this probe ran in degraded mode (no inference available).
    pub degraded: bool,
}

/// Run the memory injection fidelity probe against the active model.
///
/// Constructs a prompt with `config.injected_fact` as context, asks
/// `config.probe_prompt`, and scores the response via keyword overlap
/// against `config.expected_answer`.
///
/// # Graceful Degradation
/// When `inference_client` is None or inference fails:
/// 1. Logs a warning
/// 2. Returns score 0.0 with `degraded: true`
/// 3. Does NOT return an error
pub async fn run(
    config: &MemoryFidelityProbeConfig,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<MemoryFidelityResult> {
    let start = Instant::now();

    // Validate config fields
    if config.injected_fact.is_empty() {
        return Err(SenaError::new(
            ErrorCode::ProbeFailed,
            "memory_fidelity probe config has empty injected_fact",
        ));
    }
    if config.expected_answer.is_empty() {
        return Err(SenaError::new(
            ErrorCode::ProbeFailed,
            "memory_fidelity probe config has empty expected_answer",
        ));
    }

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "memory_fidelity",
        event_type = "probe_started",
        "memory fidelity probe starting"
    );

    let degraded = inference_client.is_none();

    if degraded {
        tracing::warn!(
            subsystem = "model_probe",
            probe_name = "memory_fidelity",
            reason = "inference_unavailable",
            "probe degraded — InferenceService client not available"
        );
    }

    let fidelity_score = if let Some(mut client) = inference_client {
        let prompt = format!("{}\n\n{}", config.injected_fact, config.probe_prompt);

        let request = CompleteRequest {
            prompt,
            model_id: String::new(),
            max_tokens: 256,
            temperature: 0.0,
            priority: 3,
            request_id: format!("probe-memory-fidelity-{}", timestamp_nanos()),
        };

        match client.complete(request).await {
            Ok(response) => {
                let text = response.into_inner().text;
                score_keyword_overlap(&text, &config.expected_answer)
            }
            Err(inference_error) => {
                tracing::warn!(
                    subsystem = "model_probe",
                    probe_name = "memory_fidelity",
                    event_type = "inference_failed",
                    error = %inference_error,
                    "inference call failed — returning degraded result"
                );
                0.0
            }
        }
    } else {
        0.0
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "memory_fidelity",
        event_type = "probe_completed",
        score = fidelity_score,
        duration_ms = duration_ms,
        degraded = degraded || fidelity_score == 0.0,
        "memory fidelity probe completed"
    );

    Ok(MemoryFidelityResult {
        fidelity_score,
        duration_ms,
        degraded: degraded || fidelity_score == 0.0,
    })
}

fn timestamp_nanos() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Score a model response against the expected answer by keyword overlap.
///
/// Not yet called by the stub, but provided so the scoring logic compiles
/// and can be unit-tested independently of the inference backend.
///
/// Returns a score between 0.0 and 1.0 representing the fraction of
/// whitespace-delimited tokens in `expected` that appear (case-insensitive)
/// somewhere in `response`.
pub fn score_keyword_overlap(response: &str, expected: &str) -> f32 {
    let response_lower = response.to_lowercase();
    let expected_tokens: Vec<&str> = expected.split_whitespace().collect();

    if expected_tokens.is_empty() {
        return 0.0;
    }

    let matched_count = expected_tokens
        .iter()
        .filter(|token| response_lower.contains(&token.to_lowercase()))
        .count();

    matched_count as f32 / expected_tokens.len() as f32
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_overlap_all_present() {
        let response = "The deadline is March 15 2025 and the lead is Dr. Ravi Patel.";
        let expected = "March 15 2025 Dr. Ravi Patel";
        let score = score_keyword_overlap(response, expected);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "all keywords present should score 1.0, got {score}"
        );
    }

    #[test]
    fn keyword_overlap_none_present() {
        let response = "I don't know the answer.";
        let expected = "March 15 2025 Dr. Ravi Patel";
        let score = score_keyword_overlap(response, expected);
        assert!(
            score < 0.5,
            "no keywords present should score low, got {score}"
        );
    }

    #[test]
    fn keyword_overlap_partial() {
        let response = "The deadline is March 15 2025.";
        let expected = "March 15 2025 Dr. Ravi Patel";
        let score = score_keyword_overlap(response, expected);
        assert!(
            score > 0.0 && score < 1.0,
            "partial keywords should score between 0 and 1, got {score}"
        );
    }

    #[test]
    fn keyword_overlap_case_insensitive() {
        let response = "MARCH 15 2025 DR. RAVI PATEL";
        let expected = "March 15 2025 Dr. Ravi Patel";
        let score = score_keyword_overlap(response, expected);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "case-insensitive match should score 1.0, got {score}"
        );
    }

    #[test]
    fn keyword_overlap_empty_expected() {
        let score = score_keyword_overlap("some response", "");
        assert!(
            score.abs() < f32::EPSILON,
            "empty expected should score 0.0, got {score}"
        );
    }

    #[test]
    fn keyword_overlap_empty_response() {
        let score = score_keyword_overlap("", "March 15 2025");
        assert!(
            score.abs() < f32::EPSILON,
            "empty response should score 0.0, got {score}"
        );
    }

    fn test_config() -> MemoryFidelityProbeConfig {
        MemoryFidelityProbeConfig {
            injected_fact: "Project Zenith's deadline is March 15, 2025.".to_string(),
            probe_prompt: "When is the deadline?".to_string(),
            expected_answer: "March 15, 2025".to_string(),
            pass_threshold: 0.7,
        }
    }

    #[test]
    fn stub_returns_zero() {
        let config = test_config();
        let score = run_stub(&config).expect("stub should not fail with valid config");
        assert!(
            score.abs() < f32::EPSILON,
            "stub should return 0.0, got {score}"
        );
    }

    #[test]
    fn stub_rejects_empty_injected_fact() {
        let mut config = test_config();
        config.injected_fact = String::new();
        let result = run_stub(&config);
        assert!(result.is_err());
    }

    #[test]
    fn stub_rejects_empty_expected_answer() {
        let mut config = test_config();
        config.expected_answer = String::new();
        let result = run_stub(&config);
        assert!(result.is_err());
    }
}
