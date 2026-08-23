//! Keeping the last of what the daemon said.
//!
//! Logs go to stderr, which is right for a process someone is watching and
//! useless for one started with `--detach`. So the daemon also keeps the most
//! recent records in memory, and serves them over `/v1/logs`.
//!
//! In memory rather than in a file, deliberately: CtxC should not start writing
//! to somebody's disk forever as a side effect of running in the background,
//! and a diagnostics panel only ever wants the recent past. The buffer is
//! bounded, the oldest record is dropped to make room, and how many were
//! dropped is reported rather than hidden.
//!
//! The buffer is process-wide because logging is: `tracing` has one subscriber
//! per process, so pretending each [`ApiState`](crate::state::ApiState) has its
//! own would be a fiction with two sources of truth.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// Records kept before the oldest starts falling out.
///
/// A few thousand lines is minutes of a busy daemon and hours of a quiet one,
/// for well under a megabyte.
pub const CAPACITY: usize = 2_000;

/// One thing the daemon said.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// Monotonic within a daemon run, so a client can ask for what is new.
    pub seq: u64,
    pub at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
    /// Structured fields other than the message, as they were rendered.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

/// A bounded window onto recent logging.
#[derive(Debug)]
pub struct LogBuffer {
    records: Mutex<VecDeque<LogRecord>>,
    next: AtomicU64,
    dropped: AtomicU64,
}

/// What a read of the buffer found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPage {
    pub records: Vec<LogRecord>,
    /// Records that fell out of the buffer before anyone read them.
    pub dropped: u64,
    /// How many the buffer holds when full.
    pub capacity: usize,
}

impl LogBuffer {
    fn new() -> LogBuffer {
        LogBuffer {
            records: Mutex::new(VecDeque::with_capacity(64)),
            next: AtomicU64::new(1),
            dropped: AtomicU64::new(0),
        }
    }

    /// Add a record, dropping the oldest if the buffer is full.
    pub fn push(&self, at: i64, level: &str, target: &str, message: String, fields: Fields) {
        let record = LogRecord {
            seq: self.next.fetch_add(1, Ordering::Relaxed),
            at,
            level: level.to_owned(),
            target: target.to_owned(),
            message,
            fields: fields.0,
        };

        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if records.len() == CAPACITY {
            records.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        records.push_back(record);
    }

    /// The most recent records, oldest first.
    ///
    /// `after` skips everything a client has already seen, so a diagnostics
    /// panel can poll without re-reading the same lines. `level` keeps only
    /// records at least as severe as the one named.
    pub fn page(&self, limit: usize, after: Option<u64>, level: Option<Severity>) -> LogPage {
        let records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let matching = records
            .iter()
            .filter(|record| after.map_or(true, |seq| record.seq > seq))
            .filter(|record| match level {
                None => true,
                Some(wanted) => {
                    Severity::parse(&record.level).is_some_and(|severity| severity.at_least(wanted))
                }
            });

        // Take the newest `limit`, then hand them back oldest first: a log is
        // read downwards, and reversing it in the browser would be the
        // dashboard deciding something the daemon already knows.
        let mut page: Vec<LogRecord> = matching.rev().take(limit).cloned().collect();
        page.reverse();

        LogPage {
            records: page,
            dropped: self.dropped.load(Ordering::Relaxed),
            capacity: CAPACITY,
        }
    }

    /// How many records are held right now.
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The process's log buffer.
pub fn buffer() -> &'static LogBuffer {
    static BUFFER: OnceLock<LogBuffer> = OnceLock::new();
    BUFFER.get_or_init(LogBuffer::new)
}

/// How severe a record is, for filtering.
///
/// Spelled out here rather than borrowed from `tracing` so that a client can
/// send `"warn"` in a query string and mean the same thing the daemon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn parse(value: &str) -> Option<Severity> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(Severity::Trace),
            "debug" => Some(Severity::Debug),
            "info" => Some(Severity::Info),
            "warn" | "warning" => Some(Severity::Warn),
            "error" => Some(Severity::Error),
            _ => None,
        }
    }

    fn at_least(self, floor: Severity) -> bool {
        self >= floor
    }
}

