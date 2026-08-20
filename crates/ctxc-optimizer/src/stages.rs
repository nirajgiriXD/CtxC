//! Shared stage machinery.
//!
//! Every optimizer runs the same shape of pipeline — transform, measure,
//! attribute — and the invariant that *every saved token is attributed to a
//! stage* is worth having in exactly one place. Optimizers describe what each
//! stage does; the runner does the counting.

use ctxc_context::fragment::{self, Fragment, SplitStrategy};
use ctxc_core::optimization::{
    Counts, OptimizationResult, OptimizedContext, SavingsByStage, Stage,
};
use ctxc_core::{Context, TokenBudget, Tokenizer};
use ctxc_semantic::{collapse_redundant, Embedder, Verdict};

/// Runs an optimizer's stages over a context, measuring after each one.
pub(crate) struct StageRunner<'a> {
    tokenizer: &'a dyn Tokenizer,
    strategy: SplitStrategy,
    fragments: Vec<Fragment>,
    savings: SavingsByStage,
    original_tokens: u32,
    original_fragments: u32,
    /// Tokens after the most recently applied stage.
    current_tokens: u32,
}

impl<'a> StageRunner<'a> {
    /// Start a pipeline over `context`, split with `strategy`.
    pub fn new(tokenizer: &'a dyn Tokenizer, context: &Context, strategy: SplitStrategy) -> Self {
        let fragments = fragment::split(&context.content, strategy);
        let original_tokens = tokenizer.count(&context.content);

        StageRunner {
            tokenizer,
            strategy,
            original_fragments: fragments.len() as u32,
            fragments,
            savings: SavingsByStage::default(),
            original_tokens,
            current_tokens: original_tokens,
        }
    }

    /// Apply a stage that rewrites the fragment list.
    pub fn apply(&mut self, stage: Stage, transform: impl FnOnce(&mut Vec<Fragment>)) {
        transform(&mut self.fragments);
        self.record(stage);
    }

    /// Apply a stage that rewrites the content as a whole, for optimizers whose
    /// material is structural rather than line oriented.
    pub fn apply_text(&mut self, stage: Stage, transform: impl FnOnce(&str) -> String) {
        let rewritten = transform(&self.render());
        self.fragments = fragment::split(&rewritten, self.strategy);
        self.record(stage);
    }

    /// Drop fragments from the end until the content fits `budget`.
    ///
    /// Position is the only signal available before ranking exists, and the top
    /// of a document is where its subject, headings and imports live.
    pub fn select(&mut self, budget: Option<TokenBudget>) {
        let Some(budget) = budget else {
            return;
        };
        if self.current_tokens <= budget.total() {
            return;
        }

        // Estimate a cut point from per-fragment counts, then verify with a
        // real measurement, which also accounts for the separators.
        let mut running = 0u32;
        let mut keep = 0usize;
        for item in &self.fragments {
            let tokens = self.tokenizer.count(&item.content);
            if running.saturating_add(tokens) > budget.total() {
                break;
            }
            running = running.saturating_add(tokens);
            keep += 1;
        }
        self.fragments.truncate(keep);

        while !self.fragments.is_empty() && self.measure() > budget.total() {
            self.fragments.pop();
        }
        self.record(Stage::Selection);
    }

    /// Finish the pipeline and produce the optimized context.
    pub fn finish(self, optimizer: &'static str, context: &Context) -> OptimizedContext {
        let preserved = self.fragments.len() as u32;
        let content = self.render();

        OptimizedContext {
            source_id: context.id.clone(),
            content,
            result: OptimizationResult::new(
                optimizer,
                self.tokenizer,
                Counts {
                    original_tokens: self.original_tokens,
                    optimized_tokens: self.current_tokens,
                    preserved_fragments: preserved,
                    removed_fragments: self.original_fragments.saturating_sub(preserved),
                },
                self.savings,
            ),
        }
    }

    fn render(&self) -> String {
        fragment::join(&self.fragments, self.strategy)
    }

    fn measure(&self) -> u32 {
        self.tokenizer.count(&self.render())
    }

    /// Attribute whatever the last transform changed to `stage`.
    ///
    /// The delta is signed, so a stage that added tokens is recorded as such
    /// and the breakdown still sums to the overall saving.
    fn record(&mut self, stage: Stage) {
        let now = self.measure();
        let delta = (self.current_tokens as i64 - now as i64)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        self.savings.record(stage, delta);
        self.current_tokens = now;
    }
}

