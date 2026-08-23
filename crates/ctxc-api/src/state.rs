//! What the API needs in order to answer.
//!
//! The database is shared behind a mutex rather than pooled: SQLite serializes
//! writers anyway, the daemon is a single local process, and one connection
//! with WAL enabled is both simpler and faster than a pool that has to be kept
//! honest.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use std::path::{Path, PathBuf};

use ctxc_core::{Config, Timestamp};
use ctxc_metrics::{Collector, MetricEvent};
use ctxc_store::Database;

use crate::events::{Broadcaster, StreamEvent};

/// The token a client must present.
///
/// Generated per daemon run and never stored in the database: it lives in the
/// lockfile, which is readable only by the user who started the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    /// Create a token from process- and clock-derived material.
    ///
    /// This is an access control boundary for a loopback socket, not a secret
    /// against a determined local attacker: anyone who can read the lockfile
    /// can already read the database it protects.
    pub fn generate() -> Self {
        let seed = format!(
            "{}-{}-{:?}",
            std::process::id(),
            Timestamp::now().as_millis(),
            std::time::SystemTime::now()
        );
        AccessToken(ctxc_core::id::content_hash(seed.as_bytes())[..32].to_string())
    }

    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        (!trimmed.is_empty() && trimmed.len() <= 128).then(|| AccessToken(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compare in constant time, so a wrong token cannot be narrowed down by
    /// timing the rejection.
    pub fn matches(&self, candidate: &str) -> bool {
        if self.0.len() != candidate.len() {
            return false;
        }
        self.0
            .bytes()
            .zip(candidate.bytes())
            .fold(0u8, |differences, (left, right)| {
                differences | (left ^ right)
            })
            == 0
    }
}

/// How a project is being observed, as the API reports it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WatchReport {
    pub project: String,
    pub path: String,
    /// False when CtxC fell back to periodic scanning.
    pub watching: bool,
    /// Why watching is unavailable, when it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
    /// Changes waiting to settle.
    pub pending_changes: usize,
}

/// Where this installation keeps its things.
///
/// Resolved once, by whatever started the daemon, and carried rather than
/// re-derived: a handler that resolved paths for itself could answer with a
/// different config file than the one the daemon actually loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Locations {
    pub config_file: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub database: PathBuf,
}

impl Locations {
    /// Resolve from the platform directories and the configuration that was
    /// loaded on top of them.
    pub fn resolve(config: &Config, paths: &ctxc_core::Paths) -> Locations {
        Locations {
            config_file: paths.config_file(),
            config_dir: paths.config_dir().to_path_buf(),
            data_dir: paths.data_dir().to_path_buf(),
            cache_dir: paths.cache_dir().to_path_buf(),
            database: config.database_path(paths),
        }
    }

    /// Use a configuration file other than the platform default, as `--config`
    /// does.
    pub fn with_config_file(mut self, path: impl AsRef<Path>) -> Locations {
        self.config_file = path.as_ref().to_path_buf();
        self
    }
}

/// Everything a request handler can reach.
#[derive(Clone)]
pub struct ApiState {
    inner: Arc<Inner>,
}

struct Inner {
    database: Mutex<Database>,
    config: Config,
    locations: Locations,
    token: AccessToken,
    started: Instant,
    started_at: Timestamp,
    /// What the watcher supervisor is doing, refreshed as it runs.
    watching: Mutex<Vec<WatchReport>>,
    /// Measurements taken by handlers, written by the supervisor's tick.
    metrics: Collector,
    /// The live event stream the dashboard subscribes to.
    events: Broadcaster,
    /// Set when a client asks the daemon to stop.
    shutdown: tokio::sync::Notify,
}

impl ApiState {
    pub fn new(
        database: Database,
        config: Config,
        token: AccessToken,
        locations: Locations,
    ) -> Self {
        ApiState {
            inner: Arc::new(Inner {
                database: Mutex::new(database),
                metrics: Collector::from_config(&config),
                events: Broadcaster::new(),
                locations,
                config,
                token,
                started: Instant::now(),
                started_at: Timestamp::now(),
                watching: Mutex::new(Vec::new()),
                shutdown: tokio::sync::Notify::new(),
            }),
        }
    }

