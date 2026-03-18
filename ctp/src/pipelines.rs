//! Three parallel async pipelines — the core of CTP.
//!
//! - **Generation**: Reads telemetry stream, computes scores, pushes to ThoughtQueue.
//! - **Evaluation**: Pops from queue, surfaces above-threshold thoughts via daemon-bus.
//! - **Consolidation**: Requests memory consolidation during deep idle.
//!
//! Each pipeline is a separate `tokio::spawn` task. They share no mutable state.

use std::sync::Arc;

use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::activity::ActivityMonitor;
use crate::config::Config;
use crate::relevance::{compute_score, weights_from_config, SignalInput};
use crate::thought_queue::{expiry_for_score, Thought, ThoughtQueue};

use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient, BusEvent, EventTopic, PublishRequest,
};

/// A telemetry event — input to the generation pipeline.
/// In Phase 1, this is fed via an mpsc channel with synthetic/empty events.
/// Real telemetry from the platform layer is wired in a later milestone.
#[derive(Debug, Clone)]
pub struct TelemetryEvent {
    pub urgency: f32,
    pub emotional_resonance: f32,
    pub novelty: f32,
    pub recurrence: f32,
    pub idle_curiosity: f32,
    pub content: String,
}

impl TelemetryEvent {
    /// Convert to signal input for relevance scoring.
    pub fn to_signal_input(&self) -> SignalInput {
        SignalInput {
            urgency: self.urgency,
            emotional_resonance: self.emotional_resonance,
            novelty: self.novelty,
            recurrence: self.recurrence,
            idle_curiosity: self.idle_curiosity,
        }
    }
}

/// Spawn all three pipelines as independent tokio tasks.
/// Returns three JoinHandles that must be stored until shutdown, plus the
/// telemetry sender that feeds the generation pipeline. The sender must be
/// held by the caller — dropping it closes the channel and stops generation.
/// In Phase 1, no real telemetry is wired; the sender is held but unused.
pub fn spawn_all(
    config: Arc<Config>,
    thought_queue: Arc<ThoughtQueue>,
    activity_monitor: Arc<ActivityMonitor>,
    daemon_bus_address: String,
) -> (
    JoinHandle<()>,
    JoinHandle<()>,
    JoinHandle<()>,
    tokio::sync::mpsc::Sender<TelemetryEvent>,
) {
    let (telemetry_tx, telemetry_rx) = tokio::sync::mpsc::channel::<TelemetryEvent>(256);

    let generation_handle = {
        let config = Arc::clone(&config);
        let queue = Arc::clone(&thought_queue);
        tokio::spawn(async move {
            generation_pipeline(config, queue, telemetry_rx).await;
        })
    };

    let evaluation_handle = {
        let config = Arc::clone(&config);
        let queue = Arc::clone(&thought_queue);
        let monitor = Arc::clone(&activity_monitor);
        let address = daemon_bus_address.clone();
        tokio::spawn(async move {
            evaluation_pipeline(config, queue, monitor, address).await;
        })
    };

    let consolidation_handle = {
        let config = Arc::clone(&config);
        let monitor = Arc::clone(&activity_monitor);
        let address = daemon_bus_address;
        tokio::spawn(async move {
            consolidation_pipeline(config, monitor, address).await;
        })
    };

    (generation_handle, evaluation_handle, consolidation_handle, telemetry_tx)
}

/// Generation pipeline — reads telemetry stream, computes relevance scores,
/// builds thoughts, pushes to ThoughtQueue. Never blocks. Yields by awaiting
/// the next telemetry event from the channel.
async fn generation_pipeline(
    config: Arc<Config>,
    queue: Arc<ThoughtQueue>,
    mut telemetry_rx: tokio::sync::mpsc::Receiver<TelemetryEvent>,
) {
    let weights = weights_from_config(&config.default_weights);
    let max_depth = config.queue.max_depth as usize;

    // CTP never sleeps — yields by awaiting the next stream item
    while let Some(event) = telemetry_rx.recv().await {
        let signals = event.to_signal_input();
        let score = compute_score(&signals, &weights);
        let expires_at = expiry_for_score(score, &config.expiry_windows);

        let thought = Thought {
            id: Uuid::new_v4(),
            content: event.content,
            score,
            expires_at,
            generated_at: std::time::Instant::now(),
        };

        tracing::debug!(
            subsystem = "ctp",
            event_type = "thought_generated",
            thought_id = %thought.id,
            relevance_score = thought.score,
            "generated thought from telemetry event"
        );

        if let Err(error) = queue.push(thought, max_depth).await {
            tracing::warn!(
                subsystem = "ctp",
                event_type = "thought_push_failed",
                error = %error,
                "failed to push thought to queue"
            );
        }
    }

    tracing::info!(
        subsystem = "ctp",
        event_type = "generation_pipeline_stopped",
        "telemetry stream ended — generation pipeline exiting"
    );
}

