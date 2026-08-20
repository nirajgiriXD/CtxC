//! Aggregates.
//!
//! Raw events are pruned after a few weeks; aggregates are kept for years. That
//! only works if an aggregate stores sums and never ratios — sums can be added
//! together, averages cannot — so every rollup here is additive and every ratio
//! is computed at the moment a report is rendered.
//!
//! ```text
//! events --group by (granularity, bucket, project, operation)--> rollups
//! ```
//!
//! Aggregation is pure code rather than SQL so that there is exactly one
//! implementation of what "hourly" means, and so it can be tested without a
//! database.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use ctxc_core::Timestamp;

use crate::event::{MetricEvent, Operation, Outcome, UnknownName};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// The size of an aggregate bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Hour,
    Day,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::Hour => "hour",
            Granularity::Day => "day",
        }
    }

    /// Length of one bucket, in milliseconds.
    pub const fn millis(self) -> i64 {
        match self {
            Granularity::Hour => HOUR_MS,
            Granularity::Day => DAY_MS,
        }
    }

    /// The start of the bucket `at` falls in.
    ///
    /// Buckets are aligned to UTC, not to local time: a machine that changes
    /// timezone or crosses a daylight-saving boundary must not produce buckets
    /// that overlap or leave a gap.
    pub fn bucket(self, at: Timestamp) -> Timestamp {
        let millis = at.as_millis();
        // Saturating, because `Window::all_time` starts at the first
        // representable instant and flooring that has nowhere to go.
        let floored = millis.saturating_sub(millis.rem_euclid(self.millis()));
        Timestamp::from_millis(floored)
    }
}

impl fmt::Display for Granularity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Granularity {
    type Err = UnknownName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "hour" => Ok(Granularity::Hour),
            "day" => Ok(Granularity::Day),
            other => Err(UnknownName {
                kind: "granularity",
                value: other.to_owned(),
            }),
        }
    }
}

/// Which bucket a rollup belongs to.
///
/// Project is a `String` rather than an `Option`, with the empty string meaning
/// "no project", because this doubles as a primary key and SQLite will not
/// compare NULLs for uniqueness.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RollupKey {
    pub granularity: Granularity,
    pub bucket_start: Timestamp,
    pub project_id: String,
    pub operation: Operation,
}

impl RollupKey {
    /// The bucket an event belongs to at this granularity.
    pub fn of(event: &MetricEvent, granularity: Granularity) -> Self {
        RollupKey {
            granularity,
            bucket_start: granularity.bucket(event.recorded_at),
            project_id: event.project_id.clone().unwrap_or_default(),
            operation: event.operation,
        }
    }

    /// The project, or `None` for operations that had none.
    pub fn project(&self) -> Option<&str> {
        (!self.project_id.is_empty()).then_some(self.project_id.as_str())
    }
}

/// Additive totals for one bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub operations: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub saved_filtering: i64,
    pub saved_deduplication: i64,
    pub saved_compression: i64,
    pub saved_selection: i64,
    pub duration_ms_total: i64,
    pub duration_ms_max: i64,
    pub errors: u64,
    pub degradations: u64,
    pub cache_hits: u64,
    /// Operations that consulted a cache at all. The denominator of the hit
    /// rate, which is not the operation count: most operations have no cache.
    pub cache_lookups: u64,
    /// Operations whose token counts were estimated. Reported so a summary can
    /// say honestly whether *any* of what it adds up is exact.
    pub estimated: u64,
}

impl Totals {
    /// Fold one event in.
    pub fn add_event(&mut self, event: &MetricEvent) {
        let savings = &event.savings_by_stage;

        self.operations += 1;
        self.input_tokens += event.input_tokens as i64;
        self.output_tokens += event.output_tokens as i64;
        self.saved_filtering += savings.filtering as i64;
        self.saved_deduplication += savings.deduplication as i64;
        self.saved_compression += savings.compression as i64;
        self.saved_selection += savings.selection as i64;
        self.duration_ms_total += event.duration_ms as i64;
        self.duration_ms_max = self.duration_ms_max.max(event.duration_ms as i64);

        match event.outcome {
            Outcome::Success => {}
            Outcome::Degraded => self.degradations += 1,
            Outcome::Failed => self.errors += 1,
        }
        self.cache_lookups += event.cache.lookups as u64;
        self.cache_hits += event.cache.hits as u64;
        if event.estimated {
            self.estimated += 1;
        }
    }

