//! The JSON optimizer.
//!
//! Two stages with very different risk profiles, kept apart deliberately:
//!
//! * *compression* re-emits the document without insignificant whitespace.
//!   This is lossless — the value is identical — and on pretty-printed API
//!   responses it is often half the tokens.
//! * *selection* shortens long arrays and long strings, and only ever runs when
//!   a budget demands it. It is lossy, so it never happens by default.
//!
//! Output stays parseable JSON at every step. Truncation replaces what it
//! removes with a marker value rather than cutting the text, because handing a
//! model a JSON document that no longer parses is worse than handing it a
//! larger one.
//!
//! Anything that is not valid JSON is returned untouched: this optimizer would
//! rather do nothing than guess at a structure it could not parse.

use std::sync::Arc;

use ctxc_context::fragment::SplitStrategy;
use ctxc_core::optimization::{OptimizedContext, Stage};
use ctxc_core::{ContentType, Context, TokenBudget, Tokenizer};
use serde_json::Value;

use crate::error::Result;
use crate::stages::StageRunner;
use crate::ContextOptimizer;

/// Array lengths tried, largest first, when shrinking to fit a budget.
const ARRAY_LIMITS: [usize; 6] = [100, 50, 20, 10, 5, 2];

/// String lengths tried, largest first, when shrinking to fit a budget.
const STRING_LIMITS: [usize; 5] = [512, 256, 128, 64, 32];

/// Optimizer for JSON documents and JSON Lines streams.
pub struct JsonOptimizer {
    tokenizer: Arc<dyn Tokenizer>,
}

impl JsonOptimizer {
    pub fn new(tokenizer: Arc<dyn Tokenizer>) -> Self {
        JsonOptimizer { tokenizer }
    }
}

impl ContextOptimizer for JsonOptimizer {
    fn name(&self) -> &'static str {
        "json"
    }

    fn supports(&self, context: &Context) -> bool {
        context.metadata.content_type == ContentType::Json
    }

    fn optimize(&self, context: &Context, budget: Option<TokenBudget>) -> Result<OptimizedContext> {
        let Some(document) = Document::parse(&context.content) else {
            // Not parseable: hand it back exactly as it arrived.
            let runner = StageRunner::new(&*self.tokenizer, context, SplitStrategy::Lines);
            return Ok(runner.finish(self.name(), context));
        };

        let mut runner = StageRunner::new(&*self.tokenizer, context, SplitStrategy::Lines);
        runner.apply_text(Stage::Compression, |_| document.render());

        if let Some(budget) = budget {
            let tokenizer = Arc::clone(&self.tokenizer);
            runner.apply_text(Stage::Selection, |current| {
                if tokenizer.count(current) <= budget.total() {
                    return current.to_string();
                }
                shrink_to_fit(&document, budget, tokenizer.as_ref())
            });
        }

        Ok(runner.finish(self.name(), context))
    }
}

/// One JSON value, or a stream of them (JSON Lines).
enum Document {
    Single(Value),
    Lines(Vec<Value>),
}

impl Document {
    /// Parse `text` as JSON, then as JSON Lines. `None` if it is neither.
    fn parse(text: &str) -> Option<Document> {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            return Some(Document::Single(value));
        }

