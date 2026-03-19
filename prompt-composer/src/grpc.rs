//! gRPC service implementation for PcService.
//!
//! Receives `AssembleRequest` from CTP, converts proto types to local assembler
//! types, calls `assembler::assemble`, publishes telemetry to daemon-bus, and
//! returns the `AssembleResponse`. Fully stateless — concurrent calls never
//! interfere.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::assembler::{
    self, MemoryResult, ModelCapabilityProfile, OsContext, PromptContext, SoulBoxSnapshot,
    TelemetrySignal,
};
use crate::config::Config;
use crate::generated::sena_daemonbus_v1::event_bus_service_client::EventBusServiceClient;
use crate::generated::sena_daemonbus_v1::pc_service_server::PcService;
use crate::generated::sena_daemonbus_v1::{
    AssembleRequest, AssembleResponse, BusEvent, EventTopic, PcMemoryEntry, PromptContextProto,
    PublishRequest,
};

/// PcService gRPC implementation. Holds config and daemon-bus client.
/// No mutable state — fully safe for concurrent use.
pub struct PcGrpcService {
    config: Arc<Config>,
    daemon_bus_client: Arc<tokio::sync::Mutex<EventBusServiceClient<tonic::transport::Channel>>>,
}

impl PcGrpcService {
    pub fn new(
        config: Arc<Config>,
        daemon_bus_client: EventBusServiceClient<tonic::transport::Channel>,
    ) -> Self {
        Self {
            config,
            daemon_bus_client: Arc::new(tokio::sync::Mutex::new(daemon_bus_client)),
        }
    }
}

