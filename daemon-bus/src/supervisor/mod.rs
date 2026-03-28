//! Process supervision for all child subsystems.
//!
//! daemon-bus owns the lifecycle of every child process in Sena's process tree.
//! It spawns, watches, and restarts subsystems using the exact retry policy
//! defined in PRD §10.2: immediate → 5s backoff → 30s backoff → degraded mode.
//! Max 3 retries — never more.
//!
//! Before any restart attempt, daemon-bus logs the last known state and crash
//! context. It never restarts blindly (per daemon-bus.instructions.md).
//!
//! daemon-bus never restarts itself — nothing supervises daemon-bus.
//!
//! ## Async type recursion
//!
//! The spawn → watch → restart → spawn call chain creates a recursive async
//! type.  We break it with `Box::pin` in `attempt_restart_with_policy` so
//! that the future returned by `spawn_subsystem` is type-erased and the
//! compiler can prove `Send` without computing an infinite type.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::{Mutex as TokioMutex, RwLock};
use tokio::task::JoinHandle;

use crate::bus::{EventBus, InternalBusEvent};
use crate::config::{SupervisedSubsystemConfig, SupervisorConfig};
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::{EventTopic, SubsystemState};

// ─────────────────────────────────────────────────────────────────────────────
// Supervised process state
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime state for a single supervised subsystem process.
#[derive(Debug)]
struct SupervisedProcess {
    /// Static config for this subsystem — command, args, working directory.
    config: SupervisedSubsystemConfig,
    /// Current lifecycle state.
    state: SubsystemState,
    /// Number of restart attempts since last successful start. Resets on
    /// successful ready signal.
    restart_count: u32,
    /// OS process ID if the process is currently alive.
    pid: Option<u32>,
    /// Last error message captured from the process or from spawn failure.
    /// Safe for cross-process propagation — no debug_context here.
    last_error: Option<String>,
    /// Handle to the background task that watches the child process for exit.
    /// Stored so it is never silently dropped — dropped handles cancel the task.
    watcher_handle: Option<JoinHandle<()>>,
    /// Shared child process handle for cooperative shutdown.
    ///
    /// The watcher task takes the child from this slot when it starts waiting.
    /// `shutdown_all()` also locks this slot to claim the child for
    /// kill + wait. Whichever claims it first owns the wait. If the watcher
    /// already claimed it, `shutdown_all()` falls back to awaiting the
    /// aborted watcher JoinHandle (which drops the child, firing kill_on_drop).
    child: Arc<TokioMutex<Option<tokio::process::Child>>>,
}

