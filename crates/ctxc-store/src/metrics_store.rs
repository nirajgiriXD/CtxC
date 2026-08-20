//! SQLite implementation of the metrics store.
//!
//! The rules — what a bucket is, what gets pruned, what a window means — live
//! in `ctxc-metrics`. This is the adapter that puts rows in and takes them out.
//!
//! Aggregation is deliberately *not* done here as a `GROUP BY`. Doing it in
//! Rust means there is one definition of an hourly bucket rather than two that
//! can drift, and it can be tested without a database.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

use ctxc_core::optimization::SavingsByStage;
use ctxc_core::Timestamp;
use ctxc_metrics::collector::MetricsSink;
use ctxc_metrics::event::{CacheUse, MetricEvent, Operation, Outcome};
use ctxc_metrics::rollup::{Granularity, Rollup, RollupKey, Totals};
use ctxc_metrics::store::{MetricsStore, RollupQuery, Window};
use ctxc_metrics::{MetricsError, Result as MetricsResult};

use crate::db::Database;

/// SQLite-backed [`MetricsStore`].
pub struct SqliteMetricsStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteMetricsStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteMetricsStore {
            conn: database.connection(),
        }
    }
}

/// Begin a write transaction. `BEGIN IMMEDIATE` for the reason given on
/// [`crate::db::begin_write`]: a deferred transaction that reads before it
/// writes cannot upgrade its snapshot once another process has committed.
fn begin_write(conn: &Connection) -> rusqlite::Result<Transaction<'_>> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
}

/// Turn a database failure into something the metrics subsystem can carry.
fn storage(error: rusqlite::Error) -> MetricsError {
    MetricsError::Storage(error.to_string())
}

const EVENT_COLUMNS: &str = "project_id, operation, source, optimizer, recorded_at,
     input_tokens, output_tokens, duration_ms,
     saved_filtering, saved_deduplication, saved_compression, saved_selection,
     cache_hits, cache_lookups, outcome, detail, estimated";

fn read_event(row: &Row<'_>) -> rusqlite::Result<std::result::Result<MetricEvent, MetricsError>> {
    let operation: String = row.get(1)?;
    let outcome: String = row.get(14)?;

    let (operation, outcome) = match (operation.parse::<Operation>(), outcome.parse::<Outcome>()) {
        (Ok(operation), Ok(outcome)) => (operation, outcome),
        (Err(err), _) => return Ok(Err(MetricsError::CorruptRow(err.to_string()))),
        (_, Err(err)) => return Ok(Err(MetricsError::CorruptRow(err.to_string()))),
    };

    Ok(Ok(MetricEvent {
        project_id: row.get(0)?,
        operation,
        source: row.get(2)?,
        optimizer: row.get(3)?,
        recorded_at: Timestamp::from_millis(row.get(4)?),
        input_tokens: row.get::<_, i64>(5)?.clamp(0, u32::MAX as i64) as u32,
        output_tokens: row.get::<_, i64>(6)?.clamp(0, u32::MAX as i64) as u32,
        duration_ms: row.get::<_, i64>(7)?.max(0) as u64,
        savings_by_stage: SavingsByStage {
            filtering: row
                .get::<_, i64>(8)?
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            deduplication: row
                .get::<_, i64>(9)?
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            compression: row
                .get::<_, i64>(10)?
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            selection: row
                .get::<_, i64>(11)?
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        },
        cache: CacheUse::of(
            row.get::<_, i64>(12)?.clamp(0, u32::MAX as i64) as u32,
            row.get::<_, i64>(13)?.clamp(0, u32::MAX as i64) as u32,
        ),
        outcome,
        detail: row.get(15)?,
        estimated: row.get::<_, i64>(16)? != 0,
    }))
}

fn read_rollup(row: &Row<'_>) -> rusqlite::Result<std::result::Result<Rollup, MetricsError>> {
    let granularity: String = row.get(0)?;
    let operation: String = row.get(3)?;

    let (granularity, operation) = match (
        granularity.parse::<Granularity>(),
        operation.parse::<Operation>(),
    ) {
        (Ok(granularity), Ok(operation)) => (granularity, operation),
        (Err(err), _) => return Ok(Err(MetricsError::CorruptRow(err.to_string()))),
        (_, Err(err)) => return Ok(Err(MetricsError::CorruptRow(err.to_string()))),
    };

    Ok(Ok(Rollup {
        key: RollupKey {
            granularity,
            bucket_start: Timestamp::from_millis(row.get(1)?),
            project_id: row.get(2)?,
            operation,
        },
        totals: Totals {
            operations: row.get::<_, i64>(4)?.max(0) as u64,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            saved_filtering: row.get(7)?,
            saved_deduplication: row.get(8)?,
            saved_compression: row.get(9)?,
            saved_selection: row.get(10)?,
            duration_ms_total: row.get(11)?,
            duration_ms_max: row.get(12)?,
            errors: row.get::<_, i64>(13)?.max(0) as u64,
            degradations: row.get::<_, i64>(14)?.max(0) as u64,
            cache_hits: row.get::<_, i64>(15)?.max(0) as u64,
            cache_lookups: row.get::<_, i64>(16)?.max(0) as u64,
            estimated: row.get::<_, i64>(17)?.max(0) as u64,
        },
    }))
}

