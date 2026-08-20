//! Finding the context that answers a question.
//!
//! Retrieval is hybrid and deterministic: full-text search over indexed file
//! content, symbol-name matching, and expansion through the dependency graph,
//! combined by a ranking function whose weights are configuration rather than
//! constants. No embedding model is involved, and CtxC works without one.
//!
//! Semantic retrieval belongs behind the same interface later; nothing here
//! assumes it will be absent, and nothing requires it to be present.

pub mod error;
pub mod query;
pub mod rank;
pub mod search;

pub use error::{Result, RetrievalError};
pub use query::Query;
pub use rank::{RankingWeights, Signals};
pub use search::{Reason, Retrieval, RetrievalOptions, RetrievedFile, Retriever, SimilaritySource};