    /// Merge another bucket's totals in.
    pub fn merge(&mut self, other: &Totals) {
        self.operations += other.operations;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.saved_filtering += other.saved_filtering;
        self.saved_deduplication += other.saved_deduplication;
        self.saved_compression += other.saved_compression;
        self.saved_selection += other.saved_selection;
        self.duration_ms_total += other.duration_ms_total;
        self.duration_ms_max = self.duration_ms_max.max(other.duration_ms_max);
        self.errors += other.errors;
        self.degradations += other.degradations;
        self.cache_hits += other.cache_hits;
        self.cache_lookups += other.cache_lookups;
        self.estimated += other.estimated;
    }

    /// Tokens removed overall.
    pub fn tokens_saved(&self) -> i64 {
        self.input_tokens - self.output_tokens
    }

    /// Share of the input removed. Growth reports as no reduction, matching
    /// [`ctxc_core::optimization::reduction_ratio`].
    pub fn reduction_ratio(&self) -> f64 {
        if self.input_tokens <= 0 {
            return 0.0;
        }
        (self.tokens_saved().max(0) as f64) / self.input_tokens as f64
    }

    /// Mean operation latency, or `None` when nothing was recorded.
    pub fn average_duration_ms(&self) -> Option<f64> {
        (self.operations > 0).then(|| self.duration_ms_total as f64 / self.operations as f64)
    }

    /// Share of cache lookups that hit, or `None` when nothing consulted a
    /// cache. Zero and "not applicable" must not look the same.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        (self.cache_lookups > 0).then(|| self.cache_hits as f64 / self.cache_lookups as f64)
    }
}

/// One aggregate row: where it belongs, and what it adds up to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rollup {
    #[serde(flatten)]
    pub key: RollupKey,
    #[serde(flatten)]
    pub totals: Totals,
}

/// Group events into buckets.
///
/// The result is sorted by key, which keeps writes in a stable order and makes
/// the output of a rollup run reproducible.
pub fn aggregate(events: &[MetricEvent], granularity: Granularity) -> Vec<Rollup> {
    let mut buckets: BTreeMap<RollupKey, Totals> = BTreeMap::new();
    for event in events {
        buckets
            .entry(RollupKey::of(event, granularity))
            .or_default()
            .add_event(event);
    }

    buckets
        .into_iter()
        .map(|(key, totals)| Rollup { key, totals })
        .collect()
}