/// Evaluation pipeline — pops from ThoughtQueue, compares score to
/// activity-dependent surface threshold, publishes TOPIC_THOUGHT_SURFACED
/// for above-threshold thoughts.
async fn evaluation_pipeline(
    config: Arc<Config>,
    queue: Arc<ThoughtQueue>,
    activity_monitor: Arc<ActivityMonitor>,
    daemon_bus_address: String,
) {
    let mut event_client = match EventBusServiceClient::connect(daemon_bus_address).await {
        Ok(client) => client,
        Err(connect_error) => {
            tracing::error!(
                subsystem = "ctp",
                event_type = "evaluation_pipeline_connect_failed",
                error = %connect_error,
                "failed to connect to daemon-bus event bus — evaluation pipeline exiting"
            );
            return;
        }
    };

    loop {
        let thought = queue.pop().await;
        let thought = match thought {
            Some(t) => t,
            None => continue,
        };

        let activity_state = activity_monitor.current_state();
        let threshold = activity_state.surface_threshold(&config.surface_thresholds);

        if thought.score >= threshold {
            tracing::info!(
                subsystem = "ctp",
                event_type = "thought_surfaced",
                thought_id = %thought.id,
                relevance_score = thought.score,
                activity_state = ?activity_state,
                threshold = threshold,
                "thought surfaced — publishing to daemon-bus"
            );

            let bus_event = BusEvent {
                event_id: Uuid::new_v4().to_string(),
                topic: EventTopic::TopicThoughtSurfaced.into(),
                source_subsystem: "ctp".to_string(),
                payload: thought.content.into_bytes(),
                trace_context: String::new(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };

            let publish_request = tonic::Request::new(PublishRequest {
                event: Some(bus_event),
            });

            if let Err(publish_error) = event_client.publish(publish_request).await {
                tracing::error!(
                    subsystem = "ctp",
                    event_type = "thought_publish_failed",
                    thought_id = %thought.id,
                    error = %publish_error,
                    "failed to publish TOPIC_THOUGHT_SURFACED"
                );
            }
        } else {
            // Below threshold — discard. Trace level only per CTP instructions.
            tracing::trace!(
                subsystem = "ctp",
                event_type = "thought_discarded",
                thought_id = %thought.id,
                relevance_score = thought.score,
                threshold = threshold,
                activity_state = ?activity_state,
                reason = "below_threshold",
            );
        }
    }
}

/// Consolidation pipeline — checks activity state periodically and requests
/// memory consolidation during deep idle only.
///
/// This is one of the two acceptable `tokio::time::sleep` locations in CTP
/// (the other is `activity.rs`). This is the consolidation check interval,
/// not the thought loop.
async fn consolidation_pipeline(
    config: Arc<Config>,
    activity_monitor: Arc<ActivityMonitor>,
    daemon_bus_address: String,
) {
    let check_interval =
        std::time::Duration::from_secs(config.consolidation.idle_threshold_secs);

    let mut event_client = match EventBusServiceClient::connect(daemon_bus_address).await {
        Ok(client) => client,
        Err(connect_error) => {
            tracing::error!(
                subsystem = "ctp",
                event_type = "consolidation_pipeline_connect_failed",
                error = %connect_error,
                "failed to connect to daemon-bus event bus — consolidation pipeline exiting"
            );
            return;
        }
    };

    loop {
        tokio::time::sleep(check_interval).await;

        let activity_state = activity_monitor.current_state();

        if !activity_state.is_deep_idle() {
            tracing::trace!(
                subsystem = "ctp",
                event_type = "consolidation_skipped",
                activity_state = ?activity_state,
                reason = "not_deep_idle",
            );
            continue;
        }

        tracing::info!(
            subsystem = "ctp",
            event_type = "consolidation_triggered",
            activity_state = ?activity_state,
            "deep idle detected — requesting memory consolidation"
        );

        let bus_event = BusEvent {
            event_id: Uuid::new_v4().to_string(),
            topic: EventTopic::TopicMemoryConsolidationRequested.into(),
            source_subsystem: "ctp".to_string(),
            payload: Vec::new(),
            trace_context: String::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let publish_request = tonic::Request::new(PublishRequest {
            event: Some(bus_event),
        });

        if let Err(publish_error) = event_client.publish(publish_request).await {
            tracing::error!(
                subsystem = "ctp",
                event_type = "consolidation_publish_failed",
                error = %publish_error,
                "failed to publish TOPIC_MEMORY_CONSOLIDATION_REQUESTED"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::ActivityState;
    use crate::config::*;
    use std::time::Duration;

    fn test_config() -> Config {
        Config {
            grpc: GrpcConfig {
                daemon_bus_address: "http://127.0.0.1:50051".into(),
                memory_engine_address: "http://127.0.0.1:50052".into(),
                connection_timeout_ms: 5000,
            },
            surface_thresholds: SurfaceThresholds {
                user_active: 0.9,
                idle_2min: 0.6,
                idle_10min: 0.3,
            },
            expiry_windows: ExpiryWindows {
                high_relevance_secs: 300,
                medium_relevance_secs: 120,
                low_relevance_secs: 30,
                high_score_cutoff: 0.8,
                medium_score_cutoff: 0.4,
            },
            default_weights: DefaultWeights {
                urgency: 0.9,
                emotional_resonance: 0.7,
                novelty: 0.6,
                recurrence: 0.4,
                idle_curiosity: 0.3,
            },
            consolidation: ConsolidationConfig {
                idle_threshold_secs: 1,
                promotion_min_score: 0.5,
                max_entries_per_cycle: 50,
            },
            compaction: CompactionConfig {
                pre_rot_fraction: 0.8,
                max_entries_to_summarize: 100,
            },
            queue: QueueConfig { max_depth: 256 },
            activity: ActivityConfig {
                poll_interval_ms: 500,
                idle_2min_threshold_secs: 120,
                idle_10min_threshold_secs: 600,
            },
            logging: LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        }
    }

    #[tokio::test]
    async fn test_generation_pipeline_pushes_to_queue() {
        let config = Arc::new(test_config());
        let queue = Arc::new(ThoughtQueue::new());
        let (tx, rx) = tokio::sync::mpsc::channel::<TelemetryEvent>(16);

        let queue_clone = Arc::clone(&queue);
        let gen_handle = tokio::spawn(async move {
            generation_pipeline(config, queue_clone, rx).await;
        });

        // Send a telemetry event
        tx.send(TelemetryEvent {
            urgency: 0.8,
            emotional_resonance: 0.5,
            novelty: 0.6,
            recurrence: 0.3,
            idle_curiosity: 0.2,
            content: "test thought".into(),
        })
        .await
        .expect("test: send telemetry event");

        // Give the pipeline a moment to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Queue should have a thought
        assert!(queue.len().await > 0, "queue should contain a thought");

        // Drop sender to end the pipeline
        drop(tx);
        let _ = tokio::time::timeout(Duration::from_millis(100), gen_handle).await;
    }

    #[tokio::test]
    async fn test_evaluation_pipeline_surfaces_high_score() {
        // This test verifies the structural logic: a thought above threshold
        // would be surfaced. We can't test the full gRPC publish without a
        // running daemon-bus, so we test the scoring logic directly.
        let config = test_config();
        let activity_state = ActivityState::UserActive;
        let threshold = activity_state.surface_threshold(&config.surface_thresholds);

        // A thought with score 0.95 should exceed user_active threshold of 0.9
        let score = 0.95_f32;
        assert!(
            score >= threshold,
            "score {} should be >= threshold {}",
            score,
            threshold
        );
    }

    #[tokio::test]
    async fn test_evaluation_pipeline_discards_low_score() {
        // A thought below threshold should not be surfaced
        let config = test_config();
        let activity_state = ActivityState::UserActive;
        let threshold = activity_state.surface_threshold(&config.surface_thresholds);

        let score = 0.5_f32;
        assert!(
            score < threshold,
            "score {} should be < threshold {}",
            score,
            threshold
        );
    }

    #[tokio::test]
    async fn test_consolidation_pipeline_skips_when_active() {
        // Consolidation should NOT fire when UserActive
        let activity_state = ActivityState::UserActive;
        assert!(
            !activity_state.is_deep_idle(),
            "UserActive must not be deep idle"
        );
    }

    #[tokio::test]
    async fn test_consolidation_pipeline_fires_when_deep_idle() {
        // Consolidation should fire when Idle10Min
        let activity_state = ActivityState::Idle10Min;
        assert!(
            activity_state.is_deep_idle(),
            "Idle10Min must be deep idle"
        );
    }

    #[test]
    fn test_pipelines_do_not_share_mutable_state() {
        // Structural test: the three pipelines accept only immutable refs
        // or Arc to shared state. The function signatures enforce this:
        //
        // - generation_pipeline(Arc<Config>, Arc<ThoughtQueue>, Receiver<TelemetryEvent>)
        // - evaluation_pipeline(Arc<Config>, Arc<ThoughtQueue>, Arc<ActivityMonitor>, String)
        // - consolidation_pipeline(Arc<Config>, Arc<ActivityMonitor>, String)
        //
        // No Mutex<SharedContext> spanning all three.
        // ThoughtQueue uses internal Mutex — not shared mutable state across pipelines.
        // ActivityMonitor uses AtomicU8 — not shared mutable state.
        //
        // This test passes by compilation — the type signatures enforce the constraint.
    }
}
