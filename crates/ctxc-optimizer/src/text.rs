//! The text optimizer.
//!
//! Three deterministic stages, each measured separately so the result explains
//! itself:
//!
//! ```text
//! fragments -> filtering -> deduplication -> selection -> optimized context
//! ```
//!
//! *Filtering* removes what carries no information: trailing whitespace, blank
//! runs, empty fragments. *Deduplication* drops fragments that repeat content
//! already present. *Selection* drops whole fragments to fit a token budget,
//! and is the only stage that can lose information — which is why the original
//! stays retrievable through the context id.
//!
//! Two policies exist because the safe amount of rewriting depends on the
//! material: prose tolerates de-duplication, source code does not, since two
//! identical blocks in a file are usually both required.

use std::sync::Arc;

use ctxc_context::fragment;
use ctxc_core::optimization::{OptimizedContext, Stage};
use ctxc_core::{ContentType, Context, TokenBudget, Tokenizer};

use ctxc_semantic::Embedder;

use crate::error::Result;
use crate::stages::{deduplicate, deduplicate_similar, trim_and_drop_blanks, StageRunner};
use crate::ContextOptimizer;

/// Similarity at which two fragments count as saying the same thing.
///
/// Deliberately high. Wrongly dropping content is worse than keeping a repeat,
/// and the original stays retrievable either way.
const DEFAULT_REDUNDANCY_THRESHOLD: f32 = 0.92;

/// How aggressive the text optimizer is allowed to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPolicy {
    /// Remove fragments whose content already appeared.
    pub deduplicate: bool,
}

impl TextPolicy {
    /// Prose, documentation, logs and terminal output.
    pub const STANDARD: TextPolicy = TextPolicy { deduplicate: true };

    /// Source code and structured data, where repetition is usually load
    /// bearing. Whitespace filtering and budget selection still apply.
    pub const CONSERVATIVE: TextPolicy = TextPolicy { deduplicate: false };
}

/// Line- and block-oriented optimizer for textual content.
pub struct TextOptimizer {
    tokenizer: Arc<dyn Tokenizer>,
    policy: TextPolicy,
    /// Embedder used to catch repeats that exact matching misses. `None` — the
    /// default — leaves deduplication exactly as it was.
    embedder: Option<Arc<dyn Embedder>>,
    /// Similarity at which two fragments count as saying the same thing.
    redundancy_threshold: f32,
    name: &'static str,
    supported: &'static [ContentType],
}

impl TextOptimizer {
    /// Optimizer for prose-like material.
    pub fn standard(tokenizer: Arc<dyn Tokenizer>) -> Self {
        TextOptimizer {
            tokenizer,
            policy: TextPolicy::STANDARD,
            embedder: None,
            redundancy_threshold: DEFAULT_REDUNDANCY_THRESHOLD,
            name: "text",
            supported: &[
                ContentType::PlainText,
                ContentType::Markdown,
                ContentType::Log,
                ContentType::Terminal,
                ContentType::Unknown,
            ],
        }
    }

    /// Optimizer for code and structured data.
    pub fn conservative(tokenizer: Arc<dyn Tokenizer>) -> Self {
        TextOptimizer {
            tokenizer,
            policy: TextPolicy::CONSERVATIVE,
            embedder: None,
            redundancy_threshold: DEFAULT_REDUNDANCY_THRESHOLD,
            name: "text-conservative",
            supported: &[ContentType::Code, ContentType::Json],
        }
    }

    /// Also collapse fragments that say the same thing in different words.
    ///
    /// Has no effect under [`TextPolicy::CONSERVATIVE`]: source code that
    /// repeats itself is usually required to, and paraphrase is not a concept
    /// that applies to it.
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>, threshold: f32) -> Self {
        self.embedder = Some(embedder);
        self.redundancy_threshold = threshold;
        self
    }

    pub fn policy(&self) -> TextPolicy {
        self.policy
    }
}

impl ContextOptimizer for TextOptimizer {
    fn name(&self) -> &'static str {
        self.name
    }

    fn supports(&self, context: &Context) -> bool {
        self.supported.contains(&context.metadata.content_type)
    }

    fn optimize(&self, context: &Context, budget: Option<TokenBudget>) -> Result<OptimizedContext> {
        let strategy = fragment::strategy_for(context.metadata.content_type);
        let mut runner = StageRunner::new(&*self.tokenizer, context, strategy);

        runner.apply(Stage::Filtering, trim_and_drop_blanks);

        if self.policy.deduplicate {
            runner.apply(Stage::Deduplication, |fragments| {
                deduplicate(fragments, fingerprint, None)
            });

            // A second pass, only when an embedder is configured, for repeats
            // that are worded differently — the same paragraph rewritten, the
            // same error described twice. Exact matching cannot see those.
            //
            // It runs after exact deduplication rather than instead of it:
            // exact matching is cheaper and certain, and this only has to
            // consider what survived.
            if let Some(embedder) = &self.embedder {
                let threshold = self.redundancy_threshold;
                runner.apply(Stage::Deduplication, |fragments| {
                    deduplicate_similar(
                        fragments,
                        embedder.as_ref(),
                        threshold,
                        Some(annotate_repeats),
                    )
                });
            }
        }

        runner.select(budget);
        Ok(runner.finish(self.name, context))
    }
}

/// Marker on a fragment that stood in for several others that said the same
/// thing. Without it, content would vanish with no sign it was ever there.
fn annotate_repeats(content: &str, repeats: usize) -> String {
    format!(
        "{content}\n\n(and {} more saying the same thing)",
        repeats - 1
    )
}

