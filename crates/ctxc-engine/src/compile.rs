//! Compilation: several contexts into one AI-ready document.
//!
//! Each input is optimized, then labelled with where it came from and with the
//! reference that resolves back to its unoptimized form. Budget is spent in
//! input order: the first input may use the whole budget, later ones get what
//! is left. That is a deliberate placeholder for relevance-driven allocation —
//! it is predictable and explainable, which is what a budget has to be before
//! it can be clever.

use serde::{Deserialize, Serialize};

use ctxc_context::ingest;
use ctxc_core::optimization::{
    compression_ratio, reduction_ratio, OptimizationResult, SavingsByStage,
};
use ctxc_core::{Context, TokenBudget};

use crate::engine::Engine;
use crate::error::{EngineError, Result};

/// One input's contribution to a compilation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledSection {
    /// Where this section came from.
    pub source: String,
    /// Reference that resolves back to the unoptimized context.
    pub reference: String,
    pub result: OptimizationResult,
}

/// Several optimized contexts, assembled into one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compilation {
    pub content: String,
    pub sections: Vec<CompiledSection>,
    pub original_tokens: u32,
    pub optimized_tokens: u32,
    pub reduction_ratio: f64,
    pub compression_ratio: f64,
    pub savings_by_stage: SavingsByStage,
    /// Budget the compilation was held to, if any.
    pub budget: Option<u32>,
}

impl Engine {
    /// Optimize several contexts and join them into one document.
    pub fn compile(
        &self,
        contexts: &[Context],
        budget: Option<TokenBudget>,
    ) -> Result<Compilation> {
        if contexts.is_empty() {
            return Err(EngineError::NoInput);
        }

        let budget = budget.or(Some(self.options().default_budget));
        let mut remaining = budget.map(TokenBudget::total);

        let mut content = String::new();
        let mut sections = Vec::with_capacity(contexts.len());
        let mut savings = SavingsByStage::default();
        let mut original_tokens = 0u32;

        for context in contexts {
            let source = ingest::label(&context.metadata.source);
            let header = section_header(&source, &context.id.to_uri());

            // The header is part of what the caller pays for, so reserve it
            // before handing the rest of the budget to the optimizer.
            let header_tokens = self.tokenizer().count(&header);
            let allowance = remaining.map(|left| left.saturating_sub(header_tokens));
            let optimized = self.optimize(context, allowance.map(TokenBudget::new))?;

            if !optimized.content.trim().is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&header);
                content.push_str(&optimized.content);
            }

            original_tokens = original_tokens.saturating_add(optimized.result.original_tokens);
            savings.merge(&optimized.result.savings_by_stage);
            remaining = remaining.map(|left| {
                left.saturating_sub(header_tokens)
                    .saturating_sub(optimized.result.optimized_tokens)
            });

            sections.push(CompiledSection {
                source,
                reference: optimized.reference(),
                result: optimized.result,
            });
        }

        // The assembled document carries headers the individual sections did
        // not, so it is measured as a whole rather than summed.
        let optimized_tokens = self.tokenizer().count(&content);

        Ok(Compilation {
            original_tokens,
            optimized_tokens,
            reduction_ratio: reduction_ratio(original_tokens, optimized_tokens),
            compression_ratio: compression_ratio(original_tokens, optimized_tokens),
            savings_by_stage: savings,
            budget: budget.map(TokenBudget::total),
            content,
            sections,
        })
    }
}

/// Header introducing a section, carrying its retrievable reference.
fn section_header(source: &str, reference: &str) -> String {
    format!("=== {source} ({reference}) ===\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineOptions;
    use ctxc_core::{ContentType, ContextSource, HeuristicTokenizer};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn engine() -> Engine {
        Engine::new(
            Arc::new(HeuristicTokenizer::new()),
            EngineOptions::default(),
        )
    }

    fn file_context(name: &str, content: &str) -> Context {
        Context::new(
            ContextSource::File {
                path: PathBuf::from(name),
            },
            ContentType::PlainText,
            content,
        )
    }

    #[test]
    fn sections_are_labelled_and_referenced() {
        let contexts = vec![
            file_context("a.txt", "first file\n"),
            file_context("b.txt", "second file\n"),
        ];
        let compiled = engine().compile(&contexts, None).unwrap();

        assert_eq!(compiled.sections.len(), 2);
        assert!(compiled.content.contains("=== a.txt (ctxc://context/"));
        assert!(compiled.content.contains("first file"));
        assert!(compiled.content.contains("second file"));
        assert!(compiled.sections[1]
            .reference
            .starts_with("ctxc://context/"));
    }

    #[test]
    fn totals_cover_every_input() {
        let contexts = vec![
            file_context("a.txt", "duplicate\n\nduplicate\n"),
            file_context("b.txt", "unique content here\n"),
        ];
        let compiled = engine().compile(&contexts, None).unwrap();

        let summed: u32 = compiled
            .sections
            .iter()
            .map(|section| section.result.original_tokens)
            .sum();
        assert_eq!(compiled.original_tokens, summed);
        assert!(compiled.savings_by_stage.deduplication > 0);
    }

    #[test]
    fn the_budget_is_spent_in_input_order() {
        let long = (0..40)
            .map(|index| format!("paragraph {index} with several words of content"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let contexts = vec![
            file_context("first.txt", &long),
            file_context("second.txt", &long),
        ];

        // Section headers cost roughly 25 tokens each, so the budget has to be
        // large enough for a header plus some content to be meaningful.
        let compiled = engine()
            .compile(&contexts, Some(TokenBudget::new(120)))
            .unwrap();

        assert!(
            compiled.optimized_tokens <= 120,
            "the assembled document, headers included, must fit the budget; got {}",
            compiled.optimized_tokens
        );
        assert!(compiled.sections[0].result.optimized_tokens > 0);
        assert_eq!(
            compiled.sections[1].result.optimized_tokens, 0,
            "a spent budget leaves nothing for later inputs"
        );
        assert!(compiled.content.contains("first.txt"));
        assert!(
            !compiled.content.contains("second.txt"),
            "empty sections are omitted rather than left as headers"
        );
    }

    #[test]
    fn compiling_nothing_is_an_error() {
        let error = engine().compile(&[], None).unwrap_err();
        assert!(matches!(error, EngineError::NoInput));
        assert!(error.hint().is_some());
    }

    #[test]
    fn a_single_input_still_compiles() {
        let compiled = engine()
            .compile(&[file_context("only.txt", "content\n")], None)
            .unwrap();

        assert_eq!(compiled.sections.len(), 1);
        assert_eq!(compiled.budget, Some(32_000));
    }
}
