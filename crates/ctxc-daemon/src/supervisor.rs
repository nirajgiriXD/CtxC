//! Keeping projects current.
//!
//! One loop looks after every active project: it watches where watching works,
//! polls where it does not, and applies settled batches of changes to the
//! index. It is deliberately a single blocking thread rather than a task per
//! project — the work is I/O bound and short, and one thread is far easier to
//! reason about than a pool that has to be drained on shutdown.
//!
//! ```text
//! registry -> watcher -> debouncer -> settled batch -> incremental index
//!                  \-> unavailable -> periodic full pass
//! ```
//!
//! Two safety nets sit under the watcher. A periodic full pass runs even when
//! watching works, because platforms drop events under load and a missed event
//! must not mean a permanently stale index. And any project whose watcher could
//! not start falls back to polling and says so, rather than silently going
//! quiet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ctxc_api::{ApiState, WatchReport};
use ctxc_core::Timestamp;
use ctxc_engine::index::{IndexOptions, Indexer};
use ctxc_metrics::{MetricEvent, Metrics, Operation};
use ctxc_project::{Project, ProjectId, Registry};
use ctxc_store::{SqliteIndexStore, SqliteMetricsStore, SqliteProjectStore};
use ctxc_watcher::{Debouncer, ProjectWatcher};

use crate::error::{DaemonError, Result};

/// How often the loop wakes when nothing is pending.
const TICK: Duration = Duration::from_millis(250);

/// How often the registry is re-read, so projects added while the daemon runs
/// are picked up without a restart.
const REGISTRY_REFRESH: Duration = Duration::from_secs(5);

/// How often buffered measurements are written.
///
/// Often enough that `ctxc metrics` in another terminal is close to current,
/// rarely enough that a busy daemon writes batches rather than single rows.
const METRICS_FLUSH: Duration = Duration::from_secs(5);

/// How often events are rolled up and old rows pruned. Aggregates only need to
/// be minutes fresh, and each pass reads every event since the last one.
const METRICS_MAINTENANCE: Duration = Duration::from_secs(300);

/// Whether work last done at `last` has come due again.
///
/// `map_or(true, ..)` rather than `is_none_or`, which needs a newer Rust than
/// this workspace commits to supporting.
fn due(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last.map_or(true, |last| now.duration_since(last) >= interval)
}

/// How the supervisor should behave.
#[derive(Debug, Clone)]
pub struct SupervisorOptions {
    /// Quiet period before a changed file is acted on.
    pub debounce: Duration,
    /// How often a watched project gets a full pass anyway, to catch anything
    /// the platform dropped.
    pub safety_scan: Duration,
    /// How often a project that cannot be watched is scanned.
    pub poll_interval: Duration,
    /// Whether to watch at all. When false, everything polls.
    pub watch_enabled: bool,
}

impl Default for SupervisorOptions {
    fn default() -> Self {
        SupervisorOptions {
            debounce: Duration::from_millis(300),
            safety_scan: Duration::from_secs(300),
            poll_interval: Duration::from_secs(30),
            watch_enabled: true,
        }
    }
}

/// One project the supervisor is looking after.
struct Watched {
    project: Project,
    root: PathBuf,
    watcher: Option<ProjectWatcher>,
    debouncer: Debouncer,
    /// Why watching is unavailable, when it is.
    degraded: Option<String>,
    /// When the last full pass ran. `None` means one is due now: a project
    /// CtxC has just started looking after may have changed while it was not
    /// running, and watching only reports what happens from now on.
    last_scan: Option<Instant>,
}

impl Watched {
    fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// How long until this project needs attention again.
    fn due_in(&self, now: Instant, options: &SupervisorOptions) -> Duration {
        let until_scan = match self.last_scan {
            None => Duration::ZERO,
            Some(last) => self
                .scan_interval(options)
                .saturating_sub(now.duration_since(last)),
        };

        match self.debouncer.next_deadline(now) {
            Some(until_batch) => until_batch.min(until_scan),
            None => until_scan,
        }
    }

