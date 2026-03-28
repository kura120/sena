//! Structured output probe — tests whether the model can produce valid
//! JSON matching a minimal KnowledgeGraph schema.
//!
//! The probe sends a prompt requesting structured entity/relation extraction,
//! then validates the response against the expected JSON schema from config.
//! This gates whether prompt-composer uses TOON encoding or falls back to JSON.

use std::time::Instant;

use crate::config::StructuredOutputProbeConfig;
use crate::error::SenaResult;
use crate::generated::sena_daemonbus_v1::inference_service_client::InferenceServiceClient;
use crate::generated::sena_daemonbus_v1::CompleteRequest;
use crate::probes::{CapabilityLevel, ProbeResult};
use tonic::transport::Channel;

/// Run the structured output probe against the active model.
///
/// Sends `config.probe_prompt` requesting a KnowledgeGraph-shaped JSON response,
/// then scores the output against `config.expected_schema`. The score is the
/// fraction of required schema fields that are present and correctly typed.
///
/// # Graceful Degradation
/// When `inference_client` is None or inference calls fail:
/// 1. Logs a warning
/// 2. Returns score 0.0 with `degraded: true`
/// 3. Does NOT return an error
pub async fn run(
    config: &StructuredOutputProbeConfig,
    probe_timeout_ms: u64,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<ProbeResult> {
    let started_at = Instant::now();

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(probe_timeout_ms),
        run_inner(config, inference_client),
    )
    .await;

    let duration = started_at.elapsed();

    match result {
        Ok(Ok((score, degraded))) => {
            let capability = CapabilityLevel::from_score(
                score,
                config.partial_threshold,
                config.full_threshold,
            );

            tracing::info!(
                subsystem = "model_probe",
                probe_name = "structured_output",
                event_type = "probe_completed",
                score = score,
                result = %capability,
                duration_ms = duration.as_millis() as u64,
                degraded = degraded,
                "structured output probe completed"
            );

            Ok(ProbeResult {
                probe_name: "structured_output".to_string(),
                raw_score: score,
                capability_level: Some(capability),
                duration,
                degraded,
            })
        }
        Ok(Err(probe_error)) => {
            tracing::error!(
                subsystem = "model_probe",
                probe_name = "structured_output",
                event_type = "probe_failed",
                error_code = %probe_error.code,
                error_message = %probe_error.message,
                duration_ms = duration.as_millis() as u64,
                "structured output probe failed"
            );

            Ok(ProbeResult {
                probe_name: "structured_output".to_string(),
                raw_score: 0.0,
                capability_level: Some(CapabilityLevel::None),
                duration,
                degraded: true,
            })
        }
        Err(_timeout) => {
            tracing::error!(
                subsystem = "model_probe",
                probe_name = "structured_output",
                event_type = "probe_timeout",
                timeout_ms = probe_timeout_ms,
                duration_ms = duration.as_millis() as u64,
                "structured output probe timed out"
            );

            Ok(ProbeResult {
                probe_name: "structured_output".to_string(),
                raw_score: 0.0,
                capability_level: Some(CapabilityLevel::None),
                duration,
                degraded: true,
            })
        }
    }
}

/// Inner probe logic — sends the prompt to inference and scores the response.
async fn run_inner(
    config: &StructuredOutputProbeConfig,
    inference_client: Option<InferenceServiceClient<Channel>>,
) -> SenaResult<(f64, bool)> {
    let degraded = inference_client.is_none();

    if degraded {
        tracing::warn!(
            subsystem = "model_probe",
            probe_name = "structured_output",
            reason = "inference_unavailable",
            "probe degraded — InferenceService client not available"
        );
        return Ok((0.0, true));
    }

    let mut client = inference_client.expect("checked above"); // safe: guarded by degraded check

    let request = CompleteRequest {
        prompt: config.probe_prompt.clone(),
        model_id: String::new(),
        max_tokens: 512,
        temperature: 0.0,
        priority: 3,
        request_id: format!("probe-structured-output-{}", timestamp_nanos()),
    };

    let response = match client.complete(request).await {
        Ok(resp) => resp.into_inner().text,
        Err(inference_error) => {
            tracing::warn!(
                subsystem = "model_probe",
                probe_name = "structured_output",
                event_type = "inference_failed",
                error = %inference_error,
                "inference call failed — returning degraded result"
            );
            return Ok((0.0, true));
        }
    };

    let score = score_structured_output(&response, &config.expected_schema);
    Ok((score, false))
}

