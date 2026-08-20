//! Analysis: what a context is, and what optimizing it would achieve.
//!
//! Analysis never modifies anything and never applies a budget. It runs the
//! pipeline as a dry run so the reported figures come from the same code that
//! would do the real work — a prediction produced by a separate estimate would
//! drift away from reality immediately.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use ctxc_context::fragment;
use ctxc_core::optimization::SavingsByStage;
use ctxc_core::{ContentType, Context};

use crate::engine::Engine;
use crate::error::Result;

/// Structural facts about a context, plus a projection of optimizing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Where the context came from, as shown to the user.
    pub source: String,
    pub content_type: ContentType,
    pub bytes: u64,
    pub lines: u32,
    pub fragments: u32,
    /// Fragments whose content repeats an earlier fragment.
    pub duplicate_fragments: u32,
    pub tokens: u32,
    pub tokenizer: String,
    /// Token counts are estimates unless produced by the exact target
    /// tokenizer.
    pub estimated: bool,
    /// Optimizer that would handle this context.
    pub optimizer: String,
    /// Tokens that would remain after optimization, with no budget applied.
    pub projected_tokens: u32,
    pub projected_reduction: f64,
    pub projected_savings_by_stage: SavingsByStage,
}

impl Engine {
    /// Describe a context and project what optimizing it would do.
    pub fn analyze(&self, context: &Context) -> Result<Analysis> {
        let optimized = self.optimize_unbounded(context)?;
        let strategy = fragment::strategy_for(context.metadata.content_type);
        let fragments = fragment::split(&context.content, strategy);

        Ok(Analysis {
            source: ctxc_context::ingest::label(&context.metadata.source),
            content_type: context.metadata.content_type,
            bytes: context.metadata.byte_len,
            lines: context.content.lines().count() as u32,
            fragments: fragments.len() as u32,
            duplicate_fragments: count_duplicates(&fragments),
            tokens: optimized.result.original_tokens,
            tokenizer: optimized.result.tokenizer.clone(),
            estimated: optimized.result.estimated,
            optimizer: optimized.result.optimizer.clone(),
            projected_tokens: optimized.result.optimized_tokens,
            projected_reduction: optimized.result.reduction_ratio,
            projected_savings_by_stage: optimized.result.savings_by_stage,
        })
    }
}

/// Fragments repeating content that already appeared, ignoring whitespace.
fn count_duplicates(fragments: &[fragment::Fragment]) -> u32 {
    let mut seen = HashSet::new();
    fragments
        .iter()
        .filter(|item| {
            !seen.insert(
                item.content
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineOptions;
    use ctxc_core::{ContextSource, HeuristicTokenizer};
    use std::sync::Arc;

    fn engine() -> Engine {
        Engine::new(
            Arc::new(HeuristicTokenizer::new()),
            EngineOptions::default(),
        )
    }

    fn context(content_type: ContentType, content: &str) -> Context {
        Context::new(ContextSource::Stdin, content_type, content)
    }

    #[test]
    fn reports_structure_and_projection() {
        let input = "alpha paragraph\n\nbeta paragraph\n\nalpha paragraph\n";
        let analysis = engine()
            .analyze(&context(ContentType::PlainText, input))
            .unwrap();

        assert_eq!(analysis.content_type, ContentType::PlainText);
        assert_eq!(analysis.fragments, 3);
        assert_eq!(analysis.duplicate_fragments, 1);
        assert_eq!(analysis.lines, 5);
        assert_eq!(analysis.optimizer, "text");
        assert!(analysis.projected_tokens < analysis.tokens);
        assert!(analysis.projected_reduction > 0.0);
        assert!(analysis.projected_savings_by_stage.deduplication > 0);
    }

    #[test]
    fn analysis_ignores_the_budget() {
        let options = EngineOptions {
            optimization_enabled: true,
            default_budget: ctxc_core::TokenBudget::new(1),
            ..EngineOptions::default()
        };
        let engine = Engine::new(Arc::new(HeuristicTokenizer::new()), options);
        let input = "one\n\ntwo\n\nthree\n\nfour\n";

        let analysis = engine
            .analyze(&context(ContentType::PlainText, input))
            .unwrap();
        assert_eq!(
            analysis.projected_savings_by_stage.selection, 0,
            "analysis must not report cuts a budget would make"
        );
    }

    #[test]
    fn clean_content_projects_no_reduction() {
        let analysis = engine()
            .analyze(&context(ContentType::PlainText, "a single clean line\n"))
            .unwrap();

        assert_eq!(analysis.projected_reduction, 0.0);
        assert_eq!(analysis.duplicate_fragments, 0);
    }

    #[test]
    fn token_counts_are_labelled_as_estimates() {
        let analysis = engine()
            .analyze(&context(ContentType::PlainText, "text\n"))
            .unwrap();

        assert!(analysis.estimated);
        assert_eq!(analysis.tokenizer, "heuristic");
    }

    #[test]
    fn sources_are_labelled() {
        let context = Context::new(
            ContextSource::File {
                path: std::path::PathBuf::from("notes.md"),
            },
            ContentType::Markdown,
            "# notes\n",
        );
        assert_eq!(engine().analyze(&context).unwrap().source, "notes.md");
    }
}
