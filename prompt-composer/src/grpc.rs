//! gRPC service implementation for PromptComposerService.
//!
//! Wires AssemblePrompt RPC to the assembler logic and publishes
//! TOPIC_PC_PROMPT_ASSEMBLED events after successful assembly.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::assembler::PromptAssembler;
use crate::config::Config;
use crate::generated::sena_daemonbus_v1::{
    event_bus_service_client::EventBusServiceClient,
    prompt_composer_service_server::PromptComposerService, AssemblePromptRequest,
    AssemblePromptResponse, BusEvent, EventTopic, PublishRequest,
};

const SUBSYSTEM_ID: &str = "prompt_composer";

/// gRPC service implementation for prompt assembly.
///
/// The service is stateless — all context is provided in each request.
pub struct PromptComposerGrpcService {
    assembler: PromptAssembler,
    config: Arc<Config>,
    event_bus_client: Arc<tokio::sync::Mutex<EventBusServiceClient<tonic::transport::Channel>>>,
}

impl PromptComposerGrpcService {
    pub fn new(
        config: Arc<Config>,
        event_bus_client: EventBusServiceClient<tonic::transport::Channel>,
    ) -> Self {
        Self {
            assembler: PromptAssembler::new(),
            config,
            event_bus_client: Arc::new(tokio::sync::Mutex::new(event_bus_client)),
        }
    }
}

#[tonic::async_trait]
impl PromptComposerService for PromptComposerGrpcService {
    async fn assemble_prompt(
        &self,
        request: Request<AssemblePromptRequest>,
    ) -> Result<Response<AssemblePromptResponse>, Status> {
        let req = request.into_inner();
        let request_id = if req.request_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            req.request_id.clone()
        };

        tracing::info!(
            subsystem = "prompt_composer",
            event_type = "assemble_request_received",
            request_id = %request_id,
            "received AssemblePrompt request"
        );

        // Extract trace_context before moving context
        let trace_context = req.context.as_ref()
            .map(|c| c.trace_context.clone())
            .unwrap_or_default();

        // Extract context
        let context = req.context.ok_or_else(|| {
            tracing::error!(
                subsystem = "prompt_composer",
                event_type = "assemble_request_invalid",
                request_id = %request_id,
                error = "missing context",
                "AssemblePrompt request missing context"
            );
            Status::invalid_argument("missing context")
        })?;

        // Assemble the prompt
        let assembly_result = self
            .assembler
            .assemble(&context, &self.config)
            .map_err(|e| {
                tracing::error!(
                    subsystem = "prompt_composer",
                    event_type = "assemble_failed",
                    request_id = %request_id,
                    error = %e,
                    "prompt assembly failed"
                );
                Status::from(e)
            })?;

        tracing::info!(
            subsystem = "prompt_composer",
            event_type = "assemble_complete",
            request_id = %request_id,
            token_count = assembly_result.trace.token_count,
            token_budget = assembly_result.trace.token_budget,
            encoding = %assembly_result.trace.encoding_used,
            "prompt assembly successful"
        );

        // Publish TOPIC_PC_PROMPT_ASSEMBLED event
        self.publish_prompt_assembled_event(&assembly_result, &request_id, &trace_context)
            .await;

        let response = AssemblePromptResponse {
            assembled_prompt: assembly_result.assembled_prompt,
            assembly_trace: Some(assembly_result.trace),
            request_id,
        };

        Ok(Response::new(response))
    }
}

impl PromptComposerGrpcService {
    /// Publish TOPIC_PC_PROMPT_ASSEMBLED event after successful assembly.
    async fn publish_prompt_assembled_event(
        &self,
        assembly_result: &crate::assembler::AssemblyResult,
        request_id: &str,
        trace_context: &str,
    ) {
        // Build JSON payload with prompt trace metadata
        let payload_json = serde_json::json!({
            "sections": assembly_result.trace.included_tiers,
            "toon_output": truncate_preview(&assembly_result.assembled_prompt, 500),
            "token_count": assembly_result.trace.token_count,
            "token_budget": assembly_result.trace.token_budget,
            "encoding_used": assembly_result.trace.encoding_used,
            "dropped_tiers": assembly_result.trace.dropped_tiers,
            "request_id": request_id,
        });

        let payload_bytes = payload_json.to_string().into_bytes();

        let event = BusEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            topic: EventTopic::TopicPcPromptAssembled.into(),
            source_subsystem: SUBSYSTEM_ID.to_owned(),
            payload: payload_bytes,
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
                    topic = "TOPIC_PC_PROMPT_ASSEMBLED",
                    request_id = %request_id,
                    "prompt assembly event published successfully"
                );
            }
            Err(e) => {
                tracing::warn!(
                    subsystem = SUBSYSTEM_ID,
                    event_type = "event_publish_failed",
                    topic = "TOPIC_PC_PROMPT_ASSEMBLED",
                    request_id = %request_id,
                    error = %e,
                    "failed to publish prompt assembly event, continuing anyway"
                );
                // Event publishing is best-effort — don't fail the RPC
            }
        }
    }
}

