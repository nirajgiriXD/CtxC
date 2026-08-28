//! The orchestration layer.
//!
//! The engine coordinates; it does not optimize. It resolves the budget, asks
//! the router which optimizer applies, runs it, and hands back a structured
//! result. Everything it composes — router, optimizers, tokenizer — is injected,
//! so the pipeline can be exercised without touching the filesystem, the store,
//! or the CLI.

use std::sync::Arc;

use ctxc_core::{Config, Context, OptimizedContext, TokenBudget, Tokenizer};
use ctxc_optimizer::{
    ContextOptimizer, JsonOptimizer, LogOptimizer, PassthroughOptimizer, TextOptimizer,
    ToolOutputOptimizer,
};

use crate::error::Result;
use crate::router::Router;

/// Engine behaviour that comes from configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineOptions {
    /// When false, every context is passed through untouched. This is the
    /// `optimization.enabled = false` escape hatch, and it must remain a real
    /// no-op rather than a weaker optimizer.
    pub optimization_enabled: bool,
    /// Budget applied when a caller does not name one.
    pub default_budget: TokenBudget,
    /// Whether the text optimizer also collapses fragments that say the same
    /// thing in different words.
    pub semantic_deduplication: bool,
    /// Similarity at which two fragments count as saying the same thing.
    pub redundancy_threshold: f32,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            optimization_enabled: true,
            default_budget: TokenBudget::new(32_000),
            semantic_deduplication: false,
            redundancy_threshold: 0.92,
        }
    }
}

impl EngineOptions {
    /// Read the options out of a loaded configuration.
    pub fn from_config(config: &Config) -> Self {
        EngineOptions {
            optimization_enabled: config.optimization.enabled,
            default_budget: config.default_budget(),
            // Embeddings being on is what makes this available; it is the same
            // switch, because an optimizer that embedded while the index did
            // not would be paying twice for one capability.
            semantic_deduplication: config.semantic.enabled,
            redundancy_threshold: config.semantic.redundancy_threshold as f32,
        }
    }
}

/// Runs optimization pipelines.
pub struct Engine {
    router: Router,
    tokenizer: Arc<dyn Tokenizer>,
    passthrough: Arc<dyn ContextOptimizer>,
    options: EngineOptions,
}

impl Engine {
    /// Build an engine with the standard optimizer registry.
    ///
    /// Order is the routing policy: a command's own profile knows more than a
    /// content type does, a parseable document is better served by the
    /// optimizer that can parse it than by a line filter, and the generic
    /// command profile catches whatever is left before content-type routing
    /// takes over.
    pub fn new(tokenizer: Arc<dyn Tokenizer>, options: EngineOptions) -> Self {
        let passthrough: Arc<dyn ContextOptimizer> =
            Arc::new(PassthroughOptimizer::new(Arc::clone(&tokenizer)));

        let mut optimizers: Vec<Arc<dyn ContextOptimizer>> = Vec::new();
        let mut tools = ToolOutputOptimizer::registry(Arc::clone(&tokenizer));
        let generic_tool = tools.pop();

        for tool in tools {
            optimizers.push(Arc::new(tool));
        }
        optimizers.push(Arc::new(JsonOptimizer::new(Arc::clone(&tokenizer))));
        optimizers.push(Arc::new(LogOptimizer::new(Arc::clone(&tokenizer))));
        if let Some(generic_tool) = generic_tool {
            optimizers.push(Arc::new(generic_tool));
        }
        let mut text = TextOptimizer::standard(Arc::clone(&tokenizer));
        if options.semantic_deduplication {
            // A failure to build the embedder costs the extra pass, not the
            // optimizer: text still gets filtered, deduplicated exactly, and
            // budgeted.
            match ctxc_semantic::embedder(&ctxc_semantic::SemanticOptions {
                enabled: true,
                ..Default::default()
            }) {
                Ok(embedder) => {
                    text = text.with_embedder(embedder, options.redundancy_threshold);
                }
                Err(err) => {
                    tracing::warn!(error = %err, "semantic deduplication is unavailable");
                }
            }
        }
        optimizers.push(Arc::new(text));
        optimizers.push(Arc::new(TextOptimizer::conservative(Arc::clone(
            &tokenizer,
        ))));

        let router = Router::new(optimizers, Arc::clone(&passthrough));

        Engine {
            router,
            tokenizer,
            passthrough,
            options,
        }
    }

    /// Build an engine from configuration, with the tokenizer it selects.
    ///
    /// That is the estimator unless `budget.tokenizer` names another one this
    /// build carries. Everything downstream reports the tokenizer by name and
    /// whether it estimates, so a number can always be traced to how it was
    /// counted.
    pub fn from_config(config: &Config) -> Self {
        Engine::new(config.tokenizer(), EngineOptions::from_config(config))
    }

    /// Replace the router, for tests and for callers assembling their own
    /// registry.
    pub fn with_router(mut self, router: Router) -> Self {
        self.router = router;
        self
    }

    pub fn options(&self) -> EngineOptions {
        self.options
    }

