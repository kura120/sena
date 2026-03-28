//! Capability constants for the inference subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what features this subsystem provides.
//!
//! Format:
//! - `capability_name` for operational capabilities
//! - `capability_name:degraded` for degraded capabilities

/// Model loading and unloading
pub const MODEL_LOADING: &str = "model_loading";

/// Text completion generation
pub const TEXT_COMPLETION: &str = "text_completion";

/// Streaming completion support
pub const STREAMING_COMPLETION: &str = "streaming_completion";

/// Model registry management
pub const MODEL_REGISTRY: &str = "model_registry";

/// List available models
pub const LIST_MODELS: &str = "list_models";

/// Load a specific model
pub const LOAD_MODEL: &str = "load_model";

/// Unload current model
pub const UNLOAD_MODEL: &str = "unload_model";

/// Returns the list of capabilities the inference subsystem currently provides.
///
/// This is called when signaling INFERENCE_READY to daemon-bus.
pub fn get_capabilities() -> Vec<String> {
    vec![
        MODEL_LOADING.to_string(),
        TEXT_COMPLETION.to_string(),
        STREAMING_COMPLETION.to_string(),
        MODEL_REGISTRY.to_string(),
    ]
}

/// Returns capabilities when in degraded mode (e.g., partial OOM recovery).
pub fn get_degraded_capabilities() -> Vec<String> {
    vec![
        format!("{}:degraded", TEXT_COMPLETION),
        MODEL_LOADING.to_string(),
    ]
}
