//! Watchdog — wall-clock task timeout enforcement.
//!
//! daemon-bus runs a watchdog timer on every dispatched agent task (PRD §12.2).
//! If a task exceeds its wall-clock quota without completing, the watchdog
//! publishes a timeout event so the supervisor can terminate the agent process
//! and the reactive loop can inform the user that the task failed.
//!
//! The watchdog never performs blocking I/O — all timers are tokio-based.
//! All tunable values (default timeout, max timeout, sweep interval, capacity)
//! come from `config/daemon-bus.toml` `[watchdog]` — nothing is hardcoded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::bus::{EventBus, InternalBusEvent};
use crate::config::WatchdogConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::EventTopic;

// ─────────────────────────────────────────────────────────────────────────────
// Tracked task
// ─────────────────────────────────────────────────────────────────────────────

/// A single task being tracked by the watchdog.
#[derive(Debug, Clone)]
struct TrackedTask {
    /// Unique task identifier — provided by the caller at registration time.
    task_id: String,
    /// Which subsystem/agent owns this task.
    subsystem_id: String,
    /// Wall-clock deadline. If `Instant::now()` exceeds this, the task has timed out.
    deadline: Instant,
    /// The timeout duration that was granted (for logging).
    timeout: Duration,
    /// OpenTelemetry trace context for the operation this task belongs to.
    trace_context: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Watchdog
// ─────────────────────────────────────────────────────────────────────────────

/// Wall-clock task timeout enforcer.
///
/// Subsystems register tasks with a timeout via gRPC. The watchdog runs a
/// periodic sweep that checks all tracked tasks against their deadlines.
/// When a task exceeds its deadline, the watchdog publishes `TOPIC_TASK_TIMEOUT`
/// to the event bus and removes the task from tracking.
///
/// The supervisor is responsible for actually terminating the offending process —
/// the watchdog only detects and signals. This separation keeps the watchdog
/// simple and testable.
///
/// Cloneable via inner `Arc` — the gRPC service layer and the sweep task both
/// hold handles to the same watchdog instance.
#[derive(Clone)]
pub struct Watchdog {
    inner: Arc<WatchdogInner>,
}

struct WatchdogInner {
    /// Configuration from `[watchdog]` in daemon-bus.toml.
    config: WatchdogConfig,
    /// Currently tracked tasks, keyed by task_id.
    tasks: RwLock<HashMap<String, TrackedTask>>,
    /// Task IDs that were recently timed out — available for diagnostics queries.
    /// Bounded to avoid unbounded growth; older entries are evicted when the
    /// list exceeds a reasonable size (2x max_tracked_tasks as a ceiling).
    recently_timed_out: RwLock<Vec<String>>,
    /// Event bus handle for publishing timeout events.
    event_bus: EventBus,
}

/// The handle returned by `Watchdog::start_sweep_loop`. Stored by the caller
/// so the sweep task is never silently dropped (dropped JoinHandles cancel work).
pub struct WatchdogSweepHandle {
    _sweep_handle: JoinHandle<()>,
}

impl Watchdog {
    /// Create a new watchdog from config.
    ///
    /// Does not start the sweep loop — call `start_sweep_loop()` after construction
    /// to begin periodic timeout enforcement.
    pub fn new(config: WatchdogConfig, event_bus: EventBus) -> Self {
        tracing::info!(
            subsystem = "daemon_bus",
            event_type = "watchdog_init",
            default_timeout_ms = config.default_task_timeout_ms,
            max_timeout_ms = config.max_task_timeout_ms,
            sweep_interval_ms = config.sweep_interval_ms,
            max_tracked_tasks = config.max_tracked_tasks,
            "watchdog initialized"
        );

        Self {
            inner: Arc::new(WatchdogInner {
                config,
                tasks: RwLock::new(HashMap::new()),
                recently_timed_out: RwLock::new(Vec::new()),
                event_bus,
            }),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Task registration
    // ─────────────────────────────────────────────────────────────────────────

    /// Register a task with the watchdog. Starts the timeout timer.
    ///
    /// The granted timeout is clamped to `config.max_task_timeout_ms`. If the
    /// caller provides zero or no timeout, `config.default_task_timeout_ms` is used.
    ///
    /// Returns an error if the watchdog is at capacity (`config.max_tracked_tasks`).
    pub async fn register_task(
        &self,
        task_id: String,
        subsystem_id: String,
        requested_timeout_ms: u64,
        trace_context: String,
    ) -> SenaResult<()> {
        let mut tasks = self.inner.tasks.write().await;

        // Capacity check — never exceed the configured maximum.
        if tasks.len() >= self.inner.config.max_tracked_tasks {
            tracing::warn!(
                subsystem = "daemon_bus",
                event_type = "watchdog_capacity_exceeded",
                task_id = %task_id,
                requesting_subsystem = %subsystem_id,
                current_count = tasks.len(),
                max_tracked = self.inner.config.max_tracked_tasks,
                "watchdog at capacity — rejecting task registration"
            );
            return Err(SenaError::new(
                ErrorCode::WatchdogCapacityExceeded,
                format!(
                    "watchdog tracking capacity reached ({}/{})",
                    tasks.len(),
                    self.inner.config.max_tracked_tasks
                ),
            ));
        }

        // Determine the effective timeout: use default if caller sends 0,
        // clamp to the configured ceiling so no task can request an unbounded window.
        let effective_timeout_ms = if requested_timeout_ms == 0 {
            self.inner.config.default_task_timeout_ms
        } else {
            requested_timeout_ms.min(self.inner.config.max_task_timeout_ms)
        };

        let timeout = Duration::from_millis(effective_timeout_ms);
        let deadline = Instant::now() + timeout;

        let tracked = TrackedTask {
            task_id: task_id.clone(),
            subsystem_id: subsystem_id.clone(),
            deadline,
            timeout,
            trace_context,
        };

        // If a task with the same ID already exists, the new registration
        // replaces it. This handles the case where a subsystem re-registers
        // a task (e.g., after extending its timeout window via a new request).
        let was_existing = tasks.insert(task_id.clone(), tracked).is_some();

        if was_existing {
            tracing::debug!(
                subsystem = %subsystem_id,
                event_type = "watchdog_task_replaced",
                task_id = %task_id,
                timeout_ms = effective_timeout_ms,
                "existing tracked task replaced with new deadline"
            );
        } else {
            tracing::debug!(
                subsystem = %subsystem_id,
                event_type = "watchdog_task_registered",
                task_id = %task_id,
                timeout_ms = effective_timeout_ms,
                tracked_count = tasks.len(),
                "task registered with watchdog"
            );
        }

        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Task completion
    // ─────────────────────────────────────────────────────────────────────────

    /// Signal that a task has completed. Cancels the timeout timer.
    ///
    /// Returns true if the task was being tracked and was successfully removed.
    /// Returns false if the task was not found (already timed out or never registered).
    pub async fn complete_task(&self, task_id: &str) -> bool {
        let mut tasks = self.inner.tasks.write().await;
        let removed = tasks.remove(task_id);

        match &removed {
            Some(tracked) => {
                tracing::debug!(
                    subsystem = %tracked.subsystem_id,
                    event_type = "watchdog_task_completed",
                    task_id = %task_id,
                    tracked_count = tasks.len(),
                    "task completed — removed from watchdog tracking"
                );
                true
            }
            None => {
                tracing::debug!(
                    subsystem = "daemon_bus",
                    event_type = "watchdog_task_not_found",
                    task_id = %task_id,
                    "task completion signal for unknown/already-expired task"
                );
                false
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Sweep loop
    // ─────────────────────────────────────────────────────────────────────────

    /// Start the periodic sweep loop that checks for timed-out tasks.
    ///
    /// Returns a `WatchdogSweepHandle` that **must** be stored by the caller.
    /// Dropping the handle cancels the sweep loop — this is by design so the
    /// watchdog stops cleanly when daemon-bus shuts down.
    pub fn start_sweep_loop(&self) -> WatchdogSweepHandle {
        let watchdog = self.clone();
        let sweep_interval = Duration::from_millis(self.inner.config.sweep_interval_ms);

        let sweep_handle = tokio::spawn(async move {
            tracing::info!(
                subsystem = "daemon_bus",
                event_type = "watchdog_sweep_started",
                interval_ms = sweep_interval.as_millis() as u64,
                "watchdog sweep loop started"
            );

            let mut interval = tokio::time::interval(sweep_interval);
            // The first tick completes immediately — skip it so the first
            // real sweep happens after one full interval.
            interval.tick().await;

            loop {
                interval.tick().await;
                watchdog.sweep_expired_tasks().await;
            }
        });

        WatchdogSweepHandle {
            _sweep_handle: sweep_handle,
        }
    }

    /// Single sweep pass: find all tasks past their deadline, remove them,
    /// publish timeout events, and record them in the recently-timed-out list.
    async fn sweep_expired_tasks(&self) {
        let now = Instant::now();
        let mut expired_tasks: Vec<TrackedTask> = Vec::new();

        // Scope the write lock tightly — collect expired tasks and remove them,
        // then release the lock before doing any async bus publishing.
        {
            let mut tasks = self.inner.tasks.write().await;
            let task_ids_to_remove: Vec<String> = tasks
                .iter()
                .filter(|(_task_id, tracked)| now >= tracked.deadline)
                .map(|(task_id, _tracked)| task_id.clone())
                .collect();

            for task_id in &task_ids_to_remove {
                if let Some(tracked) = tasks.remove(task_id) {
                    expired_tasks.push(tracked);
                }
            }
        }
        // Write lock released here — no lock held during bus publishing.

        if expired_tasks.is_empty() {
            return;
        }

        // Record timed-out task IDs for diagnostics queries.
        {
            let mut recently_timed_out = self.inner.recently_timed_out.write().await;
            for expired in &expired_tasks {
                recently_timed_out.push(expired.task_id.clone());
            }
            // Bound the list to prevent unbounded growth. 2x max_tracked_tasks
            // is an arbitrary but reasonable ceiling for a diagnostics buffer.
            let max_recent = self.inner.config.max_tracked_tasks * 2;
            if recently_timed_out.len() > max_recent {
                let drain_count = recently_timed_out.len() - max_recent;
                recently_timed_out.drain(..drain_count);
            }
        }

        // Publish timeout events for each expired task.
        for expired in expired_tasks {
            let overtime_ms = now.duration_since(expired.deadline).as_millis() as u64;

            tracing::warn!(
                subsystem = %expired.subsystem_id,
                event_type = "task_timeout",
                task_id = %expired.task_id,
                timeout_ms = expired.timeout.as_millis() as u64,
                overtime_ms = overtime_ms,
                "task exceeded wall-clock timeout — signaling termination"
            );

            // Publish TOPIC_TASK_TIMEOUT so the supervisor can terminate the process
            // and the reactive loop can inform the user.
            let payload = build_timeout_payload(
                &expired.task_id,
                &expired.subsystem_id,
                expired.timeout.as_millis() as u64,
            );

            let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::new(
                EventTopic::TopicTaskTimeout,
                &expired.subsystem_id,
                payload,
                &expired.trace_context,
            ));

            // Also publish TOPIC_TASK_TERMINATED to signal that the watchdog
            // considers this task dead. The supervisor listens for this to
            // trigger the restart policy on the owning agent process.
            let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::new(
                EventTopic::TopicTaskTerminated,
                &expired.subsystem_id,
                build_terminated_payload(&expired.task_id, &expired.subsystem_id),
                &expired.trace_context,
            ));
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Status queries
    // ─────────────────────────────────────────────────────────────────────────

    /// Returns the number of currently tracked tasks.
    pub async fn active_task_count(&self) -> usize {
        let tasks = self.inner.tasks.read().await;
        tasks.len()
    }

    /// Returns task IDs that have recently timed out.
    /// The list is bounded and eventually evicts old entries.
    pub async fn recently_timed_out_task_ids(&self) -> Vec<String> {
        let recently_timed_out = self.inner.recently_timed_out.read().await;
        recently_timed_out.clone()
    }

    /// Clears the recently-timed-out list. Called after a diagnostics query
    /// has consumed the data.
    pub async fn clear_recently_timed_out(&self) {
        let mut recently_timed_out = self.inner.recently_timed_out.write().await;
        recently_timed_out.clear();
    }

    /// Returns true if a specific task is currently being tracked.
    pub async fn is_task_tracked(&self, task_id: &str) -> bool {
        let tasks = self.inner.tasks.read().await;
        tasks.contains_key(task_id)
    }

    /// Returns the remaining time (in milliseconds) before a tracked task
    /// times out. Returns None if the task is not tracked.
    pub async fn remaining_ms(&self, task_id: &str) -> Option<u64> {
        let tasks = self.inner.tasks.read().await;
        tasks.get(task_id).map(|tracked| {
            let now = Instant::now();
            if now >= tracked.deadline {
                0
            } else {
                (tracked.deadline - now).as_millis() as u64
            }
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload builders
// ─────────────────────────────────────────────────────────────────────────────
// These produce simple JSON payloads for bus events. The payloads are opaque
// bytes from the bus's perspective — receivers deserialize based on topic contract.
// Using JSON here because these are non-uniform internal structures (per
// serialization rules in copilot-instructions.md).

/// Build the payload bytes for a TOPIC_TASK_TIMEOUT event.
fn build_timeout_payload(task_id: &str, subsystem_id: &str, timeout_ms: u64) -> Vec<u8> {
    // Simple JSON — no external serde_json dependency needed for this scaffold.
    // In production, this would use serde_json::to_vec with a typed struct.
    format!(
        r#"{{"task_id":"{}","subsystem_id":"{}","timeout_ms":{}}}"#,
        task_id, subsystem_id, timeout_ms
    )
    .into_bytes()
}

/// Build the payload bytes for a TOPIC_TASK_TERMINATED event.
fn build_terminated_payload(task_id: &str, subsystem_id: &str) -> Vec<u8> {
    format!(
        r#"{{"task_id":"{}","subsystem_id":"{}","reason":"watchdog_timeout"}}"#,
        task_id, subsystem_id
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::WatchdogConfig;

    fn test_config() -> WatchdogConfig {
        WatchdogConfig {
            default_task_timeout_ms: 1000,
            max_task_timeout_ms: 5000,
            sweep_interval_ms: 50,
            max_tracked_tasks: 10,
        }
    }

    #[tokio::test]
    async fn register_and_complete_task() {
        let bus = EventBus::new(16);
        let watchdog = Watchdog::new(test_config(), bus);

        watchdog
            .register_task(
                "task-1".to_string(),
                "test_agent".to_string(),
                2000,
                "trace-abc".to_string(),
            )
            .await
            .expect("registration should succeed");

        assert!(watchdog.is_task_tracked("task-1").await);
        assert_eq!(watchdog.active_task_count().await, 1);

        let completed = watchdog.complete_task("task-1").await;
        assert!(completed);
        assert!(!watchdog.is_task_tracked("task-1").await);
        assert_eq!(watchdog.active_task_count().await, 0);
    }

    #[tokio::test]
    async fn complete_unknown_task_returns_false() {
        let bus = EventBus::new(16);
        let watchdog = Watchdog::new(test_config(), bus);

        let completed = watchdog.complete_task("nonexistent").await;
        assert!(!completed);
    }

    #[tokio::test]
    async fn capacity_limit_enforced() {
        let bus = EventBus::new(16);
        let config = WatchdogConfig {
            max_tracked_tasks: 2,
            ..test_config()
        };
        let watchdog = Watchdog::new(config, bus);

        watchdog
            .register_task("t1".to_string(), "a".to_string(), 1000, String::new())
            .await
            .expect("first registration should succeed");

        watchdog
            .register_task("t2".to_string(), "a".to_string(), 1000, String::new())
            .await
            .expect("second registration should succeed");

        let result = watchdog
            .register_task("t3".to_string(), "a".to_string(), 1000, String::new())
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::WatchdogCapacityExceeded);
    }

    #[tokio::test]
    async fn timeout_clamped_to_max() {
        let bus = EventBus::new(16);
        let watchdog = Watchdog::new(test_config(), bus);

        // Request a timeout exceeding the max — should be clamped.
        watchdog
            .register_task(
                "task-huge".to_string(),
                "agent".to_string(),
                999_999,
                String::new(),
            )
            .await
            .expect("registration should succeed");

        // The remaining time should be at most max_task_timeout_ms (5000ms)
        // plus a small margin for test execution time.
        let remaining = watchdog
            .remaining_ms("task-huge")
            .await
            .expect("task should be tracked");
        assert!(
            remaining <= 5100,
            "remaining {remaining}ms exceeds clamped max"
        );
    }

    #[tokio::test]
    async fn zero_timeout_uses_default() {
        let bus = EventBus::new(16);
        let watchdog = Watchdog::new(test_config(), bus);

        watchdog
            .register_task(
                "task-default".to_string(),
                "agent".to_string(),
                0,
                String::new(),
            )
            .await
            .expect("registration should succeed");

        let remaining = watchdog
            .remaining_ms("task-default")
            .await
            .expect("task should be tracked");
        // Default is 1000ms — allow margin for test execution.
        assert!(remaining <= 1100, "remaining {remaining}ms exceeds default");
        assert!(
            remaining > 0,
            "remaining should be positive immediately after registration"
        );
    }

    #[tokio::test]
    async fn sweep_detects_expired_tasks() {
        let bus = EventBus::new(64);
        let config = WatchdogConfig {
            default_task_timeout_ms: 50,
            max_task_timeout_ms: 100,
            sweep_interval_ms: 10,
            max_tracked_tasks: 10,
        };
        let watchdog = Watchdog::new(config, bus);

        watchdog
            .register_task(
                "expires-fast".to_string(),
                "agent".to_string(),
                50,
                String::new(),
            )
            .await
            .expect("registration should succeed");

        // Wait for the task to expire.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Run a manual sweep.
        watchdog.sweep_expired_tasks().await;

        // Task should no longer be tracked.
        assert!(!watchdog.is_task_tracked("expires-fast").await);

        // Task should appear in recently timed out.
        let timed_out = watchdog.recently_timed_out_task_ids().await;
        assert!(timed_out.contains(&"expires-fast".to_string()));
    }
}
