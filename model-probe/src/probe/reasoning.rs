//! Reasoning quality baseline probe and reasoning gap detection.
//!
//! The reasoning probe establishes a `reasoning_quality` score (0.0–1.0) by
//! presenting a chain-of-thought problem with a known correct answer. The score
//! is stable and comparable across runs — it forms the baseline against which
//! reasoning gap detection computes drift.
//!
//! Reasoning gap detection compares the current score against the score recorded
//! at the last LoRA training run. If the gap exceeds the configured threshold
//! and the model is LoRA-compatible, the caller publishes LORA_TRAINING_RECOMMENDED.
//! model-probe never trains adapters — it only detects and signals.

use std::time::Instant;

use tracing;

use crate::config::ReasoningProbeConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::inference_service_client::InferenceServiceClient;
use crate::generated::sena_daemonbus_v1::CompleteRequest;
use crate::probes::CapabilityLevel;
use tonic::transport::Channel;

/// Result of the reasoning quality baseline probe.
#[derive(Debug, Clone)]
pub struct ReasoningProbeResult {
    /// Raw quality score between 0.0 and 1.0.
    pub reasoning_quality: f32,
    /// Whether the final answer matched the expected answer exactly.
    pub answer_correct: bool,
    /// How many expected reasoning steps were found in the model's output.
    pub matched_reasoning_steps: usize,
    /// Total expected reasoning steps from config.
    pub total_reasoning_steps: usize,
    /// Wall-clock duration of the probe in milliseconds.
    pub duration_ms: u64,
}

/// Result of reasoning gap detection.
#[derive(Debug, Clone)]
pub struct ReasoningGapResult {
    /// Whether LoRA training is recommended based on gap analysis.
    pub lora_training_recommended: bool,
    /// The computed gap (last_trained_score - current_score). Zero if no prior score exists.
    pub gap: f64,
    /// The last training score used for comparison, if available.
    pub last_trained_score: Option<f64>,
}

