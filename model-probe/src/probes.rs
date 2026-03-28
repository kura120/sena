//! Concurrent probe runner and profile assembly for model-probe.
//!
//! Orchestrates the full probe battery against the active model, runs
//! independent probes concurrently via `tokio::join!`, assembles the
//! `ModelCapabilityProfile` from individual probe results, and handles
//! reasoning gap detection (which depends on the reasoning probe result).
//!
//! All thresholds and scoring parameters come from config — nothing hardcoded.

use std::fmt;
use std::time::Instant;

use crate::config::ModelProbeConfig;
use crate::error::{SenaError, SenaResult};
use crate::probe::{
    context_window, graph_extraction, instruction_following, lora_compat, memory_fidelity,
    reasoning, structured_output,
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared types used by all probe modules
// ─────────────────────────────────────────────────────────────────────────────

/// Graduated capability level for probe results.
///
/// Three tiers so downstream subsystems (PC, CTP) can make nuanced decisions
/// rather than binary pass/fail. Maps to PRD's capability gating requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CapabilityLevel {
    /// Probe passed — full capability available.
    Full,
    /// Probe passed partially — use with fallbacks.
    Partial,
    /// Probe failed completely — capability unavailable.
    None,
}

impl CapabilityLevel {
    /// Derive a `CapabilityLevel` from a numeric score using config-driven thresholds.
    ///
    /// - `>= full_threshold` → `Full`
    /// - `>= partial_threshold` → `Partial`
    /// - below partial → `None`
    pub fn from_score(score: f64, partial_threshold: f64, full_threshold: f64) -> Self {
        if score >= full_threshold {
            CapabilityLevel::Full
        } else if score >= partial_threshold {
            CapabilityLevel::Partial
        } else {
            CapabilityLevel::None
        }
    }
}

impl fmt::Display for CapabilityLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityLevel::Full => write!(formatter, "Full"),
            CapabilityLevel::Partial => write!(formatter, "Partial"),
            CapabilityLevel::None => write!(formatter, "None"),
        }
    }
}

/// The complete model capability profile published to daemon-bus.
///
/// Every field is populated from a specific probe result. Fields are documented
/// with which probe populates them and which downstream subsystem consumes them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelCapabilityProfile {
    /// Identifier of the probed model (derived from model file path or metadata).
    pub model_id: String,
    /// Advertised context window in tokens — for reference only, never used as budget.
    pub context_window: u32,
    /// Conservative token budget before performance cliff. Set by the context
    /// window probe. This is the real ceiling used by prompt-composer.
    pub pre_rot_threshold: u32,
    /// Whether the model can produce reliable structured output (TOON/JSON).
    /// Set by the structured output probe. Consumed by prompt-composer.
    pub structured_output: CapabilityLevel,
    /// How well the model follows multi-step instructions.
    /// Set by the instruction following probe. Consumed by prompt-composer.
    pub instruction_following: CapabilityLevel,
    /// Raw reasoning quality score (0.0–1.0). Set by the reasoning probe.
    /// Used as baseline for reasoning gap detection across runs.
    pub reasoning_quality: f32,
    /// Whether the model's architecture supports LoRA adapter training.
    /// Set by the LoRA compatibility probe. Gates lora-manager.
    pub lora_compatible: bool,
    /// How accurately the model retrieves facts from injected context (0.0–1.0).
    /// Set by the memory fidelity probe. Consumed by prompt-composer to decide
    /// memory injection depth.
    pub memory_injection_fidelity: f32,
    /// Whether the model can extract knowledge graphs from text.
    /// Set by the graph extraction probe. Gates ech0 graph vs vector-only mode.
    pub graph_extraction: CapabilityLevel,
    /// Whether reasoning gap detection recommends a LoRA training cycle.
    /// Set by comparing current reasoning_quality against prior training score.
    pub lora_training_recommended: bool,
    /// List of probe names that ran in degraded mode (formula/stub fallback
    /// due to inference unavailability). Empty list means all probes used real inference.
    /// When non-empty, downstream systems should treat capabilities as conservative estimates.
    pub degraded_probes: Vec<String>,
}

