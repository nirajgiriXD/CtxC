//! Errors raised while acquiring context.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = ContextError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("no such file: {}", path.display())]
    NotFound { path: PathBuf },

    #[error("{} is a directory", path.display())]
    IsDirectory { path: PathBuf },

    #[error("{} is not a directory", path.display())]
    NotADirectory { path: PathBuf },

    #[error("failed to {action}: {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("input is {size} bytes, which is over the {limit} byte limit")]
    TooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },

    #[error("{} is not text", describe(path))]
    Binary { path: Option<PathBuf> },
}

impl ContextError {
    /// A concrete next step, shown under `Try:`.
    pub fn hint(&self) -> Option<String> {
        match self {
            ContextError::NotFound { .. } => Some("check the path and try again".into()),
            ContextError::IsDirectory { .. } => {
                Some("pass a single file; whole-project input arrives with indexing".into())
            }
            ContextError::NotADirectory { .. } => Some(
                "pass a directory to index; a single file is optimized with `ctxc optimize`".into(),
            ),
            ContextError::TooLarge { .. } => {
                Some("split the input, or optimize the parts you actually need".into())
            }
            ContextError::Binary { .. } => Some(
                "CtxC optimizes text; binary files are left to the tools that read them".into(),
            ),
            ContextError::Io { .. } => None,
        }
    }
}

fn describe(path: &Option<PathBuf>) -> String {
    match path {
        Some(path) => path.display().to_string(),
        None => "the input".into(),
    }
}
