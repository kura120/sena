//! Test that prompt-composer emits TOPIC_PC_PROMPT_ASSEMBLED event after assembly.

use prompt_composer::assembler::PromptAssembler;
use prompt_composer::config::Config;
use prompt_composer::generated::sena_daemonbus_v1::{ModelProfile, PromptContext};

#[tokio::test]
async fn test_prompt_assembly_emits_event() {
    // This test verifies that when a prompt is assembled, an event should be published
    // to the event bus with topic TOPIC_PC_PROMPT_ASSEMBLED.
    
    // For now, this is a placeholder test that verifies the assembler works.
    // The actual event emission test will require a mock event bus client,
    // which we'll add after implementing the event publishing in grpc.rs.
    
    let config = test_config();
    let assembler = PromptAssembler::new();
    
    let context = PromptContext {
        soulbox_snapshot: "test soul".into(),
        user_intent: "test intent".into(),
        user_message: "hello".into(),
        short_term: vec![],
        long_term: vec![],
        episodic: vec![],
        os_context: String::new(),
        telemetry_signals: vec![],
        model_profile: Some(ModelProfile {
            model_id: "test-model".into(),
            context_window: 8192,
            output_reserve: 1024,
        }),
        trace_context: "test-trace".into(),
    };
    
    let result = assembler.assemble(&context, &config);
    assert!(result.is_ok());
    
    let assembly = result.unwrap(); // test: confirmed is_ok
    assert!(assembly.trace.token_count > 0);
    assert_eq!(assembly.trace.token_budget, 8192 - 1024);
    
    // Event emission will be verified in integration tests once
    // the event bus client is wired into the gRPC service
}

fn test_config() -> Config {
    Config {
        grpc: prompt_composer::config::GrpcConfig {
            daemon_bus_address: "http://127.0.0.1:50051".into(),
            listen_address: "0.0.0.0".into(),
            listen_port: 50057,
            connection_timeout_ms: 5000,
        },
        boot: prompt_composer::config::BootConfig {
            ready_signal_timeout_ms: 5000,
        },
        context_window: prompt_composer::config::ContextWindowConfig {
            esu_savings_threshold: 0.15,
            tokens_per_char_estimate: 0.25,
        },
        sacred: prompt_composer::config::SacredConfig {
            sacred_fields: vec!["soulbox_snapshot".into(), "user_intent".into()],
        },
        response_format: prompt_composer::config::ResponseFormatConfig {
            system_instruction: "Respond conversationally.".into(),
        },
        logging: prompt_composer::config::LoggingConfig {
            level: "info".into(),
            format: "json".into(),
        },
    }
}
