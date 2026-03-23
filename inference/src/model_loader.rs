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
    pub(crate) display_name: String,
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

/// Extract display name from GGUF metadata if available, otherwise fall back to model_id.
/// This is called after the model is loaded to get a human-readable name for the UI.
fn extract_display_name(model_path: &Path, model_id: &str) -> String {
    parse_gguf_general_name(model_path).unwrap_or_else(|| extract_display_name_fallback(model_id))
}

/// Fallback display name when GGUF metadata is unavailable — just use model_id.
fn extract_display_name_fallback(model_id: &str) -> String {
    model_id.to_string()
}

/// Parse the GGUF file header and extract the `general.name` metadata field.
/// Returns `None` if the file cannot be read, has an invalid GGUF header, or the key is absent.
///
/// GGUF metadata format (simplified):
/// - Magic: "GGUF" (4 bytes)
/// - Version: u32
/// - Tensor count: u64
/// - Metadata KV count: u64
/// - Metadata KV pairs: key_len (u64), key (bytes), value_type (u32), value_data
///
/// String value type = 8, followed by string_len (u64), string_bytes
fn parse_gguf_general_name(model_path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(model_path).ok()?;

    // Read GGUF magic
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }

    // Read version (u32)
    let mut version_buf = [0u8; 4];
    file.read_exact(&mut version_buf).ok()?;
    let _version = u32::from_le_bytes(version_buf);

    // Read tensor_count (u64)
    let mut tensor_count_buf = [0u8; 8];
    file.read_exact(&mut tensor_count_buf).ok()?;
    let _tensor_count = u64::from_le_bytes(tensor_count_buf);

    // Read metadata_kv_count (u64)
    let mut metadata_kv_count_buf = [0u8; 8];
    file.read_exact(&mut metadata_kv_count_buf).ok()?;
    let metadata_kv_count = u64::from_le_bytes(metadata_kv_count_buf);

    // Iterate through metadata KV pairs looking for "general.name"
    for _ in 0..metadata_kv_count {
        // Read key length (u64)
        let mut key_len_buf = [0u8; 8];
        file.read_exact(&mut key_len_buf).ok()?;
        let key_len = u64::from_le_bytes(key_len_buf);

        // Read key bytes
        let mut key_bytes = vec![0u8; key_len as usize];
        file.read_exact(&mut key_bytes).ok()?;
        let key = String::from_utf8(key_bytes).ok()?;

        // Read value type (u32)
        let mut value_type_buf = [0u8; 4];
        file.read_exact(&mut value_type_buf).ok()?;
        let value_type = u32::from_le_bytes(value_type_buf);

        // If this is "general.name" and value_type is string (8)
        if key == "general.name" && value_type == 8 {
            // Read string length (u64)
            let mut str_len_buf = [0u8; 8];
            file.read_exact(&mut str_len_buf).ok()?;
            let str_len = u64::from_le_bytes(str_len_buf);

            // Read string bytes
            let mut str_bytes = vec![0u8; str_len as usize];
            file.read_exact(&mut str_bytes).ok()?;
            return String::from_utf8(str_bytes).ok();
        } else {
            // Skip the value data — we need to know the size based on value_type
            // For simplicity, if not string or not the key we want, skip by reading
            // value data. Value types and sizes vary, so this is a simplified version.
            // For complete implementation, we'd need to handle all GGUF value types.
            // For now, we'll just try to find general.name and skip others conservatively.

            // Skip value based on type
            // Type 8 = string: u64 len + bytes
            // Type 4 = u32: 4 bytes
            // Type 5 = i32: 4 bytes
            // Type 6 = f32: 4 bytes
            // Type 7 = bool: 1 byte
            // Type 9 = array: u32 type + u64 len + elements
            // For simplicity in this MVP, we only parse strings fully.
            // For other types, we'll skip conservatively or return None.
            match value_type {
                8 => {
                    // String: u64 len + bytes
                    let mut str_len_buf = [0u8; 8];
                    file.read_exact(&mut str_len_buf).ok()?;
                    let str_len = u64::from_le_bytes(str_len_buf);
                    file.seek(SeekFrom::Current(str_len as i64)).ok()?;
                }
                4 | 5 | 6 => {
                    // u32, i32, f32: 4 bytes
                    file.seek(SeekFrom::Current(4)).ok()?;
                }
                7 => {
                    // bool: 1 byte
                    file.seek(SeekFrom::Current(1)).ok()?;
                }
                0 | 1 | 2 | 3 => {
                    // u8, i8, u16, i16: 1, 1, 2, 2 bytes
                    let size = match value_type {
                        0 | 1 => 1,
                        2 | 3 => 2,
                        _ => return None,
                    };
                    file.seek(SeekFrom::Current(size)).ok()?;
                }
                10 | 11 | 12 => {
                    // u64, i64, f64: 8 bytes
                    file.seek(SeekFrom::Current(8)).ok()?;
                }
                9 => {
                    // Array: more complex, skip for now
                    // Read array type (u32) and array length (u64)
                    let mut arr_type_buf = [0u8; 4];
                    file.read_exact(&mut arr_type_buf).ok()?;
                    let arr_type = u32::from_le_bytes(arr_type_buf);

                    let mut arr_len_buf = [0u8; 8];
                    file.read_exact(&mut arr_len_buf).ok()?;
                    let arr_len = u64::from_le_bytes(arr_len_buf);

                    // Calculate element size and skip
                    let elem_size = match arr_type {
                        0 | 1 | 7 => 1,
                        2 | 3 => 2,
                        4 | 5 | 6 => 4,
                        10 | 11 | 12 => 8,
                        8 => {
                            // Array of strings — too complex for MVP, return None
                            return None;
                        }
                        _ => return None,
                    };
                    file.seek(SeekFrom::Current((arr_len * elem_size) as i64))
                        .ok()?;
                }
                _ => {
                    // Unknown type, cannot continue parsing safely
                    return None;
                }
            }
        }
    }

    None
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

        // Extract display name from GGUF metadata
        let display_name = extract_display_name(&model_path, &model_id);

        Ok::<ModelHandle, InferenceError>(ModelHandle {
            model: Arc::new(model),
            backend,
            model_id,
            context_length,
            display_name,
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

    #[test]
    fn test_extract_display_name_fallback_to_model_id() {
        // When GGUF metadata extraction returns None or fails,
        // display_name should fall back to model_id
        let model_id = "gemma-2b-it";
        let fallback = extract_display_name_fallback(model_id);
        assert_eq!(fallback, model_id);
    }

    #[test]
    fn test_parse_gguf_general_name_missing_file() {
        // Should return None for missing file
        let result = parse_gguf_general_name(Path::new("/nonexistent/model.gguf"));
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_gguf_general_name_invalid_header() {
        // Should return None for file with invalid GGUF magic
        let mut temp = tempfile::NamedTempFile::new().expect("test: temp file creation");
        temp.write_all(b"NOT_GGUF_MAGIC").expect("test: write data");

        let result = parse_gguf_general_name(temp.path());
        assert!(result.is_none());
    }
}
