//! Watching a project directory.
//!
//! Wraps the platform watcher — inotify, FSEvents, ReadDirectoryChangesW — and
//! turns its events into [`Change`]s. Two things happen before anything else:
//!
//! * ignore rules are applied to the raw path, so a `node_modules` write costs
//!   one string comparison rather than a queue entry and an index lookup;
//! * rename pairs are matched up, because a move within a project is not a
//!   deletion followed by an unrelated creation.
//!
//! Watching can fail for reasons that are nobody's fault: inotify watch limits,
//! network filesystems, a directory that disappears. That is reported as a
//! degradation rather than an error, and the caller falls back to scanning.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RemoveKind, RenameMode};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use ctxc_context::walk::{IgnoreRules, WalkOptions};

use crate::change::{Change, ChangeKind};
use crate::error::{Result, WatchError};

/// How long a rename's two halves may be apart before they stop being a pair.
///
/// Platforms deliver the "from" and "to" of a move as separate events; they
/// arrive together in practice, and anything slower is not one move.
const RENAME_WINDOW: Duration = Duration::from_millis(200);

/// A watcher for one project.
///
/// Dropping it stops watching.
pub struct ProjectWatcher {
    root: PathBuf,
    changes: Receiver<Change>,
    /// Held so the platform watcher stays alive.
    _watcher: RecommendedWatcher,
}

