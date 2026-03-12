//! Priority tier arbitration.
//!
//! daemon-bus is the sole arbitrator of all priority decisions (PRD §9.1).
//! No subsystem self-promotes — every escalation request is submitted here
//! and either granted or queued.
//!
//! Key invariants enforced by this module:
//!
//! - **Tier 2 is exclusive**: only one subsystem can hold Tier 2 at a time.
//!   A second request is queued, never granted concurrently
//!   (PRD §9.3, daemon-bus.instructions.md).
//!
//! - **Every escalation has a bounded expiry**: a tokio timer is scheduled at
//!   grant time. On expiry the subsystem is de-escalated automatically — the
//!   watchdog never waits for voluntary release
//!   (PRD §9.3: "they expire automatically if not completed").
//!
//! - **Reactive always wins over CTP**: when both request Tier 2 simultaneously,
//!   the reactive loop is granted first. CTP is queued. This is not configurable
//!   (PRD §9.4).
//!
//! All tunable values (max duration, default duration, queue depth, reactive
//! subsystem ID) come from `config/daemon-bus.toml` `[arbitration]` — nothing
//! is hardcoded.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::bus::{EventBus, InternalBusEvent};
use crate::config::ArbitrationConfig;
use crate::error::SenaResult;
use crate::generated::sena_daemonbus_v1::EventTopic;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Result of an escalation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationOutcome {
    /// Tier 2 was granted immediately.
    Granted {
        escalation_id: String,
        granted_duration_ms: u64,
    },
    /// Tier 2 is currently held by another subsystem — request was queued.
    Queued { escalation_id: String },
    /// Request denied — the escalation queue is full.
    Denied { reason: String },
}

/// A pending escalation request sitting in the queue.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields read during queue priority sorting in grant_next_in_queue — full implementation pending.
struct QueuedEscalation {
    escalation_id: String,
    subsystem_id: String,
    reason: String,
    requested_duration_ms: u64,
    trace_context: String,
    /// True if this request comes from the reactive subsystem. Used for
    /// priority sorting — reactive always jumps ahead of CTP in the queue.
    is_reactive: bool,
}

/// The currently active Tier 2 escalation.
/// Retained for the full implementation where the JoinHandle is tracked
/// alongside the escalation metadata in a single struct.
#[derive(Debug)]
#[allow(dead_code)]
struct ActiveEscalation {
    escalation_id: String,
    subsystem_id: String,
    granted_duration: Duration,
    /// Handle to the tokio timer task that will auto-expire this escalation.
    /// Stored so it is never silently dropped — dropping the handle would
    /// cancel the expiry timer and leave the escalation permanently held.
    expiry_handle: JoinHandle<()>,
}

/// Mutable interior state protected by `RwLock`.
#[derive(Debug)]
struct ArbitrationState {
    /// The subsystem currently holding Tier 2, if any.
    /// `None` means Tier 2 is free.
    current_holder: Option<ActiveEscalationSnapshot>,
    /// Ordered queue of pending escalation requests.
    /// Reactive requests are inserted at the front; all others at the back.
    queue: VecDeque<QueuedEscalation>,
}

