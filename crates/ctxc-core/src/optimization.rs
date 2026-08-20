//! The shape of an optimization outcome.
//!
//! These are data types, not logic: optimizers produce them, the engine
//! aggregates them, and the CLI and (later) the metrics subsystem read them.
//! Savings are attributed per stage, because "70% smaller" says nothing about
//! *why*, and a stage breakdown is what makes an optimizer debuggable.

use serde::{Deserialize, Serialize};

use crate::id::ContextId;

/// A step in the pipeline that can remove tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Removing noise: trailing whitespace, blank runs, control characters.
    Filtering,
    /// Removing repeated content.
    Deduplication,
    /// Rewriting content into a shorter form that means the same thing.
    Compression,
    /// Dropping whole fragments to fit a budget.
    Selection,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Filtering => "filtering",
            Stage::Deduplication => "deduplication",
            Stage::Compression => "compression",
            Stage::Selection => "selection",
        }
    }
}

/// Tokens saved by each stage.
///
/// Signed, because a stage can legitimately cost tokens: collapsing four
/// hundred repeated log lines into one and noting that it happened four hundred
/// times adds a few tokens to the line it keeps. Recording that honestly is
/// what makes the breakdown add up to the total every time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingsByStage {
    pub filtering: i32,
    pub deduplication: i32,
    pub compression: i32,
    pub selection: i32,
}

impl SavingsByStage {
    /// Add `tokens` to one stage. Negative values record a stage that grew the
    /// content.
    pub fn record(&mut self, stage: Stage, tokens: i32) {
        let slot = match stage {
            Stage::Filtering => &mut self.filtering,
            Stage::Deduplication => &mut self.deduplication,
            Stage::Compression => &mut self.compression,
            Stage::Selection => &mut self.selection,
        };
        *slot = slot.saturating_add(tokens);
    }

    /// Tokens saved across every stage.
    pub fn total(&self) -> i32 {
        self.filtering
            .saturating_add(self.deduplication)
            .saturating_add(self.compression)
            .saturating_add(self.selection)
    }

    /// Merge another breakdown into this one, for multi-input compilations.
    pub fn merge(&mut self, other: &SavingsByStage) {
        self.filtering = self.filtering.saturating_add(other.filtering);
        self.deduplication = self.deduplication.saturating_add(other.deduplication);
        self.compression = self.compression.saturating_add(other.compression);
        self.selection = self.selection.saturating_add(other.selection);
    }
}

/// What an optimization did, in numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationResult {
    /// Optimizer that produced this result.
    pub optimizer: String,
    /// Tokenizer the counts came from, and whether they are estimates.
    pub tokenizer: String,
    pub estimated: bool,
    pub original_tokens: u32,
    pub optimized_tokens: u32,
    /// Share of the original removed, between 0.0 and 1.0.
    pub reduction_ratio: f64,
    /// Original size divided by optimized size.
    pub compression_ratio: f64,
    pub preserved_fragments: u32,
    pub removed_fragments: u32,
    pub savings_by_stage: SavingsByStage,
}

/// What an optimizer measured while working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub original_tokens: u32,
    pub optimized_tokens: u32,
    pub preserved_fragments: u32,
    pub removed_fragments: u32,
}

impl OptimizationResult {
    /// Build a result, deriving the ratios from the token counts.
    pub fn new(
        optimizer: impl Into<String>,
        tokenizer: &dyn crate::token::Tokenizer,
        counts: Counts,
        savings_by_stage: SavingsByStage,
    ) -> Self {
        OptimizationResult {
            optimizer: optimizer.into(),
            tokenizer: tokenizer.name().to_owned(),
            estimated: tokenizer.is_estimate(),
            original_tokens: counts.original_tokens,
            optimized_tokens: counts.optimized_tokens,
            reduction_ratio: reduction_ratio(counts.original_tokens, counts.optimized_tokens),
            compression_ratio: compression_ratio(counts.original_tokens, counts.optimized_tokens),
            preserved_fragments: counts.preserved_fragments,
            removed_fragments: counts.removed_fragments,
            savings_by_stage,
        }
    }

    /// Tokens removed overall. Negative if optimization grew the content.
    pub fn tokens_saved(&self) -> i32 {
        (self.original_tokens as i64 - self.optimized_tokens as i64)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }
}

/// Optimized content plus the numbers describing how it got that way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizedContext {
    /// Id of the context this was derived from, so the original stays
    /// reachable at `ctxc://context/<id>`.
    pub source_id: ContextId,
    pub content: String,
    pub result: OptimizationResult,
}

impl OptimizedContext {
    /// Reference that resolves back to the unoptimized context.
    pub fn reference(&self) -> String {
        self.source_id.to_uri()
    }
}

/// Share of `original` removed, guarding against division by zero.
pub fn reduction_ratio(original: u32, optimized: u32) -> f64 {
    if original == 0 {
        return 0.0;
    }
    let saved = original.saturating_sub(optimized) as f64;
    saved / original as f64
}

/// How many times smaller the content became. Empty output reports the
/// original size rather than infinity.
pub fn compression_ratio(original: u32, optimized: u32) -> f64 {
    match (original, optimized) {
        (0, _) => 1.0,
        (original, 0) => original as f64,
        (original, optimized) => original as f64 / optimized as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_describe_the_reduction() {
        assert!((reduction_ratio(1000, 250) - 0.75).abs() < 1e-9);
        assert!((compression_ratio(1000, 250) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn ratios_survive_degenerate_inputs() {
        assert_eq!(reduction_ratio(0, 0), 0.0);
        assert_eq!(compression_ratio(0, 0), 1.0);
        assert_eq!(compression_ratio(100, 0), 100.0);
        assert_eq!(
            reduction_ratio(100, 200),
            0.0,
            "growth is reported as no reduction, never as a negative"
        );
    }

    #[test]
    fn stage_savings_add_up() {
        let mut savings = SavingsByStage::default();
        savings.record(Stage::Filtering, 400);
        savings.record(Stage::Filtering, 100);
        savings.record(Stage::Deduplication, 200);
        savings.record(Stage::Selection, 300);

        assert_eq!(savings.filtering, 500);
        assert_eq!(savings.total(), 1_000);
        assert_eq!(savings.compression, 0);
    }

    #[test]
    fn breakdowns_merge() {
        let mut left = SavingsByStage {
            filtering: 10,
            deduplication: 20,
            compression: 0,
            selection: 5,
        };
        left.merge(&SavingsByStage {
            filtering: 1,
            deduplication: 2,
            compression: 3,
            selection: 4,
        });

        assert_eq!(left.total(), 45);
        assert_eq!(left.compression, 3);
    }

    #[test]
    fn results_derive_their_ratios() {
        let result = OptimizationResult::new(
            "text",
            &crate::token::HeuristicTokenizer::new(),
            Counts {
                original_tokens: 1_000,
                optimized_tokens: 400,
                preserved_fragments: 8,
                removed_fragments: 2,
            },
            SavingsByStage {
                filtering: 300,
                deduplication: 300,
                compression: 0,
                selection: 0,
            },
        );

        assert_eq!(result.tokens_saved(), 600);
        assert!((result.reduction_ratio - 0.6).abs() < 1e-9);
        assert_eq!(result.savings_by_stage.total(), 600);
    }

    #[test]
    fn stage_names_are_stable() {
        assert_eq!(Stage::Filtering.as_str(), "filtering");
        assert_eq!(
            serde_json::to_string(&Stage::Deduplication).unwrap(),
            "\"deduplication\""
        );
    }
}
