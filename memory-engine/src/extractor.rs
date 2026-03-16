//! Extractor trait implementation for ech0, backed by llama-cpp-rs.
//!
//! `LlamaExtractor` implements ech0's `Extractor` trait using `llama-cpp-2` for
//! local graph/entity extraction via structured output. The model path, GPU
//! layers, max tokens, and temperature come from `Config` — never hardcoded.
//!
//! The trait contract requires returning `EchoError` (not `SenaError`).
//! Conversion to `SenaError` happens at the call site in `engine.rs`.
//!
//! llama-cpp-2 inference is blocking — all calls go through
//! `tokio::task::spawn_blocking` to avoid blocking the async runtime.
//!
//! When the model cannot produce structured output (`degraded_extractor` flag
//! from `ProfileDerivedConfig`), a `DegradedExtractor` stub is used instead.
//! It returns empty extraction results and logs at warn level. This is an
//! explicit, visible degradation — never silent.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ech0::{EchoError, ErrorContext, ExtractionResult, Extractor};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;

use crate::config::ExtractorConfig;

// ─────────────────────────────────────────────────────────────────────────────
// LlamaExtractor
// ─────────────────────────────────────────────────────────────────────────────

/// Graph/entity extractor backed by a GGUF model loaded through llama-cpp-2.
///
/// Holds an `Arc<LlamaModel>` and `Arc<LlamaBackend>` so they can be shared
/// across async tasks and sent into `spawn_blocking` closures without lifetime
/// issues.
pub struct LlamaExtractor {
    /// The loaded llama-cpp-2 model used for extraction inference.
    /// Wrapped in `Arc` because `spawn_blocking` requires `'static` captures
    /// and the model is shared across concurrent extract calls.
    model: Arc<LlamaModel>,

    /// The llama backend instance — required by `new_context`.
    /// Must outlive all contexts created from `model`.
    backend: Arc<LlamaBackend>,

    /// Maximum number of tokens the model may generate per extraction call.
    max_tokens: u32,

    /// Temperature for extraction inference. 0.0 for deterministic output.
    temperature: f32,

    /// Path to the GGUF model file — retained for diagnostics logging only.
    /// Never logged with user content, only referenced in error context.
    model_path: PathBuf,
}

// SAFETY: LlamaModel and LlamaBackend are thread-safe behind Arc.
// The llama-cpp-2 crate documents that model and backend are safe to
// share across threads when accessed through proper synchronization.
unsafe impl Send for LlamaExtractor {}
unsafe impl Sync for LlamaExtractor {}

