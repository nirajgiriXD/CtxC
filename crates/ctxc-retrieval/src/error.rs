//! Errors raised while retrieving context.

use thiserror::Error;

pub type Result<T, E = RetrievalError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum RetrievalError {
    #[error("failed to read the index")]
    Store(#[from] ctxc_store::StoreError),
}

impl RetrievalError {
    pub fn hint(&self) -> Option<String> {
        match self {
            RetrievalError::Store(err) => err.hint(),
        }
    }
}
