//! A project's own configuration.
//!
//! `.ctxc.toml` in a project root lets a repository carry its CtxC settings
//! with it, and — through `[project] id` — its identity, so that moving or
//! renaming the directory does not lose its history.
//!
//! The file is optional and CtxC never writes one on its own: registering a
//! project must not modify the project.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{ProjectError, Result};
use crate::model::ProjectId;

/// Names accepted for a project configuration file, in order of preference.
pub const CONFIG_NAMES: [&str; 2] = [".ctxc.toml", "ctxc.toml"];

/// What a project says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    #[serde(default)]
    pub project: ProjectSection,
    #[serde(default)]
    pub watch: ToggleSection,
    #[serde(default)]
    pub index: ToggleSection,
    #[serde(default)]
    pub budget: BudgetSection,
    #[serde(default)]
    pub ignore: IgnoreSection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    /// Identity that survives a rename.
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToggleSection {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSection {
    pub default: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IgnoreSection {
    #[serde(default)]
    pub patterns: Vec<String>,
}

impl ProjectFile {
    /// Read a project's configuration file, if it has one.
    pub fn load(root: &Path) -> Result<Option<(PathBuf, ProjectFile)>> {
        for name in CONFIG_NAMES {
            let path = root.join(name);
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(ProjectError::Io {
                        action: "read project configuration",
                        path,
                        source,
                    })
                }
            };

            let file: ProjectFile =
                toml::from_str(&text).map_err(|source| ProjectError::Config {
                    path: path.clone(),
                    source: Box::new(source),
                })?;
            return Ok(Some((path, file)));
        }
        Ok(None)
    }

    /// The identity this project claims, if it claims one.
    pub fn declared_id(&self) -> Option<ProjectId> {
        self.project.id.as_deref().and_then(ProjectId::parse)
    }

    /// Whether the project asks to be watched and indexed. Both default to on.
    pub fn watch_enabled(&self) -> bool {
        self.watch.enabled.unwrap_or(true)
    }

    pub fn index_enabled(&self) -> bool {
        self.index.enabled.unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let path = std::env::temp_dir()
                .join("ctxc-project-config-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Fixture(path)
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.0.join(name), contents).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_project_without_a_file_is_fine() {
        let fixture = Fixture::new("absent");
        assert!(ProjectFile::load(&fixture.0).unwrap().is_none());
    }

    #[test]
    fn the_documented_shape_parses() {
        let fixture = Fixture::new("full");
        fixture.write(
            ".ctxc.toml",
            "[project]\n\
             name = \"acme-web\"\n\
             id = \"acme-web-1\"\n\n\
             [watch]\n\
             enabled = true\n\n\
             [index]\n\
             enabled = true\n\n\
             [budget]\n\
             default = 32000\n\n\
             [ignore]\n\
             patterns = [\"generated/**\"]\n",
        );

        let (path, file) = ProjectFile::load(&fixture.0).unwrap().unwrap();
        assert!(path.ends_with(".ctxc.toml"));
        assert_eq!(file.project.name.as_deref(), Some("acme-web"));
        assert_eq!(file.declared_id().unwrap().as_str(), "acme-web-1");
        assert_eq!(file.budget.default, Some(32_000));
        assert_eq!(file.ignore.patterns, vec!["generated/**"]);
        assert!(file.watch_enabled() && file.index_enabled());
    }

    #[test]
    fn both_file_names_work_with_the_dotted_one_preferred() {
        let fixture = Fixture::new("names");
        fixture.write("ctxc.toml", "[project]\nname = \"plain\"\n");
        let (_, file) = ProjectFile::load(&fixture.0).unwrap().unwrap();
        assert_eq!(file.project.name.as_deref(), Some("plain"));

        fixture.write(".ctxc.toml", "[project]\nname = \"dotted\"\n");
        let (_, file) = ProjectFile::load(&fixture.0).unwrap().unwrap();
        assert_eq!(file.project.name.as_deref(), Some("dotted"));
    }

    #[test]
    fn an_empty_file_means_defaults() {
        let fixture = Fixture::new("empty");
        fixture.write(".ctxc.toml", "");

        let (_, file) = ProjectFile::load(&fixture.0).unwrap().unwrap();
        assert_eq!(file, ProjectFile::default());
        assert!(
            file.watch_enabled(),
            "watching is on unless it is turned off"
        );
        assert!(file.declared_id().is_none());
    }

    #[test]
    fn a_typo_is_an_error_rather_than_a_shrug() {
        let fixture = Fixture::new("typo");
        fixture.write(".ctxc.toml", "[watch]\nenbaled = true\n");

        let error = ProjectFile::load(&fixture.0).unwrap_err();
        assert!(matches!(error, ProjectError::Config { .. }));
        assert!(error.hint().unwrap().contains(".ctxc.toml"));
    }

    #[test]
    fn a_malformed_id_is_ignored_rather_than_fatal() {
        let fixture = Fixture::new("bad-id");
        fixture.write(".ctxc.toml", "[project]\nid = \"has spaces\"\n");

        let (_, file) = ProjectFile::load(&fixture.0).unwrap().unwrap();
        assert!(file.declared_id().is_none());
    }
}