    /// Run `work` against the database.
    ///
    /// Every query is small and local; the lock is held for the duration of one
    /// operation and never across an await.
    pub fn with_database<T>(&self, work: impl FnOnce(&Database) -> T) -> T {
        let database = self
            .inner
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        work(&database)
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The configuration the daemon is running with, and where it came from.
    ///
    /// This is what was loaded at startup, not what the file says now: an edit
    /// made through the API changes the file, and the daemon keeps working from
    /// what it read until it is restarted. Reporting the live values is the
    /// only way a settings screen can honestly say a restart is needed.
    pub fn locations(&self) -> &Locations {
        &self.inner.locations
    }

    pub fn token(&self) -> &AccessToken {
        &self.inner.token
    }

    pub fn started_at(&self) -> Timestamp {
        self.inner.started_at
    }

    pub fn uptime_ms(&self) -> u64 {
        self.inner.started.elapsed().as_millis() as u64
    }

    /// Replace what the API reports about watching.
    ///
    /// Announced only when it actually changed. The supervisor republishes on
    /// every tick, and a stream that repeated "still watching one project" four
    /// times a second would drown everything worth reading.
    pub fn set_watching(&self, reports: Vec<WatchReport>) {
        let mut watching = self
            .inner
            .watching
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if *watching == reports {
            return;
        }
        *watching = reports.clone();
        drop(watching);

        self.publish(StreamEvent::Watching { projects: reports });
    }

    /// What the supervisor last reported.
    pub fn watching(&self) -> Vec<WatchReport> {
        self.inner
            .watching
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Measure an operation. Never blocks and never fails, so a handler's
    /// answer does not depend on whether it could be counted.
    pub fn record(&self, event: MetricEvent) {
        // One call, two consumers: what gets counted and what gets announced
        // are the same fact, so they cannot drift apart.
        self.inner.events.publish_operation(event.clone());
        self.inner.metrics.record(event);
    }

    /// Announce something that is not an operation.
    pub fn publish(&self, event: StreamEvent) {
        self.inner.events.publish(event);
    }

    /// The live stream, for the WebSocket handler.
    pub fn events(&self) -> &Broadcaster {
        &self.inner.events
    }

    /// Write buffered measurements.
    ///
    /// Called from the supervisor's tick rather than from a handler: a request
    /// should not pay for someone else's metrics, and the daemon has a loop
    /// already running that can.
    pub fn flush_metrics(&self) {
        if self.inner.metrics.pending() == 0 {
            return;
        }
        self.with_database(|database| {
            let store = ctxc_store::SqliteMetricsStore::new(database);
            self.inner.metrics.flush_quietly(&store);
        });
    }

    /// Ask the daemon to shut down.
    pub fn request_shutdown(&self) {
        self.inner.shutdown.notify_waiters();
    }

    /// Resolves when a shutdown has been requested.
    pub async fn shutdown_requested(&self) {
        self.inner.shutdown.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_per_call() {
        let first = AccessToken::generate();
        let second = AccessToken::generate();
        assert_ne!(first, second);
        assert_eq!(first.as_str().len(), 32);
    }

    #[test]
    fn tokens_compare_by_value() {
        let token = AccessToken::generate();
        assert!(token.matches(token.as_str()));
        assert!(!token.matches("wrong"));
        assert!(!token.matches(&format!("{}x", token.as_str())));
    }

    #[test]
    fn tokens_from_a_lockfile_are_accepted_when_reasonable() {
        assert!(AccessToken::parse("abc123").is_some());
        assert!(AccessToken::parse("  padded  ").is_some());
        assert!(AccessToken::parse("").is_none());
        assert!(AccessToken::parse(&"x".repeat(200)).is_none());
    }

    #[tokio::test]
    async fn recording_counts_and_announces_the_same_fact() {
        use ctxc_metrics::Operation;

        let state = ApiState::new(
            Database::open_in_memory().unwrap(),
            Config::default(),
            AccessToken::generate(),
            Locations::default(),
        );
        let mut subscriber = state.events().subscribe();

        state.record(MetricEvent::new(Operation::Optimize, "stdin"));

        match subscriber.next().await.unwrap().event {
            StreamEvent::Operation { event } => assert_eq!(event.source, "stdin"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn watching_is_announced_only_when_it_changes() {
        let state = ApiState::new(
            Database::open_in_memory().unwrap(),
            Config::default(),
            AccessToken::generate(),
            Locations::default(),
        );
        let mut subscriber = state.events().subscribe();

        let reports = vec![WatchReport {
            project: "acme".into(),
            path: "/tmp/acme".into(),
            watching: true,
            degraded_reason: None,
            pending_changes: 0,
        }];

        state.set_watching(reports.clone());
        state.set_watching(reports.clone());
        state.set_watching(Vec::new());

        match subscriber.next().await.unwrap().event {
            StreamEvent::Watching { projects } => assert_eq!(projects.len(), 1),
            other => panic!("unexpected event: {other:?}"),
        }
        match subscriber.next().await.unwrap().event {
            StreamEvent::Watching { projects } => assert!(
                projects.is_empty(),
                "the repeated report must not have been announced"
            ),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn recorded_events_wait_for_a_flush_and_then_land() {
        use ctxc_metrics::store::{MetricsStore, Window};
        use ctxc_metrics::Operation;

        let state = ApiState::new(
            Database::open_in_memory().unwrap(),
            Config::default(),
            AccessToken::generate(),
            Locations::default(),
        );

        state.record(MetricEvent::new(Operation::Search, "api"));
        state.flush_metrics();

        state.with_database(|database| {
            let store = ctxc_store::SqliteMetricsStore::new(database);
            assert_eq!(store.events_in(Window::all_time()).unwrap().len(), 1);
        });
    }

    #[test]
    fn flushing_nothing_is_a_no_op() {
        let state = ApiState::new(
            Database::open_in_memory().unwrap(),
            Config::default(),
            AccessToken::generate(),
            Locations::default(),
        );
        state.flush_metrics();
    }

    #[test]
    fn state_exposes_the_database_and_the_clock() {
        let state = ApiState::new(
            Database::open_in_memory().unwrap(),
            Config::default(),
            AccessToken::generate(),
            Locations::default(),
        );

        let version = state.with_database(|database| database.schema_version().unwrap());
        assert!(version >= 5);
        assert!(state.started_at().as_millis() > 0);
    }
}