    /// How often this project gets a full pass.
    ///
    /// A watched project still gets one, rarely: platforms drop events under
    /// load, and a missed event must not mean a permanently stale index.
    fn scan_interval(&self, options: &SupervisorOptions) -> Duration {
        if self.is_watching() {
            options.safety_scan
        } else {
            options.poll_interval
        }
    }

    /// Whether a full pass is due.
    fn scan_due(&self, now: Instant, options: &SupervisorOptions) -> bool {
        match self.last_scan {
            None => true,
            Some(last) => now.duration_since(last) >= self.scan_interval(options),
        }
    }

    fn report(&self) -> WatchReport {
        WatchReport {
            project: self.project.name.clone(),
            path: self.project.path.clone(),
            watching: self.is_watching(),
            degraded_reason: self.degraded.clone(),
            pending_changes: self.debouncer.len(),
        }
    }
}

/// Runs the observation loop until asked to stop.
pub struct Supervisor {
    state: ApiState,
    options: SupervisorOptions,
    watched: HashMap<ProjectId, Watched>,
    last_registry_read: Option<Instant>,
    last_metrics_flush: Option<Instant>,
    last_metrics_maintenance: Option<Instant>,
}

impl Supervisor {
    pub fn new(state: ApiState, options: SupervisorOptions) -> Self {
        Supervisor {
            state,
            options,
            watched: HashMap::new(),
            last_registry_read: None,
            last_metrics_flush: None,
            last_metrics_maintenance: None,
        }
    }

    /// Watch and index until `stop` is set.
    pub fn run(mut self, stop: Arc<AtomicBool>) {
        tracing::info!(
            watching = self.options.watch_enabled,
            "continuous mode started"
        );

        while !stop.load(Ordering::Relaxed) {
            let now = Instant::now();

            if let Err(err) = self.sync_registry(now) {
                tracing::warn!(error = %err, "could not read the project registry");
            }
            self.collect_events(now);

            if let Err(err) = self.apply_due_work(now) {
                tracing::warn!(error = %err, "could not update the index");
            }

            self.publish();
            self.keep_metrics(now);
            std::thread::sleep(self.sleep_for(Instant::now()));
        }

        // Whatever was measured in the last few seconds is still worth having.
        self.state.flush_metrics();
        tracing::info!("continuous mode stopped");
    }

    /// Write buffered measurements, and roll them up now and then.
    ///
    /// Both are the daemon's job rather than a request handler's: metrics
    /// upkeep must never sit in front of an operation someone is waiting for.
    fn keep_metrics(&mut self, now: Instant) {
        if due(self.last_metrics_flush, now, METRICS_FLUSH) {
            self.last_metrics_flush = Some(now);
            self.state.flush_metrics();
        }

        if !due(self.last_metrics_maintenance, now, METRICS_MAINTENANCE) {
            return;
        }
        self.last_metrics_maintenance = Some(now);

        let outcome = self.state.with_database(|database| {
            let store = SqliteMetricsStore::new(database);
            Metrics::new(&store, self.state.config()).maintain()
        });
        if let Err(err) = outcome {
            tracing::warn!(error = %err, "could not roll up metrics");
        }
    }

    /// How long to sleep: until the soonest thing is due, capped by the tick so
    /// a shutdown is never waited on for long.
    fn sleep_for(&self, now: Instant) -> Duration {
        self.watched
            .values()
            .map(|watched| watched.due_in(now, &self.options))
            .min()
            .unwrap_or(TICK)
            .clamp(Duration::from_millis(10), TICK)
    }

    /// Start watching projects that were added, and stop watching ones that
    /// were paused or removed.
    fn sync_registry(&mut self, now: Instant) -> Result<()> {
        // `map_or(true, ..)` rather than `is_none_or`, which needs a newer Rust
        // than this workspace commits to supporting.
        let due = self
            .last_registry_read
            .map_or(true, |last| now.duration_since(last) >= REGISTRY_REFRESH);
        if !due {
            return Ok(());
        }
        self.last_registry_read = Some(now);

        let active = self.state.with_database(|database| {
            let store = SqliteProjectStore::new(database);
            Registry::new(&store).active().map_err(DaemonError::Project)
        })?;

        let wanted: Vec<Project> = active
            .into_iter()
            .filter(|project| project.exists())
            .collect();

        self.watched
            .retain(|id, _| wanted.iter().any(|project| project.id == *id));

        for project in wanted {
            if self.watched.contains_key(&project.id) {
                continue;
            }
            self.begin_watching(project, now);
        }
        Ok(())
    }

