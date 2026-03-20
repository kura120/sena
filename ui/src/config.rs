use serde::Deserialize;
use std::fs;

#[derive(Debug, Clone, Deserialize)]
pub struct UiConfig {
    pub grpc: GrpcConfig,
    pub window: WindowConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub reactive_loop_address: String,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub min_width: f64,
    pub min_height: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl UiConfig {
    pub fn load() -> Self {
        let config_str = fs::read_to_string("ui/config/ui.toml")
            .unwrap_or_else(|_| include_str!("../config/ui.toml").to_string());
        toml::from_str(&config_str).expect("Failed to parse config")
    }
}