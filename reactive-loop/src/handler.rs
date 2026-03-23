//! Core message handling logic for reactive-loop.

use crate::config::Config;
use crate::error::ReactiveLoopError;
use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient, inference_service_client::InferenceServiceClient,
    memory_service_client::MemoryServiceClient, prompt_composer_service_client::PromptComposerServiceClient,
    AssemblePromptRequest, BusEvent, CompleteRequest, CompleteResponse, EventTopic,
    MemoryWriteRequest, ModelProfile, PromptAssemblyTrace, PromptContext, PublishRequest,
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
    pub pre_thought_text: Option<String>,
    pub thought_content: Option<String>,
    pub chain_of_thought_supported: bool,
}

/// Parsed model response with chain-of-thought content extracted.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub pre_thought_text: Option<String>,
    pub thought_content: Option<String>,
    pub final_response: String,
    pub chain_of_thought_supported: bool,
}

/// Message handler orchestrating the full conversation flow.
pub struct MessageHandler {
    config: Arc<Config>,
    event_bus_client: Arc<tokio::sync::Mutex<EventBusServiceClient<Channel>>>,
    inference_client: Arc<tokio::sync::Mutex<InferenceServiceClient<Channel>>>,
    prompt_composer_client: Arc<tokio::sync::Mutex<PromptComposerServiceClient<Channel>>>,
    memory_service_client: Option<Arc<tokio::sync::Mutex<MemoryServiceClient<Channel>>>>,
}