    /// Start looking after one project.
    fn begin_watching(&mut self, project: Project, _now: Instant) {
        let root = PathBuf::from(&project.path);
        let (watcher, degraded) = if self.options.watch_enabled {
            match ProjectWatcher::start(&root, &Default::default()) {
                Ok(watcher) => (Some(watcher), None),
                // Watch limits, network filesystems, permissions: real and
                // common. Polling is worse, but it is not nothing.
                Err(err) => {
                    tracing::warn!(project = %project.name, error = %err, "falling back to periodic scanning");
                    (None, Some(err.to_string()))
                }
            }
        } else {
            (None, Some("watching is disabled in configuration".into()))
        };

        tracing::info!(project = %project.name, watching = watcher.is_some(), "observing project");

        // A project that has to be polled is a degradation someone should see
        // in metrics, not only in a log line that has already scrolled past.
        let event = MetricEvent::new(Operation::Watch, "start").for_project(project.id.as_str());
        self.state.record(match &degraded {
            Some(reason) => event.degraded(reason.clone()),
            None => event,
        });

        self.watched.insert(
            project.id.clone(),
            Watched {
                project,
                root,
                watcher,
                debouncer: Debouncer::with_quiet(self.options.debounce),
                degraded,
                last_scan: None,
            },
        );
    }

    /// Move whatever the platform reported into the debouncers.
    fn collect_events(&mut self, now: Instant) {
        for watched in self.watched.values_mut() {
            let Some(watcher) = &watched.watcher else {
                continue;
            };
            for change in watcher.drain() {
                watched.debouncer.record(change, now);
            }
        }
    }

    /// Apply settled batches, and run scans that have come due.
    fn apply_due_work(&mut self, now: Instant) -> Result<()> {
        let ids: Vec<ProjectId> = self.watched.keys().cloned().collect();

        for id in ids {
            let Some(watched) = self.watched.get_mut(&id) else {
                continue;
            };

            let batch = watched.debouncer.take_ready(now);
            let scan_due = watched.scan_due(now, &self.options);

            if batch.is_empty() && !scan_due {
                continue;
            }

            let root = watched.root.clone();
            let name = watched.project.name.clone();
            if scan_due {
                watched.last_scan = Some(now);
            }

            // Filled in inside the closure below, read once it has returned.
            let mut applied: Option<(usize, Duration)> = None;
            let outcome = self.state.with_database(|database| {
                let store = SqliteIndexStore::new(database);

                if !batch.is_empty() {
                    let started = Instant::now();
                    let report = database
                        .transaction(|| Indexer::new(&store).apply(&root, &batch))?;
                    applied = Some((batch.len(), started.elapsed()));
                    if report.changed_anything() {
                        tracing::debug!(
                            project = %name,
                            indexed = report.indexed,
                            removed = report.removed,
                            renamed = report.renamed,
                            "applied changes"
                        );
                    }
                }

                if scan_due {
                    let report = database.transaction(|| {
                        Indexer::new(&store).index(&root, &IndexOptions::default())
                    })?;
                    if !report.is_up_to_date() {
                        tracing::debug!(project = %name, indexed = report.indexed, "scan updated the index");
                    }
                }

                Ok::<(), DaemonError>(())
            });

            if let Err(err) = outcome {
                // One project failing must not stop the others.
                tracing::warn!(project = %name, error = %err, "could not update the index");
                self.state.record(
                    MetricEvent::new(Operation::Watch, "batch")
                        .for_project(id.as_str())
                        .failed(err.to_string()),
                );
                continue;
            }

            // One event per settled batch, carrying how many changes it held.
            // Watch event volume is what tells someone whether a project is
            // churning or quiet.
            if let Some((changes, elapsed)) = applied {
                self.state.record(
                    MetricEvent::new(Operation::Watch, "batch")
                        .for_project(id.as_str())
                        .with_tokens(changes.min(u32::MAX as usize) as u32, 0)
                        .took(elapsed),
                );
            }

            self.state.with_database(|database| {
                let store = SqliteProjectStore::new(database);
                Registry::new(&store)
                    .record_indexed(&id, Timestamp::now())
                    .map_err(DaemonError::Project)
            })?;
        }
        Ok(())
    }

