//! Local metrics for CtxC.
//!
//! A dedicated subsystem, not a side effect of logging. Every operation emits
//! an event; events are rolled up into hourly and daily aggregates; raw events
//! are pruned after a while and aggregates are kept.
//!
//! ```text
//! operation -> Collector -> MetricsStore -> rollups -> Summary / Timeseries
//! ```
//!
//! Two rules shape everything here. Recording must never block or break the
//! operation being measured, which is why [`Collector`] buffers in memory and
//! returns nothing. And savings are attributed to the stage that produced
//! them, because "62% smaller" cannot be acted on and "deduplication took
//! 2,000 tokens" can.
//!
//! Metrics are local. Nothing in this crate transmits anything; telemetry is a
//! separate setting, off by default, and unrelated.

pub mod collector;
pub mod cost;
pub mod error;
pub mod event;
pub mod report;
pub mod rollup;
pub mod store;

pub use collector::{Collector, MetricsSink};
pub use cost::{CostEstimate, CostRates};
pub use error::{MetricsError, Result};
pub use event::{CacheUse, MetricEvent, Operation, Outcome};
pub use report::{Breakdown, OperationSummary, StageSavings, Summary, Timeseries, TimeseriesPoint};
pub use rollup::{aggregate, Granularity, Rollup, RollupKey, Totals};
pub use store::{MetricsStore, RollupQuery, Window};

use ctxc_core::Timestamp;

/// How long each kind of row is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Days of raw events to keep. Zero disables pruning, which is what an
    /// operator debugging something wants.
    pub raw_days: u32,
    /// Days of hourly aggregates to keep. Daily aggregates are never pruned:
    /// they are the long-term record, and one row per project per operation
    /// per day costs nothing to keep.
    pub hourly_days: u32,
}

impl Retention {
    pub fn from_config(config: &ctxc_core::Config) -> Self {
        Retention {
            raw_days: config.metrics.raw_retention_days,
            hourly_days: config.metrics.hourly_retention_days,
        }
    }

    /// The cutoff `days` before `now`, or `None` when retention is disabled.
    fn cutoff(days: u32, now: Timestamp) -> Option<Timestamp> {
        (days > 0).then(|| {
            let span = (days as i64).saturating_mul(86_400_000);
            Timestamp::from_millis(now.as_millis().saturating_sub(span))
        })
    }
}

/// What a maintenance pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Maintenance {
    pub buckets_written: usize,
    pub events_pruned: usize,
    pub rollups_pruned: usize,
}

impl Maintenance {
    pub fn did_nothing(&self) -> bool {
        *self == Maintenance::default()
    }
}

/// Reading and maintaining recorded metrics.
///
/// Borrows a store rather than owning one, because callers already hold an open
/// database and opening a second connection to answer one question would be
/// both slower and a source of lock contention with the daemon.
pub struct Metrics<'a> {
    store: &'a dyn MetricsStore,
    rates: Option<CostRates>,
    retention: Retention,
}

impl<'a> Metrics<'a> {
    pub fn new(store: &'a dyn MetricsStore, config: &ctxc_core::Config) -> Self {
        Metrics {
            store,
            rates: CostRates::from_config(config),
            retention: Retention::from_config(config),
        }
    }

    /// Build one with explicit settings, for tests and for callers with no
    /// loaded configuration.
    pub fn with_settings(
        store: &'a dyn MetricsStore,
        rates: Option<CostRates>,
        retention: Retention,
    ) -> Self {
        Metrics {
            store,
            rates,
            retention,
        }
    }

    /// Headline numbers for a window.
    pub fn summary(&self, window: Window, project_id: Option<&str>) -> Result<Summary> {
        let rollups = self.read(Granularity::Hour, window, project_id, None)?;
        Ok(Summary::build(
            window,
            project_id.map(str::to_owned),
            &rollup::total(&rollups),
            self.rates.as_ref(),
        ))
    }

