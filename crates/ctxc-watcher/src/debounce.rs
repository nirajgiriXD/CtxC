//! Debouncing.
//!
//! An editor saving a file produces a burst of events — often a dozen for one
//! Ctrl-S — and a build produces thousands. Reacting to each one would make
//! CtxC exactly the kind of background process people uninstall.
//!
//! Two rules govern the wait:
//!
//! * a path is released once it has been quiet for the debounce period, so a
//!   burst collapses into one change;
//! * a path is released regardless once it has been waiting for the maximum
//!   delay, so a file that is written continuously — a log, a watch-mode build
//!   output — still gets indexed rather than being starved forever.
//!
//! Time is passed in rather than read, which is what lets all of this be tested
//! deterministically instead of with sleeps.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::change::{coalesce, Change, ChangeKind};

/// How long a path must be quiet before it is released.
pub const DEFAULT_QUIET: Duration = Duration::from_millis(300);

/// How long a path may be held back while it keeps changing.
pub const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(5);

/// A change waiting to be released.
#[derive(Debug, Clone)]
struct Pending {
    kind: ChangeKind,
    first_seen: Instant,
    last_seen: Instant,
}

/// Collects filesystem changes and releases them in settled batches.
#[derive(Debug)]
pub struct Debouncer {
    quiet: Duration,
    max_delay: Duration,
    pending: HashMap<String, Pending>,
}

impl Debouncer {
    pub fn new(quiet: Duration, max_delay: Duration) -> Self {
        Debouncer {
            // A zero quiet period would defeat the purpose and make a save
            // burst into a burst of work.
            quiet: quiet.max(Duration::from_millis(1)),
            max_delay: max_delay.max(quiet),
            pending: HashMap::new(),
        }
    }

    /// A debouncer with the usual settings.
    pub fn with_quiet(quiet: Duration) -> Self {
        Debouncer::new(quiet, DEFAULT_MAX_DELAY)
    }

    /// Note that something happened.
    pub fn record(&mut self, change: Change, now: Instant) {
        match self.pending.get_mut(&change.path) {
            Some(pending) => {
                pending.kind = coalesce(pending.kind.clone(), change.kind);
                pending.last_seen = now;
            }
            None => {
                self.pending.insert(
                    change.path,
                    Pending {
                        kind: change.kind,
                        first_seen: now,
                        last_seen: now,
                    },
                );
            }
        }
    }

    /// Take everything that has settled.
    ///
    /// The batch is ordered by when each path was last touched, most recent
    /// first: if the work is cut short, what someone is actively editing is
    /// what got done.
    pub fn take_ready(&mut self, now: Instant) -> Vec<Change> {
        let ready: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| self.is_ready(pending, now))
            .map(|(path, _)| path.clone())
            .collect();

        let mut changes: Vec<(Instant, Change)> = ready
            .into_iter()
            .filter_map(|path| {
                let pending = self.pending.remove(&path)?;
                Some((
                    pending.last_seen,
                    Change {
                        path,
                        kind: pending.kind,
                    },
                ))
            })
            .collect();

