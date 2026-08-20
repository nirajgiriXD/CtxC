//! Semantic errors.

use thiserror::Error;

pub type Result<T, E = SemanticError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("no embedding provider named `{name}`")]
    UnknownProvider {
        name: String,
        available: Vec<&'static str>,
    },

    /// Stored vectors were produced by a different provider, or a different
    /// dimension count, and cannot be compared with new ones.
    #[error("stored embeddings came from `{stored}` and this build uses `{current}`")]
    ProviderChanged { stored: String, current: String },

    #[error(transparent)]
    Core(#[from] ctxc_core::Error),
}

impl SemanticError {
    /// A concrete next step, shown under `Try:` when the error is reported.
    pub fn hint(&self) -> Option<String> {
        match self {
            SemanticError::UnknownProvider { available, .. } => Some(format!(
                "set semantic.provider to one of: {}",
                available.join(", ")
            )),
            SemanticError::ProviderChanged { .. } => {
                Some("run `ctxc index --force` to rebuild them".into())
            }
            SemanticError::Core(err) => err.hint(),
        }
    }
}
