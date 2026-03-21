/// Encoding Selection Utility (ESU)
///
/// Selects between TOON and JSON encoding based on estimated token savings.
/// Sacred content always prefers fidelity (JSON) over compression.

use crate::config::ContextWindowConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EncodingChoice {
    Toon,
    Json,
}

impl EncodingChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            EncodingChoice::Toon => "TOON",
            EncodingChoice::Json => "JSON",
        }
    }
}

/// Selects the optimal encoding for the given content.
///
/// For Phase 1, this uses a simplified TOON encoding (key=value pairs).
/// In Phase 2, this will integrate with the actual toon-format crate.
pub fn select_encoding(
    content: &str,
    is_sacred: bool,
    config: &ContextWindowConfig,
) -> (EncodingChoice, String) {
    // Sacred content always uses JSON for fidelity
    if is_sacred {
        return (EncodingChoice::Json, content.to_owned());
    }

    // Estimate token counts for both encodings
    let json_tokens = estimate_tokens(content, config.tokens_per_char_estimate);
    let toon_encoded = encode_toon_simplified(content);
    let toon_tokens = estimate_tokens(&toon_encoded, config.tokens_per_char_estimate);

    // Calculate savings percentage
    let savings = if json_tokens > 0 {
        (json_tokens as f32 - toon_tokens as f32) / json_tokens as f32
    } else {
        0.0
    };

    tracing::debug!(
        subsystem = "prompt_composer",
        event_type = "esu_selection",
        json_tokens = json_tokens,
        toon_tokens = toon_tokens,
        savings_pct = %format!("{:.1}%", savings * 100.0),
        threshold_pct = %format!("{:.1}%", config.esu_savings_threshold * 100.0),
        selected = if savings >= config.esu_savings_threshold { "TOON" } else { "JSON" },
        "ESU encoding selection"
    );

    // Prefer TOON if savings exceed threshold
    if savings >= config.esu_savings_threshold {
        (EncodingChoice::Toon, toon_encoded)
    } else {
        (EncodingChoice::Json, content.to_owned())
    }
}

/// Estimate token count from character count.
///
/// This is a rough approximation. In Phase 2, we'll integrate with a proper
/// tokenizer that matches the inference model's tokenization.
fn estimate_tokens(content: &str, tokens_per_char: f32) -> u32 {
    (content.len() as f32 * tokens_per_char).ceil() as u32
}

/// Simplified TOON encoding for Phase 1.
///
/// Converts JSON-like structures to a more compact key=value format.
/// This is a placeholder until the toon-format crate is implemented.
///
/// Example transformation:
/// ```json
/// {"type": "memory", "content": "User prefers dark mode", "relevance": 0.9}
/// ```
/// becomes:
/// ```
/// type=memory content="User prefers dark mode" relevance=0.9
/// ```
fn encode_toon_simplified(content: &str) -> String {
    // Try to parse as JSON first
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(content) {
        match json_value {
            serde_json::Value::Object(map) => {
                // Convert to key=value pairs
                let pairs: Vec<String> = map
                    .iter()
                    .map(|(k, v)| match v {
                        serde_json::Value::String(s) => {
                            // Quote strings if they contain spaces
                            if s.contains(' ') {
                                format!("{}=\"{}\"", k, s)
                            } else {
                                format!("{}={}", k, s)
                            }
                        }
                        serde_json::Value::Number(n) => format!("{}={}", k, n),
                        serde_json::Value::Bool(b) => format!("{}={}", k, b),
                        _ => format!("{}={}", k, v),
                    })
                    .collect();
                pairs.join(" ")
            }
            _ => {
                // Not an object — keep as-is
                content.to_owned()
            }
        }
    } else {
        // Not valid JSON — keep as-is
        content.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ContextWindowConfig {
        ContextWindowConfig {
            esu_savings_threshold: 0.15,
            tokens_per_char_estimate: 0.25,
        }
    }

    #[test]
    fn test_sacred_content_always_json() {
        let config = test_config();
        let content = r#"{"soul": "essence", "identity": "core"}"#;

        let (choice, encoded) = select_encoding(content, true, &config);

        assert_eq!(choice, EncodingChoice::Json);
        assert_eq!(encoded, content);
    }

    #[test]
    fn test_toon_encoding_simplified() {
        let json = r#"{"type": "memory", "relevance": 0.9}"#;
        let toon = encode_toon_simplified(json);

        // Should be more compact (no quotes, no braces, no commas)
        assert!(toon.len() < json.len());
        assert!(toon.contains("type=memory"));
        assert!(toon.contains("relevance=0.9"));
    }

    #[test]
    fn test_toon_selected_when_savings_exceed_threshold() {
        let config = ContextWindowConfig {
            esu_savings_threshold: 0.10, // 10% threshold
            tokens_per_char_estimate: 0.25,
        };

        // Long JSON with lots of formatting overhead
        let json = r#"{"field_one": "value", "field_two": "value", "field_three": "value"}"#;

        let (choice, _) = select_encoding(json, false, &config);

        // TOON should be selected due to savings
        assert_eq!(choice, EncodingChoice::Toon);
    }

    #[test]
    fn test_json_selected_when_savings_below_threshold() {
        let config = ContextWindowConfig {
            esu_savings_threshold: 0.50, // 50% threshold (very high)
            tokens_per_char_estimate: 0.25,
        };

        // Plain text that won't compress well with TOON (not JSON)
        let text = "This is just plain text without structure";

        let (choice, encoded) = select_encoding(text, false, &config);

        // JSON should be selected because there's no savings (not valid JSON)
        assert_eq!(choice, EncodingChoice::Json);
        assert_eq!(encoded, text);
    }

    #[test]
    fn test_estimate_tokens() {
        let config = test_config();
        let content = "a".repeat(100); // 100 characters

        let tokens = estimate_tokens(&content, config.tokens_per_char_estimate);

        // 100 chars * 0.25 = 25 tokens
        assert_eq!(tokens, 25);
    }

    #[test]
    fn test_toon_encoding_with_quoted_strings() {
        let json = r#"{"message": "hello world", "count": 42}"#;
        let toon = encode_toon_simplified(json);

        // String with spaces should be quoted
        assert!(toon.contains("message=\"hello world\""));
        // Number should not be quoted
        assert!(toon.contains("count=42"));
    }

    #[test]
    fn test_encoding_choice_as_str() {
        assert_eq!(EncodingChoice::Toon.as_str(), "TOON");
        assert_eq!(EncodingChoice::Json.as_str(), "JSON");
    }
}