#[tonic::async_trait]
impl PcService for PcGrpcService {
    async fn assemble(
        &self,
        request: Request<AssembleRequest>,
    ) -> Result<Response<AssembleResponse>, Status> {
        let req = request.into_inner();
        let request_id = req.request_id.clone();

        let proto_context = req
            .context
            .ok_or_else(|| Status::invalid_argument("missing context in AssembleRequest"))?;

        let local_context = convert_proto_to_local(proto_context);

        let result = assembler::assemble(&local_context, &self.config)
            .await
            .map_err(|pc_error| {
                tracing::error!(
                    subsystem = "prompt-composer",
                    event_type = "assemble_failed",
                    request_id = %request_id,
                    error = %pc_error,
                    "prompt assembly failed"
                );
                Status::from(pc_error)
            })?;

        // Publish telemetry (fire-and-forget — don't block the response)
        let bus_client = Arc::clone(&self.daemon_bus_client);
        let telemetry_request_id = request_id.clone();
        let telemetry_hash = result.unique_hash.clone();
        let telemetry_model_id = result.model_id.clone();
        let telemetry_token_count = result.token_count;
        let telemetry_truncated = result.truncated;
        tokio::spawn(async move {
            let event = BusEvent {
                event_id: uuid::Uuid::new_v4().to_string(),
                topic: EventTopic::TopicPcPromptAssembled.into(),
                source_subsystem: "prompt-composer".to_string(),
                payload: serde_json::json!({
                    "request_id": telemetry_request_id,
                    "prompt_hash": telemetry_hash,
                    "model_id": telemetry_model_id,
                    "token_count": telemetry_token_count,
                    "truncated": telemetry_truncated,
                })
                .to_string()
                .into_bytes(),
                trace_context: String::new(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };

            let publish_request = tonic::Request::new(PublishRequest {
                event: Some(event),
            });

            let mut client = bus_client.lock().await;
            if let Err(publish_error) = client.publish(publish_request).await {
                tracing::warn!(
                    subsystem = "prompt-composer",
                    event_type = "telemetry_publish_failed",
                    error = %publish_error,
                    "failed to publish PC_PROMPT_ASSEMBLED telemetry"
                );
            }
        });

        tracing::info!(
            subsystem = "prompt-composer",
            event_type = "prompt_assembled",
            request_id = %request_id,
            token_count = result.token_count,
            model_id = %result.model_id,
            truncated = result.truncated,
            dropped_tiers = ?result.dropped_tiers,
        );

        Ok(Response::new(AssembleResponse {
            prompt: result.prompt,
            token_count: result.token_count,
            model_id: result.model_id,
            request_id,
            truncated: result.truncated,
            dropped_tiers: result.dropped_tiers,
        }))
    }
}

/// Convert proto PromptContextProto to local PromptContext.
fn convert_proto_to_local(proto: PromptContextProto) -> PromptContext {
    PromptContext {
        soulbox_snapshot: SoulBoxSnapshot {
            personality_summary: proto.soulbox_snapshot_json,
        },
        short_term: convert_memory_entries(proto.short_term),
        long_term: convert_memory_entries(proto.long_term),
        episodic: convert_memory_entries(proto.episodic),
        os_context: OsContext {
            active_window: proto.os_context_json.clone(),
            recent_events: Vec::new(),
        },
        model_profile: ModelCapabilityProfile {
            model_id: proto.model_id,
            context_window: proto.context_window,
            output_reserve: proto.output_reserve,
        },
        user_intent: if proto.user_intent.is_empty() {
            None
        } else {
            Some(proto.user_intent)
        },
        telemetry_signals: proto
            .telemetry_signals
            .into_iter()
            .map(|s| TelemetrySignal {
                signal_type: s.signal_type,
                value: s.value,
                relevance: s.relevance,
            })
            .collect(),
    }
}

/// Convert proto PcMemoryEntry list to local MemoryResult list.
fn convert_memory_entries(entries: Vec<PcMemoryEntry>) -> Vec<MemoryResult> {
    entries
        .into_iter()
        .map(|e| MemoryResult {
            node_id: e.node_id,
            summary: e.summary,
            score: e.score,
            tier: e.tier,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::sena_daemonbus_v1::TelemetrySignal as ProtoTelemetrySignal;

    fn test_proto_context() -> PromptContextProto {
        PromptContextProto {
            soulbox_snapshot_json: "warm and curious".to_string(),
            short_term: vec![PcMemoryEntry {
                node_id: "st1".into(),
                summary: "recent conversation".into(),
                score: 0.9,
                tier: "short_term".into(),
            }],
            long_term: vec![],
            episodic: vec![],
            os_context_json: "Visual Studio Code".to_string(),
            model_id: "test-model".to_string(),
            context_window: 4096,
            output_reserve: 512,
            user_intent: "help with Rust code".to_string(),
            activity_state: "active".to_string(),
            telemetry_signals: vec![],
        }
    }

    #[test]
    fn test_convert_proto_to_local() {
        let proto = test_proto_context();
        let local = convert_proto_to_local(proto);

        assert_eq!(local.soulbox_snapshot.personality_summary, "warm and curious");
        assert_eq!(local.short_term.len(), 1);
        assert_eq!(local.short_term[0].node_id, "st1");
        assert_eq!(local.model_profile.model_id, "test-model");
        assert_eq!(local.model_profile.context_window, 4096);
        assert_eq!(local.user_intent, Some("help with Rust code".to_string()));
    }

    #[test]
    fn test_convert_empty_user_intent() {
        let mut proto = test_proto_context();
        proto.user_intent = String::new();
        let local = convert_proto_to_local(proto);
        assert_eq!(local.user_intent, None);
    }

    #[test]
    fn test_convert_memory_entries_empty() {
        let entries: Vec<PcMemoryEntry> = vec![];
        let results = convert_memory_entries(entries);
        assert!(results.is_empty());
    }

    #[test]
    fn test_convert_telemetry_signals() {
        let mut proto = test_proto_context();
        proto.telemetry_signals = vec![ProtoTelemetrySignal {
            signal_type: "cpu".into(),
            value: "high".into(),
            relevance: 0.8,
        }];
        let local = convert_proto_to_local(proto);
        assert_eq!(local.telemetry_signals.len(), 1);
        assert_eq!(local.telemetry_signals[0].signal_type, "cpu");
    }
}
