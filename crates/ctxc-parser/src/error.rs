//! Errors raised while parsing source code.

use thiserror::Error;

use crate::language::Language;

pub type Result<T, E = ParserError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum ParserError {
    #[error("the {} grammar could not be loaded", language.as_str())]
    Grammar { language: Language, message: String },

    #[error("the {} query is invalid", language.as_str())]
    Query { language: Language, message: String },

    #[error("failed to parse {} source", language.as_str())]
    Failed { language: Language },
}

impl ParserError {
    /// Grammar and query failures are build-time mistakes rather than anything
    /// a user can fix, so they say so plainly.
    pub fn hint(&self) -> Option<String> {
        match self {
            ParserError::Grammar { message, .. } | ParserError::Query { message, .. } => {
                Some(format!("this is a bug in CtxC: {message}"))
            }
            ParserError::Failed { .. } => None,
        }
    }
}
