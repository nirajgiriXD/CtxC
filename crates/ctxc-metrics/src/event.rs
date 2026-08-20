//! What one operation cost and what it saved.
//!
//! An event is a fact about a single operation, recorded once and never
//! updated. Everything the metrics subsystem reports is derived from these, so
//! the type carries raw numbers and none of the ratios: a ratio computed at
//! write time cannot be re-aggregated later without lying.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use ctxc_core::optimization::{reduction_ratio, OptimizationResult, SavingsByStage};
use ctxc_core::Timestamp;

/// The kind of work an event describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Analyze,
    Optimize,
    Compile,
    Index,
    Search,
    Retrieve,
    /// A settled batch of file changes applied to the index.
    Watch,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Analyze => "analyze",
            Operation::Optimize => "optimize",
            Operation::Compile => "compile",
            Operation::Index => "index",
            Operation::Search => "search",
            Operation::Retrieve => "retrieve",
            Operation::Watch => "watch",
        }
    }

    /// Every operation, in the order reports list them.
    pub const ALL: [Operation; 7] = [
        Operation::Analyze,
        Operation::Optimize,
        Operation::Compile,
        Operation::Index,
        Operation::Search,
        Operation::Retrieve,
        Operation::Watch,
    ];
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Operation {
    type Err = UnknownName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Operation::ALL
            .into_iter()
            .find(|operation| operation.as_str() == value)
            .ok_or_else(|| UnknownName {
                kind: "operation",
                value: value.to_owned(),
            })
    }
}

/// How an operation ended.
///
/// Degradation is recorded separately from failure because it is the more
/// interesting signal: a watcher that fell back to polling still produced a
/// correct index, but the fallback is exactly what someone reading metrics
/// needs to see.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Success,
    Degraded,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Degraded => "degraded",
            Outcome::Failed => "failed",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Outcome {
    type Err = UnknownName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "success" => Ok(Outcome::Success),
            "degraded" => Ok(Outcome::Degraded),
            "failed" => Ok(Outcome::Failed),
            other => Err(UnknownName {
                kind: "outcome",
                value: other.to_owned(),
            }),
        }
    }
}

/// How much of an operation was answered from a cache.
///
/// Counts rather than a boolean, because the operations with the most
/// interesting hit rate answer many lookups at once: an index pass that skipped
/// 900 unchanged files out of 1,000 is 90% hits, and a single `bool` cannot say
/// that. `lookups == 0` means the operation consulted no cache at all, which is
/// not the same as consulting one and missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheUse {
    pub hits: u32,
    pub lookups: u32,
}

impl CacheUse {
    /// One lookup, hit or miss.
    pub fn once(hit: bool) -> Self {
        CacheUse {
            hits: u32::from(hit),
            lookups: 1,
        }
    }

    /// `hits` out of `lookups`, clamped so a caller cannot report more hits
    /// than it made lookups.
    pub fn of(hits: u32, lookups: u32) -> Self {
        CacheUse {
            hits: hits.min(lookups),
            lookups,
        }
    }

    /// Whether a cache was consulted at all.
    pub fn is_used(&self) -> bool {
        self.lookups > 0
    }
}

/// A stored name this build does not recognise, which is what reading a
/// database written by a newer CtxC looks like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownName {
    pub kind: &'static str,
    pub value: String,
}

impl fmt::Display for UnknownName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {} `{}`", self.kind, self.value)
    }
}

impl std::error::Error for UnknownName {}

/// One recorded operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricEvent {
    /// The project this belongs to, when the operation had one. A piped
    /// `git status` does not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub operation: Operation,
    /// Where the input came from: `stdin`, a file, a command line.
    pub source: String,
    /// Optimizer that handled it, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<String>,
    pub recorded_at: Timestamp,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration_ms: u64,
    pub savings_by_stage: SavingsByStage,
    /// Lookups this operation answered from a cache. Zeroed when it has none.
    pub cache: CacheUse,
    pub outcome: Outcome,
    /// Why an operation degraded or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Whether the token counts came from an estimator rather than the exact
    /// target tokenizer. Carried per event so that a later exact tokenizer does
    /// not retroactively relabel rows it did not produce.
    pub estimated: bool,
}

impl MetricEvent {
    /// An event for an operation that does not move tokens.
    pub fn new(operation: Operation, source: impl Into<String>) -> Self {
        MetricEvent {
            project_id: None,
            operation,
            source: source.into(),
            optimizer: None,
            recorded_at: Timestamp::now(),
            input_tokens: 0,
            output_tokens: 0,
            duration_ms: 0,
            savings_by_stage: SavingsByStage::default(),
            cache: CacheUse::default(),
            outcome: Outcome::Success,
            detail: None,
            estimated: true,
        }
    }

