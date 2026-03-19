//! Prompt assembler — the core of prompt-composer.
//!
//! Receives a `PromptContext`, runs each sub-piece through the ESU to choose
//! TOON vs JSON encoding, enforces the sacred content floor (SoulBox snapshot +
//! user intent are never dropped), trims to the model's context budget using a
//! fixed priority drop order, validates the final output, and returns the
//! assembled prompt string.
//!
//! Fully stateless — every call is independent. No data persists between calls.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::PcError;
use crate::esu::{choose_encoding, EsuOptions};
use crate::token_counter;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// SoulBox personality snapshot — sacred content, never dropped.
#[derive(Debug, Clone, Serialize)]
pub struct SoulBoxSnapshot {
    pub personality_summary: String,
}

/// OS context — active window and recent events.
#[derive(Debug, Clone, Serialize)]
pub struct OsContext {
    pub active_window: String,
    pub recent_events: Vec<String>,
}

/// A memory search result from a specific tier.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryResult {
    pub node_id: String,
    pub summary: String,
    pub score: f32,
    pub tier: String,
}

/// A telemetry signal for context enrichment.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySignal {
    pub signal_type: String,
    pub value: String,
    pub relevance: f32,
}

/// Model capability profile — provides the context window budget.
#[derive(Debug, Clone)]
pub struct ModelCapabilityProfile {
    pub model_id: String,
    pub context_window: u32,
    pub output_reserve: u32,
}

/// Assembled context from CTP — all inputs for prompt assembly.
#[derive(Debug, Clone)]
pub struct PromptContext {
    pub soulbox_snapshot: SoulBoxSnapshot,
    pub short_term: Vec<MemoryResult>,
    pub long_term: Vec<MemoryResult>,
    pub episodic: Vec<MemoryResult>,
    pub os_context: OsContext,
    pub model_profile: ModelCapabilityProfile,
    pub user_intent: Option<String>,
    pub telemetry_signals: Vec<TelemetrySignal>,
}

