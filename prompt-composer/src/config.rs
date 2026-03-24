use crate::error::PromptComposerError;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub boot: BootConfig,
    pub context_window: ContextWindowConfig,
    pub sacred: SacredConfig,
    #[serde(default)]
    pub response_format: ResponseFormatConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub listen_address: String,
    pub listen_port: u16,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BootConfig {
    pub ready_signal_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextWindowConfig {
    /// ESU savings threshold — if TOON saves more than this percentage vs JSON, prefer TOON.
    pub esu_savings_threshold: f32,
    /// Rough estimate of tokens per character for budget calculations.
    pub tokens_per_char_estimate: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SacredConfig {
    /// Field identifiers that must never be dropped (e.g., "soulbox_snapshot", "user_intent").
    pub sacred_fields: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[derive(Default)]
pub struct ResponseFormatConfig {
    /// Instruction injected into the system prompt to prevent reasoning leakage.
    #[serde(default)]
    pub system_instruction: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, PromptComposerError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| PromptComposerError::ConfigLoad {
                reason: format!("failed to read config file: {}", e),
            })?;

        let config: Config =
            toml::from_str(&content).map_err(|e| PromptComposerError::ConfigLoad {
                reason: format!("failed to parse TOML: {}", e),
            })?;

        // Validate configuration
        if config.context_window.esu_savings_threshold < 0.0
            || config.context_window.esu_savings_threshold > 1.0
        {
            return Err(PromptComposerError::ConfigValidation {
                field: "context_window.esu_savings_threshold".into(),
                reason: "must be between 0.0 and 1.0".into(),
            });
        }

        if config.context_window.tokens_per_char_estimate <= 0.0 {
            return Err(PromptComposerError::ConfigValidation {
                field: "context_window.tokens_per_char_estimate".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if config.sacred.sacred_fields.is_empty() {
            return Err(PromptComposerError::ConfigValidation {
                field: "sacred.sacred_fields".into(),
                reason: "must specify at least one sacred field".into(),
            });
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_config(content: &str) -> tempfile::NamedTempFile {
        // unwrap acceptable: temp file creation is infallible in test environments;
        // a failure here means the OS temp directory is broken, not a test logic error.
        let mut file = tempfile::NamedTempFile::new().expect("test: temp file creation failed");
        file.write_all(content.as_bytes())
            .expect("test: writing temp config failed");
        file
    }

    const VALID_CONFIG: &str = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50057
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[context_window]
esu_savings_threshold = 0.15
tokens_per_char_estimate = 0.25

[sacred]
sacred_fields = ["soulbox_snapshot", "user_intent"]

[response_format]
system_instruction = "Respond conversationally and directly."

[logging]
level = "info"
format = "json"
"#;

    #[test]
    fn test_config_load_valid() {
        let file = write_temp_config(VALID_CONFIG);
        // unwrap acceptable: this test asserts that valid config loads successfully
        let config = Config::load(file.path()).expect("test: valid config must load");

        assert_eq!(config.grpc.daemon_bus_address, "http://127.0.0.1:50051");
        assert_eq!(config.grpc.listen_port, 50057);
        assert_eq!(config.context_window.esu_savings_threshold, 0.15);
        assert_eq!(config.sacred.sacred_fields.len(), 2);
    }

    #[test]
    fn test_config_validation_invalid_threshold() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50057
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[context_window]
esu_savings_threshold = 1.5
tokens_per_char_estimate = 0.25

[sacred]
sacred_fields = ["soulbox_snapshot", "user_intent"]

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let result = Config::load(file.path());

        // expect Err: this test validates that invalid threshold is rejected
        let error = result.expect_err("test: invalid threshold must be rejected");
        match error {
            PromptComposerError::ConfigValidation { field, .. } => {
                assert_eq!(field, "context_window.esu_savings_threshold");
            }
            other => panic!("Expected ConfigValidation error, got: {other}"),
        }
    }

    #[test]
    fn test_config_validation_no_sacred_fields() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50057
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[context_window]
esu_savings_threshold = 0.15
tokens_per_char_estimate = 0.25

[sacred]
sacred_fields = []

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let result = Config::load(file.path());

        // expect Err: this test validates that empty sacred fields is rejected
        let error = result.expect_err("test: empty sacred_fields must be rejected");
        match error {
            PromptComposerError::ConfigValidation { field, .. } => {
                assert_eq!(field, "sacred.sacred_fields");
            }
            other => panic!("Expected ConfigValidation error, got: {other}"),
        }
    }

    #[test]
    fn test_config_validation_missing_file() {
        let nonexistent_path = Path::new("/nonexistent/path/to/config.toml");
        let result = Config::load(nonexistent_path);

        // expect Err: this test validates that missing file returns ConfigLoad
        let error = result.expect_err("test: nonexistent path must fail");
        match error {
            PromptComposerError::ConfigLoad { .. } => {}
            other => panic!("Expected ConfigLoad error, got: {other}"),
        }
    }

    #[test]
    fn test_response_format_instruction_loaded() {
        let file = write_temp_config(VALID_CONFIG);
        // unwrap acceptable: this test asserts that valid config loads successfully
        let config = Config::load(file.path()).expect("test: valid config must load");

        assert!(!config.response_format.system_instruction.is_empty());
        assert_eq!(config.response_format.system_instruction, "Respond conversationally and directly.");
    }

    #[test]
    fn test_response_format_defaults_to_empty() {
        let config_without_response_format = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50057
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[context_window]
esu_savings_threshold = 0.15
tokens_per_char_estimate = 0.25

[sacred]
sacred_fields = ["soulbox_snapshot", "user_intent"]

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(config_without_response_format);
        // unwrap acceptable: this test asserts that config without response_format loads successfully
        let config = Config::load(file.path()).expect("test: config without response_format must load");

        // Should default to empty string
        assert_eq!(config.response_format.system_instruction, "");
    }
}