impl MetricsSink for SqliteMetricsStore<'_> {
    fn write_events(&self, events: &[MetricEvent]) -> MetricsResult<()> {
        self.insert_events(events)
    }
}

impl MetricsStore for SqliteMetricsStore<'_> {
    fn insert_events(&self, events: &[MetricEvent]) -> MetricsResult<()> {
        if events.is_empty() {
            return Ok(());
        }

        // One transaction: a flush is all-or-nothing, so a failure leaves the
        // buffer to be retried rather than half the batch written twice.
        let transaction = begin_write(self.conn).map_err(storage)?;
        {
            let mut statement = self
                .conn
                .prepare_cached(&format!(
                    "INSERT INTO metric_events ({EVENT_COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                             ?16, ?17)"
                ))
                .map_err(storage)?;

            for event in events {
                let savings = &event.savings_by_stage;
                statement
                    .execute(rusqlite::params![
                        event.project_id,
                        event.operation.as_str(),
                        event.source,
                        event.optimizer,
                        event.recorded_at.as_millis(),
                        event.input_tokens as i64,
                        event.output_tokens as i64,
                        event.duration_ms as i64,
                        savings.filtering as i64,
                        savings.deduplication as i64,
                        savings.compression as i64,
                        savings.selection as i64,
                        event.cache.hits as i64,
                        event.cache.lookups as i64,
                        event.outcome.as_str(),
                        event.detail,
                        event.estimated as i64,
                    ])
                    .map_err(storage)?;
            }
        }
        transaction.commit().map_err(storage)?;
        Ok(())
    }

    fn events_in(&self, window: Window) -> MetricsResult<Vec<MetricEvent>> {
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM metric_events
                 WHERE recorded_at >= ?1 AND recorded_at < ?2
                 ORDER BY recorded_at, id"
            ))
            .map_err(storage)?;

        let rows = statement
            .query_map([window.from.as_millis(), window.to.as_millis()], read_event)
            .map_err(storage)?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?
            .into_iter()
            .collect()
    }

    fn recent_events(
        &self,
        project_id: Option<&str>,
        limit: usize,
    ) -> MetricsResult<Vec<MetricEvent>> {
        let limit = limit as i64;
        let events = match project_id {
            Some(project) => {
                let mut statement = self
                    .conn
                    .prepare(&format!(
                        "SELECT {EVENT_COLUMNS} FROM metric_events
                         WHERE project_id = ?1 ORDER BY recorded_at DESC, id DESC LIMIT ?2"
                    ))
                    .map_err(storage)?;
                let rows = statement
                    .query_map(rusqlite::params![project, limit], read_event)
                    .map_err(storage)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(storage)?
            }
            None => {
                let mut statement = self
                    .conn
                    .prepare(&format!(
                        "SELECT {EVENT_COLUMNS} FROM metric_events
                         ORDER BY recorded_at DESC, id DESC LIMIT ?1"
                    ))
                    .map_err(storage)?;
                let rows = statement.query_map([limit], read_event).map_err(storage)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(storage)?
            }
        };

        events.into_iter().collect()
    }

    fn earliest_event(&self) -> MetricsResult<Option<Timestamp>> {
        let earliest: Option<i64> = self
            .conn
            .query_row("SELECT MIN(recorded_at) FROM metric_events", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage)?
            .flatten();
        Ok(earliest.map(Timestamp::from_millis))
    }

    fn upsert_rollups(&self, rollups: &[Rollup]) -> MetricsResult<()> {
        if rollups.is_empty() {
            return Ok(());
        }

        let transaction = begin_write(self.conn).map_err(storage)?;
        {
            // Replace rather than add: a bucket is recomputed from its events
            // every run until it stops changing, so adding would double-count
            // the bucket that was still filling last time.
            let mut statement = self
                .conn
                .prepare_cached(
                    "INSERT OR REPLACE INTO metric_rollups
                         (granularity, bucket_start, project_id, operation,
                          operations, input_tokens, output_tokens,
                          saved_filtering, saved_deduplication, saved_compression,
                          saved_selection, duration_ms_total, duration_ms_max,
                          errors, degradations, cache_hits, cache_lookups, estimated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                             ?14, ?15, ?16, ?17, ?18)",
                )
                .map_err(storage)?;

            for rollup in rollups {
                let totals = &rollup.totals;
                statement
                    .execute(rusqlite::params![
                        rollup.key.granularity.as_str(),
                        rollup.key.bucket_start.as_millis(),
                        rollup.key.project_id,
                        rollup.key.operation.as_str(),
                        totals.operations as i64,
                        totals.input_tokens,
                        totals.output_tokens,
                        totals.saved_filtering,
                        totals.saved_deduplication,
                        totals.saved_compression,
                        totals.saved_selection,
                        totals.duration_ms_total,
                        totals.duration_ms_max,
                        totals.errors as i64,
                        totals.degradations as i64,
                        totals.cache_hits as i64,
                        totals.cache_lookups as i64,
                        totals.estimated as i64,
                    ])
                    .map_err(storage)?;
            }
        }
        transaction.commit().map_err(storage)?;
        Ok(())
    }

    fn latest_rollup_bucket(&self, granularity: Granularity) -> MetricsResult<Option<Timestamp>> {
        let latest: Option<i64> = self
            .conn
            .query_row(
                "SELECT MAX(bucket_start) FROM metric_rollups WHERE granularity = ?1",
                [granularity.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?
            .flatten();
        Ok(latest.map(Timestamp::from_millis))
    }

    fn rollups(&self, query: &RollupQuery) -> MetricsResult<Vec<Rollup>> {
        let mut sql = String::from(
            "SELECT granularity, bucket_start, project_id, operation,
                    operations, input_tokens, output_tokens,
                    saved_filtering, saved_deduplication, saved_compression,
                    saved_selection, duration_ms_total, duration_ms_max,
                    errors, degradations, cache_hits, cache_lookups, estimated
             FROM metric_rollups
             WHERE granularity = ?1 AND bucket_start >= ?2 AND bucket_start < ?3",
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(query.granularity.as_str().to_owned()),
            Box::new(query.window.from.as_millis()),
            Box::new(query.window.to.as_millis()),
        ];
        if let Some(project) = &query.project_id {
            sql.push_str(&format!(" AND project_id = ?{}", params.len() + 1));
            params.push(Box::new(project.clone()));
        }
        if let Some(operation) = query.operation {
            sql.push_str(&format!(" AND operation = ?{}", params.len() + 1));
            params.push(Box::new(operation.as_str().to_owned()));
        }
        sql.push_str(" ORDER BY bucket_start, project_id, operation");

        let mut statement = self.conn.prepare(&sql).map_err(storage)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(params.iter()), read_rollup)
            .map_err(storage)?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?
            .into_iter()
            .collect()
    }

    fn delete_events_before(&self, before: Timestamp) -> MetricsResult<usize> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM metric_events WHERE recorded_at < ?1",
                [before.as_millis()],
            )
            .map_err(storage)?;
        Ok(removed)
    }

    fn delete_rollups_before(
        &self,
        granularity: Granularity,
        before: Timestamp,
    ) -> MetricsResult<usize> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM metric_rollups WHERE granularity = ?1 AND bucket_start < ?2",
                rusqlite::params![granularity.as_str(), before.as_millis()],
            )
            .map_err(storage)?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_metrics::{Collector, Metrics, Retention};

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 86_400_000;

    fn event(at: i64, input: u32, output: u32) -> MetricEvent {
        MetricEvent::new(Operation::Optimize, "stdin")
            .at(Timestamp::from_millis(at))
            .with_tokens(input, output)
    }

    fn retention() -> Retention {
        Retention {
            raw_days: 30,
            hourly_days: 90,
        }
    }

    #[test]
    fn events_round_trip_through_the_database() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);

        let original = event(1_700_000_000_000, 1_000, 250)
            .for_project("acme")
            .took(std::time::Duration::from_millis(42))
            .with_cache_hit(true)
            .degraded("index was stale");
        store
            .insert_events(std::slice::from_ref(&original))
            .unwrap();

        let read = store.events_in(Window::all_time()).unwrap();
        assert_eq!(read, vec![original]);
    }

    #[test]
    fn a_negative_stage_saving_survives_a_round_trip() {
        use ctxc_core::optimization::Stage;

        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);

        let mut original = event(0, 100, 90);
        original.savings_by_stage.record(Stage::Deduplication, -20);
        store.insert_events(&[original]).unwrap();

        let read = store.events_in(Window::all_time()).unwrap();
        assert_eq!(read[0].savings_by_stage.deduplication, -20);
    }

    #[test]
    fn an_event_with_no_project_reads_back_with_none() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        store.insert_events(&[event(0, 10, 5)]).unwrap();

        let read = store.events_in(Window::all_time()).unwrap();
        assert_eq!(read[0].project_id, None);
        assert!(
            !read[0].cache.is_used(),
            "no cache is not the same as a cache miss"
        );
    }

    #[test]
    fn events_are_filtered_by_a_half_open_window() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        store
            .insert_events(&[event(0, 1, 1), event(HOUR, 1, 1), event(2 * HOUR, 1, 1)])
            .unwrap();

        let window =
            Window::new(Timestamp::from_millis(0), Timestamp::from_millis(2 * HOUR)).unwrap();
        let read = store.events_in(window).unwrap();
        assert_eq!(read.len(), 2);
    }

    #[test]
    fn the_activity_feed_is_newest_first_and_can_be_scoped() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        store
            .insert_events(&[
                event(0, 1, 1).for_project("acme"),
                event(HOUR, 1, 1).for_project("other"),
                event(2 * HOUR, 1, 1).for_project("acme"),
            ])
            .unwrap();

        let all = store.recent_events(None, 10).unwrap();
        assert_eq!(all[0].recorded_at.as_millis(), 2 * HOUR);

        let scoped = store.recent_events(Some("acme"), 10).unwrap();
        assert_eq!(scoped.len(), 2);
        assert_eq!(store.recent_events(None, 1).unwrap().len(), 1);
    }

    #[test]
    fn rollups_replace_rather_than_accumulate() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        store.insert_events(&[event(0, 1_000, 400)]).unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        metrics.maintain_at(Timestamp::from_millis(HOUR)).unwrap();
        metrics.maintain_at(Timestamp::from_millis(HOUR)).unwrap();

        let summary = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(summary.operations, 1);
        assert_eq!(summary.tokens_saved, 600);
    }

    #[test]
    fn aggregates_outlive_the_events_they_came_from() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        store.insert_events(&[event(0, 20_000, 6_000)]).unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        let report = metrics
            .maintain_at(Timestamp::from_millis(60 * DAY))
            .unwrap();
        assert_eq!(report.events_pruned, 1);

        assert!(store.events_in(Window::all_time()).unwrap().is_empty());
        let summary = metrics.summary(Window::all_time(), None).unwrap();
        assert_eq!(summary.tokens_saved, 14_000);
    }

    #[test]
    fn a_rollup_query_can_be_narrowed_to_a_project_and_operation() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        store
            .insert_events(&[
                event(0, 100, 50).for_project("acme"),
                event(0, 100, 60).for_project("other"),
                MetricEvent::new(Operation::Search, "cli")
                    .at(Timestamp::from_millis(0))
                    .for_project("acme"),
            ])
            .unwrap();

        let metrics = Metrics::with_settings(&store, None, retention());
        metrics.maintain_at(Timestamp::from_millis(HOUR)).unwrap();

        let query = RollupQuery::new(Granularity::Hour, Window::all_time())
            .for_project("acme")
            .for_operation(Operation::Optimize);
        let rows = store.rollups(&query).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].totals.tokens_saved(), 50);
        assert_eq!(rows[0].key.project(), Some("acme"));
    }

    #[test]
    fn the_latest_bucket_is_reported_per_granularity() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);
        assert!(store
            .latest_rollup_bucket(Granularity::Hour)
            .unwrap()
            .is_none());
        assert!(store.earliest_event().unwrap().is_none());

        store
            .insert_events(&[event(0, 1, 1), event(25 * HOUR, 1, 1)])
            .unwrap();
        Metrics::with_settings(&store, None, retention())
            .maintain_at(Timestamp::from_millis(25 * HOUR))
            .unwrap();

        assert_eq!(
            store
                .latest_rollup_bucket(Granularity::Hour)
                .unwrap()
                .unwrap()
                .as_millis(),
            25 * HOUR
        );
        assert_eq!(
            store
                .latest_rollup_bucket(Granularity::Day)
                .unwrap()
                .unwrap()
                .as_millis(),
            DAY
        );
        assert_eq!(store.earliest_event().unwrap().unwrap().as_millis(), 0);
    }

    #[test]
    fn a_collector_flushes_into_sqlite() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteMetricsStore::new(&database);

        let collector = Collector::default();
        collector.record(event(0, 100, 40));
        collector.record(event(HOUR, 100, 40));
        assert_eq!(collector.flush(&store).unwrap(), 2);

        assert_eq!(store.events_in(Window::all_time()).unwrap().len(), 2);
    }

    #[test]
    fn an_unknown_operation_is_reported_rather_than_guessed_at() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection()
            .execute_batch(
                "INSERT INTO metric_events
                     (operation, source, recorded_at, input_tokens, output_tokens, duration_ms,
                      saved_filtering, saved_deduplication, saved_compression, saved_selection,
                      cache_hits, cache_lookups, outcome, estimated)
                 VALUES ('teleport', 'stdin', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 'success', 1)",
            )
            .unwrap();

        let store = SqliteMetricsStore::new(&database);
        let error = store.events_in(Window::all_time()).unwrap_err();
        assert!(matches!(error, MetricsError::CorruptRow(_)));
    }
}
