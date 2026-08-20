//! What a reader is shown.
//!
//! Reports are built from [`Totals`], never stored, so a ratio is always
//! computed from the sums it belongs to. They are the shape the CLI renders and
//! the HTTP API serializes, which is why they carry their own honesty flags:
//! whether the token counts were estimated, and what a cost figure assumes.

use serde::{Deserialize, Serialize};

use ctxc_core::Timestamp;

use crate::cost::{CostEstimate, CostRates};
use crate::event::Operation;
use crate::rollup::{Granularity, Rollup, Totals};
use crate::store::Window;

/// Savings attributed to the stage that produced them.
///
/// The point of the whole subsystem: "62% smaller" is trivia, "filtering took
/// 4,000 and deduplication took 2,000" is something a person can act on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageSavings {
    pub filtering: i64,
    pub deduplication: i64,
    pub compression: i64,
    pub selection: i64,
}

impl StageSavings {
    pub fn from_totals(totals: &Totals) -> Self {
        StageSavings {
            filtering: totals.saved_filtering,
            deduplication: totals.saved_deduplication,
            compression: totals.saved_compression,
            selection: totals.saved_selection,
        }
    }

    pub fn total(&self) -> i64 {
        self.filtering + self.deduplication + self.compression + self.selection
    }

    /// Each stage with its name, in pipeline order.
    pub fn labelled(&self) -> [(&'static str, i64); 4] {
        [
            ("filtering", self.filtering),
            ("deduplication", self.deduplication),
            ("compression", self.compression),
            ("selection", self.selection),
        ]
    }
}

/// Headline numbers for a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub window: Window,
    /// The project this covers, or `None` for everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub operations: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tokens_saved: i64,
    pub reduction_ratio: f64,
    pub savings_by_stage: StageSavings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_duration_ms: Option<f64>,
    pub slowest_duration_ms: i64,
    pub errors: u64,
    pub degradations: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_saved: Option<CostEstimate>,
    /// True when any of the token counts came from an estimator.
    pub estimated: bool,
}

impl Summary {
    /// Build a summary from totals, pricing the saving if rates are configured.
    pub fn build(
        window: Window,
        project_id: Option<String>,
        totals: &Totals,
        rates: Option<&CostRates>,
    ) -> Self {
        Summary {
            window,
            project_id,
            operations: totals.operations,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            tokens_saved: totals.tokens_saved(),
            reduction_ratio: totals.reduction_ratio(),
            savings_by_stage: StageSavings::from_totals(totals),
            average_duration_ms: totals.average_duration_ms(),
            slowest_duration_ms: totals.duration_ms_max,
            errors: totals.errors,
            degradations: totals.degradations,
            cache_hit_rate: totals.cache_hit_rate(),
            estimated_cost_saved: rates.map(|rates| rates.estimate(totals.tokens_saved())),
            estimated: totals.estimated > 0,
        }
    }

    /// Whether anything at all was recorded.
    pub fn is_empty(&self) -> bool {
        self.operations == 0
    }
}

/// One operation's share of a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationSummary {
    pub operation: Operation,
    pub operations: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tokens_saved: i64,
    pub reduction_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_duration_ms: Option<f64>,
    pub errors: u64,
    pub degradations: u64,
}

impl OperationSummary {
    pub fn build(operation: Operation, totals: &Totals) -> Self {
        OperationSummary {
            operation,
            operations: totals.operations,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            tokens_saved: totals.tokens_saved(),
            reduction_ratio: totals.reduction_ratio(),
            average_duration_ms: totals.average_duration_ms(),
            errors: totals.errors,
            degradations: totals.degradations,
        }
    }
}

/// Where the reduction came from, cut two ways.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Breakdown {
    pub window: Window,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub savings_by_stage: StageSavings,
    /// Busiest operation first.
    pub by_operation: Vec<OperationSummary>,
}

/// One point on a chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeseriesPoint {
    pub bucket_start: Timestamp,
    pub operations: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tokens_saved: i64,
    pub reduction_ratio: f64,
    pub savings_by_stage: StageSavings,
    pub errors: u64,
    pub degradations: u64,
}

impl TimeseriesPoint {
    pub fn build(bucket_start: Timestamp, totals: &Totals) -> Self {
        TimeseriesPoint {
            bucket_start,
            operations: totals.operations,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            tokens_saved: totals.tokens_saved(),
            reduction_ratio: totals.reduction_ratio(),
            savings_by_stage: StageSavings::from_totals(totals),
            errors: totals.errors,
            degradations: totals.degradations,
        }
    }
}

