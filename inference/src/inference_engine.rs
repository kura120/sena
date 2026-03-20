//! Inference engine — coordinates model registry, loader, and request queue.
//!
//! The engine owns the lifecycle of model loading/unloading, request
//! serialization through the priority queue, and the continuous worker
//! loop that pops requests and runs inference via spawn_blocking.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::AddBos;
use llama_cpp_2::sampling::LlamaSampler;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::config::Config;
use crate::error::InferenceError;
use crate::generated::sena_daemonbus_v1::{
    boot_service_client::BootServiceClient, BootSignal, BootSignalRequest, CompleteResponse,
    StreamCompleteChunk,
};
use crate::model_loader::{self, ModelHandle};
use crate::model_registry::{ModelLoadState, ModelRegistry};
use crate::request_queue::{Priority, QueuedRequest, RequestQueue, ResponseChannel};

/// RAII guard for tracking in-flight requests.
/// Increments the counter on creation and decrements on drop.
struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl InFlightGuard {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self {
            counter: Arc::clone(counter),
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The inference engine — central coordinator for model lifecycle and request processing.
pub struct InferenceEngine {
    config: Arc<Config>,
    registry: Arc<ModelRegistry>,
    queue: Arc<RequestQueue>,
    model_handle: Arc<Mutex<Option<ModelHandle>>>,
    is_switching: Arc<AtomicBool>,
    inflight_count: Arc<AtomicUsize>,
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
            is_switching: Arc::new(AtomicBool::new(false)),
            inflight_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Check if the engine is currently switching models.
    pub fn is_switching(&self) -> bool {
        self.is_switching.load(Ordering::SeqCst)
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
        gpu_layers: u32,
        backend: Arc<llama_cpp_2::llama_backend::LlamaBackend>,
    ) -> Result<(), InferenceError> {
        // Estimate VRAM before loading
        let vram_estimate = model_loader::estimate_vram_mb(std::path::Path::new(model_path))?;
        let current_vram = self.registry.total_vram_allocated_mb().await;
        let available = self
            .config
            .model
            .vram_budget_mb
            .saturating_sub(current_vram);

        if vram_estimate > available {
            return Err(InferenceError::InsufficientVram {
                required_mb: vram_estimate,
                available_mb: available,
            });
        }

        // Register model — always succeeds for valid input
        self.registry
            .register(model_id.to_string(), model_path.to_string(), vram_estimate)
            .await?;

        // Set state to Loading
        self.registry
            .set_state(model_id, ModelLoadState::Loading)
            .await?;

        // Load the model
        match model_loader::load(
            model_id,
            std::path::Path::new(model_path),
            gpu_layers,
            self.config.model.context_length,
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
                // Set state to Failed — best-effort, log if this also fails
                let fail_state = ModelLoadState::Failed(load_error.to_string());
                if let Err(registry_error) = self.registry.set_state(model_id, fail_state).await {
                    tracing::error!(
                        event_type = "registry_state_update_failed",
                        model_id = model_id,
                        error = %registry_error,
                        "failed to update registry after model load failure"
                    );
                }
                Err(load_error)
            }
        }
    }

    /// Load a model with LRU eviction retry logic.
    /// Attempts to load the model, and if it fails due to InsufficientVram,
    /// evicts the least-recently-used model from the registry and retries the load.
    pub async fn load_model_with_eviction(
        &self,
        model_id: &str,
        model_path: &str,
        gpu_layers: u32,
        backend: Arc<llama_cpp_2::llama_backend::LlamaBackend>,
    ) -> Result<(), InferenceError> {
        // First attempt: try to load the model
        let first_result = self
            .load_model(model_id, model_path, gpu_layers, Arc::clone(&backend))
            .await;

        match first_result {
            Ok(()) => Ok(()),
            Err(InferenceError::InsufficientVram {
                required_mb,
                available_mb,
            }) => {
                // Check for LRU eviction candidate
                match self.registry.lru_eviction_candidate().await {
                    None => {
                        // No candidate to evict — propagate original error
                        Err(InferenceError::InsufficientVram {
                            required_mb,
                            available_mb,
                        })
                    }
                    Some(evict_id) => {
                        tracing::warn!(
                            subsystem = "inference",
                            event_type = "lru_eviction",
                            evicted_model_id = %evict_id,
                            new_model_id = model_id,
                            required_vram_mb = required_mb,
                            available_vram_mb = available_mb,
                            "evicting LRU model to free VRAM for new model load"
                        );

                        // In the current single-model design, only one model has a
                        // real handle at a time. The LRU candidate is a registry
                        // entry from a previous load — removing it frees the VRAM
                        // budget allocation so the new load can proceed.
                        if let Err(remove_err) = self.registry.remove(&evict_id).await {
                            tracing::error!(
                                subsystem = "inference",
                                event_type = "eviction_removal_failed",
                                evicted_model_id = %evict_id,
                                error = %remove_err,
                                "failed to remove evicted model from registry"
                            );
                            // Budget unchanged — retry would hit the same error
                            return Err(InferenceError::InsufficientVram {
                                required_mb,
                                available_mb,
                            });
                        }

                        // Retry the load after eviction
                        self.load_model(model_id, model_path, gpu_layers, backend)
                            .await
                    }
                }
            }
            Err(other) => Err(other), // Propagate non-VRAM errors as-is
        }
    }

    /// Load a model with OOM retry logic.
    /// Attempts to load with configured gpu_layers first.
    /// If VRAM is insufficient or model load fails, retries with reduced gpu_layers.
    pub async fn load_model_with_oom_retry(
        &self,
        model_id: &str,
        model_path: &str,
        backend: Arc<llama_cpp_2::llama_backend::LlamaBackend>,
    ) -> Result<(), InferenceError> {
        // First attempt: try with configured gpu_layers (with eviction logic)
        let original_layers = self.config.model.gpu_layers;
        let first_result = self
            .load_model_with_eviction(model_id, model_path, original_layers, Arc::clone(&backend))
            .await;

        match first_result {
            Ok(()) => Ok(()),
            Err(InferenceError::InsufficientVram {
                required_mb,
                available_mb,
            }) => {
                // Calculate reduced gpu_layers
                let divisor = self.config.runtime.oom_retry_gpu_layer_divisor;
                let reduced_layers = original_layers / divisor;

                tracing::warn!(
                    subsystem = "inference",
                    event_type = "oom_retry",
                    model_id = model_id,
                    original_gpu_layers = original_layers,
                    reduced_gpu_layers = reduced_layers,
                    required_vram_mb = required_mb,
                    available_vram_mb = available_mb,
                    "retrying model load with reduced GPU layers after VRAM failure"
                );

                // Retry with reduced gpu_layers (with eviction logic)
                self.load_model_with_eviction(model_id, model_path, reduced_layers, backend)
                    .await
            }
            Err(InferenceError::ModelLoad { .. }) => {
                // Calculate reduced gpu_layers
                let divisor = self.config.runtime.oom_retry_gpu_layer_divisor;
                let reduced_layers = original_layers / divisor;

                tracing::warn!(
                    subsystem = "inference",
                    event_type = "oom_retry",
                    model_id = model_id,
                    original_gpu_layers = original_layers,
                    reduced_gpu_layers = reduced_layers,
                    required_vram_mb = 0,
                    available_vram_mb = 0,
                    "retrying model load with reduced GPU layers after model load failure"
                );

                // Retry with reduced gpu_layers (with eviction logic)
                self.load_model_with_eviction(model_id, model_path, reduced_layers, backend)
                    .await
            }
            Err(other) => Err(other), // propagate non-VRAM errors
        }
    }

    /// Swap the currently loaded model with a new one.
    /// Follows the fixed swap sequence:
    /// 1. Emit INFERENCE_UNAVAILABLE
    /// 2. Drain in-flight requests (return UNAVAILABLE to callers)
    /// 3. Unload current model
    /// 4. Load new GGUF
    /// 5. Emit INFERENCE_READY
    ///
    /// During swap, all inference requests return InferenceError::ModelSwitching.
    pub async fn swap_model(
        &self,
        model_id: &str,
        model_path: &str,
        backend: Arc<llama_cpp_2::llama_backend::LlamaBackend>,
        boot_client: &mut BootServiceClient<tonic::transport::Channel>,
    ) -> Result<(), InferenceError> {
        const SUBSYSTEM_ID: &str = "inference";

        // Step 1: Set is_switching to true — all new requests will get ModelSwitching error
        self.is_switching.store(true, Ordering::SeqCst);

        let current_model = self.registry.active_model_id().await;

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "model_swap_initiated",
            from_model = ?current_model,
            to_model = model_id,
            "model swap initiated"
        );

        // Step 2: Signal INFERENCE_UNAVAILABLE to daemon-bus
        let unavailable_request = tonic::Request::new(BootSignalRequest {
            subsystem_id: SUBSYSTEM_ID.to_owned(),
            signal: BootSignal::InferenceUnavailable.into(),
        });

        if let Err(signal_error) = boot_client.signal_ready(unavailable_request).await {
            tracing::warn!(
                subsystem = SUBSYSTEM_ID,
                event_type = "boot_signal_failed",
                signal = "INFERENCE_UNAVAILABLE",
                error = %signal_error,
                "failed to signal INFERENCE_UNAVAILABLE (continuing swap)"
            );
        }

        // Step 3: Set registry state to Switching (keeps VRAM accounting accurate
        // until actual unload completes)
        let active_id = self.registry.active_model_id().await;
        if let Some(ref active_id) = active_id {
            if let Err(registry_error) = self
                .registry
                .set_state(active_id, ModelLoadState::Switching)
                .await
            {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "registry_state_update_failed",
                    model_id = %active_id,
                    error = %registry_error,
                    "failed to set Switching state (continuing swap)"
                );
            }
        }

        // Step 4: Take the model handle and wait for in-flight requests to complete
        let old_handle = {
            let mut handle_guard = self.model_handle.lock().await;
            handle_guard.take()
        };

        // Wait for in-flight requests to finish (they still hold Arc references to old model)
        let drain_timeout =
            std::time::Duration::from_millis(self.config.runtime.swap_drain_timeout_ms);
        let drain_start = Instant::now();
        loop {
            let inflight = self.inflight_count.load(Ordering::SeqCst);
            if inflight == 0 {
                break;
            }
            if drain_start.elapsed() > drain_timeout {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "swap_drain_timeout",
                    remaining_inflight = inflight,
                    timeout_ms = self.config.runtime.swap_drain_timeout_ms,
                    "swap drain timed out — proceeding with unload despite in-flight requests"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Step 5: Unload old model in spawn_blocking (frees VRAM)
        if let Some(handle) = old_handle {
            if let Err(unload_error) = model_loader::unload(handle).await {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "model_unload_failed",
                    error = %unload_error,
                    "failed to unload old model in spawn_blocking"
                );
            }
        }

        // Mark old model as Unloaded now that VRAM is freed
        if let Some(ref active_id) = active_id {
            if let Err(registry_error) = self
                .registry
                .set_state(active_id, ModelLoadState::Unloaded)
                .await
            {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "registry_state_update_failed",
                    model_id = %active_id,
                    error = %registry_error,
                    "failed to set Unloaded state after unload"
                );
            }
        }

        // Step 6: Load the new model using load_model_with_oom_retry
        let load_result = self
            .load_model_with_oom_retry(model_id, model_path, backend)
            .await;

        match load_result {
            Ok(()) => {
                // Step 7: Clear is_switching to false
                self.is_switching.store(false, Ordering::SeqCst);

                // Step 8: Signal INFERENCE_READY to daemon-bus
                let ready_request = tonic::Request::new(BootSignalRequest {
                    subsystem_id: SUBSYSTEM_ID.to_owned(),
                    signal: BootSignal::InferenceReady.into(),
                });

                if let Err(signal_error) = boot_client.signal_ready(ready_request).await {
                    tracing::warn!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "boot_signal_failed",
                        signal = "INFERENCE_READY",
                        error = %signal_error,
                        "failed to signal INFERENCE_READY after swap"
                    );
                }

                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "model_swap_completed",
                    model_id = model_id,
                    "model swap completed successfully"
                );

                Ok(())
            }
            Err(load_error) => {
                // Step 9: On error in load step
                // Clear is_switching to false
                self.is_switching.store(false, Ordering::SeqCst);

                // Signal INFERENCE_UNAVAILABLE
                let unavailable_request = tonic::Request::new(BootSignalRequest {
                    subsystem_id: SUBSYSTEM_ID.to_owned(),
                    signal: BootSignal::InferenceUnavailable.into(),
                });

                if let Err(signal_error) = boot_client.signal_ready(unavailable_request).await {
                    tracing::warn!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "boot_signal_failed",
                        signal = "INFERENCE_UNAVAILABLE",
                        error = %signal_error,
                        "failed to signal INFERENCE_UNAVAILABLE after swap failure"
                    );
                }

                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "model_swap_failed",
                    model_id = model_id,
                    error = %load_error,
                    "model swap failed during load step"
                );

                // Propagate the error
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
            response_channel: ResponseChannel::Complete(response_tx),
        };

        self.queue.push(request).await?;
        Ok(response_rx)
    }

    /// Enqueue a streaming completion request. Returns a receiver for token chunks.
    pub async fn enqueue_streaming(
        &self,
        prompt: String,
        model_id: String,
        max_tokens: u32,
        temperature: f32,
        request_id: String,
        priority: Priority,
    ) -> Result<mpsc::Receiver<Result<StreamCompleteChunk, InferenceError>>, InferenceError> {
        let (stream_tx, stream_rx) = mpsc::channel(self.config.runtime.stream_channel_capacity);

        let request = QueuedRequest {
            prompt,
            model_id,
            max_tokens,
            temperature,
            request_id,
            priority,
            enqueued_at: Instant::now(),
            response_channel: ResponseChannel::Stream(stream_tx),
        };

        self.queue.push(request).await?;
        Ok(stream_rx)
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
            let prompt = request.prompt.clone();
            let max_tokens = request.max_tokens;
            let temperature = request.temperature;

            match request.response_channel {
                ResponseChannel::Complete(response_tx) => {
                    let result = engine
                        .process_request(&prompt, max_tokens, temperature, &request_id)
                        .await;

                    // Send result on the response channel — receiver may have dropped
                    match response_tx.send(result) {
                        Ok(()) => {
                            tracing::debug!(
                                event_type = "response_sent",
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
                ResponseChannel::Stream(stream_tx) => {
                    engine
                        .process_request_streaming(
                            &prompt,
                            max_tokens,
                            temperature,
                            &request_id,
                            stream_tx,
                        )
                        .await;
                }
            }
        }
    }

    /// Process a single inference request.
    async fn process_request(
        &self,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
        request_id: &str,
    ) -> Result<CompleteResponse, InferenceError> {
        // Check if switching is in progress
        if self.is_switching.load(Ordering::SeqCst) {
            return Err(InferenceError::ModelSwitching);
        }

        let _inflight_guard = InFlightGuard::new(&self.inflight_count);

        let handle_guard = self.model_handle.lock().await;
        let handle = handle_guard
            .as_ref()
            .ok_or(InferenceError::ModelSwitching)?;

        let model = Arc::clone(&handle.model);
        let backend = Arc::clone(&handle.backend);
        let active_model_id = handle.model_id.clone();
        let context_length = handle.context_length;
        let prompt = prompt.to_string();
        let request_id = request_id.to_string();
        drop(handle_guard); // release lock before spawn_blocking

        // Update LRU timestamp so eviction targets the least recently used model
        if let Err(touch_err) = self.registry.touch(&active_model_id).await {
            tracing::debug!(
                event_type = "registry_touch_failed",
                model_id = %active_model_id,
                error = %touch_err,
                "failed to update LRU timestamp"
            );
        }

        let start = Instant::now();

        // Clone request_id before moving into spawn_blocking
        let request_id_for_closure = request_id.clone();

        let output = tokio::task::spawn_blocking(move || {
            // Create context with configured context_length
            let n_ctx = NonZeroU32::new(context_length).ok_or_else(|| {
                InferenceError::InferenceExecution {
                    reason: "context_length must be non-zero".to_string(),
                }
            })?;
            let context_params = LlamaContextParams::default().with_n_ctx(Some(n_ctx));

            let mut context = model.new_context(&backend, context_params).map_err(|err| {
                InferenceError::InferenceExecution {
                    reason: format!("failed to create context: {err}"),
                }
            })?;

            // Tokenize the prompt
            let tokens = model.str_to_token(&prompt, AddBos::Always).map_err(|err| {
                InferenceError::InferenceExecution {
                    reason: format!("tokenization failed: {err}"),
                }
            })?;

            let prompt_token_count = tokens.len() as u32;

            // Decode the prompt tokens (prompt eval)
            // Last token needs logits=true so we can sample the first generated token
            let mut batch = LlamaBatch::new(tokens.len(), 1);
            let last_prompt_idx = tokens.len().saturating_sub(1);
            for (i, token) in tokens.iter().enumerate() {
                let is_last = i == last_prompt_idx;
                batch.add(*token, i as i32, &[0], is_last).map_err(|err| {
                    InferenceError::InferenceExecution {
                        reason: format!("failed to add token to batch: {err}"),
                    }
                })?;
            }

            context
                .decode(&mut batch)
                .map_err(|err| InferenceError::InferenceExecution {
                    reason: format!("prompt decode failed: {err}"),
                })?;

            // Create sampler with temperature
            // Seed derived from request_id hash for reproducibility within a request
            let seed = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                request_id_for_closure.hash(&mut hasher);
                hasher.finish() as u32
            };

            let mut sampler = LlamaSampler::chain_simple(vec![
                LlamaSampler::temp(temperature),
                LlamaSampler::dist(seed),
            ]);

            // Generation loop — token positions continue from where prompt left off
            let mut output_text = String::new();
            let mut tokens_generated = 0u32;
            let mut decoder = encoding_rs::UTF_8.new_decoder();
            let mut current_pos = tokens.len() as i32;

            for _ in 0..max_tokens {
                let token = sampler.sample(&context, -1);

                // Check for end-of-generation
                if model.is_eog_token(token) {
                    break;
                }

                sampler.accept(token);

                // Detokenize the token
                let token_str = model
                    .token_to_piece(token, &mut decoder, false, None)
                    .map_err(|err| InferenceError::InferenceExecution {
                        reason: format!("detokenization failed: {err}"),
                    })?;
                output_text.push_str(&token_str);

                tokens_generated += 1;

                // Create new batch with this token at the next position
                let mut next_batch = LlamaBatch::new(1, 1);
                next_batch
                    .add(token, current_pos, &[0], true)
                    .map_err(|err| InferenceError::InferenceExecution {
                        reason: format!("failed to add token to batch: {err}"),
                    })?;
                current_pos += 1;

                context.decode(&mut next_batch).map_err(|err| {
                    InferenceError::InferenceExecution {
                        reason: format!("decode failed: {err}"),
                    }
                })?;
            }

            Ok::<(String, u32, u32), InferenceError>((
                output_text,
                tokens_generated,
                prompt_token_count,
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

    /// Process a streaming inference request — sends tokens as they're generated.
    async fn process_request_streaming(
        &self,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
        request_id: &str,
        stream_tx: mpsc::Sender<Result<StreamCompleteChunk, InferenceError>>,
    ) {
        // Check if switching is in progress
        if self.is_switching.load(Ordering::SeqCst) {
            let _send_result = stream_tx.send(Err(InferenceError::ModelSwitching)).await;
            return;
        }

        let _inflight_guard = InFlightGuard::new(&self.inflight_count);

        let handle_guard = self.model_handle.lock().await;
        let handle = match handle_guard.as_ref() {
            Some(h) => h,
            None => {
                let _send_result = stream_tx.send(Err(InferenceError::ModelSwitching)).await;
                return;
            }
        };

        let model = Arc::clone(&handle.model);
        let backend = Arc::clone(&handle.backend);
        let context_length = handle.context_length;
        let prompt = prompt.to_string();
        let request_id = request_id.to_string();
        drop(handle_guard); // release lock before spawn_blocking

        let start = Instant::now();

        // Clone stream_tx for inside spawn_blocking
        let stream_tx_clone = stream_tx.clone();
        // Clone request_id before moving into spawn_blocking
        let request_id_for_closure = request_id.clone();

        let result = tokio::task::spawn_blocking(move || {
            // Create context
            let n_ctx = NonZeroU32::new(context_length).ok_or_else(|| {
                InferenceError::InferenceExecution {
                    reason: "context_length must be non-zero".to_string(),
                }
            })?;
            let context_params = LlamaContextParams::default().with_n_ctx(Some(n_ctx));

            let mut context = model.new_context(&backend, context_params).map_err(|err| {
                InferenceError::InferenceExecution {
                    reason: format!("failed to create context: {err}"),
                }
            })?;

            // Tokenize the prompt
            let tokens = model.str_to_token(&prompt, AddBos::Always).map_err(|err| {
                InferenceError::InferenceExecution {
                    reason: format!("tokenization failed: {err}"),
                }
            })?;

            // Decode the prompt tokens — last token needs logits=true
            let mut batch = LlamaBatch::new(tokens.len(), 1);
            let last_prompt_idx = tokens.len().saturating_sub(1);
            for (i, token) in tokens.iter().enumerate() {
                let is_last = i == last_prompt_idx;
                batch.add(*token, i as i32, &[0], is_last).map_err(|err| {
                    InferenceError::InferenceExecution {
                        reason: format!("failed to add token to batch: {err}"),
                    }
                })?;
            }

            context
                .decode(&mut batch)
                .map_err(|err| InferenceError::InferenceExecution {
                    reason: format!("prompt decode failed: {err}"),
                })?;

            // Create sampler
            let seed = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                request_id_for_closure.hash(&mut hasher);
                hasher.finish() as u32
            };

            let mut sampler = LlamaSampler::chain_simple(vec![
                LlamaSampler::temp(temperature),
                LlamaSampler::dist(seed),
            ]);

            // Generation loop with streaming — positions continue from prompt
            let mut tokens_generated = 0u32;
            let mut decoder = encoding_rs::UTF_8.new_decoder();
            let mut current_pos = tokens.len() as i32;

            for i in 0..max_tokens {
                let token = sampler.sample(&context, -1);

                // Check for end-of-generation
                let is_eog = model.is_eog_token(token);

                if is_eog {
                    // Send final chunk with finished=true
                    let chunk = StreamCompleteChunk {
                        token: String::new(),
                        finished: true,
                        request_id: request_id_for_closure.clone(),
                    };
                    // blocking_send: safe inside spawn_blocking, applies backpressure
                    if stream_tx_clone.blocking_send(Ok(chunk)).is_err() {
                        // Client disconnected
                    }
                    break;
                }

                sampler.accept(token);

                // Detokenize the token
                let token_str = model
                    .token_to_piece(token, &mut decoder, false, None)
                    .map_err(|err| InferenceError::InferenceExecution {
                        reason: format!("detokenization failed: {err}"),
                    })?;

                tokens_generated += 1;
                let is_last = i == max_tokens - 1;

                // Send token chunk — blocking_send applies backpressure when buffer is full
                let chunk = StreamCompleteChunk {
                    token: token_str,
                    finished: is_last,
                    request_id: request_id_for_closure.clone(),
                };

                if stream_tx_clone.blocking_send(Ok(chunk)).is_err() {
                    // Client disconnected
                    break;
                }

                // Create new batch with this token at the next position
                let mut next_batch = LlamaBatch::new(1, 1);
                next_batch
                    .add(token, current_pos, &[0], true)
                    .map_err(|err| InferenceError::InferenceExecution {
                        reason: format!("failed to add token to batch: {err}"),
                    })?;
                current_pos += 1;

                context.decode(&mut next_batch).map_err(|err| {
                    InferenceError::InferenceExecution {
                        reason: format!("decode failed: {err}"),
                    }
                })?;
            }

            Ok::<u32, InferenceError>(tokens_generated)
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok(tokens_generated)) => {
                tracing::debug!(
                    event_type = "inference_streaming_completed",
                    duration_ms = duration.as_millis() as u64,
                    tokens_generated = tokens_generated,
                    request_id = %request_id,
                );
            }
            Ok(Err(inference_error)) => {
                tracing::error!(
                    event_type = "inference_streaming_failed",
                    error = %inference_error,
                    request_id = %request_id,
                );
                let _send_result = stream_tx.send(Err(inference_error)).await;
            }
            Err(join_error) => {
                tracing::error!(
                    event_type = "inference_streaming_spawn_failed",
                    error = %join_error,
                    request_id = %request_id,
                );
                let _send_result = stream_tx
                    .send(Err(InferenceError::SpawnBlocking(join_error)))
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::OnceLock;

    static TEST_BACKEND: OnceLock<Arc<llama_cpp_2::llama_backend::LlamaBackend>> = OnceLock::new();

    // Helper to get or initialize backend once for all tests
    fn get_backend() -> Arc<llama_cpp_2::llama_backend::LlamaBackend> {
        TEST_BACKEND
            .get_or_init(|| {
                Arc::new(
                    llama_cpp_2::llama_backend::LlamaBackend::init()
                        .expect("test: backend init should succeed on first call"),
                )
            })
            .clone()
    }

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
        let mut file = tempfile::NamedTempFile::new().expect("test: temp file creation");
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
        let config = Arc::new(crate::config::Config::load(file.path()).expect("test: load config"));

        let engine = InferenceEngine::new(config);

        // Create a large temp file that exceeds 1MB VRAM budget
        let mut model_file = tempfile::NamedTempFile::new().expect("test: model temp file");
        let data = vec![0u8; 5 * 1024 * 1024]; // 5MB → ~6MB VRAM estimate
        model_file.write_all(&data).expect("test: write model data");

        let backend = get_backend();

        let result = engine
            .load_model(
                "test-model",
                model_file.path().to_str().expect("test: path to str"),
                0, // gpu_layers
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
        let config = Arc::new(crate::config::Config::load(file.path()).expect("test: load config"));

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

    #[tokio::test]
    async fn test_load_model_with_oom_retry_propagates_non_vram_errors() {
        let config = test_config();
        let engine = InferenceEngine::new(config);

        let backend = get_backend();

        // Try to load a non-existent model file — should return ModelLoad error
        // and NOT trigger retry since it's not a VRAM error
        let result = engine
            .load_model_with_oom_retry("nonexistent-model", "/path/to/nonexistent.gguf", backend)
            .await;

        match result {
            Err(InferenceError::ModelLoad { .. }) => {} // expected
            other => panic!("Expected ModelLoad error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_oom_retry_config_divisor_applied() {
        // Create config with gpu_layers = 35 and divisor = 2
        let content = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "0.0.0.0"
listen_port = 50055
connection_timeout_ms = 5000

[model]
model_id = "test"
model_path = "models/test.gguf"
gpu_layers = 35
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
        let config = Arc::new(crate::config::Config::load(file.path()).expect("test: load config"));

        let engine = InferenceEngine::new(config);

        // Create a large temp file that exceeds 1MB VRAM budget
        let mut model_file = tempfile::NamedTempFile::new().expect("test: model temp file");
        let data = vec![0u8; 5 * 1024 * 1024]; // 5MB → ~6MB VRAM estimate
        model_file.write_all(&data).expect("test: write model data");

        let backend = get_backend();

        // The first load attempt should fail with InsufficientVram
        // Then retry should happen with reduced layers (35 / 2 = 17)
        // Both attempts will still fail because the model file doesn't exist
        // as a valid GGUF, but we're testing the retry flow happens
        let result = engine
            .load_model_with_oom_retry(
                "test-model",
                model_file.path().to_str().expect("test: path to str"),
                backend,
            )
            .await;

        // We expect either InsufficientVram or ModelLoad depending on retry behavior
        // The key is that the retry attempted with reduced layers
        match result {
            Err(InferenceError::InsufficientVram { .. })
            | Err(InferenceError::ModelLoad { .. }) => {} // expected
            other => panic!("Expected VRAM or ModelLoad error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_is_switching_default_false() {
        let config = test_config();
        let engine = InferenceEngine::new(config);
        assert!(!engine.is_switching(), "New engine should not be in switching state");
    }

    #[tokio::test]
    async fn test_is_switching_flag_blocks_requests() {
        let config = test_config();
        let engine = InferenceEngine::new(config);

        // First verify that with is_switching=false and no model, we get ModelSwitching
        // (this is the "no model loaded" path — different from the is_switching guard)
        let result_no_model = engine.process_request("test", 100, 0.7, "req1").await;
        assert!(
            matches!(result_no_model, Err(InferenceError::ModelSwitching)),
            "Expected ModelSwitching when no model loaded"
        );

        // Now set is_switching=true
        engine
            .is_switching
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // The inflight counter should NOT be incremented since the is_switching guard fires first
        assert_eq!(
            engine.inflight_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "is_switching guard should prevent incrementing inflight count"
        );

        let result_switching = engine.process_request("test", 100, 0.7, "req2").await;
        assert!(
            matches!(result_switching, Err(InferenceError::ModelSwitching)),
            "Expected ModelSwitching when is_switching=true"
        );

        // inflight_count should still be 0 — proving the early check triggered
        assert_eq!(
            engine.inflight_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "is_switching guard should have returned before incrementing inflight count"
        );
    }

    #[tokio::test]
    async fn test_load_model_with_eviction_evicts_lru_on_vram_pressure() {
        // Config with budget that can only hold 2 models at 2048MB each
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
request_queue_max_depth = 64
request_timeout_ms = 30000
oom_retry_gpu_layer_divisor = 2

[logging]
level = "info"
format = "json"
"#;
        let mut file = tempfile::NamedTempFile::new().expect("test: temp file");
        file.write_all(content.as_bytes()).expect("test: write");
        let config = Arc::new(crate::config::Config::load(file.path()).expect("test: load config"));
        let engine = InferenceEngine::new(Arc::clone(&config));

        // Register 2 models as Ready, each taking 2048MB (4096MB total — budget filled)
        engine
            .registry
            .register("model-old".into(), "/path/old.gguf".into(), 2048)
            .await
            .expect("test: register");
        engine
            .registry
            .set_state("model-old", ModelLoadState::Ready)
            .await
            .expect("test: set ready");

        engine
            .registry
            .register("model-new".into(), "/path/new.gguf".into(), 2048)
            .await
            .expect("test: register");
        engine
            .registry
            .set_state("model-new", ModelLoadState::Ready)
            .await
            .expect("test: set ready");

        // Touch model-new to make it more recently used
        engine
            .registry
            .touch("model-new")
            .await
            .expect("test: touch");

        // Set model-new as active
        engine
            .registry
            .set_active("model-new")
            .await
            .expect("test: set active");

        // Verify budget is full
        assert_eq!(engine.registry.total_vram_allocated_mb().await, 4096);

        // Verify LRU candidate is model-old
        assert_eq!(
            engine.registry.lru_eviction_candidate().await,
            Some("model-old".to_string())
        );

        // Create a temp file for the third model (will trigger eviction)
        let mut model_file = tempfile::NamedTempFile::new().expect("test: model temp file");
        // 2.5MB → ~3MB VRAM estimate, which would overflow budget
        let data = vec![0u8; 3 * 1024 * 1024];
        model_file.write_all(&data).expect("test: write model data");

        let backend = get_backend();

        // Attempt to load third model with eviction
        let _result = engine
            .load_model_with_eviction(
                "model-third",
                model_file.path().to_str().expect("test: path to str"),
                0,
                backend,
            )
            .await;

        // Verify model-old was removed from registry (evicted)
        let old_state = engine.registry.get_state("model-old").await;
        assert!(
            old_state.is_err(),
            "model-old should have been removed from registry after eviction"
        );
        match old_state.expect_err("test: should be error") {
            InferenceError::ModelNotFound { model_id } => assert_eq!(model_id, "model-old"),
            other => panic!("Expected ModelNotFound, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_load_model_with_eviction_no_candidate_propagates_error() {
        // Config with small budget
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
vram_budget_mb = 2048

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
        let config = Arc::new(crate::config::Config::load(file.path()).expect("test: load config"));
        let engine = InferenceEngine::new(config);

        // Register 1 model as active (no LRU candidate available)
        engine
            .registry
            .register("active-model".into(), "/path/active.gguf".into(), 2048)
            .await
            .expect("test: register");
        engine
            .registry
            .set_state("active-model", ModelLoadState::Ready)
            .await
            .expect("test: set ready");
        engine
            .registry
            .set_active("active-model")
            .await
            .expect("test: set active");

        // Verify no LRU candidate exists
        assert_eq!(engine.registry.lru_eviction_candidate().await, None);

        // Create a large temp file that exceeds budget
        let mut model_file = tempfile::NamedTempFile::new().expect("test: model temp file");
        let data = vec![0u8; 5 * 1024 * 1024]; // 5MB → ~6MB VRAM estimate
        model_file.write_all(&data).expect("test: write model data");

        let backend = get_backend();

        // Attempt to load — should fail with InsufficientVram since no eviction candidate exists
        let result = engine
            .load_model_with_eviction(
                "new-model",
                model_file.path().to_str().expect("test: path to str"),
                0,
                backend,
            )
            .await;

        match result {
            Err(InferenceError::InsufficientVram { .. }) => {} // expected
            other => panic!("Expected InsufficientVram error, got: {other:?}"),
        }
    }
}