/// Trim trailing whitespace from every line and drop fragments left empty.
///
/// The filtering stage of every line-oriented optimizer starts here.
pub(crate) fn trim_and_drop_blanks(fragments: &mut Vec<Fragment>) {
    for item in fragments.iter_mut() {
        item.content = item.trimmed();
    }
    fragments.retain(|item| !item.is_blank());
}

/// Drop fragments whose `key` was already seen, keeping the first occurrence.
///
/// `annotate` receives the kept fragment and how many times its key occurred,
/// so an optimizer can record "this happened 400 times" rather than silently
/// discarding that fact.
pub(crate) fn deduplicate(
    fragments: &mut Vec<Fragment>,
    key: impl Fn(&str) -> String,
    annotate: Option<fn(&str, usize) -> String>,
) {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in fragments.iter() {
        *counts.entry(key(&item.content)).or_insert(0) += 1;
    }

    let mut seen = std::collections::HashSet::new();
    fragments.retain(|item| seen.insert(key(&item.content)));

    if let Some(annotate) = annotate {
        for item in fragments.iter_mut() {
            let repeats = counts.get(&key(&item.content)).copied().unwrap_or(1);
            if repeats > 1 {
                item.content = annotate(&item.content, repeats);
            }
        }
    }
}

/// Drop fragments that repeat something already kept, allowing for rewording.
///
/// Exact deduplication compares strings, so a hundred log lines that differ
/// only in their timestamp are a hundred distinct fragments. This compares
/// embeddings, which is what makes it catch them.
///
/// It is still deduplication, and its savings are attributed to
/// [`Stage::Deduplication`] rather than a stage of its own — the difference is
/// how sameness is decided, not what the pipeline is doing.
///
/// `annotate` marks a survivor with how many fragments collapsed into it, so
/// the output says "repeated 100 times" rather than silently losing 99 lines.
pub(crate) fn deduplicate_similar(
    fragments: &mut Vec<Fragment>,
    embedder: &dyn Embedder,
    threshold: f32,
    annotate: Option<fn(&str, usize) -> String>,
) {
    if fragments.len() < 2 {
        return;
    }

    let embeddings: Vec<_> = fragments
        .iter()
        .map(|fragment| embedder.embed(&fragment.trimmed()))
        .collect();
    let verdicts = collapse_redundant(&embeddings, threshold);

    // How many fragments each survivor stands for, so the annotation is the
    // real count rather than "at least two".
    let mut repeats = vec![1usize; fragments.len()];
    for verdict in &verdicts {
        if let Verdict::Redundant { duplicate_of } = verdict {
            repeats[*duplicate_of] += 1;
        }
    }

    let mut index = 0;
    fragments.retain(|_| {
        let keep = verdicts[index] == Verdict::Keep;
        index += 1;
        keep
    });

    if let Some(annotate) = annotate {
        let kept: Vec<usize> = verdicts
            .iter()
            .enumerate()
            .filter(|(_, verdict)| **verdict == Verdict::Keep)
            .map(|(index, _)| index)
            .collect();

        for (position, fragment) in fragments.iter_mut().enumerate() {
            let count = repeats[kept[position]];
            if count > 1 {
                fragment.content = annotate(&fragment.content, count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContentType, ContextSource, HeuristicTokenizer};
    use ctxc_semantic::HashedEmbedder;

    fn context(content: &str) -> Context {
        Context::new(ContextSource::Stdin, ContentType::PlainText, content)
    }

    #[test]
    fn savings_are_attributed_to_the_stage_that_caused_them() {
        let tokenizer = HeuristicTokenizer::new();
        let source = context("alpha   \n\nalpha\n\nbeta\n");
        let mut runner = StageRunner::new(&tokenizer, &source, SplitStrategy::Blocks);

        runner.apply(Stage::Filtering, trim_and_drop_blanks);
        runner.apply(Stage::Deduplication, |fragments| {
            deduplicate(fragments, |text| text.to_string(), None)
        });

        let optimized = runner.finish("test", &source);
        assert_eq!(optimized.content, "alpha\n\nbeta\n");
        assert!(optimized.result.savings_by_stage.deduplication > 0);
        assert_eq!(
            optimized.result.savings_by_stage.total(),
            optimized.result.tokens_saved()
        );
    }

    #[test]
    fn rewording_does_not_hide_a_repeat_from_deduplication() {
        let mut fragments = fragment::split(
            "ERROR 12:00:01 connection to database failed after 3 retries\n\n\
             ERROR 12:00:04 connection to database failed after 3 retries\n\n\
             ERROR 12:00:09 connection to database failed after 3 retries\n\n\
             INFO  12:00:11 listening on port 8080\n",
            SplitStrategy::Blocks,
        );
        assert_eq!(fragments.len(), 4, "exact deduplication keeps all four");

        deduplicate_similar(&mut fragments, &HashedEmbedder::default(), 0.9, None);

        assert_eq!(fragments.len(), 2, "{fragments:?}");
        assert!(fragments[1].content.contains("listening on port"));
    }

    #[test]
    fn a_collapsed_run_says_how_many_it_stood_for() {
        let mut fragments = fragment::split(
            "same line here\n\nsame line here\n\nsame line here\n",
            SplitStrategy::Blocks,
        );

        deduplicate_similar(
            &mut fragments,
            &HashedEmbedder::default(),
            0.9,
            Some(|content, repeats| format!("{content}  (repeated {repeats} times)")),
        );

        assert_eq!(fragments.len(), 1);
        assert!(
            fragments[0].content.contains("repeated 3 times"),
            "losing two lines silently is worse than keeping them: {:?}",
            fragments[0].content
        );
    }

    #[test]
    fn genuinely_different_fragments_survive() {
        let mut fragments = fragment::split(
            "the cat sat on the mat\n\na dog stood on the floor\n\nbirds fly south\n",
            SplitStrategy::Blocks,
        );

        deduplicate_similar(&mut fragments, &HashedEmbedder::default(), 0.92, None);
        assert_eq!(fragments.len(), 3, "{fragments:?}");
    }

    #[test]
    fn nothing_to_compare_is_left_alone() {
        let mut fragments = fragment::split("only one fragment\n", SplitStrategy::Blocks);
        deduplicate_similar(&mut fragments, &HashedEmbedder::default(), 0.5, None);

        assert_eq!(fragments.len(), 1);
    }

    #[test]
    fn text_stages_can_rewrite_everything_at_once() {
        let tokenizer = HeuristicTokenizer::new();
        let source = context("one two three four five six\n");
        let mut runner = StageRunner::new(&tokenizer, &source, SplitStrategy::Blocks);

        runner.apply_text(Stage::Compression, |text| {
            text.split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        });

        let optimized = runner.finish("test", &source);
        assert_eq!(optimized.content, "one two\n");
        assert!(optimized.result.savings_by_stage.compression > 0);
    }

    #[test]
    fn selection_is_a_no_op_without_a_budget() {
        let tokenizer = HeuristicTokenizer::new();
        let source = context("one\n\ntwo\n\nthree\n");
        let mut runner = StageRunner::new(&tokenizer, &source, SplitStrategy::Blocks);

        runner.select(None);
        let optimized = runner.finish("test", &source);

        assert_eq!(optimized.content, "one\n\ntwo\n\nthree\n");
        assert_eq!(optimized.result.savings_by_stage.selection, 0);
    }

    #[test]
    fn duplicates_can_be_annotated_with_their_count() {
        let mut fragments = fragment::split("boom\nboom\nboom\nfine\n", SplitStrategy::Lines);
        deduplicate(
            &mut fragments,
            |text| text.to_string(),
            Some(|content, repeats| format!("{content}  (repeated {repeats} times)")),
        );

        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0].content, "boom  (repeated 3 times)");
        assert_eq!(fragments[1].content, "fine", "unique lines are left alone");
    }

    #[test]
    fn deduplication_keeps_the_first_occurrence_and_the_order() {
        let mut fragments = fragment::split("b\na\nb\nc\na\n", SplitStrategy::Lines);
        deduplicate(&mut fragments, |text| text.to_string(), None);

        let kept: Vec<&str> = fragments.iter().map(|f| f.content.as_str()).collect();
        assert_eq!(kept, vec!["b", "a", "c"]);
    }
}
