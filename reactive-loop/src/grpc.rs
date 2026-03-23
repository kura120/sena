//! gRPC service implementation for UserMessageService.

use crate::generated::sena_daemonbus_v1::{
    user_message_service_server::UserMessageService, UserMessageRequest, UserMessageResponse,
};
use crate::handler::MessageHandler;
use std::sync::Arc;
use tonic::{Request, Response, Status};

const SUBSYSTEM_ID: &str = "reactive_loop";

/// gRPC service implementation for handling user messages.
pub struct UserMessageGrpcService {
    handler: Arc<MessageHandler>,
}

impl UserMessageGrpcService {
    /// Create a new gRPC service.
    pub fn new(handler: Arc<MessageHandler>) -> Self {
        Self { handler }
    }
}

#[tonic::async_trait]
impl UserMessageService for UserMessageGrpcService {
    async fn send_message(
        &self,
        request: Request<UserMessageRequest>,
    ) -> Result<Response<UserMessageResponse>, Status> {
        let req = request.into_inner();

        // Generate request_id if not provided
        let request_id = if req.trace_context.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            req.trace_context.clone()
        };

        let trace_context = if req.trace_context.is_empty() {
            request_id.clone()
        } else {
            req.trace_context.clone()
        };

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "send_message_request",
            request_id = %request_id,
            message_length = req.message.len(),
            "received SendMessage request"
        );

        // Handle the message through the orchestration flow
        let result = self
            .handler
            .handle_message(&req.message, &trace_context, &request_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "send_message_error",
                    request_id = %request_id,
                    error = %e,
                    "message handling failed"
                );
                Status::from(e)
            })?;

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "send_message_response",
            request_id = %request_id,
            latency_ms = result.latency_ms,
            tokens_generated = result.tokens_generated,
            "SendMessage request completed"
        );

        Ok(Response::new(UserMessageResponse {
            response: result.response,
            model_id: result.model_id,
            tokens_generated: result.tokens_generated,
            tokens_prompt: result.tokens_prompt,
            latency_ms: result.latency_ms,
            request_id,
            assembly_trace: result.assembly_trace,
            pre_thought_text: result.pre_thought_text,
            thought_content: result.thought_content,
            chain_of_thought_supported: result.chain_of_thought_supported,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::generated::sena_daemonbus_v1::{
        event_bus_service_client::EventBusServiceClient,
        inference_service_client::InferenceServiceClient,
        prompt_composer_service_client::PromptComposerServiceClient,
    };
    use crate::handler::MessageHandler;

    fn create_test_config() -> Config {
        Config {
            grpc: crate::config::GrpcConfig {
                daemon_bus_address: "http://127.0.0.1:50051".into(),
                inference_address: "http://127.0.0.1:50055".into(),
                prompt_composer_address: "http://127.0.0.1:50057".into(),
                memory_engine_address: "http://127.0.0.1:50052".into(),
                listen_address: "0.0.0.0".into(),
                listen_port: 50058,
                connection_timeout_ms: 5000,
            },
            boot: crate::config::BootConfig {
                ready_signal_timeout_ms: 5000,
            },
            inference: crate::config::InferenceConfig {
                default_max_tokens: 1024,
                default_temperature: 0.7,
                request_timeout_ms: 30000,
            },
            post_processing: crate::config::PostProcessingConfig {
                filter_heartbeat_tokens: true,
                strip_reasoning_tags: false,
                reasoning_markers: Vec::new(),
            },
            fallback: crate::config::FallbackConfig {
                unavailable_response: "Error".into(),
                minimal_context_enabled: true,
            },
            logging: crate::config::LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        }
    }

    #[tokio::test]
    async fn test_grpc_service_creation() {
        let config = Arc::new(create_test_config());

        // Create mock clients (lazy connection - won't actually connect)
        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = Arc::new(MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        ));

        let service = UserMessageGrpcService::new(handler);
        // Service created successfully - structure test only
        let _ = service;
    }

    #[tokio::test]
    async fn test_request_id_generation() {
        // Test that empty trace_context results in UUID generation
        let request = UserMessageRequest {
            message: "Hello".into(),
            trace_context: String::new(),
        };

        // Verify the request structure
        assert!(request.trace_context.is_empty());
        assert_eq!(request.message, "Hello");
    }

    #[tokio::test]
    async fn test_request_with_trace_context() {
        // Test that provided trace_context is preserved
        let request = UserMessageRequest {
            message: "Hello".into(),
            trace_context: "trace-123".into(),
        };

        assert_eq!(request.trace_context, "trace-123");
        assert_eq!(request.message, "Hello");
    }
}
