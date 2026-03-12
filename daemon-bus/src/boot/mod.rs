//! Boot sequence orchestrator.
//!
//! Tracks subsystem ready signals with per-subsystem timeouts from config,
//! halts boot if a required subsystem fails, and emits `SENA_READY` when all
//! required subsystems have signaled ready. The boot sequence ordering is
//! defined by the dependency graph in `config/daemon-bus.toml` — each subsystem
//! declares which boot signals it depends on before it is expected to start.
//!
//! Boot sequence (PRD §3.2):
//! ```text
//! 1.   daemon-bus starts                    → DAEMON_BUS_READY
//! 2.   memory-engine initializes            → MEMORY_ENGINE_READY
//! 3.   platform layer starts                → PLATFORM_READY
//! 4.   sena-agents starts                   → AGENTS_READY
//! 5.   Ollama loads, health check passes    → OLLAMA_READY
//! 5.5. ModelProbe runs capability battery   → MODEL_PROFILE_READY
//! 5.6. LoRA Manager loads active adapter    → LORA_READY | LORA_SKIPPED
//! 6.   CTP starts                           → CTP_READY
//! 7.   sena-ui spawns (on user activation)  → UI_READY
//! 8.   [ SENA_READY ]
//! ```
//!
//! daemon-bus never enters a partially-ready state — it is either fully ready
//! or explicitly not ready (PRD §10.5).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;

use crate::bus::{EventBus, InternalBusEvent};
use crate::config::BootConfig;
use crate::error::SenaResult;
use crate::generated::sena_daemonbus_v1::EventTopic;
use crate::supervisor::Supervisor;

// ─────────────────────────────────────────────────────────────────────────────
// Boot signal registry
// ─────────────────────────────────────────────────────────────────────────────

/// Tracks which boot signals have been received, which are still pending,
/// and whether the overall boot sequence has completed or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootPhase {
    /// Boot is in progress — waiting for subsystem signals.
    InProgress,
    /// All required subsystems signaled ready — SENA_READY emitted.
    Ready,
    /// A required subsystem failed to signal within its timeout — boot halted.
    Failed { reason: String },
}

