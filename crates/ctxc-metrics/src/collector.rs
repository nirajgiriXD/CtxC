//! Recording, off the hot path.
//!
//! An optimization must never wait on a metrics write, and must never fail
//! because one did. So [`Collector::record`] only appends to a bounded
//! in-memory buffer: no lock is held across I/O, no error is returned, and the
//! database is touched only when someone calls [`Collector::flush`] — the CLI
//! once, after the command it was measuring has already produced its output;
//! the daemon on its maintenance tick.
//!
//! ```text
//! operation -> record() -> buffer -> flush() -> MetricsSink
//! ```
//!
//! The buffer is bounded, so a daemon that cannot write for an hour uses a
//! fixed amount of memory instead of an unbounded one. When it fills, the
//! oldest events go first and the number dropped is counted, because silently
//! discarding measurements would make every number downstream a guess.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::error::Result;
use crate::event::MetricEvent;

/// How many events are held before the oldest are dropped.
///
/// A busy daemon records a handful of events a second; this is minutes of
/// headroom for a few hundred kilobytes.
pub const DEFAULT_CAPACITY: usize = 4_096;

/// Somewhere flushed events can be written.
///
/// Implemented by the storage layer. It is a trait here so this crate stays
/// free of any particular database, and so tests can flush into a vector.
pub trait MetricsSink {
    /// Persist every event, or none of them.
    fn write_events(&self, events: &[MetricEvent]) -> Result<()>;
}

/// Buffers events until they can be written.
#[derive(Debug)]
pub struct Collector {
    enabled: bool,
    capacity: usize,
    buffer: Mutex<VecDeque<MetricEvent>>,
    dropped: AtomicU64,
}

impl Default for Collector {
    fn default() -> Self {
        Collector::new(true, DEFAULT_CAPACITY)
    }
}

impl Collector {
    pub fn new(enabled: bool, capacity: usize) -> Self {
        Collector {
            enabled,
            // A zero-capacity buffer would drop everything on the floor while
            // still claiming to be enabled; treat it as "use the default".
            capacity: capacity.max(1),
            buffer: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
        }
    }

    /// A collector that records nothing, for `metrics.enabled = false`.
    ///
    /// Switched off means switched off: nothing is buffered, so nothing can be
    /// written later by a component that did not check the setting.
    pub fn disabled() -> Self {
        Collector::new(false, 1)
    }

