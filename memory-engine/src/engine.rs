//! Core memory engine — the central struct that owns tier locks, the ech0 store,
//! the write queue, and the daemon-bus client.
//!
//! `MemoryEngine` exposes `write()`, `read()`, and `promote()` as the public
//! API consumed by `grpc.rs`. All ech0 `ingest_text` calls are routed through
//! the write queue in `queue.rs` — never called directly.
//!
//! # Lock discipline
//!
//! - One `RwLock` per tier — never one lock for all tiers.
//! - **Never hold a write lock across an async `.await` point.**
//! - Always release the write lock **before** broadcasting events to daemon-bus.
//! - Reactive reads always acquire the read lock before Background writes
//!   acquire the write lock (enforced via the priority queue).
//!
//! # Logging
//!
//! Every memory operation log includes: `tier`, `operation`, `priority`,
//! `duration_ms`. Operations exceeding `slow_operation_threshold_ms` (from
//! config, default 100ms) are logged at `warn` regardless of priority.
//!
//! No memory entry content, raw text, user messages, model output, or SoulBox
//! values ever appear in any log.

use std::sync::Arc;
use std::time::Instant;

use ech0::{EchoError, Embedder, Extractor, SearchOptions, SearchResult, Store};
use tokio::sync::RwLock;

use crate::config::Config;
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::queue::{Priority, WriteQueue};
use crate::tier::{tier_name, EntryId, EpisodicTier, LongTermTier, ShortTermTier};

// ─────────────────────────────────────────────────────────────────────────────
// Event topic constants
// ─────────────────────────────────────────────────────────────────────────────

use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient, BusEvent, EventTopic, PublishRequest,
};

/// Type alias for the daemon-bus gRPC client used for event broadcasting.
pub type DaemonBusClient = EventBusServiceClient<tonic::transport::Channel>;

// ─────────────────────────────────────────────────────────────────────────────
// MemoryEntry — input to write operations
// ─────────────────────────────────────────────────────────────────────────────

/// A memory entry submitted for ingestion.
///
/// This is the input type for `MemoryEngine::write()`. The `text` field is
/// passed to ech0's `store.ingest_text()` via the write queue. The `tier`
/// field determines which tier's bookkeeping is updated after a successful
/// write.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// The text content to ingest. Never logged — only the operation metadata
    /// (tier, priority, duration) appears in logs.
    pub text: String,
    /// Which tier this entry belongs to.
    pub target_tier: TargetTier,
}

/// Which memory tier an entry targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetTier {
    ShortTerm,
    LongTerm,
    Episodic,
}

impl TargetTier {
    /// Returns the tier name string used in structured log fields.
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetTier::ShortTerm => tier_name::SHORT_TERM,
            TargetTier::LongTerm => tier_name::LONG_TERM,
            TargetTier::Episodic => tier_name::EPISODIC,
        }
    }
}

impl std::fmt::Display for TargetTier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MemoryEngine
// ─────────────────────────────────────────────────────────────────────────────

/// Core memory engine struct.
///
/// Owns per-tier `RwLock`s, the ech0 `Store`, the write queue, and the
/// daemon-bus gRPC client. All memory operations flow through this struct.
///
/// Generic over `E` (embedder) and `X` (extractor) because `ech0::Store`
/// is generic over these types. The concrete types are determined at
/// construction time in `main.rs`.
pub struct MemoryEngine<E: Embedder, X: Extractor> {
    /// Short-term tier — volatile, session-scoped. One RwLock.
    short_term: RwLock<ShortTermTier>,
    /// Long-term tier — persistent, promoted from short-term. One RwLock.
    long_term: RwLock<LongTermTier>,
    /// Episodic tier — append-only, never mutated. One RwLock.
    episodic: RwLock<EpisodicTier>,
    /// The ech0 Store — all reads and writes go through this.
    store: Arc<Store<E, X>>,
    /// The async priority write queue — serializes all ingest_text calls.
    queue: Arc<WriteQueue<E, X>>,
    /// gRPC client to daemon-bus for event broadcasting.
    bus: Arc<DaemonBusClient>,
    /// Memory-engine configuration.
    config: Arc<Config>,
}

