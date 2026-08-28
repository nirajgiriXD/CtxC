//! SQLite persistence for CtxC.
//!
//! Everything above this crate talks to repository traits ([`ContextStore`]
//! today, more as later phases land) rather than to SQL, which keeps the
//! storage engine replaceable and the rest of the system testable against an
//! in-memory database.

pub mod context_store;
pub mod db;
pub mod embedding_store;
pub mod error;
pub mod index_store;
pub mod memory_store;
pub mod metrics_store;
pub mod migrations;
pub mod project_store;

pub use context_store::{ContextStore, SqliteContextStore};
pub use db::Database;
pub use embedding_store::{
    EmbeddingState, EmbeddingStore, Provider, SqliteEmbeddingStore, StoredEmbedding,
};
pub use error::{Result, StoreError};
pub use index_store::{IndexCounts, IndexStore, SqliteIndexStore, StoredFingerprint, SymbolHit};
pub use memory_store::{Memory, MemoryStore, SqliteMemoryStore};
pub use metrics_store::SqliteMetricsStore;
pub use migrations::latest_version as latest_schema_version;
pub use project_store::SqliteProjectStore;
