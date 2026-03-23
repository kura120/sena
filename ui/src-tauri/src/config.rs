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
    #[serde(default)]
    pub daemon_bus: DaemonBusLaunchConfig,
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
    #[serde(default = "default_resources_window")]
    pub resources_window: WindowPosition,
    #[serde(default = "default_thought_stream_window")]
    pub thought_stream_window: WindowPosition,
    #[serde(default = "default_memory_stats_window")]
    pub memory_stats_window: WindowPosition,
    #[serde(default = "default_prompt_trace_window")]
    pub prompt_trace_window: WindowPosition,
    #[serde(default = "default_conversation_timeline_window")]
    pub conversation_timeline_window: WindowPosition,
    #[serde(default = "default_widget_bar_window")]
    pub widget_bar_window: WindowPosition,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowPosition {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonBusLaunchConfig {
    #[serde(default = "default_daemon_bus_address")]
    pub address: String,
    #[serde(default = "default_binary_path")]
    pub binary_path: String,
    #[serde(default = "default_binary_path_release")]
    pub binary_path_release: String,
    #[serde(default = "default_launch_timeout_ms")]
    pub launch_timeout_ms: u64,
    #[serde(default = "default_daemon_config_path")]
    pub config_path: String,
}

fn default_daemon_bus_address() -> String {
    "http://127.0.0.1:50051".to_string()
}

#[cfg(target_os = "windows")]
fn default_binary_path() -> String {
    "target/debug/daemon-bus.exe".to_string()
}

#[cfg(not(target_os = "windows"))]
fn default_binary_path() -> String {
    "target/debug/daemon-bus".to_string()
}

#[cfg(target_os = "windows")]
fn default_binary_path_release() -> String {
    "target/release/daemon-bus.exe".to_string()
}

#[cfg(not(target_os = "windows"))]
fn default_binary_path_release() -> String {
    "target/release/daemon-bus".to_string()
}

fn default_launch_timeout_ms() -> u64 {
    15000
}

fn default_daemon_config_path() -> String {
    "daemon-bus/config/daemon-bus.toml".to_string()
}

fn default_resources_window() -> WindowPosition {
    WindowPosition { x: 960.0, y: 620.0, width: 320.0, height: 200.0 }
}

fn default_thought_stream_window() -> WindowPosition {
    WindowPosition { x: 100.0, y: 840.0, width: 400.0, height: 300.0 }
}

fn default_memory_stats_window() -> WindowPosition {
    WindowPosition { x: 520.0, y: 840.0, width: 320.0, height: 200.0 }
}

fn default_prompt_trace_window() -> WindowPosition {
    WindowPosition { x: 1300.0, y: 100.0, width: 450.0, height: 400.0 }
}

fn default_conversation_timeline_window() -> WindowPosition {
    WindowPosition { x: 1300.0, y: 520.0, width: 450.0, height: 350.0 }
}

fn default_widget_bar_window() -> WindowPosition {
    WindowPosition { x: 660.0, y: 20.0, width: 600.0, height: 48.0 }
}

/// Resolve the workspace root directory.
/// Search order:
/// 1. Environment variable SENA_WORKSPACE_ROOT
/// 2. Walk up from current exe looking for Cargo.toml with [workspace]
/// 3. Walk up from CARGO_MANIFEST_DIR
/// 4. Current working directory
pub fn resolve_workspace_root() -> Result<PathBuf, String> {
    // Try env var first
    if let Ok(root) = std::env::var("SENA_WORKSPACE_ROOT") {
        let path = PathBuf::from(root);
        if path.exists() {
            return Ok(path);
        }
    }
    
    // Try walking up from exe directory
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            let cargo_toml = d.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        return Ok(d);
                    }
                }
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    
    // Try CARGO_MANIFEST_DIR (available in dev builds)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = PathBuf::from(manifest_dir);
        // manifest dir is ui/src-tauri, so go up twice to workspace root
        if let Some(workspace) = path.parent().and_then(|p| p.parent()) {
            if workspace.join("Cargo.toml").exists() {
                return Ok(workspace.to_path_buf());
            }
        }
    }
    
    // Fallback to current directory
    std::env::current_dir().map_err(|e| format!("Cannot determine workspace root: {}", e))
}

/// Resolve the config file path for a given subsystem.
/// Maps subsystem names to their config file paths relative to workspace root.
pub fn resolve_subsystem_config_path(subsystem: &str) -> Result<PathBuf, String> {
    let workspace_root = resolve_workspace_root()?;
    
    let relative_path = match subsystem {
        "daemon-bus" => "daemon-bus/config/daemon-bus.toml",
        "inference" => "inference/config/inference.toml",
        "memory-engine" => "memory-engine/config/memory-engine.toml",
        "ctp" => "ctp/config/ctp.toml",
        "prompt-composer" => "prompt-composer/config/prompt-composer.toml",
        "reactive-loop" => "reactive-loop/config/reactive-loop.toml",
        "ui" => "ui/config/ui.toml",
        _ => return Err(format!("Unknown subsystem: {}", subsystem)),
    };
    
    let path = workspace_root.join(relative_path);
    if !path.exists() {
        return Err(format!("Config file not found: {}", path.display()));
    }
    
    Ok(path)
}

impl Default for DaemonBusLaunchConfig {
    fn default() -> Self {
        Self {
            address: default_daemon_bus_address(),
            binary_path: default_binary_path(),
            binary_path_release: default_binary_path_release(),
            launch_timeout_ms: default_launch_timeout_ms(),
            config_path: default_daemon_config_path(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();
        info!(?path, "Loading UI configuration");

        let content =
            fs::read_to_string(path).map_err(|e| format!("Failed to read config file: {}", e))?;

        let config: Config =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config TOML: {}", e))?;

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
