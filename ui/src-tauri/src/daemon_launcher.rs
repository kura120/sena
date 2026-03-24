use crate::config::DaemonBusLaunchConfig;
use crate::toast;
use std::path::PathBuf;
use std::process::Command;
use tauri::AppHandle;
use tonic::transport::Channel;
use tracing::{error, info, warn};

/// Result of attempting to ensure daemon-bus is running
#[derive(Debug)]
pub enum DaemonLaunchResult {
    /// daemon-bus was already running
    AlreadyRunning,
    /// daemon-bus was launched successfully
    Launched,
    /// Failed to find the binary
    BinaryNotFound(#[allow(dead_code)] String),
    /// Launch timed out
    Timeout,
    /// Other error
    Error(#[allow(dead_code)] String),
}

/// Check if daemon-bus is reachable by attempting a gRPC connection.
async fn is_daemon_bus_running(address: &str, timeout_ms: u64) -> bool {
    let result = Channel::from_shared(address.to_string()).map(|endpoint| {
        endpoint.connect_timeout(std::time::Duration::from_millis(timeout_ms))
    });

    let endpoint = match result {
        Ok(ep) => ep,
        Err(_) => return false,
    };

    // Try to connect with a short timeout
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            endpoint.connect(),
        )
        .await,
        Ok(Ok(_))
    )
}

/// Resolve the daemon-bus binary path relative to the workspace root.
/// The workspace root is inferred from the Cargo manifest directory:
/// ui/src-tauri/ -> up two levels -> workspace root
fn resolve_binary_path(config: &DaemonBusLaunchConfig) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // manifest_dir = <workspace>/ui/src-tauri
    // workspace root = manifest_dir.parent().parent()
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest_dir);

    let binary_path = if cfg!(debug_assertions) {
        &config.binary_path
    } else {
        &config.binary_path_release
    };

    let resolved = workspace_root.join(binary_path);
    info!(?resolved, "Resolved daemon-bus binary path");
    resolved
}

/// Resolve the daemon-bus config path relative to workspace root.
fn resolve_config_path(config: &DaemonBusLaunchConfig) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest_dir);

    workspace_root.join(&config.config_path)
}