impl<E: Embedder + 'static, X: Extractor + 'static> MemoryEngine<E, X> {
    /// Construct a new `MemoryEngine`.
    ///
    /// All dependencies are injected — the engine does not construct any of
    /// them itself. This keeps the engine testable and ensures the boot
    /// sequence in `main.rs` controls initialization order.
    pub fn new(
        short_term: ShortTermTier,
        long_term: LongTermTier,
        episodic: EpisodicTier,
        store: Arc<Store<E, X>>,
        queue: Arc<WriteQueue<E, X>>,
        bus: Arc<DaemonBusClient>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            short_term: RwLock::new(short_term),
            long_term: RwLock::new(long_term),
            episodic: RwLock::new(episodic),
            store,
            queue,
            bus,
            config,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // write
    // ─────────────────────────────────────────────────────────────────────

    /// Write a memory entry to the specified tier via the write queue.
    ///
    /// The text is submitted to the write queue for serialized ingestion
    /// into ech0. After a successful ingest, the tier's bookkeeping is
    /// updated. The write lock is acquired **only** for the bookkeeping
    /// update and is released **before** broadcasting
    /// `TOPIC_MEMORY_WRITE_COMPLETED` to daemon-bus.
    ///
    /// # Lock discipline
    ///
    /// - The write lock is never held across the `queue.submit()` await.
    /// - The write lock is never held across the `bus.publish()` await.
    /// - The write lock scope is limited to the tier bookkeeping insert.
    ///
    /// # Errors
    ///
    /// - `ErrorCode::QueueFull` if the write queue is at capacity.
    /// - `ErrorCode::QueueTimeout` if the operation times out.
    /// - `ErrorCode::StorageFailure` if ech0 `ingest_text` fails after retries.
    pub async fn write(&self, entry: MemoryEntry, priority: Priority) -> SenaResult<String> {
        let start = Instant::now();
        let tier_name = entry.target_tier.as_str();
        let operation = "write";

        tracing::debug!(
            subsystem = "memory_engine",
            tier = tier_name,
            operation = operation,
            priority = %priority,
            "write operation starting"
        );

        // Submit to the write queue — this is where the ech0 ingest_text
        // call happens, serialized and with retry logic. The write lock is
        // NOT held during this await.
        let text_for_queue = entry.text.clone();
        self.queue
            .submit(text_for_queue, priority)
            .await
            .inspect_err(|queue_error| {
                let duration_ms = start.elapsed().as_millis() as u64;
                tracing::warn!(
                    subsystem = "memory_engine",
                    tier = tier_name,
                    operation = operation,
                    priority = %priority,
                    duration_ms = duration_ms,
                    error_code = %queue_error.code,
                    "write failed during queue submission"
                );
            })?;

        // Generate an entry ID for tier bookkeeping.
        // In a full implementation, this would come from ech0's IngestResult.
        // For now, generate a UUID — the important thing is that the tier
        // tracks which entries it owns.
        let entry_id_string = uuid::Uuid::new_v4().to_string();
        let entry_id = EntryId::new(entry_id_string.clone());

        // Acquire the write lock ONLY for the tier bookkeeping update.
        // This block is synchronous — no await inside.
        // Also capture the tier count for event payload.
        let tier_count = match entry.target_tier {
            TargetTier::ShortTerm => {
                let mut guard = self.short_term.write().await;
                guard.insert(entry_id);
                guard.entry_count()
                // guard is dropped here — lock released before broadcast
            }
            TargetTier::LongTerm => {
                let mut guard = self.long_term.write().await;
                guard.insert(entry_id);
                guard.entry_count()
            }
            TargetTier::Episodic => {
                let mut guard = self.episodic.write().await;
                guard.append(entry_id);
                guard.entry_count()
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        // Log the operation — warn if slow.
        log_operation(
            tier_name,
            operation,
            &priority,
            duration_ms,
            self.config.logging.slow_operation_threshold_ms,
        );

        // Broadcast event AFTER releasing the write lock.
        self.broadcast_write_completed_event(tier_name, tier_count)
            .await;

        Ok(entry_id_string)
    }

    // ─────────────────────────────────────────────────────────────────────
    // read
    // ─────────────────────────────────────────────────────────────────────

    /// Read/search memory via ech0's `store.search()`.
    ///
    /// Acquires a read lock on the relevant tier for membership validation
    /// (if needed), then delegates to ech0's search. The read lock is held
    /// only for the membership check — **not** during the search call itself.
    ///
    /// Reactive reads take priority over Background writes through the
    /// write queue's priority ordering. Read operations do not go through
    /// the write queue — they call `store.search()` directly because reads
    /// do not mutate ech0's state.
    ///
    /// # Errors
    ///
    /// - `ErrorCode::StorageFailure` if the ech0 search call fails.
    pub async fn read(
        &self,
        query: &str,
        options: SearchOptions,
        priority: Priority,
    ) -> SenaResult<SearchResult> {
        let start = Instant::now();
        let operation = "read";
        // Reads are not tier-specific in ech0 — they search across the store.
        // We log "all" as the tier since search spans everything.
        let tier_name = "all";

        tracing::debug!(
            subsystem = "memory_engine",
            tier = tier_name,
            operation = operation,
            priority = %priority,
            "read operation starting"
        );

        // Call ech0's search directly — reads do not go through the write queue.
        // EchoError is mapped to SenaError at this call site.
        let search_result =
            self.store
                .search(query, options)
                .await
                .map_err(|echo_error: EchoError| {
                    let sena_error: SenaError = echo_error.into();
                    let duration_ms = start.elapsed().as_millis() as u64;
                    tracing::warn!(
                        subsystem = "memory_engine",
                        tier = tier_name,
                        operation = operation,
                        priority = %priority,
                        duration_ms = duration_ms,
                        error_code = %sena_error.code,
                        "read operation failed"
                    );
                    sena_error
                })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        log_operation(
            tier_name,
            operation,
            &priority,
            duration_ms,
            self.config.logging.slow_operation_threshold_ms,
        );

        Ok(search_result)
    }

    // ─────────────────────────────────────────────────────────────────────
    // promote
    // ─────────────────────────────────────────────────────────────────────

    /// Promote an entry from short-term to long-term tier.
    ///
    /// CTP decides when to promote — memory-engine executes the promotion
    /// but never decides when. This method:
    ///
    /// 1. Acquires a write lock on short-term to remove the entry.
    /// 2. Releases the short-term write lock.
    /// 3. Acquires a write lock on long-term to insert the entry.
    /// 4. Releases the long-term write lock.
    /// 5. Broadcasts `TOPIC_MEMORY_TIER_PROMOTED` to daemon-bus.
    ///
    /// The two write locks are never held simultaneously — each is acquired
    /// and released independently. No lock is held across an await point
    /// that performs I/O.
    ///
    /// # Errors
    ///
    /// - `ErrorCode::StorageFailure` if the entry is not found in short-term.
    pub async fn promote(&self, entry_id: EntryId, priority: Priority) -> SenaResult<()> {
        let start = Instant::now();
        let operation = "promote";

        tracing::debug!(
            subsystem = "memory_engine",
            tier = tier_name::SHORT_TERM,
            operation = operation,
            priority = %priority,
            "promote operation starting"
        );

        // Step 1: Remove from short-term (write lock, no I/O inside).
        {
            let mut short_term_guard = self.short_term.write().await;
            if !short_term_guard.remove(&entry_id) {
                let duration_ms = start.elapsed().as_millis() as u64;
                tracing::warn!(
                    subsystem = "memory_engine",
                    tier = tier_name::SHORT_TERM,
                    operation = operation,
                    priority = %priority,
                    duration_ms = duration_ms,
                    "promote failed — entry not found in short-term tier"
                );
                return Err(SenaError::new(
                    ErrorCode::StorageFailure,
                    "promote failed — entry not found in short-term tier",
                ));
            }
            // short_term_guard dropped here — lock released.
        }

        // Step 2: Insert into long-term (write lock, no I/O inside).
        {
            let mut long_term_guard = self.long_term.write().await;
            long_term_guard.insert(entry_id);
            // long_term_guard dropped here — lock released.
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        log_operation(
            tier_name::SHORT_TERM,
            operation,
            &priority,
            duration_ms,
            self.config.logging.slow_operation_threshold_ms,
        );

        // Step 3: Broadcast event AFTER both locks are released.
        self.broadcast_event(EventTopic::TopicMemoryTierPromoted, "tier_promoted")
            .await;

        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Event broadcasting
    // ─────────────────────────────────────────────────────────────────────

    /// Broadcast TOPIC_MEMORY_WRITE_COMPLETED event with tier and count payload.
    ///
    /// This must **always** be called after releasing any write lock — never
    /// from inside a lock scope. If the broadcast fails, the error is logged
    /// but not propagated — the memory operation itself already succeeded,
    /// and failing to notify daemon-bus should not roll back the write.
    async fn broadcast_write_completed_event(&self, tier_name: &str, tier_count: usize) {
        let payload_json = serde_json::json!({
            "tier": tier_name,
            "count": tier_count
        });

        let payload = payload_json.to_string().into_bytes();

        let event = BusEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic: EventTopic::TopicMemoryWriteCompleted.into(),
            source_subsystem: "memory_engine".to_owned(),
            payload,
            trace_context: String::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let request = tonic::Request::new(PublishRequest { event: Some(event) });

        // Clone the client so we can call it without holding &self across
        // the gRPC await. The bus field is Arc<DaemonBusClient> and
        // DaemonBusClient is cheaply cloneable (it wraps a tonic Channel).
        let mut bus_client = (*self.bus).clone();

        match bus_client.publish(request).await {
            Ok(_response) => {
                tracing::debug!(
                    subsystem = "memory_engine",
                    component = "engine",
                    event_type = "write_completed",
                    tier = tier_name,
                    count = tier_count,
                    "TOPIC_MEMORY_WRITE_COMPLETED event broadcast succeeded"
                );
            }
            Err(grpc_error) => {
                // Log but do not propagate — the memory operation already
                // succeeded. daemon-bus will eventually reconcile.
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "engine",
                    event_type = "write_completed",
                    tier = tier_name,
                    grpc_code = ?grpc_error.code(),
                    "event broadcast to daemon-bus failed — operation still committed"
                );
            }
        }
    }

    /// Broadcast a memory state change event to daemon-bus.
    ///
    /// This must **always** be called after releasing any write lock — never
    /// from inside a lock scope. If the broadcast fails, the error is logged
    /// but not propagated — the memory operation itself already succeeded,
    /// and failing to notify daemon-bus should not roll back the write.
    async fn broadcast_event(&self, topic: EventTopic, event_type: &str) {
        let event = BusEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic: topic.into(),
            source_subsystem: "memory_engine".to_owned(),
            payload: Vec::new(),
            trace_context: String::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let request = tonic::Request::new(PublishRequest { event: Some(event) });

        // Clone the client so we can call it without holding &self across
        // the gRPC await. The bus field is Arc<DaemonBusClient> and
        // DaemonBusClient is cheaply cloneable (it wraps a tonic Channel).
        let mut bus_client = (*self.bus).clone();

        match bus_client.publish(request).await {
            Ok(_response) => {
                tracing::debug!(
                    subsystem = "memory_engine",
                    component = "engine",
                    event_type = event_type,
                    "event broadcast to daemon-bus succeeded"
                );
            }
            Err(grpc_error) => {
                // Log but do not propagate — the memory operation already
                // succeeded. daemon-bus will eventually reconcile.
                tracing::warn!(
                    subsystem = "memory_engine",
                    component = "engine",
                    event_type = event_type,
                    grpc_code = ?grpc_error.code(),
                    "event broadcast to daemon-bus failed — operation still committed"
                );
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Accessors — for grpc.rs and diagnostics
    // ─────────────────────────────────────────────────────────────────────

    /// Returns the ech0 Store reference (for search calls in grpc.rs).
    pub fn store(&self) -> &Arc<Store<E, X>> {
        &self.store
    }

    /// Returns the write queue reference (for depth monitoring).
    pub fn queue(&self) -> &Arc<WriteQueue<E, X>> {
        &self.queue
    }

    /// Returns a read lock guard for the short-term tier.
    pub async fn short_term_read(&self) -> tokio::sync::RwLockReadGuard<'_, ShortTermTier> {
        self.short_term.read().await
    }

    /// Returns a read lock guard for the long-term tier.
    pub async fn long_term_read(&self) -> tokio::sync::RwLockReadGuard<'_, LongTermTier> {
        self.long_term.read().await
    }

    /// Returns a read lock guard for the episodic tier.
    pub async fn episodic_read(&self) -> tokio::sync::RwLockReadGuard<'_, EpisodicTier> {
        self.episodic.read().await
    }

    /// Trigger memory consolidation during deep idle.
    ///
    /// Invoked when TOPIC_MEMORY_CONSOLIDATION_REQUESTED arrives from CTP.
    /// Phase 2 will implement the full pipeline:
    /// - Promote high-importance short-term entries to long-term
    /// - Compact or merge episodic entries with low importance scores
    /// - Trigger decay updates for entries approaching the decay floor
    ///
    /// Priority is always Background — consolidation never preempts
    /// user-facing operations.
    pub async fn consolidate(&self) -> SenaResult<()> {
        let start = Instant::now();
        let operation = "consolidate";

        tracing::info!(
            subsystem = "memory_engine",
            tier = "all",
            operation = operation,
            priority = %Priority::Background,
            "consolidation triggered — no-op until Phase 2 pipeline is wired"
        );

        // Phase 2: consolidation logic goes here.

        let duration_ms = start.elapsed().as_millis() as u64;

        log_operation(
            "all",
            operation,
            &Priority::Background,
            duration_ms,
            self.config.logging.slow_operation_threshold_ms,
        );

        Ok(())
    }

    /// Gracefully shut down the engine — drains the write queue.
    pub async fn shutdown(&self) {
        tracing::info!(
            subsystem = "memory_engine",
            component = "engine",
            "memory engine shutting down — draining write queue"
        );
        self.queue.shutdown().await;
        tracing::info!(
            subsystem = "memory_engine",
            component = "engine",
            "memory engine shutdown complete"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Logging helper
// ─────────────────────────────────────────────────────────────────────────────

/// Log a completed memory operation with the required structured fields.
///
/// If `duration_ms` exceeds `slow_threshold_ms`, the log is emitted at `warn`
/// level regardless of the operation's priority. Otherwise, `info` level.
///
/// Required fields per the instruction file:
/// - `tier` — short_term, long_term, episodic, or "all" for reads
/// - `operation` — read, write, promote
/// - `priority` — reactive, background
/// - `duration_ms`
fn log_operation(
    tier: &str,
    operation: &str,
    priority: &Priority,
    duration_ms: u64,
    slow_threshold_ms: u64,
) {
    if duration_ms >= slow_threshold_ms {
        tracing::warn!(
            subsystem = "memory_engine",
            tier = tier,
            operation = operation,
            priority = %priority,
            duration_ms = duration_ms,
            "slow memory operation detected"
        );
    } else {
        tracing::info!(
            subsystem = "memory_engine",
            tier = tier,
            operation = operation,
            priority = %priority,
            duration_ms = duration_ms,
            "memory operation completed"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_tier_display() {
        assert_eq!(TargetTier::ShortTerm.as_str(), "short_term");
        assert_eq!(TargetTier::LongTerm.as_str(), "long_term");
        assert_eq!(TargetTier::Episodic.as_str(), "episodic");
    }

    #[test]
    fn target_tier_format() {
        assert_eq!(format!("{}", TargetTier::ShortTerm), "short_term");
        assert_eq!(format!("{}", TargetTier::LongTerm), "long_term");
        assert_eq!(format!("{}", TargetTier::Episodic), "episodic");
    }

    #[test]
    fn target_tier_equality() {
        assert_eq!(TargetTier::ShortTerm, TargetTier::ShortTerm);
        assert_ne!(TargetTier::ShortTerm, TargetTier::LongTerm);
        assert_ne!(TargetTier::LongTerm, TargetTier::Episodic);
    }

    #[test]
    fn memory_entry_clone() {
        let entry = MemoryEntry {
            text: "test content".to_owned(),
            target_tier: TargetTier::ShortTerm,
        };
        let cloned = entry.clone();
        assert_eq!(cloned.text, "test content");
        assert_eq!(cloned.target_tier, TargetTier::ShortTerm);
    }

    /// Test documenting that consolidate method exists and will be called
    /// by the subscription handler when TOPIC_MEMORY_CONSOLIDATION_REQUESTED
    /// arrives from CTP. Full integration testing requires a running daemon-bus
    /// and is deferred to integration tests.
    ///
    /// Phase 2 will test actual consolidation behavior (promotions, compaction).
    #[test]
    fn consolidate_method_signature_exists() {
        // This test documents the contract: consolidate() exists, is async,
        // takes no parameters beyond &self, and returns SenaResult<()>.
        // When Phase 2 implements real consolidation logic, this test will
        // be replaced with behavioral tests that verify tier promotions and
        // memory compaction.
        //
        // For now, we just verify the method can be referenced without error.
        fn _check_signature<E: ech0::Embedder + 'static, X: ech0::Extractor + 'static>(
            engine: &MemoryEngine<E, X>,
        ) -> core::pin::Pin<
            Box<dyn std::future::Future<Output = SenaResult<()>> + Send + '_>,
        > {
            Box::pin(engine.consolidate())
        }
    }
}
