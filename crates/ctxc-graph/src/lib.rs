//! Code intelligence model and relationship graph.
//!
//! This crate holds what CtxC knows about a codebase — files, symbols and the
//! relationships between them — and the queries over it. It is deliberately
//! free of parsing and of storage: the parser produces these types, the store
//! persists them, and neither has to know about the other.

pub mod graph;
pub mod model;

pub use graph::{DependencyGraph, Edge};
pub use model::{
    FileFingerprint, FileId, FileIntelligence, IndexedFile, Relationship, RelationshipKind, Symbol,
    SymbolKind,
};
