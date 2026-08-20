//! Ranking.
//!
//! Every candidate gets a score built from independent signals, each normalized
//! to 0..1 before it is weighted, so that changing a weight changes importance
//! and nothing else. The weights are configuration, not constants: what makes a
//! file relevant differs between a monorepo and a library, and freezing one
//! formula would be a decision made on the wrong side of the interface.
//!
//! ```text
//! score = keyword·w1 + symbol·w2 + graph·w3 + recency·w4   (then decayed per hop)
//! ```

use serde::{Deserialize, Serialize};

/// How much each signal counts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankingWeights {
    /// Full-text relevance of the file's content.
    pub keyword: f64,
    /// Similarity between the query and the file's embedding.
    ///
    /// Zero by default, which is what keeps ranking identical for anyone who
    /// has not turned embeddings on.
    pub semantic: f64,
    /// A symbol in the file whose name matches the query.
    pub symbol: f64,
    /// How much the rest of the project depends on the file.
    pub graph: f64,
    /// How recently the file changed.
    pub recency: f64,
    /// Multiplier applied per graph hop away from a direct match.
    pub hop_decay: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        RankingWeights {
            // A name match is the strongest evidence a file is the subject of
            // the query; body text is weaker but far more available.
            keyword: 1.0,
            semantic: 0.0,
            symbol: 1.5,
            graph: 0.5,
            recency: 0.3,
            hop_decay: 0.4,
        }
    }
}

impl RankingWeights {
    /// Reject weights that would make scores meaningless.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("keyword", self.keyword),
            ("semantic", self.semantic),
            ("symbol", self.symbol),
            ("graph", self.graph),
            ("recency", self.recency),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("ranking.{name} must be zero or more"));
            }
        }
        if !(0.0..=1.0).contains(&self.hop_decay) {
            return Err("ranking.hop_decay must be between 0.0 and 1.0".into());
        }
        Ok(())
    }
}

/// The evidence gathered about one candidate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    /// Normalized full-text relevance, 0..1.
    pub keyword: f64,
    /// Embedding similarity to the query, 0..1. Zero when there is no vector
    /// for the file, which is the same as no evidence either way.
    #[serde(default)]
    pub semantic: f64,
    /// Best symbol name match, 0..1.
    pub symbol: f64,
    /// Normalized dependent count, 0..1.
    pub graph: f64,
    /// Normalized recency, 0..1.
    pub recency: f64,
    /// Graph hops from a direct match. Zero for a file the query hit directly.
    pub hops: u32,
}

/// Combine signals into a score.
pub fn score(signals: &Signals, weights: &RankingWeights) -> f64 {
    let base = signals.keyword * weights.keyword
        + signals.semantic * weights.semantic
        + signals.symbol * weights.symbol
        + signals.graph * weights.graph
        + signals.recency * weights.recency;

    base * weights.hop_decay.powi(signals.hops as i32)
}

/// Scale BM25 relevance into 0..1 against the best result in the same set.
///
/// BM25 has no absolute scale — it only compares documents within one query —
/// so normalizing against the best hit is the only honest way to combine it
/// with signals that do have one.
pub fn normalize_keyword(relevance: f64, best: f64) -> f64 {
    if best <= 0.0 || !relevance.is_finite() {
        return 0.0;
    }
    (relevance / best).clamp(0.0, 1.0)
}

/// Scale a dependent count into 0..1.
///
/// Logarithmic, because the difference between one dependent and five matters
/// much more than between fifty and fifty-five.
pub fn normalize_graph(dependents: usize, most_dependents: usize) -> f64 {
    if most_dependents == 0 {
        return 0.0;
    }
    let value = (1.0 + dependents as f64).ln();
    let ceiling = (1.0 + most_dependents as f64).ln();
    (value / ceiling).clamp(0.0, 1.0)
}