/// Spawn daemon-bus as a fully detached process.
#[cfg(target_os = "windows")]
fn spawn_daemon_bus_detached(
    binary_path: &std::path::Path,
    config_path: &std::path::Path,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    // DETACHED_PROCESS (0x00000008) | CREATE_NEW_PROCESS_GROUP (0x00000200)
    const DETACHED_FLAGS: u32 = 0x00000008 | 0x00000200;

    info!(
        binary = %binary_path.display(),
        config = %config_path.display(),
        "Spawning daemon-bus (Windows detached)"
    );

    let _child = Command::new(binary_path)
        .env("DAEMON_BUS_CONFIG", config_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_FLAGS)
        .spawn()
        .map_err(|e| format!("Failed to spawn daemon-bus: {}", e))?;

    // Drop the child handle — process is fully detached
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn spawn_daemon_bus_detached(
    binary_path: &std::path::Path,
    config_path: &std::path::Path,
) -> Result<(), String> {
    info!(
        binary = %binary_path.display(),
        config = %config_path.display(),
        "Spawning daemon-bus (Unix detached)"
    );

    let _child = Command::new(binary_path)
        .env("DAEMON_BUS_CONFIG", config_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn daemon-bus: {}", e))?;

    // Drop the child handle — process is fully detached
    Ok(())
}

/// Ensure daemon-bus is running. If not, launch it and wait for it to become available.
pub async fn ensure_daemon_bus_running(
    app: &AppHandle,
    config: &DaemonBusLaunchConfig,
) -> DaemonLaunchResult {
    let connection_check_timeout_ms = 2000;

    // 1. Check if daemon-bus is already running
    if is_daemon_bus_running(&config.address, connection_check_timeout_ms).await {
        info!("daemon-bus is already running");
        return DaemonLaunchResult::AlreadyRunning;
    }

    // 2. Not running — resolve binary path and check it exists
    let binary_path = resolve_binary_path(config);
    if !binary_path.exists() {
        let msg = format!(
            "daemon-bus binary not found at {}. Run: cargo build -p daemon-bus",
            binary_path.display()
        );
        error!(%msg, "daemon-bus binary not found");
        toast::emit_toast(app, "error", "Cannot Start", &msg);
        return DaemonLaunchResult::BinaryNotFound(msg);
    }

    let config_path = resolve_config_path(config);

    // 3. Emit starting toast
    toast::emit_toast(app, "info", "Starting Sena", "Launching daemon-bus...");

    // 4. Spawn daemon-bus
    if let Err(e) = spawn_daemon_bus_detached(&binary_path, &config_path) {
        error!(error = %e, "Failed to spawn daemon-bus");
        toast::emit_toast(app, "error", "Launch Failed", &e);
        return DaemonLaunchResult::Error(e);
    }

    info!("daemon-bus spawned successfully");

    // 5. Wait for daemon-bus to become available
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(config.launch_timeout_ms);
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        if start.elapsed() > timeout {
            warn!("daemon-bus launch timed out");
            toast::emit_toast(
                app,
                "warning",
                "Slow Start",
                "daemon-bus is taking longer than expected",
            );
            return DaemonLaunchResult::Timeout;
        }

        tokio::time::sleep(poll_interval).await;

        if is_daemon_bus_running(&config.address, connection_check_timeout_ms).await {
            info!("daemon-bus is now running after launch");
            toast::emit_toast(app, "success", "Sena Connected", "daemon-bus is running");
            return DaemonLaunchResult::Launched;
        }
    }
}

/// Kill the process listening on a specific port.
#[cfg(target_os = "windows")]
pub async fn kill_process_on_port(port: u16) -> Result<(), String> {
    info!(port, "Killing process on port (Windows)");

    // Run netstat to find the PID
    let output = tokio::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .await
        .map_err(|e| format!("Failed to run netstat: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let search_pattern = format!(":{}", port);

    let mut pids: Vec<u32> = Vec::new();
    for line in stdout.lines() {
        if line.contains(&search_pattern) && line.contains("LISTENING") {
            // Parse PID from last column
            if let Some(pid_str) = line.split_whitespace().last() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    if pid > 0 && !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
        }
    }

    if pids.is_empty() {
        info!(port, "No process found listening on port");
        return Ok(());
    }

    for pid in &pids {
        info!(pid, port, "Killing process");
        let kill_result = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .await;

        match kill_result {
            Ok(output) if output.status.success() => {
                info!(pid, "Process killed successfully");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(pid, stderr = %stderr, "taskkill returned non-zero");
            }
            Err(e) => {
                error!(pid, error = %e, "Failed to run taskkill");
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub async fn kill_process_on_port(port: u16) -> Result<(), String> {
    info!(port, "Killing process on port (macOS)");

    let output = tokio::process::Command::new("lsof")
        .args(["-ti", &format!("tcp:{}", port)])
        .output()
        .await
        .map_err(|e| format!("Failed to run lsof: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<&str> = stdout.trim().lines().collect();

    if pids.is_empty() {
        info!(port, "No process found on port");
        return Ok(());
    }

    for pid in pids {
        info!(pid, "Killing process");
        let _ = tokio::process::Command::new("kill")
            .args(["-9", pid])
            .output()
            .await;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
pub async fn kill_process_on_port(port: u16) -> Result<(), String> {
    info!(port, "Killing process on port (Linux)");

    let output = tokio::process::Command::new("lsof")
        .args(["-ti", &format!("tcp:{}", port)])
        .output()
        .await
        .map_err(|e| format!("Failed to run lsof: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<&str> = stdout.trim().lines().collect();

    if pids.is_empty() {
        info!(port, "No process found on port");
        return Ok(());
    }

    for pid in pids {
        info!(pid, "Killing process");
        let _ = tokio::process::Command::new("kill")
            .args(["-9", pid])
            .output()
            .await;
    }

    Ok(())
}

/// Reboot daemon-bus: gracefully shut down, kill by port, wait, respawn.
pub async fn reboot_daemon_bus(
    app: &AppHandle,
    config: &DaemonBusLaunchConfig,
) -> Result<(), String> {
    use tauri::Emitter;

    toast::emit_toast(app, "warning", "Rebooting", "Restarting daemon-bus...");

    // Emit subsystems-reset to all windows so UI clears immediately
    if let Err(e) = app.emit("subsystems-reset", ()) {
        warn!(error = %e, "Failed to emit subsystems-reset event");
    }

    // 1. Try graceful shutdown via gRPC (best effort)
    // The daemon-bus should handle a shutdown signal if available.
    // For now, skip graceful — go straight to port kill as the primary mechanism.
    info!("Attempting to stop daemon-bus via port kill");

    // 2. Parse port from address
    let port = parse_port_from_address(&config.address).unwrap_or(50051);

    // 3. Kill the process on the port
    kill_process_on_port(port).await?;

    // 4. Wait for port to free
    info!("Waiting for port to free");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 5. Spawn fresh daemon-bus
    let binary_path = resolve_binary_path(config);
    if !binary_path.exists() {
        let msg = format!("daemon-bus binary not found at {}", binary_path.display());
        toast::emit_toast(app, "error", "Reboot Failed", &msg);
        return Err(msg);
    }

    let config_path = resolve_config_path(config);

    if let Err(e) = spawn_daemon_bus_detached(&binary_path, &config_path) {
        toast::emit_toast(app, "error", "Reboot Failed", &e);
        return Err(e);
    }

    // 6. Wait for daemon-bus to come back
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(config.launch_timeout_ms);
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        if start.elapsed() > timeout {
            toast::emit_toast(
                app,
                "warning",
                "Slow Reboot",
                "daemon-bus is taking longer than expected to restart",
            );
            return Ok(());
        }

        tokio::time::sleep(poll_interval).await;

        if is_daemon_bus_running(&config.address, 2000).await {
            info!("daemon-bus restarted successfully");
            toast::emit_toast(
                app,
                "success",
                "Reboot Complete",
                "daemon-bus restarted successfully",
            );
            return Ok(());
        }
    }
}

/// Parse port number from a gRPC address like "http://127.0.0.1:50051"
fn parse_port_from_address(address: &str) -> Option<u16> {
    address
        .rsplit(':')
        .next()
        .and_then(|port_str| port_str.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_port_from_address() {
        assert_eq!(
            parse_port_from_address("http://127.0.0.1:50051"),
            Some(50051)
        );
        assert_eq!(parse_port_from_address("http://localhost:8080"), Some(8080));
        assert_eq!(parse_port_from_address("invalid"), None);
    }

    #[test]
    fn test_resolve_binary_path_exists() {
        let config = DaemonBusLaunchConfig::default();
        let path = resolve_binary_path(&config);
        // Path should be constructed correctly even if binary doesn't exist
        assert!(path.to_string_lossy().contains("daemon-bus"));
    }
}
