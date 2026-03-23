//! Configuration loading and validation for reactive-loop.

use crate::error::ReactiveLoopError;
use serde::Deserialize;
use std::path::Path;

/// Top-level configuration for reactive-loop.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub boot: BootConfig,
    pub inference: InferenceConfig,
    #[serde(default = "default_post_processing_config")]
    pub post_processing: PostProcessingConfig,
    pub fallback: FallbackConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub inference_address: String,
    pub prompt_composer_address: String,
    pub memory_engine_address: String,
    pub listen_address: String,
    pub listen_port: u16,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BootConfig {
    pub ready_signal_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InferenceConfig {
    pub default_max_tokens: u32,
    pub default_temperature: f32,
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostProcessingConfig {
    pub filter_heartbeat_tokens: bool,
    #[serde(default)]
    pub strip_reasoning_tags: bool,
    #[serde(default)]
    pub reasoning_markers: Vec<String>,
}

fn default_post_processing_config() -> PostProcessingConfig {
    PostProcessingConfig {
        filter_heartbeat_tokens: true,
        strip_reasoning_tags: false,
        reasoning_markers: Vec::new(),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FallbackConfig {
    pub unavailable_response: String,
    pub minimal_context_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn load(path: &Path) -> Result<Self, ReactiveLoopError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;

        // Validate critical fields
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values.
    fn validate(&self) -> Result<(), ReactiveLoopError> {
        if self.grpc.listen_port == 0 {
            return Err(ReactiveLoopError::ConfigValidation {
                field: "grpc.listen_port".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if self.grpc.connection_timeout_ms == 0 {
            return Err(ReactiveLoopError::ConfigValidation {
                field: "grpc.connection_timeout_ms".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if self.inference.default_max_tokens == 0 {
            return Err(ReactiveLoopError::ConfigValidation {
                field: "inference.default_max_tokens".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if self.inference.default_temperature < 0.0 || self.inference.default_temperature > 2.0 {
            return Err(ReactiveLoopError::ConfigValidation {
                field: "inference.default_temperature".into(),
                reason: "must be between 0.0 and 2.0".into(),
            });
        }

        if self.inference.request_timeout_ms == 0 {
            return Err(ReactiveLoopError::ConfigValidation {
                field: "inference.request_timeout_ms".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if self.fallback.unavailable_response.is_empty() {
            return Err(ReactiveLoopError::ConfigValidation {
                field: "fallback.unavailable_response".into(),
                reason: "cannot be empty".into(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const VALID_CONFIG: &str = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
inference_address = "http://127.0.0.1:50055"
prompt_composer_address = "http://127.0.0.1:50057"
memory_engine_address = "http://127.0.0.1:50052"
listen_address = "0.0.0.0"
listen_port = 50058
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[inference]
default_max_tokens = 1024
default_temperature = 0.7
request_timeout_ms = 30000

[fallback]
unavailable_response = "I'm currently unable to process your request. The inference engine is not available."
minimal_context_enabled = true

[logging]
level = "info"
format = "json"
"#;

    fn write_temp_config(content: &str) -> tempfile::NamedTempFile {
        let mut file =
            tempfile::NamedTempFile::new().expect("test: temp file creation must succeed");
        file.write_all(content.as_bytes())
            .expect("test: writing temp config must succeed");
        file
    }

    #[test]
    fn test_config_load_valid() {
        let file = write_temp_config(VALID_CONFIG);
        let config = Config::load(file.path()).expect("test: valid config must load");

        assert_eq!(
            config.grpc.daemon_bus_address,
            "http://127.0.0.1:50051"
        );
        assert_eq!(config.grpc.inference_address, "http://127.0.0.1:50055");
        assert_eq!(
            config.grpc.prompt_composer_address,
            "http://127.0.0.1:50057"
        );
        assert_eq!(
            config.grpc.memory_engine_address,
            "http://127.0.0.1:50052"
        );
        assert_eq!(config.grpc.listen_port, 50058);
        assert_eq!(config.grpc.connection_timeout_ms, 5000);
        assert_eq!(config.inference.default_max_tokens, 1024);
        assert_eq!(config.inference.default_temperature, 0.7);
        assert!(config.fallback.minimal_context_enabled);
    }

    #[test]
    fn test_config_validation_zero_port() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
inference_address = "http://127.0.0.1:50055"
prompt_composer_address = "http://127.0.0.1:50057"
memory_engine_address = "http://127.0.0.1:50052"
listen_address = "0.0.0.0"
listen_port = 0
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[inference]
default_max_tokens = 1024
default_temperature = 0.7
request_timeout_ms = 30000

[fallback]
unavailable_response = "Error"
minimal_context_enabled = true

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let error =
            Config::load(file.path()).expect_err("test: zero listen_port must be rejected");

        match error {
            ReactiveLoopError::ConfigValidation { field, .. } => {
                assert_eq!(field, "grpc.listen_port");
            }
            other => panic!("Expected ConfigValidation error, got: {:?}", other),
        }
    }

    #[test]
    fn test_config_validation_zero_max_tokens() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
inference_address = "http://127.0.0.1:50055"
prompt_composer_address = "http://127.0.0.1:50057"
memory_engine_address = "http://127.0.0.1:50052"
listen_address = "0.0.0.0"
listen_port = 50058
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[inference]
default_max_tokens = 0
default_temperature = 0.7
request_timeout_ms = 30000

[fallback]
unavailable_response = "Error"
minimal_context_enabled = true

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let error =
            Config::load(file.path()).expect_err("test: zero max_tokens must be rejected");

        match error {
            ReactiveLoopError::ConfigValidation { field, .. } => {
                assert_eq!(field, "inference.default_max_tokens");
            }
            other => panic!("Expected ConfigValidation error, got: {:?}", other),
        }
    }

    #[test]
    fn test_config_validation_invalid_temperature() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
inference_address = "http://127.0.0.1:50055"
prompt_composer_address = "http://127.0.0.1:50057"
memory_engine_address = "http://127.0.0.1:50052"
listen_address = "0.0.0.0"
listen_port = 50058
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[inference]
default_max_tokens = 1024
default_temperature = 3.0
request_timeout_ms = 30000

[fallback]
unavailable_response = "Error"
minimal_context_enabled = true

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let error =
            Config::load(file.path()).expect_err("test: invalid temperature must be rejected");

        match error {
            ReactiveLoopError::ConfigValidation { field, .. } => {
                assert_eq!(field, "inference.default_temperature");
            }
            other => panic!("Expected ConfigValidation error, got: {:?}", other),
        }
    }

    #[test]
    fn test_config_validation_empty_fallback_response() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
inference_address = "http://127.0.0.1:50055"
prompt_composer_address = "http://127.0.0.1:50057"
memory_engine_address = "http://127.0.0.1:50052"
listen_address = "0.0.0.0"
listen_port = 50058
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[inference]
default_max_tokens = 1024
default_temperature = 0.7
request_timeout_ms = 30000

[fallback]
unavailable_response = ""
minimal_context_enabled = true

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let error = Config::load(file.path())
            .expect_err("test: empty fallback response must be rejected");

        match error {
            ReactiveLoopError::ConfigValidation { field, .. } => {
                assert_eq!(field, "fallback.unavailable_response");
            }
            other => panic!("Expected ConfigValidation error, got: {:?}", other),
        }
    }

    #[test]
    fn test_config_load_nonexistent_file() {
        let error = Config::load(Path::new("/nonexistent/path.toml"))
            .expect_err("test: loading nonexistent file must fail");

        match error {
            ReactiveLoopError::ConfigLoad { .. } => {
                // Expected
            }
            other => panic!("Expected ConfigLoad error, got: {:?}", other),
        }
    }

    #[test]
    fn test_config_load_invalid_toml() {
        let invalid_toml = "this is not valid toml {[}";
        let file = write_temp_config(invalid_toml);
        let error =
            Config::load(file.path()).expect_err("test: invalid TOML must be rejected");

        match error {
            ReactiveLoopError::ConfigLoad { .. } => {
                // Expected
            }
            other => panic!("Expected ConfigLoad error, got: {:?}", other),
        }
    }

    #[test]
    fn test_config_load_with_post_processing() {
        let config_with_post_processing = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
inference_address = "http://127.0.0.1:50055"
prompt_composer_address = "http://127.0.0.1:50057"
memory_engine_address = "http://127.0.0.1:50052"
listen_address = "0.0.0.0"
listen_port = 50058
connection_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 5000

[inference]
default_max_tokens = 1024
default_temperature = 0.7
request_timeout_ms = 120000

[post_processing]
filter_heartbeat_tokens = true

[fallback]
unavailable_response = "I'm currently unable to process your request. The inference engine is not available."
minimal_context_enabled = true

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(config_with_post_processing);
        let config = Config::load(file.path())
            .expect("test: config with post_processing must load");

        assert_eq!(config.inference.request_timeout_ms, 120000);
        assert!(config.post_processing.filter_heartbeat_tokens);
    }
}