    /// Build one from configuration.
    pub fn from_config(config: &ctxc_core::Config) -> Self {
        Collector::new(config.metrics.enabled, DEFAULT_CAPACITY)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Record an event. Cheap, infallible, and never blocks on I/O.
    pub fn record(&self, event: MetricEvent) {
        if !self.enabled {
            return;
        }

        let mut buffer = self.lock();
        while buffer.len() >= self.capacity {
            buffer.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        buffer.push_back(event);
    }

    /// How many events were dropped because the buffer was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// How many events are waiting to be written.
    pub fn pending(&self) -> usize {
        self.lock().len()
    }

    /// Write everything buffered to `sink`.
    ///
    /// Returns the number of events written. On failure the events go back in
    /// the buffer, oldest first, so a database that is briefly busy costs a
    /// retry rather than the measurements themselves.
    pub fn flush(&self, sink: &dyn MetricsSink) -> Result<usize> {
        let events: Vec<MetricEvent> = {
            let mut buffer = self.lock();
            buffer.drain(..).collect()
        };
        if events.is_empty() {
            return Ok(0);
        }

        match sink.write_events(&events) {
            Ok(()) => Ok(events.len()),
            Err(err) => {
                self.requeue(events);
                Err(err)
            }
        }
    }

    /// Flush, reporting a failure through the log rather than to the caller.
    ///
    /// This is what a command whose real work already succeeded should use:
    /// losing metrics is worth a warning, never a non-zero exit code.
    pub fn flush_quietly(&self, sink: &dyn MetricsSink) {
        match self.flush(sink) {
            Ok(0) => {}
            Ok(written) => tracing::debug!(events = written, "metrics written"),
            Err(err) => tracing::warn!(error = %err, "could not write metrics"),
        }

        let dropped = self.dropped();
        if dropped > 0 {
            tracing::warn!(dropped, "metrics buffer overflowed; some events were lost");
        }
    }

    /// Put unwritten events back at the front, honouring the capacity.
    fn requeue(&self, events: Vec<MetricEvent>) {
        let mut buffer = self.lock();
        for event in events.into_iter().rev() {
            if buffer.len() >= self.capacity {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            buffer.push_front(event);
        }
    }

    /// A poisoned metrics buffer is not a reason to bring down the process it
    /// is measuring; the events inside are still perfectly readable.
    fn lock(&self) -> MutexGuard<'_, VecDeque<MetricEvent>> {
        self.buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MetricsError;
    use crate::event::Operation;
    use std::sync::Arc;

    /// A sink that remembers what it was given, and can be told to fail.
    #[derive(Default)]
    struct Recording {
        written: Mutex<Vec<MetricEvent>>,
        fail: bool,
    }

    impl MetricsSink for Recording {
        fn write_events(&self, events: &[MetricEvent]) -> Result<()> {
            if self.fail {
                return Err(MetricsError::Storage("disk on fire".into()));
            }
            self.written.lock().unwrap().extend_from_slice(events);
            Ok(())
        }
    }

    fn event(source: &str) -> MetricEvent {
        MetricEvent::new(Operation::Optimize, source)
    }

    #[test]
    fn recorded_events_are_written_on_flush() {
        let collector = Collector::default();
        collector.record(event("one"));
        collector.record(event("two"));
        assert_eq!(collector.pending(), 2);

        let sink = Recording::default();
        assert_eq!(collector.flush(&sink).unwrap(), 2);
        assert_eq!(collector.pending(), 0);
        assert_eq!(sink.written.lock().unwrap().len(), 2);
    }

    #[test]
    fn flushing_nothing_touches_nothing() {
        let sink = Recording {
            fail: true,
            ..Recording::default()
        };
        assert_eq!(Collector::default().flush(&sink).unwrap(), 0);
    }

    #[test]
    fn a_disabled_collector_records_nothing() {
        let collector = Collector::disabled();
        collector.record(event("one"));

        assert!(!collector.is_enabled());
        assert_eq!(collector.pending(), 0);
        assert_eq!(collector.dropped(), 0, "refusing is not dropping");
    }

    #[test]
    fn the_oldest_events_go_when_the_buffer_fills() {
        let collector = Collector::new(true, 2);
        for source in ["one", "two", "three"] {
            collector.record(event(source));
        }

        assert_eq!(collector.pending(), 2);
        assert_eq!(collector.dropped(), 1);

        let sink = Recording::default();
        collector.flush(&sink).unwrap();
        let written = sink.written.lock().unwrap();
        assert_eq!(written[0].source, "two");
        assert_eq!(written[1].source, "three");
    }

    #[test]
    fn a_failed_flush_keeps_the_events_in_order() {
        let collector = Collector::default();
        collector.record(event("one"));
        collector.record(event("two"));

        let failing = Recording {
            fail: true,
            ..Recording::default()
        };
        assert!(collector.flush(&failing).is_err());
        assert_eq!(collector.pending(), 2, "events survive a failed write");

        collector.record(event("three"));
        let sink = Recording::default();
        collector.flush(&sink).unwrap();

        let written = sink.written.lock().unwrap();
        let sources: Vec<&str> = written.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(sources, ["one", "two", "three"]);
    }

    #[test]
    fn recording_is_shareable_across_threads() {
        let collector = Arc::new(Collector::default());
        let threads: Vec<_> = (0..4)
            .map(|index| {
                let collector = Arc::clone(&collector);
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        collector.record(event(&format!("thread-{index}")));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(collector.pending(), 100);
    }

    #[test]
    fn configuration_switches_collection_off() {
        let mut config = ctxc_core::Config::default();
        assert!(Collector::from_config(&config).is_enabled());

        config.metrics.enabled = false;
        assert!(!Collector::from_config(&config).is_enabled());
    }
}
