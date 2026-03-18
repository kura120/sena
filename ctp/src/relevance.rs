//! Relevance scoring engine.
//!
//! Computes a relevance score in [0.0, 1.0] for a candidate thought given
//! a set of input signals and relevance weights. Weights come from a
//! `WeightsSnapshot` — either from SoulBox (future) or the config default
//! fallback (Phase 1).
//!
//! `compute_score` is a pure function — no side effects, no I/O.

use crate::config::DefaultWeights;

/// Input signals for relevance scoring. All values are in [0.0, 1.0].
#[derive(Debug, Clone)]
pub struct SignalInput {
    pub urgency: f32,
    pub emotional_resonance: f32,
    pub novelty: f32,
    pub recurrence: f32,
    pub idle_curiosity: f32,
}

/// Snapshot of relevance weights — loaded from config (default) or SoulBox (future).
#[derive(Debug, Clone)]
pub struct WeightsSnapshot {
    pub urgency: f32,
    pub emotional_resonance: f32,
    pub novelty: f32,
    pub recurrence: f32,
    pub idle_curiosity: f32,
}

/// Compute a relevance score in [0.0, 1.0] as a weighted average of signals.
///
/// This is a pure function — no side effects, no I/O, fully deterministic.
pub fn compute_score(signals: &SignalInput, weights: &WeightsSnapshot) -> f32 {
    let total_weight = weights.urgency
        + weights.emotional_resonance
        + weights.novelty
        + weights.recurrence
        + weights.idle_curiosity;

    if total_weight <= 0.0 {
        return 0.0;
    }

    let weighted_sum = signals.urgency * weights.urgency
        + signals.emotional_resonance * weights.emotional_resonance
        + signals.novelty * weights.novelty
        + signals.recurrence * weights.recurrence
        + signals.idle_curiosity * weights.idle_curiosity;

    (weighted_sum / total_weight).clamp(0.0, 1.0)
}

/// Build a `WeightsSnapshot` from config defaults — the fallback until SoulBox is live.
pub fn weights_from_config(config: &DefaultWeights) -> WeightsSnapshot {
    WeightsSnapshot {
        urgency: config.urgency,
        emotional_resonance: config.emotional_resonance,
        novelty: config.novelty,
        recurrence: config.recurrence,
        idle_curiosity: config.idle_curiosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_weights() -> WeightsSnapshot {
        WeightsSnapshot {
            urgency: 0.9,
            emotional_resonance: 0.7,
            novelty: 0.6,
            recurrence: 0.4,
            idle_curiosity: 0.3,
        }
    }

    fn mid_signals() -> SignalInput {
        SignalInput {
            urgency: 0.5,
            emotional_resonance: 0.5,
            novelty: 0.5,
            recurrence: 0.5,
            idle_curiosity: 0.5,
        }
    }

    #[test]
    fn test_score_clamps_to_unit_interval() {
        let weights = default_weights();

        // All max signals
        let max_signals = SignalInput {
            urgency: 1.0,
            emotional_resonance: 1.0,
            novelty: 1.0,
            recurrence: 1.0,
            idle_curiosity: 1.0,
        };
        let score = compute_score(&max_signals, &weights);
        assert!((0.0..=1.0).contains(&score), "max score out of range: {}", score);

        // All zero signals
        let zero_signals = SignalInput {
            urgency: 0.0,
            emotional_resonance: 0.0,
            novelty: 0.0,
            recurrence: 0.0,
            idle_curiosity: 0.0,
        };
        let score = compute_score(&zero_signals, &weights);
        assert!((0.0..=1.0).contains(&score), "zero score out of range: {}", score);

        // Values above 1.0 should still clamp
        let over_signals = SignalInput {
            urgency: 2.0,
            emotional_resonance: 2.0,
            novelty: 2.0,
            recurrence: 2.0,
            idle_curiosity: 2.0,
        };
        let score = compute_score(&over_signals, &weights);
        assert!((0.0..=1.0).contains(&score), "over score out of range: {}", score);
    }

    #[test]
    fn test_zero_weights_produces_zero_score() {
        let zero_weights = WeightsSnapshot {
            urgency: 0.0,
            emotional_resonance: 0.0,
            novelty: 0.0,
            recurrence: 0.0,
            idle_curiosity: 0.0,
        };
        let signals = mid_signals();
        let score = compute_score(&signals, &zero_weights);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_urgency_dominates_when_high() {
        let urgency_weights = WeightsSnapshot {
            urgency: 10.0,
            emotional_resonance: 0.1,
            novelty: 0.1,
            recurrence: 0.1,
            idle_curiosity: 0.1,
        };

        let high_urgency = SignalInput {
            urgency: 1.0,
            emotional_resonance: 0.0,
            novelty: 0.0,
            recurrence: 0.0,
            idle_curiosity: 0.0,
        };

        let score = compute_score(&high_urgency, &urgency_weights);
        assert!(score > 0.9, "urgency-dominated score should approach 1.0, got {}", score);
    }

    #[test]
    fn test_score_increases_with_signal_strength() {
        let weights = default_weights();

        let low_signals = SignalInput {
            urgency: 0.1,
            emotional_resonance: 0.1,
            novelty: 0.1,
            recurrence: 0.1,
            idle_curiosity: 0.1,
        };

        let high_signals = SignalInput {
            urgency: 0.9,
            emotional_resonance: 0.9,
            novelty: 0.9,
            recurrence: 0.9,
            idle_curiosity: 0.9,
        };

        let low_score = compute_score(&low_signals, &weights);
        let high_score = compute_score(&high_signals, &weights);

        assert!(
            high_score > low_score,
            "higher signals should produce higher score: low={}, high={}",
            low_score,
            high_score
        );
    }

    #[test]
    fn test_default_weights_produce_mid_range_score() {
        let weights = default_weights();
        let signals = mid_signals();
        let score = compute_score(&signals, &weights);
        assert!(
            score > 0.2 && score < 0.8,
            "mid-range score should be in (0.2, 0.8), got {}",
            score
        );
    }

    #[test]
    fn test_score_is_deterministic() {
        let weights = default_weights();
        let signals = mid_signals();

        let score1 = compute_score(&signals, &weights);
        let score2 = compute_score(&signals, &weights);
        let score3 = compute_score(&signals, &weights);

        assert_eq!(score1, score2);
        assert_eq!(score2, score3);
    }

    #[test]
    fn test_weights_from_config() {
        let config = DefaultWeights {
            urgency: 0.9,
            emotional_resonance: 0.7,
            novelty: 0.6,
            recurrence: 0.4,
            idle_curiosity: 0.3,
        };
        let snapshot = weights_from_config(&config);
        assert_eq!(snapshot.urgency, config.urgency);
        assert_eq!(snapshot.emotional_resonance, config.emotional_resonance);
        assert_eq!(snapshot.novelty, config.novelty);
        assert_eq!(snapshot.recurrence, config.recurrence);
        assert_eq!(snapshot.idle_curiosity, config.idle_curiosity);
    }
}
