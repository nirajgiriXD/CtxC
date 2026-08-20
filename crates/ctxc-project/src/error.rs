//! Errors raised by the project registry.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = ProjectError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("no such directory: {}", path.display())]
    NoSuchDirectory { path: PathBuf },

    #[error("{} is not a directory", path.display())]
    NotADirectory { path: PathBuf },

    #[error("no project matches {reference}")]
    NotRegistered { reference: String },

    #[error("{reference} matches {} projects", matches.len())]
    Ambiguous {
        reference: String,
        matches: Vec<String>,
    },

    #[error("invalid project configuration: {}", path.display())]
    Config {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },

    #[error("failed to {action}: {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Raised by store implementations; the registry itself never constructs it.
    #[error("{0}")]
    Storage(String),
}

impl ProjectError {
    pub fn hint(&self) -> Option<String> {
        match self {
            ProjectError::NoSuchDirectory { .. } => Some("check the path and try again".into()),
            ProjectError::NotADirectory { .. } => {
                Some("a project is a directory; pass the folder that contains it".into())
            }
            ProjectError::NotRegistered { .. } => {
                Some("run `ctxc project list` to see what is registered".into())
            }
            ProjectError::Ambiguous { matches, .. } => Some(format!(
                "name the path or the id instead; candidates: {}",
                matches.join(", ")
            )),
            ProjectError::Config { path, .. } => Some(format!(
                "fix {}, or delete it to fall back to defaults",
                path.display()
            )),
            ProjectError::Io { .. } | ProjectError::Storage(_) => None,
        }
    }
}
