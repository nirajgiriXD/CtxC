//! Integration errors.
//!
//! Installing an integration writes into a file somebody else owns, so every
//! failure here names the exact path it was working on. "Permission denied" is
//! not useful; "permission denied writing CLAUDE.md" is.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = IntegrationError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("no integration named `{name}`")]
    Unknown { name: String },

    #[error("failed to {action} {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{} is not a directory", path.display())]
    NotADirectory { path: PathBuf },

    /// The file exists but CtxC will not touch it — it is not text.
    #[error("{} is not a text file", path.display())]
    NotText { path: PathBuf },

    /// A configuration file CtxC would have to rewrite is not valid JSON.
    #[error("{} is not valid JSON", path.display())]
    NotJson { path: PathBuf },

    #[error(transparent)]
    Core(#[from] ctxc_core::Error),
}

impl IntegrationError {
    /// A concrete next step, shown under `Try:` when the error is reported.
    pub fn hint(&self) -> Option<String> {
        match self {
            IntegrationError::Unknown { .. } => {
                Some("run `ctxc integrations list` to see the names".into())
            }
            IntegrationError::Io { path, source, .. }
                if source.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                Some(format!("check that {} is writable", path.display()))
            }
            IntegrationError::Io { .. } => None,
            IntegrationError::NotADirectory { .. } => {
                Some("pass a project directory with --path".into())
            }
            IntegrationError::NotText { path } => Some(format!(
                "move or delete {}, then install again",
                path.display()
            )),
            // Very often this is JSONC — comments and trailing commas — which
            // CtxC will not rewrite, because doing so would delete them.
            IntegrationError::NotJson { path } => Some(format!(
                "fix or remove the comments in {}, then install again",
                path.display()
            )),
            IntegrationError::Core(err) => err.hint(),
        }
    }
}