    /// Where the reduction came from.
    pub fn breakdown(&self, window: Window, project_id: Option<&str>) -> Result<Breakdown> {
        let rollups = self.read(Granularity::Hour, window, project_id, None)?;
        let totals = rollup::total(&rollups);

        Ok(Breakdown {
            window,
            project_id: project_id.map(str::to_owned),
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            savings_by_stage: StageSavings::from_totals(&totals),
            by_operation: report::totals_by_operation(&rollups)
                .into_iter()
                .map(|(operation, totals)| OperationSummary::build(operation, &totals))
                .collect(),
        })
    }

    /// Activity over time, one point per bucket.
    ///
    /// Quiet buckets are present and zeroed rather than absent: a gap in a
    /// chart should mean "nothing happened", and a missing point cannot say
    /// that.
    pub fn timeseries(
        &self,
        granularity: Granularity,
        window: Window,
        project_id: Option<&str>,
    ) -> Result<Timeseries> {
        let window = window.aligned(granularity);
        let rollups = self.read(granularity, window, project_id, None)?;
        let measured: std::collections::BTreeMap<Timestamp, Totals> =
            report::totals_by_bucket(&rollups).into_iter().collect();

        let step = granularity.millis();
        let mut points = Vec::new();
        let mut bucket = granularity.bucket(window.from).as_millis();
        // The window is half-open, so the last bucket is the one holding the
        // final representable instant inside it — not the one starting at
        // `to`, which belongs to the next window.
        let last = granularity
            .bucket(Timestamp::from_millis(
                window.to.as_millis().saturating_sub(1),
            ))
            .as_millis();

        // A window that starts at the beginning of representable time would run
        // for longer than anyone will wait; bound the series by what was
        // actually recorded in that case.
        if window.from.as_millis() == i64::MIN {
            bucket = measured
                .keys()
                .next()
                .map_or(last, |first| first.as_millis());
        }

        while bucket <= last {
            let at = Timestamp::from_millis(bucket);
            let totals = measured.get(&at).copied().unwrap_or_default();
            points.push(TimeseriesPoint::build(at, &totals));
            match bucket.checked_add(step) {
                Some(next) => bucket = next,
                None => break,
            }
        }

        Ok(Timeseries {
            granularity,
            window,
            project_id: project_id.map(str::to_owned),
            points,
        })
    }

    /// The most recent operations, newest first.
    pub fn activity(&self, project_id: Option<&str>, limit: usize) -> Result<Vec<MetricEvent>> {
        self.store.recent_events(project_id, limit)
    }

    /// Bring aggregates up to date, without pruning anything.
    ///
    /// Every report reads aggregates, and events recorded since the last
    /// maintenance pass are not in them yet. So a reader calls this first:
    /// rolling up is idempotent and touches only buckets that have changed,
    /// whereas pruning is a retention decision and has no business firing
    /// because somebody looked at a chart.
    pub fn refresh(&self) -> Result<usize> {
        self.refresh_at(Timestamp::now())
    }

    /// Refresh against an explicit clock.
    pub fn refresh_at(&self, now: Timestamp) -> Result<usize> {
        let mut written = 0;
        for granularity in [Granularity::Hour, Granularity::Day] {
            written += self.roll_up(granularity, now)?;
        }
        Ok(written)
    }

    /// Roll up, then prune. In that order, always.
    ///
    /// Pruning first would delete raw events that no aggregate had absorbed
    /// yet, which loses history silently — the one failure mode a metrics
    /// subsystem cannot have.
    pub fn maintain(&self) -> Result<Maintenance> {
        self.maintain_at(Timestamp::now())
    }

    /// Maintenance against an explicit clock, so retention can be tested
    /// without waiting a month.
    pub fn maintain_at(&self, now: Timestamp) -> Result<Maintenance> {
        let mut report = Maintenance {
            buckets_written: self.refresh_at(now)?,
            ..Maintenance::default()
        };

        if let Some(before) = Retention::cutoff(self.retention.raw_days, now) {
            report.events_pruned = self.store.delete_events_before(before)?;
        }
        if let Some(before) = Retention::cutoff(self.retention.hourly_days, now) {
            report.rollups_pruned = self
                .store
                .delete_rollups_before(Granularity::Hour, before)?;
        }

        if !report.did_nothing() {
            tracing::debug!(
                buckets = report.buckets_written,
                events_pruned = report.events_pruned,
                rollups_pruned = report.rollups_pruned,
                "metrics maintenance"
            );
        }
        Ok(report)
    }