/// Truncate a string to a maximum length with ellipsis.
fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::sena_daemonbus_v1::{ModelProfile, PromptContext};

    fn test_config() -> Arc<Config> {
        Arc::new(Config {
            grpc: crate::config::GrpcConfig {
                daemon_bus_address: "http://127.0.0.1:50051".into(),
                listen_address: "0.0.0.0".into(),
                listen_port: 50057,
                connection_timeout_ms: 5000,
            },
            boot: crate::config::BootConfig {
                ready_signal_timeout_ms: 5000,
            },
            context_window: crate::config::ContextWindowConfig {
                esu_savings_threshold: 0.15,
                tokens_per_char_estimate: 0.25,
            },
            sacred: crate::config::SacredConfig {
                sacred_fields: vec!["soulbox_snapshot".into(), "user_intent".into()],
            },
            response_format: crate::config::ResponseFormatConfig {
                system_instruction: "Respond conversationally and directly.".into(),
            },
            logging: crate::config::LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        })
    }

    fn test_event_bus_client() -> EventBusServiceClient<tonic::transport::Channel> {
        // Create a lazy connection client for tests (won't actually connect)
        let channel = tonic::transport::Channel::from_static("http://127.0.0.1:50051")
            .connect_lazy();
        EventBusServiceClient::new(channel)
    }

    #[tokio::test]
    async fn test_assemble_prompt_success() {
        let service = PromptComposerGrpcService::new(test_config(), test_event_bus_client());

        let context = PromptContext {
            soulbox_snapshot: "soul data".into(),
            user_intent: "user wants help".into(),
            user_message: "hello".into(),
            short_term: vec![],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: Some(ModelProfile {
                model_id: "test".into(),
                context_window: 8192,
                output_reserve: 1024,
            }),
            trace_context: String::new(),
        };

        let request = Request::new(AssemblePromptRequest {
            context: Some(context),
            request_id: "test-req".into(),
        });

        let result = service.assemble_prompt(request).await;

        assert!(result.is_ok());
        let response = result.unwrap().into_inner(); // test: just confirmed is_ok
        assert_eq!(response.request_id, "test-req");
        assert!(response.assembled_prompt.contains("soul data"));
        assert!(response.assembly_trace.is_some());

        let trace = response.assembly_trace.unwrap(); // test: just confirmed is_some
        assert!(trace.token_count > 0);
        assert_eq!(trace.token_budget, 8192 - 1024);
    }

    #[tokio::test]
    async fn test_assemble_prompt_missing_context() {
        let service = PromptComposerGrpcService::new(test_config(), test_event_bus_client());

        let request = Request::new(AssemblePromptRequest {
            context: None, // Missing!
            request_id: "test-req".into(),
        });

        let result = service.assemble_prompt(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(status.message().contains("missing context"));
    }

    #[tokio::test]
    async fn test_assemble_prompt_missing_model_profile() {
        let service = PromptComposerGrpcService::new(test_config(), test_event_bus_client());

        let context = PromptContext {
            soulbox_snapshot: "soul".into(),
            user_intent: "intent".into(),
            user_message: String::new(),
            short_term: vec![],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: None, // Missing!
            trace_context: String::new(),
        };

        let request = Request::new(AssemblePromptRequest {
            context: Some(context),
            request_id: "test-req".into(),
        });

        let result = service.assemble_prompt(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_assemble_prompt_auto_generates_request_id() {
        let service = PromptComposerGrpcService::new(test_config(), test_event_bus_client());

        let context = PromptContext {
            soulbox_snapshot: "soul".into(),
            user_intent: "intent".into(),
            user_message: String::new(),
            short_term: vec![],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: Some(ModelProfile {
                model_id: "test".into(),
                context_window: 4096,
                output_reserve: 512,
            }),
            trace_context: String::new(),
        };

        let request = Request::new(AssemblePromptRequest {
            context: Some(context),
            request_id: String::new(), // Empty — should auto-generate
        });

        let result = service.assemble_prompt(request).await;

        assert!(result.is_ok());
        let response = result.unwrap().into_inner(); // test: just confirmed is_ok
        assert!(!response.request_id.is_empty());
        // Should be a valid UUID
        assert!(uuid::Uuid::parse_str(&response.request_id).is_ok());
    }

    #[tokio::test]
    async fn test_assemble_prompt_budget_exhausted() {
        let service = PromptComposerGrpcService::new(test_config(), test_event_bus_client());

        // Create a context where sacred content exceeds budget
        let context = PromptContext {
            soulbox_snapshot: "x".repeat(1000), // ~250 tokens
            user_intent: "y".repeat(1000),      // ~250 tokens
            user_message: String::new(),
            short_term: vec![],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: Some(ModelProfile {
                model_id: "test".into(),
                context_window: 100,
                output_reserve: 50,
            }), // Budget = 50 tokens
            trace_context: String::new(),
        };

        let request = Request::new(AssemblePromptRequest {
            context: Some(context),
            request_id: "test-req".into(),
        });

        let result = service.assemble_prompt(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err(); // test: just confirmed is_err
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
        assert!(status.message().contains("budget exhausted"));
    }
}