/// Scale a modification time into 0..1, halving every `half_life_days`.
///
/// Decay against the clock rather than against the other results: "changed this
/// morning" should mean the same thing in every query.
pub fn normalize_recency(mtime_ms: i64, now_ms: i64, half_life_days: f64) -> f64 {
    if mtime_ms <= 0 || half_life_days <= 0.0 {
        return 0.0;
    }
    let age_days = (now_ms - mtime_ms).max(0) as f64 / 86_400_000.0;
    0.5_f64.powf(age_days / half_life_days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights() -> RankingWeights {
        RankingWeights::default()
    }

    #[test]
    fn every_signal_raises_the_score() {
        let baseline = score(&Signals::default(), &weights());
        assert_eq!(baseline, 0.0);

        for signals in [
            Signals {
                keyword: 1.0,
                ..Signals::default()
            },
            Signals {
                symbol: 1.0,
                ..Signals::default()
            },
            Signals {
                graph: 1.0,
                ..Signals::default()
            },
            Signals {
                recency: 1.0,
                ..Signals::default()
            },
        ] {
            assert!(score(&signals, &weights()) > baseline);
        }
    }

    #[test]
    fn a_name_match_outranks_a_body_match() {
        let by_name = Signals {
            symbol: 1.0,
            ..Signals::default()
        };
        let by_body = Signals {
            keyword: 1.0,
            ..Signals::default()
        };
        assert!(score(&by_name, &weights()) > score(&by_body, &weights()));
    }

    #[test]
    fn distance_from_the_match_costs_score() {
        let direct = Signals {
            keyword: 1.0,
            ..Signals::default()
        };
        let one_hop = Signals { hops: 1, ..direct };
        let two_hops = Signals { hops: 2, ..direct };

        assert!(score(&direct, &weights()) > score(&one_hop, &weights()));
        assert!(score(&one_hop, &weights()) > score(&two_hops, &weights()));
    }

    #[test]
    fn weights_can_turn_a_signal_off() {
        let signals = Signals {
            graph: 1.0,
            ..Signals::default()
        };
        let without_graph = RankingWeights {
            graph: 0.0,
            ..weights()
        };
        assert_eq!(score(&signals, &without_graph), 0.0);
    }

    #[test]
    fn keyword_relevance_is_normalized_against_the_best_hit() {
        assert_eq!(normalize_keyword(4.0, 4.0), 1.0);
        assert_eq!(normalize_keyword(2.0, 4.0), 0.5);
        assert_eq!(normalize_keyword(-1.0, 4.0), 0.0, "clamped at zero");
        assert_eq!(normalize_keyword(1.0, 0.0), 0.0, "no best, no signal");
    }

    #[test]
    fn graph_importance_grows_but_flattens() {
        assert_eq!(normalize_graph(0, 10), 0.0);
        assert_eq!(normalize_graph(10, 10), 1.0);

        let small_step = normalize_graph(5, 50) - normalize_graph(1, 50);
        let large_step = normalize_graph(50, 50) - normalize_graph(46, 50);
        assert!(
            small_step > large_step,
            "early dependents matter more than late ones"
        );
    }

    #[test]
    fn recency_halves_over_the_half_life() {
        let now = 1_700_000_000_000;
        let day = 86_400_000;

        assert!((normalize_recency(now, now, 30.0) - 1.0).abs() < 1e-9);
        assert!((normalize_recency(now - 30 * day, now, 30.0) - 0.5).abs() < 1e-6);
        assert!(normalize_recency(now - 365 * day, now, 30.0) < 0.01);
        assert_eq!(normalize_recency(0, now, 30.0), 0.0);
    }

    #[test]
    fn future_timestamps_do_not_exceed_the_maximum() {
        let now = 1_700_000_000_000;
        assert_eq!(normalize_recency(now + 86_400_000, now, 30.0), 1.0);
    }

    #[test]
    fn weights_are_validated() {
        assert!(weights().validate().is_ok());

        let negative = RankingWeights {
            keyword: -1.0,
            ..weights()
        };
        assert!(negative.validate().unwrap_err().contains("ranking.keyword"));

        let bad_decay = RankingWeights {
            hop_decay: 1.5,
            ..weights()
        };
        assert!(bad_decay.validate().is_err());
    }
}