impl SupervisedProcess {
    fn new(config: SupervisedSubsystemConfig) -> Self {
        Self {
            config,
            state: SubsystemState::Stopped,
            restart_count: 0,
            pid: None,
            last_error: None,
            watcher_handle: None,
            child: Arc::new(TokioMutex::new(None)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Supervisor
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of a subsystem's current state, suitable for gRPC responses.
pub struct SubsystemSnapshot {
    pub subsystem_id: String,
    pub state: SubsystemState,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub pid: Option<u32>,
}

/// Process supervisor that owns the lifecycle of all child subsystems.
///
/// Cloneable via inner `Arc` — the gRPC service layer and boot orchestrator
/// both hold handles to the same supervisor instance.
#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<SupervisorInner>,
}

struct SupervisorInner {
    /// Global supervisor configuration — retry limits, backoff durations.
    config: SupervisorConfig,
    /// Per-subsystem runtime state, keyed by subsystem_id.
    processes: RwLock<HashMap<String, SupervisedProcess>>,
    /// Event bus handle for publishing supervision lifecycle events.
    event_bus: EventBus,
}

impl Supervisor {
    /// Create a new supervisor from config.
    ///
    /// Registers all subsystems defined in `config.subsystems` but does not
    /// spawn any processes. Spawning happens during the boot sequence via
    /// `spawn_subsystem()`.
    pub fn new(config: SupervisorConfig, event_bus: EventBus) -> Self {
        let mut processes = HashMap::new();

        for (subsystem_name, subsystem_config) in &config.subsystems {
            let subsystem_id = subsystem_config.subsystem_id.clone();
            tracing::info!(
                subsystem = %subsystem_id,
                event_type = "supervisor_register",
                command = %subsystem_config.command,
                "registered subsystem for supervision"
            );
            processes.insert(
                subsystem_name.clone(),
                SupervisedProcess::new(subsystem_config.clone()),
            );

            // Avoid unused variable warning — subsystem_id consumed above.
            let _ = subsystem_id;
        }

        Self {
            inner: Arc::new(SupervisorInner {
                config,
                processes: RwLock::new(processes),
                event_bus,
            }),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Spawn
    // ─────────────────────────────────────────────────────────────────────────

    /// Spawn a subsystem process by its config key name.
    ///
    /// Sets state to `Starting`, launches the OS process, records its PID,
    /// and starts a background watcher task that detects when the process exits.
    ///
    /// The write lock on `processes` is acquired and released in narrow scopes
    /// so that the future returned by this function (and any future that awaits
    /// it inside a `tokio::spawn`) remains `Send`.  Holding a `RwLockWriteGuard`
    /// across the `tokio::spawn` of the watcher would make the outer future
    /// non-`Send` because the guard is tied to the future's state machine.
    ///
    /// Returns a `Pin<Box<...>>` future to break the recursive async type:
    /// `spawn_subsystem` → `tokio::spawn(watch_process)` →
    /// `attempt_restart_with_policy` → `spawn_subsystem`.
    /// Without `Box::pin`, the compiler would try to compute an infinite
    /// `impl Future` type and fail.
    pub fn spawn_subsystem(
        &self,
        subsystem_name: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = SenaResult<u32>> + Send + '_>> {
        let subsystem_name_owned = subsystem_name.to_string();
        Box::pin(async move { self.spawn_subsystem_inner(&subsystem_name_owned).await })
    }

    /// Inner implementation of `spawn_subsystem` — the actual async logic.
    /// Separated so that `spawn_subsystem` can wrap it in `Box::pin` to
    /// break the recursive async type without cluttering the logic.
    async fn spawn_subsystem_inner(&self, subsystem_name: &str) -> SenaResult<u32> {
        // ── Phase 1: extract spawn info under lock, mark Starting ───────
        let (command, args, working_directory, subsystem_id, child_slot) = {
            let mut processes = self.inner.processes.write().await;
            let process_entry = processes.get_mut(subsystem_name).ok_or_else(|| {
                SenaError::new(
                    ErrorCode::SupervisionSpawnFailed,
                    format!("unknown subsystem: {subsystem_name}"),
                )
            })?;

            process_entry.state = SubsystemState::Starting;

            (
                process_entry.config.command.clone(),
                process_entry.config.args.clone(),
                process_entry.config.working_directory.clone(),
                process_entry.config.subsystem_id.clone(),
                // Clone the Arc so the watcher and shutdown_all() can both
                // access the child handle without holding the processes lock.
                Arc::clone(&process_entry.child),
            )
        };
        // Write lock released here — no guard held during OS spawn.

        // ── Phase 2: spawn the OS process (no lock held) ────────────────
        let child_result = Command::new(&command)
            .args(&args)
            .current_dir(&working_directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // kill_on_drop(true): when the Child is dropped (e.g. because the
            // watcher task is aborted during shutdown), tokio sends
            // TerminateProcess on Windows so the child cannot outlive daemon-bus.
            .kill_on_drop(true)
            .spawn();

        match child_result {
            Ok(child) => {
                let pid = child.id().unwrap_or(0);

                // ── Phase 3: update state under lock ────────────────────
                {
                    let mut processes = self.inner.processes.write().await;
                    if let Some(process_entry) = processes.get_mut(subsystem_name) {
                        process_entry.pid = Some(pid);
                        process_entry.state = SubsystemState::Running;
                        process_entry.last_error = None;
                    }
                }
                // Write lock released here.

                // ── Phase 3.5: store child in shared slot ───────────────
                //
                // Placed after releasing the processes write lock so we never
                // hold two locks simultaneously. The watcher and shutdown_all()
                // both access the child through this Arc<Mutex<Option<Child>>>.
                {
                    let mut child_guard = child_slot.lock().await;
                    *child_guard = Some(child);
                }

                tracing::info!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_spawned",
                    pid = pid,
                    command = %command,
                    "subsystem process spawned"
                );

                let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::signal(
                    EventTopic::TopicSubsystemStarted,
                    &subsystem_id,
                    "",
                ));

                // ── Phase 4: spawn watcher (no lock held) ───────────────
                let supervisor_clone = self.clone();
                let subsystem_name_owned = subsystem_name.to_string();
                let child_slot_for_watcher = Arc::clone(&child_slot);
                let watcher_handle = tokio::spawn(async move {
                    Self::watch_process(supervisor_clone, subsystem_name_owned, child_slot_for_watcher).await;
                });

                // ── Phase 5: store handle under lock ────────────────────
                {
                    let mut processes = self.inner.processes.write().await;
                    if let Some(process_entry) = processes.get_mut(subsystem_name) {
                        process_entry.watcher_handle = Some(watcher_handle);
                    }
                }

                Ok(pid)
            }
            Err(spawn_error) => {
                let error_message = format!(
                    "failed to spawn subsystem '{}': {}",
                    subsystem_name, spawn_error
                );

                {
                    let mut processes = self.inner.processes.write().await;
                    if let Some(process_entry) = processes.get_mut(subsystem_name) {
                        process_entry.state = SubsystemState::Failed;
                        process_entry.last_error = Some(error_message.clone());
                    }
                }

                tracing::error!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_spawn_failed",
                    error = %spawn_error,
                    command = %command,
                    "failed to spawn subsystem process"
                );

                Err(
                    SenaError::new(ErrorCode::SupervisionSpawnFailed, error_message)
                        .with_debug_context(format!("spawn IO error: {spawn_error}")),
                )
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Watch
    // ─────────────────────────────────────────────────────────────────────────

    /// Background task that waits for a child process to exit, then triggers
    /// the restart policy.
    ///
    /// This function is spawned as a tokio task — its JoinHandle is stored
    /// in `SupervisedProcess.watcher_handle` so it is never silently dropped.
    ///
    /// The child is passed via a shared `Arc<Mutex<Option<Child>>>` rather than
    /// by value so that `shutdown_all()` can also claim the child for a
    /// kill + wait (with timeout) when daemon-bus is shutting down. Whichever
    /// side locks and takes the child first owns the wait; the other side
    /// detects `None` and falls back to its own cleanup path.
    async fn watch_process(
        supervisor: Supervisor,
        subsystem_name: String,
        child_slot: Arc<TokioMutex<Option<tokio::process::Child>>>,
    ) {
        // Take the child from the shared slot. If shutdown_all() already
        // claimed it (rare race on immediate shutdown), exit silently —
        // shutdown_all() is handling the kill + wait for this child.
        let mut child = {
            let mut guard = child_slot.lock().await;
            match guard.take() {
                Some(c) => c,
                None => {
                    tracing::debug!(
                        subsystem = "daemon_bus",
                        event_type = "watch_process_child_gone",
                        subsystem_name = %subsystem_name,
                        "child slot empty at watcher start — shutdown claimed child"
                    );
                    return;
                }
            }
        };
        // Mutex guard released here — child is now locally owned.

        // Wait for the process to exit. This is non-blocking — tokio's
        // process driver uses OS-specific async wait mechanisms.
        let exit_status = child.wait().await;

        let subsystem_id = {
            let processes = supervisor.inner.processes.read().await;
            processes
                .get(&subsystem_name)
                .map(|process| process.config.subsystem_id.clone())
                .unwrap_or_else(|| subsystem_name.clone())
        };

        match exit_status {
            Ok(status) if status.success() => {
                // Clean exit — subsystem shut down intentionally.
                tracing::info!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_exited_clean",
                    exit_code = status.code().unwrap_or(-1),
                    "subsystem exited cleanly"
                );

                let mut processes = supervisor.inner.processes.write().await;
                if let Some(process_entry) = processes.get_mut(&subsystem_name) {
                    process_entry.state = SubsystemState::Stopped;
                    process_entry.pid = None;
                    process_entry.watcher_handle = None;
                }
                // Clean exit does not trigger restart.
            }
            Ok(status) => {
                // Non-zero exit — subsystem crashed.
                let exit_code = status.code().unwrap_or(-1);

                tracing::error!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_crashed",
                    exit_code = exit_code,
                    "subsystem crashed with non-zero exit code"
                );

                {
                    let mut processes = supervisor.inner.processes.write().await;
                    if let Some(process_entry) = processes.get_mut(&subsystem_name) {
                        process_entry.last_error =
                            Some(format!("process exited with code {exit_code}"));
                        process_entry.pid = None;
                        process_entry.watcher_handle = None;
                    }
                }

                // Publish crash event before attempting restart.
                let _receiver_count = supervisor.inner.event_bus.publish(InternalBusEvent::signal(
                    EventTopic::TopicSubsystemCrashed,
                    &subsystem_id,
                    "",
                ));

                // Attempt restart with the defined retry policy.
                supervisor
                    .attempt_restart_with_policy(&subsystem_name)
                    .await;
            }
            Err(wait_error) => {
                // OS-level failure waiting for the process — treat as a crash.
                tracing::error!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_wait_failed",
                    error = %wait_error,
                    "failed to wait on subsystem process"
                );

                {
                    let mut processes = supervisor.inner.processes.write().await;
                    if let Some(process_entry) = processes.get_mut(&subsystem_name) {
                        process_entry.last_error = Some(format!("wait error: {wait_error}"));
                        process_entry.pid = None;
                        process_entry.watcher_handle = None;
                    }
                }

                let _receiver_count = supervisor.inner.event_bus.publish(InternalBusEvent::signal(
                    EventTopic::TopicSubsystemCrashed,
                    &subsystem_id,
                    "",
                ));

                supervisor
                    .attempt_restart_with_policy(&subsystem_name)
                    .await;
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Restart with retry policy
    // ─────────────────────────────────────────────────────────────────────────

    /// Attempts to restart a subsystem using the exact retry policy from config.
    ///
    /// Policy (PRD §10.2): immediate → 5s backoff → 30s backoff → degraded mode.
    /// The backoff durations come from `config.backoff_ms` — never hardcoded.
    /// Max retries comes from `config.max_retries`.
    ///
    /// Before each restart attempt, the last known state is logged (per
    /// daemon-bus.instructions.md: "never restart blindly without capturing
    /// the crash context first").
    async fn attempt_restart_with_policy(&self, subsystem_name: &str) {
        let max_retries = self.inner.config.max_retries;
        let backoff_durations: Vec<Duration> = self
            .inner
            .config
            .backoff_ms
            .iter()
            .map(|ms| Duration::from_millis(*ms))
            .collect();

        let subsystem_id = {
            let processes = self.inner.processes.read().await;
            processes
                .get(subsystem_name)
                .map(|process| process.config.subsystem_id.clone())
                .unwrap_or_else(|| subsystem_name.to_string())
        };

        // Read the cumulative restart count that persisted from any previous
        // crash–restart cycles. The `restart_count` field is never reset on
        // spawn — it is only reset when the subsystem reaches the Ready state
        // (i.e. its boot signal is received). This prevents an infinite loop
        // of: crash → attempt_restart_with_policy(from=0) → spawn → crash → …
        let prior_restart_count = {
            let processes = self.inner.processes.read().await;
            processes
                .get(subsystem_name)
                .map(|process| process.restart_count)
                .unwrap_or(0)
        };

        // If we have already exhausted all retries in a previous call, go
        // directly to degraded mode without attempting another restart.
        if prior_restart_count >= max_retries {
            tracing::warn!(
                subsystem = %subsystem_id,
                event_type = "restart_retries_already_exhausted",
                restart_count = prior_restart_count,
                max_retries = max_retries,
                "restart count already at max — entering degraded mode immediately"
            );
            self.enter_degraded_mode(subsystem_name, &subsystem_id)
                .await;
            return;
        }

        // The remaining attempts are those not yet consumed from prior calls.
        let remaining_start = prior_restart_count;

        for attempt in remaining_start..max_retries {
            // Log last known state before each restart attempt (never restart blindly).
            {
                let processes = self.inner.processes.read().await;
                if let Some(process_entry) = processes.get(subsystem_name) {
                    tracing::info!(
                        subsystem = %subsystem_id,
                        event_type = "restart_attempt",
                        attempt = attempt + 1,
                        max_retries = max_retries,
                        last_state = ?process_entry.state,
                        last_error = process_entry.last_error.as_deref().unwrap_or("none"),
                        last_pid = process_entry.pid.unwrap_or(0),
                        "attempting subsystem restart — last known state captured"
                    );
                }
            }

            // Apply backoff delay. Index safety: backoff_durations length is validated
            // in config to equal max_retries.
            let backoff = backoff_durations
                .get(attempt as usize)
                .copied()
                .unwrap_or(Duration::from_secs(30));

            if !backoff.is_zero() {
                tracing::info!(
                    subsystem = %subsystem_id,
                    event_type = "restart_backoff",
                    attempt = attempt + 1,
                    backoff_ms = backoff.as_millis() as u64,
                    "waiting before restart attempt"
                );
                tokio::time::sleep(backoff).await;
            }

            // Update state to Restarting.
            {
                let mut processes = self.inner.processes.write().await;
                if let Some(process_entry) = processes.get_mut(subsystem_name) {
                    process_entry.state = SubsystemState::Restarting;
                    process_entry.restart_count = attempt + 1;
                }
            }

            // Attempt to spawn.
            match self.spawn_subsystem(subsystem_name).await {
                Ok(new_pid) => {
                    tracing::info!(
                        subsystem = %subsystem_id,
                        event_type = "restart_succeeded",
                        attempt = attempt + 1,
                        new_pid = new_pid,
                        "subsystem restarted successfully"
                    );

                    let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::signal(
                        EventTopic::TopicSubsystemRestarted,
                        &subsystem_id,
                        "",
                    ));

                    return;
                }
                Err(spawn_error) => {
                    tracing::error!(
                        subsystem = %subsystem_id,
                        event_type = "restart_failed",
                        attempt = attempt + 1,
                        error = %spawn_error,
                        "restart attempt failed"
                    );
                    // Continue to next retry (or fall through to degraded mode).
                }
            }
        }

        // All retries exhausted — enter degraded mode.
        self.enter_degraded_mode(subsystem_name, &subsystem_id)
            .await;
    }

    /// Transitions a subsystem to degraded mode after all retry attempts are exhausted.
    ///
    /// In degraded mode, Sena continues operating with reduced capability.
    /// The user is explicitly informed about what capability has been lost
    /// (per PRD §10.2 and §10.3). daemon-bus publishes the degraded event
    /// so the reactive loop can surface the information.
    async fn enter_degraded_mode(&self, subsystem_name: &str, subsystem_id: &str) {
        tracing::error!(
            subsystem = %subsystem_id,
            event_type = "subsystem_degraded",
            max_retries = self.inner.config.max_retries,
            "all restart attempts exhausted — entering degraded mode"
        );

        {
            let mut processes = self.inner.processes.write().await;
            if let Some(process_entry) = processes.get_mut(subsystem_name) {
                process_entry.state = SubsystemState::Degraded;
                process_entry.last_error = Some(format!(
                    "all {} restart attempts exhausted — subsystem is in degraded mode",
                    self.inner.config.max_retries
                ));
            }
        }

        let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::signal(
            EventTopic::TopicSubsystemDegraded,
            subsystem_id,
            "",
        ));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Manual restart (for gRPC service layer)
    // ─────────────────────────────────────────────────────────────────────────

    /// Manually request a subsystem restart. Used by the gRPC SupervisorService.
    ///
    /// Terminates the current process (if running) and triggers the retry policy
    /// from attempt 0. The caller receives confirmation that the request was
    /// accepted — the actual restart is asynchronous.
    pub async fn request_restart(&self, subsystem_name: &str, reason: &str) -> SenaResult<()> {
        let subsystem_id = {
            let processes = self.inner.processes.read().await;
            let process_entry = processes.get(subsystem_name).ok_or_else(|| {
                SenaError::new(
                    ErrorCode::SupervisionSpawnFailed,
                    format!("unknown subsystem: {subsystem_name}"),
                )
            })?;
            process_entry.config.subsystem_id.clone()
        };

        tracing::info!(
            subsystem = %subsystem_id,
            event_type = "manual_restart_requested",
            reason = reason,
            "manual subsystem restart requested"
        );

        // Reset restart count for a manual restart — this is a deliberate
        // user/system action, not a crash recovery continuation.
        {
            let mut processes = self.inner.processes.write().await;
            if let Some(process_entry) = processes.get_mut(subsystem_name) {
                // Abort the existing watcher task if one is running.
                if let Some(handle) = process_entry.watcher_handle.take() {
                    handle.abort();
                }
                process_entry.restart_count = 0;
                process_entry.state = SubsystemState::Restarting;

                // Terminate the running process if it has a known PID.
                // On Windows, tokio's Child.kill() calls TerminateProcess.
                // We don't have the Child handle here — the watcher owns it.
                // Aborting the watcher task above will trigger kill_on_drop
                // if the Child was configured for it. For the scaffold, this
                // is a known limitation: full implementation will pipe a
                // shutdown signal to the child via gRPC before hard-killing.
                process_entry.pid = None;
            }
        }

        // Spawn the restart policy on a background task so the gRPC call
        // returns immediately. Store the handle at the process level.
        let supervisor_clone = self.clone();
        let subsystem_name_owned = subsystem_name.to_string();
        let restart_handle = tokio::spawn(async move {
            supervisor_clone
                .attempt_restart_with_policy(&subsystem_name_owned)
                .await;
        });

        // Store the restart task handle so it is not silently dropped.
        {
            let mut processes = self.inner.processes.write().await;
            if let Some(process_entry) = processes.get_mut(subsystem_name) {
                process_entry.watcher_handle = Some(restart_handle);
            }
        }

        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Status queries
    // ─────────────────────────────────────────────────────────────────────────

    /// Returns a snapshot of all supervised subsystem states.
    pub async fn get_all_statuses(&self) -> Vec<SubsystemSnapshot> {
        let processes = self.inner.processes.read().await;
        processes
            .values()
            .map(|process| SubsystemSnapshot {
                subsystem_id: process.config.subsystem_id.clone(),
                state: process.state,
                restart_count: process.restart_count,
                last_error: process.last_error.clone(),
                pid: process.pid,
            })
            .collect()
    }

    /// Returns the state of a single subsystem, or None if unknown.
    pub async fn get_subsystem_status(&self, subsystem_name: &str) -> Option<SubsystemSnapshot> {
        let processes = self.inner.processes.read().await;
        processes
            .get(subsystem_name)
            .map(|process| SubsystemSnapshot {
                subsystem_id: process.config.subsystem_id.clone(),
                state: process.state,
                restart_count: process.restart_count,
                last_error: process.last_error.clone(),
                pid: process.pid,
            })
    }

    /// Returns the list of subsystem config keys that should be spawned at boot.
    pub fn boot_spawn_list(&self) -> Vec<String> {
        self.inner
            .config
            .subsystems
            .iter()
            .filter(|(_name, subsystem_config)| subsystem_config.spawn_at_boot)
            .map(|(name, _subsystem_config)| name.clone())
            .collect()
    }

    /// Marks a subsystem as ready. Called when the boot orchestrator receives
    /// the subsystem's ready signal. Resets the restart counter.
    pub async fn mark_ready(&self, subsystem_name: &str) {
        let mut processes = self.inner.processes.write().await;
        if let Some(process_entry) = processes.get_mut(subsystem_name) {
            process_entry.state = SubsystemState::Ready;
            process_entry.restart_count = 0;

            tracing::info!(
                subsystem = %process_entry.config.subsystem_id,
                event_type = "subsystem_ready",
                pid = process_entry.pid.unwrap_or(0),
                "subsystem marked ready — restart counter reset"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Shutdown
    // ─────────────────────────────────────────────────────────────────────────

    /// Gracefully shut down all supervised subsystems.
    ///
    /// For each running subsystem:
    /// 1. Aborts the watcher task (prevents crash-triggered restarts during shutdown).
    /// 2. Claims the child process handle from the shared slot.
    /// 3. Sends a kill signal via `Child::kill()`.
    /// 4. Awaits `Child::wait()` with a 2000 ms timeout so file locks (e.g.
    ///    the redb database lock) are confirmed released before this method
    ///    returns and a new instance can start.
    ///
    /// If the watcher already claimed the child (it was in `child.wait()` when
    /// abort fired), the child is dropped by the task cancellation which fires
    /// `kill_on_drop` (TerminateProcess on Windows). In that case we await the
    /// aborted JoinHandle instead — the handle resolves only after the task
    /// future has been fully dropped, confirming the kill has been issued.
    pub async fn shutdown_all(&self) {
        tracing::info!(
            subsystem = "daemon_bus",
            event_type = "supervisor_shutdown",
            "shutting down all supervised subsystems"
        );

        // ── Phase 1: collect handles + child slots, mark all stopped ─────
        //
        // Gather everything we need before releasing the lock.
        // Handles are NOT aborted yet — we abort in Phase 2 so we can still
        // collect them here without consuming.
        type ShutdownTarget = (
            String,
            Option<JoinHandle<()>>,
            Arc<TokioMutex<Option<tokio::process::Child>>>,
        );
        let shutdown_targets: Vec<ShutdownTarget>;
        {
            let mut processes = self.inner.processes.write().await;
            shutdown_targets = processes
                .iter_mut()
                .map(|(subsystem_name, process_entry)| {
                    let handle = process_entry.watcher_handle.take();
                    let pid = process_entry.pid.take();
                    process_entry.state = SubsystemState::Stopped;

                    tracing::info!(
                        subsystem = %process_entry.config.subsystem_id,
                        event_type = "subsystem_stopped",
                        subsystem_name = %subsystem_name,
                        pid = pid.unwrap_or(0),
                        "subsystem stopped during supervisor shutdown"
                    );

                    (
                        process_entry.config.subsystem_id.clone(),
                        handle,
                        Arc::clone(&process_entry.child),
                    )
                })
                .collect();
        }
        // Write lock released here.

        // ── Phase 2: abort all watcher tasks ─────────────────────────────
        //
        // `JoinHandle::abort` takes `&self` so handles are not consumed here.
        // The abort signal is sent but task cancellation is asynchronous —
        // we confirm completion in Phase 3.
        for (_, handle_opt, _) in &shutdown_targets {
            if let Some(handle) = handle_opt {
                handle.abort();
            }
        }

        // ── Phase 3: kill + wait for each child ──────────────────────────
        //
        // Two paths depending on who owns the child handle:
        //
        // A) We claim the child from the shared slot (watcher hadn't started
        //    or hadn't taken it yet): call kill() + wait(timeout) directly.
        //    This is the common path for a daemon-bus restart scenario.
        //
        // B) The watcher already claimed the child (it was suspended in
        //    child.wait()): the slot is empty. The abort fired above will
        //    cancel the watcher at its next await point, dropping the Child
        //    which fires kill_on_drop (TerminateProcess on Windows). We await
        //    the aborted JoinHandle with the same timeout to confirm the task
        //    has been fully cancelled and the kill has been issued.
        for (subsystem_id, handle_opt, child_slot) in shutdown_targets {
            let child_opt = {
                let mut guard = child_slot.lock().await;
                guard.take()
            };

            if let Some(mut child) = child_opt {
                // Path A: we own the child — kill and wait with timeout.
                tracing::info!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_kill_and_wait",
                    "killing child process and awaiting exit to release file locks"
                );

                let kill_result = child.kill().await;
                if let Err(kill_error) = kill_result {
                    tracing::warn!(
                        subsystem = %subsystem_id,
                        error = %kill_error,
                        "child.kill() failed — process may already be dead"
                    );
                }

                match tokio::time::timeout(
                    Duration::from_millis(2000),
                    child.wait(),
                )
                .await
                {
                    Ok(Ok(exit_status)) => {
                        tracing::info!(
                            subsystem = %subsystem_id,
                            event_type = "subsystem_exit_confirmed",
                            exit_code = exit_status.code().unwrap_or(-1),
                            "child process exited — file locks released"
                        );
                    }
                    Ok(Err(wait_error)) => {
                        tracing::warn!(
                            subsystem = %subsystem_id,
                            error = %wait_error,
                            "child.wait() failed after kill"
                        );
                    }
                    Err(_elapsed) => {
                        tracing::warn!(
                            subsystem = %subsystem_id,
                            event_type = "subsystem_exit_timeout",
                            timeout_ms = 2000u64,
                            "timed out waiting for child process to exit"
                        );
                    }
                }
            } else {
                // Path B: watcher already claimed the child. The abort in
                // Phase 2 will cancel the watcher at child.wait(), dropping
                // the Child and firing kill_on_drop. Await the handle (with
                // a timeout) to confirm the task has been fully cancelled.
                tracing::debug!(
                    subsystem = %subsystem_id,
                    event_type = "subsystem_child_claimed_by_watcher",
                    "child slot empty — awaiting aborted watcher for kill_on_drop confirmation"
                );

                if let Some(handle) = handle_opt {
                    match tokio::time::timeout(
                        Duration::from_millis(2000),
                        handle,
                    )
                    .await
                    {
                        Ok(_) => {
                            tracing::debug!(
                                subsystem = %subsystem_id,
                                event_type = "subsystem_watcher_cancelled",
                                "aborted watcher task resolved — kill_on_drop confirmed"
                            );
                        }
                        Err(_elapsed) => {
                            tracing::warn!(
                                subsystem = %subsystem_id,
                                event_type = "subsystem_watcher_cancel_timeout",
                                timeout_ms = 2000u64,
                                "timed out waiting for aborted watcher task to resolve"
                            );
                        }
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::SupervisorConfig;

    /// Verify that when all retries are exhausted, the supervisor publishes
    /// a DEGRADED_MODE event to the event bus.
    #[tokio::test]
    async fn degraded_mode_broadcast_after_retry_exhaustion() {
        let bus = EventBus::new(16);
        let mut bus_rx = bus
            .subscribe("test_subscriber", &[EventTopic::TopicSubsystemDegraded])
            .await;

        // Create a config with zero retries so we go directly to degraded mode.
        let config = SupervisorConfig {
            max_retries: 0,
            backoff_ms: vec![],
            process_start_grace_ms: 2000,
            subsystems: std::collections::HashMap::new(),
        };

        let supervisor = Supervisor::new(config, bus.clone());

        // Manually invoke enter_degraded_mode.
        supervisor
            .enter_degraded_mode("test_subsystem", "test_subsystem")
            .await;

        // Check that a DEGRADED event was published.
        let event = tokio::time::timeout(Duration::from_millis(100), bus_rx.recv())
            .await
            .expect("should receive event within timeout")
            .expect("bus should not be closed");

        assert_eq!(
            event.topic,
            EventTopic::TopicSubsystemDegraded,
            "event topic should be DEGRADED"
        );
        assert_eq!(
            event.source_subsystem, "test_subsystem",
            "source should be the failed subsystem"
        );
    }
}