/// Run the reasoning quality baseline probe.
///
/// Presents a chain-of-thought problem with a known correct answer. The model's
/// response is scored on two axes:
/// 1. Whether the final answer matches `config.expected_answer`
/// 2. How many `config.expected_reasoning_steps` appear in the response
///
/// The two axes are combined into a single 0.0–1.0 score with the final answer
/// weighted at 60% and reasoning step coverage at 40%. These weights reflect that
/// arriving at the correct answer matters more than showing work, but showing
/// coherent work is still valuable signal for gap detection.
pub async fn run_reasoning_probe(
    config: &ReasoningProbeConfig,
    _model_id: &str,
    per_probe_timeout_ms: u64,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<ReasoningProbeResult> {
    let config_clone = config.clone();
    let timeout_duration = std::time::Duration::from_millis(per_probe_timeout_ms);

    let result = tokio::time::timeout(timeout_duration, async {
        run_reasoning_probe_inner(&config_clone, inference_client).await
    })
    .await;

    match result {
        Ok(inner_result) => inner_result,
        Err(_elapsed) => {
            tracing::error!(
                subsystem = "model_probe",
                probe_name = "reasoning",
                event_type = "probe_timeout",
                timeout_ms = per_probe_timeout_ms,
                "reasoning probe timed out"
            );
            Err(SenaError::new(
                ErrorCode::ProbeTimeout,
                "reasoning probe exceeded per-probe timeout",
            ))
        }
    }
}

/// Inner implementation — sends the reasoning prompt to inference and scores the response.
async fn run_reasoning_probe_inner(
    config: &ReasoningProbeConfig,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<ReasoningProbeResult> {
    let start = Instant::now();

    let degraded = inference_client.is_none();

    if degraded {
        tracing::warn!(
            subsystem = "model_probe",
            probe_name = "reasoning",
            reason = "inference_unavailable",
            "probe degraded — InferenceService client not available"
        );
    }

    let model_response = if let Some(mut client) = inference_client {
        let request = CompleteRequest {
            prompt: config.probe_prompt.clone(),
            model_id: String::new(),
            max_tokens: 256,
            temperature: 0.0,
            priority: 3,
            request_id: format!("probe-reasoning-{}", timestamp_nanos()),
        };

        match client.complete(request).await {
            Ok(response) => response.into_inner().text,
            Err(inference_error) => {
                tracing::warn!(
                    subsystem = "model_probe",
                    probe_name = "reasoning",
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

    let answer_correct = check_final_answer(&model_response, &config.expected_answer);
    let matched_reasoning_steps =
        count_matched_reasoning_steps(&model_response, &config.expected_reasoning_steps);
    let total_reasoning_steps = config.expected_reasoning_steps.len();

    let reasoning_quality = compute_reasoning_score(
        answer_correct,
        matched_reasoning_steps,
        total_reasoning_steps,
        config.answer_weight as f32,
        config.reasoning_steps_weight as f32,
    );

    let duration_ms = start.elapsed().as_millis() as u64;

    let result = ReasoningProbeResult {
        reasoning_quality,
        answer_correct,
        matched_reasoning_steps,
        total_reasoning_steps,
        duration_ms,
    };

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "reasoning",
        event_type = "probe_completed",
        score = result.reasoning_quality,
        answer_correct = result.answer_correct,
        matched_steps = result.matched_reasoning_steps,
        total_steps = result.total_reasoning_steps,
        duration_ms = result.duration_ms,
        degraded = degraded || model_response.is_empty(),
        "reasoning quality probe completed"
    );

    Ok(result)
}

fn timestamp_nanos() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Check whether the model's final answer matches the expected answer.
///
/// Extracts the last non-empty line from the response and compares it
/// (case-insensitive, trimmed) against the expected answer. This is
/// deliberately strict — the probe prompt instructs the model to put
/// the final answer on the last line.
fn check_final_answer(model_response: &str, expected_answer: &str) -> bool {
    let last_line = model_response
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty());

    match last_line {
        Some(line) => line.trim().eq_ignore_ascii_case(expected_answer.trim()),
        None => false,
    }
}

/// Count how many expected reasoning steps appear in the model's response.
///
/// Each step is checked as a case-insensitive substring match. A step that
/// appears anywhere in the response counts as matched.
fn count_matched_reasoning_steps(model_response: &str, expected_steps: &[String]) -> usize {
    let response_lower = model_response.to_lowercase();

    expected_steps
        .iter()
        .filter(|step| response_lower.contains(&step.to_lowercase()))
        .count()
}

/// Combine answer correctness and reasoning step coverage into a single 0.0–1.0 score.
///
/// Weighting: 60% final answer, 40% reasoning step coverage.
/// Weights are config-driven via `answer_weight` and `reasoning_steps_weight`.
fn compute_reasoning_score(
    answer_correct: bool,
    matched_steps: usize,
    total_steps: usize,
    answer_weight: f32,
    reasoning_steps_weight: f32,
) -> f32 {
    let answer_component = if answer_correct { answer_weight } else { 0.0 };

    let step_coverage = if total_steps > 0 {
        matched_steps as f32 / total_steps as f32
    } else {
        0.0
    };
    let step_component = step_coverage * reasoning_steps_weight;

    answer_component + step_component
}

/// Derive a `CapabilityLevel` from a reasoning quality score using the
/// config-driven quality threshold.
///
/// The reasoning probe uses a single threshold (not partial/full) because
/// the raw score is what matters for gap detection. The CapabilityLevel is
/// derived for downstream gating only.
pub fn derive_reasoning_capability(
    reasoning_quality: f32,
    quality_threshold: f64,
) -> CapabilityLevel {
    if reasoning_quality as f64 >= quality_threshold {
        CapabilityLevel::Full
    } else if reasoning_quality > 0.0 {
        CapabilityLevel::Partial
    } else {
        CapabilityLevel::None
    }
}

/// Run reasoning gap detection against a prior training score.
///
/// If no prior score exists (first run, or no LoRA training has ever occurred),
/// gap detection does not fire — there is no baseline to compare against.
///
/// Gap = last_trained_score - current_score. Only positive gaps (regression)
/// trigger the recommendation. If the model improved, gap is zero.
pub fn detect_reasoning_gap(
    current_score: f64,
    last_trained_score: Option<f64>,
    trigger_threshold: f64,
    lora_compatible: bool,
) -> ReasoningGapResult {
    let (gap, lora_training_recommended) = match last_trained_score {
        Some(trained_score) => {
            // Only positive gaps (performance regression) matter.
            let computed_gap = (trained_score - current_score).max(0.0);
            let recommended = computed_gap > trigger_threshold && lora_compatible;

            if recommended {
                tracing::info!(
                    subsystem = "model_probe",
                    event_type = "reasoning_gap_detected",
                    current_score = current_score,
                    last_trained_score = trained_score,
                    gap = computed_gap,
                    trigger_threshold = trigger_threshold,
                    "reasoning gap exceeds threshold — LoRA training recommended"
                );
            }

            (computed_gap, recommended)
        }
        None => {
            // No prior training score — cannot compute gap. This is expected
            // on first boot or when no LoRA training has ever been performed.
            tracing::debug!(
                subsystem = "model_probe",
                event_type = "reasoning_gap_skipped",
                reason = "no prior training score available",
                "reasoning gap detection skipped — no baseline"
            );
            (0.0, false)
        }
    };

    ReasoningGapResult {
        lora_training_recommended,
        gap,
        last_trained_score,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_final_answer_exact_match() {
        assert!(check_final_answer("Some reasoning\n8", "8"));
    }

    #[test]
    fn check_final_answer_case_insensitive() {
        assert!(check_final_answer("thinking...\nEIGHT", "eight"));
    }

    #[test]
    fn check_final_answer_with_trailing_whitespace() {
        assert!(check_final_answer("reasoning\n  8  \n", "8"));
    }

    #[test]
    fn check_final_answer_empty_response() {
        assert!(!check_final_answer("", "8"));
    }

    #[test]
    fn check_final_answer_wrong_answer() {
        assert!(!check_final_answer("The answer is 15\n15", "8"));
    }

    #[test]
    fn count_steps_all_present() {
        let response = "All but 8 die means 8 sheep remain. The answer is 8.";
        let steps = vec![
            "all but 8".to_string(),
            "8 sheep remain".to_string(),
            "8".to_string(),
        ];
        assert_eq!(count_matched_reasoning_steps(response, &steps), 3);
    }

    #[test]
    fn count_steps_partial_match() {
        let response = "All but 8 die. Answer: 8.";
        let steps = vec![
            "all but 8".to_string(),
            "8 sheep remain".to_string(),
            "8".to_string(),
        ];
        assert_eq!(count_matched_reasoning_steps(response, &steps), 2);
    }

    #[test]
    fn count_steps_none_present() {
        let response = "I don't know.";
        let steps = vec!["all but 8".to_string(), "8 sheep remain".to_string()];
        assert_eq!(count_matched_reasoning_steps(response, &steps), 0);
    }

    #[test]
    fn count_steps_empty_response() {
        let steps = vec!["all but 8".to_string()];
        assert_eq!(count_matched_reasoning_steps("", &steps), 0);
    }

    #[test]
    fn count_steps_empty_expected() {
        assert_eq!(count_matched_reasoning_steps("anything", &[]), 0);
    }

    #[test]
    fn score_perfect() {
        let score = compute_reasoning_score(true, 3, 3, 0.6, 0.4);
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn score_correct_answer_no_steps() {
        let score = compute_reasoning_score(true, 0, 3, 0.6, 0.4);
        assert!((score - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn score_wrong_answer_all_steps() {
        let score = compute_reasoning_score(false, 3, 3, 0.6, 0.4);
        assert!((score - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn score_wrong_answer_no_steps() {
        let score = compute_reasoning_score(false, 0, 3, 0.6, 0.4);
        assert!((score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn score_correct_answer_partial_steps() {
        let score = compute_reasoning_score(true, 1, 2, 0.6, 0.4);
        // 0.6 + (0.5 * 0.4) = 0.8
        assert!((score - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn score_with_zero_total_steps() {
        // Edge case: no expected steps configured. Only answer matters.
        let score = compute_reasoning_score(true, 0, 0, 0.6, 0.4);
        assert!((score - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn derive_capability_full_above_threshold() {
        assert_eq!(
            derive_reasoning_capability(0.85, 0.60),
            CapabilityLevel::Full
        );
    }

    #[test]
    fn derive_capability_full_at_threshold() {
        assert_eq!(
            derive_reasoning_capability(0.60, 0.60),
            CapabilityLevel::Full
        );
    }

    #[test]
    fn derive_capability_partial_below_threshold() {
        assert_eq!(
            derive_reasoning_capability(0.30, 0.60),
            CapabilityLevel::Partial
        );
    }

    #[test]
    fn derive_capability_none_at_zero() {
        assert_eq!(
            derive_reasoning_capability(0.0, 0.60),
            CapabilityLevel::None
        );
    }

    #[test]
    fn gap_detection_no_prior_score() {
        let result = detect_reasoning_gap(0.7, None, 0.15, true);
        assert!(!result.lora_training_recommended);
        assert!((result.gap - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gap_detection_gap_below_threshold() {
        let result = detect_reasoning_gap(0.80, Some(0.90), 0.15, true);
        assert!(!result.lora_training_recommended);
        assert!((result.gap - 0.10).abs() < f64::EPSILON);
    }

    #[test]
    fn gap_detection_gap_above_threshold_lora_compatible() {
        let result = detect_reasoning_gap(0.60, Some(0.90), 0.15, true);
        assert!(result.lora_training_recommended);
        assert!((result.gap - 0.30).abs() < f64::EPSILON);
    }

    #[test]
    fn gap_detection_gap_above_threshold_lora_incompatible() {
        // Gap exceeds threshold but model is not LoRA compatible — no recommendation.
        let result = detect_reasoning_gap(0.60, Some(0.90), 0.15, false);
        assert!(!result.lora_training_recommended);
        assert!((result.gap - 0.30).abs() < f64::EPSILON);
    }

    #[test]
    fn gap_detection_model_improved() {
        // Current score is higher than last trained — no regression, gap is zero.
        let result = detect_reasoning_gap(0.95, Some(0.80), 0.15, true);
        assert!(!result.lora_training_recommended);
        assert!((result.gap - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gap_detection_exact_threshold_does_not_trigger() {
        // Gap equals threshold exactly — does not trigger (must exceed, not equal).
        // Uses 0.50 and 0.25 to avoid f64 subtraction imprecision that plagues
        // values like 0.90 - 0.75 (which produces 0.15000000000000002).
        let result = detect_reasoning_gap(0.50, Some(0.75), 0.25, true);
        assert!(!result.lora_training_recommended);
        assert!((result.gap - 0.25).abs() < f64::EPSILON);
    }
}