/// Per-subsystem tracking entry used during boot.
#[derive(Debug, Clone)]
#[allow(dead_code)] // depends_on used for boot DAG validation and ordering in full implementation.
struct BootEntry {
    /// The signal name this subsystem emits when ready (e.g. "MEMORY_ENGINE_READY").
    signal: String,
    /// Optional alternative signal that counts as completion for optional subsystems
    /// (e.g. "LORA_SKIPPED").
    skip_signal: Option<String>,
    /// How long to wait for this subsystem's ready signal before considering it failed.
    timeout: Duration,
    /// Whether this subsystem must signal ready for SENA_READY to be emitted.
    required: bool,
    /// Boot signals that must have been received before this subsystem is expected to start.
    depends_on: Vec<String>,
    /// Whether this subsystem's ready signal (or skip signal) has been received.
    signaled: bool,
    /// Config key name used for supervisor operations.
    config_key: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot Orchestrator
// ─────────────────────────────────────────────────────────────────────────────

/// The boot sequence orchestrator.
///
/// Cloneable via inner `Arc` — the gRPC BootService and the main startup
/// routine both hold handles to the same orchestrator instance.
#[derive(Clone)]
pub struct BootOrchestrator {
    inner: Arc<BootOrchestratorInner>,
}

struct BootOrchestratorInner {
    /// Per-subsystem boot tracking, keyed by signal name.
    entries: RwLock<HashMap<String, BootEntry>>,
    /// Set of all signals that have been received so far.
    received_signals: RwLock<HashSet<String>>,
    /// Current boot phase — watched by anyone who needs to block on SENA_READY.
    phase: watch::Sender<BootPhase>,
    /// Receiver side kept alive so new watchers can be created via `subscribe()`.
    _phase_receiver: watch::Receiver<BootPhase>,
    /// Overall boot timeout from config.
    total_timeout: Duration,
    /// Event bus for publishing boot lifecycle events.
    event_bus: EventBus,
    /// Supervisor handle for spawning subsystems during boot.
    supervisor: Supervisor,
    /// Handles for per-subsystem timeout tasks. Stored so they are never
    /// silently dropped — dropped JoinHandles cancel work.
    timeout_handles: RwLock<Vec<JoinHandle<()>>>,
    /// Handle for the overall boot timeout task.
    total_timeout_handle: RwLock<Option<JoinHandle<()>>>,
}

impl BootOrchestrator {
    /// Create a new boot orchestrator from config.
    ///
    /// Registers all subsystems from the boot config but does not start the
    /// boot sequence. Call `run()` to begin.
    pub fn new(config: &BootConfig, event_bus: EventBus, supervisor: Supervisor) -> Self {
        let mut entries = HashMap::new();

        for (config_key, subsystem_config) in &config.subsystems {
            let entry = BootEntry {
                signal: subsystem_config.signal.clone(),
                skip_signal: subsystem_config.skip_signal.clone(),
                timeout: Duration::from_millis(subsystem_config.timeout_ms),
                required: subsystem_config.required,
                depends_on: subsystem_config.depends_on.clone(),
                signaled: false,
                config_key: config_key.clone(),
            };

            tracing::info!(
                subsystem = "daemon_bus",
                event_type = "boot_register",
                signal = %subsystem_config.signal,
                required = subsystem_config.required,
                timeout_ms = subsystem_config.timeout_ms,
                depends_on = ?subsystem_config.depends_on,
                "registered subsystem for boot sequence"
            );

            // Key by the signal name so lookups on signal receipt are O(1).
            entries.insert(subsystem_config.signal.clone(), entry.clone());

            // If a skip signal is defined, also register a lookup entry
            // pointing to the same boot entry so that receiving either signal
            // completes this subsystem's boot step.
            if let Some(ref skip_signal) = subsystem_config.skip_signal {
                let skip_entry = BootEntry {
                    signal: subsystem_config.signal.clone(),
                    skip_signal: Some(skip_signal.clone()),
                    timeout: Duration::from_millis(subsystem_config.timeout_ms),
                    required: subsystem_config.required,
                    depends_on: subsystem_config.depends_on.clone(),
                    signaled: false,
                    config_key: config_key.clone(),
                };
                entries.insert(skip_signal.clone(), skip_entry);
            }
        }

        let (phase_sender, phase_receiver) = watch::channel(BootPhase::InProgress);

        Self {
            inner: Arc::new(BootOrchestratorInner {
                entries: RwLock::new(entries),
                received_signals: RwLock::new(HashSet::new()),
                phase: phase_sender,
                _phase_receiver: phase_receiver,
                total_timeout: Duration::from_millis(config.total_timeout_ms),
                event_bus,
                supervisor,
                timeout_handles: RwLock::new(Vec::new()),
                total_timeout_handle: RwLock::new(None),
            }),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Run the boot sequence
    // ─────────────────────────────────────────────────────────────────────────

    /// Start the boot sequence.
    ///
    /// 1. Emits `DAEMON_BUS_READY` (daemon-bus is always the first signal).
    /// 2. Spawns all boot-time subsystems via the supervisor.
    /// 3. Starts per-subsystem timeout watchers.
    /// 4. Starts the overall boot timeout.
    ///
    /// Returns a `watch::Receiver<BootPhase>` that transitions to
    /// `BootPhase::Ready` when all required subsystems signal, or
    /// `BootPhase::Failed` if a required subsystem times out.
    pub async fn run(&self) -> SenaResult<watch::Receiver<BootPhase>> {
        tracing::info!(
            subsystem = "daemon_bus",
            event_type = "boot_sequence_started",
            total_timeout_ms = self.inner.total_timeout.as_millis() as u64,
            "boot sequence started"
        );

        // Step 1: daemon-bus signals itself as ready — it is the root of the
        // dependency graph and does not depend on anything.
        self.signal_ready("daemon_bus", "DAEMON_BUS_READY").await?;

        // Step 2: spawn all boot-time subsystems via the supervisor.
        let spawn_list = self.inner.supervisor.boot_spawn_list();
        for subsystem_name in &spawn_list {
            let subsystem_name_clone = subsystem_name.clone();
            match self
                .inner
                .supervisor
                .spawn_subsystem(&subsystem_name_clone)
                .await
            {
                Ok(pid) => {
                    tracing::info!(
                        subsystem = %subsystem_name,
                        event_type = "boot_spawn",
                        pid = pid,
                        "spawned subsystem during boot"
                    );
                }
                Err(spawn_error) => {
                    // A spawn failure during boot is a hard stop for required subsystems.
                    // For optional ones, we log and continue.
                    let is_required = self
                        .is_subsystem_required_by_config_key(subsystem_name)
                        .await;
                    if is_required {
                        let reason = format!(
                            "required subsystem '{}' failed to spawn: {}",
                            subsystem_name, spawn_error
                        );
                        self.halt_boot(&reason).await;
                        return Ok(self.subscribe());
                    }

                    tracing::warn!(
                        subsystem = %subsystem_name,
                        event_type = "boot_spawn_optional_failed",
                        error = %spawn_error,
                        "optional subsystem failed to spawn during boot — continuing"
                    );
                }
            }
        }

        // Step 3: start per-subsystem timeout watchers for required subsystems.
        self.start_subsystem_timeout_watchers().await;

        // Step 4: start the overall boot timeout.
        self.start_total_timeout_watcher().await;

        Ok(self.subscribe())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Signal reception
    // ─────────────────────────────────────────────────────────────────────────

    /// Called when a subsystem signals that it is ready.
    ///
    /// This is the entry point for the gRPC `BootService.SignalReady` handler
    /// and for internal signals (e.g. DAEMON_BUS_READY from the orchestrator itself).
    pub async fn signal_ready(&self, subsystem_id: &str, signal_name: &str) -> SenaResult<bool> {
        // Check if boot is already terminal (Ready or Failed).
        {
            let current_phase = self.current_phase();
            match current_phase {
                BootPhase::Ready => {
                    tracing::debug!(
                        subsystem = %subsystem_id,
                        event_type = "boot_signal_after_ready",
                        signal = %signal_name,
                        "boot signal received after SENA_READY — ignored"
                    );
                    return Ok(true);
                }
                BootPhase::Failed { .. } => {
                    tracing::debug!(
                        subsystem = %subsystem_id,
                        event_type = "boot_signal_after_failure",
                        signal = %signal_name,
                        "boot signal received after boot failure — ignored"
                    );
                    return Ok(false);
                }
                BootPhase::InProgress => { /* continue processing */ }
            }
        }

        // Record the signal.
        {
            let mut received = self.inner.received_signals.write().await;
            received.insert(signal_name.to_string());
        }

        // Mark the corresponding boot entry as signaled.
        {
            let mut entries = self.inner.entries.write().await;
            if let Some(entry) = entries.get_mut(signal_name) {
                entry.signaled = true;

                tracing::info!(
                    subsystem = %subsystem_id,
                    event_type = "boot_signal_received",
                    signal = %signal_name,
                    required = entry.required,
                    "boot signal received"
                );

                // If this is a skip signal, also mark the primary signal's entry.
                let primary_signal = entry.signal.clone();
                if primary_signal != signal_name {
                    if let Some(primary_entry) = entries.get_mut(&primary_signal) {
                        primary_entry.signaled = true;
                    }
                }
            } else {
                // Signal not registered in boot config — could be a late or unknown signal.
                tracing::debug!(
                    subsystem = %subsystem_id,
                    event_type = "boot_signal_unknown",
                    signal = %signal_name,
                    "received boot signal not registered in boot config"
                );
            }
        }

        // Mark the subsystem as ready in the supervisor.
        let config_key = {
            let entries = self.inner.entries.read().await;
            entries
                .get(signal_name)
                .map(|entry| entry.config_key.clone())
        };
        if let Some(key) = config_key {
            self.inner.supervisor.mark_ready(&key).await;
        }

        // Publish the signal to the event bus.
        let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::signal(
            EventTopic::TopicBootSignal,
            subsystem_id,
            "",
        ));

        // Check if all required subsystems have now signaled.
        self.check_boot_complete().await;

        Ok(true)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Completion check
    // ─────────────────────────────────────────────────────────────────────────

    /// Check whether all required subsystems have signaled ready.
    /// If so, emit SENA_READY and transition to `BootPhase::Ready`.
    async fn check_boot_complete(&self) {
        let entries = self.inner.entries.read().await;

        // Collect unique required subsystems by their primary signal name.
        // Skip signal entries share the same primary signal — avoid counting
        // a subsystem twice.
        let mut required_signals: HashSet<&str> = HashSet::new();
        let mut signaled_required: HashSet<&str> = HashSet::new();

        for entry in entries.values() {
            if entry.required {
                required_signals.insert(&entry.signal);
                if entry.signaled {
                    signaled_required.insert(&entry.signal);
                }
            }
        }

        let all_required_signaled = required_signals
            .iter()
            .all(|signal| signaled_required.contains(signal));

        if !all_required_signaled {
            let pending: Vec<&str> = required_signals
                .difference(&signaled_required)
                .copied()
                .collect();
            tracing::debug!(
                subsystem = "daemon_bus",
                event_type = "boot_progress",
                signaled = signaled_required.len(),
                total_required = required_signals.len(),
                pending = ?pending,
                "boot progress — waiting for remaining required subsystems"
            );
            return;
        }

        // Release the read lock before mutating phase.
        drop(entries);

        // All required subsystems have signaled — boot is complete.
        tracing::info!(
            subsystem = "daemon_bus",
            event_type = "sena_ready",
            "all required subsystems signaled — emitting SENA_READY"
        );

        // Transition phase to Ready.
        // send() only fails if all receivers are dropped, which should not
        // happen during boot — but we handle it gracefully regardless.
        let _send_result = self.inner.phase.send(BootPhase::Ready);

        // Publish SENA_READY to the event bus.
        let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::signal(
            EventTopic::TopicBootSignal,
            "daemon_bus",
            "",
        ));

        // Cancel all remaining timeout watchers — they are no longer needed.
        self.cancel_all_timeout_watchers().await;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Boot failure
    // ─────────────────────────────────────────────────────────────────────────

    /// Halt the boot sequence with a failure reason.
    ///
    /// PRD §10.5: Sena never silently starts in a degraded state due to a boot failure.
    /// A minimal system notification is shown explaining which subsystem failed and why.
    /// Sena does not enter a partially-ready state.
    async fn halt_boot(&self, reason: &str) {
        // Only halt once — ignore if already terminal.
        let current = self.current_phase();
        if !matches!(current, BootPhase::InProgress) {
            return;
        }

        tracing::error!(
            subsystem = "daemon_bus",
            event_type = "boot_failed",
            reason = %reason,
            "boot sequence halted — Sena is not ready"
        );

        let _send_result = self.inner.phase.send(BootPhase::Failed {
            reason: reason.to_string(),
        });

        // Publish boot failure to the event bus.
        let payload = format!(r#"{{"reason":"{}"}}"#, reason).into_bytes();
        let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::new(
            EventTopic::TopicBootFailed,
            "daemon_bus",
            payload,
            "",
        ));

        self.cancel_all_timeout_watchers().await;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Timeout watchers
    // ─────────────────────────────────────────────────────────────────────────

    /// Start per-subsystem timeout watchers for all required subsystems.
    ///
    /// Each watcher sleeps for the subsystem's configured timeout. When it
    /// wakes, it checks if the signal has been received. If not, it halts
    /// the boot sequence.
    async fn start_subsystem_timeout_watchers(&self) {
        let entries = self.inner.entries.read().await;

        // Deduplicate by primary signal to avoid starting two watchers for a
        // subsystem that has both a signal and a skip_signal.
        let mut seen_signals: HashSet<String> = HashSet::new();
        let mut watchers: Vec<(String, Duration, bool)> = Vec::new();

        for entry in entries.values() {
            if seen_signals.contains(&entry.signal) {
                continue;
            }
            seen_signals.insert(entry.signal.clone());

            // Only required subsystems get timeout watchers that halt boot.
            // Optional subsystems time out silently.
            watchers.push((entry.signal.clone(), entry.timeout, entry.required));
        }

        drop(entries);

        let mut handles = self.inner.timeout_handles.write().await;

        for (signal_name, timeout, required) in watchers {
            let orchestrator = self.clone();
            let signal_clone = signal_name.clone();

            let handle = tokio::spawn(async move {
                tokio::time::sleep(timeout).await;

                // Check if the signal was received while we were sleeping.
                let received = {
                    let received_signals = orchestrator.inner.received_signals.read().await;
                    received_signals.contains(&signal_clone)
                };

                if received {
                    return;
                }

                // Also check for the skip signal if one exists.
                let skip_received = {
                    let entries = orchestrator.inner.entries.read().await;
                    if let Some(entry) = entries.get(&signal_clone) {
                        if let Some(ref skip_signal) = entry.skip_signal {
                            let received_signals = orchestrator.inner.received_signals.read().await;
                            received_signals.contains(skip_signal)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if skip_received {
                    return;
                }

                // Signal was not received within the timeout.
                if required {
                    let reason = format!(
                        "required subsystem signal '{}' not received within {}ms timeout",
                        signal_clone,
                        timeout.as_millis()
                    );
                    orchestrator.halt_boot(&reason).await;
                } else {
                    tracing::warn!(
                        subsystem = "daemon_bus",
                        event_type = "boot_optional_timeout",
                        signal = %signal_clone,
                        timeout_ms = timeout.as_millis() as u64,
                        "optional subsystem did not signal within timeout — continuing boot"
                    );
                }
            });

            handles.push(handle);
        }
    }

    /// Start the overall boot timeout watcher.
    ///
    /// If SENA_READY is not reached within `config.total_timeout_ms`, boot
    /// is halted regardless of which individual subsystems have signaled.
    async fn start_total_timeout_watcher(&self) {
        let orchestrator = self.clone();
        let total_timeout = self.inner.total_timeout;

        let handle = tokio::spawn(async move {
            tokio::time::sleep(total_timeout).await;

            let current = orchestrator.current_phase();
            if matches!(current, BootPhase::InProgress) {
                let reason = format!(
                    "overall boot timeout exceeded ({}ms) — not all required subsystems signaled",
                    total_timeout.as_millis()
                );
                orchestrator.halt_boot(&reason).await;
            }
        });

        let mut total_handle = self.inner.total_timeout_handle.write().await;
        *total_handle = Some(handle);
    }

    /// Cancel all timeout watcher tasks.
    async fn cancel_all_timeout_watchers(&self) {
        {
            let mut handles = self.inner.timeout_handles.write().await;
            for handle in handles.drain(..) {
                handle.abort();
            }
        }
        {
            let mut total_handle = self.inner.total_timeout_handle.write().await;
            if let Some(handle) = total_handle.take() {
                handle.abort();
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Queries
    // ─────────────────────────────────────────────────────────────────────────

    /// Returns the current boot phase.
    pub fn current_phase(&self) -> BootPhase {
        self.inner.phase.borrow().clone()
    }

    /// Returns a new `watch::Receiver` that will be notified when the boot
    /// phase changes. Use this to `await` SENA_READY from any context.
    pub fn subscribe(&self) -> watch::Receiver<BootPhase> {
        self.inner.phase.subscribe()
    }

    /// Returns a map of signal name → whether it has been received.
    /// Used by the gRPC `BootService.GetBootStatus` handler.
    pub async fn get_signal_statuses(&self) -> HashMap<String, bool> {
        let entries = self.inner.entries.read().await;
        let received = self.inner.received_signals.read().await;

        let mut seen: HashSet<String> = HashSet::new();
        let mut statuses = HashMap::new();

        for entry in entries.values() {
            if seen.contains(&entry.signal) {
                continue;
            }
            seen.insert(entry.signal.clone());

            let is_signaled = received.contains(&entry.signal)
                || entry
                    .skip_signal
                    .as_ref()
                    .map(|skip| received.contains(skip))
                    .unwrap_or(false);

            statuses.insert(entry.signal.clone(), is_signaled);
        }

        statuses
    }

    /// Check whether the boot sequence has completed successfully.
    pub fn is_ready(&self) -> bool {
        matches!(self.current_phase(), BootPhase::Ready)
    }

    /// Check whether the boot sequence has failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.current_phase(), BootPhase::Failed { .. })
    }

    /// Returns a snapshot of the current boot status suitable for the gRPC response.
    pub async fn get_boot_status(&self) -> BootStatusSnapshot {
        let signal_statuses = self.get_signal_statuses().await;
        BootStatusSnapshot {
            subsystem_signals: signal_statuses,
            sena_ready: self.is_ready(),
            phase: self.current_phase(),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Internal helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Check if a subsystem is required, looked up by its config key name
    /// (the key in `[boot.subsystems.<key>]`).
    async fn is_subsystem_required_by_config_key(&self, config_key: &str) -> bool {
        let entries = self.inner.entries.read().await;
        entries
            .values()
            .any(|entry| entry.config_key == config_key && entry.required)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot status snapshot
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of the boot sequence state for gRPC and diagnostics.
#[derive(Debug, Clone)]
pub struct BootStatusSnapshot {
    /// Map of signal name → whether it has been received.
    pub subsystem_signals: HashMap<String, bool>,
    /// True only when all required subsystems have signaled and SENA_READY is emitted.
    pub sena_ready: bool,
    /// The current boot phase.
    pub phase: BootPhase,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::{BootConfig, BootSubsystemConfig, SupervisorConfig};

    /// Build a minimal boot config for testing with two required subsystems
    /// and one optional subsystem.
    fn test_boot_config() -> BootConfig {
        let mut subsystems = HashMap::new();

        subsystems.insert(
            "memory_engine".to_string(),
            BootSubsystemConfig {
                signal: "MEMORY_ENGINE_READY".to_string(),
                skip_signal: None,
                timeout_ms: 500,
                required: true,
                depends_on: vec!["DAEMON_BUS_READY".to_string()],
            },
        );

        subsystems.insert(
            "ctp".to_string(),
            BootSubsystemConfig {
                signal: "CTP_READY".to_string(),
                skip_signal: None,
                timeout_ms: 500,
                required: true,
                depends_on: vec!["MEMORY_ENGINE_READY".to_string()],
            },
        );

        subsystems.insert(
            "lora_manager".to_string(),
            BootSubsystemConfig {
                signal: "LORA_READY".to_string(),
                skip_signal: Some("LORA_SKIPPED".to_string()),
                timeout_ms: 500,
                required: false,
                depends_on: vec![],
            },
        );

        BootConfig {
            total_timeout_ms: 2000,
            subsystems,
        }
    }

    /// Build a minimal supervisor config for testing — no real processes.
    fn test_supervisor_config() -> SupervisorConfig {
        SupervisorConfig {
            max_retries: 3,
            backoff_ms: vec![0, 5000, 30000],
            process_start_grace_ms: 1000,
            subsystems: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn boot_completes_when_all_required_signal() {
        let bus = EventBus::new(64);
        let supervisor = Supervisor::new(test_supervisor_config(), bus.clone());
        let orchestrator = BootOrchestrator::new(&test_boot_config(), bus, supervisor);

        // Simulate daemon-bus ready (normally done inside run()).
        orchestrator
            .signal_ready("daemon_bus", "DAEMON_BUS_READY")
            .await
            .expect("signal should succeed");

        // Signal both required subsystems.
        orchestrator
            .signal_ready("memory_engine", "MEMORY_ENGINE_READY")
            .await
            .expect("signal should succeed");

        assert!(
            !orchestrator.is_ready(),
            "should not be ready yet — CTP missing"
        );

        orchestrator
            .signal_ready("ctp", "CTP_READY")
            .await
            .expect("signal should succeed");

        assert!(
            orchestrator.is_ready(),
            "should be ready — all required subsystems signaled"
        );
    }

    #[tokio::test]
    async fn skip_signal_counts_as_completion() {
        let bus = EventBus::new(64);
        let supervisor = Supervisor::new(test_supervisor_config(), bus.clone());

        // Config where lora_manager is required to test the skip path.
        let mut config = test_boot_config();
        config
            .subsystems
            .get_mut("lora_manager")
            .expect("should exist")
            .required = true;

        let orchestrator = BootOrchestrator::new(&config, bus, supervisor);

        orchestrator
            .signal_ready("daemon_bus", "DAEMON_BUS_READY")
            .await
            .expect("signal should succeed");
        orchestrator
            .signal_ready("memory_engine", "MEMORY_ENGINE_READY")
            .await
            .expect("signal should succeed");
        orchestrator
            .signal_ready("ctp", "CTP_READY")
            .await
            .expect("signal should succeed");

        // Use the skip signal instead of the primary signal.
        orchestrator
            .signal_ready("lora_manager", "LORA_SKIPPED")
            .await
            .expect("signal should succeed");

        assert!(
            orchestrator.is_ready(),
            "skip signal should count as completion for required subsystem"
        );
    }

    #[tokio::test]
    async fn required_timeout_halts_boot() {
        let bus = EventBus::new(64);
        let supervisor = Supervisor::new(test_supervisor_config(), bus.clone());

        // Use a very short timeout.
        let mut config = test_boot_config();
        config.total_timeout_ms = 100;
        config
            .subsystems
            .get_mut("memory_engine")
            .expect("should exist")
            .timeout_ms = 50;

        let orchestrator = BootOrchestrator::new(&config, bus, supervisor);

        // Start the timeout watchers manually (normally done in run()).
        orchestrator
            .signal_ready("daemon_bus", "DAEMON_BUS_READY")
            .await
            .expect("signal should succeed");
        orchestrator.start_subsystem_timeout_watchers().await;
        orchestrator.start_total_timeout_watcher().await;

        // Do NOT signal memory_engine — let it timeout.
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert!(
            orchestrator.is_failed(),
            "boot should have failed due to memory_engine timeout"
        );
    }

    #[tokio::test]
    async fn optional_timeout_does_not_halt_boot() {
        let bus = EventBus::new(64);
        let supervisor = Supervisor::new(test_supervisor_config(), bus.clone());

        let mut config = test_boot_config();
        // Make lora_manager optional (it already is, but be explicit).
        config
            .subsystems
            .get_mut("lora_manager")
            .expect("should exist")
            .required = false;
        config
            .subsystems
            .get_mut("lora_manager")
            .expect("should exist")
            .timeout_ms = 50;

        let orchestrator = BootOrchestrator::new(&config, bus, supervisor);

        orchestrator
            .signal_ready("daemon_bus", "DAEMON_BUS_READY")
            .await
            .expect("signal should succeed");
        orchestrator.start_subsystem_timeout_watchers().await;

        // Signal required subsystems.
        orchestrator
            .signal_ready("memory_engine", "MEMORY_ENGINE_READY")
            .await
            .expect("signal should succeed");
        orchestrator
            .signal_ready("ctp", "CTP_READY")
            .await
            .expect("signal should succeed");

        // Wait for optional timeout to fire.
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(
            orchestrator.is_ready(),
            "boot should succeed despite optional subsystem timeout"
        );
        assert!(
            !orchestrator.is_failed(),
            "boot should not be marked as failed"
        );
    }

    #[tokio::test]
    async fn get_boot_status_returns_correct_snapshot() {
        let bus = EventBus::new(64);
        let supervisor = Supervisor::new(test_supervisor_config(), bus.clone());
        let orchestrator = BootOrchestrator::new(&test_boot_config(), bus, supervisor);

        orchestrator
            .signal_ready("daemon_bus", "DAEMON_BUS_READY")
            .await
            .expect("signal should succeed");
        orchestrator
            .signal_ready("memory_engine", "MEMORY_ENGINE_READY")
            .await
            .expect("signal should succeed");

        let status = orchestrator.get_boot_status().await;

        assert!(!status.sena_ready, "should not be ready yet");
        assert_eq!(
            status.subsystem_signals.get("MEMORY_ENGINE_READY"),
            Some(&true)
        );
        assert_eq!(status.subsystem_signals.get("CTP_READY"), Some(&false));
    }

    #[tokio::test]
    async fn duplicate_signal_is_idempotent() {
        let bus = EventBus::new(64);
        let supervisor = Supervisor::new(test_supervisor_config(), bus.clone());
        let orchestrator = BootOrchestrator::new(&test_boot_config(), bus, supervisor);

        orchestrator
            .signal_ready("daemon_bus", "DAEMON_BUS_READY")
            .await
            .expect("signal should succeed");

        // Signal memory_engine twice — second should be harmless.
        orchestrator
            .signal_ready("memory_engine", "MEMORY_ENGINE_READY")
            .await
            .expect("first signal should succeed");
        orchestrator
            .signal_ready("memory_engine", "MEMORY_ENGINE_READY")
            .await
            .expect("duplicate signal should succeed");

        let status = orchestrator.get_boot_status().await;
        assert_eq!(
            status.subsystem_signals.get("MEMORY_ENGINE_READY"),
            Some(&true)
        );
    }
}
