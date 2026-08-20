//! The content router.
//!
//! Picks the optimizer for a context. Routing is a lookup, never a model call:
//! MIME-style content type first, and whichever registered optimizer claims it
//! first wins. The registry ends with a fallback that accepts everything, so
//! routing can never fail.

use std::sync::Arc;

use ctxc_core::Context;
use ctxc_optimizer::ContextOptimizer;

/// Ordered registry of optimizers.
pub struct Router {
    optimizers: Vec<Arc<dyn ContextOptimizer>>,
    fallback: Arc<dyn ContextOptimizer>,
}

impl Router {
    /// Build a router. `fallback` must accept every context.
    pub fn new(
        optimizers: Vec<Arc<dyn ContextOptimizer>>,
        fallback: Arc<dyn ContextOptimizer>,
    ) -> Self {
        Router {
            optimizers,
            fallback,
        }
    }

    /// The optimizer that will handle `context`.
    pub fn route(&self, context: &Context) -> Arc<dyn ContextOptimizer> {
        self.optimizers
            .iter()
            .find(|optimizer| optimizer.supports(context))
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.fallback))
    }

    /// Names of the registered optimizers, fallback last.
    pub fn optimizer_names(&self) -> Vec<&'static str> {
        self.optimizers
            .iter()
            .map(|optimizer| optimizer.name())
            .chain(std::iter::once(self.fallback.name()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContentType, ContextSource, HeuristicTokenizer};
    use ctxc_optimizer::{PassthroughOptimizer, TextOptimizer};

    fn router() -> Router {
        let tokenizer = Arc::new(HeuristicTokenizer::new());
        Router::new(
            vec![
                Arc::new(TextOptimizer::standard(tokenizer.clone())),
                Arc::new(TextOptimizer::conservative(tokenizer.clone())),
            ],
            Arc::new(PassthroughOptimizer::new(tokenizer)),
        )
    }

    fn context(content_type: ContentType) -> Context {
        Context::new(ContextSource::Stdin, content_type, "content")
    }

    #[test]
    fn text_content_reaches_the_text_optimizer() {
        assert_eq!(
            router().route(&context(ContentType::PlainText)).name(),
            "text"
        );
        assert_eq!(
            router().route(&context(ContentType::Markdown)).name(),
            "text"
        );
        assert_eq!(router().route(&context(ContentType::Log)).name(), "text");
    }

    #[test]
    fn code_reaches_the_conservative_optimizer() {
        assert_eq!(
            router().route(&context(ContentType::Code)).name(),
            "text-conservative"
        );
        assert_eq!(
            router().route(&context(ContentType::Json)).name(),
            "text-conservative"
        );
    }

    #[test]
    fn unhandled_content_reaches_the_fallback() {
        assert_eq!(
            router().route(&context(ContentType::Binary)).name(),
            "passthrough"
        );
    }

    #[test]
    fn the_registry_is_introspectable() {
        assert_eq!(
            router().optimizer_names(),
            vec!["text", "text-conservative", "passthrough"]
        );
    }
}
