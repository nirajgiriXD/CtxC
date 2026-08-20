//! What the metrics subsystem needs from storage.
//!
//! The trait lives here, next to the rules that use it, rather than in
//! `ctxc-store`: aggregation policy, retention policy and what a window means
//! are decisions, and decisions do not belong in an adapter. The SQLite
//! implementation is `ctxc_store::SqliteMetricsStore`.

use serde::{Deserialize, Serialize};

use ctxc_core::Timestamp;

use crate::collector::MetricsSink;
use crate::error::{MetricsError, Result};
use crate::event::{MetricEvent, Operation};
use crate::rollup::{Granularity, Rollup};

/// A half-open span of time, `[from, to)`.
///
/// Half-open so that adjacent windows neither overlap nor leave a gap, which is
/// what makes daily buckets add up to the same total as the hourly ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub from: Timestamp,
    pub to: Timestamp,
}

impl Window {
    /// Build a window, refusing one that runs backwards.
    pub fn new(from: Timestamp, to: Timestamp) -> Result<Self> {
        if from > to {
            return Err(MetricsError::BadWindow {
                start: from.to_rfc3339(),
                end: to.to_rfc3339(),
            });
        }
        Ok(Window { from, to })
    }

    /// The last `days` days, ending now.
    pub fn last_days(days: u32) -> Self {
        let to = Timestamp::now();
        let span = (days as i64).saturating_mul(86_400_000);
        Window {
            from: Timestamp::from_millis(to.as_millis().saturating_sub(span)),
            to,
        }
    }

    /// Everything ever recorded.
    pub fn all_time() -> Self {
        Window {
            from: Timestamp::from_millis(i64::MIN),
            to: Timestamp::from_millis(i64::MAX),
        }
    }

    /// Whether `at` falls inside.
    pub fn contains(&self, at: Timestamp) -> bool {
        at >= self.from && at < self.to
    }

    /// Widen to whole buckets, so a report never shows a partial bucket as if
    /// it were a whole one.
    pub fn aligned(&self, granularity: Granularity) -> Window {
        Window {
            from: granularity.bucket(self.from),
            to: self.to,
        }
    }
}

/// Which rows a report is asking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollupQuery {
    pub granularity: Granularity,
    pub window: Window,
    /// Restrict to one project. `None` means every project *and* the operations
    /// that belong to none.
    pub project_id: Option<String>,
    /// Restrict to one operation.
    pub operation: Option<Operation>,
}

impl RollupQuery {
    pub fn new(granularity: Granularity, window: Window) -> Self {
        RollupQuery {
            granularity,
            window,
            project_id: None,
            operation: None,
        }
    }

    pub fn for_project(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    pub fn for_project_opt(mut self, project_id: Option<String>) -> Self {
        self.project_id = project_id;
        self
    }

    pub fn for_operation(mut self, operation: Operation) -> Self {
        self.operation = Some(operation);
        self
    }
}

/// Persistence for events and aggregates.
pub trait MetricsStore: MetricsSink {
    /// Append events. All of them, or none.
    fn insert_events(&self, events: &[MetricEvent]) -> Result<()>;

    /// Every event recorded in `window`, oldest first.
    fn events_in(&self, window: Window) -> Result<Vec<MetricEvent>>;

    /// The most recent events, newest first — the live activity feed.
    fn recent_events(&self, project_id: Option<&str>, limit: usize) -> Result<Vec<MetricEvent>>;

    /// When the oldest surviving raw event was recorded, if any.
    fn earliest_event(&self) -> Result<Option<Timestamp>>;

    /// Insert or replace aggregate rows, keyed by bucket.
    ///
    /// Replacing rather than adding is what makes a rollup run repeatable: the
    /// current bucket is still filling, so it gets recomputed from its events
    /// every time until it stops changing.
    fn upsert_rollups(&self, rollups: &[Rollup]) -> Result<()>;

    /// The newest bucket already aggregated at this granularity.
    fn latest_rollup_bucket(&self, granularity: Granularity) -> Result<Option<Timestamp>>;

    /// Aggregate rows matching `query`, oldest bucket first.
    fn rollups(&self, query: &RollupQuery) -> Result<Vec<Rollup>>;

    /// Delete raw events recorded before `before`. Returns how many went.
    fn delete_events_before(&self, before: Timestamp) -> Result<usize>;

    /// Delete aggregates older than `before`. Returns how many went.
    fn delete_rollups_before(&self, granularity: Granularity, before: Timestamp) -> Result<usize>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_runs_backwards_is_refused() {
        let error = Window::new(Timestamp::from_millis(10), Timestamp::from_millis(5)).unwrap_err();
        assert!(matches!(error, MetricsError::BadWindow { .. }));
    }

    #[test]
    fn windows_are_half_open_so_they_tile() {
        let window = Window::new(Timestamp::from_millis(0), Timestamp::from_millis(10)).unwrap();
        assert!(window.contains(Timestamp::from_millis(0)));
        assert!(window.contains(Timestamp::from_millis(9)));
        assert!(
            !window.contains(Timestamp::from_millis(10)),
            "the end belongs to the next window, not this one"
        );
    }

    #[test]
    fn a_day_window_ends_now_and_starts_a_day_earlier() {
        let window = Window::last_days(7);
        let span = window.to.as_millis() - window.from.as_millis();
        assert_eq!(span, 7 * 86_400_000);
    }

    #[test]
    fn aligning_moves_the_start_back_to_a_bucket_boundary() {
        let window = Window::new(
            Timestamp::from_millis(1_700_000_000_000),
            Timestamp::from_millis(1_700_000_100_000),
        )
        .unwrap();

        let aligned = window.aligned(Granularity::Day);
        assert_eq!(aligned.from.to_rfc3339(), "2023-11-14T00:00:00.000Z");
        assert_eq!(aligned.to, window.to);
    }

    #[test]
    fn all_time_contains_everything_representable() {
        let window = Window::all_time();
        assert!(window.contains(Timestamp::from_millis(0)));
        assert!(window.contains(Timestamp::now()));
    }
}
