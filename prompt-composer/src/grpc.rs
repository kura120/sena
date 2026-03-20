//! gRPC service implementation for PromptComposerService.
//!
//! Wires AssemblePrompt RPC to the assembler logic.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::assembler::PromptAssembler;
use crate::config::Config;
use crate::generated::sena_daemonbus_v1::{
    prompt_composer_service_server::PromptComposerService, AssemblePromptRequest,
    AssemblePromptResponse,
};

/// gRPC service implementation for prompt assembly.
///
/// The service is stateless — all context is provided in each request.
pub struct PromptComposerGrpcService {
    assembler: PromptAssembler,
    config: Arc<Config>,
}

impl PromptComposerGrpcService {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            assembler: PromptAssembler::new(),
            config,
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

        let response = AssemblePromptResponse {
            assembled_prompt: assembly_result.assembled_prompt,
            assembly_trace: Some(assembly_result.trace),
            request_id,
        };

        Ok(Response::new(response))
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
            logging: crate::config::LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        })
    }

    #[tokio::test]
    async fn test_assemble_prompt_success() {
        let service = PromptComposerGrpcService::new(test_config());

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
        let service = PromptComposerGrpcService::new(test_config());

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
        let service = PromptComposerGrpcService::new(test_config());

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
        let service = PromptComposerGrpcService::new(test_config());

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
        let service = PromptComposerGrpcService::new(test_config());

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
