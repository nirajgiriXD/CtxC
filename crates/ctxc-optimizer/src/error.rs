//! Errors raised by optimizers.

use thiserror::Error;

pub type Result<T, E = OptimizerError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum OptimizerError {
    /// An optimizer was handed material it does not claim to support. This is a
    /// routing bug rather than a user error.
    #[error("the {optimizer} optimizer does not handle {content_type} content")]
    Unsupported {
        optimizer: &'static str,
        content_type: &'static str,
    },
}

impl OptimizerError {
    pub fn hint(&self) -> Option<String> {
        match self {
            OptimizerError::Unsupported { .. } => {
                Some("this is an internal routing error; please report it".into())
            }
        }
    }
}