    /// An event describing what an optimizer did.
    pub fn from_result(
        operation: Operation,
        source: impl Into<String>,
        result: &OptimizationResult,
    ) -> Self {
        MetricEvent {
            optimizer: Some(result.optimizer.clone()),
            input_tokens: result.original_tokens,
            output_tokens: result.optimized_tokens,
            savings_by_stage: result.savings_by_stage,
            estimated: result.estimated,
            ..MetricEvent::new(operation, source)
        }
    }

    pub fn for_project(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    /// Attach a project only when the caller found one.
    pub fn for_project_opt(mut self, project_id: Option<String>) -> Self {
        self.project_id = project_id;
        self
    }

    pub fn took(mut self, duration: std::time::Duration) -> Self {
        self.duration_ms = duration.as_millis().min(u64::MAX as u128) as u64;
        self
    }

    /// Record a single cache lookup.
    pub fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache = CacheUse::once(hit);
        self
    }

    /// Record many lookups at once.
    pub fn with_cache(mut self, hits: u32, lookups: u32) -> Self {
        self.cache = CacheUse::of(hits, lookups);
        self
    }

    pub fn with_tokens(mut self, input: u32, output: u32) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }

    pub fn degraded(mut self, reason: impl Into<String>) -> Self {
        self.outcome = Outcome::Degraded;
        self.detail = Some(reason.into());
        self
    }

    pub fn failed(mut self, reason: impl Into<String>) -> Self {
        self.outcome = Outcome::Failed;
        self.detail = Some(reason.into());
        self
    }

    /// Timestamp an event explicitly, for tests and for replaying history.
    pub fn at(mut self, recorded_at: Timestamp) -> Self {
        self.recorded_at = recorded_at;
        self
    }

    /// Tokens removed. Negative when the operation grew its input.
    pub fn tokens_saved(&self) -> i64 {
        self.input_tokens as i64 - self.output_tokens as i64
    }

    /// Share of the input removed, between 0.0 and 1.0.
    pub fn reduction_ratio(&self) -> f64 {
        reduction_ratio(self.input_tokens, self.output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::optimization::{Counts, Stage};
    use ctxc_core::HeuristicTokenizer;

    #[test]
    fn operation_names_round_trip() {
        for operation in Operation::ALL {
            assert_eq!(operation.as_str().parse::<Operation>().unwrap(), operation);
        }
        assert!("nonsense".parse::<Operation>().is_err());
    }

    #[test]
    fn outcome_names_round_trip() {
        for outcome in [Outcome::Success, Outcome::Degraded, Outcome::Failed] {
            assert_eq!(outcome.as_str().parse::<Outcome>().unwrap(), outcome);
        }
        assert!("maybe".parse::<Outcome>().is_err());
    }

    #[test]
    fn an_event_carries_what_the_optimizer_measured() {
        let tokenizer = HeuristicTokenizer::new();
        let mut savings = SavingsByStage::default();
        savings.record(Stage::Filtering, 400);

        let result = OptimizationResult::new(
            "text",
            &tokenizer,
            Counts {
                original_tokens: 1_000,
                optimized_tokens: 600,
                preserved_fragments: 3,
                removed_fragments: 1,
            },
            savings,
        );

        let event = MetricEvent::from_result(Operation::Optimize, "stdin", &result)
            .for_project("acme-web")
            .took(std::time::Duration::from_millis(42));

        assert_eq!(event.tokens_saved(), 400);
        assert!((event.reduction_ratio() - 0.4).abs() < 1e-9);
        assert_eq!(event.optimizer.as_deref(), Some("text"));
        assert_eq!(event.project_id.as_deref(), Some("acme-web"));
        assert_eq!(event.duration_ms, 42);
        assert!(event.estimated, "the heuristic tokenizer only estimates");
    }

    #[test]
    fn growth_is_recorded_as_a_negative_saving() {
        let event = MetricEvent::new(Operation::Optimize, "stdin").with_tokens(100, 140);
        assert_eq!(event.tokens_saved(), -40);
        assert_eq!(
            event.reduction_ratio(),
            0.0,
            "a ratio never goes negative; the token delta carries the sign"
        );
    }

    #[test]
    fn cache_use_counts_rather_than_flags() {
        assert_eq!(
            CacheUse::once(true),
            CacheUse {
                hits: 1,
                lookups: 1
            }
        );
        assert_eq!(
            CacheUse::once(false),
            CacheUse {
                hits: 0,
                lookups: 1
            }
        );
        assert!(!CacheUse::default().is_used());
        assert!(CacheUse::once(false).is_used(), "a miss is still a lookup");
    }

    #[test]
    fn more_hits_than_lookups_is_clamped_rather_than_believed() {
        assert_eq!(
            CacheUse::of(50, 10),
            CacheUse {
                hits: 10,
                lookups: 10
            }
        );
    }

    #[test]
    fn a_degraded_event_keeps_its_reason() {
        let event =
            MetricEvent::new(Operation::Watch, "supervisor").degraded("watch limit reached");
        assert_eq!(event.outcome, Outcome::Degraded);
        assert_eq!(event.detail.as_deref(), Some("watch limit reached"));
    }
}
