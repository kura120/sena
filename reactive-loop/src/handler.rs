//! Core message handling logic for reactive-loop.

use crate::config::Config;
use crate::error::ReactiveLoopError;
use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient, inference_service_client::InferenceServiceClient,
    prompt_composer_service_client::PromptComposerServiceClient, AssemblePromptRequest,
    BusEvent, CompleteRequest, CompleteResponse, EventTopic, ModelProfile,
    PromptAssemblyTrace, PromptContext, PublishRequest,
};
use std::sync::Arc;
use std::time::Instant;
use tonic::transport::Channel;

const SUBSYSTEM_ID: &str = "reactive_loop";

/// Result of handling a user message.
#[derive(Debug, Clone)]
pub struct HandlerResult {
    pub response: String,
    pub model_id: String,
    pub tokens_generated: u32,
    pub tokens_prompt: u32,
    pub latency_ms: u64,
    pub assembly_trace: Option<PromptAssemblyTrace>,
}

/// Message handler orchestrating the full conversation flow.
pub struct MessageHandler {
    config: Arc<Config>,
    event_bus_client: Arc<tokio::sync::Mutex<EventBusServiceClient<Channel>>>,
    inference_client: Arc<tokio::sync::Mutex<InferenceServiceClient<Channel>>>,
    prompt_composer_client: Arc<tokio::sync::Mutex<PromptComposerServiceClient<Channel>>>,
}

impl MessageHandler {
    /// Create a new message handler.
    pub fn new(
        config: Arc<Config>,
        event_bus_client: EventBusServiceClient<Channel>,
        inference_client: InferenceServiceClient<Channel>,
        prompt_composer_client: PromptComposerServiceClient<Channel>,
    ) -> Self {
        Self {
            config,
            event_bus_client: Arc::new(tokio::sync::Mutex::new(event_bus_client)),
            inference_client: Arc::new(tokio::sync::Mutex::new(inference_client)),
            prompt_composer_client: Arc::new(tokio::sync::Mutex::new(prompt_composer_client)),
        }
    }

    /// Handle a user message through the full orchestration flow.
    pub async fn handle_message(
        &self,
        message: &str,
        trace_context: &str,
        request_id: &str,
    ) -> Result<HandlerResult, ReactiveLoopError> {
        let start_time = Instant::now();

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "user_message_received",
            request_id = %request_id,
            message_length = message.len(),
            "processing user message"
        );

        // Step 1: Publish TOPIC_USER_MESSAGE_RECEIVED
        self.publish_user_message_received(message, trace_context, request_id)
            .await?;

        // Step 2: Build minimal PromptContext (CTP doesn't exist yet)
        let context = self.build_minimal_context(message, trace_context);

        // Step 3: Assemble prompt via prompt-composer (with fallback)
        let (assembled_prompt, assembly_trace) = self
            .assemble_prompt_with_fallback(context, request_id)
            .await?;

