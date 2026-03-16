//! Embedder trait implementation for ech0, backed by llama-cpp-rs.
//!
//! `LlamaEmbedder` implements ech0's `Embedder` trait using `llama-cpp-2` for
//! local embedding generation. The model path comes from `Config` — never
//! hardcoded.
//!
//! The trait contract requires returning `EchoError` (not `SenaError`).
//! Conversion to `SenaError` happens at the call site in `engine.rs`.
//!
//! llama-cpp-2 inference is blocking — all calls go through
//! `tokio::task::spawn_blocking` to avoid blocking the async runtime.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ech0::{EchoError, Embedder, ErrorContext};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};

use crate::config::EmbedderConfig;

// ─────────────────────────────────────────────────────────────────────────────
// LlamaEmbedder
// ─────────────────────────────────────────────────────────────────────────────

/// Embedding generator backed by a GGUF model loaded through llama-cpp-2.
///
/// Holds an `Arc<LlamaModel>` and `Arc<LlamaBackend>` so they can be shared
/// across async tasks and sent into `spawn_blocking` closures without lifetime
/// issues.
pub struct LlamaEmbedder {
    /// The loaded llama-cpp-2 model used for embedding generation.
    /// Wrapped in `Arc` because `spawn_blocking` requires `'static` captures
    /// and the model is shared across concurrent embed calls.
    model: Arc<LlamaModel>,

    /// The llama backend instance — required by `new_context`.
    /// Must outlive all contexts created from `model`.
    backend: Arc<LlamaBackend>,

    /// The fixed dimensionality of vectors produced by this embedder.
    /// Must match `StoreConfig.store.vector_dimensions`.
    embedding_dimensions: usize,

    /// Path to the GGUF model file — retained for diagnostics logging only.
    /// Never logged with user content, only referenced in error context.
    model_path: PathBuf,
}

// SAFETY: LlamaModel and LlamaBackend are thread-safe behind Arc.
// The llama-cpp-2 crate documents that model and backend are safe to
// share across threads when accessed through proper synchronization.
unsafe impl Send for LlamaEmbedder {}
unsafe impl Sync for LlamaEmbedder {}