    /// Publish what is being watched, so `ctxc status` can say so.
    fn publish(&self) {
        let mut reports: Vec<WatchReport> = self.watched.values().map(Watched::report).collect();
        reports.sort_by(|left, right| left.project.cmp(&right.project));
        self.state.set_watching(reports);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_api::AccessToken;
    use ctxc_core::Config;
    use ctxc_store::Database;

    fn state() -> ApiState {
        ApiState::new(
            Database::open_in_memory().unwrap(),
            Config::default(),
            AccessToken::generate(),
        )
    }

    fn watched(watching: bool, pending: usize) -> Watched {
        let mut debouncer = Debouncer::with_quiet(Duration::from_millis(300));
        for index in 0..pending {
            debouncer.record(
                ctxc_watcher::Change::modified(format!("file-{index}.rs")),
                Instant::now(),
            );
        }

        Watched {
            project: Project {
                id: ProjectId::parse("demo").unwrap(),
                name: "demo".into(),
                path: "/repo/demo".into(),
                status: ctxc_project::ProjectStatus::Active,
                added_at: Timestamp::now(),
                last_indexed_at: None,
                detection: Default::default(),
            },
            root: PathBuf::from("/repo/demo"),
            watcher: None,
            debouncer,
            degraded: (!watching).then(|| "inotify limit reached".to_string()),
            last_scan: Some(Instant::now()),
        }
    }

    #[test]
    fn a_degraded_project_says_why() {
        let report = watched(false, 0).report();

        assert!(!report.watching);
        assert_eq!(
            report.degraded_reason.as_deref(),
            Some("inotify limit reached")
        );
    }

    #[test]
    fn pending_changes_are_visible_before_they_are_applied() {
        let report = watched(false, 3).report();
        assert_eq!(report.pending_changes, 3);
    }

    #[test]
    fn a_polling_project_is_due_sooner_than_a_watched_one() {
        let options = SupervisorOptions::default();
        let now = Instant::now();

        let polling = watched(false, 0);
        let mut watching = watched(true, 0);
        watching.watcher = None; // no real watcher in a unit test
        watching.degraded = None;

        assert!(polling.due_in(now, &options) <= options.poll_interval);
        assert!(options.poll_interval < options.safety_scan);
    }

    #[test]
    fn a_pending_batch_brings_the_deadline_forward() {
        let options = SupervisorOptions::default();
        let now = Instant::now();

        let idle = watched(false, 0);
        let busy = watched(false, 1);

        assert!(
            busy.due_in(now, &options) < idle.due_in(now, &options),
            "waiting work must be acted on before the next scan"
        );
    }

    #[test]
    fn the_loop_sleeps_briefly_when_nothing_is_registered() {
        let supervisor = Supervisor::new(state(), SupervisorOptions::default());
        assert_eq!(supervisor.sleep_for(Instant::now()), TICK);
    }

    #[test]
    fn a_supervisor_with_nothing_to_do_stops_when_asked() {
        let supervisor = Supervisor::new(state(), SupervisorOptions::default());
        let stop = Arc::new(AtomicBool::new(true));

        // Returns rather than looping, because the flag is already set.
        supervisor.run(stop);
    }

    #[test]
    fn watching_can_be_switched_off_entirely() {
        let options = SupervisorOptions {
            watch_enabled: false,
            ..SupervisorOptions::default()
        };
        let mut supervisor = Supervisor::new(state(), options);

        let project = watched(false, 0).project;
        supervisor.begin_watching(project, Instant::now());

        let reports: Vec<WatchReport> = supervisor.watched.values().map(Watched::report).collect();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].watching);
        assert!(reports[0]
            .degraded_reason
            .as_deref()
            .unwrap()
            .contains("disabled"));
    }
}
