//! Event bridge — subscribes to daemon-bus events and feeds the CTP telemetry channel.
//!
//! This bridges daemon-bus events into TelemetryEvents for the generation pipeline.
//! In Milestone A, this is the primary source of thought triggers. Real OS telemetry
//! (keyboard, mouse, app focus) is wired in Phase 2.

use crate::config::{EventBridgeConfig, SignalWeightsConfig};
use crate::pipelines::TelemetryEvent;
use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient,
    EventTopic, SubscribeRequest,
};
use tokio::sync::mpsc::Sender;

const SUBSYSTEM_ID: &str = "ctp";

/// Spawn the event bridge as an async task.
/// Subscribes to daemon-bus events and converts matching events to TelemetryEvents.
pub fn spawn_event_bridge(
    config: EventBridgeConfig,
    daemon_bus_address: String,
    telemetry_tx: Sender<TelemetryEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_event_bridge(config, daemon_bus_address, telemetry_tx).await;
    })
}

async fn run_event_bridge(
    config: EventBridgeConfig,
    daemon_bus_address: String,
    telemetry_tx: Sender<TelemetryEvent>,
) {
    // Connect to daemon-bus
    let mut client = match EventBusServiceClient::connect(daemon_bus_address.clone()).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "event_bridge_connect_failed",
                error = %e,
                "failed to connect to daemon-bus — event bridge exiting"
            );
            return;
        }
    };

    // Subscribe to relevant topics
    let topics = vec![
        EventTopic::TopicUserMessageReceived as i32,
        EventTopic::TopicUserMessageResponse as i32,
        EventTopic::TopicMemoryWriteCompleted as i32,
    ];
    
    let subscribe_request = tonic::Request::new(SubscribeRequest {
        topics,
        subscriber_id: format!("{}_event_bridge", SUBSYSTEM_ID),
    });

    let mut stream = match client.subscribe(subscribe_request).await {
        Ok(response) => response.into_inner(),
        Err(e) => {
            tracing::error!(
                subsystem = SUBSYSTEM_ID,
                event_type = "event_bridge_subscribe_failed",
                error = %e,
                "failed to subscribe to daemon-bus events — event bridge exiting"
            );
            return;
        }
    };

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "event_bridge_started",
        "event bridge subscribed to daemon-bus events"
    );

    // Process events
    while let Ok(Some(bus_event)) = stream.message().await {
        let topic = EventTopic::try_from(bus_event.topic).unwrap_or(EventTopic::Unspecified);
        
        let telemetry_event = match topic {
            EventTopic::TopicUserMessageReceived => {
                let content = String::from_utf8_lossy(&bus_event.payload).to_string();
                Some(create_telemetry_event(&config.user_message, 
                    format!("User said: {}", truncate_content(&content, 200))))
            }
            EventTopic::TopicUserMessageResponse => {
                let content = String::from_utf8_lossy(&bus_event.payload).to_string();
                Some(create_telemetry_event(&config.user_response,
                    format!("Responded: {}", truncate_content(&content, 200))))
            }
            EventTopic::TopicMemoryWriteCompleted => {
                let content = String::from_utf8_lossy(&bus_event.payload).to_string();
                Some(create_telemetry_event(&config.memory_write,
                    format!("Memory updated: {}", truncate_content(&content, 100))))
            }
            _ => None,
        };

        if let Some(event) = telemetry_event {
            tracing::debug!(
                subsystem = SUBSYSTEM_ID,
                event_type = "event_bridge_telemetry_created",
                source_topic = ?topic,
                content_length = event.content.len(),
                "created telemetry event from daemon-bus event"
            );
            
            if let Err(e) = telemetry_tx.send(event).await {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_bridge_send_failed",
                    error = %e,
                    "failed to send telemetry event — generation pipeline may have stopped"
                );
                break;
            }
        }
    }

    tracing::info!(
        subsystem = SUBSYSTEM_ID,
        event_type = "event_bridge_stopped",
        "event bridge stream ended"
    );
}

fn create_telemetry_event(weights: &SignalWeightsConfig, content: String) -> TelemetryEvent {
    TelemetryEvent {
        urgency: weights.urgency,
        emotional_resonance: weights.emotional_resonance,
        novelty: weights.novelty,
        recurrence: weights.recurrence,
        idle_curiosity: weights.idle_curiosity,
        content,
    }
}

fn truncate_content(content: &str, max_len: usize) -> &str {
    if content.len() <= max_len {
        content
    } else {
        // Find a safe char boundary
        let mut end = max_len;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        &content[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_telemetry_event_from_weights() {
        let weights = SignalWeightsConfig {
            urgency: 0.3,
            emotional_resonance: 0.5,
            novelty: 0.8,
            recurrence: 0.2,
            idle_curiosity: 0.1,
        };

        let event = create_telemetry_event(&weights, "test content".to_string());
        assert!((event.urgency - 0.3).abs() < f32::EPSILON);
        assert!((event.emotional_resonance - 0.5).abs() < f32::EPSILON);
        assert!((event.novelty - 0.8).abs() < f32::EPSILON);
        assert!((event.recurrence - 0.2).abs() < f32::EPSILON);
        assert!((event.idle_curiosity - 0.1).abs() < f32::EPSILON);
        assert_eq!(event.content, "test content");
    }

    #[test]
    fn test_truncate_content_short() {
        assert_eq!(truncate_content("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_content_long() {
        let long_text = "a".repeat(300);
        let result = truncate_content(&long_text, 200);
        assert_eq!(result.len(), 200);
    }

    #[test]
    fn test_truncate_content_unicode_boundary() {
        // Test with emoji which is multiple bytes
        let text = "hello 👋 world";
        let result = truncate_content(text, 10);
        // Should truncate safely at a char boundary
        assert!(result.len() <= 10);
        assert!(text.starts_with(result));
    }
}
