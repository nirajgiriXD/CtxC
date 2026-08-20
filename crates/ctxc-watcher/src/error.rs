//! Errors raised while watching.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = WatchError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum WatchError {
    /// Watching is not possible here. Not a failure of the program: the caller
    /// falls back to periodic scanning and says so.
    #[error("cannot watch {}: {reason}", path.display())]
    Unavailable { path: PathBuf, reason: String },

    #[error("failed to read ignore rules for {}: {reason}", path.display())]
    Ignore { path: PathBuf, reason: String },
}

impl WatchError {
    pub fn hint(&self) -> Option<String> {
        match self {
            WatchError::Unavailable { .. } => Some(
                "CtxC will fall back to periodic scanning; on Linux, raising \
                 fs.inotify.max_user_watches usually fixes this"
                    .into(),
            ),
            WatchError::Ignore { .. } => Some("check the project's .gitignore".into()),
        }
    }
}
