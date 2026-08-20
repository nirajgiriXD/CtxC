//! The generic fallback.
//!
//! Whatever the router is handed, something must handle it. This optimizer
//! returns the content untouched and reports honest zeroes, so an unknown or
//! unsupported input costs nothing and loses nothing. A generic fallback must
//! always exist, and it must never be the one doing clever things.

use std::sync::Arc;

use ctxc_context::fragment;
use ctxc_core::optimization::{Counts, OptimizationResult, OptimizedContext, SavingsByStage};
use ctxc_core::{Context, TokenBudget, Tokenizer};

use crate::error::Result;
use crate::ContextOptimizer;

/// Returns context unchanged.
pub struct PassthroughOptimizer {
    tokenizer: Arc<dyn Tokenizer>,
}

impl PassthroughOptimizer {
    pub fn new(tokenizer: Arc<dyn Tokenizer>) -> Self {
        PassthroughOptimizer { tokenizer }
    }
}

impl ContextOptimizer for PassthroughOptimizer {
    fn name(&self) -> &'static str {
        "passthrough"
    }

    /// Accepts everything, which is what makes it usable as the last entry in
    /// the router's registry.
    fn supports(&self, _context: &Context) -> bool {
        true
    }

    fn optimize(
        &self,
        context: &Context,
        _budget: Option<TokenBudget>,
    ) -> Result<OptimizedContext> {
        let strategy = fragment::strategy_for(context.metadata.content_type);
        let fragments = fragment::split(&context.content, strategy).len() as u32;
        let tokens = self.tokenizer.count(&context.content);

        Ok(OptimizedContext {
            source_id: context.id.clone(),
            content: context.content.clone(),
            result: OptimizationResult::new(
                self.name(),
                &*self.tokenizer,
                Counts {
                    original_tokens: tokens,
                    optimized_tokens: tokens,
                    preserved_fragments: fragments,
                    removed_fragments: 0,
                },
                SavingsByStage::default(),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContentType, ContextSource, HeuristicTokenizer};

    fn optimizer() -> PassthroughOptimizer {
        PassthroughOptimizer::new(Arc::new(HeuristicTokenizer::new()))
    }

    #[test]
    fn content_is_returned_untouched() {
        let source = Context::new(
            ContextSource::Stdin,
            ContentType::Unknown,
            "  ragged   content  \n\n\n",
        );
        let optimized = optimizer().optimize(&source, None).unwrap();

        assert_eq!(optimized.content, source.content);
        assert_eq!(optimized.result.tokens_saved(), 0);
        assert_eq!(optimized.result.savings_by_stage.total(), 0);
        assert_eq!(optimized.result.reduction_ratio, 0.0);
    }

    #[test]
    fn a_budget_does_not_make_it_cut() {
        let source = Context::new(ContextSource::Stdin, ContentType::Unknown, "a\n\nb\n\nc\n");
        let optimized = optimizer()
            .optimize(&source, Some(TokenBudget::new(1)))
            .unwrap();

        assert_eq!(optimized.content, source.content);
    }

    #[test]
    fn it_supports_everything() {
        let source = Context::new(ContextSource::Stdin, ContentType::Binary, "\u{fffd}");
        assert!(optimizer().supports(&source));
    }
}
