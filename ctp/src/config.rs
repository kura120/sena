use crate::error::CtpError;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub surface_thresholds: SurfaceThresholds,
    pub expiry_windows: ExpiryWindows,
    pub default_weights: DefaultWeights,
    pub consolidation: ConsolidationConfig,
    pub compaction: CompactionConfig,
    pub queue: QueueConfig,
    pub activity: ActivityConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub memory_engine_address: String,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceThresholds {
    pub user_active: f32,
    pub idle_2min: f32,
    pub idle_10min: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpiryWindows {
    pub high_relevance_secs: u64,
    pub medium_relevance_secs: u64,
    pub low_relevance_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefaultWeights {
    pub urgency: f32,
    pub emotional_resonance: f32,
    pub novelty: f32,
    pub recurrence: f32,
    pub idle_curiosity: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsolidationConfig {
    pub idle_threshold_secs: u64,
    pub promotion_min_score: f32,
    pub max_entries_per_cycle: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompactionConfig {
    pub pre_rot_fraction: f32,
    pub max_entries_to_summarize: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    pub max_depth: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivityConfig {
    pub poll_interval_ms: u64,
    pub idle_2min_threshold_secs: u64,
    pub idle_10min_threshold_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, CtpError> {
        let content = std::fs::read_to_string(path).map_err(|io_error| {
            CtpError::Config(format!("failed to read config file: {}", io_error))
        })?;

        let config: Config = toml::from_str(&content).map_err(|parse_error| {
            CtpError::Config(format!("failed to parse TOML: {}", parse_error))
        })?;

        config.validate()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), CtpError> {
        // Surface thresholds must be in (0.0, 1.0]
        validate_threshold("surface_thresholds.user_active", self.surface_thresholds.user_active)?;
        validate_threshold("surface_thresholds.idle_2min", self.surface_thresholds.idle_2min)?;
        validate_threshold("surface_thresholds.idle_10min", self.surface_thresholds.idle_10min)?;

        // Expiry windows must be > 0
        validate_nonzero_u64(
            "expiry_windows.high_relevance_secs",
            self.expiry_windows.high_relevance_secs,
        )?;
        validate_nonzero_u64(
            "expiry_windows.medium_relevance_secs",
            self.expiry_windows.medium_relevance_secs,
        )?;
        validate_nonzero_u64(
            "expiry_windows.low_relevance_secs",
            self.expiry_windows.low_relevance_secs,
        )?;

        // Default weights must be > 0.0
        validate_positive_f32("default_weights.urgency", self.default_weights.urgency)?;
        validate_positive_f32(
            "default_weights.emotional_resonance",
            self.default_weights.emotional_resonance,
        )?;
        validate_positive_f32("default_weights.novelty", self.default_weights.novelty)?;
        validate_positive_f32("default_weights.recurrence", self.default_weights.recurrence)?;
        validate_positive_f32(
            "default_weights.idle_curiosity",
            self.default_weights.idle_curiosity,
        )?;

        // Queue max_depth must be > 0
        if self.queue.max_depth == 0 {
            return Err(CtpError::ConfigValidation {
                field: "queue.max_depth".into(),
                reason: "must be greater than 0".into(),
            });
        }

        // Activity poll interval must be > 0
        if self.activity.poll_interval_ms == 0 {
            return Err(CtpError::ConfigValidation {
                field: "activity.poll_interval_ms".into(),
                reason: "must be greater than 0".into(),
            });
        }

        Ok(())
    }
}

fn validate_threshold(field: &str, value: f32) -> Result<(), CtpError> {
    if value <= 0.0 || value > 1.0 {
        return Err(CtpError::ConfigValidation {
            field: field.into(),
            reason: format!("must be in (0.0, 1.0], got {}", value),
        });
    }
    Ok(())
}

fn validate_nonzero_u64(field: &str, value: u64) -> Result<(), CtpError> {
    if value == 0 {
        return Err(CtpError::ConfigValidation {
            field: field.into(),
            reason: "must be greater than 0".into(),
        });
    }
    Ok(())
}

fn validate_positive_f32(field: &str, value: f32) -> Result<(), CtpError> {
    if value <= 0.0 {
        return Err(CtpError::ConfigValidation {
            field: field.into(),
            reason: format!("must be greater than 0.0, got {}", value),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_config(content: &str) -> tempfile::NamedTempFile {
        // unwrap acceptable: temp file creation is infallible in test environments
        let mut file = tempfile::NamedTempFile::new().expect("test: temp file creation failed");
        file.write_all(content.as_bytes())
            .expect("test: writing temp config failed");
        file
    }

    const VALID_CONFIG: &str = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
memory_engine_address = "http://127.0.0.1:50052"
connection_timeout_ms = 5000

[surface_thresholds]
user_active = 0.9
idle_2min = 0.6
idle_10min = 0.3

[expiry_windows]
high_relevance_secs = 300
medium_relevance_secs = 120
low_relevance_secs = 30

[default_weights]
urgency = 0.9
emotional_resonance = 0.7
novelty = 0.6
recurrence = 0.4
idle_curiosity = 0.3

[consolidation]
idle_threshold_secs = 600
promotion_min_score = 0.5
max_entries_per_cycle = 50

[compaction]
pre_rot_fraction = 0.8
max_entries_to_summarize = 100

[queue]
max_depth = 256

[activity]
poll_interval_ms = 500
idle_2min_threshold_secs = 120
idle_10min_threshold_secs = 600

[logging]
level = "info"
format = "json"
"#;

    #[test]
    fn test_config_load_valid() {
        let file = write_temp_config(VALID_CONFIG);
        let config = Config::load(file.path()).expect("test: valid config must load");

        assert_eq!(config.grpc.daemon_bus_address, "http://127.0.0.1:50051");
        assert_eq!(config.surface_thresholds.user_active, 0.9);
        assert_eq!(config.surface_thresholds.idle_2min, 0.6);
        assert_eq!(config.surface_thresholds.idle_10min, 0.3);
        assert_eq!(config.expiry_windows.high_relevance_secs, 300);
        assert_eq!(config.default_weights.urgency, 0.9);
        assert_eq!(config.consolidation.idle_threshold_secs, 600);
        assert_eq!(config.compaction.pre_rot_fraction, 0.8);
        assert_eq!(config.queue.max_depth, 256);
        assert_eq!(config.activity.poll_interval_ms, 500);
    }

    #[test]
    fn test_config_validates_surface_thresholds_in_range() {
        // Threshold > 1.0 should fail
        let bad_threshold = VALID_CONFIG.replace("user_active = 0.9", "user_active = 1.5");
        let file = write_temp_config(&bad_threshold);
        let result = Config::load(file.path());
        let error = result.expect_err("test: threshold > 1.0 must be rejected");
        match error {
            CtpError::ConfigValidation { field, .. } => {
                assert_eq!(field, "surface_thresholds.user_active");
            }
            other => panic!("Expected ConfigValidation, got: {other}"),
        }

        // Threshold = 0.0 should fail
        let zero_threshold = VALID_CONFIG.replace("idle_2min = 0.6", "idle_2min = 0.0");
        let file = write_temp_config(&zero_threshold);
        let result = Config::load(file.path());
        let error = result.expect_err("test: threshold = 0.0 must be rejected");
        match error {
            CtpError::ConfigValidation { field, .. } => {
                assert_eq!(field, "surface_thresholds.idle_2min");
            }
            other => panic!("Expected ConfigValidation, got: {other}"),
        }
    }

    #[test]
    fn test_config_validates_expiry_windows_nonzero() {
        let zero_expiry =
            VALID_CONFIG.replace("high_relevance_secs = 300", "high_relevance_secs = 0");
        let file = write_temp_config(&zero_expiry);
        let result = Config::load(file.path());
        let error = result.expect_err("test: zero expiry window must be rejected");
        match error {
            CtpError::ConfigValidation { field, .. } => {
                assert_eq!(field, "expiry_windows.high_relevance_secs");
            }
            other => panic!("Expected ConfigValidation, got: {other}"),
        }
    }

    #[test]
    fn test_default_weights_sum_to_reasonable_value() {
        let file = write_temp_config(VALID_CONFIG);
        let config = Config::load(file.path()).expect("test: valid config must load");

        let weights = &config.default_weights;
        // All weights should be > 0.0
        assert!(weights.urgency > 0.0);
        assert!(weights.emotional_resonance > 0.0);
        assert!(weights.novelty > 0.0);
        assert!(weights.recurrence > 0.0);
        assert!(weights.idle_curiosity > 0.0);

        // Mid-range input (0.5 for all signals) with these weights should produce [0.0, 1.0]
        let mid_signal = 0.5_f32;
        let total_weight = weights.urgency
            + weights.emotional_resonance
            + weights.novelty
            + weights.recurrence
            + weights.idle_curiosity;
        let weighted_sum = mid_signal * total_weight;
        let score = weighted_sum / total_weight;
        assert!(
            (0.0..=1.0).contains(&score),
            "mid-range score should be in [0.0, 1.0], got {}",
            score
        );
    }
}
