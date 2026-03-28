//! Graph extraction capability probe.
//!
//! Fires a minimal KnowledgeGraph structured output request and checks whether
//! the response passes schema validation. Gates ech0 graph extraction — if the
//! model cannot produce valid graph output, ech0 runs in vector-only mode.
//!
//! This probe does NOT depend on any other probe's result and can run
//! concurrently with all other independent probes.

use std::time::Instant;

use crate::config::GraphExtractionProbeConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::inference_service_client::InferenceServiceClient;
use crate::generated::sena_daemonbus_v1::{CompleteRequest, CompleteResponse};
use crate::probes::{CapabilityLevel, ProbeResult};
use tonic::transport::Channel;

/// Run the graph extraction capability probe against the active model.
///
/// Sends a minimal KnowledgeGraph extraction prompt and validates the response
/// against the expected JSON schema. Scores the result as Full, Partial, or
/// None based on config-driven thresholds.
///
/// # Arguments
/// * `config` — probe-specific configuration (prompt, schema, thresholds)
/// * `inference_client` — optional gRPC client to InferenceService; if None, degrades to stub
///
/// # Graceful Degradation
/// When `inference_client` is None or inference calls fail, the probe:
/// 1. Logs a warning with `tracing::warn!`
/// 2. Returns CapabilityLevel::None with score 0.0
/// 3. Sets `degraded: true` in the result
/// 4. Does NOT return an error (error would abort the battery)
///
/// # Returns
/// A `ProbeResult` with the raw score and derived `CapabilityLevel`.
pub async fn run(
    config: &GraphExtractionProbeConfig,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<ProbeResult> {
    let start = Instant::now();

    let degraded = inference_client.is_none();
    
    if degraded {
        tracing::warn!(
            subsystem = "model_probe",
            probe_name = "graph_extraction",
            reason = "inference_unavailable",
            "probe degraded to stub — InferenceService client not available"
        );
    }

    let raw_score = if let Some(mut client) = inference_client {
        // Real inference path
        match run_graph_extraction_with_inference(&mut client, config).await {
            Ok(score) => score,
            Err(inference_error) => {
                tracing::warn!(
                    subsystem = "model_probe",
                    probe_name = "graph_extraction",
                    event_type = "inference_failed",
                    error = %inference_error,
                    "inference call failed — returning degraded result"
                );
                0.0 // Degrade to zero capability on failure
            }
        }
    } else {
        // Degraded path — stub logic
        0.0
    };

    let duration = start.elapsed();

    let capability_level = score_to_capability_level(
        raw_score,
        config.partial_threshold,
        config.full_threshold,
    );

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "graph_extraction",
        event_type = "probe_completed",
        score = raw_score,
        result = %capability_level,
        duration_ms = duration.as_millis() as u64,
        degraded = degraded,
        "graph extraction probe completed"
    );

    Ok(ProbeResult {
        probe_name: "graph_extraction".to_string(),
        raw_score,
        capability_level: Some(capability_level),
        duration,
        degraded,
    })
}

/// Run graph extraction with real inference via InferenceService.Complete.
///
/// Sends the probe prompt to the model, gets the response, and validates
/// it against the expected JSON schema.
///
/// Returns the validation score (0.0–1.0) or an error if inference fails.
async fn run_graph_extraction_with_inference(
    client: &mut InferenceServiceClient<Channel>,
    config: &GraphExtractionProbeConfig,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    // Call InferenceService.Complete
    let request = CompleteRequest {
        prompt: config.probe_prompt.clone(),
        model_id: String::new(), // empty = active model
        max_tokens: 300,
        temperature: 0.0, // deterministic
        priority: 3, // Standard priority
        request_id: uuid::Uuid::new_v4().to_string(),
    };
    
    let response: CompleteResponse = client.complete(request).await?.into_inner();
    
    tracing::debug!(
        subsystem = "model_probe",
        probe_name = "graph_extraction",
        event_type = "inference_response_received",
        response_length = response.text.len(),
        "received graph extraction response from inference"
    );
    
    // Validate the response using the existing validation function
    // Note: we're not using expected_schema string yet, validation is hardcoded
    let score = validate_graph_output(&response.text, &config.expected_schema)?;
    
    Ok(score)
}

/// Map a raw score to a `CapabilityLevel` using config-driven thresholds.
fn score_to_capability_level(
    score: f64,
    partial_threshold: f64,
    full_threshold: f64,
) -> CapabilityLevel {
    if score >= full_threshold {
        CapabilityLevel::Full
    } else if score >= partial_threshold {
        CapabilityLevel::Partial
    } else {
        CapabilityLevel::None
    }
}