/// Snapshot of the active escalation — excludes the `JoinHandle` so it can
/// live behind `RwLock` without `Send` issues on the handle.
#[derive(Debug, Clone)]
#[allow(dead_code)] // granted_duration_ms read in diagnostics logging — full implementation pending.
struct ActiveEscalationSnapshot {
    escalation_id: String,
    subsystem_id: String,
    granted_duration_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Arbiter
// ─────────────────────────────────────────────────────────────────────────────

/// Priority tier arbiter.
///
/// Cloneable via inner `Arc` — the gRPC service layer and the boot orchestrator
/// both hold handles to the same arbiter instance.
#[derive(Clone)]
pub struct Arbiter {
    inner: Arc<ArbiterInner>,
}

struct ArbiterInner {
    config: ArbitrationConfig,
    state: RwLock<ArbitrationState>,
    /// Handle to the expiry task for the currently active escalation.
    /// Kept outside `ArbitrationState` because `JoinHandle` must be managed
    /// carefully — aborting it on release/expiry, storing a new one on grant.
    ///
    /// Uses `std::sync::Mutex` instead of `tokio::sync::RwLock` because:
    /// 1. The lock is held only for a pointer swap — never across an await.
    /// 2. Making `schedule_expiry_timer` and `cancel_expiry_timer` synchronous
    ///    (non-async) breaks the recursive async type chain that would otherwise
    ///    prevent the spawned expiry future from being `Send`.
    expiry_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    event_bus: EventBus,
}

impl Arbiter {
    /// Create a new arbiter from config. No escalation is active initially.
    pub fn new(config: ArbitrationConfig, event_bus: EventBus) -> Self {
        tracing::info!(
            subsystem = "daemon_bus",
            event_type = "arbiter_init",
            max_escalation_duration_ms = config.max_escalation_duration_ms,
            default_escalation_duration_ms = config.default_escalation_duration_ms,
            max_queue_depth = config.max_queue_depth,
            reactive_subsystem_id = %config.reactive_subsystem_id,
            "priority arbiter initialized"
        );

        Self {
            inner: Arc::new(ArbiterInner {
                config,
                state: RwLock::new(ArbitrationState {
                    current_holder: None,
                    queue: VecDeque::new(),
                }),
                expiry_handle: std::sync::Mutex::new(None),
                event_bus,
            }),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Request escalation
    // ─────────────────────────────────────────────────────────────────────────

    /// Request a Tier 2 escalation for a subsystem.
    ///
    /// If Tier 2 is free, the request is granted immediately and an expiry
    /// timer is started. If Tier 2 is held, the request is queued (reactive
    /// requests jump to the front of the queue). If the queue is full, the
    /// request is denied.
    pub async fn request_escalation(
        &self,
        subsystem_id: String,
        reason: String,
        requested_duration_ms: u64,
        trace_context: String,
    ) -> SenaResult<EscalationOutcome> {
        let escalation_id = uuid::Uuid::new_v4().to_string();
        let is_reactive = subsystem_id == self.inner.config.reactive_subsystem_id;

        let mut state = self.inner.state.write().await;

        // ── Tier 2 is free — grant immediately ──────────────────────────
        if state.current_holder.is_none() {
            let granted_duration_ms = self.clamp_duration(requested_duration_ms);
            let snapshot = ActiveEscalationSnapshot {
                escalation_id: escalation_id.clone(),
                subsystem_id: subsystem_id.clone(),
                granted_duration_ms,
            };
            state.current_holder = Some(snapshot);

            // Release the state lock before scheduling the expiry timer and
            // publishing the event — never hold a write lock across async work.
            drop(state);

            // schedule_expiry_timer is synchronous — safe to call without await.
            self.schedule_expiry_timer(&escalation_id, &subsystem_id, granted_duration_ms);

            self.publish_escalation_event(
                EventTopic::TopicEscalationGranted,
                &escalation_id,
                &subsystem_id,
                &trace_context,
            );

            tracing::info!(
                subsystem = %subsystem_id,
                event_type = "escalation_granted",
                escalation_id = %escalation_id,
                granted_duration_ms = granted_duration_ms,
                reason = %reason,
                "tier 2 escalation granted"
            );

            return Ok(EscalationOutcome::Granted {
                escalation_id,
                granted_duration_ms,
            });
        }

        // ── Tier 2 is held — check if reactive should preempt ───────────
        // PRD §9.4: if the reactive loop requests Tier 2 and CTP currently
        // holds it, reactive takes precedence. However, the spec says the
        // second request is *queued* (not that the current holder is evicted).
        // "First request is granted immediately. Second request is queued."
        // Reactive priority means it jumps to the front of the queue, not
        // that it evicts the current holder mid-escalation.

        // ── Queue the request ───────────────────────────────────────────
        if state.queue.len() >= self.inner.config.max_queue_depth {
            drop(state);

            tracing::warn!(
                subsystem = %subsystem_id,
                event_type = "escalation_denied",
                escalation_id = %escalation_id,
                reason = "queue full",
                queue_depth = self.inner.config.max_queue_depth,
                "escalation request denied — queue at capacity"
            );

            return Ok(EscalationOutcome::Denied {
                reason: format!(
                    "escalation queue full ({}/{})",
                    self.inner.config.max_queue_depth, self.inner.config.max_queue_depth
                ),
            });
        }

        let queued = QueuedEscalation {
            escalation_id: escalation_id.clone(),
            subsystem_id: subsystem_id.clone(),
            reason: reason.clone(),
            requested_duration_ms,
            trace_context: trace_context.clone(),
            is_reactive,
        };

        // Reactive requests are inserted at the front of the queue so they
        // are granted next when the current holder releases or expires.
        // All other requests go to the back. This is the mechanism by which
        // "reactive loop always takes precedence over CTP" (PRD §9.4).
        if is_reactive {
            state.queue.push_front(queued);
        } else {
            state.queue.push_back(queued);
        }

        let queue_depth = state.queue.len();
        let current_holder_id = state
            .current_holder
            .as_ref()
            .map(|holder| holder.subsystem_id.clone())
            .unwrap_or_default();

        drop(state);

        self.publish_escalation_event(
            EventTopic::TopicEscalationQueued,
            &escalation_id,
            &subsystem_id,
            &trace_context,
        );

        tracing::info!(
            subsystem = %subsystem_id,
            event_type = "escalation_queued",
            escalation_id = %escalation_id,
            current_holder = %current_holder_id,
            queue_depth = queue_depth,
            is_reactive = is_reactive,
            reason = %reason,
            "tier 2 escalation queued"
        );

        Ok(EscalationOutcome::Queued { escalation_id })
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Release escalation
    // ─────────────────────────────────────────────────────────────────────────

    /// Voluntarily release a Tier 2 escalation before its expiry.
    ///
    /// Returns Ok(true) if the escalation was found and released.
    /// Returns Ok(false) if the escalation_id does not match the current holder
    /// (already expired, or invalid ID).
    pub async fn release_escalation(
        &self,
        escalation_id: &str,
        subsystem_id: &str,
    ) -> SenaResult<bool> {
        let mut state = self.inner.state.write().await;

        let is_current_holder = state
            .current_holder
            .as_ref()
            .map(|holder| holder.escalation_id == escalation_id)
            .unwrap_or(false);

        if !is_current_holder {
            // Maybe the caller is trying to cancel a queued request.
            let removed_from_queue = self.remove_from_queue(&mut state.queue, escalation_id);
            drop(state);

            if removed_from_queue {
                tracing::info!(
                    subsystem = %subsystem_id,
                    event_type = "escalation_dequeued",
                    escalation_id = %escalation_id,
                    "queued escalation request cancelled"
                );
                return Ok(true);
            }

            tracing::debug!(
                subsystem = %subsystem_id,
                event_type = "escalation_release_not_found",
                escalation_id = %escalation_id,
                "release request for unknown or already-expired escalation"
            );
            return Ok(false);
        }

        // Clear the current holder.
        let released_subsystem = state
            .current_holder
            .take()
            .map(|holder| holder.subsystem_id)
            .unwrap_or_default();

        drop(state);

        // Cancel the expiry timer — the escalation was released voluntarily.
        // cancel_expiry_timer is synchronous — safe to call without await.
        self.cancel_expiry_timer();

        self.publish_escalation_event(
            EventTopic::TopicEscalationReleased,
            escalation_id,
            &released_subsystem,
            "",
        );

        tracing::info!(
            subsystem = %released_subsystem,
            event_type = "escalation_released",
            escalation_id = %escalation_id,
            "tier 2 escalation released voluntarily"
        );

        // Grant the next queued request, if any.
        self.grant_next_in_queue().await;

        Ok(true)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Expiry
    // ─────────────────────────────────────────────────────────────────────────

    /// Schedule a tokio timer that will automatically de-escalate the current
    /// holder when the granted duration expires.
    ///
    /// This is intentionally **not** async. Making it synchronous breaks the
    /// recursive async type chain: `schedule_expiry_timer` → (spawned task) →
    /// `on_escalation_expired` → `grant_next_in_queue` → `schedule_expiry_timer`.
    /// If every link in that chain were `async fn(&self)`, the compiler would
    /// need to compute a recursive `impl Future` type and fail to prove `Send`.
    /// By making this function synchronous and using `std::mem::replace` on the
    /// `Mutex`-based handle, we keep the spawned future's type finite.
    ///
    /// The timer handle is stored so it can be cancelled on voluntary release.
    /// Dropping the handle would cancel the timer — that is only correct when
    /// the escalation is released or replaced intentionally.
    fn schedule_expiry_timer(
        &self,
        escalation_id: &str,
        subsystem_id: &str,
        granted_duration_ms: u64,
    ) {
        // Cancel any existing expiry timer first — there should be at most one.
        self.cancel_expiry_timer();

        let arbiter = self.clone();
        let escalation_id_owned = escalation_id.to_string();
        let subsystem_id_owned = subsystem_id.to_string();
        let duration = Duration::from_millis(granted_duration_ms);

        let handle = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            arbiter
                .on_escalation_expired(&escalation_id_owned, &subsystem_id_owned)
                .await;
        });

        // std::sync::Mutex is fine here — held only for a pointer swap,
        // never across an await point, and never contended (only one timer
        // is active at a time).
        let mut expiry_handle = self
            .inner
            .expiry_handle
            .lock()
            .expect("expiry_handle mutex poisoned — this is a bug in daemon-bus");
        *expiry_handle = Some(handle);
    }

    /// Cancel the current expiry timer, if one is active.
    ///
    /// Synchronous for the same reason as `schedule_expiry_timer` — keeps the
    /// async type graph non-recursive so spawned futures remain `Send`.
    fn cancel_expiry_timer(&self) {
        let mut expiry_handle = self
            .inner
            .expiry_handle
            .lock()
            .expect("expiry_handle mutex poisoned — this is a bug in daemon-bus");
        if let Some(handle) = expiry_handle.take() {
            handle.abort();
        }
    }

    /// Called when the expiry timer fires. De-escalates the current holder
    /// and grants the next queued request.
    async fn on_escalation_expired(&self, escalation_id: &str, subsystem_id: &str) {
        let mut state = self.inner.state.write().await;

        // Verify the escalation_id still matches — a voluntary release could
        // have already cleared it between the timer firing and this lock acquisition.
        let is_still_current = state
            .current_holder
            .as_ref()
            .map(|holder| holder.escalation_id == escalation_id)
            .unwrap_or(false);

        if !is_still_current {
            // Already released or replaced — nothing to do.
            return;
        }

        state.current_holder = None;
        drop(state);

        self.publish_escalation_event(
            EventTopic::TopicEscalationExpired,
            escalation_id,
            subsystem_id,
            "",
        );

        tracing::warn!(
            subsystem = %subsystem_id,
            event_type = "escalation_expired",
            escalation_id = %escalation_id,
            "tier 2 escalation expired — auto de-escalated"
        );

        // Grant the next queued request, if any.
        self.grant_next_in_queue().await;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Queue management
    // ─────────────────────────────────────────────────────────────────────────

    /// Pop the next request from the queue and grant it Tier 2.
    ///
    /// Called after the current holder releases or expires. If the queue is
    /// empty, Tier 2 simply becomes free.
    async fn grant_next_in_queue(&self) {
        let next_request = {
            let mut state = self.inner.state.write().await;

            // Only grant if Tier 2 is actually free — a concurrent request
            // might have grabbed it between the release and this call.
            if state.current_holder.is_some() {
                return;
            }

            state.queue.pop_front()
        };

        let Some(next) = next_request else {
            tracing::debug!(
                subsystem = "daemon_bus",
                event_type = "escalation_queue_empty",
                "no queued escalation requests — tier 2 is free"
            );
            return;
        };

        let granted_duration_ms = self.clamp_duration(next.requested_duration_ms);
        let snapshot = ActiveEscalationSnapshot {
            escalation_id: next.escalation_id.clone(),
            subsystem_id: next.subsystem_id.clone(),
            granted_duration_ms,
        };

        {
            let mut state = self.inner.state.write().await;
            state.current_holder = Some(snapshot);
        }

        // schedule_expiry_timer is synchronous — safe to call without await.
        self.schedule_expiry_timer(&next.escalation_id, &next.subsystem_id, granted_duration_ms);

        self.publish_escalation_event(
            EventTopic::TopicEscalationGranted,
            &next.escalation_id,
            &next.subsystem_id,
            &next.trace_context,
        );

        tracing::info!(
            subsystem = %next.subsystem_id,
            event_type = "escalation_granted_from_queue",
            escalation_id = %next.escalation_id,
            granted_duration_ms = granted_duration_ms,
            reason = %next.reason,
            "queued escalation request granted tier 2"
        );
    }

    /// Remove a specific escalation from the queue by ID.
    /// Returns true if it was found and removed.
    fn remove_from_queue(
        &self,
        queue: &mut VecDeque<QueuedEscalation>,
        escalation_id: &str,
    ) -> bool {
        let original_len = queue.len();
        queue.retain(|entry| entry.escalation_id != escalation_id);
        queue.len() < original_len
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Duration clamping
    // ─────────────────────────────────────────────────────────────────────────

    /// Clamp the requested escalation duration to the configured bounds.
    ///
    /// If the requester sends 0, the default is used. The result never exceeds
    /// `config.max_escalation_duration_ms`.
    fn clamp_duration(&self, requested_duration_ms: u64) -> u64 {
        if requested_duration_ms == 0 {
            self.inner.config.default_escalation_duration_ms
        } else {
            requested_duration_ms.min(self.inner.config.max_escalation_duration_ms)
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Event publishing
    // ─────────────────────────────────────────────────────────────────────────

    /// Publish an escalation lifecycle event to the bus.
    fn publish_escalation_event(
        &self,
        topic: EventTopic,
        escalation_id: &str,
        subsystem_id: &str,
        trace_context: &str,
    ) {
        let payload = build_escalation_payload(escalation_id, subsystem_id);
        let _receiver_count = self.inner.event_bus.publish(InternalBusEvent::new(
            topic,
            subsystem_id,
            payload,
            trace_context,
        ));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Status queries (for gRPC ArbitrationService)
    // ─────────────────────────────────────────────────────────────────────────

    /// Returns a snapshot of the current arbitration state.
    pub async fn get_status(&self) -> ArbitrationSnapshot {
        let state = self.inner.state.read().await;
        ArbitrationSnapshot {
            current_tier_two_holder: state
                .current_holder
                .as_ref()
                .map(|holder| holder.subsystem_id.clone()),
            current_escalation_id: state
                .current_holder
                .as_ref()
                .map(|holder| holder.escalation_id.clone()),
            queue_depth: state.queue.len(),
        }
    }

    /// Returns the configured reactive subsystem ID.
    /// Used by callers that need to tag requests as reactive for priority sorting.
    pub fn reactive_subsystem_id(&self) -> &str {
        &self.inner.config.reactive_subsystem_id
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Status snapshot
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of the current arbitration state for diagnostics.
pub struct ArbitrationSnapshot {
    /// Subsystem currently holding Tier 2, if any.
    pub current_tier_two_holder: Option<String>,
    /// Escalation ID of the current holder, if any.
    pub current_escalation_id: Option<String>,
    /// Number of queued escalation requests.
    pub queue_depth: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload builder
// ─────────────────────────────────────────────────────────────────────────────
// JSON payload for escalation bus events — non-uniform internal structure per
// serialization rules in copilot-instructions.md.

fn build_escalation_payload(escalation_id: &str, subsystem_id: &str) -> Vec<u8> {
    format!(
        r#"{{"escalation_id":"{}","subsystem_id":"{}"}}"#,
        escalation_id, subsystem_id
    )
    .into_bytes()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::ArbitrationConfig;

    fn test_config() -> ArbitrationConfig {
        ArbitrationConfig {
            max_escalation_duration_ms: 5000,
            default_escalation_duration_ms: 2000,
            max_queue_depth: 4,
            reactive_subsystem_id: "reactive_loop".to_string(),
        }
    }

    #[tokio::test]
    async fn grant_when_tier_two_is_free() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        let outcome = arbiter
            .request_escalation(
                "ctp".to_string(),
                "background reasoning".to_string(),
                3000,
                "trace-1".to_string(),
            )
            .await
            .expect("request should succeed");

        match outcome {
            EscalationOutcome::Granted {
                granted_duration_ms,
                ..
            } => {
                assert_eq!(granted_duration_ms, 3000);
            }
            other => panic!("expected Granted, got {:?}", other),
        }

        let status = arbiter.get_status().await;
        assert_eq!(status.current_tier_two_holder.as_deref(), Some("ctp"));
    }

    #[tokio::test]
    async fn second_request_is_queued() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        // First request — granted.
        let first = arbiter
            .request_escalation("ctp".to_string(), "first".to_string(), 2000, String::new())
            .await
            .expect("first request should succeed");
        assert!(matches!(first, EscalationOutcome::Granted { .. }));

        // Second request — queued.
        let second = arbiter
            .request_escalation(
                "memory_engine".to_string(),
                "second".to_string(),
                2000,
                String::new(),
            )
            .await
            .expect("second request should succeed");
        assert!(matches!(second, EscalationOutcome::Queued { .. }));

        let status = arbiter.get_status().await;
        assert_eq!(status.queue_depth, 1);
    }

    #[tokio::test]
    async fn reactive_jumps_queue_ahead_of_ctp() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        // Grant tier 2 to some subsystem.
        arbiter
            .request_escalation(
                "some_holder".to_string(),
                "holds tier 2".to_string(),
                5000,
                String::new(),
            )
            .await
            .expect("grant should succeed");

        // Queue a CTP request.
        let ctp_outcome = arbiter
            .request_escalation(
                "ctp".to_string(),
                "background".to_string(),
                2000,
                String::new(),
            )
            .await
            .expect("ctp queue should succeed");
        let _ctp_id = match ctp_outcome {
            EscalationOutcome::Queued { escalation_id } => escalation_id,
            other => panic!("expected Queued, got {:?}", other),
        };

        // Queue a reactive request — should jump in front of CTP.
        let reactive_outcome = arbiter
            .request_escalation(
                "reactive_loop".to_string(),
                "user request".to_string(),
                1000,
                String::new(),
            )
            .await
            .expect("reactive queue should succeed");
        let _reactive_id = match reactive_outcome {
            EscalationOutcome::Queued { escalation_id } => escalation_id,
            other => panic!("expected Queued, got {:?}", other),
        };

        // Verify queue depth is 2.
        let status = arbiter.get_status().await;
        assert_eq!(status.queue_depth, 2);

        // Release the current holder — the reactive request should be granted next.
        let current_esc_id = status.current_escalation_id.expect("should have holder");
        arbiter
            .release_escalation(&current_esc_id, "some_holder")
            .await
            .expect("release should succeed");

        // Give tokio a moment to process the grant_next_in_queue.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let status_after = arbiter.get_status().await;
        assert_eq!(
            status_after.current_tier_two_holder.as_deref(),
            Some("reactive_loop"),
            "reactive loop should be granted tier 2 before CTP"
        );
        assert_eq!(status_after.queue_depth, 1, "CTP should still be queued");
    }

    #[tokio::test]
    async fn duration_clamped_to_max() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        let outcome = arbiter
            .request_escalation(
                "ctp".to_string(),
                "long request".to_string(),
                999_999,
                String::new(),
            )
            .await
            .expect("request should succeed");

        match outcome {
            EscalationOutcome::Granted {
                granted_duration_ms,
                ..
            } => {
                assert_eq!(granted_duration_ms, 5000, "should be clamped to max");
            }
            other => panic!("expected Granted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn zero_duration_uses_default() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        let outcome = arbiter
            .request_escalation(
                "ctp".to_string(),
                "default duration".to_string(),
                0,
                String::new(),
            )
            .await
            .expect("request should succeed");

        match outcome {
            EscalationOutcome::Granted {
                granted_duration_ms,
                ..
            } => {
                assert_eq!(granted_duration_ms, 2000, "should use default");
            }
            other => panic!("expected Granted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn queue_overflow_denied() {
        let bus = EventBus::new(64);
        let config = ArbitrationConfig {
            max_queue_depth: 2,
            ..test_config()
        };
        let arbiter = Arbiter::new(config, bus);

        // Grant tier 2 to hold it.
        arbiter
            .request_escalation(
                "holder".to_string(),
                "hold".to_string(),
                5000,
                String::new(),
            )
            .await
            .expect("grant should succeed");

        // Fill the queue.
        arbiter
            .request_escalation("q1".to_string(), "q".to_string(), 1000, String::new())
            .await
            .expect("queue 1 should succeed");
        arbiter
            .request_escalation("q2".to_string(), "q".to_string(), 1000, String::new())
            .await
            .expect("queue 2 should succeed");

        // Third queued request should be denied.
        let outcome = arbiter
            .request_escalation("q3".to_string(), "q".to_string(), 1000, String::new())
            .await
            .expect("request should succeed (returns Denied, not Err)");

        assert!(
            matches!(outcome, EscalationOutcome::Denied { .. }),
            "expected Denied, got {:?}",
            outcome
        );
    }

    #[tokio::test]
    async fn voluntary_release_clears_holder() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        let outcome = arbiter
            .request_escalation("ctp".to_string(), "test".to_string(), 2000, String::new())
            .await
            .expect("grant should succeed");

        let escalation_id = match outcome {
            EscalationOutcome::Granted { escalation_id, .. } => escalation_id,
            other => panic!("expected Granted, got {:?}", other),
        };

        let released = arbiter
            .release_escalation(&escalation_id, "ctp")
            .await
            .expect("release should succeed");
        assert!(released);

        let status = arbiter.get_status().await;
        assert!(status.current_tier_two_holder.is_none());
    }

    #[tokio::test]
    async fn expiry_auto_deescalates() {
        let bus = EventBus::new(16);
        let config = ArbitrationConfig {
            max_escalation_duration_ms: 100,
            default_escalation_duration_ms: 50,
            ..test_config()
        };
        let arbiter = Arbiter::new(config, bus);

        let outcome = arbiter
            .request_escalation("ctp".to_string(), "short".to_string(), 50, String::new())
            .await
            .expect("grant should succeed");
        assert!(matches!(outcome, EscalationOutcome::Granted { .. }));

        // Wait for expiry.
        tokio::time::sleep(Duration::from_millis(120)).await;

        let status = arbiter.get_status().await;
        assert!(
            status.current_tier_two_holder.is_none(),
            "escalation should have auto-expired"
        );
    }

    #[tokio::test]
    async fn release_unknown_escalation_returns_false() {
        let bus = EventBus::new(16);
        let arbiter = Arbiter::new(test_config(), bus);

        let released = arbiter
            .release_escalation("nonexistent-id", "ctp")
            .await
            .expect("release should not error");
        assert!(!released);
    }
}