    /// Aggregate everything not yet aggregated at this granularity.
    ///
    /// Starts at the newest existing bucket rather than after it: that bucket
    /// was still filling when it was last written, so it is recomputed until it
    /// stops changing. Writes replace rather than add, which is what makes
    /// running this twice harmless.
    fn roll_up(&self, granularity: Granularity, now: Timestamp) -> Result<usize> {
        let from = match self.store.latest_rollup_bucket(granularity)? {
            Some(bucket) => bucket,
            None => match self.store.earliest_event()? {
                Some(earliest) => granularity.bucket(earliest),
                None => return Ok(0),
            },
        };

        let window = Window::new(from, Timestamp::from_millis(now.as_millis() + 1))?;
        let events = self.store.events_in(window)?;
        if events.is_empty() {
            return Ok(0);
        }

        let rollups = aggregate(&events, granularity);
        self.store.upsert_rollups(&rollups)?;
        Ok(rollups.len())
    }

    /// Read aggregates, preferring the hourly table for anything short enough.
    fn read(
        &self,
        granularity: Granularity,
        window: Window,
        project_id: Option<&str>,
        operation: Option<Operation>,
    ) -> Result<Vec<Rollup>> {
        let mut query = RollupQuery::new(granularity, window.aligned(granularity))
            .for_project_opt(project_id.map(str::to_owned));
        query.operation = operation;
        self.store.rollups(&query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// An in-memory store, so the rules in this crate can be tested without a
    /// database underneath them.
    #[derive(Default)]
    struct MemoryStore {
        events: RefCell<Vec<MetricEvent>>,
        rollups: RefCell<BTreeMap<RollupKey, Totals>>,
    }

    impl MetricsSink for MemoryStore {
        fn write_events(&self, events: &[MetricEvent]) -> Result<()> {
            self.insert_events(events)
        }
    }

    impl MetricsStore for MemoryStore {
        fn insert_events(&self, events: &[MetricEvent]) -> Result<()> {
            self.events.borrow_mut().extend_from_slice(events);
            self.events
                .borrow_mut()
                .sort_by_key(|event| event.recorded_at);
            Ok(())
        }

        fn events_in(&self, window: Window) -> Result<Vec<MetricEvent>> {
            Ok(self
                .events
                .borrow()
                .iter()
                .filter(|event| window.contains(event.recorded_at))
                .cloned()
                .collect())
        }

        fn recent_events(
            &self,
            project_id: Option<&str>,
            limit: usize,
        ) -> Result<Vec<MetricEvent>> {
            Ok(self
                .events
                .borrow()
                .iter()
                .rev()
                .filter(|event| match project_id {
                    Some(wanted) => event.project_id.as_deref() == Some(wanted),
                    None => true,
                })
                .take(limit)
                .cloned()
                .collect())
        }

        fn earliest_event(&self) -> Result<Option<Timestamp>> {
            Ok(self.events.borrow().first().map(|event| event.recorded_at))
        }

        fn upsert_rollups(&self, rollups: &[Rollup]) -> Result<()> {
            let mut stored = self.rollups.borrow_mut();
            for rollup in rollups {
                stored.insert(rollup.key.clone(), rollup.totals);
            }
            Ok(())
        }

        fn latest_rollup_bucket(&self, granularity: Granularity) -> Result<Option<Timestamp>> {
            Ok(self
                .rollups
                .borrow()
                .keys()
                .filter(|key| key.granularity == granularity)
                .map(|key| key.bucket_start)
                .max())
        }

        fn rollups(&self, query: &RollupQuery) -> Result<Vec<Rollup>> {
            Ok(self
                .rollups
                .borrow()
                .iter()
                .filter(|(key, _)| {
                    key.granularity == query.granularity
                        && query.window.contains(key.bucket_start)
                        // `map_or(true, ..)` rather than `is_none_or`, which
                        // needs a newer Rust than this workspace supports.
                        && query
                            .project_id
                            .as_ref()
                            .map_or(true, |wanted| &key.project_id == wanted)
                        && query
                            .operation
                            .map_or(true, |wanted| key.operation == wanted)
                })
                .map(|(key, totals)| Rollup {
                    key: key.clone(),
                    totals: *totals,
                })
                .collect())
        }

        fn delete_events_before(&self, before: Timestamp) -> Result<usize> {
            let mut events = self.events.borrow_mut();
            let before_count = events.len();
            events.retain(|event| event.recorded_at >= before);
            Ok(before_count - events.len())
        }

        fn delete_rollups_before(
            &self,
            granularity: Granularity,
            before: Timestamp,
        ) -> Result<usize> {
            let mut rollups = self.rollups.borrow_mut();
            let before_count = rollups.len();
            rollups.retain(|key, _| !(key.granularity == granularity && key.bucket_start < before));
            Ok(before_count - rollups.len())
        }
    }

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 86_400_000;

    fn retention() -> Retention {
        Retention {
            raw_days: 30,
            hourly_days: 90,
        }
    }

    fn event(at: i64, input: u32, output: u32) -> MetricEvent {
        MetricEvent::new(Operation::Optimize, "stdin")
            .at(Timestamp::from_millis(at))
            .with_tokens(input, output)
    }

    #[test]
    fn maintenance_rolls_events_into_both_granularities() {
        let store = MemoryStore::default();
        store
            .insert_events(&[event(0, 1_000, 400), event(HOUR, 500, 100)])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        let report = metrics
            .maintain_at(Timestamp::from_millis(2 * HOUR))
            .unwrap();

        assert_eq!(report.buckets_written, 3, "two hourly buckets, one daily");
        assert_eq!(report.events_pruned, 0);
    }

    #[test]
    fn running_maintenance_twice_does_not_double_count() {
        let store = MemoryStore::default();
        store.insert_events(&[event(0, 1_000, 400)]).unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        let now = Timestamp::from_millis(HOUR);
        metrics.maintain_at(now).unwrap();
        metrics.maintain_at(now).unwrap();

        let summary = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(summary.operations, 1);
        assert_eq!(summary.tokens_saved, 600);
    }

    #[test]
    fn a_bucket_that_is_still_filling_is_recomputed() {
        let store = MemoryStore::default();
        store.insert_events(&[event(0, 1_000, 400)]).unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        metrics.maintain_at(Timestamp::from_millis(60_000)).unwrap();

        store.insert_events(&[event(120_000, 500, 100)]).unwrap();
        metrics
            .maintain_at(Timestamp::from_millis(180_000))
            .unwrap();

        let summary = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(summary.operations, 2, "the same hour picked up both events");
        assert_eq!(summary.tokens_saved, 1_000);
    }

    #[test]
    fn pruning_never_runs_before_the_rollup_that_preserves_the_history() {
        let store = MemoryStore::default();
        let now = Timestamp::from_millis(40 * DAY);
        store
            .insert_events(&[event(0, 1_000, 400), event(39 * DAY, 100, 50)])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        let report = metrics.maintain_at(now).unwrap();

        assert_eq!(report.events_pruned, 1, "the 40-day-old event went");
        let summary = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(
            summary.operations, 2,
            "aggregates still remember the pruned event"
        );
        assert_eq!(summary.tokens_saved, 650);
    }

    #[test]
    fn retention_of_zero_days_keeps_everything() {
        let store = MemoryStore::default();
        store.insert_events(&[event(0, 10, 5)]).unwrap();

        let metrics = Metrics::with_settings(
            &store,
            None,
            Retention {
                raw_days: 0,
                hourly_days: 0,
            },
        );
        let report = metrics
            .maintain_at(Timestamp::from_millis(365 * DAY))
            .unwrap();

        assert_eq!(report.events_pruned, 0);
        assert_eq!(report.rollups_pruned, 0);
    }

    #[test]
    fn hourly_aggregates_are_pruned_and_daily_ones_survive() {
        let store = MemoryStore::default();
        store.insert_events(&[event(0, 1_000, 400)]).unwrap();

        let metrics = Metrics::with_settings(
            &store,
            None,
            Retention {
                raw_days: 30,
                hourly_days: 60,
            },
        );
        let report = metrics
            .maintain_at(Timestamp::from_millis(100 * DAY))
            .unwrap();

        assert_eq!(report.rollups_pruned, 1);
        let daily = metrics
            .timeseries(
                Granularity::Day,
                Window::new(Timestamp::from_millis(0), Timestamp::from_millis(DAY - 1)).unwrap(),
                None,
            )
            .unwrap();
        assert_eq!(daily.points[0].tokens_saved, 600);
    }

    #[test]
    fn a_summary_can_be_scoped_to_one_project() {
        let store = MemoryStore::default();
        store
            .insert_events(&[
                event(0, 1_000, 400).for_project("acme"),
                event(0, 200, 100).for_project("other"),
            ])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        metrics.maintain_at(Timestamp::from_millis(HOUR)).unwrap();

        let scoped = metrics.summary(Window::all_time(), Some("acme")).unwrap();
        assert_eq!(scoped.operations, 1);
        assert_eq!(scoped.tokens_saved, 600);

        let everything = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(everything.operations, 2);
    }

    #[test]
    fn a_timeseries_fills_quiet_buckets_rather_than_skipping_them() {
        let store = MemoryStore::default();
        store
            .insert_events(&[event(0, 100, 40), event(3 * HOUR, 100, 30)])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        metrics
            .maintain_at(Timestamp::from_millis(3 * HOUR))
            .unwrap();

        let series = metrics
            .timeseries(
                Granularity::Hour,
                Window::new(
                    Timestamp::from_millis(0),
                    Timestamp::from_millis(3 * HOUR + 1),
                )
                .unwrap(),
                None,
            )
            .unwrap();

        assert_eq!(series.points.len(), 4);
        assert_eq!(series.points[0].operations, 1);
        assert_eq!(series.points[1].operations, 0);
        assert_eq!(series.points[2].operations, 0);
        assert_eq!(series.points[3].operations, 1);
    }

    #[test]
    fn a_breakdown_attributes_savings_to_stages_and_operations() {
        use ctxc_core::optimization::Stage;

        let store = MemoryStore::default();
        let mut optimize = event(0, 20_000, 6_000);
        optimize.savings_by_stage.record(Stage::Filtering, 4_000);
        optimize
            .savings_by_stage
            .record(Stage::Deduplication, 2_000);
        optimize.savings_by_stage.record(Stage::Compression, 5_000);
        optimize.savings_by_stage.record(Stage::Selection, 3_000);

        store
            .insert_events(&[
                optimize,
                MetricEvent::new(Operation::Search, "cli").at(Timestamp::from_millis(0)),
            ])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        metrics.maintain_at(Timestamp::from_millis(HOUR)).unwrap();

        let breakdown = metrics.breakdown(Window::all_time(), None).unwrap();
        assert_eq!(breakdown.savings_by_stage.compression, 5_000);
        assert_eq!(breakdown.savings_by_stage.total(), 14_000);
        assert_eq!(breakdown.by_operation.len(), 2);
    }

    #[test]
    fn activity_is_newest_first() {
        let store = MemoryStore::default();
        store
            .insert_events(&[event(0, 10, 5), event(HOUR, 20, 10)])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        let recent = metrics.activity(None, 10).unwrap();
        assert_eq!(recent[0].recorded_at.as_millis(), HOUR);
    }

    #[test]
    fn refreshing_updates_aggregates_without_pruning() {
        let store = MemoryStore::default();
        store.insert_events(&[event(0, 1_000, 400)]).unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        let now = Timestamp::from_millis(90 * DAY);
        assert_eq!(metrics.refresh_at(now).unwrap(), 2, "one hour, one day");

        let summary = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(summary.tokens_saved, 600);
        assert_eq!(
            store.events_in(Window::all_time()).unwrap().len(),
            1,
            "a read must not delete anything, however old it is"
        );
    }

    #[test]
    fn maintenance_on_an_empty_database_does_nothing() {
        let store = MemoryStore::default();
        let metrics = Metrics::with_settings(&store, None, retention());
        assert!(metrics.maintain_at(Timestamp::now()).unwrap().did_nothing());
    }

    #[test]
    fn a_collector_flushes_into_the_store() {
        let store = MemoryStore::default();
        let collector = Collector::default();
        collector.record(event(0, 100, 40));
        collector.flush(&store).unwrap();

        assert_eq!(store.events.borrow().len(), 1);
    }
}