/// Add up every rollup in a slice, ignoring which bucket each came from.
pub fn total(rollups: &[Rollup]) -> Totals {
    let mut totals = Totals::default();
    for rollup in rollups {
        totals.merge(&rollup.totals);
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::optimization::{SavingsByStage, Stage};

    fn event(operation: Operation, at: i64) -> MetricEvent {
        MetricEvent::new(operation, "stdin").at(Timestamp::from_millis(at))
    }

    #[test]
    fn buckets_floor_to_the_granularity() {
        let at = Timestamp::from_millis(1_700_000_000_000); // 2023-11-14T22:13:20Z
        assert_eq!(
            Granularity::Hour.bucket(at).to_rfc3339(),
            "2023-11-14T22:00:00.000Z"
        );
        assert_eq!(
            Granularity::Day.bucket(at).to_rfc3339(),
            "2023-11-14T00:00:00.000Z"
        );
    }

    #[test]
    fn buckets_before_the_epoch_still_floor_downwards() {
        let at = Timestamp::from_millis(-1);
        assert_eq!(
            Granularity::Day.bucket(at).to_rfc3339(),
            "1969-12-31T00:00:00.000Z"
        );
    }

    #[test]
    fn events_group_by_bucket_project_and_operation() {
        let events = vec![
            event(Operation::Optimize, 0).with_tokens(100, 40),
            event(Operation::Optimize, HOUR_MS - 1).with_tokens(200, 100),
            event(Operation::Optimize, HOUR_MS).with_tokens(50, 50),
            event(Operation::Search, 0),
            event(Operation::Optimize, 0)
                .for_project("acme")
                .with_tokens(10, 5),
        ];

        let hourly = aggregate(&events, Granularity::Hour);
        assert_eq!(hourly.len(), 4, "two buckets, two projects, two operations");

        let first = hourly
            .iter()
            .find(|rollup| {
                rollup.key.bucket_start.as_millis() == 0
                    && rollup.key.operation == Operation::Optimize
                    && rollup.key.project().is_none()
            })
            .unwrap();
        assert_eq!(first.totals.operations, 2);
        assert_eq!(first.totals.input_tokens, 300);
        assert_eq!(first.totals.tokens_saved(), 160);

        let daily = aggregate(&events, Granularity::Day);
        assert_eq!(daily.len(), 3, "one day, two projects, two operations");
    }

    #[test]
    fn stage_savings_survive_aggregation() {
        let mut savings = SavingsByStage::default();
        savings.record(Stage::Compression, 500);
        savings.record(Stage::Deduplication, -20);

        let mut totals = Totals::default();
        for _ in 0..3 {
            let mut event = event(Operation::Optimize, 0);
            event.savings_by_stage = savings;
            totals.add_event(&event);
        }

        assert_eq!(totals.saved_compression, 1_500);
        assert_eq!(
            totals.saved_deduplication, -60,
            "a stage that costs tokens must stay negative through aggregation"
        );
    }

    #[test]
    fn ratios_are_computed_from_sums_not_averaged() {
        let events = vec![
            event(Operation::Optimize, 0).with_tokens(1_000, 100),
            event(Operation::Optimize, 0).with_tokens(10, 9),
        ];
        let totals = total(&aggregate(&events, Granularity::Hour));

        // Averaging the two ratios would give roughly 0.5; the honest answer
        // weights by size, and lands near 0.89.
        assert!((totals.reduction_ratio() - 901.0 / 1010.0).abs() < 1e-9);
    }

    #[test]
    fn cache_hit_rate_ignores_operations_without_a_cache() {
        let mut totals = Totals::default();
        totals.add_event(&event(Operation::Search, 0));
        assert_eq!(totals.cache_hit_rate(), None);

        totals.add_event(&event(Operation::Search, 0).with_cache_hit(true));
        totals.add_event(&event(Operation::Search, 0).with_cache_hit(false));
        assert_eq!(totals.cache_hit_rate(), Some(0.5));
    }

    #[test]
    fn outcomes_are_counted_apart() {
        let mut totals = Totals::default();
        totals.add_event(&event(Operation::Watch, 0));
        totals.add_event(&event(Operation::Watch, 0).degraded("polling"));
        totals.add_event(&event(Operation::Watch, 0).failed("permission denied"));

        assert_eq!(totals.operations, 3);
        assert_eq!(totals.degradations, 1);
        assert_eq!(totals.errors, 1);
    }

    #[test]
    fn merging_keeps_the_maximum_rather_than_adding_it() {
        let mut left = Totals {
            duration_ms_max: 90,
            ..Totals::default()
        };
        left.merge(&Totals {
            duration_ms_max: 40,
            ..Totals::default()
        });
        assert_eq!(left.duration_ms_max, 90);
    }

    #[test]
    fn granularity_names_round_trip() {
        for granularity in [Granularity::Hour, Granularity::Day] {
            assert_eq!(
                granularity.as_str().parse::<Granularity>().unwrap(),
                granularity
            );
        }
        assert!("week".parse::<Granularity>().is_err());
    }
}
