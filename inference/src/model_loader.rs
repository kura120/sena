//! Model loader — wraps all llama-cpp-rs calls inside spawn_blocking.
//! This is the only module that directly calls llama-cpp-2 APIs.

use std::path::Path;
use std::sync::Arc;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;

use crate::error::InferenceError;

/// Opaque handle to a loaded LLM model.
/// Wraps llama-cpp-2's LlamaModel in an Arc for Send + shared ownership.
#[allow(dead_code)] // Fields will be used in future phases for context creation and inference execution
pub struct ModelHandle {
    pub(crate) model: Arc<LlamaModel>,
    pub(crate) backend: Arc<LlamaBackend>,
    pub(crate) model_id: String,
    pub(crate) context_length: u32,
}

// SAFETY: LlamaModel and LlamaBackend are thread-safe behind Arc.
// The llama-cpp-2 crate documents that model and backend are safe to
// share across threads when accessed through proper synchronization.
// This follows the exact pattern from memory-engine/src/embedder.rs.
unsafe impl Send for ModelHandle {}
unsafe impl Sync for ModelHandle {}

/// Estimate VRAM usage in MB for a model at the given path.
/// Uses a file-size-based heuristic — models are typically loaded into VRAM
/// with some overhead for context and KV cache.
///
/// # Errors
/// Returns `InferenceError::ModelLoad` if the file does not exist or cannot be read.
pub fn estimate_vram_mb(model_path: &Path) -> Result<u32, InferenceError> {
    // FIXME(inference): replace file-size heuristic with actual gguf metadata parsing for accurate VRAM estimation
    let metadata = std::fs::metadata(model_path).map_err(|err| InferenceError::ModelLoad {
        model_id: model_path.display().to_string(),
        reason: format!("Failed to read model file metadata: {err}"),
    })?;

    let size_bytes = metadata.len();
    let size_mb = size_bytes / (1024 * 1024);

    // Apply 1.2x multiplier for overhead (context, KV cache, etc.)
    let estimated_vram_mb = ((size_mb as f64) * 1.2) as u32;

    Ok(estimated_vram_mb)
}

/// Load a model from disk using llama-cpp-2.
/// All blocking llama-cpp-rs calls are executed inside `spawn_blocking`.
///
/// # Arguments
/// * `model_id` - Identifier for this model instance
/// * `model_path` - Filesystem path to the .gguf model file
/// * `gpu_layers` - Number of layers to offload to GPU
/// * `context_length` - Context window size in tokens
/// * `backend` - Shared LlamaBackend instance
///
/// # Errors
/// Returns `InferenceError::ModelLoad` if loading fails.
/// Returns `InferenceError::SpawnBlocking` if the blocking task panics.
pub async fn load(
    model_id: &str,
    model_path: &Path,
    gpu_layers: u32,
    context_length: u32,
    backend: Arc<LlamaBackend>,
) -> Result<ModelHandle, InferenceError> {
    let model_id = model_id.to_string();
    let model_path = model_path.to_path_buf();

    let handle = tokio::task::spawn_blocking(move || {
        let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);

        let model = LlamaModel::load_from_file(&backend, &model_path, &params).map_err(|err| {
            InferenceError::ModelLoad {
                model_id: model_id.clone(),
                reason: format!("llama-cpp-2 load failed: {err}"),
            }
        })?;

        Ok::<ModelHandle, InferenceError>(ModelHandle {
            model: Arc::new(model),
            backend,
            model_id,
            context_length,
        })
    })
    .await?;

    handle
}

/// Unload a model, releasing its VRAM.
/// The actual drop happens inside `spawn_blocking` to avoid blocking the async runtime.
///
/// # Errors
/// Returns `InferenceError::SpawnBlocking` if the blocking task panics.
pub async fn unload(handle: ModelHandle) -> Result<(), InferenceError> {
    tokio::task::spawn_blocking(move || {
        // Drop the model — llama-cpp-2 frees VRAM on drop
        drop(handle);
    })
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_estimate_vram_errors_on_missing_file() {
        let result = estimate_vram_mb(Path::new("/nonexistent/model.gguf"));
        assert!(result.is_err());
        match result.expect_err("test: should error on missing file") {
            InferenceError::ModelLoad { .. } => {}
            other => panic!("Expected ModelLoad error, got: {other}"),
        }
    }

    #[test]
    fn test_estimate_vram_returns_nonzero_for_valid_path() {
        // Create a temp file to act as a fake model for file-size estimation
        let mut temp = tempfile::NamedTempFile::new().expect("test: temp file creation");
        // Write 10MB of data
        let data = vec![0u8; 10 * 1024 * 1024];
        temp.write_all(&data).expect("test: write temp data");

        let result = estimate_vram_mb(temp.path());
        let vram = result.expect("test: should succeed for existing file");
        assert!(vram > 0, "VRAM estimate should be non-zero for a 10MB file");
    }

    #[test]
    fn test_model_handle_is_send() {
        // Static assertion: ModelHandle must be Send
        fn assert_send<T: Send>() {}
        assert_send::<ModelHandle>();
    }

    #[test]
    fn test_model_handle_is_sync() {
        // Static assertion: ModelHandle must be Sync
        fn assert_sync<T: Sync>() {}
        assert_sync::<ModelHandle>();
    }
}
