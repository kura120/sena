//! Profile derivation for memory-engine.
//!
//! Receives `ModelCapabilityProfile` (deserialized from the daemon-bus gRPC
//! event payload published by model-probe at boot) and derives ech0's
//! `StoreConfig` plus memory-engine-specific flags.
//!
//! This is the **only** place where capability-conditional logic lives.
//! No other module inspects `ModelCapabilityProfile` — everything downstream
//! consumes `ProfileDerivedConfig`.

use serde::Deserialize;

use crate::config::Config;
use crate::error::{ErrorCode, SenaError, SenaResult};

// ─────────────────────────────────────────────────────────────────────────────
// ModelCapabilityProfile — local mirror of model-probe's definition
// ─────────────────────────────────────────────────────────────────────────────
//
// Defined locally to avoid a direct crate dependency on model-probe.
// Deserialized from the JSON payload inside the daemon-bus BusEvent.
// If model-probe changes its schema, this struct must be updated to match.

/// Capability level for a discrete model feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CapabilityLevel {
    /// Probe passed — full capability available.
    Full,
    /// Probe passed partially — use with fallbacks.
    Partial,
    /// Probe failed completely — capability unavailable.
    None,
}

/// Model capability profile published by model-probe at boot.
///
/// memory-engine never constructs this — it is always deserialized from
/// the daemon-bus event payload.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelCapabilityProfile {
    /// Identifier of the probed model.
    pub model_id: String,
    /// Advertised context window in tokens — for reference only.
    pub context_window: u32,
    /// Conservative token budget before performance cliff.
    pub pre_rot_threshold: u32,
    /// Whether the model can produce reliable structured output.
    pub structured_output: CapabilityLevel,
    /// How well the model follows multi-step instructions.
    pub instruction_following: CapabilityLevel,
    /// Raw reasoning quality score (0.0–1.0).
    pub reasoning_quality: f32,
    /// Whether the model's architecture supports LoRA adapter training.
    pub lora_compatible: bool,
    /// How accurately the model retrieves facts from injected context (0.0–1.0).
    pub memory_injection_fidelity: f32,
    /// Whether the model can extract knowledge graphs from text.
    pub graph_extraction: CapabilityLevel,
    /// Whether reasoning gap detection recommends a LoRA training cycle.
    pub lora_training_recommended: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// ProfileDerivedConfig
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration derived from `ModelCapabilityProfile` for use by the rest of
/// memory-engine. Wraps ech0's config types so memory-engine controls what it
/// exposes to the engine layer — callers never see raw ech0 config types.
#[derive(Debug, Clone)]
pub struct ProfileDerivedConfig {
    /// Context budget in tokens derived from `pre_rot_threshold`.
    /// Drives `MemoryConfig` context budget in the ech0 `StoreConfig`.
    pub context_budget: u32,

    /// Whether dynamic linking (graph edges between nodes) should be enabled
    /// in ech0's `DynamicLinkingConfig`.
    /// Disabled when `graph_extraction == CapabilityLevel::None`.
    pub dynamic_linking_enabled: bool,

    /// Sensitivity threshold for ech0's `ContradictionConfig` (0.0–1.0).
    /// Higher `reasoning_quality` → stricter (higher) threshold so that only
    /// high-confidence contradictions are flagged. Lower quality → looser
    /// threshold to avoid false positives from a weak model.
    pub contradiction_sensitivity: f32,

    /// When true, the extractor should operate in degraded stub mode —
    /// returning minimal/empty extraction results instead of calling the
    /// model. Set when `structured_output == CapabilityLevel::None`.
    pub degraded_extractor: bool,

    /// The original model ID, carried through for logging and diagnostics.
    pub model_id: String,

    /// The memory injection fidelity score (0.0–1.0), forwarded for
    /// downstream consumers that may adjust retrieval depth.
    pub memory_injection_fidelity: f32,
}

