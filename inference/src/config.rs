use crate::error::InferenceError;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub model: ModelConfig,
    pub runtime: RuntimeConfig,
    #[serde(default = "default_streaming_config")]
    pub streaming: StreamingConfig,
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
pub struct ModelConfig {
    pub model_id: String,
    pub model_path: String,
    pub gpu_layers: u32,
    pub context_length: u32,
    pub vram_budget_mb: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    pub request_queue_max_depth: usize,
    pub request_timeout_ms: u64,
    pub oom_retry_gpu_layer_divisor: u32,
    #[serde(default = "default_stream_channel_capacity")]
    pub stream_channel_capacity: usize,
    #[serde(default = "default_swap_drain_timeout_ms")]
    pub swap_drain_timeout_ms: u64,
}

fn default_stream_channel_capacity() -> usize {
    32
}

fn default_swap_drain_timeout_ms() -> u64 {
    5000
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamingConfig {
    pub heartbeat_interval_ms: u64,
    pub heartbeat_token: String,
}

fn default_streaming_config() -> StreamingConfig {
    StreamingConfig {
        heartbeat_interval_ms: 10000,
        heartbeat_token: String::new(),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, InferenceError> {
        let content = std::fs::read_to_string(path).map_err(|e| InferenceError::ConfigLoad {
            reason: format!("failed to read config file: {}", e),
        })?;

        let config: Config = toml::from_str(&content).map_err(|e| InferenceError::ConfigLoad {
            reason: format!("failed to parse TOML: {}", e),
        })?;

        // Validate configuration
        if config.model.vram_budget_mb == 0 {
            return Err(InferenceError::ConfigValidation {
                field: "model.vram_budget_mb".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if config.runtime.request_queue_max_depth == 0 {
            return Err(InferenceError::ConfigValidation {
                field: "runtime.request_queue_max_depth".into(),
                reason: "must be greater than 0".into(),
            });
        }

        if config.runtime.request_timeout_ms == 0 {
            return Err(InferenceError::ConfigValidation {
                field: "runtime.request_timeout_ms".into(),
                reason: "must be greater than 0".into(),
            });
        }

        // gpu_layers = 0 is valid — it means CPU-only inference.
        // No validation needed for that field.

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
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "default"
model_path = "models/test.gguf"
gpu_layers = 35
context_length = 4096
vram_budget_mb = 4096

[runtime]
request_queue_max_depth = 64
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

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
        assert_eq!(config.grpc.listen_port, 50055);
        assert_eq!(config.model.model_id, "default");
        assert_eq!(config.model.vram_budget_mb, 4096);
        assert_eq!(config.runtime.request_queue_max_depth, 64);
    }

    #[test]
    fn test_config_validation_zero_vram() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "default"
model_path = "models/test.gguf"
gpu_layers = 35
context_length = 4096
vram_budget_mb = 0

[runtime]
request_queue_max_depth = 64
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let result = Config::load(file.path());

        // expect Err: this test validates that zero vram_budget is rejected
        let error = result.expect_err("test: zero vram_budget_mb must be rejected");
        match error {
            InferenceError::ConfigValidation { field, .. } => {
                assert_eq!(field, "model.vram_budget_mb");
            }
            other => panic!("Expected ConfigValidation error, got: {other}"),
        }
    }

    #[test]
    fn test_config_validation_zero_queue() {
        let invalid_config = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "default"
model_path = "models/test.gguf"
gpu_layers = 35
context_length = 4096
vram_budget_mb = 4096

[runtime]
request_queue_max_depth = 0
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(invalid_config);
        let result = Config::load(file.path());

        // expect Err: this test validates that zero queue depth is rejected
        let error = result.expect_err("test: zero request_queue_max_depth must be rejected");
        match error {
            InferenceError::ConfigValidation { field, .. } => {
                assert_eq!(field, "runtime.request_queue_max_depth");
            }
            other => panic!("Expected ConfigValidation error, got: {other}"),
        }
    }

    #[test]
    fn test_config_validation_missing_model() {
        let nonexistent_path = Path::new("/nonexistent/path/to/config.toml");
        let result = Config::load(nonexistent_path);

        // expect Err: this test validates that missing file returns ConfigLoad
        let error = result.expect_err("test: nonexistent path must fail");
        match error {
            InferenceError::ConfigLoad { .. } => {}
            other => panic!("Expected ConfigLoad error, got: {other}"),
        }
    }

    #[test]
    fn test_config_load_with_streaming() {
        let config_with_streaming = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "default"
model_path = "models/test.gguf"
gpu_layers = 35
context_length = 4096
vram_budget_mb = 4096

[runtime]
request_queue_max_depth = 64
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[streaming]
heartbeat_interval_ms = 10000
heartbeat_token = ""

[logging]
level = "info"
format = "json"
"#;

        let file = write_temp_config(config_with_streaming);
        // unwrap acceptable: this test asserts that config with streaming section loads successfully
        let config = Config::load(file.path()).expect("test: config with streaming must load");

        assert_eq!(config.streaming.heartbeat_interval_ms, 10000);
        assert_eq!(config.streaming.heartbeat_token, "");
    }
}
