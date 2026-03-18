//! gRPC service implementation for InferenceService.
//!
//! Wires Complete and StreamComplete to the inference engine.
//! ReadActivations and Steer return unimplemented status stubs.

use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::generated::sena_daemonbus_v1::{
    inference_service_server::InferenceService,
    ActivationRequest, ActivationResponse,
    CompleteRequest, CompleteResponse,
    SteeringAck, SteeringRequest,
    StreamCompleteChunk,
};
use crate::inference_engine::InferenceEngine;
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

        let receiver = self.engine.enqueue(
            req.prompt,
            req.model_id,
            req.max_tokens,
            req.temperature,
            request_id,
            priority,
        ).await.map_err(|e| tonic::Status::from(e))?;

        // Await the inference result
        let result = receiver.await.map_err(|_recv_error| {
            Status::internal("inference worker dropped the response channel")
        })?;

        let response = result.map_err(|e| tonic::Status::from(e))?;
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
        let request_id_for_stream = request_id.clone();

        let priority = Priority::from_proto(req.priority);

        let receiver = self.engine.enqueue(
            req.prompt,
            req.model_id,
            req.max_tokens,
            req.temperature,
            request_id,
            priority,
        ).await.map_err(|e| tonic::Status::from(e))?;

        // Phase 1: Buffer full completion, then stream tokens from result
        // FIXME(inference): Phase 2 will refactor to stream tokens during generation
        let (tx, rx) = tokio::sync::mpsc::channel(32);

        tokio::spawn(async move {
            let result = receiver.await;
            match result {
                Ok(Ok(complete_response)) => {
                    // Split the response text into tokens (words for Phase 1)
                    // FIXME(inference): Phase 2 will stream actual tokens during generation
                    let tokens: Vec<&str> = complete_response.text.split_whitespace().collect();
                    let total = tokens.len();
                    
                    for (idx, token) in tokens.iter().enumerate() {
                        let is_last = idx == total - 1;
                        let chunk = StreamCompleteChunk {
                            token: token.to_string(),
                            finished: is_last,
                            request_id: request_id_for_stream.clone(),
                        };
                        if tx.send(Ok(chunk)).await.is_err() {
                            // Client disconnected
                            break;
                        }
                    }
                    // If no tokens, send a single finished chunk
                    if total == 0 {
                        let chunk = StreamCompleteChunk {
                            token: String::new(),
                            finished: true,
                            request_id: request_id_for_stream,
                        };
                        // Client may have disconnected — that's fine
                        let _send_result = tx.send(Ok(chunk)).await;
                    }
                }
                Ok(Err(inference_error)) => {
                    let _send_result = tx.send(Err(tonic::Status::from(inference_error))).await;
                }
                Err(_recv_error) => {
                    let _send_result = tx.send(Err(Status::internal(
                        "inference worker dropped the response channel",
                    ))).await;
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
            "ReadActivations is reserved for Phase 2"
        );
        Err(Status::unimplemented("ReadActivations is reserved for Phase 2"))
    }

    async fn steer(
        &self,
        _request: Request<SteeringRequest>,
    ) -> Result<Response<SteeringAck>, Status> {
        tracing::debug!(
            event_type = "rpc_unimplemented",
            rpc = "Steer",
            "Steer is reserved for Phase 3"
        );
        Err(Status::unimplemented("Steer is reserved for Phase 3"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use crate::config::Config;

    fn test_config() -> Arc<Config> {
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
        let mut file = tempfile::NamedTempFile::new()
            .expect("test: temp file creation");
        file.write_all(content.as_bytes())
            .expect("test: write config");
        let config = Config::load(file.path())
            .expect("test: valid config");
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
        let _worker = tokio::spawn(async move {
            engine_clone.run_worker().await
        });

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
        let mut file = tempfile::NamedTempFile::new()
            .expect("test: temp file");
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
        engine.enqueue("first".into(), String::new(), 10, 0.7, "r1".into(), Priority::Standard)
            .await.expect("test: first enqueue");
        
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
}