    pub fn tokenizer(&self) -> &Arc<dyn Tokenizer> {
        &self.tokenizer
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    /// Optimize one context.
    ///
    /// `budget` overrides the configured default; pass `None` to use it.
    pub fn optimize(
        &self,
        context: &Context,
        budget: Option<TokenBudget>,
    ) -> Result<OptimizedContext> {
        let optimizer = self.optimizer_for(context);
        let budget = budget.or(Some(self.options.default_budget));

        tracing::debug!(
            optimizer = optimizer.name(),
            content_type = context.metadata.content_type.as_str(),
            budget = budget.map(TokenBudget::total),
            "optimizing context"
        );

        Ok(optimizer.optimize(context, budget)?)
    }

    /// Optimize without applying any budget, which is what analysis needs: it
    /// reports what optimization *would* remove, not what a budget would cut.
    pub(crate) fn optimize_unbounded(&self, context: &Context) -> Result<OptimizedContext> {
        Ok(self.optimizer_for(context).optimize(context, None)?)
    }

    /// A fingerprint of everything about this engine that decides its output.
    ///
    /// What an optimizer produces depends on more than the bytes it is given:
    /// the tokenizer counting them, whether optimization is on at all, and the
    /// deduplication settings all change the answer. Anything remembering an
    /// optimization has to key on this, or a configuration change silently
    /// keeps returning results from the old one.
    pub fn settings_fingerprint(&self) -> String {
        let described = format!(
            "tokenizer={};enabled={};default_budget={};semantic_dedup={};redundancy={:.6}",
            self.tokenizer.name(),
            self.options.optimization_enabled,
            self.options.default_budget.total(),
            self.options.semantic_deduplication,
            self.options.redundancy_threshold,
        );
        ctxc_core::id::content_hash(described.as_bytes())
    }

    /// The optimizer that will handle `context`, honouring the global switch.
    pub fn optimizer_for(&self, context: &Context) -> Arc<dyn ContextOptimizer> {
        if self.options.optimization_enabled {
            self.router.route(context)
        } else {
            Arc::clone(&self.passthrough)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContentType, ContextSource, HeuristicTokenizer};

    fn engine(options: EngineOptions) -> Engine {
        Engine::new(Arc::new(HeuristicTokenizer::new()), options)
    }

    fn context(content_type: ContentType, content: &str) -> Context {
        Context::new(ContextSource::Stdin, content_type, content)
    }

    #[test]
    fn optimization_can_be_switched_off_entirely() {
        let options = EngineOptions {
            optimization_enabled: false,
            ..EngineOptions::default()
        };
        let source = context(ContentType::PlainText, "same\n\nsame\n\n\n");
        let optimized = engine(options).optimize(&source, None).unwrap();

        assert_eq!(optimized.content, source.content);
        assert_eq!(optimized.result.optimizer, "passthrough");
    }

    #[test]
    fn the_configured_budget_applies_when_none_is_given() {
        let options = EngineOptions {
            optimization_enabled: true,
            default_budget: TokenBudget::new(5),
            ..EngineOptions::default()
        };
        let input = (0..30)
            .map(|index| format!("fragment {index} carrying several words"))
            .collect::<Vec<_>>()
            .join("\n\n");

        let optimized = engine(options)
            .optimize(&context(ContentType::PlainText, &input), None)
            .unwrap();

        assert!(optimized.result.optimized_tokens <= 5);
        assert!(optimized.result.savings_by_stage.selection > 0);
    }

    #[test]
    fn an_explicit_budget_overrides_the_configured_one() {
        let options = EngineOptions {
            optimization_enabled: true,
            default_budget: TokenBudget::new(5),
            ..EngineOptions::default()
        };
        let source = context(ContentType::PlainText, "one\n\ntwo\n\nthree\n\nfour\n");
        let optimized = engine(options)
            .optimize(&source, Some(TokenBudget::new(1_000)))
            .unwrap();

        assert_eq!(optimized.result.savings_by_stage.selection, 0);
    }

    #[test]
    fn options_come_from_configuration() {
        let mut config = Config::default();
        config.optimization.enabled = false;
        config.budget.default = 1_234;

        let options = EngineOptions::from_config(&config);
        assert!(!options.optimization_enabled);
        assert_eq!(options.default_budget.total(), 1_234);
        assert!(
            !options.semantic_deduplication,
            "embeddings being off must leave optimization exactly as it was"
        );
    }

    #[test]
    fn semantic_deduplication_follows_the_embeddings_switch() {
        let mut config = Config::default();
        config.semantic.enabled = true;

        assert!(EngineOptions::from_config(&config).semantic_deduplication);
    }

    #[test]
    fn reworded_repeats_survive_until_semantic_deduplication_is_on() {
        // Three paragraphs saying the same thing in different words. Exact
        // matching sees three distinct fragments.
        let input = "The database connection timed out after three retries.\n\n\
                     The connection to the database timed out following 3 retries.\n\n\
                     Rendering the interface layout is unrelated work.\n";
        let source = context(ContentType::PlainText, input);

        let plain = engine(EngineOptions::default())
            .optimize(&source, None)
            .unwrap();
        assert_eq!(
            plain
                .content
                .lines()
                .filter(|line| line.contains("retries"))
                .count(),
            2,
            "exact deduplication cannot see a reworded repeat: {}",
            plain.content
        );

        let semantic = engine(EngineOptions {
            semantic_deduplication: true,
            // Low enough that reworded prose collapses; the shipped default is
            // stricter, because dropping content wrongly is the worse mistake.
            redundancy_threshold: 0.6,
            ..EngineOptions::default()
        })
        .optimize(&source, None)
        .unwrap();

        assert!(
            semantic.content.contains("Rendering the interface"),
            "unrelated content must survive: {}",
            semantic.content
        );
        assert!(
            semantic.result.savings_by_stage.deduplication
                > plain.result.savings_by_stage.deduplication,
            "the extra pass should be attributed to deduplication"
        );
    }

    #[test]
    fn routing_follows_content_type() {
        let engine = engine(EngineOptions::default());
        assert_eq!(
            engine
                .optimizer_for(&context(ContentType::Code, "fn x() {}"))
                .name(),
            "text-conservative"
        );
        assert_eq!(
            engine
                .optimizer_for(&context(ContentType::Markdown, "# x"))
                .name(),
            "text"
        );
    }
}