        // Step 4: Send to inference (with fallback)
        let inference_result = self
            .complete_with_fallback(&assembled_prompt, trace_context, request_id)
            .await?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        // Step 5: Publish TOPIC_USER_MESSAGE_RESPONSE
        self.publish_user_message_response(&inference_result.text, trace_context, request_id)
            .await?;

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "user_message_completed",
            request_id = %request_id,
            latency_ms = latency_ms,
            tokens_generated = inference_result.tokens_generated,
            "message handling completed"
        );

        Ok(HandlerResult {
            response: inference_result.text,
            model_id: inference_result.model_id,
            tokens_generated: inference_result.tokens_generated,
            tokens_prompt: inference_result.tokens_prompt,
            latency_ms,
            assembly_trace,
        })
    }

    /// Publish TOPIC_USER_MESSAGE_RECEIVED event.
    async fn publish_user_message_received(
        &self,
        message: &str,
        trace_context: &str,
        request_id: &str,
    ) -> Result<(), ReactiveLoopError> {
        let event = BusEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic: EventTopic::TopicUserMessageReceived.into(),
            source_subsystem: SUBSYSTEM_ID.to_owned(),
            payload: message.as_bytes().to_vec(),
            trace_context: trace_context.to_owned(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let mut client = self.event_bus_client.lock().await;
        let request = tonic::Request::new(PublishRequest { event: Some(event) });

        match client.publish(request).await {
            Ok(_) => {
                tracing::debug!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_published",
                    topic = "TOPIC_USER_MESSAGE_RECEIVED",
                    request_id = %request_id,
                    "event published successfully"
                );
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_publish_failed",
                    topic = "TOPIC_USER_MESSAGE_RECEIVED",
                    request_id = %request_id,
                    error = %e,
                    "failed to publish event, continuing anyway"
                );
                // Don't fail the request if event publishing fails
                Ok(())
            }
        }
    }

    /// Publish TOPIC_USER_MESSAGE_RESPONSE event.
    async fn publish_user_message_response(
        &self,
        response: &str,
        trace_context: &str,
        request_id: &str,
    ) -> Result<(), ReactiveLoopError> {
        let event = BusEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic: EventTopic::TopicUserMessageResponse.into(),
            source_subsystem: SUBSYSTEM_ID.to_owned(),
            payload: response.as_bytes().to_vec(),
            trace_context: trace_context.to_owned(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let mut client = self.event_bus_client.lock().await;
        let request = tonic::Request::new(PublishRequest { event: Some(event) });

        match client.publish(request).await {
            Ok(_) => {
                tracing::debug!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_published",
                    topic = "TOPIC_USER_MESSAGE_RESPONSE",
                    request_id = %request_id,
                    "event published successfully"
                );
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_publish_failed",
                    topic = "TOPIC_USER_MESSAGE_RESPONSE",
                    request_id = %request_id,
                    error = %e,
                    "failed to publish event, continuing anyway"
                );
                // Don't fail the request if event publishing fails
                Ok(())
            }
        }
    }

    /// Build a minimal PromptContext since CTP doesn't exist yet.
    fn build_minimal_context(&self, message: &str, trace_context: &str) -> PromptContext {
        PromptContext {
            soulbox_snapshot: String::new(),
            user_intent: message.to_owned(),
            user_message: message.to_owned(),
            short_term: Vec::new(),
            long_term: Vec::new(),
            episodic: Vec::new(),
            os_context: String::new(),
            telemetry_signals: Vec::new(),
            model_profile: Some(ModelProfile {
                model_id: String::new(),
                context_window: 4096,
                output_reserve: 512,
            }),
            trace_context: trace_context.to_owned(),
        }
    }

    /// Assemble prompt via prompt-composer, with fallback to raw message on unavailability.
    async fn assemble_prompt_with_fallback(
        &self,
        context: PromptContext,
        request_id: &str,
    ) -> Result<(String, Option<PromptAssemblyTrace>), ReactiveLoopError> {
        let mut client = self.prompt_composer_client.lock().await;
        let request = tonic::Request::new(AssemblePromptRequest {
            context: Some(context.clone()),
            request_id: request_id.to_owned(),
        });

        match client.assemble_prompt(request).await {
            Ok(response) => {
                let inner = response.into_inner();
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "prompt_assembled",
                    request_id = %request_id,
                    prompt_length = inner.assembled_prompt.len(),
                    "prompt assembled successfully"
                );
                Ok((inner.assembled_prompt, inner.assembly_trace))
            }
            Err(e) => {
                if self.config.fallback.minimal_context_enabled {
                    tracing::warn!(
                        subsystem = SUBSYSTEM_ID,
                        event_type = "prompt_composer_fallback",
                        request_id = %request_id,
                        error = %e,
                        "prompt-composer unavailable, using raw message as prompt"
                    );
                    // Use the raw user message directly
                    Ok((context.user_message, None))
                } else {
                    Err(ReactiveLoopError::PromptComposerUnavailable {
                        reason: e.to_string(),
                    })
                }
            }
        }
    }

    /// Send prompt to inference, with fallback response on unavailability.
    async fn complete_with_fallback(
        &self,
        prompt: &str,
        _trace_context: &str,
        request_id: &str,
    ) -> Result<CompleteResponse, ReactiveLoopError> {
        let mut client = self.inference_client.lock().await;

        let request = tonic::Request::new(CompleteRequest {
            prompt: prompt.to_owned(),
            model_id: String::new(), // Empty = use active model
            max_tokens: self.config.inference.default_max_tokens,
            temperature: self.config.inference.default_temperature,
            priority: 1, // Priority Tier 1 (Reactive)
            request_id: request_id.to_owned(),
        });

        // Apply request timeout
        let timeout = std::time::Duration::from_millis(self.config.inference.request_timeout_ms);
        let inference_future = client.complete(request);

        match tokio::time::timeout(timeout, inference_future).await {
            Ok(Ok(response)) => {
                let inner = response.into_inner();
                tracing::info!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "inference_completed",
                    request_id = %request_id,
                    model_id = %inner.model_id,
                    tokens_generated = inner.tokens_generated,
                    "inference completed successfully"
                );
                Ok(inner)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "inference_fallback",
                    request_id = %request_id,
                    error = %e,
                    "inference unavailable, returning fallback response"
                );
                // Return fallback response
                Ok(CompleteResponse {
                    text: self.config.fallback.unavailable_response.clone(),
                    tokens_generated: 0,
                    tokens_prompt: 0,
                    model_id: "fallback".to_owned(),
                    request_id: request_id.to_owned(),
                })
            }
            Err(_elapsed) => {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "inference_timeout_fallback",
                    request_id = %request_id,
                    timeout_ms = self.config.inference.request_timeout_ms,
                    "inference request timed out, returning fallback response"
                );
                // Return fallback response
                Ok(CompleteResponse {
                    text: self.config.fallback.unavailable_response.clone(),
                    tokens_generated: 0,
                    tokens_prompt: 0,
                    model_id: "fallback".to_owned(),
                    request_id: request_id.to_owned(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_build_minimal_context() {
        let config = Arc::new(Config {
            grpc: crate::config::GrpcConfig {
                daemon_bus_address: "http://127.0.0.1:50051".into(),
                inference_address: "http://127.0.0.1:50055".into(),
                prompt_composer_address: "http://127.0.0.1:50057".into(),
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
            fallback: crate::config::FallbackConfig {
                unavailable_response: "Error".into(),
                minimal_context_enabled: true,
            },
            logging: crate::config::LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        });

        // Create mock clients (won't actually call them in this test)
        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051")
                .connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055")
                .connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057")
                .connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
        );

        let context = handler.build_minimal_context("Hello, world!", "trace-123");

        assert_eq!(context.user_message, "Hello, world!");
        assert_eq!(context.user_intent, "Hello, world!");
        assert_eq!(context.trace_context, "trace-123");
        assert!(context.soulbox_snapshot.is_empty());
        assert!(context.short_term.is_empty());
        assert!(context.long_term.is_empty());
        assert!(context.episodic.is_empty());
        assert!(context.model_profile.is_some());

        let profile = context.model_profile.unwrap();
        assert_eq!(profile.context_window, 4096);
        assert_eq!(profile.output_reserve, 512);
    }

    #[test]
    fn test_handler_result_structure() {
        let result = HandlerResult {
            response: "test response".into(),
            model_id: "llama-3.2".into(),
            tokens_generated: 42,
            tokens_prompt: 10,
            latency_ms: 1500,
            assembly_trace: None,
        };

        assert_eq!(result.response, "test response");
        assert_eq!(result.model_id, "llama-3.2");
        assert_eq!(result.tokens_generated, 42);
        assert_eq!(result.tokens_prompt, 10);
        assert_eq!(result.latency_ms, 1500);
        assert!(result.assembly_trace.is_none());
    }
}