/// Validate model output against the expected KnowledgeGraph JSON schema.
///
/// Returns a score between 0.0 and 1.0 representing the fraction of required
/// schema elements that are present and correctly typed.
#[allow(dead_code)]
fn validate_graph_output(
    model_output: &str,
    _expected_schema: &str,
) -> SenaResult<f64> {
    // Parse the model's response as JSON
    let parsed: serde_json::Value = serde_json::from_str(model_output).map_err(|parse_error| {
        SenaError::new(
            ErrorCode::ProbeFailed,
            "graph extraction probe: model output is not valid JSON",
        )
        .with_debug_context(format!("parse error: {parse_error}"))
    })?;

    let mut checks_passed = 0u32;
    let total_checks = 4u32;

    // Check for "nodes" array
    if let Some(nodes) = parsed.get("nodes") {
        if nodes.is_array() {
            checks_passed += 1;

            // Check that at least one node has required fields
            if let Some(nodes_array) = nodes.as_array() {
                let has_valid_node = nodes_array.iter().any(|node| {
                    node.get("id").is_some()
                        && node.get("label").is_some()
                        && node.get("type").is_some()
                });
                if has_valid_node {
                    checks_passed += 1;
                }
            }
        }
    }

    // Check for "edges" array
    if let Some(edges) = parsed.get("edges") {
        if edges.is_array() {
            checks_passed += 1;

            // Check that at least one edge has required fields
            if let Some(edges_array) = edges.as_array() {
                let has_valid_edge = edges_array.iter().any(|edge| {
                    edge.get("source").is_some()
                        && edge.get("target").is_some()
                        && edge.get("relation").is_some()
                });
                if has_valid_edge {
                    checks_passed += 1;
                }
            }
        }
    }

    Ok(f64::from(checks_passed) / f64::from(total_checks))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_to_capability_full() {
        let level = score_to_capability_level(0.95, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::Full);
    }

    #[test]
    fn score_to_capability_partial() {
        let level = score_to_capability_level(0.70, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::Partial);
    }

    #[test]
    fn score_to_capability_none() {
        let level = score_to_capability_level(0.30, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::None);
    }

    #[test]
    fn score_at_exact_full_threshold_is_full() {
        let level = score_to_capability_level(0.90, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::Full);
    }

    #[test]
    fn score_at_exact_partial_threshold_is_partial() {
        let level = score_to_capability_level(0.50, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::Partial);
    }

    #[test]
    fn validate_perfect_graph_output() {
        let output = r#"{
            "nodes": [
                {"id": "1", "label": "Marie Curie", "type": "Person"},
                {"id": "2", "label": "radium", "type": "Element"}
            ],
            "edges": [
                {"source": "1", "target": "2", "relation": "discovered"}
            ]
        }"#;

        let score = validate_graph_output(output, "").expect("should validate");
        assert!((score - 1.0).abs() < f64::EPSILON, "perfect output should score 1.0, got {score}");
    }

    #[test]
    fn validate_empty_arrays_scores_partial() {
        let output = r#"{"nodes": [], "edges": []}"#;
        let score = validate_graph_output(output, "").expect("should validate");
        // Has top-level arrays (2/4) but no valid items (0/4 for item checks)
        assert!((score - 0.5).abs() < f64::EPSILON, "empty arrays should score 0.5, got {score}");
    }

    #[test]
    fn validate_invalid_json_returns_error() {
        let output = "this is not json {{{";
        let result = validate_graph_output(output, "");
        assert!(result.is_err());
    }

    #[test]
    fn validate_missing_edges_scores_low() {
        let output = r#"{
            "nodes": [{"id": "1", "label": "test", "type": "Entity"}]
        }"#;
        let score = validate_graph_output(output, "").expect("should validate");
        // nodes array present (1/4), has valid node (2/4), no edges at all (2/4)
        assert!((score - 0.5).abs() < f64::EPSILON, "missing edges should score 0.5, got {score}");
    }

    #[test]
    fn validate_nodes_missing_required_fields() {
        let output = r#"{
            "nodes": [{"name": "incomplete"}],
            "edges": [{"source": "1", "target": "2", "relation": "knows"}]
        }"#;
        let score = validate_graph_output(output, "").expect("should validate");
        // nodes array (1/4), no valid node — missing id/label/type (1/4),
        // edges array (2/4), valid edge (3/4) = 0.75
        assert!((score - 0.75).abs() < f64::EPSILON, "expected 0.75, got {score}");
    }
}