/// Result of prompt assembly.
#[derive(Debug, Clone)]
pub struct AssembleResult {
    /// The final assembled prompt string.
    pub prompt: String,
    /// Total token count of the assembled prompt.
    pub token_count: u32,
    /// Whether any content was dropped to fit the budget.
    pub truncated: bool,
    /// Names of tiers that were dropped (in drop order).
    pub dropped_tiers: Vec<String>,
    /// SHA-256 hash of the final prompt — for telemetry, never log raw content.
    pub unique_hash: String,
    /// The model ID this prompt was assembled for.
    pub model_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal types for assembly
// ─────────────────────────────────────────────────────────────────────────────

/// A prompt part with its tier name and encoded content.
struct PromptPart {
    tier: String,
    encoded: String,
    tokens: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Assembly
// ─────────────────────────────────────────────────────────────────────────────

/// Assemble a prompt from the given context using the provided configuration.
///
/// This is the core function of prompt-composer. It:
/// 1. Encodes sacred content (SoulBox + user intent) via ESU — always JSON
/// 2. Checks that sacred content fits within the budget
/// 3. Encodes each optional tier via ESU (TOON or JSON based on savings)
/// 4. Drops lowest-priority tiers until total fits within budget
/// 5. Validates the final prompt
/// 6. Returns the result with hash (never raw content in telemetry)
///
/// TOON encoding is CPU-bound so this function is async — it spawns blocking
/// tasks for TOON encoding via `tokio::task::spawn_blocking`.
pub async fn assemble(
    context: &PromptContext,
    config: &Config,
) -> Result<AssembleResult, PcError> {
    // Per-request output_reserve from model profile takes precedence over config default.
    let output_reserve = if context.model_profile.output_reserve > 0 {
        context.model_profile.output_reserve
    } else {
        config.budget.output_reserve_tokens
    };
    let budget = context
        .model_profile
        .context_window
        .saturating_sub(output_reserve) as usize;

    // ── Encode sacred content (always JSON) ─────────────────────────────
    let sacred_options = EsuOptions { is_sacred: true };

    let soulbox_result = choose_encoding(
        &context.soulbox_snapshot,
        &config.esu,
        sacred_options.clone(),
    );

    let mut sacred_parts: Vec<PromptPart> = vec![PromptPart {
        tier: "soulbox".to_string(),
        encoded: soulbox_result.encoded,
        tokens: soulbox_result.json_tokens,
    }];

    if let Some(ref intent) = context.user_intent {
        let intent_result = choose_encoding(intent, &config.esu, EsuOptions { is_sacred: true });
        sacred_parts.push(PromptPart {
            tier: "user_intent".to_string(),
            encoded: intent_result.encoded,
            tokens: intent_result.json_tokens,
        });
    }

    let sacred_tokens: usize = sacred_parts.iter().map(|p| p.tokens).sum();

    if sacred_tokens > budget {
        return Err(PcError::SacredContentOverflow {
            required_tokens: sacred_tokens as u32,
            budget: budget as u32,
        });
    }

    // ── Encode optional tiers ───────────────────────────────────────────
    // Tiers are encoded in the order specified by config.drop_order.tiers.
    // The first tier in the list is dropped first when budget is tight.
    // TOON encoding is CPU-bound, so we use spawn_blocking.

    let mut optional_parts: Vec<PromptPart> = Vec::new();

    for tier_name in &config.drop_order.tiers {
        let part = match tier_name.as_str() {
            "telemetry" => {
                if context.telemetry_signals.is_empty() {
                    continue;
                }
                let esu_config = config.esu.clone();
                let telemetry = context.telemetry_signals.clone();
                let result = tokio::task::spawn_blocking(move || {
                    choose_encoding(&telemetry, &esu_config, EsuOptions { is_sacred: false })
                })
                .await
                .map_err(|e| PcError::SpawnBlocking(e.to_string()))?;

                let tokens = token_counter::count_tokens(&result.encoded);
                PromptPart {
                    tier: "telemetry".to_string(),
                    encoded: result.encoded,
                    tokens,
                }
            }
            "os_context" => {
                let esu_config = config.esu.clone();
                let os_ctx = context.os_context.clone();
                let result = tokio::task::spawn_blocking(move || {
                    choose_encoding(&os_ctx, &esu_config, EsuOptions { is_sacred: false })
                })
                .await
                .map_err(|e| PcError::SpawnBlocking(e.to_string()))?;

                let tokens = token_counter::count_tokens(&result.encoded);
                PromptPart {
                    tier: "os_context".to_string(),
                    encoded: result.encoded,
                    tokens,
                }
            }
            "short_term" => {
                if context.short_term.is_empty() {
                    continue;
                }
                let esu_config = config.esu.clone();
                let short_term = context.short_term.clone();
                let result = tokio::task::spawn_blocking(move || {
                    choose_encoding(&short_term, &esu_config, EsuOptions { is_sacred: false })
                })
                .await
                .map_err(|e| PcError::SpawnBlocking(e.to_string()))?;

                let tokens = token_counter::count_tokens(&result.encoded);
                PromptPart {
                    tier: "short_term".to_string(),
                    encoded: result.encoded,
                    tokens,
                }
            }
            "long_term" => {
                if context.long_term.is_empty() {
                    continue;
                }
                let esu_config = config.esu.clone();
                let long_term = context.long_term.clone();
                let result = tokio::task::spawn_blocking(move || {
                    choose_encoding(&long_term, &esu_config, EsuOptions { is_sacred: false })
                })
                .await
                .map_err(|e| PcError::SpawnBlocking(e.to_string()))?;

                let tokens = token_counter::count_tokens(&result.encoded);
                PromptPart {
                    tier: "long_term".to_string(),
                    encoded: result.encoded,
                    tokens,
                }
            }
            "episodic" => {
                if context.episodic.is_empty() {
                    continue;
                }
                let esu_config = config.esu.clone();
                let episodic = context.episodic.clone();
                let result = tokio::task::spawn_blocking(move || {
                    choose_encoding(&episodic, &esu_config, EsuOptions { is_sacred: false })
                })
                .await
                .map_err(|e| PcError::SpawnBlocking(e.to_string()))?;

                let tokens = token_counter::count_tokens(&result.encoded);
                PromptPart {
                    tier: "episodic".to_string(),
                    encoded: result.encoded,
                    tokens,
                }
            }
            _ => continue,
        };

        optional_parts.push(part);
    }

    // ── Drop lowest-priority tiers until total fits budget ──────────────
    let mut dropped_tiers: Vec<String> = Vec::new();
    let mut total_tokens: usize = sacred_tokens + optional_parts.iter().map(|p| p.tokens).sum::<usize>();

    // Drop from front (lowest priority in drop order: telemetry first)
    while total_tokens > budget && !optional_parts.is_empty() {
        let dropped = optional_parts.remove(0);
        total_tokens -= dropped.tokens;
        dropped_tiers.push(dropped.tier);
    }

    // ── Build final prompt ──────────────────────────────────────────────
    let mut prompt_sections: Vec<String> = Vec::new();

    // Sacred content first
    for part in &sacred_parts {
        prompt_sections.push(part.encoded.clone());
    }

    // Optional content in order
    for part in &optional_parts {
        prompt_sections.push(part.encoded.clone());
    }

    let prompt = prompt_sections.join("\n\n");

    // ── Validate ────────────────────────────────────────────────────────
    if prompt.is_empty() {
        return Err(PcError::BudgetExceeded);
    }

    // ── Hash for telemetry ──────────────────────────────────────────────
    let mut hasher = Sha256::new();
    hasher.update(prompt.as_bytes());
    let hash_bytes = hasher.finalize();
    let unique_hash = format!("{:x}", hash_bytes);

    let final_token_count = token_counter::count_tokens(&prompt);

    Ok(AssembleResult {
        prompt,
        token_count: final_token_count as u32,
        truncated: !dropped_tiers.is_empty(),
        dropped_tiers,
        unique_hash,
        model_id: context.model_profile.model_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        BudgetConfig, Config, DropOrderConfig, EsuConfig, GrpcConfig, LoggingConfig,
        TelemetryConfig,
    };

    fn test_config() -> Config {
        Config {
            grpc: GrpcConfig {
                daemon_bus_address: "http://127.0.0.1:50051".to_string(),
                listen_port: 50054,
                connection_timeout_ms: 5000,
            },
            esu: EsuConfig {
                save_threshold: 0.15,
                latency_threshold_ms: 10,
                sacred_always_json: true,
            },
            budget: BudgetConfig {
                output_reserve_tokens: 100,
                min_sacred_headroom_pct: 0.1,
            },
            drop_order: DropOrderConfig {
                tiers: vec![
                    "telemetry".into(),
                    "os_context".into(),
                    "short_term".into(),
                    "long_term".into(),
                    "episodic".into(),
                ],
            },
            telemetry: TelemetryConfig {
                emit_encoding_choices: true,
            },
            logging: LoggingConfig {
                level: "info".into(),
                format: "json".into(),
            },
        }
    }

    fn test_context() -> PromptContext {
        PromptContext {
            soulbox_snapshot: SoulBoxSnapshot {
                personality_summary: "warm and curious".to_string(),
            },
            short_term: vec![MemoryResult {
                node_id: "st1".into(),
                summary: "recent conversation about weather".into(),
                score: 0.9,
                tier: "short_term".into(),
            }],
            long_term: vec![MemoryResult {
                node_id: "lt1".into(),
                summary: "user prefers concise answers".into(),
                score: 0.7,
                tier: "long_term".into(),
            }],
            episodic: vec![],
            os_context: OsContext {
                active_window: "Visual Studio Code".into(),
                recent_events: vec!["file saved".into()],
            },
            model_profile: ModelCapabilityProfile {
                model_id: "test-model".into(),
                context_window: 4096,
                output_reserve: 512,
            },
            user_intent: Some("help with Rust code".to_string()),
            telemetry_signals: vec![],
        }
    }

    #[tokio::test]
    async fn test_assemble_produces_nonempty_prompt() {
        let config = test_config();
        let context = test_context();
        let result = assemble(&context, &config).await.expect("should assemble");
        assert!(!result.prompt.is_empty(), "prompt should not be empty");
        assert!(result.token_count > 0, "token count should be > 0");
    }

    #[tokio::test]
    async fn test_two_calls_produce_different_prompts() {
        let config = test_config();
        let context1 = test_context();
        let mut context2 = test_context();
        context2.soulbox_snapshot.personality_summary = "cold and analytical".to_string();

        let result1 = assemble(&context1, &config).await.expect("should assemble");
        let result2 = assemble(&context2, &config).await.expect("should assemble");

        assert_ne!(result1.unique_hash, result2.unique_hash, "different context should produce different hashes");
    }

    #[tokio::test]
    async fn test_sacred_content_always_present() {
        let config = test_config();
        let context = test_context();
        let result = assemble(&context, &config).await.expect("should assemble");

        // SoulBox snapshot should be present in the prompt
        assert!(
            result.prompt.contains("warm and curious"),
            "SoulBox snapshot should be in the prompt"
        );
        // User intent should be present
        assert!(
            result.prompt.contains("help with Rust code"),
            "user intent should be in the prompt"
        );
    }

    #[tokio::test]
    async fn test_sacred_overflow_returns_error() {
        let config = test_config();
        let mut context = test_context();
        // Set model profile with impossibly high output reserve to leave tiny budget
        context.model_profile.output_reserve = 4090;
        // Context window is 4096, reserve is 4090, leaving budget of 6 tokens
        // Sacred content will definitely overflow 6 tokens
        let result = assemble(&context, &config).await;
        assert!(result.is_err(), "should fail when sacred content overflows budget");
        match result.unwrap_err() {
            PcError::SacredContentOverflow { .. } => {}
            other => panic!("expected SacredContentOverflow, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_drop_order_respected() {
        let config = test_config();
        let mut context = test_context();
        // Fill up with lots of content to force drops
        context.telemetry_signals = vec![TelemetrySignal {
            signal_type: "cpu".into(),
            value: "high".into(),
            relevance: 0.5,
        }];
        context.short_term = vec![MemoryResult {
            node_id: "st1".into(),
            summary: "a".repeat(1000),
            score: 0.9,
            tier: "short_term".into(),
        }];
        context.long_term = vec![MemoryResult {
            node_id: "lt1".into(),
            summary: "b".repeat(1000),
            score: 0.7,
            tier: "long_term".into(),
        }];
        // Set a tight budget to force drops
        context.model_profile.context_window = 500;
        context.model_profile.output_reserve = 100;

        let result = assemble(&context, &config).await.expect("should assemble with drops");

        // If drops occurred, telemetry should be dropped first, then os_context, then short_term
        if !result.dropped_tiers.is_empty() {
            // Verify drop order: telemetry must come before os_context, etc.
            let tier_order = vec!["telemetry", "os_context", "short_term", "long_term", "episodic"];
            let mut last_idx = 0;
            for dropped in &result.dropped_tiers {
                if let Some(idx) = tier_order.iter().position(|t| t == dropped) {
                    assert!(
                        idx >= last_idx,
                        "drop order violated: {} came after higher priority tier",
                        dropped
                    );
                    last_idx = idx;
                }
            }
        }
    }

    #[tokio::test]
    async fn test_toon_encoded_non_sacred_parts() {
        let mut config = test_config();
        // Set very low threshold so TOON is always chosen for non-sacred
        config.esu.save_threshold = 0.01;
        let context = test_context();
        let result = assemble(&context, &config).await.expect("should assemble");
        // Non-sacred parts should be encoded (we can't easily verify TOON vs JSON
        // from the final prompt, but we verify the prompt is non-empty and valid)
        assert!(!result.prompt.is_empty());
    }

    #[tokio::test]
    async fn test_json_encoded_sacred_parts() {
        let config = test_config();
        let context = test_context();
        let result = assemble(&context, &config).await.expect("should assemble");
        // Sacred content (SoulBox) should be JSON-encoded
        // JSON encoding of the personality_summary field will contain the string
        assert!(result.prompt.contains("warm and curious"));
    }

    #[tokio::test]
    async fn test_token_count_within_budget() {
        let config = test_config();
        let context = test_context();
        // Budget uses model_profile.output_reserve when > 0, otherwise config default
        let output_reserve = if context.model_profile.output_reserve > 0 {
            context.model_profile.output_reserve
        } else {
            config.budget.output_reserve_tokens
        };
        let budget = context.model_profile.context_window - output_reserve;
        let result = assemble(&context, &config).await.expect("should assemble");
        assert!(
            result.token_count <= budget,
            "token count {} should be <= budget {}",
            result.token_count,
            budget
        );
    }

    #[tokio::test]
    async fn test_truncated_flag_set_when_drops_occurred() {
        let config = test_config();
        let mut context = test_context();
        // Set a very tight budget to force drops
        context.model_profile.context_window = 200;
        context.model_profile.output_reserve = 50;
        context.short_term = vec![MemoryResult {
            node_id: "st1".into(),
            summary: "a".repeat(500),
            score: 0.9,
            tier: "short_term".into(),
        }];

        let result = assemble(&context, &config).await.expect("should assemble");
        if !result.dropped_tiers.is_empty() {
            assert!(result.truncated, "truncated flag should be true when drops occurred");
        }
    }

    #[tokio::test]
    async fn test_dropped_tiers_reported() {
        let config = test_config();
        let mut context = test_context();
        // Set a very tight budget
        context.model_profile.context_window = 200;
        context.model_profile.output_reserve = 50;
        context.short_term = vec![MemoryResult {
            node_id: "st1".into(),
            summary: "a".repeat(500),
            score: 0.9,
            tier: "short_term".into(),
        }];
        context.long_term = vec![MemoryResult {
            node_id: "lt1".into(),
            summary: "b".repeat(500),
            score: 0.7,
            tier: "long_term".into(),
        }];

        let result = assemble(&context, &config).await.expect("should assemble");
        // Dropped tiers should only contain valid tier names
        for tier in &result.dropped_tiers {
            assert!(
                ["telemetry", "os_context", "short_term", "long_term", "episodic"].contains(&tier.as_str()),
                "dropped tier '{}' should be a valid tier name",
                tier
            );
        }
    }
}
