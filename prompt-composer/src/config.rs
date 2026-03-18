use std::path::Path;
use serde::Deserialize;
use crate::error::PcError;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub esu: EsuConfig,
    pub budget: BudgetConfig,
    pub drop_order: DropOrderConfig,
    pub telemetry: TelemetryConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub listen_port: u16,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EsuConfig {
    pub save_threshold: f32,
    pub latency_threshold_ms: u64,
    pub sacred_always_json: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BudgetConfig {
    pub output_reserve_tokens: u32,
    pub min_sacred_headroom_pct: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DropOrderConfig {
    pub tiers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelemetryConfig {
    pub emit_encoding_choices: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, PcError> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            PcError::Config(format!(
                "failed to read config file '{}': {}",
                path.display(),
                e
            ))
        })?;

        let config: Config = toml::from_str(&raw).map_err(|e| {
            PcError::Config(format!(
                "failed to parse config '{}': {}",
                path.display(),
                e
            ))
        })?;

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), PcError> {
        if self.esu.save_threshold <= 0.0 || self.esu.save_threshold >= 1.0 {
            return Err(PcError::Config(format!(
                "esu.save_threshold must be in (0.0, 1.0), got {}",
                self.esu.save_threshold
            )));
        }

        if self.budget.output_reserve_tokens == 0 {
            return Err(PcError::Config(
                "budget.output_reserve_tokens must be > 0".to_string(),
            ));
        }

        if self.drop_order.tiers.is_empty() {
            return Err(PcError::Config(
                "drop_order.tiers must not be empty".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const VALID_TOML: &str = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_port = 50054
connection_timeout_ms = 5000

[esu]
save_threshold = 0.15
latency_threshold_ms = 10
sacred_always_json = true

[budget]
output_reserve_tokens = 512
min_sacred_headroom_pct = 0.1

[drop_order]
tiers = ["telemetry", "os_context", "short_term", "long_term", "episodic"]

[telemetry]
emit_encoding_choices = true

[logging]
level = "info"
format = "json"
"#;

    #[test]
    fn test_config_load_valid() {
        let temp_dir = std::env::temp_dir().join("pc_config_valid_test");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let config_path = temp_dir.join("prompt-composer.toml");
        let mut file = std::fs::File::create(&config_path).expect("create file");
        file.write_all(VALID_TOML.as_bytes()).expect("write file");

        let config = Config::load(&config_path).expect("config should load");
        assert_eq!(config.grpc.daemon_bus_address, "http://127.0.0.1:50051");
        assert_eq!(config.esu.save_threshold, 0.15);
        assert_eq!(config.budget.output_reserve_tokens, 512);
        assert_eq!(config.drop_order.tiers.len(), 5);
        assert!(config.telemetry.emit_encoding_choices);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_config_save_threshold_in_range() {
        let temp_dir = std::env::temp_dir().join("pc_config_threshold_test");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        // Test threshold = 0.0 (invalid)
        let bad_toml = VALID_TOML.replace("save_threshold = 0.15", "save_threshold = 0.0");
        let config_path = temp_dir.join("prompt-composer.toml");
        std::fs::write(&config_path, &bad_toml).expect("write file");
        let result = Config::load(&config_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("save_threshold"));

        // Test threshold = 1.0 (invalid)
        let bad_toml = VALID_TOML.replace("save_threshold = 0.15", "save_threshold = 1.0");
        std::fs::write(&config_path, &bad_toml).expect("write file");
        let result = Config::load(&config_path);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_config_output_reserve_nonzero() {
        let temp_dir = std::env::temp_dir().join("pc_config_reserve_test");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let bad_toml = VALID_TOML.replace("output_reserve_tokens = 512", "output_reserve_tokens = 0");
        let config_path = temp_dir.join("prompt-composer.toml");
        std::fs::write(&config_path, &bad_toml).expect("write file");
        let result = Config::load(&config_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("output_reserve_tokens"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
