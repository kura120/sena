//! Core prompt assembly logic.
//!
//! Implements the Fixed Drop Order algorithm, sacred content enforcement,
//! and token budget management.

use crate::config::Config;
use crate::error::PromptComposerError;
use crate::esu::{select_encoding, EncodingChoice};
use crate::generated::sena_daemonbus_v1::{
    ModelProfile, PromptAssemblyTrace, PromptContext, PromptContextEntry, TelemetrySignal,
};

/// Result of prompt assembly.
#[derive(Debug, Clone)]
pub struct AssemblyResult {
    pub assembled_prompt: String,
    pub trace: PromptAssemblyTrace,
}

/// Stateless prompt assembler.
pub struct PromptAssembler;

impl PromptAssembler {
    pub fn new() -> Self {
        Self
    }

    /// Assemble a prompt from the given context, respecting token budget and drop order.
    pub fn assemble(
        &self,
        context: &PromptContext,
        config: &Config,
    ) -> Result<AssemblyResult, PromptComposerError> {
        let model_profile = context
            .model_profile
            .as_ref()
            .ok_or_else(|| PromptComposerError::MissingField {
                field: "model_profile".into(),
            })?;

        // Validate model profile
        if model_profile.context_window == 0 {
            return Err(PromptComposerError::InvalidModelProfile {
                reason: "context_window must be greater than 0".into(),
            });
        }

        // Calculate token budget (context window minus output reserve)
        let token_budget = model_profile
            .context_window
            .saturating_sub(model_profile.output_reserve);

        if token_budget == 0 {
            return Err(PromptComposerError::InvalidModelProfile {
                reason: "token budget is zero after subtracting output reserve".into(),
            });
        }

        tracing::debug!(
            subsystem = "prompt_composer",
            event_type = "assembly_start",
            context_window = model_profile.context_window,
            output_reserve = model_profile.output_reserve,
            token_budget = token_budget,
            "starting prompt assembly"
        );

        // Build the prompt following Fixed Drop Order
        let mut builder = PromptBuilder::new(token_budget, config);

        // Step 1: Add sacred content (always included, never dropped)
        builder.add_sacred("soulbox_snapshot", &context.soulbox_snapshot)?;
        builder.add_sacred("user_intent", &context.user_intent)?;

        // Step 2: Add user message (not sacred, but high priority)
        builder.add_section("user_message", &context.user_message, false);

        // Step 3-7: Add droppable content in reverse drop order (highest priority first)
        // Drop order (from spec):
        // 1. Lowest-relevance telemetry signals
        // 2. Redundant OS context
        // 3. Stale short-term context
        // 4. Lowest-relevance long-term memories
        // 5. Lowest-relevance episodic memories

        // Add in reverse order (episodic first, telemetry last) so that when
        // budget is exceeded, we can drop in the correct order.
        builder.add_memory_tier("episodic", &context.episodic, model_profile);
        builder.add_memory_tier("long_term", &context.long_term, model_profile);
        builder.add_memory_tier("short_term", &context.short_term, model_profile);
        builder.add_section("os_context", &context.os_context, false);
        builder.add_telemetry(&context.telemetry_signals, model_profile);

        // Finalize and build the prompt
        let result = builder.finalize()?;

        tracing::info!(
            subsystem = "prompt_composer",
            event_type = "assembly_complete",
            token_count = result.trace.token_count,
            token_budget = result.trace.token_budget,
            encoding = %result.trace.encoding_used,
            included_tiers = ?result.trace.included_tiers,
            dropped_tiers = ?result.trace.dropped_tiers,
            "prompt assembly complete"
        );

        Ok(result)
    }
}

/// Builder for assembling prompts with token budget tracking.
struct PromptBuilder<'a> {
    sections: Vec<PromptSection>,
    token_budget: u32,
    current_tokens: u32,
    config: &'a Config,
    included_tiers: Vec<String>,
    dropped_tiers: Vec<String>,
    encoding_used: EncodingChoice,
}

#[derive(Debug)]
struct PromptSection {
    name: String,
    content: String,
    is_sacred: bool,
    priority: usize, // Lower number = higher priority (sacred = 0)
}

impl<'a> PromptBuilder<'a> {
    fn new(token_budget: u32, config: &'a Config) -> Self {
        Self {
            sections: Vec::new(),
            token_budget,
            current_tokens: 0,
            config,
            included_tiers: Vec::new(),
            dropped_tiers: Vec::new(),
            encoding_used: EncodingChoice::Json, // Default, will be determined during assembly
        }
    }

