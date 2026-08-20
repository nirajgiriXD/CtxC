//! The optimization pipeline.
//!
//! ```text
//! Context -> Router -> Optimizer -> Budget -> Optimized context
//! ```
//!
//! The engine owns the sequencing and nothing else: optimizers live in
//! `ctxc-optimizer`, representation in `ctxc-context`, persistence in
//! `ctxc-store`. Nothing here reads the filesystem or a database, which is what
//! keeps the pipeline testable and reusable from the CLI, the daemon and, later,
//! the HTTP API.

pub mod analyze;
pub mod compile;
pub mod embed;
pub mod engine;
pub mod error;
pub mod index;
pub mod router;

pub use analyze::Analysis;
pub use compile::{Compilation, CompiledSection};
pub use embed::{ProjectEmbedder, QuerySimilarity, Similar};
pub use engine::{Engine, EngineOptions};
pub use error::{EngineError, Result};
pub use index::{IndexOptions, IndexReport, Indexer};
pub use router::Router;
