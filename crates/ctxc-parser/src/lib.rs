//! Deterministic source parsing.
//!
//! Tree-sitter reads code the way a compiler front end would: no model, no
//! network, no guessing. Everything language specific lives either in a grammar
//! or in a query file, so support for a language is a self-contained addition.

pub mod error;
pub mod extract;
pub mod language;
pub mod resolve;

pub use error::{ParserError, Result};
pub use extract::ParserRegistry;
pub use language::Language;
pub use resolve::{resolve_import, FileLookup};