/// Identity used for de-duplication: content with runs of whitespace collapsed,
/// so that the same line indented differently still counts as a repeat.
fn fingerprint(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_whitespace = false;
    for ch in content.chars() {
        if ch.is_whitespace() {
            in_whitespace = true;
            continue;
        }
        if in_whitespace && !out.is_empty() {
            out.push(' ');
        }
        in_whitespace = false;
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContextSource, HeuristicTokenizer};

    fn tokenizer() -> Arc<dyn Tokenizer> {
        Arc::new(HeuristicTokenizer::new())
    }

    fn context(content_type: ContentType, content: &str) -> Context {
        Context::new(ContextSource::Stdin, content_type, content)
    }

    #[test]
    fn supported_types_are_disjoint_between_policies() {
        let standard = TextOptimizer::standard(tokenizer());
        let conservative = TextOptimizer::conservative(tokenizer());

        assert!(standard.supports(&context(ContentType::PlainText, "x")));
        assert!(!standard.supports(&context(ContentType::Code, "x")));
        assert!(conservative.supports(&context(ContentType::Code, "x")));
        assert!(!conservative.supports(&context(ContentType::PlainText, "x")));
    }

    #[test]
    fn filtering_removes_whitespace_noise() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let input = "first line   \n\n\n\n\nsecond line\t\n";
        let optimized = optimizer
            .optimize(&context(ContentType::PlainText, input), None)
            .unwrap();

        assert_eq!(optimized.content, "first line\n\nsecond line\n");
        assert!(optimized.result.savings_by_stage.filtering > 0);
        assert_eq!(optimized.result.savings_by_stage.deduplication, 0);
    }

    #[test]
    fn duplicate_fragments_are_removed_once() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let input = "repeated paragraph\n\nunique paragraph\n\nrepeated paragraph\n";
        let optimized = optimizer
            .optimize(&context(ContentType::PlainText, input), None)
            .unwrap();

        assert_eq!(
            optimized.content,
            "repeated paragraph\n\nunique paragraph\n"
        );
        assert!(optimized.result.savings_by_stage.deduplication > 0);
        assert_eq!(optimized.result.removed_fragments, 1);
        assert_eq!(optimized.result.preserved_fragments, 2);
    }

    #[test]
    fn duplicates_are_matched_across_indentation() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let input = "connection refused\n    connection    refused\n";
        let optimized = optimizer
            .optimize(&context(ContentType::Log, input), None)
            .unwrap();

        assert_eq!(optimized.content, "connection refused\n");
    }

    #[test]
    fn code_is_never_deduplicated() {
        let optimizer = TextOptimizer::conservative(tokenizer());
        let input =
            "fn a() {\n    log();\n}\n\nfn b() {\n    log();\n}\n\nfn a() {\n    log();\n}\n";
        let optimized = optimizer
            .optimize(&context(ContentType::Code, input), None)
            .unwrap();

        assert_eq!(
            optimized.content.matches("fn a()").count(),
            2,
            "identical code blocks must both survive"
        );
        assert_eq!(optimized.result.savings_by_stage.deduplication, 0);
    }

    #[test]
    fn selection_enforces_the_budget() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let input = (0..50)
            .map(|index| format!("paragraph number {index} with some words in it"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let source = context(ContentType::PlainText, &input);

        let budget = TokenBudget::new(40);
        let optimized = optimizer.optimize(&source, Some(budget)).unwrap();

        assert!(
            optimized.result.optimized_tokens <= 40,
            "produced {} tokens",
            optimized.result.optimized_tokens
        );
        assert!(optimized.result.savings_by_stage.selection > 0);
        assert!(optimized.content.starts_with("paragraph number 0"));
    }

    #[test]
    fn a_generous_budget_removes_nothing() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let source = context(ContentType::PlainText, "one\n\ntwo\n\nthree\n");
        let optimized = optimizer
            .optimize(&source, Some(TokenBudget::new(10_000)))
            .unwrap();

        assert_eq!(optimized.content, "one\n\ntwo\n\nthree\n");
        assert_eq!(optimized.result.savings_by_stage.selection, 0);
    }

    #[test]
    fn stage_savings_always_sum_to_the_total() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let input = "alpha   \n\nalpha\n\nbeta\n\ngamma\n\ndelta\n\nepsilon\n";
        let optimized = optimizer
            .optimize(
                &context(ContentType::PlainText, input),
                Some(TokenBudget::new(6)),
            )
            .unwrap();

        let result = &optimized.result;
        assert_eq!(
            result.savings_by_stage.total(),
            result.tokens_saved(),
            "attribution must account for every saved token"
        );
    }

    #[test]
    fn empty_input_is_handled() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let optimized = optimizer
            .optimize(&context(ContentType::PlainText, "   \n\n"), None)
            .unwrap();

        assert_eq!(optimized.content, "");
        assert_eq!(optimized.result.preserved_fragments, 0);
        assert_eq!(optimized.result.optimized_tokens, 0);
        assert_eq!(
            optimized.result.savings_by_stage.total(),
            optimized.result.tokens_saved()
        );
    }

    #[test]
    fn the_original_stays_addressable() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let source = context(ContentType::PlainText, "something worth keeping\n");
        let optimized = optimizer.optimize(&source, None).unwrap();

        assert_eq!(optimized.source_id, source.id);
        assert!(optimized.reference().starts_with("ctxc://context/"));
    }

    #[test]
    fn results_name_their_tokenizer() {
        let optimizer = TextOptimizer::standard(tokenizer());
        let optimized = optimizer
            .optimize(&context(ContentType::PlainText, "text\n"), None)
            .unwrap();

        assert_eq!(optimized.result.tokenizer, "heuristic");
        assert!(optimized.result.estimated);
        assert_eq!(optimized.result.optimizer, "text");
    }
}