impl ProfileDerivedConfig {
    /// Construct a safe, conservative default for use before the
    /// `ModelCapabilityProfile` arrives from model-probe.
    ///
    /// memory-engine boots with this config so it can signal
    /// `MEMORY_ENGINE_READY` without blocking on model-probe, which does not
    /// start until after the inference subsystem is running — and inference
    /// does not start until memory-engine is ready.  The real profile is
    /// applied when the background `MODEL_PROFILE_READY` subscriber fires.
    ///
    /// Conservative choices:
    /// - `dynamic_linking_enabled = false` — avoids garbage edges when the
    ///   model's graph-extraction capability is unknown.
    /// - `contradiction_sensitivity = 0.3` — minimum floor value; prevents
    ///   false positives from a model with unknown reasoning quality.
    /// - `degraded_extractor = true` — always safe; also forced by the Phase 1
    ///   override in `derive_config` regardless of the profile.
    /// - `context_budget = 0` — no budget constraint; prompt-composer clamps
    ///   to its own defaults when the budget is absent.
    pub fn without_profile() -> Self {
        Self {
            context_budget: 0,
            dynamic_linking_enabled: false,
            contradiction_sensitivity: 0.3,
            degraded_extractor: true,
            model_id: "unknown".to_owned(),
            memory_injection_fidelity: 0.5,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Derivation logic
// ─────────────────────────────────────────────────────────────────────────────

/// Derive a `ProfileDerivedConfig` from a `ModelCapabilityProfile` and the
/// memory-engine `Config`.
///
/// This function contains all capability-conditional logic. Every derived
/// value is logged at `debug` level (field names and values only — no content,
/// no user data, no model output).
///
/// # Errors
///
/// Returns `ErrorCode::ProfileInvalid` if the profile contains values that
/// cannot be used to construct a valid config (e.g. `pre_rot_threshold == 0`).
pub fn derive_config(
    profile: &ModelCapabilityProfile,
    _config: &Config,
) -> SenaResult<ProfileDerivedConfig> {
    // ── Validate ────────────────────────────────────────────────────────
    if profile.pre_rot_threshold == 0 {
        return Err(SenaError::new(
            ErrorCode::ProfileInvalid,
            "pre_rot_threshold is zero — cannot derive context budget",
        )
        .with_debug_context(format!("model_id: {}", profile.model_id)));
    }

    if !(0.0..=1.0).contains(&profile.reasoning_quality) {
        return Err(SenaError::new(
            ErrorCode::ProfileInvalid,
            "reasoning_quality outside valid range 0.0–1.0",
        )
        .with_debug_context(format!(
            "model_id: {}, reasoning_quality: {}",
            profile.model_id, profile.reasoning_quality
        )));
    }

    // ── Context budget ──────────────────────────────────────────────────
    // Use pre_rot_threshold directly as the context budget. prompt-composer
    // uses this to cap how much memory is injected into a prompt.
    let context_budget = profile.pre_rot_threshold;

    // ── Dynamic linking ─────────────────────────────────────────────────
    // Graph extraction requires the model to produce structured node/edge
    // output. If the model cannot do this at all, disable linking to avoid
    // garbage edges in the knowledge graph.
    let dynamic_linking_enabled = profile.graph_extraction != CapabilityLevel::None;

    if !dynamic_linking_enabled {
        tracing::info!(
            subsystem = "memory_engine",
            component = "profile",
            model_id = %profile.model_id,
            graph_extraction = ?profile.graph_extraction,
            "dynamic linking disabled — model cannot extract knowledge graphs"
        );
    }

    // ── Contradiction sensitivity ───────────────────────────────────────
    // Maps reasoning_quality (0.0–1.0) to a contradiction detection
    // sensitivity threshold. A higher-quality model can detect subtler
    // contradictions, so we set a stricter (higher) threshold.
    //
    // Linear mapping: sensitivity = 0.3 + (reasoning_quality * 0.5)
    // This gives a range of 0.3 (weak model) to 0.8 (strong model).
    // The constants come from the domain — below 0.3, contradiction
    // detection produces too many false positives to be useful.
    let contradiction_sensitivity_floor = 0.3_f32;
    let contradiction_sensitivity_range = 0.5_f32;
    let contradiction_sensitivity = contradiction_sensitivity_floor
        + (profile.reasoning_quality * contradiction_sensitivity_range);

    // ── Degraded extractor ──────────────────────────────────────────────
    // Phase 2: When the LlamaExtractor inference loop is wired, restore
    // the capability-based check below and remove the forced override.
    //
    // Phase 2: let degraded_extractor = profile.structured_output == CapabilityLevel::None;
    //
    // Phase 2: if degraded_extractor {
    // Phase 2:     tracing::warn!(
    // Phase 2:         subsystem = "memory_engine",
    // Phase 2:         component = "profile",
    // Phase 2:         model_id = %profile.model_id,
    // Phase 2:         structured_output = ?profile.structured_output,
    // Phase 2:         "extractor set to degraded stub mode — model cannot produce structured output"
    // Phase 2:     );
    // Phase 2: }
    let degraded_extractor = true;

    tracing::info!(
        subsystem = "memory_engine",
        component = "profile",
        structured_output = ?profile.structured_output,
        "LlamaExtractor inference loop not wired in Phase 1 — forcing DegradedExtractor regardless of model capability"
    );

    let derived = ProfileDerivedConfig {
        context_budget,
        dynamic_linking_enabled,
        contradiction_sensitivity,
        degraded_extractor,
        model_id: profile.model_id.clone(),
        memory_injection_fidelity: profile.memory_injection_fidelity,
    };

    // ── Log all derived values at debug level ───────────────────────────
    tracing::debug!(
        subsystem = "memory_engine",
        component = "profile",
        model_id = %derived.model_id,
        context_budget = derived.context_budget,
        dynamic_linking_enabled = derived.dynamic_linking_enabled,
        contradiction_sensitivity = derived.contradiction_sensitivity,
        degraded_extractor = derived.degraded_extractor,
        memory_injection_fidelity = derived.memory_injection_fidelity,
        "profile-derived config computed"
    );

    Ok(derived)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid config for testing. Uses hardcoded values only
    /// inside tests — production code always loads from TOML.
    fn test_config() -> Config {
        Config {
            grpc: crate::config::GrpcConfig {
                daemon_bus_address: "http://127.0.0.1:50051".to_owned(),
                listen_port: 50053,
                connect_timeout_ms: 5000,
            },
            boot: crate::config::BootConfig {
                ready_signal_timeout_ms: 15000,
            },
            store: crate::config::StorePathsConfig {
                graph_path: "data/graph.redb".to_owned(),
                vector_path: "data/vectors.usearch".to_owned(),
            },
            tier: crate::config::TierConfig {
                short_term: crate::config::ShortTermTierConfig { max_entries: 256 },
                long_term: crate::config::LongTermTierConfig { max_entries: 10000 },
                episodic: crate::config::EpisodicTierConfig { max_entries: 50000 },
            },
            decay: crate::config::DecayConfig {
                rate: 0.95,
                floor: 0.05,
            },
            queue: crate::config::QueueConfig {
                max_depth: 512,
                operation_timeout_ms: 10000,
                retry: crate::config::RetryConfig {
                    max_attempts: 3,
                    backoff_ms: 500,
                },
            },
            embedder: crate::config::EmbedderConfig {
                model_path: "models/embedding.gguf".to_owned(),
                embedding_dim: 768,
                batch_size: 32,
                gpu_layers: 99,
            },
            extractor: crate::config::ExtractorConfig {
                model_path: "models/default.gguf".to_owned(),
                gpu_layers: 99,
                max_tokens: 512,
                temperature: 0.0,
            },
            logging: crate::config::LoggingConfig {
                level: "info".to_owned(),
                format: "pretty".to_owned(),
                slow_operation_threshold_ms: 100,
            },
        }
    }

    fn base_profile() -> ModelCapabilityProfile {
        ModelCapabilityProfile {
            model_id: "test-model-7b".to_owned(),
            context_window: 8192,
            pre_rot_threshold: 6144,
            structured_output: CapabilityLevel::Full,
            instruction_following: CapabilityLevel::Full,
            reasoning_quality: 0.75,
            lora_compatible: true,
            memory_injection_fidelity: 0.85,
            graph_extraction: CapabilityLevel::Full,
            lora_training_recommended: false,
        }
    }

    #[test]
    fn derive_full_capability_profile() {
        let config = test_config();
        let profile = base_profile();

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        assert_eq!(derived.context_budget, 6144);
        assert!(derived.dynamic_linking_enabled);
        // Phase 1: degraded_extractor is always true regardless of model capability
        assert!(derived.degraded_extractor);
        assert_eq!(derived.model_id, "test-model-7b");
        assert_eq!(derived.memory_injection_fidelity, 0.85);

        // contradiction_sensitivity = 0.3 + (0.75 * 0.5) = 0.675
        let expected_sensitivity = 0.3 + (0.75 * 0.5);
        assert!(
            (derived.contradiction_sensitivity - expected_sensitivity).abs() < f32::EPSILON,
            "expected sensitivity {}, got {}",
            expected_sensitivity,
            derived.contradiction_sensitivity
        );
    }

    #[test]
    fn graph_extraction_none_disables_dynamic_linking() {
        let config = test_config();
        let mut profile = base_profile();
        profile.graph_extraction = CapabilityLevel::None;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        assert!(!derived.dynamic_linking_enabled);
    }

    #[test]
    fn graph_extraction_partial_keeps_dynamic_linking_enabled() {
        let config = test_config();
        let mut profile = base_profile();
        profile.graph_extraction = CapabilityLevel::Partial;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        assert!(derived.dynamic_linking_enabled);
    }

    #[test]
    fn structured_output_none_sets_degraded_extractor() {
        let config = test_config();
        let mut profile = base_profile();
        profile.structured_output = CapabilityLevel::None;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        // Phase 1: always true regardless of structured_output capability
        assert!(derived.degraded_extractor);
    }

    #[test]
    fn structured_output_partial_does_not_degrade_extractor() {
        let config = test_config();
        let mut profile = base_profile();
        profile.structured_output = CapabilityLevel::Partial;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        // Phase 1: degraded_extractor is forced true — LlamaExtractor inference loop not wired
        assert!(derived.degraded_extractor);
    }

    #[test]
    fn high_reasoning_quality_yields_strict_contradiction_sensitivity() {
        let config = test_config();
        let mut profile = base_profile();
        profile.reasoning_quality = 1.0;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        // sensitivity = 0.3 + (1.0 * 0.5) = 0.8
        let expected = 0.8_f32;
        assert!(
            (derived.contradiction_sensitivity - expected).abs() < f32::EPSILON,
            "expected {}, got {}",
            expected,
            derived.contradiction_sensitivity
        );
    }

    #[test]
    fn low_reasoning_quality_yields_loose_contradiction_sensitivity() {
        let config = test_config();
        let mut profile = base_profile();
        profile.reasoning_quality = 0.0;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        // sensitivity = 0.3 + (0.0 * 0.5) = 0.3
        let expected = 0.3_f32;
        assert!(
            (derived.contradiction_sensitivity - expected).abs() < f32::EPSILON,
            "expected {}, got {}",
            expected,
            derived.contradiction_sensitivity
        );
    }

    #[test]
    fn zero_pre_rot_threshold_returns_profile_invalid() {
        let config = test_config();
        let mut profile = base_profile();
        profile.pre_rot_threshold = 0;

        let error = derive_config(&profile, &config).expect_err("should reject zero threshold");

        assert_eq!(error.code, ErrorCode::ProfileInvalid);
        assert!(error.message.contains("pre_rot_threshold"));
    }

    #[test]
    fn reasoning_quality_out_of_range_returns_profile_invalid() {
        let config = test_config();

        let mut profile_high = base_profile();
        profile_high.reasoning_quality = 1.5;
        let error = derive_config(&profile_high, &config).expect_err("should reject quality > 1.0");
        assert_eq!(error.code, ErrorCode::ProfileInvalid);
        assert!(error.message.contains("reasoning_quality"));

        let mut profile_low = base_profile();
        profile_low.reasoning_quality = -0.1;
        let error = derive_config(&profile_low, &config).expect_err("should reject quality < 0.0");
        assert_eq!(error.code, ErrorCode::ProfileInvalid);
    }

    #[test]
    fn context_budget_equals_pre_rot_threshold() {
        let config = test_config();
        let mut profile = base_profile();
        profile.pre_rot_threshold = 4096;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        assert_eq!(derived.context_budget, 4096);
    }

    #[test]
    fn memory_injection_fidelity_forwarded() {
        let config = test_config();
        let mut profile = base_profile();
        profile.memory_injection_fidelity = 0.42;

        let derived = derive_config(&profile, &config).expect("should derive successfully");

        assert_eq!(derived.memory_injection_fidelity, 0.42);
    }
}