        changes.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.path.cmp(&right.1.path))
        });
        changes.into_iter().map(|(_, change)| change).collect()
    }

    fn is_ready(&self, pending: &Pending, now: Instant) -> bool {
        now.duration_since(pending.last_seen) >= self.quiet
            || now.duration_since(pending.first_seen) >= self.max_delay
    }

    /// How long until something is ready, if anything is waiting.
    ///
    /// Lets a caller sleep exactly as long as it needs to rather than polling.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.pending
            .values()
            .map(|pending| {
                let quiet_at = pending.last_seen + self.quiet;
                let forced_at = pending.first_seen + self.max_delay;
                let due = quiet_at.min(forced_at);
                due.saturating_duration_since(now)
            })
            .min()
    }

    /// Release everything, settled or not. Used when shutting down.
    pub fn drain(&mut self) -> Vec<Change> {
        let mut changes: Vec<Change> = self
            .pending
            .drain()
            .map(|(path, pending)| Change {
                path,
                kind: pending.kind,
            })
            .collect();
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        changes
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn debouncer() -> Debouncer {
        Debouncer::new(Duration::from_millis(300), Duration::from_secs(5))
    }

    #[test]
    fn a_save_burst_becomes_one_change() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        // Fifteen events in two seconds, as an editor produces.
        for step in 0..15 {
            debouncer.record(
                Change::modified("src/auth.ts"),
                start + Duration::from_millis(step * 130),
            );
        }
        assert_eq!(debouncer.len(), 1);

        // Nothing is released while the writes keep coming.
        assert!(debouncer
            .take_ready(start + Duration::from_millis(15 * 130))
            .is_empty());

        let settled = debouncer.take_ready(start + Duration::from_millis(15 * 130 + 300));
        assert_eq!(settled, vec![Change::modified("src/auth.ts")]);
        assert!(debouncer.is_empty());
    }

    #[test]
    fn quiet_paths_are_released_and_busy_ones_are_not() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        debouncer.record(Change::modified("quiet.rs"), start);
        debouncer.record(Change::modified("busy.rs"), start);
        debouncer.record(
            Change::modified("busy.rs"),
            start + Duration::from_millis(250),
        );

        let released = debouncer.take_ready(start + Duration::from_millis(400));
        assert_eq!(released, vec![Change::modified("quiet.rs")]);
        assert_eq!(debouncer.len(), 1, "busy.rs is still settling");
    }

    #[test]
    fn a_continuously_written_file_is_not_starved() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        // A file written every 100ms forever would never go quiet.
        for step in 0..100 {
            debouncer.record(
                Change::modified("build.log"),
                start + Duration::from_millis(step * 100),
            );
        }

        let released = debouncer.take_ready(start + Duration::from_secs(6));
        assert_eq!(
            released,
            vec![Change::modified("build.log")],
            "the maximum delay must release it anyway"
        );
    }

    #[test]
    fn a_batch_leads_with_what_was_touched_most_recently() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        debouncer.record(Change::modified("old.rs"), start);
        debouncer.record(
            Change::modified("recent.rs"),
            start + Duration::from_millis(100),
        );

        let released = debouncer.take_ready(start + Duration::from_millis(500));
        assert_eq!(
            released
                .iter()
                .map(|change| change.path.as_str())
                .collect::<Vec<_>>(),
            vec!["recent.rs", "old.rs"],
            "what someone is editing right now comes first"
        );
    }

    #[test]
    fn events_for_one_path_are_coalesced_by_meaning() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        debouncer.record(Change::created("new.rs"), start);
        debouncer.record(Change::modified("new.rs"), start);
        debouncer.record(Change::modified("new.rs"), start);

        let released = debouncer.take_ready(start + Duration::from_millis(400));
        assert_eq!(released, vec![Change::created("new.rs")]);
    }

    #[test]
    fn a_file_created_and_deleted_in_a_burst_ends_deleted() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        debouncer.record(Change::created("temp.rs"), start);
        debouncer.record(
            Change::deleted("temp.rs"),
            start + Duration::from_millis(10),
        );

        let released = debouncer.take_ready(start + Duration::from_millis(400));
        assert_eq!(released, vec![Change::deleted("temp.rs")]);
    }

    #[test]
    fn the_next_deadline_is_when_the_soonest_path_settles() {
        let mut debouncer = debouncer();
        let start = Instant::now();
        assert!(debouncer.next_deadline(start).is_none());

        debouncer.record(Change::modified("a.rs"), start);
        let deadline = debouncer
            .next_deadline(start)
            .expect("something is waiting");
        assert_eq!(deadline, Duration::from_millis(300));

        // Once the quiet period has passed the deadline is zero: act now.
        assert_eq!(
            debouncer.next_deadline(start + Duration::from_millis(500)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn draining_releases_everything_unsettled() {
        let mut debouncer = debouncer();
        let start = Instant::now();
        debouncer.record(Change::modified("b.rs"), start);
        debouncer.record(Change::modified("a.rs"), start);

        let drained = debouncer.drain();
        assert_eq!(
            drained
                .iter()
                .map(|change| change.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.rs", "b.rs"]
        );
        assert!(debouncer.is_empty());
    }

    #[test]
    fn a_thousand_paths_coalesce_without_growing_unbounded() {
        let mut debouncer = debouncer();
        let start = Instant::now();

        // A build touching a thousand files, ten events each.
        for round in 0..10 {
            for file in 0..1_000 {
                debouncer.record(
                    Change::modified(format!("target/file-{file}.rs")),
                    start + Duration::from_millis(round * 10),
                );
            }
        }

        assert_eq!(debouncer.len(), 1_000, "one entry per path, not per event");
        assert_eq!(
            debouncer.take_ready(start + Duration::from_secs(1)).len(),
            1_000
        );
    }

    #[test]
    fn a_zero_quiet_period_is_refused() {
        let mut debouncer = Debouncer::new(Duration::ZERO, Duration::from_secs(1));
        let start = Instant::now();
        debouncer.record(Change::modified("a.rs"), start);

        assert!(
            debouncer.take_ready(start).is_empty(),
            "debouncing with no quiet period would not be debouncing"
        );
    }
}
