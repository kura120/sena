//! Capability constants for the reactive-loop subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what features this subsystem provides.

/// Handle user messages through the full conversation flow
pub const MESSAGE_HANDLING: &str = "message_handling";

/// Route inference requests to the inference subsystem
pub const INFERENCE_ROUTING: &str = "inference_routing";

/// Publish events to daemon-bus event bus
pub const EVENT_PUBLISHING: &str = "event_publishing";

/// Returns the list of capabilities the reactive-loop subsystem currently provides.
///
/// This is called when signaling REACTIVE_LOOP_READY to daemon-bus.
pub fn get_capabilities() -> Vec<String> {
    vec![
        MESSAGE_HANDLING.to_string(),
        INFERENCE_ROUTING.to_string(),
        EVENT_PUBLISHING.to_string(),
    ]
}