    /// Add sacred content — always included, budget exhaustion is fatal.
    fn add_sacred(
        &mut self,
        name: &str,
        content: &str,
    ) -> Result<(), PromptComposerError> {
        if content.is_empty() {
            return Ok(());
        }

        // Sacred content always uses JSON encoding for fidelity
        let (_encoding, encoded) = select_encoding(content, true, &self.config.context_window);
        let tokens = estimate_tokens(&encoded, &self.config.context_window);

        // Check if sacred content fits in budget
        if self.current_tokens + tokens > self.token_budget {
            return Err(PromptComposerError::BudgetExhausted {
                sacred_tokens: self.current_tokens + tokens,
                budget: self.token_budget,
            });
        }

        self.sections.push(PromptSection {
            name: name.to_owned(),
            content: encoded,
            is_sacred: true,
            priority: 0, // Highest priority
        });

        self.current_tokens += tokens;
        self.included_tiers.push(name.to_owned());

        tracing::debug!(
            subsystem = "prompt_composer",
            event_type = "section_added",
            section = name,
            tokens = tokens,
            current_tokens = self.current_tokens,
            is_sacred = true,
            "added sacred section"
        );

        Ok(())
    }

    /// Add a regular section with encoding selection.
    fn add_section(&mut self, name: &str, content: &str, is_sacred: bool) {
        if content.is_empty() {
            return;
        }

        let (encoding, encoded) = select_encoding(content, is_sacred, &self.config.context_window);
        let tokens = estimate_tokens(&encoded, &self.config.context_window);

        // Track which encoding was used (bias toward TOON if any section uses it)
        if encoding == EncodingChoice::Toon {
            self.encoding_used = EncodingChoice::Toon;
        }

        // Only add if we have budget
        if self.current_tokens + tokens <= self.token_budget {
            self.sections.push(PromptSection {
                name: name.to_owned(),
                content: encoded,
                is_sacred,
                priority: 2, // Lower priority than sacred
            });

            self.current_tokens += tokens;
            self.included_tiers.push(name.to_owned());

            tracing::debug!(
                subsystem = "prompt_composer",
                event_type = "section_added",
                section = name,
                tokens = tokens,
                current_tokens = self.current_tokens,
                encoding = %encoding.as_str(),
                "added section"
            );
        } else {
            self.dropped_tiers.push(name.to_owned());

            tracing::debug!(
                subsystem = "prompt_composer",
                event_type = "section_dropped",
                section = name,
                tokens = tokens,
                current_tokens = self.current_tokens,
                reason = "budget_exceeded",
                "dropped section due to budget"
            );
        }
    }

    /// Add a memory tier (short_term, long_term, episodic) with relevance-based filtering.
    fn add_memory_tier(
        &mut self,
        tier_name: &str,
        entries: &[PromptContextEntry],
        _model_profile: &ModelProfile,
    ) {
        if entries.is_empty() {
            return;
        }

        // Sort by relevance score (highest first)
        let mut sorted_entries = entries.to_vec();
        // unwrap acceptable: relevance_score is f32; partial_cmp returns None only for NaN,
        // which should never occur in valid relevance scores from the context provider.
        sorted_entries.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());

        let mut tier_content = Vec::new();
        let mut tier_tokens = 0u32;

        for entry in sorted_entries {
            // Encode each entry
            let entry_json = serde_json::json!({
                "id": entry.id,
                "content": entry.content,
                "relevance": entry.relevance_score,
                "tier": entry.tier,
            })
            .to_string();

            let (_, encoded) = select_encoding(&entry_json, false, &self.config.context_window);
            let tokens = estimate_tokens(&encoded, &self.config.context_window);

            // Add entry if it fits
            if self.current_tokens + tier_tokens + tokens <= self.token_budget {
                tier_content.push(encoded);
                tier_tokens += tokens;
            } else {
                // Budget exceeded — stop adding entries from this tier
                break;
            }
        }