        let mut values = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            values.push(serde_json::from_str::<Value>(line).ok()?);
        }
        (!values.is_empty()).then_some(Document::Lines(values))
    }

    /// Compact rendering, one value per line for a stream.
    fn render(&self) -> String {
        self.render_limited(usize::MAX, usize::MAX)
    }

    /// Compact rendering with arrays and strings shortened.
    fn render_limited(&self, array_limit: usize, string_limit: usize) -> String {
        match self {
            Document::Single(value) => compact(&truncate(value, array_limit, string_limit)),
            Document::Lines(values) => values
                .iter()
                .map(|value| compact(&truncate(value, array_limit, string_limit)))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// The smallest rendering that fits, or the smallest available if none does.
///
/// Falling back to the most aggressive limits rather than to broken output
/// keeps the promise that what comes out is still JSON.
fn shrink_to_fit(document: &Document, budget: TokenBudget, tokenizer: &dyn Tokenizer) -> String {
    let mut smallest = document.render();

    for (array_limit, string_limit) in ARRAY_LIMITS.iter().zip(pad(&STRING_LIMITS)) {
        let candidate = document.render_limited(*array_limit, string_limit);
        smallest = candidate;
        if tokenizer.count(&smallest) <= budget.total() {
            break;
        }
    }
    smallest
}

/// Pair each array limit with a string limit, reusing the last one once the
/// shorter list runs out.
fn pad(limits: &[usize]) -> Vec<usize> {
    let mut padded = limits.to_vec();
    while padded.len() < ARRAY_LIMITS.len() {
        padded.push(*limits.last().unwrap_or(&32));
    }
    padded
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Rebuild a value with long arrays and long strings shortened, recording what
/// was dropped in place of the dropped content.
fn truncate(value: &Value, array_limit: usize, string_limit: usize) -> Value {
    match value {
        Value::Array(items) => {
            let mut kept: Vec<Value> = items
                .iter()
                .take(array_limit)
                .map(|item| truncate(item, array_limit, string_limit))
                .collect();
            if items.len() > array_limit {
                kept.push(Value::String(format!(
                    "[ctxc: {} more items omitted]",
                    items.len() - array_limit
                )));
            }
            Value::Array(kept)
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), truncate(field, array_limit, string_limit)))
                .collect(),
        ),
        Value::String(text) if text.chars().count() > string_limit => {
            let kept: String = text.chars().take(string_limit).collect();
            let dropped = text.chars().count() - string_limit;
            Value::String(format!("{kept}[ctxc: {dropped} more characters omitted]"))
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContextSource, HeuristicTokenizer};

    fn optimizer() -> JsonOptimizer {
        JsonOptimizer::new(Arc::new(HeuristicTokenizer::new()))
    }

    fn context(content: &str) -> Context {
        Context::new(ContextSource::Stdin, ContentType::Json, content)
    }

    #[test]
    fn pretty_printed_json_is_minified_losslessly() {
        let pretty = "{\n  \"name\": \"ctxc\",\n  \"tags\": [\n    \"a\",\n    \"b\"\n  ]\n}\n";
        let optimized = optimizer().optimize(&context(pretty), None).unwrap();

        assert_eq!(
            optimized.content,
            "{\"name\":\"ctxc\",\"tags\":[\"a\",\"b\"]}\n"
        );
        assert!(optimized.result.savings_by_stage.compression > 0);
        assert_eq!(optimized.result.savings_by_stage.selection, 0);

        let value: Value = serde_json::from_str(&optimized.content).unwrap();
        assert_eq!(value, serde_json::from_str::<Value>(pretty).unwrap());
    }

    #[test]
    fn nothing_is_dropped_without_a_budget() {
        let items: Vec<String> = (0..500).map(|index| format!("\"item-{index}\"")).collect();
        let json = format!("{{\"items\":[{}]}}", items.join(","));

        let optimized = optimizer().optimize(&context(&json), None).unwrap();
        let value: Value = serde_json::from_str(&optimized.content).unwrap();
        assert_eq!(value["items"].as_array().unwrap().len(), 500);
    }

    #[test]
    fn long_arrays_shrink_under_a_budget_and_say_so() {
        let items: Vec<String> = (0..500).map(|index| format!("\"item-{index}\"")).collect();
        let json = format!("{{\"items\":[{}]}}", items.join(","));

        let optimized = optimizer()
            .optimize(&context(&json), Some(TokenBudget::new(200)))
            .unwrap();

        let value: Value = serde_json::from_str(&optimized.content).expect("still valid json");
        let array = value["items"].as_array().unwrap();
        assert!(array.len() < 500);
        assert!(array
            .last()
            .unwrap()
            .as_str()
            .unwrap()
            .contains("more items omitted"));
        assert!(optimized.result.savings_by_stage.selection > 0);
    }

    #[test]
    fn long_strings_shrink_under_a_budget() {
        let json = format!("{{\"body\":\"{}\"}}", "x".repeat(5_000));
        let optimized = optimizer()
            .optimize(&context(&json), Some(TokenBudget::new(60)))
            .unwrap();

        let value: Value = serde_json::from_str(&optimized.content).expect("still valid json");
        assert!(value["body"]
            .as_str()
            .unwrap()
            .contains("more characters omitted"));
    }

    #[test]
    fn output_stays_parseable_even_when_it_cannot_fit() {
        let items: Vec<String> = (0..2_000).map(|index| format!("{index}")).collect();
        let json = format!("[{}]", items.join(","));

        let optimized = optimizer()
            .optimize(&context(&json), Some(TokenBudget::new(1)))
            .unwrap();

        serde_json::from_str::<Value>(&optimized.content)
            .expect("a budget that cannot be met must not produce broken json");
    }

    #[test]
    fn json_lines_are_handled_one_value_per_line() {
        let jsonl = "{\"a\": 1}\n{\"b\": 2}\n";
        let optimized = optimizer().optimize(&context(jsonl), None).unwrap();

        assert_eq!(optimized.content, "{\"a\":1}\n{\"b\":2}\n");
        for line in optimized.content.lines() {
            serde_json::from_str::<Value>(line).unwrap();
        }
    }

    #[test]
    fn unparseable_content_is_returned_untouched() {
        let broken = "{\"name\": \"ctxc\", oops}\n";
        let optimized = optimizer().optimize(&context(broken), None).unwrap();

        assert_eq!(optimized.content, broken);
        assert_eq!(optimized.result.tokens_saved(), 0);
    }

    #[test]
    fn key_order_is_preserved() {
        let json = "{\"zebra\": 1, \"apple\": 2}";
        let optimized = optimizer().optimize(&context(json), None).unwrap();
        assert_eq!(optimized.content, "{\"zebra\":1,\"apple\":2}\n");
    }

    #[test]
    fn stage_savings_sum_to_the_total() {
        let items: Vec<String> = (0..300).map(|index| format!("\"item-{index}\"")).collect();
        let json = format!("{{\n  \"items\": [\n    {}\n  ]\n}}", items.join(",\n    "));

        let optimized = optimizer()
            .optimize(&context(&json), Some(TokenBudget::new(150)))
            .unwrap();

        assert_eq!(
            optimized.result.savings_by_stage.total(),
            optimized.result.tokens_saved()
        );
    }

    #[test]
    fn it_only_claims_json() {
        let optimizer = optimizer();
        assert!(optimizer.supports(&context("{}")));
        assert!(!optimizer.supports(&Context::new(ContextSource::Stdin, ContentType::Log, "{}")));
    }
}