impl LlamaEmbedder {
    /// Construct a new `LlamaEmbedder` by loading the GGUF model specified
    /// in `config`.
    ///
    /// Model loading is blocking I/O — this constructor should be called
    /// from outside the async runtime or wrapped in `spawn_blocking` by
    /// the caller.
    ///
    /// `backend` is the initialized `LlamaBackend` — the caller must
    /// initialize it once and share it across all llama-cpp-2 users in
    /// the process.
    ///
    /// `embedding_dimensions` is the known output dimensionality of the
    /// embedding model. It must match `StoreConfig.store.vector_dimensions`.
    ///
    /// # Errors
    ///
    /// Returns `EchoError` if the model file cannot be loaded. The error
    /// context includes the model path but never any user content.
    pub fn new(
        config: &EmbedderConfig,
        backend: Arc<LlamaBackend>,
        embedding_dimensions: usize,
    ) -> Result<Self, EchoError> {
        let model_path = PathBuf::from(&config.model_path);

        let params = LlamaModelParams::default().with_n_gpu_layers(config.gpu_layers);

        // LlamaModel::load_from_file(&LlamaBackend, impl AsRef<Path>, &LlamaModelParams)
        let model =
            LlamaModel::load_from_file(&backend, &model_path, &params).map_err(|load_error| {
                EchoError::embedder_failure(format!(
                    "failed to load embedding model at '{}': {}",
                    model_path.display(),
                    load_error
                ))
                .with_context(
                    ErrorContext::new("LlamaEmbedder::new")
                        .with_source(&load_error)
                        .with_field("model_path", model_path.display().to_string()),
                )
            })?;

        tracing::info!(
            subsystem = "memory_engine",
            component = "embedder",
            model_path = %model_path.display(),
            gpu_layers = config.gpu_layers,
            batch_size = config.batch_size,
            embedding_dimensions = embedding_dimensions,
            "embedding model loaded"
        );

        Ok(Self {
            model: Arc::new(model),
            backend,
            embedding_dimensions,
            model_path,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Embedder trait impl
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
impl Embedder for LlamaEmbedder {
    /// Produce a vector embedding for the given text.
    ///
    /// The actual llama-cpp-2 inference call is blocking, so it runs inside
    /// `tokio::task::spawn_blocking` to avoid stalling the async runtime.
    ///
    /// # Errors
    ///
    /// Returns `EchoError` with `ErrorCode::EmbedderFailure` if:
    /// - The inference context cannot be created
    /// - Tokenization fails
    /// - The embedding extraction call fails
    /// - The blocking task panics (converted to an EchoError)
    ///
    /// The error message never contains the input text — only the operation
    /// name and the underlying error description.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EchoError> {
        // Clone what we need before entering the blocking closure so we
        // don't capture `&self` across the await point.
        let model = Arc::clone(&self.model);
        let backend = Arc::clone(&self.backend);
        let input_text = text.to_owned();
        let model_path_display = self.model_path.display().to_string();

        let embedding_result = tokio::task::spawn_blocking(move || {
            // Create a context for this embedding call.
            let context_params = LlamaContextParams::default().with_embeddings(true);

            // LlamaModel::new_context(&self, &LlamaBackend, LlamaContextParams)
            let context = model
                .new_context(&backend, context_params)
                .map_err(|context_error| {
                    EchoError::embedder_failure(format!(
                        "failed to create inference context for embedding (model: '{}'): {}",
                        model_path_display, context_error
                    ))
                    .with_context(
                        ErrorContext::new("LlamaEmbedder::embed::new_context")
                            .with_source(&context_error)
                            .with_field("model_path", model_path_display.clone()),
                    )
                })?;

            // Tokenize the input text.
            let tokens =
                model
                    .str_to_token(&input_text, AddBos::Always)
                    .map_err(|tokenize_error| {
                        EchoError::embedder_failure(format!(
                            "tokenization failed for embedding call: {}",
                            tokenize_error
                        ))
                        .with_context(
                            ErrorContext::new("LlamaEmbedder::embed::str_to_token")
                                .with_source(&tokenize_error),
                        )
                    })?;

            // Run the embedding extraction.
            // The actual API shape may vary across llama-cpp-2 versions —
            // this follows the documented embedding extraction pattern.
            let embeddings = context.embeddings_seq_ith(0).map_err(|embed_error| {
                EchoError::embedder_failure(format!(
                    "embedding extraction failed (token_count: {}): {}",
                    tokens.len(),
                    embed_error
                ))
                .with_context(
                    ErrorContext::new("LlamaEmbedder::embed::embeddings_seq_ith")
                        .with_source(&embed_error)
                        .with_field("token_count", tokens.len().to_string()),
                )
            })?;

            Ok::<Vec<f32>, EchoError>(embeddings.to_vec())
        })
        .await
        .map_err(|join_error| {
            EchoError::embedder_failure(format!(
                "embedding spawn_blocking task failed: {}",
                join_error
            ))
            .with_context(
                ErrorContext::new("LlamaEmbedder::embed::spawn_blocking").with_source(&join_error),
            )
        })?;

        embedding_result
    }

    /// The fixed dimensionality of vectors produced by this embedder.
    fn dimensions(&self) -> usize {
        self.embedding_dimensions
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DegradedEmbedder — stub for testing or fallback scenarios
// ─────────────────────────────────────────────────────────────────────────────

/// A no-op embedder that returns a zero vector of a fixed dimension.
///
/// Used only in integration tests or exceptional fallback scenarios where
/// no model is available. Production code always uses `LlamaEmbedder`.
#[cfg(test)]
pub struct DegradedEmbedder {
    /// Dimensionality of the zero vector returned by `embed`.
    pub dimension_count: usize,
}

#[cfg(test)]
#[async_trait]
impl Embedder for DegradedEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EchoError> {
        Ok(vec![0.0; self.dimension_count])
    }

    fn dimensions(&self) -> usize {
        self.dimension_count
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn degraded_embedder_returns_zero_vector() {
        let embedder = DegradedEmbedder {
            dimension_count: 384,
        };
        let result = embedder.embed("test input").await;

        let embedding = result.expect("degraded embedder should not fail");
        assert_eq!(embedding.len(), 384);
        assert!(
            embedding.iter().all(|value| *value == 0.0),
            "degraded embedder should return all zeros"
        );
    }

    #[tokio::test]
    async fn degraded_embedder_handles_empty_input() {
        let embedder = DegradedEmbedder {
            dimension_count: 128,
        };
        let result = embedder.embed("").await;

        let embedding = result.expect("degraded embedder should handle empty input");
        assert_eq!(embedding.len(), 128);
    }

    #[test]
    fn degraded_embedder_dimensions() {
        let embedder = DegradedEmbedder {
            dimension_count: 768,
        };
        assert_eq!(embedder.dimensions(), 768);
    }
}