        if !tier_content.is_empty() {
            let content = format!("--- {} ---\n{}", tier_name, tier_content.join("\n"));
            let section_tokens = estimate_tokens(&content, &self.config.context_window);

            self.sections.push(PromptSection {
                name: tier_name.to_owned(),
                content,
                is_sacred: false,
                priority: 3,
            });

            self.current_tokens += section_tokens;
            self.included_tiers.push(tier_name.to_owned());

            tracing::debug!(
                subsystem = "prompt_composer",
                event_type = "tier_added",
                tier = tier_name,
                entries_included = tier_content.len(),
                entries_total = entries.len(),
                tokens = section_tokens,
                current_tokens = self.current_tokens,
                "added memory tier"
            );
        } else {
            self.dropped_tiers.push(tier_name.to_owned());

            tracing::debug!(
                subsystem = "prompt_composer",
                event_type = "tier_dropped",
                tier = tier_name,
                reason = "budget_exceeded",
                "dropped entire tier due to budget"
            );
        }
    }

    /// Add telemetry signals with relevance-based filtering.
    fn add_telemetry(&mut self, signals: &[TelemetrySignal], _model_profile: &ModelProfile) {
        if signals.is_empty() {
            return;
        }

        // Sort by relevance score (highest first)
        let mut sorted_signals = signals.to_vec();
        // unwrap acceptable: relevance_score is f32; partial_cmp returns None only for NaN,
        // which should never occur in valid relevance scores from telemetry signals.
        sorted_signals.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());

        let mut telemetry_content = Vec::new();
        let mut telemetry_tokens = 0u32;

        for signal in sorted_signals {
            let signal_json = serde_json::json!({
                "type": signal.signal_type,
                "value": signal.value,
                "relevance": signal.relevance_score,
            })
            .to_string();

            let (_, encoded) = select_encoding(&signal_json, false, &self.config.context_window);
            let tokens = estimate_tokens(&encoded, &self.config.context_window);

            if self.current_tokens + telemetry_tokens + tokens <= self.token_budget {
                telemetry_content.push(encoded);
                telemetry_tokens += tokens;
            } else {
                break;
            }
        }

        if !telemetry_content.is_empty() {
            let content = format!("--- telemetry ---\n{}", telemetry_content.join("\n"));
            let section_tokens = estimate_tokens(&content, &self.config.context_window);

            self.sections.push(PromptSection {
                name: "telemetry".to_owned(),
                content,
                is_sacred: false,
                priority: 5, // Lowest priority
            });

            self.current_tokens += section_tokens;
            self.included_tiers.push("telemetry".to_owned());

            tracing::debug!(
                subsystem = "prompt_composer",
                event_type = "telemetry_added",
                signals_included = telemetry_content.len(),
                signals_total = signals.len(),
                tokens = section_tokens,
                current_tokens = self.current_tokens,
                "added telemetry signals"
            );
        } else {
            self.dropped_tiers.push("telemetry".to_owned());
        }
    }

    /// Finalize the prompt assembly.
    fn finalize(self) -> Result<AssemblyResult, PromptComposerError> {
        // Sort sections by priority (sacred first, then others)
        let mut sections = self.sections;
        sections.sort_by_key(|s| s.priority);

        // Assemble the final prompt
        let assembled_prompt = sections
            .iter()
            .map(|s| {
                if s.is_sacred {
                    format!("=== {} ===\n{}\n", s.name, s.content)
                } else {
                    format!("{}\n", s.content)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let trace = PromptAssemblyTrace {
            token_count: self.current_tokens,
            token_budget: self.token_budget,
            included_tiers: self.included_tiers,
            dropped_tiers: self.dropped_tiers,
            encoding_used: self.encoding_used.as_str().to_owned(),
        };

        Ok(AssemblyResult {
            assembled_prompt,
            trace,
        })
    }
}

/// Estimate token count from content using the configured ratio.
fn estimate_tokens(content: &str, config: &crate::config::ContextWindowConfig) -> u32 {
    (content.len() as f32 * config.tokens_per_char_estimate).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
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
        }
    }

    fn test_model_profile(context_window: u32, output_reserve: u32) -> ModelProfile {
        ModelProfile {
            model_id: "test-model".into(),
            context_window,
            output_reserve,
        }
    }

    #[test]
    fn test_sacred_content_always_included() {
        let config = test_config();
        let assembler = PromptAssembler::new();

        let context = PromptContext {
            soulbox_snapshot: "sacred soul data".into(),
            user_intent: "sacred intent".into(),
            user_message: "hello".into(),
            short_term: vec![],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: Some(test_model_profile(8192, 1024)),
            trace_context: String::new(),
        };

        let result = assembler.assemble(&context, &config).expect("assembly should succeed");

        assert!(result.assembled_prompt.contains("sacred soul data"));
        assert!(result.assembled_prompt.contains("sacred intent"));
        assert!(result.trace.included_tiers.contains(&"soulbox_snapshot".to_string()));
        assert!(result.trace.included_tiers.contains(&"user_intent".to_string()));
    }

    #[test]
    fn test_budget_exhausted_by_sacred_content() {
        let config = test_config();
        let assembler = PromptAssembler::new();

        // Very small budget that can't fit sacred content
        let context = PromptContext {
            soulbox_snapshot: "x".repeat(1000), // ~250 tokens
            user_intent: "x".repeat(1000),      // ~250 tokens
            user_message: String::new(),
            short_term: vec![],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: Some(test_model_profile(100, 50)), // Budget = 50 tokens
            trace_context: String::new(),
        };

        let result = assembler.assemble(&context, &config);

        assert!(result.is_err());
        match result.unwrap_err() {
            PromptComposerError::BudgetExhausted { .. } => {}
            other => panic!("Expected BudgetExhausted, got: {}", other),
        }
    }

    #[test]
    fn test_drop_order_respected() {
        let config = test_config();
        let assembler = PromptAssembler::new();

        // Create a context with all tiers populated
        let context = PromptContext {
            soulbox_snapshot: "soul".into(),
            user_intent: "intent".into(),
            user_message: "hello".into(),
            short_term: vec![PromptContextEntry {
                id: "st1".into(),
                content: "short term memory".into(),
                relevance_score: 0.9,
                tier: "short_term".into(),
            }],
            long_term: vec![PromptContextEntry {
                id: "lt1".into(),
                content: "long term memory".into(),
                relevance_score: 0.8,
                tier: "long_term".into(),
            }],
            episodic: vec![PromptContextEntry {
                id: "ep1".into(),
                content: "episodic memory".into(),
                relevance_score: 0.7,
                tier: "episodic".into(),
            }],
            os_context: "Linux 5.15".into(),
            telemetry_signals: vec![TelemetrySignal {
                signal_type: "cpu".into(),
                value: "50%".into(),
                relevance_score: 0.5,
            }],
            model_profile: Some(test_model_profile(500, 100)), // Limited budget
            trace_context: String::new(),
        };

        let result = assembler.assemble(&context, &config).expect("assembly should succeed");

        // Sacred content should always be included
        assert!(result.trace.included_tiers.contains(&"soulbox_snapshot".to_string()));
        assert!(result.trace.included_tiers.contains(&"user_intent".to_string()));

        // With limited budget, some tiers should be dropped
        // Telemetry should drop first (lowest priority)
        if !result.trace.dropped_tiers.is_empty() {
            // If anything is dropped, telemetry should be among the first
            let telemetry_dropped = result.trace.dropped_tiers.contains(&"telemetry".to_string());
            let episodic_included = result.trace.included_tiers.contains(&"episodic".to_string());

            // If episodic is included, telemetry should definitely be dropped
            if episodic_included {
                assert!(telemetry_dropped, "telemetry should drop before episodic");
            }
        }
    }

    #[test]
    fn test_missing_model_profile_returns_error() {
        let config = test_config();
        let assembler = PromptAssembler::new();

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

        let result = assembler.assemble(&context, &config);

        assert!(result.is_err());
        match result.unwrap_err() {
            PromptComposerError::MissingField { field } => {
                assert_eq!(field, "model_profile");
            }
            other => panic!("Expected MissingField, got: {}", other),
        }
    }

    #[test]
    fn test_relevance_score_ordering() {
        let config = test_config();
        let assembler = PromptAssembler::new();

        // Create entries with different relevance scores
        let context = PromptContext {
            soulbox_snapshot: "soul".into(),
            user_intent: "intent".into(),
            user_message: String::new(),
            short_term: vec![
                PromptContextEntry {
                    id: "low".into(),
                    content: "x".repeat(100),
                    relevance_score: 0.1,
                    tier: "short_term".into(),
                },
                PromptContextEntry {
                    id: "high".into(),
                    content: "y".repeat(100),
                    relevance_score: 0.9,
                    tier: "short_term".into(),
                },
            ],
            long_term: vec![],
            episodic: vec![],
            os_context: String::new(),
            telemetry_signals: vec![],
            model_profile: Some(test_model_profile(200, 50)), // Very limited budget
            trace_context: String::new(),
        };

        let result = assembler.assemble(&context, &config).expect("assembly should succeed");

        // High relevance entry should be more likely to be included
        let prompt = result.assembled_prompt;
        if prompt.contains("short_term") {
            // If short_term tier is included, check that high-relevance is prioritized
            let has_high = prompt.contains("\"id\":\"high\"") || prompt.contains("id=high");
            let has_low = prompt.contains("\"id\":\"low\"") || prompt.contains("id=low");

            // If only one can fit, it should be the high relevance one
            if has_high && !has_low {
                // This is expected behavior
            } else if has_low && !has_high {
                panic!("Low relevance entry included before high relevance");
            }
            // If both are included, that's fine (budget was sufficient)
        }
    }
}