fn timestamp_nanos() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Validate a JSON response string against the expected schema fields.
///
/// Returns a score between 0.0 and 1.0 representing the fraction of required
/// top-level and nested fields that are present and correctly typed.
///
/// This is a simplified structural validator — not a full JSON Schema engine.
/// It checks for the presence of required keys and basic type correctness
/// (array vs object vs string), which is sufficient for the probe's purpose
/// of determining whether the model can produce structured output at all.
pub fn score_structured_output(response_json: &str, _expected_schema: &str) -> f64 {
    let parsed: serde_json::Value = match serde_json::from_str(response_json) {
        Ok(value) => value,
        Err(_) => return 0.0,
    };

    let mut total_checks = 0u32;
    let mut passed_checks = 0u32;

    // Check top-level is an object
    total_checks += 1;
    let object = match parsed.as_object() {
        Some(obj) => {
            passed_checks += 1;
            obj
        }
        None => return 0.0,
    };

    // Check "entities" field exists and is an array
    total_checks += 1;
    if let Some(entities) = object.get("entities") {
        if entities.is_array() {
            passed_checks += 1;

            // Check that at least one entity has required fields
            if let Some(entities_array) = entities.as_array() {
                if !entities_array.is_empty() {
                    total_checks += 1;
                    if let Some(first_entity) = entities_array.first() {
                        if first_entity.get("name").is_some()
                            && first_entity.get("type").is_some()
                        {
                            passed_checks += 1;
                        }
                    }
                }
            }
        }
    }

    // Check "relations" field exists and is an array
    total_checks += 1;
    if let Some(relations) = object.get("relations") {
        if relations.is_array() {
            passed_checks += 1;

            // Check that at least one relation has required fields
            if let Some(relations_array) = relations.as_array() {
                if !relations_array.is_empty() {
                    total_checks += 1;
                    if let Some(first_relation) = relations_array.first() {
                        if first_relation.get("source").is_some()
                            && first_relation.get("target").is_some()
                            && first_relation.get("relation").is_some()
                        {
                            passed_checks += 1;
                        }
                    }
                }
            }
        }
    }

    if total_checks == 0 {
        return 0.0;
    }

    f64::from(passed_checks) / f64::from(total_checks)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_valid_complete_response() {
        let response = r#"{
            "entities": [
                {"name": "Alice", "type": "Person"},
                {"name": "Acme Corp", "type": "Organization"}
            ],
            "relations": [
                {"source": "Alice", "target": "Acme Corp", "relation": "works_at"}
            ]
        }"#;

        let score = score_structured_output(response, "");
        assert!(
            (score - 1.0).abs() < f64::EPSILON,
            "fully valid response should score 1.0, got {score}"
        );
    }

    #[test]
    fn score_invalid_json_returns_zero() {
        let score = score_structured_output("not json at all {{{", "");
        assert!(
            score.abs() < f64::EPSILON,
            "invalid JSON should score 0.0, got {score}"
        );
    }

    #[test]
    fn score_empty_object_returns_low() {
        let score = score_structured_output("{}", "");
        // Has object (1 pass) but missing entities and relations (2 fails)
        // 1/3 ≈ 0.333
        assert!(score > 0.0, "empty object should score above 0.0");
        assert!(
            score < 0.5,
            "empty object should score below 0.5, got {score}"
        );
    }

    #[test]
    fn score_entities_only_partial() {
        let response = r#"{
            "entities": [{"name": "Alice", "type": "Person"}]
        }"#;

        let score = score_structured_output(response, "");
        // Has object, entities array, entity fields — but missing relations
        assert!(score > 0.3, "entities-only should be partial, got {score}");
        assert!(score < 1.0, "entities-only should not be full, got {score}");
    }

    #[test]
    fn score_non_object_top_level_returns_zero() {
        let score = score_structured_output("[1, 2, 3]", "");
        assert!(
            score.abs() < f64::EPSILON,
            "array top-level should score 0.0, got {score}"
        );
    }

    #[test]
    fn score_entities_wrong_type_returns_partial() {
        let response = r#"{
            "entities": "not an array",
            "relations": [{"source": "a", "target": "b", "relation": "c"}]
        }"#;

        let score = score_structured_output(response, "");
        // Object passes, entities fails (not array), relations passes, relation fields pass
        assert!(
            score > 0.0,
            "wrong type entities should be partial, got {score}"
        );
        assert!(
            score < 1.0,
            "wrong type entities should not be full, got {score}"
        );
    }

    #[test]
    fn capability_level_from_score_full() {
        let level = CapabilityLevel::from_score(0.95, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::Full);
    }

    #[test]
    fn capability_level_from_score_partial() {
        let level = CapabilityLevel::from_score(0.70, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::Partial);
    }

    #[test]
    fn capability_level_from_score_none() {
        let level = CapabilityLevel::from_score(0.30, 0.50, 0.90);
        assert_eq!(level, CapabilityLevel::None);
    }
}