impl LlamaExtractor {
    /// Construct a new `LlamaExtractor` by loading the GGUF model specified
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
    /// # Errors
    ///
    /// Returns `EchoError` if the model file cannot be loaded. The error
    /// context includes the model path but never any user content.
    pub fn new(config: &ExtractorConfig, backend: Arc<LlamaBackend>) -> Result<Self, EchoError> {
        let model_path = PathBuf::from(&config.model_path);

        let params = LlamaModelParams::default().with_n_gpu_layers(config.gpu_layers);

        // LlamaModel::load_from_file(&LlamaBackend, impl AsRef<Path>, &LlamaModelParams)
        let model =
            LlamaModel::load_from_file(&backend, &model_path, &params).map_err(|load_error| {
                EchoError::extractor_failure(format!(
                    "failed to load extractor model at '{}': {}",
                    model_path.display(),
                    load_error
                ))
                .with_context(
                    ErrorContext::new("LlamaExtractor::new")
                        .with_source(&load_error)
                        .with_field("model_path", model_path.display().to_string()),
                )
            })?;

        tracing::info!(
            subsystem = "memory_engine",
            component = "extractor",
            model_path = %model_path.display(),
            gpu_layers = config.gpu_layers,
            max_tokens = config.max_tokens,
            temperature = config.temperature,
            "extractor model loaded"
        );

        Ok(Self {
            model: Arc::new(model),
            backend,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            model_path,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Extractor trait impl
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
impl Extractor for LlamaExtractor {
    /// Extract entities, relations, and graph structure from the given text.
    ///
    /// The actual llama-cpp-2 inference call is blocking, so it runs inside
    /// `tokio::task::spawn_blocking` to avoid stalling the async runtime.
    ///
    /// # Errors
    ///
    /// Returns `EchoError` with `ErrorCode::ExtractorFailure` if:
    /// - The inference context cannot be created
    /// - Tokenization fails
    /// - The generation call fails
    /// - The output cannot be parsed into an `ExtractionResult`
    /// - The blocking task panics (converted to an EchoError)
    ///
    /// The error message never contains the input text — only the operation
    /// name and the underlying error description.
    async fn extract(&self, text: &str) -> Result<ExtractionResult, EchoError> {
        // Clone what we need before entering the blocking closure so we
        // don't capture `&self` across the await point.
        let model = Arc::clone(&self.model);
        let backend = Arc::clone(&self.backend);
        let input_text = text.to_owned();
        let max_tokens = self.max_tokens;
        let temperature = self.temperature;
        let model_path_display = self.model_path.display().to_string();

        let extraction_result = tokio::task::spawn_blocking(move || {
            // Create a context for this extraction call.
            let context_params = llama_cpp_2::context::params::LlamaContextParams::default();

            // LlamaModel::new_context(&self, &LlamaBackend, LlamaContextParams)
            let context = model
                .new_context(&backend, context_params)
                .map_err(|context_error| {
                    EchoError::extractor_failure(format!(
                        "failed to create inference context for extraction (model: '{}'): {}",
                        model_path_display, context_error
                    ))
                    .with_context(
                        ErrorContext::new("LlamaExtractor::extract::new_context")
                            .with_source(&context_error)
                            .with_field("model_path", model_path_display.clone()),
                    )
                })?;

            // Tokenize the input text.
            let tokens = model
                .str_to_token(&input_text, llama_cpp_2::model::AddBos::Always)
                .map_err(|tokenize_error| {
                    EchoError::extractor_failure(format!(
                        "tokenization failed for extraction call: {}",
                        tokenize_error
                    ))
                    .with_context(
                        ErrorContext::new("LlamaExtractor::extract::str_to_token")
                            .with_source(&tokenize_error),
                    )
                })?;

            // TODO(phase2): Wire up the full generation loop.
            //
            // The real implementation will:
            // 1. Feed the tokenized prompt into the context
            // 2. Sample up to `max_tokens` tokens at the configured `temperature`
            // 3. Decode the generated tokens back to a string
            // 4. Parse the string as structured output into `ExtractionResult`
            //
            // This is deferred to Phase 2 per PRD §6.5 — the extraction
            // pipeline requires structured output prompting which is a
            // Phase 2 capability. The DegradedExtractor is the V1 fallback.
            //
            // For now, return an error indicating the extraction pipeline is
            // not yet wired. This ensures no caller silently receives empty
            // results without knowing extraction is unimplemented.
            let _ = (context, tokens, max_tokens, temperature);

            Err::<ExtractionResult, EchoError>(
                EchoError::extractor_failure(
                    "LlamaExtractor inference loop not yet wired — pending llama-cpp-2 integration",
                )
                .with_context(ErrorContext::new("LlamaExtractor::extract::inference_loop")),
            )
        })
        .await
        .map_err(|join_error| {
            EchoError::extractor_failure(format!(
                "extraction spawn_blocking task failed: {}",
                join_error
            ))
            .with_context(
                ErrorContext::new("LlamaExtractor::extract::spawn_blocking")
                    .with_source(&join_error),
            )
        })?;

        extraction_result
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DegradedExtractor
// ─────────────────────────────────────────────────────────────────────────────

/// A stub extractor that returns empty `ExtractionResult` values.
///
/// Used when `ProfileDerivedConfig.degraded_extractor` is `true` — meaning
/// the active model cannot produce structured output at all
/// (`structured_output == CapabilityLevel::None`).
///
/// Every call logs at warn level so the degradation is never silent.
/// This is an explicit, visible fallback — not a hidden default.
pub struct DegradedExtractor;

impl DegradedExtractor {
    pub fn new() -> Self {
        tracing::warn!(
            subsystem = "memory_engine",
            component = "extractor",
            mode = "degraded",
            "DegradedExtractor initialized — extraction calls will return empty results"
        );
        Self
    }
}

#[async_trait]
impl Extractor for DegradedExtractor {
    /// Returns an empty `ExtractionResult` and logs a warning.
    ///
    /// This is the correct behavior when the model cannot produce structured
    /// output — returning an empty result is better than returning hallucinated
    /// garbage that would corrupt the knowledge graph.
    async fn extract(&self, _text: &str) -> Result<ExtractionResult, EchoError> {
        tracing::warn!(
            subsystem = "memory_engine",
            component = "extractor",
            mode = "degraded",
            operation = "extract",
            "extraction skipped — degraded extractor returning empty result"
        );

        Ok(ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn degraded_extractor_returns_empty_result() {
        let extractor = DegradedExtractor::new();
        let result = extractor.extract("some text that should be ignored").await;

        let extraction = result.expect("degraded extractor should not return an error");
        assert!(
            extraction.is_empty(),
            "degraded extractor should return empty ExtractionResult"
        );
        assert_eq!(extraction.len(), 0);
    }

    #[tokio::test]
    async fn degraded_extractor_handles_empty_input() {
        let extractor = DegradedExtractor::new();
        let result = extractor.extract("").await;

        let extraction =
            result.expect("degraded extractor should handle empty input without error");
        assert!(extraction.is_empty());
    }
}
