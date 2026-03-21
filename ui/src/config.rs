use serde::Deserialize;
use std::path::Path;

/// Top-level configuration for the UI subsystem, loaded from `config/ui.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub window: WindowConfig,
    pub debug_panel: DebugPanelConfig,
    pub reconnect: ReconnectConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DebugPanelConfig {
    pub width: f64,
    pub animation_duration_ms: u64,
    pub thought_feed_max: usize,
    pub event_feed_max: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReconnectConfig {
    pub initial_delay_ms: u64,
    pub second_delay_ms: u64,
    pub third_delay_ms: u64,
    pub steady_state_delay_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    /// Load configuration from the given TOML file path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|io_err| ConfigError::ReadFailed {
            path: path.display().to_string(),
            source: io_err,
        })?;
        let config: Config =
            toml::from_str(&content).map_err(|parse_err| ConfigError::ParseFailed {
                path: path.display().to_string(),
                source: parse_err,
            })?;
        Ok(config)
    }
}

#[derive(Debug)]
pub enum ConfigError {
    ReadFailed {
        path: String,
        source: std::io::Error,
    },
    ParseFailed {
        path: String,
        source: toml::de::Error,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::ReadFailed { path, source } => {
                write!(f, "failed to read config file {path}: {source}")
            }
            ConfigError::ParseFailed { path, source } => {
                write!(f, "failed to parse config file {path}: {source}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loads_from_valid_toml() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/ui.toml");
        let config = Config::load(&path).expect("config should load successfully");
        assert_eq!(config.grpc.daemon_bus_address, "http://127.0.0.1:50051");
        assert!(!config.window.title.is_empty());
        assert!(config.debug_panel.thought_feed_max > 0);
        assert!(config.debug_panel.event_feed_max > 0);
    }

    #[test]
    fn test_config_error_on_missing_file() {
        let result = Config::load(Path::new("/nonexistent/path.toml"));
        assert!(result.is_err());
    }
}
