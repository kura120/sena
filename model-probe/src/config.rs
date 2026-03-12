//! Strongly-typed configuration for model-probe.
//!
//! Every tunable value — thresholds, timeouts, model parameters, scoring weights —
//! lives in `config/model-probe.toml`. This module deserializes that file into typed
//! structs passed by reference throughout the probe lifecycle.

use std::path::Path;

use serde::Deserialize;

use crate::error::{ErrorCode, SenaError, SenaResult};

// ─────────────────────────────────────────────────────────────────────────────
// Root config
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ModelProbeConfig {
    pub grpc: GrpcConfig,
    pub model: ModelConfig,
    pub hardware: HardwareConfig,
    pub probes: ProbesConfig,
    pub logging: LoggingConfig,
}

// ─────────────────────────────────────────────────────────────────────────────
// gRPC connection to daemon-bus
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    /// daemon-bus gRPC address to connect to (e.g. "http://127.0.0.1:50051").
    pub daemon_bus_address: String,
    /// Timeout in milliseconds for the initial gRPC connection attempt.
    pub connect_timeout_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Model configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    /// Filesystem path to the GGUF model file loaded by llama-cpp-rs.
    pub model_path: String,
    /// Number of GPU layers to offload. 0 = CPU-only.
    pub gpu_layers: u32,
    /// Advertised context length of the model. Used as an upper bound for
    /// context window probing — never trusted as the actual usable limit.
    pub advertised_context_length: u32,
    /// Seed for deterministic probe inference. Fixed to ensure reproducibility.
    pub seed: u32,
    /// Maximum tokens any single probe inference may generate.
    pub max_probe_tokens: u32,
    /// Temperature override for all probe inferences. Must be 0.0 for
    /// deterministic, reproducible results.
    pub temperature: f32,
    /// List of model architecture families known to support LoRA adapters.
    /// Used by the LoRA compatibility probe — structural check, not inference.
    pub lora_compatible_architectures: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Hardware tier thresholds
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct HardwareConfig {
    /// VRAM threshold in MB below which the hardware is classified as Low tier.
    pub low_tier_vram_ceiling_mb: u64,
    /// VRAM threshold in MB at or above which the hardware is classified as High tier.
    pub high_tier_vram_floor_mb: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Probe-specific configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ProbesConfig {
    /// Overall timeout in milliseconds for the entire probe battery.
    pub battery_timeout_ms: u64,
    /// Per-probe timeout in milliseconds. Individual probes that exceed this
    /// are terminated and scored as CapabilityLevel::None / 0.0.
    pub per_probe_timeout_ms: u64,
    pub context_window: ContextWindowProbeConfig,
    pub structured_output: StructuredOutputProbeConfig,
    pub instruction_following: InstructionFollowingProbeConfig,
    pub reasoning: ReasoningProbeConfig,
    pub lora_compat: LoraCompatProbeConfig,
    pub memory_fidelity: MemoryFidelityProbeConfig,
    pub graph_extraction: GraphExtractionProbeConfig,
    pub reasoning_gap: ReasoningGapConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextWindowProbeConfig {
    /// The known token sequence repeated to measure context retention.
    pub probe_token_sequence: String,
    /// Fractions of advertised context length at which to test retention
    /// (e.g. [0.25, 0.50, 0.75]).
    pub retention_test_fractions: Vec<f64>,
    /// Safety margin subtracted from the highest passing retention level to
    /// derive `pre_rot_threshold`. Expressed as a fraction (e.g. 0.10 = 10%).
    pub safety_margin_fraction: f64,
    /// Expected answer fragment that the model must reproduce to pass each
    /// retention test level.
    pub expected_answer: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StructuredOutputProbeConfig {
    /// The prompt requesting a minimal KnowledgeGraph structured output.
    pub probe_prompt: String,
    /// JSON schema string used to validate the model's structured output.
    pub expected_schema: String,
    /// Minimum fraction of schema fields that must be present for Partial.
    pub partial_threshold: f64,
    /// Minimum fraction of schema fields that must be present for Full.
    pub full_threshold: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstructionFollowingProbeConfig {
    /// Multi-step instruction prompt with a precise expected format.
    pub probe_prompt: String,
    /// The exact expected output for scoring.
    pub expected_output: String,
    /// Score at or above which the capability is Partial.
    pub partial_threshold: f64,
    /// Score at or above which the capability is Full.
    pub full_threshold: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReasoningProbeConfig {
    /// Chain-of-thought problem prompt with a known correct answer.
    pub probe_prompt: String,
    /// The known correct final answer to extract and compare.
    pub expected_answer: String,
    /// Keywords or reasoning steps expected in the chain-of-thought.
    /// Each matched step contributes to the coherence score.
    pub expected_reasoning_steps: Vec<String>,
    /// Score at or above which reasoning quality is considered acceptable
    /// (no LoRA training recommended on its own — gap detection still applies).
    pub quality_threshold: f64,
    
    pub answer_weight: f64,
    pub reasoning_steps_weight: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoraCompatProbeConfig {
    /// Architecture metadata key to inspect in the model file.
    pub architecture_metadata_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryFidelityProbeConfig {
    /// A known fact injected into context for the model to reason about.
    pub injected_fact: String,
    /// The question posed about the injected fact.
    pub probe_prompt: String,
    /// The expected answer derived from the injected fact.
    pub expected_answer: String,
    /// Score at or above which memory injection fidelity is considered passing.
    pub pass_threshold: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphExtractionProbeConfig {
    /// Prompt requesting a minimal KnowledgeGraph structured output.
    pub probe_prompt: String,
    /// JSON schema for validating the graph extraction output.
    pub expected_schema: String,
    /// Minimum fraction of schema fields for Partial.
    pub partial_threshold: f64,
    /// Minimum fraction of schema fields for Full.
    pub full_threshold: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReasoningGapConfig {
    /// If the gap between last-trained reasoning score and current score
    /// exceeds this threshold, publish LORA_TRAINING_RECOMMENDED.
    pub trigger_threshold: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Logging
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Minimum log level: trace, debug, info, warn, error.
    pub level: String,
    /// Output format: "json" for structured production logs, "pretty" for dev.
    pub format: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Loading
// ─────────────────────────────────────────────────────────────────────────────

impl ModelProbeConfig {
    /// Load and parse model-probe configuration from the given TOML file path.
    ///
    /// Uses `spawn_blocking` to keep file I/O off the async runtime.
    pub async fn load(path: &Path) -> SenaResult<Self> {
        let path_owned = path.to_path_buf();

        let raw_toml = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path_owned))
            .await
            .map_err(|join_error| {
                SenaError::new(
                    ErrorCode::ConfigLoadFailed,
                    "config read task was cancelled or panicked",
                )
                .with_debug_context(format!("JoinError: {join_error}"))
            })?
            .map_err(|io_error| {
                SenaError::new(
                    ErrorCode::ConfigLoadFailed,
                    format!("failed to read config file: {}", path.display()),
                )
                .with_debug_context(format!("IO error: {io_error}"))
            })?;

        let config: ModelProbeConfig = toml::from_str(&raw_toml).map_err(|parse_error| {
            SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!("failed to parse config file: {}", path.display()),
            )
            .with_debug_context(format!("TOML parse error: {parse_error}"))
        })?;

        config.validate()?;

        Ok(config)
    }

    /// Validates invariants that TOML deserialization alone cannot enforce.
    fn validate(&self) -> SenaResult<()> {
        if self.model.temperature != 0.0 {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!(
                    "model.temperature must be 0.0 for deterministic probes, got {}",
                    self.model.temperature
                ),
            ));
        }

        if self.probes.context_window.retention_test_fractions.is_empty() {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "probes.context_window.retention_test_fractions must not be empty",
            ));
        }

        for fraction in &self.probes.context_window.retention_test_fractions {
            if *fraction <= 0.0 || *fraction > 1.0 {
                return Err(SenaError::new(
                    ErrorCode::ConfigLoadFailed,
                    format!(
                        "retention_test_fractions values must be in (0.0, 1.0], got {}",
                        fraction
                    ),
                ));
            }
        }

        if self.probes.context_window.safety_margin_fraction < 0.0
            || self.probes.context_window.safety_margin_fraction >= 1.0
        {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!(
                    "safety_margin_fraction must be in [0.0, 1.0), got {}",
                    self.probes.context_window.safety_margin_fraction
                ),
            ));
        }

        if self.hardware.low_tier_vram_ceiling_mb >= self.hardware.high_tier_vram_floor_mb {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!(
                    "hardware.low_tier_vram_ceiling_mb ({}) must be less than hardware.high_tier_vram_floor_mb ({})",
                    self.hardware.low_tier_vram_ceiling_mb,
                    self.hardware.high_tier_vram_floor_mb
                ),
            ));
        }

        if self.probes.per_probe_timeout_ms == 0 {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "probes.per_probe_timeout_ms must be greater than zero",
            ));
        }

        if self.probes.battery_timeout_ms == 0 {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "probes.battery_timeout_ms must be greater than zero",
            ));
        }

        if self.probes.reasoning_gap.trigger_threshold <= 0.0
            || self.probes.reasoning_gap.trigger_threshold > 1.0
        {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!(
                    "probes.reasoning_gap.trigger_threshold must be in (0.0, 1.0], got {}",
                    self.probes.reasoning_gap.trigger_threshold
                ),
            ));
        }

        if self.model.lora_compatible_architectures.is_empty() {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "model.lora_compatible_architectures must not be empty",
            ));
        }

        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper that produces a valid config TOML string for testing.
    fn valid_config_toml() -> String {
        r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
connect_timeout_ms = 5000

[model]
model_path = "models/default.gguf"
gpu_layers = 99
advertised_context_length = 8192
seed = 42
max_probe_tokens = 256
temperature = 0.0
lora_compatible_architectures = ["llama", "mistral", "qwen", "gemma", "phi"]

[hardware]
low_tier_vram_ceiling_mb = 8192
high_tier_vram_floor_mb = 16384

[probes]
battery_timeout_ms = 30000
per_probe_timeout_ms = 5000

[probes.context_window]
probe_token_sequence = "The quick brown fox jumps over the lazy dog."
retention_test_fractions = [0.25, 0.50, 0.75]
safety_margin_fraction = 0.10
expected_answer = "lazy dog"

[probes.structured_output]
probe_prompt = "Extract entities from: 'Alice works at Acme Corp in Paris.' Return as JSON with fields: entities (list of {name, type})."
expected_schema = '{"type":"object","required":["entities"],"properties":{"entities":{"type":"array","items":{"type":"object","required":["name","type"],"properties":{"name":{"type":"string"},"type":{"type":"string"}}}}}}'
partial_threshold = 0.5
full_threshold = 0.9

[probes.instruction_following]
probe_prompt = "List exactly 3 colors, one per line, no numbering, no extra text."
expected_output = "red\nblue\ngreen"
partial_threshold = 0.5
full_threshold = 0.9

[probes.reasoning]
probe_prompt = "A farmer has 15 sheep. All but 8 die. How many sheep are left alive? Think step by step, then give the final answer as a single number on the last line."
expected_answer = "8"
expected_reasoning_steps = ["all but 8", "15 is irrelevant", "8 sheep"]
quality_threshold = 0.6
answer_weight = 0.60
reasoning_steps_weight = 0.40

[probes.lora_compat]
architecture_metadata_key = "general.architecture"

[probes.memory_fidelity]
injected_fact = "Project Zenith's deadline is March 15, 2025."
probe_prompt = "Based on the context above, when is Project Zenith's deadline?"
expected_answer = "March 15, 2025"
pass_threshold = 0.7

[probes.graph_extraction]
probe_prompt = "Extract a knowledge graph from: 'Marie Curie discovered radium in 1898 at the University of Paris.' Return JSON with fields: nodes (list of {id, label, type}) and edges (list of {source, target, relation})."
expected_schema = '{"type":"object","required":["nodes","edges"],"properties":{"nodes":{"type":"array","items":{"type":"object","required":["id","label","type"]}},"edges":{"type":"array","items":{"type":"object","required":["source","target","relation"]}}}}'
partial_threshold = 0.5
full_threshold = 0.9

[probes.reasoning_gap]
trigger_threshold = 0.15

[logging]
level = "info"
format = "json"
"#
        .to_string()
    }

    #[test]
    fn test_valid_config_parses() {
        let raw = valid_config_toml();
        let config: ModelProbeConfig =
            toml::from_str(&raw).expect("valid config should parse without error");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_nonzero_temperature_rejected() {
        let raw = valid_config_toml().replace("temperature = 0.0", "temperature = 0.7");
        let config: ModelProbeConfig = toml::from_str(&raw).expect("should parse");
        let result = config.validate();
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigLoadFailed);
        assert!(error.message.contains("temperature"));
    }

    #[test]
    fn test_empty_retention_fractions_rejected() {
        let raw = valid_config_toml()
            .replace("retention_test_fractions = [0.25, 0.50, 0.75]", "retention_test_fractions = []");
        let config: ModelProbeConfig = toml::from_str(&raw).expect("should parse");
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_hardware_tiers_rejected() {
        let raw = valid_config_toml()
            .replace("low_tier_vram_ceiling_mb = 8192", "low_tier_vram_ceiling_mb = 20000");
        let config: ModelProbeConfig = toml::from_str(&raw).expect("should parse");
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_zero_battery_timeout_rejected() {
        let raw = valid_config_toml()
            .replace("battery_timeout_ms = 30000", "battery_timeout_ms = 0");
        let config: ModelProbeConfig = toml::from_str(&raw).expect("should parse");
        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_reasoning_gap_threshold_rejected() {
        let raw = valid_config_toml()
            .replace("trigger_threshold = 0.15", "trigger_threshold = 0.0");
        let config: ModelProbeConfig = toml::from_str(&raw).expect("should parse");
        let result = config.validate();
        assert!(result.is_err());
    }
}