/// Fields collected from one event, other than its message.
#[derive(Debug, Default)]
pub struct Fields(BTreeMap<String, String>);

impl Fields {
    pub fn insert(&mut self, name: &str, value: String) {
        self.0.insert(name.to_owned(), value);
    }
}

/// The `tracing` layer that fills the buffer.
///
/// Installed alongside the stderr writer rather than instead of it: someone
/// running the daemon in a terminal should still see it work.
pub mod layer {
    use std::fmt;

    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::Layer;

    use super::{buffer, Fields};

    /// Capture events into the process's log buffer.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct Capture;

    /// A layer that records what the daemon says, for `/v1/logs`.
    pub fn capture() -> Capture {
        Capture
    }

    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            let metadata = event.metadata();
            let mut collected = Collector::default();
            event.record(&mut collected);

            buffer().push(
                ctxc_core::Timestamp::now().as_millis(),
                metadata.level().as_str(),
                metadata.target(),
                collected.message,
                collected.fields,
            );
        }
    }

    /// Pull the message and the fields out of an event.
    #[derive(Default)]
    struct Collector {
        message: String,
        fields: Fields,
    }

    impl Visit for Collector {
        fn record_str(&mut self, field: &Field, value: &str) {
            // Strings are recorded unquoted: a log line reads better as
            // `path=/src/main.rs` than as `path="/src/main.rs"`.
            self.put(field, value.to_owned());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.put(field, format!("{value:?}"));
        }
    }

    impl Collector {
        fn put(&mut self, field: &Field, value: String) {
            if field.name() == "message" {
                self.message = value;
            } else {
                self.fields.insert(field.name(), value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(buffer: &LogBuffer, level: &str, message: &str) {
        buffer.push(0, level, "ctxc", message.to_owned(), Fields::default());
    }

    #[test]
    fn records_come_back_oldest_first() {
        let buffer = LogBuffer::new();
        record(&buffer, "INFO", "first");
        record(&buffer, "INFO", "second");

        let page = buffer.page(10, None, None);
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.records[0].message, "first");
        assert_eq!(page.records[1].message, "second");
        assert_eq!(page.dropped, 0);
    }

    #[test]
    fn a_limit_keeps_the_newest() {
        let buffer = LogBuffer::new();
        for index in 0..5 {
            record(&buffer, "INFO", &format!("line {index}"));
        }

        let page = buffer.page(2, None, None);
        assert_eq!(page.records[0].message, "line 3");
        assert_eq!(page.records[1].message, "line 4");
    }

    #[test]
    fn a_client_can_ask_for_only_what_is_new() {
        let buffer = LogBuffer::new();
        record(&buffer, "INFO", "old");
        let seen = buffer.page(10, None, None).records[0].seq;
        record(&buffer, "INFO", "new");

        let page = buffer.page(10, Some(seen), None);
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].message, "new");
    }

    #[test]
    fn filtering_keeps_everything_at_least_as_severe() {
        let buffer = LogBuffer::new();
        record(&buffer, "DEBUG", "noise");
        record(&buffer, "WARN", "careful");
        record(&buffer, "ERROR", "broken");

        let page = buffer.page(10, None, Some(Severity::Warn));
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.records[0].message, "careful");
    }

    #[test]
    fn the_oldest_records_fall_out_and_are_counted() {
        let buffer = LogBuffer::new();
        for index in 0..(CAPACITY + 3) {
            record(&buffer, "INFO", &format!("line {index}"));
        }

        let page = buffer.page(CAPACITY, None, None);
        assert_eq!(page.records.len(), CAPACITY);
        assert_eq!(page.dropped, 3);
        assert_eq!(page.records[0].message, "line 3");
    }

    #[test]
    fn severity_names_are_the_ones_a_client_would_send() {
        assert_eq!(Severity::parse("warn"), Some(Severity::Warn));
        assert_eq!(Severity::parse("WARNING"), Some(Severity::Warn));
        assert_eq!(Severity::parse("nonsense"), None);
        assert!(Severity::Error.at_least(Severity::Warn));
        assert!(!Severity::Info.at_least(Severity::Warn));
    }
}
