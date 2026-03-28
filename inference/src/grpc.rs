//! gRPC service implementation for InferenceService.
//!
//! Wires Complete and StreamComplete to the inference engine.
//! ReadActivations and Steer return unimplemented status stubs.

use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::generated::sena_daemonbus_v1::{
    inference_service_server::InferenceService, ActivationRequest, ActivationResponse,
    CompleteRequest, CompleteResponse, ListModelsRequest, ListModelsResponse, LoadModelRequest,
    LoadModelResponse, ModelInfo, SteeringAck, SteeringRequest, StreamCompleteChunk,
    UnloadModelRequest, UnloadModelResponse,
};
use crate::inference_engine::InferenceEngine;
use crate::model_registry::ModelLoadState;
use crate::request_queue::Priority;

/// gRPC service implementation wrapping the inference engine.
pub struct InferenceGrpcService {
    engine: Arc<InferenceEngine>,
}

impl InferenceGrpcService {
    pub fn new(engine: Arc<InferenceEngine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl InferenceService for InferenceGrpcService {
    async fn complete(
        &self,
        request: Request<CompleteRequest>,
    ) -> Result<Response<CompleteResponse>, Status> {
        let req = request.into_inner();
        let request_id = if req.request_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            req.request_id.clone()
        };

        let priority = Priority::from_proto(req.priority);

        let receiver = self
            .engine
            .enqueue(
                req.prompt,
                req.model_id,
                req.max_tokens,
                req.temperature,
                request_id,
                priority,
            )
            .await
            .map_err(tonic::Status::from)?;

        // Await the inference result
        let result = receiver.await.map_err(|_recv_error| {
            Status::internal("inference worker dropped the response channel")
        })?;

        let response = result.map_err(tonic::Status::from)?;
        Ok(Response::new(response))
    }

    type StreamCompleteStream = ReceiverStream<Result<StreamCompleteChunk, Status>>;

    async fn stream_complete(
        &self,
        request: Request<CompleteRequest>,
    ) -> Result<Response<Self::StreamCompleteStream>, Status> {
        let req = request.into_inner();
        let request_id = if req.request_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            req.request_id.clone()
        };

        let priority = Priority::from_proto(req.priority);

        // Use the new streaming enqueue — tokens arrive as they're generated
        let mut receiver = self
            .engine
            .enqueue_streaming(
                req.prompt,
                req.model_id,
                req.max_tokens,
                req.temperature,
                request_id,
                priority,
            )
            .await
            .map_err(tonic::Status::from)?;

        // Convert InferenceError to tonic::Status in the stream
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            while let Some(result) = receiver.recv().await {
                let mapped = result.map_err(tonic::Status::from);
                if tx.send(mapped).await.is_err() {
                    // Client disconnected
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn read_activations(
        &self,
        _request: Request<ActivationRequest>,
    ) -> Result<Response<ActivationResponse>, Status> {
        tracing::debug!(
            event_type = "rpc_unimplemented",
            rpc = "ReadActivations",
            "ReadActivations deferred to Milestone D (activation steering)"
        );
        Err(Status::unimplemented(
            "ReadActivations deferred to Milestone D",
        ))
    }

    async fn steer(
        &self,
        _request: Request<SteeringRequest>,
    ) -> Result<Response<SteeringAck>, Status> {
        tracing::debug!(
            event_type = "rpc_unimplemented",
            rpc = "Steer",
            "Steer deferred to Milestone D (activation steering)"
        );
        Err(Status::unimplemented("Steer deferred to Milestone D"))
    }

    async fn list_models(
        &self,
        _request: Request<ListModelsRequest>,
    ) -> Result<Response<ListModelsResponse>, Status> {
        let models = self.engine.registry().list_all().await;

        let model_infos: Vec<ModelInfo> = models
            .into_iter()
            .map(|entry| {
                let status = match entry.state {
                    ModelLoadState::Unloaded => "unloaded",
                    ModelLoadState::Loading => "loading",
                    ModelLoadState::Ready => "ready",
                    ModelLoadState::Failed(_) => "failed",
                    ModelLoadState::Switching => "switching",
                };

                ModelInfo {
                    model_id: entry.model_id,
                    status: status.to_string(),
                    path: entry.model_path,
                    vram_usage_mb: entry.vram_estimate_mb as u64,
                    display_name: entry.display_name,
                }
            })
            .collect();

        tracing::debug!(
            event_type = "list_models_completed",
            model_count = model_infos.len(),
            "listed all models from registry"
        );

        Ok(Response::new(ListModelsResponse {
            models: model_infos,
        }))
    }

    async fn load_model(
        &self,
        request: Request<LoadModelRequest>,
    ) -> Result<Response<LoadModelResponse>, Status> {
        let req = request.into_inner();

        // Validate model_path exists
        let model_path = std::path::Path::new(&req.model_path);
        if !model_path.exists() {
            tracing::warn!(
                event_type = "load_model_path_not_found",
                path = %req.model_path,
                "model path does not exist"
            );
            return Err(Status::not_found(format!(
                "model path does not exist: {}",
                req.model_path
            )));
        }

        // Derive model_id from filename if not provided
        let model_id = if req.model_id.is_empty() {
            model_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown-model")
                .to_string()
        } else {
            req.model_id.clone()
        };

        // Initialize llama backend (required for load_model)
        let backend = Arc::new(
            llama_cpp_2::llama_backend::LlamaBackend::init().map_err(|e| {
                Status::internal(format!("failed to initialize llama backend: {}", e))
            })?,
        );

        // Attempt to load the model
        let load_result = self
            .engine
            .load_model(&model_id, &req.model_path, req.gpu_layers, backend)
            .await;

        match load_result {
            Ok(()) => {
                tracing::info!(
                    event_type = "model_loaded_via_rpc",
                    model_id = %model_id,
                    path = %req.model_path,
                    gpu_layers = req.gpu_layers,
                    "model loaded successfully via LoadModel RPC"
                );

                Ok(Response::new(LoadModelResponse {
                    model_id,
                    status: "ready".to_string(),
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                let error_msg = e.to_string();
                tracing::error!(
                    event_type = "model_load_failed_via_rpc",
                    model_id = %model_id,
                    path = %req.model_path,
                    error = %error_msg,
                    "model load failed via LoadModel RPC"
                );

                // Return error as tonic::Status
                Err(Status::from(e))
            }
        }
    }

    async fn unload_model(
        &self,
        request: Request<UnloadModelRequest>,
    ) -> Result<Response<UnloadModelResponse>, Status> {
        let req = request.into_inner();

        let unload_result = self.engine.unload_model(&req.model_id).await;

        match unload_result {
            Ok(()) => {
                tracing::info!(
                    event_type = "model_unloaded_via_rpc",
                    model_id = %req.model_id,
                    "model unloaded successfully via UnloadModel RPC"
                );

                Ok(Response::new(UnloadModelResponse {
                    success: true,
                    message: format!("model '{}' unloaded successfully", req.model_id),
                }))
            }
            Err(e) => {
                tracing::error!(
                    event_type = "model_unload_failed_via_rpc",
                    model_id = %req.model_id,
                    error = %e,
                    "model unload failed via UnloadModel RPC"
                );

                Err(Status::from(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::io::Write;

    fn test_config() -> Arc<Config> {
        let content = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "127.0.0.1"
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
        let config = Config::load(file.path()).expect("test: valid config");
        Arc::new(config)
    }

    fn test_engine() -> Arc<InferenceEngine> {
        Arc::new(InferenceEngine::new(test_config()))
    }

    #[tokio::test]
    async fn test_read_activations_returns_unimplemented() {
        let service = InferenceGrpcService::new(test_engine());
        let request = Request::new(ActivationRequest {
            layer: 0,
            request_id: "test".into(),
        });
        let result = service.read_activations(request).await;
        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
        assert_eq!(status.code(), tonic::Code::Unimplemented);
    }

    #[tokio::test]
    async fn test_steer_returns_unimplemented() {
        let service = InferenceGrpcService::new(test_engine());
        let request = Request::new(SteeringRequest {
            layer: 0,
            direction: vec![],
            magnitude: 0.0,
            request_id: "test".into(),
        });
        let result = service.steer(request).await;
        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
        assert_eq!(status.code(), tonic::Code::Unimplemented);
    }

    #[tokio::test]
    async fn test_complete_enqueues_request() {
        let engine = test_engine();
        let service = InferenceGrpcService::new(Arc::clone(&engine));

        // Spawn a worker task to process requests
        let engine_clone = Arc::clone(&engine);
        let _worker = tokio::spawn(async move { engine_clone.run_worker().await });

        let request = Request::new(CompleteRequest {
            prompt: "test prompt".into(),
            model_id: String::new(),
            max_tokens: 10,
            temperature: 0.7,
            priority: 1,
            request_id: "test-req".into(),
        });

        // This will enqueue and then the worker processes it
        // Since no model is loaded, the worker will return ModelSwitching error
        // which maps to Status::unavailable
        let result = service.complete(request).await;

        // We expect either a response (if worker processed with placeholder)
        // or an error (if no model loaded). Either way, it proves enqueue worked.
        // With current placeholder implementation and no model loaded, expects error.
        match result {
            Ok(response) => {
                assert_eq!(response.into_inner().request_id, "test-req");
            }
            Err(status) => {
                // ModelSwitching (no model handle) → Unavailable is expected
                assert_eq!(status.code(), tonic::Code::Unavailable);
            }
        }
    }

    #[tokio::test]
    async fn test_complete_queue_full_returns_resource_exhausted() {
        // Config with max_depth = 1
        let content = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "127.0.0.1"
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
        let config = Arc::new(Config::load(file.path()).expect("test: load"));

        let engine = Arc::new(InferenceEngine::new(config));
        let service = InferenceGrpcService::new(Arc::clone(&engine));

        // Fill the queue (don't start worker — queue stays full)
        let _first_req = Request::new(CompleteRequest {
            prompt: "first".into(),
            model_id: String::new(),
            max_tokens: 10,
            temperature: 0.7,
            priority: 3,
            request_id: "r1".into(),
        });

        // Enqueue directly through engine so first request sits in queue
        engine
            .enqueue(
                "first".into(),
                String::new(),
                10,
                0.7,
                "r1".into(),
                Priority::Standard,
            )
            .await
            .expect("test: first enqueue");

        // Second request through gRPC should fail
        let second_req = Request::new(CompleteRequest {
            prompt: "second".into(),
            model_id: String::new(),
            max_tokens: 10,
            temperature: 0.7,
            priority: 3,
            request_id: "r2".into(),
        });

        let result = service.complete(second_req).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::ResourceExhausted);
    }

    #[tokio::test]
    async fn test_list_models_empty() {
        let service = InferenceGrpcService::new(test_engine());
        let request = Request::new(crate::generated::sena_daemonbus_v1::ListModelsRequest {});
        let result = service.list_models(request).await;

        assert!(result.is_ok());
        let response = result.unwrap(); // test: just confirmed is_ok
        assert_eq!(response.into_inner().models.len(), 0);
    }

    #[tokio::test]
    async fn test_load_model_validates_path() {
        let service = InferenceGrpcService::new(test_engine());
        let request = Request::new(crate::generated::sena_daemonbus_v1::LoadModelRequest {
            model_path: "/nonexistent/invalid/path.gguf".into(),
            model_id: "test-model".into(),
            gpu_layers: 0,
        });

        let result = service.load_model(request).await;
        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
                                          // Should return NotFound or Internal for invalid path
        assert!(
            status.code() == tonic::Code::NotFound || status.code() == tonic::Code::Internal,
            "Expected NotFound or Internal, got {:?}",
            status.code()
        );
    }

    #[tokio::test]
    async fn test_unload_model_not_found() {
        let service = InferenceGrpcService::new(test_engine());
        let request = Request::new(crate::generated::sena_daemonbus_v1::UnloadModelRequest {
            model_id: "nonexistent-model-id".into(),
        });

        let result = service.unload_model(request).await;
        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_list_models_with_registered_models() {
        let engine = test_engine();
        let service = InferenceGrpcService::new(Arc::clone(&engine));

        // Register some test models directly in the registry
        engine
            .registry()
            .register(
                "model-a".to_string(),
                "/path/to/model-a.gguf".to_string(),
                2048,
                "Model A Display Name".to_string(),
            )
            .await
            .expect("test: register model-a");

        engine
            .registry()
            .set_state("model-a", ModelLoadState::Ready)
            .await
            .expect("test: set model-a ready");

        engine
            .registry()
            .register(
                "model-b".to_string(),
                "/path/to/model-b.gguf".to_string(),
                4096,
                "Model B Display Name".to_string(),
            )
            .await
            .expect("test: register model-b");

        engine
            .registry()
            .set_state("model-b", ModelLoadState::Unloaded)
            .await
            .expect("test: set model-b unloaded");

        // Call ListModels
        let request = Request::new(ListModelsRequest {});
        let result = service.list_models(request).await;

        assert!(result.is_ok());
        let response = result.unwrap(); // test: just confirmed is_ok
        let models = response.into_inner().models;

        // Should have 2 models
        assert_eq!(models.len(), 2);

        // Find model-a and verify fields
        let model_a = models
            .iter()
            .find(|m| m.model_id == "model-a")
            .expect("test: model-a exists");
        assert_eq!(model_a.status, "ready");
        assert_eq!(model_a.path, "/path/to/model-a.gguf");
        assert_eq!(model_a.vram_usage_mb, 2048);
        assert_eq!(model_a.display_name, "Model A Display Name");

        // Find model-b and verify fields
        let model_b = models
            .iter()
            .find(|m| m.model_id == "model-b")
            .expect("test: model-b exists");
        assert_eq!(model_b.status, "unloaded");
        assert_eq!(model_b.path, "/path/to/model-b.gguf");
        assert_eq!(model_b.vram_usage_mb, 4096);
        assert_eq!(model_b.display_name, "Model B Display Name");
    }

    #[tokio::test]
    async fn test_unload_model_idempotent() {
        let engine = test_engine();
        let service = InferenceGrpcService::new(Arc::clone(&engine));

        // Register a model
        engine
            .registry()
            .register(
                "test-model".to_string(),
                "/path/to/test.gguf".to_string(),
                1024,
                "Test Model".to_string(),
            )
            .await
            .expect("test: register");

        // First unload (model is already unloaded by default)
        let request1 = Request::new(UnloadModelRequest {
            model_id: "test-model".to_string(),
        });
        let result1 = service.unload_model(request1).await;
        assert!(result1.is_ok(), "first unload should succeed");
        let response1 = result1.unwrap(); // test: just confirmed is_ok
        assert!(response1.into_inner().success);

        // Second unload (should be idempotent)
        let request2 = Request::new(UnloadModelRequest {
            model_id: "test-model".to_string(),
        });
        let result2 = service.unload_model(request2).await;
        assert!(
            result2.is_ok(),
            "second unload should also succeed (idempotent)"
        );
        let response2 = result2.unwrap(); // test: just confirmed is_ok
        assert!(response2.into_inner().success);
    }

    #[tokio::test]
    async fn test_load_model_empty_model_id_derives_from_filename() {
        // Create a temporary file to test path validation
        use tempfile::NamedTempFile;
        let temp_file = NamedTempFile::new().expect("test: create temp file");
        let temp_path = temp_file.path().to_str().expect("test: path to string");

        let service = InferenceGrpcService::new(test_engine());
        let request = Request::new(LoadModelRequest {
            model_path: temp_path.to_string(),
            model_id: String::new(), // Empty model_id — should be derived
            gpu_layers: 0,
        });

        // Will fail during actual model load, but should get past path validation
        let result = service.load_model(request).await;

        // We expect an error during actual llama backend init or model load
        // because it's not a real GGUF file, but it should NOT be NotFound
        if let Err(status) = result {
            // Path exists, so it shouldn't be NotFound
            assert_ne!(
                status.code(),
                tonic::Code::NotFound,
                "Should get past path validation, got: {:?}",
                status
            );
        }
    }
}