/// A series of buckets, evenly spaced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeseries {
    pub granularity: Granularity,
    pub window: Window,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Oldest bucket first. Buckets with no activity are present and zeroed, so
    /// a chart shows a quiet hour as quiet rather than as missing.
    pub points: Vec<TimeseriesPoint>,
}

/// Collapse rollups that share a bucket, ignoring project and operation.
pub fn totals_by_bucket(rollups: &[Rollup]) -> Vec<(Timestamp, Totals)> {
    let mut buckets: std::collections::BTreeMap<Timestamp, Totals> =
        std::collections::BTreeMap::new();
    for rollup in rollups {
        buckets
            .entry(rollup.key.bucket_start)
            .or_default()
            .merge(&rollup.totals);
    }
    buckets.into_iter().collect()
}

/// Collapse rollups by operation, busiest first.
pub fn totals_by_operation(rollups: &[Rollup]) -> Vec<(Operation, Totals)> {
    let mut by_operation: std::collections::BTreeMap<Operation, Totals> =
        std::collections::BTreeMap::new();
    for rollup in rollups {
        by_operation
            .entry(rollup.key.operation)
            .or_default()
            .merge(&rollup.totals);
    }

    let mut rows: Vec<(Operation, Totals)> = by_operation.into_iter().collect();
    rows.sort_by(|left, right| {
        right
            .1
            .operations
            .cmp(&left.1.operations)
            .then_with(|| left.0.as_str().cmp(right.0.as_str()))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::MetricEvent;
    use crate::rollup::aggregate;

    fn totals(input: u32, output: u32) -> Totals {
        let mut totals = Totals::default();
        totals
            .add_event(&MetricEvent::new(Operation::Optimize, "stdin").with_tokens(input, output));
        totals
    }

    #[test]
    fn a_summary_reports_the_saving_and_its_provenance() {
        let summary = Summary::build(Window::all_time(), None, &totals(1_000, 250), None);

        assert_eq!(summary.tokens_saved, 750);
        assert!((summary.reduction_ratio - 0.75).abs() < 1e-9);
        assert!(summary.estimated);
        assert!(summary.estimated_cost_saved.is_none());
    }

    #[test]
    fn a_summary_prices_the_saving_when_rates_are_configured() {
        let rates = CostRates {
            model: "some-model".into(),
            input_per_million: 4.0,
            currency: "USD".into(),
        };
        let summary = Summary::build(
            Window::all_time(),
            None,
            &totals(2_000_000, 1_000_000),
            Some(&rates),
        );

        let cost = summary.estimated_cost_saved.unwrap();
        assert!((cost.amount - 4.0).abs() < 1e-9);
        assert_eq!(cost.model, "some-model");
    }

    #[test]
    fn an_empty_summary_says_so_rather_than_dividing_by_zero() {
        let summary = Summary::build(Window::all_time(), None, &Totals::default(), None);

        assert!(summary.is_empty());
        assert_eq!(summary.reduction_ratio, 0.0);
        assert_eq!(summary.average_duration_ms, None);
        assert!(
            !summary.estimated,
            "nothing was measured, so nothing was estimated"
        );
    }

    #[test]
    fn operations_are_listed_busiest_first() {
        let mut events = vec![MetricEvent::new(Operation::Optimize, "stdin")];
        events.extend((0..3).map(|_| MetricEvent::new(Operation::Search, "cli")));

        let rows = totals_by_operation(&aggregate(&events, Granularity::Hour));
        assert_eq!(rows[0].0, Operation::Search);
        assert_eq!(rows[0].1.operations, 3);
        assert_eq!(rows[1].0, Operation::Optimize);
    }

    #[test]
    fn buckets_collapse_across_projects() {
        let at = Timestamp::from_millis(0);
        let events = vec![
            MetricEvent::new(Operation::Optimize, "a")
                .at(at)
                .for_project("one")
                .with_tokens(100, 50),
            MetricEvent::new(Operation::Optimize, "b")
                .at(at)
                .for_project("two")
                .with_tokens(100, 60),
        ];

        let buckets = totals_by_bucket(&aggregate(&events, Granularity::Hour));
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].1.operations, 2);
        assert_eq!(buckets[0].1.tokens_saved(), 90);
    }

    #[test]
    fn stage_savings_add_up_to_the_total() {
        let savings = StageSavings {
            filtering: 4_000,
            deduplication: 2_000,
            compression: 5_000,
            selection: 3_000,
        };
        assert_eq!(savings.total(), 14_000);
        assert_eq!(savings.labelled()[0], ("filtering", 4_000));
    }
}