impl MessageHandler {
    /// Create a new message handler.
    pub fn new(
        config: Arc<Config>,
        event_bus_client: EventBusServiceClient<Channel>,
        inference_client: InferenceServiceClient<Channel>,
        prompt_composer_client: PromptComposerServiceClient<Channel>,
        memory_service_client: Option<MemoryServiceClient<Channel>>,
    ) -> Self {
        Self {
            config,
            event_bus_client: Arc::new(tokio::sync::Mutex::new(event_bus_client)),
            inference_client: Arc::new(tokio::sync::Mutex::new(inference_client)),
            prompt_composer_client: Arc::new(tokio::sync::Mutex::new(prompt_composer_client)),
            memory_service_client: memory_service_client
                .map(|client| Arc::new(tokio::sync::Mutex::new(client))),
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

        // Step 4.5: Parse response for chain-of-thought content
        let parsed = self.parse_response(&inference_result.text);

        let latency_ms = start_time.elapsed().as_millis() as u64;

        // Step 5: Publish TOPIC_USER_MESSAGE_RESPONSE
        self.publish_user_message_response(&parsed.final_response, trace_context, request_id)
            .await?;

        // Step 6: Write conversation turn to memory-engine (best-effort)
        self.write_conversation_turn(message, &parsed.final_response, trace_context, request_id)
            .await;

        tracing::info!(
            subsystem = SUBSYSTEM_ID,
            event_type = "user_message_completed",
            request_id = %request_id,
            latency_ms = latency_ms,
            tokens_generated = inference_result.tokens_generated,
            "message handling completed"
        );

        Ok(HandlerResult {
            response: parsed.final_response,
            model_id: inference_result.model_id,
            tokens_generated: inference_result.tokens_generated,
            tokens_prompt: inference_result.tokens_prompt,
            latency_ms,
            assembly_trace,
            pre_thought_text: parsed.pre_thought_text,
            thought_content: parsed.thought_content,
            chain_of_thought_supported: parsed.chain_of_thought_supported,
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

    /// Write the conversation turn (user message + assistant response) to memory-engine.
    /// Gracefully handles memory-engine unavailability.
    async fn write_conversation_turn(
        &self,
        user_message: &str,
        assistant_response: &str,
        trace_context: &str,
        request_id: &str,
    ) {
        let memory_client = match &self.memory_service_client {
            Some(client) => client,
            None => {
                tracing::debug!(
                    subsystem = "reactive_loop",
                    event_type = "memory_write_skipped",
                    request_id = %request_id,
                    reason = "no_memory_client",
                    "memory service client not available — skipping conversation write"
                );
                return;
            }
        };

        let mut client = memory_client.lock().await;

        // Write user message
        let user_entry = MemoryWriteRequest {
            text: format!("[user] {}", user_message),
            target_tier: "short_term".to_string(),
            priority: "reactive".to_string(),
            trace_context: trace_context.to_string(),
        };

        match client.write(tonic::Request::new(user_entry)).await {
            Ok(_) => {
                tracing::debug!(
                    subsystem = "reactive_loop",
                    event_type = "memory_write_user",
                    request_id = %request_id,
                    "user message written to short-term memory"
                );
            }
            Err(e) => {
                tracing::warn!(
                    subsystem = "reactive_loop",
                    event_type = "memory_write_failed",
                    request_id = %request_id,
                    role = "user",
                    error = %e,
                    "failed to write user message to memory — continuing"
                );
            }
        }

        // Write assistant response
        let assistant_entry = MemoryWriteRequest {
            text: format!("[assistant] {}", assistant_response),
            target_tier: "short_term".to_string(),
            priority: "reactive".to_string(),
            trace_context: trace_context.to_string(),
        };

        match client.write(tonic::Request::new(assistant_entry)).await {
            Ok(_) => {
                tracing::debug!(
                    subsystem = "reactive_loop",
                    event_type = "memory_write_assistant",
                    request_id = %request_id,
                    "assistant response written to short-term memory"
                );
            }
            Err(e) => {
                tracing::warn!(
                    subsystem = "reactive_loop",
                    event_type = "memory_write_failed",
                    request_id = %request_id,
                    role = "assistant",
                    error = %e,
                    "failed to write assistant response to memory — continuing"
                );
            }
        }
    }

    /// Parse model response to extract chain-of-thought content.
    /// Detects thinking patterns from DeepSeek-R1 (<think>...</think>),
    /// Qwen (<|thinking|>...</|thinking|>), and strips reasoning markers
    /// per config.
    fn parse_response(&self, raw_response: &str) -> ParsedResponse {
        // Try DeepSeek-R1 style: <think>...</think>
        if let Some(parsed) = Self::try_parse_think_tags(raw_response, "<think>", "</think>") {
            return parsed;
        }

        // Try Qwen style: <|thinking|>...</|thinking|>
        if let Some(parsed) = Self::try_parse_think_tags(raw_response, "<|thinking|>", "</|thinking|>") {
            return parsed;
        }

        // No thinking pattern detected — apply legacy stripping and return
        let stripped = self.strip_reasoning_markers(raw_response);
        ParsedResponse {
            pre_thought_text: None,
            thought_content: None,
            final_response: stripped,
            chain_of_thought_supported: false,
        }
    }

    /// Try to parse thinking content between open_tag and close_tag.
    /// Returns None if the open_tag is not found.
    fn try_parse_think_tags(raw: &str, open_tag: &str, close_tag: &str) -> Option<ParsedResponse> {
        let open_pos = raw.find(open_tag)?;

        let pre_thought = if open_pos > 0 {
            let text = raw[..open_pos].trim();
            if text.is_empty() { None } else { Some(text.to_string()) }
        } else {
            None
        };

        let after_open = &raw[open_pos + open_tag.len()..];
        let (thought, final_text) = if let Some(close_pos) = after_open.find(close_tag) {
            let thought_text = after_open[..close_pos].trim().to_string();
            let remaining = after_open[close_pos + close_tag.len()..].trim().to_string();
            (thought_text, remaining)
        } else {
            // Opening tag without closing — all remaining text is thought
            (after_open.trim().to_string(), String::new())
        };

        Some(ParsedResponse {
            pre_thought_text: pre_thought,
            thought_content: if thought.is_empty() { None } else { Some(thought) },
            final_response: final_text,
            chain_of_thought_supported: true,
        })
    }

    /// Strip reasoning markers from response text (legacy behavior for models without thinking tags).
    fn strip_reasoning_markers(&self, raw_response: &str) -> String {
        if !self.config.post_processing.strip_reasoning_tags {
            return raw_response.to_string();
        }
        
        let mut result = raw_response.to_string();
        let mut total_stripped_bytes = 0usize;
        
        for marker in &self.config.post_processing.reasoning_markers {
            // For paired markers like <think>...</think>, strip the content between them
            if marker.starts_with('<') && !marker.starts_with("</") {
                let close_tag = format!("</{}", &marker[1..]);
                while let Some(start) = result.find(marker.as_str()) {
                    if let Some(end) = result[start..].find(close_tag.as_str()) {
                        let removed = &result[start..start + end + close_tag.len()];
                        total_stripped_bytes += removed.len();
                        result = format!("{}{}", &result[..start], &result[start + end + close_tag.len()..]);
                    } else {
                        // Opening tag without closing — strip from marker to end
                        total_stripped_bytes += result[start..].len();
                        result = result[..start].to_string();
                        break;
                    }
                }
            } else if marker == "***" {
                // Strip content between paired *** markers
                while let Some(first) = result.find("***") {
                    if let Some(second) = result[first + 3..].find("***") {
                        let end = first + 3 + second + 3;
                        total_stripped_bytes += end - first;
                        result = format!("{}{}", &result[..first], &result[end..]);
                    } else {
                        // Single *** without closing — strip from marker to end
                        total_stripped_bytes += result[first..].len();
                        result = result[..first].to_string();
                        break;
                    }
                }
            } else {
                // Single markers like "**Explanation:**" — strip from marker to end of line/text
                while let Some(pos) = result.find(marker.as_str()) {
                    let end = result[pos..].find('\n').map_or(result.len(), |n| pos + n);
                    total_stripped_bytes += end - pos;
                    result = format!("{}{}", &result[..pos], &result[end..]);
                }
            }
        }
        
        if total_stripped_bytes > 0 {
            tracing::debug!(
                subsystem = "reactive_loop",
                event_type = "response_post_processed",
                stripped_bytes = total_stripped_bytes,
                "stripped reasoning markers from model response"
            );
        }
        
        result.trim().to_string()
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
            None,
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
            pre_thought_text: None,
            thought_content: Some("reasoning here".into()),
            chain_of_thought_supported: true,
        };

        assert_eq!(result.response, "test response");
        assert_eq!(result.model_id, "llama-3.2");
        assert_eq!(result.tokens_generated, 42);
        assert_eq!(result.tokens_prompt, 10);
        assert_eq!(result.latency_ms, 1500);
        assert!(result.assembly_trace.is_none());
        assert!(result.pre_thought_text.is_none());
        assert_eq!(result.thought_content.as_deref(), Some("reasoning here"));
        assert!(result.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_post_process_strip_paired_star_markers() {
        let config = Arc::new(Config {
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
                strip_reasoning_tags: true,
                reasoning_markers: vec!["***".into()],
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

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        let parsed = handler.parse_response("Hello *** internal reasoning *** world");
        assert_eq!(parsed.final_response, "Hello  world");
        assert!(!parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_post_process_strip_think_tags() {
        let config = Arc::new(Config {
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
                strip_reasoning_tags: true,
                reasoning_markers: vec!["<think>".into(), "</think>".into()],
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

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        let parsed = handler.parse_response("<think>reasoning</think>Hello");
        assert_eq!(parsed.final_response, "Hello");
        assert_eq!(parsed.thought_content.as_deref(), Some("reasoning"));
        assert!(parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_post_process_strip_explanation_marker() {
        let config = Arc::new(Config {
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
                strip_reasoning_tags: true,
                reasoning_markers: vec!["**Explanation:**".into()],
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

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        let parsed = handler.parse_response("Hello\n**Explanation:** stuff");
        assert_eq!(parsed.final_response, "Hello");
        assert!(!parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_post_process_clean_response_unchanged() {
        let config = Arc::new(Config {
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
                strip_reasoning_tags: true,
                reasoning_markers: vec!["***".into(), "**Explanation:**".into()],
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

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        let clean_text = "This is a clean response with no markers.";
        let parsed = handler.parse_response(clean_text);
        assert_eq!(parsed.final_response, clean_text);
        assert!(!parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_post_process_disabled() {
        let config = Arc::new(Config {
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
                reasoning_markers: vec!["***".into()],
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

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        let raw_text = "Hello *** reasoning *** world";
        let parsed = handler.parse_response(raw_text);
        // When disabled, the markers should NOT be stripped
        assert_eq!(parsed.final_response, raw_text);
        assert!(!parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_write_conversation_turn_graceful_without_client() {
        let config = Arc::new(Config {
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
        });

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        // Create handler without memory service client (None)
        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        // This should complete gracefully without panic
        handler
            .write_conversation_turn("Hello", "Hi there!", "trace-123", "req-456")
            .await;

        // If we get here, the test passed - method returned without error
    }

    #[tokio::test]
    async fn test_handler_completes_without_memory_service() {
        // This test verifies that handle_message works end-to-end
        // even when memory_service_client is None.
        let config = Arc::new(Config {
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
                unavailable_response: "Service unavailable".into(),
                minimal_context_enabled: true,
            },
            logging: crate::config::LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        });

        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );

        let handler = MessageHandler::new(
            config,
            event_bus_client,
            inference_client,
            prompt_composer_client,
            None,
        );

        // The handler should use fallback responses but still complete successfully
        let result = handler
            .handle_message("Test message", "trace-789", "req-123")
            .await;

        // Should return the fallback response since inference won't actually be available
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.response, "Service unavailable");
    }

    // Helper functions to reduce test boilerplate
    fn make_test_config(strip_tags: bool, markers: Vec<String>) -> Arc<Config> {
        Arc::new(Config {
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
                strip_reasoning_tags: strip_tags,
                reasoning_markers: markers,
            },
            fallback: crate::config::FallbackConfig {
                unavailable_response: "Error".into(),
                minimal_context_enabled: true,
            },
            logging: crate::config::LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        })
    }

    fn make_test_handler(config: Arc<Config>) -> MessageHandler {
        let event_bus_client = EventBusServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50051").connect_lazy(),
        );
        let inference_client = InferenceServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50055").connect_lazy(),
        );
        let prompt_composer_client = PromptComposerServiceClient::new(
            tonic::transport::Channel::from_static("http://[::1]:50057").connect_lazy(),
        );
        MessageHandler::new(config, event_bus_client, inference_client, prompt_composer_client, None)
    }

    #[tokio::test]
    async fn test_parse_deepseek_think_tags() {
        let config = make_test_config(true, vec!["<think>".to_string()]);
        let handler = make_test_handler(config);
        let parsed = handler.parse_response("<think>reasoning about the problem</think>The answer is 42.");
        assert_eq!(parsed.thought_content.as_deref(), Some("reasoning about the problem"));
        assert_eq!(parsed.final_response, "The answer is 42.");
        assert!(parsed.pre_thought_text.is_none());
        assert!(parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_parse_qwen_thinking_tags() {
        let config = make_test_config(true, vec![]);
        let handler = make_test_handler(config);
        let parsed = handler.parse_response("<|thinking|>step by step reasoning</|thinking|>Final answer here.");
        assert_eq!(parsed.thought_content.as_deref(), Some("step by step reasoning"));
        assert_eq!(parsed.final_response, "Final answer here.");
        assert!(parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_parse_no_thinking() {
        let config = make_test_config(false, vec![]);
        let handler = make_test_handler(config);
        let parsed = handler.parse_response("Just a plain response.");
        assert!(parsed.thought_content.is_none());
        assert_eq!(parsed.final_response, "Just a plain response.");
        assert!(!parsed.chain_of_thought_supported);
    }

    #[tokio::test]
    async fn test_parse_text_before_thinking() {
        let config = make_test_config(true, vec![]);
        let handler = make_test_handler(config);
        let parsed = handler.parse_response("Hmm, let me think...<think>deep reasoning</think>The result.");
        assert_eq!(parsed.pre_thought_text.as_deref(), Some("Hmm, let me think..."));
        assert_eq!(parsed.thought_content.as_deref(), Some("deep reasoning"));
        assert_eq!(parsed.final_response, "The result.");
        assert!(parsed.chain_of_thought_supported);
    }
}
