//! Lightweight token estimation for prompt budget management.
//!
//! Uses a character-to-token ratio heuristic — approximately 1 token per 4
//! characters for English text. This avoids an external tokenizer dependency
//! while providing sufficient accuracy for context window budget decisions.
//!
//! Both functions are pure — no I/O, no allocations beyond the input, no panics.

/// Estimate the number of tokens in a text string.
///
/// Uses the approximation that 1 token ≈ 4 characters for English text.
/// Returns 0 for empty input.
pub fn count_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    (text.len() as f64 / 4.0).ceil() as usize
}

/// Estimate the percentage of token savings between JSON and TOON encodings.
///
/// Returns a value in [0.0, 1.0]. Returns 0.0 if `json_tokens` is 0 to avoid
/// division by zero.
pub fn estimate_savings_pct(json_tokens: usize, toon_tokens: usize) -> f32 {
    if json_tokens == 0 {
        return 0.0;
    }
    let savings = (json_tokens as f32 - toon_tokens as f32) / json_tokens as f32;
    savings.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_empty_string() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_count_tokens_short_ascii() {
        let count = count_tokens("hello world");
        assert!(count > 0, "short ASCII should produce > 0 tokens, got {}", count);
    }

    #[test]
    fn test_count_tokens_consistent() {
        let input = "the quick brown fox jumps over the lazy dog";
        let count1 = count_tokens(input);
        let count2 = count_tokens(input);
        assert_eq!(count1, count2, "same input must produce same count");
    }

    #[test]
    fn test_count_tokens_longer_input_higher_count() {
        let short = "hello";
        let long = "hello world, this is a much longer string for testing purposes";
        assert!(
            count_tokens(long) > count_tokens(short),
            "longer input should produce higher token count"
        );
    }

    #[test]
    fn test_count_tokens_unicode_handled() {
        // Must not panic on non-ASCII input
        let unicode = "こんにちは世界 🌍 Ñoño café";
        let count = count_tokens(unicode);
        assert!(count > 0, "unicode input should produce > 0 tokens");
    }

    #[test]
    fn test_estimation_under_1ms() {
        let large_input = "a".repeat(10_000); // 10KB string
        let start = std::time::Instant::now();
        let _count = count_tokens(&large_input);
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 1,
            "count_tokens on 10KB should be < 1ms, took {} ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_estimate_savings_pct_normal() {
        let savings = estimate_savings_pct(100, 80);
        assert!((savings - 0.2).abs() < 0.001, "expected ~0.2, got {}", savings);
    }

    #[test]
    fn test_estimate_savings_pct_zero_json() {
        assert_eq!(estimate_savings_pct(0, 0), 0.0);
    }

    #[test]
    fn test_estimate_savings_pct_clamped() {
        // If toon is somehow larger than json, clamp to 0.0
        let savings = estimate_savings_pct(50, 100);
        assert_eq!(savings, 0.0, "negative savings should clamp to 0.0");
    }
}
