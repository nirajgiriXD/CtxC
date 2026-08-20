//! What happened to a file.
//!
//! Filesystem notifications are noisy and platform specific. This is the
//! vocabulary CtxC reduces them to: a repository-relative path and what became
//! of it. Everything downstream — debouncing, indexing — works in these terms
//! and never sees a platform event.

use serde::{Deserialize, Serialize};

/// What became of a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeKind {
    /// The file appeared.
    Created,
    /// Its contents may have changed.
    Modified,
    /// It is gone.
    Deleted,
    /// It arrived here from somewhere else in the project.
    ///
    /// Kept distinct from delete-plus-create because the content is known not
    /// to have changed: the index can move the record instead of re-parsing.
    Renamed { from: String },
}

/// One thing that happened to one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Repository-relative path, with forward slashes.
    pub path: String,
    pub kind: ChangeKind,
}

impl Change {
    pub fn created(path: impl Into<String>) -> Change {
        Change {
            path: path.into(),
            kind: ChangeKind::Created,
        }
    }

    pub fn modified(path: impl Into<String>) -> Change {
        Change {
            path: path.into(),
            kind: ChangeKind::Modified,
        }
    }

    pub fn deleted(path: impl Into<String>) -> Change {
        Change {
            path: path.into(),
            kind: ChangeKind::Deleted,
        }
    }

    pub fn renamed(from: impl Into<String>, to: impl Into<String>) -> Change {
        Change {
            path: to.into(),
            kind: ChangeKind::Renamed { from: from.into() },
        }
    }

    /// Whether the path no longer exists.
    pub fn removes_path(&self) -> bool {
        matches!(self.kind, ChangeKind::Deleted)
    }

    /// Paths this change makes stale in the index.
    ///
    /// A rename makes two: the place the file left, and the place it arrived.
    pub fn affected_paths(&self) -> Vec<&str> {
        match &self.kind {
            ChangeKind::Renamed { from } => vec![from.as_str(), self.path.as_str()],
            _ => vec![self.path.as_str()],
        }
    }
}

/// Fold a later change into an earlier one for the same path.
///
/// Editors produce bursts — write, truncate, rename a temporary file over the
/// original — and what matters is the state the path ends up in, not the route
/// it took. These rules pick that end state.
pub fn coalesce(earlier: ChangeKind, later: ChangeKind) -> ChangeKind {
    match (earlier, later) {
        // A file that appeared and then changed is still a new file.
        (ChangeKind::Created, ChangeKind::Modified) => ChangeKind::Created,
        // A file that appeared and then vanished never mattered, but the index
        // may still hold an older record of that path, so deletion wins.
        (ChangeKind::Created, ChangeKind::Deleted) => ChangeKind::Deleted,
        // Deleted and then recreated is a modification, whatever the events
        // said: the path exists, and its contents are new.
        (ChangeKind::Deleted, ChangeKind::Created) => ChangeKind::Modified,
        (ChangeKind::Deleted, ChangeKind::Modified) => ChangeKind::Modified,
        // A rename keeps its origin: it is the only thing that records where
        // the content came from.
        (rename @ ChangeKind::Renamed { .. }, ChangeKind::Modified) => rename,
        (ChangeKind::Renamed { .. }, later) => later,
        (_, later) => later,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_writes_is_one_creation() {
        let kind = coalesce(ChangeKind::Created, ChangeKind::Modified);
        assert_eq!(coalesce(kind, ChangeKind::Modified), ChangeKind::Created);
    }

    #[test]
    fn a_file_written_then_removed_is_a_deletion() {
        assert_eq!(
            coalesce(ChangeKind::Created, ChangeKind::Deleted),
            ChangeKind::Deleted
        );
    }

    #[test]
    fn a_file_replaced_in_place_is_a_modification() {
        // Many editors save by deleting and rewriting; the file is still there.
        assert_eq!(
            coalesce(ChangeKind::Deleted, ChangeKind::Created),
            ChangeKind::Modified
        );
    }

    #[test]
    fn a_rename_keeps_its_origin_through_later_writes() {
        let rename = ChangeKind::Renamed {
            from: "old.rs".into(),
        };
        assert_eq!(
            coalesce(rename.clone(), ChangeKind::Modified),
            rename,
            "the index still needs to know where the content came from"
        );
    }

    #[test]
    fn a_renamed_file_that_is_then_deleted_is_deleted() {
        let rename = ChangeKind::Renamed {
            from: "old.rs".into(),
        };
        assert_eq!(coalesce(rename, ChangeKind::Deleted), ChangeKind::Deleted);
    }

    #[test]
    fn renames_affect_both_ends() {
        let change = Change::renamed("src/old.rs", "src/new.rs");
        assert_eq!(change.affected_paths(), vec!["src/old.rs", "src/new.rs"]);
        assert_eq!(Change::modified("a.rs").affected_paths(), vec!["a.rs"]);
    }

    #[test]
    fn only_deletions_remove_a_path() {
        assert!(Change::deleted("gone.rs").removes_path());
        assert!(!Change::created("new.rs").removes_path());
        assert!(!Change::renamed("a.rs", "b.rs").removes_path());
    }
}
