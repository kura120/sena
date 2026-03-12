//! Strongly-typed configuration for daemon-bus.
//!
//! Every tunable value lives in `config/daemon-bus.toml` — nothing is hardcoded
//! in source. This module deserializes the TOML file into typed structs that are
//! passed by reference throughout the daemon-bus process.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::{ErrorCode, SenaError, SenaResult};

// ─────────────────────────────────────────────────────────────────────────────
// Root config
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonBusConfig {
    pub grpc: GrpcConfig,
    pub bus: BusConfig,
    pub boot: BootConfig,
    pub supervisor: SupervisorConfig,
    pub arbitration: ArbitrationConfig,
    pub watchdog: WatchdogConfig,
    pub logging: LoggingConfig,
}

// ─────────────────────────────────────────────────────────────────────────────
// gRPC server
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    pub bind_address: String,
    pub bind_port: u16,
}

impl GrpcConfig {
    /// Returns the full socket address string for tonic server binding.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.bind_address, self.bind_port)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal event bus (tokio broadcast channel)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct BusConfig {
    pub channel_capacity: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot sequence
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct BootConfig {
    /// Overall boot sequence timeout in milliseconds.
    pub total_timeout_ms: u64,
    /// Per-subsystem boot configuration keyed by subsystem name.
    pub subsystems: HashMap<String, BootSubsystemConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BootSubsystemConfig {
    /// The boot signal this subsystem emits when ready.
    pub signal: String,
    /// Optional skip signal for optional subsystems (e.g. LORA_SKIPPED).
    pub skip_signal: Option<String>,
    /// How long to wait for this subsystem's ready signal before considering it failed.
    pub timeout_ms: u64,
    /// Whether this subsystem must signal ready for SENA_READY to be emitted.
    /// If a required subsystem fails, boot halts entirely.
    pub required: bool,
    /// List of boot signals that must have been received before this subsystem
    /// is expected to start. Defines the boot DAG ordering from PRD §3.2.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Process supervision
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SupervisorConfig {
    /// Maximum number of restart attempts before entering degraded mode.
    /// PRD §10.2: exactly 3.
    pub max_retries: u32,
    /// Backoff durations in milliseconds for each retry attempt.
    /// PRD §10.2: [0, 5000, 30000] — immediate, 5s, 30s.
    /// Length must equal max_retries.
    pub backoff_ms: Vec<u64>,
    /// Grace period after spawning a process before considering it a failed start.
    pub process_start_grace_ms: u64,
    /// Per-subsystem spawn definitions keyed by subsystem name.
    pub subsystems: HashMap<String, SupervisedSubsystemConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SupervisedSubsystemConfig {
    pub subsystem_id: String,
    /// Executable or command to spawn.
    pub command: String,
    /// Command-line arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for the spawned process.
    pub working_directory: String,
    /// Whether to spawn this subsystem during the boot sequence.
    /// False for on-demand subsystems like UI.
    #[serde(default = "default_spawn_at_boot")]
    pub spawn_at_boot: bool,
}

fn default_spawn_at_boot() -> bool {
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Priority arbitration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ArbitrationConfig {
    /// Hard ceiling on how long any Tier 2 escalation can be held (ms).
    pub max_escalation_duration_ms: u64,
    /// Default duration granted if the requester does not specify one (ms).
    pub default_escalation_duration_ms: u64,
    /// Maximum number of queued escalation requests before new ones are denied.
    pub max_queue_depth: usize,
    /// Subsystem ID that always wins Tier 2 over all others.
    /// PRD §9.4: reactive loop always takes precedence over CTP.
    pub reactive_subsystem_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Watchdog
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogConfig {
    /// Default wall-clock timeout for tasks if not specified per-task (ms).
    pub default_task_timeout_ms: u64,
    /// Absolute maximum timeout any task can request (ms).
    pub max_task_timeout_ms: u64,
    /// How often the watchdog sweeps for expired tasks (ms).
    pub sweep_interval_ms: u64,
    /// Maximum number of concurrently tracked tasks.
    pub max_tracked_tasks: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Logging
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Minimum log level: trace, debug, info, warn, error.
    pub level: String,
    /// Output format: "json" for structured production logs, "pretty" for dev.
    pub format: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Loading
// ─────────────────────────────────────────────────────────────────────────────

impl DaemonBusConfig {
    /// Load and parse the daemon-bus configuration from the given TOML file path.
    ///
    /// Uses `spawn_blocking` to avoid blocking I/O on the async runtime.
    /// Called once at startup — the returned config is then passed by reference.
    pub async fn load(path: &Path) -> SenaResult<Self> {
        let path_owned = path.to_path_buf();

        let raw_toml = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path_owned))
            .await
            .map_err(|join_error| {
                SenaError::new(
                    ErrorCode::ConfigLoadFailed,
                    "config read task was cancelled or panicked",
                )
                .with_debug_context(format!("JoinError: {join_error}"))
            })?
            .map_err(|io_error| {
                SenaError::new(
                    ErrorCode::ConfigLoadFailed,
                    format!("failed to read config file: {}", path.display()),
                )
                .with_debug_context(format!("IO error: {io_error}"))
            })?;

        let config: DaemonBusConfig = toml::from_str(&raw_toml).map_err(|parse_error| {
            SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!("failed to parse config file: {}", path.display()),
            )
            .with_debug_context(format!("TOML parse error: {parse_error}"))
        })?;

        config.validate()?;

        Ok(config)
    }

    /// Validates invariants that TOML deserialization alone cannot enforce.
    fn validate(&self) -> SenaResult<()> {
        // Backoff array length must match max_retries so every retry attempt
        // has a defined delay. A mismatch means the config author made a typo.
        if self.supervisor.backoff_ms.len() != self.supervisor.max_retries as usize {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                format!(
                    "supervisor.backoff_ms length ({}) must equal supervisor.max_retries ({})",
                    self.supervisor.backoff_ms.len(),
                    self.supervisor.max_retries
                ),
            ));
        }

        // Broadcast channel capacity must be > 0.
        if self.bus.channel_capacity == 0 {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "bus.channel_capacity must be greater than zero",
            ));
        }

        // Watchdog sweep interval must be > 0 to avoid a busy spin loop.
        if self.watchdog.sweep_interval_ms == 0 {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "watchdog.sweep_interval_ms must be greater than zero",
            ));
        }

        // Max task timeout must be >= default task timeout.
        if self.watchdog.max_task_timeout_ms < self.watchdog.default_task_timeout_ms {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "watchdog.max_task_timeout_ms must be >= watchdog.default_task_timeout_ms",
            ));
        }

        // Max escalation duration must be >= default escalation duration.
        if self.arbitration.max_escalation_duration_ms
            < self.arbitration.default_escalation_duration_ms
        {
            return Err(SenaError::new(
                ErrorCode::ConfigLoadFailed,
                "arbitration.max_escalation_duration_ms must be >= arbitration.default_escalation_duration_ms",
            ));
        }

        Ok(())
    }
}