/// Outcome of the full probe battery — carries both profiles and any
/// event signals that main.rs needs to publish to daemon-bus.
#[derive(Debug)]
pub struct ProbeBatteryOutcome {
    /// The assembled model capability profile.
    pub model_profile: ModelCapabilityProfile,
    /// Whether LORA_TRAINING_RECOMMENDED should be published to daemon-bus.
    pub lora_training_recommended: bool,
    /// Total wall-clock duration of the entire probe battery.
    pub total_duration_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe battery runner
// ─────────────────────────────────────────────────────────────────────────────

/// Run the full probe battery against the active model and assemble the profile.
///
/// Independent probes run concurrently via `tokio::join!`. The reasoning gap
/// detection runs sequentially after the reasoning probe because it depends
/// on the reasoning quality score.
///
/// # Arguments
/// * `config` — the full model-probe config (all probe thresholds and model settings)
/// * `model_id` — identifier for the active model
/// * `last_lora_training_score` — reasoning score from the last LoRA training run,
///   if available. `None` on first boot or when no training has occurred.
/// * `inference_client` — optional gRPC client to InferenceService; if None, all probes degrade
///
/// # Errors
/// Returns `SenaError` with `ProbeBatteryFailed` if a critical probe fails in a
/// way that prevents profile assembly. Individual probe failures are logged and
/// scored as `CapabilityLevel::None` / `0.0` — they do not abort the battery.
pub async fn run_probe_battery(
    config: &ModelProbeConfig,
    model_id: &str,
    last_lora_training_score: Option<f64>,
    inference_client: Option<crate::generated::sena_daemonbus_v1::inference_service_client::InferenceServiceClient<tonic::transport::Channel>>,
) -> SenaResult<ProbeBatteryOutcome> {
    let battery_start = Instant::now();
    let per_probe_timeout_ms = config.probes.per_probe_timeout_ms;

    tracing::info!(
        subsystem = "model_probe",
        event_type = "probe_battery_started",
        model_id = model_id,
        per_probe_timeout_ms = per_probe_timeout_ms,
        "starting full probe battery"
    );

    // ── Phase 1: Run independent probes concurrently ────────────────────
    //
    // These probes have no dependencies on each other's results and can
    // safely run in parallel. Each probe handles its own timeout internally.

    let model_id_owned = model_id.to_string();
    let config_clone_for_lora = config.clone();
    
    // Clone the inference client for each probe that needs it
    let context_window_client = inference_client.clone();
    let graph_extraction_client = inference_client.clone();
    let instruction_following_client = inference_client.clone();
    let structured_output_client = inference_client.clone();
    let reasoning_client = inference_client.clone();
    let memory_fidelity_client = inference_client;

    let (
        context_window_result,
        structured_output_result,
        instruction_following_result,
        reasoning_result,
        lora_compat_result,
        memory_fidelity_result,
        graph_extraction_result,
    ) = tokio::join!(
        context_window::run(
            &config.probes.context_window,
            config.model.advertised_context_length,
            context_window_client,
            per_probe_timeout_ms,
        ),
        structured_output::run(
            &config.probes.structured_output,
            per_probe_timeout_ms,
            structured_output_client,
        ),
        instruction_following::run(
            &config.probes.instruction_following,
            &model_id_owned,
            instruction_following_client,
        ),
        reasoning::run_reasoning_probe(
            &config.probes.reasoning,
            &model_id_owned,
            per_probe_timeout_ms,
            reasoning_client,
        ),
        lora_compat::run(&config_clone_for_lora),
        memory_fidelity::run(&config.probes.memory_fidelity, memory_fidelity_client),
        graph_extraction::run(&config.probes.graph_extraction, graph_extraction_client),
    );

    // ── Phase 2: Extract scores, log failures, derive capabilities ──────
    //
    // Every probe error is logged and scored as the most conservative value.
    // No probe failure aborts the battery — the profile is always assembled.

    // Track which probes ran in degraded mode
    let mut degraded_probes = Vec::new();
    
    // Check context_window result for degradation before moving it
    if let Ok(ref cw_result) = context_window_result {
        if cw_result.degraded {
            degraded_probes.push("context_window".to_string());
        }
    }

    // Context window
    let (context_window_value, pre_rot_threshold) = match context_window_result {
        Ok(result) => (
            config.model.advertised_context_length,
            result.pre_rot_threshold,
        ),
        Err(probe_error) => {
            log_probe_failure("context_window", model_id, &probe_error);
            // Conservative fallback: use 25% of advertised length
            let fallback = config.model.advertised_context_length / 4;
            (config.model.advertised_context_length, fallback)
        }
    };

    // Structured output - check degradation first
    if let Ok(ref so_result) = structured_output_result {
        if so_result.degraded {
            degraded_probes.push("structured_output".to_string());
        }
    }

    // Structured output
    let structured_output_capability = match structured_output_result {
        Ok(ref result) => result
            .capability_level
            .unwrap_or(CapabilityLevel::None),
        Err(ref probe_error) => {
            log_probe_failure("structured_output", model_id, probe_error);
            CapabilityLevel::None
        }
    };

    // Instruction following - check degradation first
    if let Ok(ref if_result) = instruction_following_result {
        if if_result.degraded {
            degraded_probes.push("instruction_following".to_string());
        }
    }

    // Instruction following
    let instruction_following_capability = match instruction_following_result {
        Ok(ref result) => result
            .capability_level
            .unwrap_or(CapabilityLevel::None),
        Err(ref probe_error) => {
            log_probe_failure("instruction_following", model_id, probe_error);
            CapabilityLevel::None
        }
    };

    // Reasoning quality
    let reasoning_quality = match reasoning_result {
        Ok(ref result) => result.reasoning_quality,
        Err(ref probe_error) => {
            log_probe_failure("reasoning", model_id, probe_error);
            0.0
        }
    };

    // LoRA compatibility
    let lora_compatible = match lora_compat_result {
        Ok(ref result) => result.lora_compatible,
        Err(ref probe_error) => {
            log_probe_failure("lora_compat", model_id, probe_error);
            false
        }
    };

    // Memory injection fidelity — check degradation first
    if let Ok(ref mf_result) = memory_fidelity_result {
        if mf_result.degraded {
            degraded_probes.push("memory_fidelity".to_string());
        }
    }

    // Memory injection fidelity
    let memory_injection_fidelity = match memory_fidelity_result {
        Ok(ref result) => result.fidelity_score,
        Err(ref probe_error) => {
            log_probe_failure("memory_fidelity", model_id, probe_error);
            0.0
        }
    };

    // Graph extraction - check degradation first
    if let Ok(ref ge_result) = graph_extraction_result {
        if ge_result.degraded {
            degraded_probes.push("graph_extraction".to_string());
        }
    }

    // Graph extraction
    let graph_extraction_capability = match graph_extraction_result {
        Ok(ref result) => result
            .capability_level
            .unwrap_or(CapabilityLevel::None),
        Err(ref probe_error) => {
            log_probe_failure("graph_extraction", model_id, probe_error);
            CapabilityLevel::None
        }
    };

    // ── Phase 3: Reasoning gap detection (sequential — depends on reasoning score) ──

    let gap_result = reasoning::detect_reasoning_gap(
        reasoning_quality as f64,
        last_lora_training_score,
        config.probes.reasoning_gap.trigger_threshold,
        lora_compatible,
    );

    let lora_training_recommended = gap_result.lora_training_recommended;

    if lora_training_recommended {
        tracing::info!(
            subsystem = "model_probe",
            event_type = "lora_training_recommended",
            model_id = model_id,
            current_score = reasoning_quality,
            gap = gap_result.gap,
            last_trained_score = ?gap_result.last_trained_score,
            "reasoning gap exceeds threshold — LORA_TRAINING_RECOMMENDED will be published"
        );
    }

    // ── Phase 4: Assemble the profile ───────────────────────────────────

    let model_profile = ModelCapabilityProfile {
        model_id: model_id.to_string(),
        context_window: context_window_value,
        pre_rot_threshold,
        structured_output: structured_output_capability,
        instruction_following: instruction_following_capability,
        reasoning_quality,
        lora_compatible,
        memory_injection_fidelity,
        graph_extraction: graph_extraction_capability,
        lora_training_recommended,
        degraded_probes: degraded_probes.clone(),
    };

    let total_duration_ms = battery_start.elapsed().as_millis() as u64;

    // Log the full profile summary at info level per instructions
    tracing::info!(
        subsystem = "model_probe",
        event_type = "probe_battery_completed",
        model_id = model_id,
        context_window = model_profile.context_window,
        pre_rot_threshold = model_profile.pre_rot_threshold,
        structured_output = %model_profile.structured_output,
        instruction_following = %model_profile.instruction_following,
        reasoning_quality = model_profile.reasoning_quality,
        lora_compatible = model_profile.lora_compatible,
        memory_injection_fidelity = model_profile.memory_injection_fidelity,
        graph_extraction = %model_profile.graph_extraction,
        lora_training_recommended = model_profile.lora_training_recommended,
        degraded_probes = ?degraded_probes,
        total_duration_ms = total_duration_ms,
        "probe battery completed — profile assembled"
    );

    Ok(ProbeBatteryOutcome {
        model_profile,
        lora_training_recommended,
        total_duration_ms,
    })
}

/// Log a probe failure at error level with full structured context.
///
/// Called when an individual probe returns an error. The battery continues —
/// the failing probe is scored conservatively (CapabilityLevel::None / 0.0).
fn log_probe_failure(probe_name: &str, model_id: &str, error: &SenaError) {
    tracing::error!(
        subsystem = "model_probe",
        probe_name = probe_name,
        model_id = model_id,
        event_type = "probe_failed",
        error_code = %error.code,
        error_message = %error.message,
        "probe failed — scoring as None/0.0"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ProbeResult — shared result type for probes that produce a scored capability
// ─────────────────────────────────────────────────────────────────────────────

/// Standardized result type for probes that produce a score and capability level.
///
/// Used by probes that follow the common pattern of: run inference → score
/// response → map to CapabilityLevel. Not all probes use this (e.g. context
/// window and reasoning have their own specialized result types).
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Name of the probe that produced this result.
    pub probe_name: String,
    /// Raw numeric score before threshold comparison.
    pub raw_score: f64,
    /// Derived capability level from score + config thresholds.
    /// `None` (the Option) means the probe doesn't produce a capability level.
    pub capability_level: Option<CapabilityLevel>,
    /// Wall-clock duration of the probe.
    pub duration: std::time::Duration,
    /// Whether this probe result is degraded (formula/stub fallback due to
    /// inference unavailability). When true, downstream systems should treat
    /// the result as a conservative estimate rather than measured capability.
    pub degraded: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_level_from_score_full() {
        assert_eq!(
            CapabilityLevel::from_score(0.95, 0.50, 0.90),
            CapabilityLevel::Full
        );
    }

    #[test]
    fn capability_level_from_score_at_full_threshold() {
        assert_eq!(
            CapabilityLevel::from_score(0.90, 0.50, 0.90),
            CapabilityLevel::Full
        );
    }

    #[test]
    fn capability_level_from_score_partial() {
        assert_eq!(
            CapabilityLevel::from_score(0.70, 0.50, 0.90),
            CapabilityLevel::Partial
        );
    }

    #[test]
    fn capability_level_from_score_at_partial_threshold() {
        assert_eq!(
            CapabilityLevel::from_score(0.50, 0.50, 0.90),
            CapabilityLevel::Partial
        );
    }

    #[test]
    fn capability_level_from_score_none() {
        assert_eq!(
            CapabilityLevel::from_score(0.30, 0.50, 0.90),
            CapabilityLevel::None
        );
    }

    #[test]
    fn capability_level_from_score_zero() {
        assert_eq!(
            CapabilityLevel::from_score(0.0, 0.50, 0.90),
            CapabilityLevel::None
        );
    }

    #[test]
    fn capability_level_display() {
        assert_eq!(format!("{}", CapabilityLevel::Full), "Full");
        assert_eq!(format!("{}", CapabilityLevel::Partial), "Partial");
        assert_eq!(format!("{}", CapabilityLevel::None), "None");
    }

    #[test]
    fn model_capability_profile_serializes_to_json() {
        let profile = ModelCapabilityProfile {
            model_id: "test-model-7b-q4".to_string(),
            context_window: 8192,
            pre_rot_threshold: 5529,
            structured_output: CapabilityLevel::Full,
            instruction_following: CapabilityLevel::Partial,
            reasoning_quality: 0.75,
            lora_compatible: true,
            memory_injection_fidelity: 0.85,
            graph_extraction: CapabilityLevel::Full,
            lora_training_recommended: false,
            degraded_probes: vec![],
        };

        let json = serde_json::to_string(&profile);
        assert!(json.is_ok(), "ModelCapabilityProfile must serialize to JSON");

        let deserialized: Result<ModelCapabilityProfile, _> =
            serde_json::from_str(&json.expect("serialization confirmed ok above"));
        assert!(
            deserialized.is_ok(),
            "ModelCapabilityProfile must round-trip through JSON"
        );

        let round_tripped = deserialized.expect("deserialization confirmed ok above");
        assert_eq!(round_tripped.model_id, "test-model-7b-q4");
        assert_eq!(round_tripped.context_window, 8192);
        assert_eq!(round_tripped.pre_rot_threshold, 5529);
        assert_eq!(round_tripped.structured_output, CapabilityLevel::Full);
        assert_eq!(
            round_tripped.instruction_following,
            CapabilityLevel::Partial
        );
        assert!((round_tripped.reasoning_quality - 0.75).abs() < f32::EPSILON);
        assert!(round_tripped.lora_compatible);
        assert!((round_tripped.memory_injection_fidelity - 0.85).abs() < f32::EPSILON);
        assert_eq!(round_tripped.graph_extraction, CapabilityLevel::Full);
        assert!(!round_tripped.lora_training_recommended);
    }

    #[test]
    fn capability_level_serialization_round_trip() {
        for level in [
            CapabilityLevel::Full,
            CapabilityLevel::Partial,
            CapabilityLevel::None,
        ] {
            let json =
                serde_json::to_string(&level).expect("CapabilityLevel should serialize to JSON");
            let deserialized: CapabilityLevel =
                serde_json::from_str(&json).expect("CapabilityLevel should deserialize from JSON");
            assert_eq!(level, deserialized);
        }
    }
}
