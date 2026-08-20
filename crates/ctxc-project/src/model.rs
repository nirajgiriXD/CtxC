//! What a registered project is.
//!
//! The registry is the shared source of truth for the CLI, the daemon and,
//! later, the dashboard — so this type is deliberately plain data with no
//! behaviour that depends on where it is being read from.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use ctxc_core::Timestamp;

/// A project's stable identity.
///
/// Generated rather than derived from the path, so that moving or renaming a
/// directory keeps the project's history instead of creating a new one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    /// Number of characters in a rendered id.
    pub const LEN: usize = 16;

    /// Derive an id from a path and the moment it was registered.
    ///
    /// Including the timestamp means re-adding a removed project produces a new
    /// identity, which is what "removed" should mean.
    pub fn generate(path: &str, added_at: Timestamp) -> Self {
        let seed = format!("{path}\u{0}{}", added_at.as_millis());
        let hash = ctxc_core::id::content_hash(seed.as_bytes());
        ProjectId(hash[..Self::LEN].to_string())
    }

    /// Accept an id that already exists, from the database or a project file.
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        let valid = !trimmed.is_empty()
            && trimmed.len() <= 64
            && trimmed
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');

        valid.then(|| ProjectId(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether CtxC is currently looking after a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    /// Indexed and kept current.
    Active,
    /// Registered, but left alone until resumed.
    Paused,
}

impl ProjectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectStatus::Active => "active",
            ProjectStatus::Paused => "paused",
        }
    }

    /// Unknown values read back as paused: doing nothing is the safe reading of
    /// a status this build does not understand.
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "active" => ProjectStatus::Active,
            _ => ProjectStatus::Paused,
        }
    }

    pub fn is_active(self) -> bool {
        self == ProjectStatus::Active
    }
}

/// What CtxC noticed about a project when it was added.
///
/// Every field is a hint: detection informs ignore defaults and parser choice,
/// and is always overridable in the project's own configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detection {
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_manager: Option<String>,
    pub git: bool,
}

impl Detection {
    /// Whether anything at all was recognised.
    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
            && self.frameworks.is_empty()
            && self.package_manager.is_none()
            && !self.git
    }
}

/// A project CtxC is looking after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Canonical root, in the same form the index uses.
    pub path: String,
    pub status: ProjectStatus,
    pub added_at: Timestamp,
    pub last_indexed_at: Option<Timestamp>,
    pub detection: Detection,
}

impl Project {
    /// The project's directory.
    pub fn root(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }

    /// Whether the directory is still there.
    ///
    /// A registered project whose directory has been deleted is not an error;
    /// it is something `ctxc project list` should be able to say out loud.
    pub fn exists(&self) -> bool {
        self.root().is_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_for_the_same_registration() {
        let at = Timestamp::from_millis(1_700_000_000_000);
        assert_eq!(
            ProjectId::generate("/repo", at),
            ProjectId::generate("/repo", at)
        );
        assert_eq!(
            ProjectId::generate("/repo", at).as_str().len(),
            ProjectId::LEN
        );
    }

    #[test]
    fn different_projects_get_different_ids() {
        let at = Timestamp::from_millis(1_700_000_000_000);
        assert_ne!(
            ProjectId::generate("/repo-a", at),
            ProjectId::generate("/repo-b", at)
        );
        assert_ne!(
            ProjectId::generate("/repo", at),
            ProjectId::generate("/repo", Timestamp::from_millis(1_700_000_001_000)),
            "re-adding a project starts a new identity"
        );
    }

    #[test]
    fn ids_from_a_project_file_are_accepted_when_reasonable() {
        assert!(ProjectId::parse("acme-web").is_some());
        assert!(ProjectId::parse("  padded  ").is_some());
        assert!(ProjectId::parse("").is_none());
        assert!(ProjectId::parse("has spaces").is_none());
        assert!(ProjectId::parse(&"x".repeat(100)).is_none());
    }

    #[test]
    fn statuses_round_trip_and_unknown_ones_pause() {
        assert_eq!(
            ProjectStatus::from_str_lossy(ProjectStatus::Active.as_str()),
            ProjectStatus::Active
        );
        assert_eq!(
            ProjectStatus::from_str_lossy("archived"),
            ProjectStatus::Paused,
            "an unrecognised status must not cause work to happen"
        );
        assert!(ProjectStatus::Active.is_active());
    }

    #[test]
    fn detection_knows_when_it_found_nothing() {
        assert!(Detection::default().is_empty());
        assert!(!Detection {
            git: true,
            ..Detection::default()
        }
        .is_empty());
    }
}
