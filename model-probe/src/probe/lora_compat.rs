//! LoRA compatibility probe — structural metadata check, not inference.
//!
//! Checks the model's architecture metadata against the configured list of
//! LoRA-compatible architecture families. This probe does NOT run any model
//! inference — it inspects model file metadata only.
//!
//! The compatible architecture list comes from config, never hardcoded.
//! Known compatible families at time of writing: llama, mistral, qwen,
//! qwen2, gemma, gemma2, phi, phi3.

use std::time::Instant;

use crate::config::ModelProbeConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};

/// Result of the LoRA compatibility probe.
#[derive(Debug, Clone)]
pub struct LoraCompatResult {
    /// Whether the model's architecture supports LoRA adapter training.
    pub lora_compatible: bool,
    /// The detected architecture family string, if available.
    pub detected_architecture: Option<String>,
    /// Wall-clock duration of the probe in milliseconds.
    pub duration_ms: u64,
}

/// Run the LoRA compatibility probe against the active model's metadata.
///
/// This is a structural check — it reads model architecture metadata and
/// compares against the configured list of LoRA-compatible families.
/// No inference call is made.
///
/// # Stub implementation
///
/// Actual metadata extraction from the GGUF file via llama-cpp-rs is not yet
/// wired. This stub returns `lora_compatible = false` and logs that it is
/// unimplemented. Once the model backend is integrated, this function will:
///
/// 1. Load model metadata from the GGUF file header
/// 2. Extract the architecture field using `config.probes.lora_compat.architecture_metadata_key`
/// 3. Normalize to lowercase and compare against `config.model.lora_compatible_architectures`
/// 4. Return the match result
pub async fn run(config: &ModelProbeConfig) -> SenaResult<LoraCompatResult> {
    let start = Instant::now();

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "lora_compat",
        event_type = "probe_started",
        model_path = %config.model.model_path,
        metadata_key = %config.probes.lora_compat.architecture_metadata_key,
        "LoRA compatibility probe starting"
    );

    // TODO(implementation): Replace with actual GGUF metadata extraction via llama-cpp-rs.
    //
    // Pseudocode for the real implementation:
    //
    //   let metadata = model.metadata();
    //   let architecture = metadata.get(&config.probes.lora_compat.architecture_metadata_key)
    //       .map(|v| v.to_lowercase());
    //   let lora_compatible = architecture
    //       .as_ref()
    //       .map(|arch| config.model.lora_compatible_architectures.iter().any(|a| arch.contains(&a.to_lowercase())))
    //       .unwrap_or(false);

    let detected_architecture = extract_architecture_stub(&config.model.model_path)?;

    let lora_compatible = match &detected_architecture {
        Some(architecture) => is_architecture_compatible(
            architecture,
            &config.model.lora_compatible_architectures,
        ),
        None => false,
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    tracing::info!(
        subsystem = "model_probe",
        probe_name = "lora_compat",
        event_type = "probe_completed",
        lora_compatible = lora_compatible,
        detected_architecture = ?detected_architecture,
        duration_ms = duration_ms,
        "LoRA compatibility probe completed"
    );

    Ok(LoraCompatResult {
        lora_compatible,
        detected_architecture,
        duration_ms,
    })
}

/// Stub architecture extraction — returns None until real GGUF parsing is wired.
///
/// Validates that the model path is non-empty so wiring failures surface early.
fn extract_architecture_stub(model_path: &str) -> SenaResult<Option<String>> {
    if model_path.is_empty() {
        return Err(SenaError::new(
            ErrorCode::ProbeFailed,
            "lora_compat probe: model_path is empty",
        ));
    }

    tracing::warn!(
        subsystem = "model_probe",
        probe_name = "lora_compat",
        event_type = "probe_stubbed",
        "LoRA compat probe is stubbed — returning None for architecture until llama-cpp-rs metadata extraction is integrated"
    );

    Ok(None)
}

/// Check whether a detected architecture is in the configured compatible list.
///
/// Comparison is case-insensitive. The detected architecture string is checked
/// for whether it contains any of the compatible family names as a substring,
/// because model metadata may include version suffixes (e.g. "llama3" should
/// match the "llama" family).
pub fn is_architecture_compatible(
    detected_architecture: &str,
    compatible_architectures: &[String],
) -> bool {
    let detected_lower = detected_architecture.to_lowercase();

    compatible_architectures
        .iter()
        .any(|family| detected_lower.contains(&family.to_lowercase()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn compatible_list() -> Vec<String> {
        vec![
            "llama".to_string(),
            "mistral".to_string(),
            "qwen".to_string(),
            "qwen2".to_string(),
            "gemma".to_string(),
            "gemma2".to_string(),
            "phi".to_string(),
            "phi3".to_string(),
        ]
    }

    #[test]
    fn exact_match_is_compatible() {
        assert!(is_architecture_compatible("llama", &compatible_list()));
    }

    #[test]
    fn case_insensitive_match() {
        assert!(is_architecture_compatible("LLAMA", &compatible_list()));
        assert!(is_architecture_compatible("Mistral", &compatible_list()));
    }

    #[test]
    fn versioned_architecture_matches_family() {
        // "llama3" contains "llama" as substring
        assert!(is_architecture_compatible("llama3", &compatible_list()));
        assert!(is_architecture_compatible("phi3.5", &compatible_list()));
        assert!(is_architecture_compatible("qwen2.5", &compatible_list()));
    }

    #[test]
    fn unknown_architecture_not_compatible() {
        assert!(!is_architecture_compatible("mamba", &compatible_list()));
        assert!(!is_architecture_compatible("rwkv", &compatible_list()));
        assert!(!is_architecture_compatible("gpt2", &compatible_list()));
    }

    #[test]
    fn empty_architecture_not_compatible() {
        assert!(!is_architecture_compatible("", &compatible_list()));
    }

    #[test]
    fn empty_compatible_list_never_matches() {
        assert!(!is_architecture_compatible("llama", &[]));
    }

    #[test]
    fn stub_rejects_empty_model_path() {
        let result = extract_architecture_stub("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::ProbeFailed);
    }

    #[test]
    fn stub_returns_none_for_valid_path() {
        let result = extract_architecture_stub("models/default.gguf");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
