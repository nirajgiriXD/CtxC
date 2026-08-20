//! Errors raised by the engine.

use thiserror::Error;

use ctxc_optimizer::OptimizerError;

pub type Result<T, E = EngineError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("optimization failed")]
    Optimizer(#[from] OptimizerError),

    #[error("nothing to compile: no input was provided")]
    NoInput,

    #[error("failed to read the project")]
    Context(ctxc_context::ContextError),

    #[error("failed to parse a source file")]
    Parser(#[from] ctxc_parser::ParserError),

    #[error("failed to update the index")]
    Store(#[from] ctxc_store::StoreError),

    #[error("embeddings are not usable")]
    Semantic(#[from] ctxc_semantic::SemanticError),
}

impl EngineError {
    pub fn hint(&self) -> Option<String> {
        match self {
            EngineError::Optimizer(err) => err.hint(),
            EngineError::NoInput => Some("pass at least one file to compile".into()),
            EngineError::Context(err) => err.hint(),
            EngineError::Parser(err) => err.hint(),
            EngineError::Store(err) => err.hint(),
            EngineError::Semantic(err) => err.hint(),
        }
    }
}
