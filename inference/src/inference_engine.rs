//! Inference engine — coordinates model registry, loader, and request queue.
//!
//! The engine owns the lifecycle of model loading/unloading, request
//! serialization through the priority queue, and the continuous worker
//! loop that pops requests and runs inference via spawn_blocking.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{oneshot, Mutex};

use crate::config::Config;
use crate::error::InferenceError;
use crate::generated::sena_daemonbus_v1::CompleteResponse;
use crate::model_loader::{self, ModelHandle};
use crate::model_registry::{ModelLoadState, ModelRegistry};
use crate::request_queue::{Priority, QueuedRequest, RequestQueue};

/// The inference engine — central coordinator for model lifecycle and request processing.
pub struct InferenceEngine {
    config: Arc<Config>,
    registry: Arc<ModelRegistry>,
    queue: Arc<RequestQueue>,
    model_handle: Arc<Mutex<Option<ModelHandle>>>,
}

impl InferenceEngine {
    /// Create a new engine with empty registry and queue configured from config.
    pub fn new(config: Arc<Config>) -> Self {
        let queue = Arc::new(RequestQueue::new(
            config.runtime.request_queue_max_depth,
            config.runtime.request_timeout_ms,
        ));

        Self {
            config,
            registry: Arc::new(ModelRegistry::new()),
            queue,
            model_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Get a reference to the model registry.
    pub fn registry(&self) -> &Arc<ModelRegistry> {
        &self.registry
    }

    /// Get a reference to the request queue.
    pub fn queue(&self) -> &Arc<RequestQueue> {
        &self.queue
    }

    /// Load a model by ID, checking VRAM budget first.
    /// Updates registry state through Loading → Ready (or Failed on error).
    pub async fn load_model(
        &self,
        model_id: &str,
        model_path: &str,
        backend: Arc<llama_cpp_2::llama_backend::LlamaBackend>,
    ) -> Result<(), InferenceError> {
        // Estimate VRAM before loading
        let vram_estimate = model_loader::estimate_vram_mb(std::path::Path::new(model_path))?;
        let current_vram = self.registry.total_vram_allocated_mb().await;
        let available = self.config.model.vram_budget_mb.saturating_sub(current_vram);

        if vram_estimate > available {
            return Err(InferenceError::InsufficientVram {
                required_mb: vram_estimate,
                available_mb: available,
            });
        }

        // Register model — always succeeds for valid input
        self.registry
            .register(
                model_id.to_string(),
                model_path.to_string(),
                vram_estimate,
            )
            .await?;

        // Set state to Loading
        self.registry
            .set_state(model_id, ModelLoadState::Loading)
            .await?;

        // Load the model
        match model_loader::load(
            model_id,
            std::path::Path::new(model_path),
            self.config.model.gpu_layers,
            backend,
        )
        .await
        {
            Ok(handle) => {
                self.registry
                    .set_state(model_id, ModelLoadState::Ready)
                    .await?;
                self.registry.set_active(model_id).await?;
                let mut guard = self.model_handle.lock().await;
                *guard = Some(handle);
                Ok(())
            }
            Err(load_error) => {
                // Set state to Failed — if this fails, we can't do much
                let fail_state = ModelLoadState::Failed(load_error.to_string());
                self.registry
                    .set_state(model_id, fail_state)
                    .await
                    .unwrap_or_else(|registry_error| {
                        tracing::error!(
                            event_type = "registry_state_update_failed",
                            model_id = model_id,
                            error = %registry_error,
                            "failed to update registry after model load failure"
                        );
                    });
                Err(load_error)
            }
        }
    }

    /// Enqueue a completion request. Returns a receiver for the result.
    pub async fn enqueue(
        &self,
        prompt: String,
        model_id: String,
        max_tokens: u32,
        temperature: f32,
        request_id: String,
        priority: Priority,
    ) -> Result<oneshot::Receiver<Result<CompleteResponse, InferenceError>>, InferenceError> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = QueuedRequest {
            prompt,
            model_id,
            max_tokens,
            temperature,
            request_id,
            priority,
            enqueued_at: Instant::now(),
            response_tx,
        };

        self.queue.push(request).await?;
        Ok(response_rx)
    }

    /// Run the worker loop. Pops requests and processes them via spawn_blocking.
    /// This should be spawned in a tokio task.
    pub async fn run_worker(self: Arc<Self>) -> Result<(), InferenceError> {
        loop {
            let request = match self.queue.pop().await {
                Some(req) => req,
                None => continue,
            };

            let engine = Arc::clone(&self);
            let request_id = request.request_id.clone();
            let model_id_requested = request.model_id.clone();

            let result = engine.process_request(&request).await;

            // Send result on the response channel — receiver may have dropped
            match request.response_tx.send(result) {
                Ok(()) => {
                    tracing::debug!(
                        event_type = "inference_completed",
                        request_id = %request_id,
                        model_id = %model_id_requested,
                    );
                }
                Err(_unsent) => {
                    tracing::warn!(
                        event_type = "response_channel_closed",
                        request_id = %request_id,
                        "caller dropped response receiver before completion"
                    );
                }
            }
        }
    }

    /// Process a single inference request.
    async fn process_request(
        &self,
        request: &QueuedRequest,
    ) -> Result<CompleteResponse, InferenceError> {
        let handle_guard = self.model_handle.lock().await;
        let handle = handle_guard
            .as_ref()
            .ok_or(InferenceError::ModelSwitching)?;

        let model = Arc::clone(&handle.model);
        let backend = Arc::clone(&handle.backend);
        let active_model_id = handle.model_id.clone();
        let prompt = request.prompt.clone();
        let max_tokens = request.max_tokens;
        let temperature = request.temperature;
        let request_id = request.request_id.clone();
        drop(handle_guard); // release lock before spawn_blocking

        let start = Instant::now();

        let output = tokio::task::spawn_blocking(move || {
            // FIXME(inference): Replace this placeholder with actual llama-cpp-2
            // completion API calls. The exact API for text generation (as opposed
            // to embedding) needs to be determined from llama-cpp-2 source.
            // For now, return a placeholder that allows the structure to compile
            // and tests to verify the coordination logic.
            //
            // The real implementation will:
            // 1. Create a context with LlamaContextParams
            // 2. Tokenize the prompt
            // 3. Run the sampling loop up to max_tokens
            // 4. Detokenize the output
            let _ = (model, backend, prompt, max_tokens, temperature);
            Ok::<(String, u32, u32), InferenceError>((
                String::from("placeholder — FIXME(inference)"),
                0u32, // tokens_generated
                0u32, // tokens_prompt
            ))
        })
        .await
        .map_err(InferenceError::SpawnBlocking)??;

        let duration = start.elapsed();
        tracing::debug!(
            event_type = "inference_completed",
            model_id = %active_model_id,
            duration_ms = duration.as_millis() as u64,
            tokens_generated = output.1,
            request_id = %request_id,
        );

        Ok(CompleteResponse {
            text: output.0,
            tokens_generated: output.1,
            tokens_prompt: output.2,
            model_id: active_model_id,
            request_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn test_config() -> Arc<Config> {
        // Write a temp config and load it
        let content = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "test-model"
model_path = "models/test.gguf"
gpu_layers = 0
context_length = 4096
vram_budget_mb = 4096

[runtime]
request_queue_max_depth = 64
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[logging]
level = "info"
format = "json"
"#;
        let mut file =
            tempfile::NamedTempFile::new().expect("test: temp file creation");
        file.write_all(content.as_bytes())
            .expect("test: write config");
        let config = crate::config::Config::load(file.path()).expect("test: valid config");
        Arc::new(config)
    }

    #[tokio::test]
    async fn test_engine_new_initializes_empty_registry() {
        let config = test_config();
        let engine = InferenceEngine::new(config);
        let active = engine.registry().active_model_id().await;
        assert!(active.is_none(), "New engine should have no active model");
    }

    #[tokio::test]
    async fn test_enqueue_returns_receiver() {
        let config = test_config();
        let engine = InferenceEngine::new(config);

        let result = engine
            .enqueue(
                "test prompt".into(),
                String::new(),
                100,
                0.7,
                "req-1".into(),
                Priority::Standard,
            )
            .await;

        assert!(result.is_ok(), "Enqueue should return a receiver");
    }

    #[tokio::test]
    async fn test_load_model_vram_check_fails_gracefully() {
        // Create a config with very small VRAM budget
        let content = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "test"
model_path = "models/test.gguf"
gpu_layers = 0
context_length = 4096
vram_budget_mb = 1

[runtime]
request_queue_max_depth = 64
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[logging]
level = "info"
format = "json"
"#;
        let mut file = tempfile::NamedTempFile::new().expect("test: temp file");
        file.write_all(content.as_bytes()).expect("test: write");
        let config = Arc::new(
            crate::config::Config::load(file.path()).expect("test: load config"),
        );

        let engine = InferenceEngine::new(config);

        // Create a large temp file that exceeds 1MB VRAM budget
        let mut model_file = tempfile::NamedTempFile::new().expect("test: model temp file");
        let data = vec![0u8; 5 * 1024 * 1024]; // 5MB → ~6MB VRAM estimate
        model_file
            .write_all(&data)
            .expect("test: write model data");

        let backend = Arc::new(
            llama_cpp_2::llama_backend::LlamaBackend::init().expect(
                "test: llama backend init — required for model loading tests",
            ),
        );

        let result = engine
            .load_model(
                "test-model",
                model_file.path().to_str().expect("test: path to str"),
                backend,
            )
            .await;

        match result {
            Err(InferenceError::InsufficientVram { .. }) => {} // expected
            other => panic!("Expected InsufficientVram error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_engine_queue_enforces_max_depth() {
        let content = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "test"
model_path = "models/test.gguf"
gpu_layers = 0
context_length = 4096
vram_budget_mb = 4096

[runtime]
request_queue_max_depth = 1
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[logging]
level = "info"
format = "json"
"#;
        let mut file = tempfile::NamedTempFile::new().expect("test: temp file");
        file.write_all(content.as_bytes()).expect("test: write");
        let config = Arc::new(
            crate::config::Config::load(file.path()).expect("test: load config"),
        );

        let engine = InferenceEngine::new(config);

        // First enqueue succeeds
        let r1 = engine
            .enqueue(
                "p1".into(),
                String::new(),
                10,
                0.7,
                "r1".into(),
                Priority::Standard,
            )
            .await;
        assert!(r1.is_ok());

        // Second should fail (max_depth = 1)
        let r2 = engine
            .enqueue(
                "p2".into(),
                String::new(),
                10,
                0.7,
                "r2".into(),
                Priority::Standard,
            )
            .await;
        assert!(r2.is_err());
        match r2.expect_err("test: should be full") {
            InferenceError::RequestQueueFull { .. } => {}
            other => panic!("Expected RequestQueueFull, got: {other}"),
        }
    }
}
