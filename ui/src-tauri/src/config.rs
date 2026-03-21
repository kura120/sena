use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// UI configuration loaded from ui.toml
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub window: WindowConfig,
    pub debug_panel: DebugPanelConfig,
    pub reconnect: ReconnectConfig,
    pub logging: LoggingConfig,
    pub overlay: OverlayConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GrpcConfig {
    pub daemon_bus_address: String,
    pub reactive_loop_address: String,
    pub connection_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DebugPanelConfig {
    pub width: f64,
    pub animation_duration_ms: u64,
    pub thought_feed_max: usize,
    pub event_feed_max: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReconnectConfig {
    pub initial_delay_ms: u64,
    pub second_delay_ms: u64,
    pub third_delay_ms: u64,
    pub steady_state_delay_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OverlayConfig {
    pub enabled: bool,
    pub toggle_key: String,
    pub health_window: WindowPosition,
    pub event_bus_window: WindowPosition,
    pub chat_window: WindowPosition,
    pub boot_timeline_window: WindowPosition,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowPosition {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();
        info!(?path, "Loading UI configuration");

        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        let config: Config = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config TOML: {}", e))?;

        Ok(config)
    }

    /// Try to find and load config from multiple possible locations.
    ///
    /// Search order:
    /// 1. `<exe_dir>/config/ui.toml` — production deployments where config is
    ///    bundled alongside the binary
    /// 2. `<manifest_dir>/../config/ui.toml` — development builds: manifest is at
    ///    `ui/src-tauri/`, so parent is `ui/`, and config lives at `ui/config/`
    /// 3. Falls back through current-directory variations for unusual layouts
    pub fn load() -> Result<Self, String> {
        // 1. Try relative to current executable (production layout)
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        if let Some(ref exe_dir) = exe_dir {
            let config_path = exe_dir.join("config").join("ui.toml");
            if config_path.exists() {
                return Self::load_from_file(config_path);
            }
        }

        // 2. Parent of manifest directory: manifest is `ui/src-tauri/`, parent is `ui/`
        //    so this resolves to `ui/config/ui.toml` — correct for dev builds.
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(ui_dir) = manifest_dir.parent() {
            let config_path = ui_dir.join("config").join("ui.toml");
            if config_path.exists() {
                return Self::load_from_file(config_path);
            }
        }

        // 3. Current working directory fallback (for unusual launch configurations)
        if let Ok(cwd) = std::env::current_dir() {
            let config_path = cwd.join("ui").join("config").join("ui.toml");
            if config_path.exists() {
                return Self::load_from_file(config_path);
            }
            let config_path = cwd.join("config").join("ui.toml");
            if config_path.exists() {
                return Self::load_from_file(config_path);
            }
        }

        Err("Could not find ui.toml — expected at <ui-dir>/config/ui.toml".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loads_from_toml() {
        // Load the actual config file from the workspace
        // Manifest dir is ui/src-tauri, config is at ui/config/ui.toml
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config_path = manifest_dir
            .parent() // Go up to ui/
            .unwrap()
            .join("config")
            .join("ui.toml");

        let config = Config::load_from_file(config_path).expect("Failed to load config");

        // Verify expected values
        assert_eq!(config.grpc.daemon_bus_address, "http://127.0.0.1:50051");
        assert_eq!(config.grpc.connection_timeout_ms, 5000);
        assert_eq!(config.window.title, "Sena");
        assert_eq!(config.debug_panel.thought_feed_max, 100);
        assert_eq!(config.debug_panel.event_feed_max, 200);
        assert_eq!(config.overlay.toggle_key, "Insert");
        assert!(config.overlay.enabled);
    }
}