impl ProjectWatcher {
    /// Start watching `root`.
    ///
    /// Returns an error the caller is expected to treat as "fall back to
    /// polling", not as a failure of the program.
    pub fn start(root: &Path, options: &WalkOptions) -> Result<ProjectWatcher> {
        let rules = IgnoreRules::for_root(root, options).map_err(|source| WatchError::Ignore {
            path: root.to_path_buf(),
            reason: source.to_string(),
        })?;

        let (sender, receiver) = mpsc::channel();
        let root_for_events = root.to_path_buf();
        let mut pairing = RenamePairing::default();

        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                match event {
                    Ok(event) => {
                        for change in translate(&event, &root_for_events, &rules, &mut pairing) {
                            // A closed receiver means the supervisor has gone; there
                            // is nothing useful left to do with the event.
                            if sender.send(change).is_err() {
                                return;
                            }
                        }
                    }
                    // The platform can drop events under load. Saying so is better
                    // than pretending nothing happened; the periodic scan is what
                    // recovers the missed work.
                    Err(err) => {
                        tracing::warn!(error = %err, "the filesystem watcher reported an error")
                    }
                }
            })
            .map_err(|source| WatchError::Unavailable {
                path: root.to_path_buf(),
                reason: source.to_string(),
            })?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|source| WatchError::Unavailable {
                path: root.to_path_buf(),
                reason: source.to_string(),
            })?;

        Ok(ProjectWatcher {
            root: root.to_path_buf(),
            changes: receiver,
            _watcher: watcher,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Take whatever has arrived, without waiting.
    pub fn drain(&self) -> Vec<Change> {
        self.changes.try_iter().collect()
    }

    /// Wait for the next change, up to `timeout`.
    pub fn next_change(&self, timeout: Duration) -> Option<Change> {
        self.changes.recv_timeout(timeout).ok()
    }

    /// A sender for tests and for feeding synthetic changes.
    pub fn channel() -> (Sender<Change>, Receiver<Change>) {
        mpsc::channel()
    }
}

/// Remembers the "from" half of a rename until its "to" arrives.
#[derive(Debug, Default)]
struct RenamePairing {
    pending: Option<(String, Instant)>,
}

impl RenamePairing {
    /// Record a path that was renamed away, returning any earlier unmatched one.
    fn depart(&mut self, path: String, now: Instant) -> Option<String> {
        let stale = self.take_expired(now);
        self.pending = Some((path, now));
        stale
    }

    /// Match an arrival against a recent departure.
    fn arrive(&mut self, now: Instant) -> Option<String> {
        match self.pending.take() {
            Some((from, at)) if now.duration_since(at) <= RENAME_WINDOW => Some(from),
            // Too old to be the other half of this move: it was a deletion.
            Some(_) => None,
            None => None,
        }
    }

    fn take_expired(&mut self, now: Instant) -> Option<String> {
        match self.pending.take() {
            Some((path, at)) if now.duration_since(at) > RENAME_WINDOW => Some(path),
            other => {
                self.pending = other;
                None
            }
        }
    }
}

/// Turn one platform event into zero or more changes.
fn translate(
    event: &notify::Event,
    root: &Path,
    rules: &IgnoreRules,
    pairing: &mut RenamePairing,
) -> Vec<Change> {
    let now = Instant::now();
    let mut changes = Vec::new();

    // Ignore rules are applied here, before anything is queued or looked up.
    let paths: Vec<String> = event
        .paths
        .iter()
        .filter_map(|path| relative(root, path))
        .filter(|path| !rules.is_ignored(path, false))
        .collect();

    if paths.is_empty() {
        return changes;
    }

    match event.kind {
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => {
            // A directory event says nothing about which files changed; the
            // files themselves generate their own events, except when a whole
            // tree is removed at once — which the deletion sweep handles by
            // path prefix.
            for path in paths {
                if matches!(event.kind, EventKind::Remove(_)) {
                    changes.push(Change::deleted(path));
                }
            }
        }
        EventKind::Create(_) => changes.extend(paths.into_iter().map(Change::created)),
        EventKind::Remove(_) => changes.extend(paths.into_iter().map(Change::deleted)),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            // Both halves in one event: the clearest case.
            if paths.len() >= 2 {
                changes.push(Change::renamed(paths[0].clone(), paths[1].clone()));
            } else {
                changes.extend(paths.into_iter().map(Change::modified));
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            for path in paths {
                if let Some(unmatched) = pairing.depart(path, now) {
                    changes.push(Change::deleted(unmatched));
                }
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            for path in paths {
                match pairing.arrive(now) {
                    Some(from) => changes.push(Change::renamed(from, path)),
                    None => changes.push(Change::created(path)),
                }
            }
        }
        EventKind::Modify(ModifyKind::Data(DataChange::Any) | ModifyKind::Any) => {
            changes.extend(paths.into_iter().map(Change::modified))
        }
        // Metadata-only changes — permissions, access times — cannot change
        // what a file says, so they are not worth re-indexing for.
        EventKind::Modify(ModifyKind::Metadata(_)) => {}
        EventKind::Modify(_) => changes.extend(paths.into_iter().map(Change::modified)),
        // Access events and anything the platform could not classify.
        EventKind::Access(_) | EventKind::Other | EventKind::Any => {}
    }

    changes
}

/// Convert an absolute path into the repository-relative form the index uses.
fn relative(root: &Path, path: &Path) -> Option<String> {
    ctxc_context::walk::relative_path(root, path)
        .filter(|relative| !relative.is_empty())
        // Paths outside the root are not this project's business.
        .or(None)
}

/// Whether a change concerns a path the index would hold.
pub fn is_interesting(change: &Change) -> bool {
    !matches!(change.kind, ChangeKind::Modified if change.path.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rules() -> IgnoreRules {
        let mut rules = IgnoreRules::default();
        rules.extend(["node_modules/", "target/", "*.tmp"]);
        rules
    }

    fn event(kind: EventKind, paths: &[&str], root: &Path) -> notify::Event {
        notify::Event {
            kind,
            paths: paths.iter().map(|path| root.join(path)).collect(),
            attrs: Default::default(),
        }
    }

    #[test]
    fn writes_become_modifications() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();
        let changes = translate(
            &event(
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                &["src/auth.ts"],
                &root,
            ),
            &root,
            &rules(),
            &mut pairing,
        );

        assert_eq!(changes, vec![Change::modified("src/auth.ts")]);
    }

    #[test]
    fn ignored_paths_never_become_changes() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        for path in ["node_modules/react/index.js", "target/debug/x.rs", "a.tmp"] {
            let changes = translate(
                &event(
                    EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                    &[path],
                    &root,
                ),
                &root,
                &rules(),
                &mut pairing,
            );
            assert!(changes.is_empty(), "{path} should have been ignored");
        }
    }

    #[test]
    fn creations_and_deletions_are_recognised() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        let created = translate(
            &event(EventKind::Create(CreateKind::File), &["new.rs"], &root),
            &root,
            &rules(),
            &mut pairing,
        );
        assert_eq!(created, vec![Change::created("new.rs")]);

        let removed = translate(
            &event(EventKind::Remove(RemoveKind::File), &["gone.rs"], &root),
            &root,
            &rules(),
            &mut pairing,
        );
        assert_eq!(removed, vec![Change::deleted("gone.rs")]);
    }

    #[test]
    fn a_rename_delivered_as_one_event_is_paired() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        let changes = translate(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &["src/old.rs", "src/new.rs"],
                &root,
            ),
            &root,
            &rules(),
            &mut pairing,
        );

        assert_eq!(changes, vec![Change::renamed("src/old.rs", "src/new.rs")]);
    }

    #[test]
    fn a_rename_delivered_as_two_events_is_paired() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        let departure = translate(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &["src/old.rs"],
                &root,
            ),
            &root,
            &rules(),
            &mut pairing,
        );
        assert!(
            departure.is_empty(),
            "nothing is decided until the other half"
        );

        let arrival = translate(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &["src/new.rs"],
                &root,
            ),
            &root,
            &rules(),
            &mut pairing,
        );
        assert_eq!(arrival, vec![Change::renamed("src/old.rs", "src/new.rs")]);
    }

    #[test]
    fn a_departure_with_no_arrival_is_eventually_a_deletion() {
        let mut pairing = RenamePairing::default();
        let start = Instant::now();

        assert!(pairing.depart("src/moved-away.rs".into(), start).is_none());
        // Another departure much later flushes the first as a plain deletion.
        let stale = pairing.depart(
            "src/other.rs".into(),
            start + RENAME_WINDOW + Duration::from_millis(50),
        );
        assert_eq!(stale.as_deref(), Some("src/moved-away.rs"));
    }

    #[test]
    fn an_arrival_long_after_a_departure_is_a_creation() {
        let mut pairing = RenamePairing::default();
        let start = Instant::now();
        pairing.depart("src/old.rs".into(), start);

        assert!(
            pairing
                .arrive(start + RENAME_WINDOW + Duration::from_millis(50))
                .is_none(),
            "two unrelated moves must not be stitched into one rename"
        );
    }

    #[test]
    fn metadata_changes_are_not_worth_re_indexing() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        let changes = translate(
            &event(
                EventKind::Modify(ModifyKind::Metadata(
                    notify::event::MetadataKind::Permissions,
                )),
                &["src/auth.ts"],
                &root,
            ),
            &root,
            &rules(),
            &mut pairing,
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn access_events_are_ignored() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        let changes = translate(
            &event(
                EventKind::Access(notify::event::AccessKind::Read),
                &["src/auth.ts"],
                &root,
            ),
            &root,
            &rules(),
            &mut pairing,
        );
        assert!(changes.is_empty(), "reading a file changes nothing");
    }

    #[test]
    fn paths_outside_the_project_are_dropped() {
        let root = PathBuf::from("/repo");
        let mut pairing = RenamePairing::default();

        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![PathBuf::from("/elsewhere/file.rs")],
            attrs: Default::default(),
        };
        assert!(translate(&event, &root, &rules(), &mut pairing).is_empty());
    }
}
