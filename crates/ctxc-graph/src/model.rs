//! What the index knows about code.
//!
//! These types are the contract between three crates that must not depend on
//! each other: the parser produces them, the store persists them, and the
//! engine turns one into the other. Nothing here parses, reads a file, or
//! touches a database.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use ctxc_core::Timestamp;

/// Row identifier for an indexed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileId(pub i64);

/// What makes a file "the same file" as last time.
///
/// Size and modification time answer that question without reading the file,
/// which is the difference between re-indexing a repository in milliseconds and
/// re-hashing every byte of it. The hash is the tie breaker: editors touch
/// mtimes without changing content, and a rebuild must not be triggered by a
/// timestamp alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
    pub size: u64,
    pub mtime_ms: i64,
    /// BLAKE3 of the file's bytes, rendered as hex.
    pub content_hash: String,
}

impl FileFingerprint {
    /// Whether metadata alone proves the file is unchanged.
    pub fn metadata_matches(&self, other: &FileFingerprint) -> bool {
        self.size == other.size && self.mtime_ms == other.mtime_ms
    }

    /// Whether the content is identical, whatever the metadata says.
    pub fn content_matches(&self, other: &FileFingerprint) -> bool {
        self.content_hash == other.content_hash
    }
}

/// A file that has been indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFile {
    pub id: FileId,
    /// Root the file was indexed under.
    pub root: PathBuf,
    /// Path relative to `root`, always with forward slashes.
    pub path: String,
    /// Detected language, if the file is one CtxC can parse.
    pub language: Option<String>,
    pub fingerprint: FileFingerprint,
    pub indexed_at: Timestamp,
}

/// Something a file defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 1-based line where the definition starts.
    pub start_line: u32,
    pub end_line: u32,
}

/// The kinds of definition CtxC records.
///
/// Deliberately a small, language-neutral vocabulary: a Rust `impl` block, a
/// Java class and a Python class are all `Class` here, because the retrieval
/// layer cares that a name is defined, not which keyword defined it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    TypeAlias,
    Constant,
    Variable,
    Module,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Constant => "constant",
            SymbolKind::Variable => "variable",
            SymbolKind::Module => "module",
        }
    }

    /// Inverse of [`SymbolKind::as_str`]. Unknown names become
    /// [`SymbolKind::Variable`], so a database written by a newer build stays
    /// readable.
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "function" => SymbolKind::Function,
            "method" => SymbolKind::Method,
            "class" => SymbolKind::Class,
            "struct" => SymbolKind::Struct,
            "enum" => SymbolKind::Enum,
            "trait" => SymbolKind::Trait,
            "interface" => SymbolKind::Interface,
            "type" => SymbolKind::TypeAlias,
            "constant" => SymbolKind::Constant,
            "module" => SymbolKind::Module,
            _ => SymbolKind::Variable,
        }
    }
}

/// A relationship between a file (or one of its symbols) and something else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub kind: RelationshipKind,
    /// Symbol the relationship starts from, when it is narrower than the file.
    pub from_symbol: Option<String>,
    /// What was written in the source: a module path, a symbol name.
    pub target: String,
    /// The file the target resolved to, when resolution succeeded. Unresolved
    /// targets are kept as written, because "imports something called express"
    /// is still worth knowing.
    pub target_path: Option<String>,
    pub line: u32,
}

/// The relationship vocabulary.
///
/// Kinds are open ended by design: later phases add `tests`, `documented_by`
/// and the rest without changing the schema, because they are stored by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    Imports,
    Exports,
    Calls,
    References,
    Extends,
    Implements,
    Contains,
    DependsOn,
    Tests,
    DocumentedBy,
    GeneratedFrom,
}

impl RelationshipKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RelationshipKind::Imports => "imports",
            RelationshipKind::Exports => "exports",
            RelationshipKind::Calls => "calls",
            RelationshipKind::References => "references",
            RelationshipKind::Extends => "extends",
            RelationshipKind::Implements => "implements",
            RelationshipKind::Contains => "contains",
            RelationshipKind::DependsOn => "depends_on",
            RelationshipKind::Tests => "tests",
            RelationshipKind::DocumentedBy => "documented_by",
            RelationshipKind::GeneratedFrom => "generated_from",
        }
    }

    /// Unknown names become [`RelationshipKind::References`], the weakest claim
    /// that can be made about two things being related.
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "imports" => RelationshipKind::Imports,
            "exports" => RelationshipKind::Exports,
            "calls" => RelationshipKind::Calls,
            "extends" => RelationshipKind::Extends,
            "implements" => RelationshipKind::Implements,
            "contains" => RelationshipKind::Contains,
            "depends_on" => RelationshipKind::DependsOn,
            "tests" => RelationshipKind::Tests,
            "documented_by" => RelationshipKind::DocumentedBy,
            "generated_from" => RelationshipKind::GeneratedFrom,
            _ => RelationshipKind::References,
        }
    }
}

/// Everything one parse produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIntelligence {
    pub symbols: Vec<Symbol>,
    pub relationships: Vec<Relationship>,
}

impl FileIntelligence {
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty() && self.relationships.is_empty()
    }

    /// Relationships of one kind.
    pub fn of_kind(&self, kind: RelationshipKind) -> impl Iterator<Item = &Relationship> {
        self.relationships
            .iter()
            .filter(move |edge| edge.kind == kind)
    }

    /// Symbol names defined in this file.
    pub fn symbol_names(&self) -> Vec<&str> {
        self.symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_compare_cheaply_then_exactly() {
        let stored = FileFingerprint {
            size: 100,
            mtime_ms: 1_700_000_000_000,
            content_hash: "abc".into(),
        };
        let touched = FileFingerprint {
            mtime_ms: 1_800_000_000_000,
            ..stored.clone()
        };

        assert!(stored.metadata_matches(&stored));
        assert!(
            !stored.metadata_matches(&touched),
            "a touched file fails the cheap check"
        );
        assert!(
            stored.content_matches(&touched),
            "but its content is unchanged, so nothing needs re-parsing"
        );
    }

    #[test]
    fn symbol_kind_names_round_trip() {
        for kind in [
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::Interface,
            SymbolKind::TypeAlias,
            SymbolKind::Constant,
            SymbolKind::Variable,
            SymbolKind::Module,
        ] {
            assert_eq!(SymbolKind::from_str_lossy(kind.as_str()), kind);
        }
    }

    #[test]
    fn relationship_kind_names_round_trip() {
        for kind in [
            RelationshipKind::Imports,
            RelationshipKind::Exports,
            RelationshipKind::Calls,
            RelationshipKind::References,
            RelationshipKind::Extends,
            RelationshipKind::Implements,
            RelationshipKind::Contains,
            RelationshipKind::DependsOn,
            RelationshipKind::Tests,
            RelationshipKind::DocumentedBy,
            RelationshipKind::GeneratedFrom,
        ] {
            assert_eq!(RelationshipKind::from_str_lossy(kind.as_str()), kind);
        }
    }

    #[test]
    fn unknown_stored_names_decay_instead_of_failing() {
        assert_eq!(SymbolKind::from_str_lossy("macro"), SymbolKind::Variable);
        assert_eq!(
            RelationshipKind::from_str_lossy("inspired_by"),
            RelationshipKind::References
        );
    }

    #[test]
    fn intelligence_filters_by_kind() {
        let intelligence = FileIntelligence {
            symbols: vec![Symbol {
                name: "authenticate".into(),
                kind: SymbolKind::Function,
                start_line: 10,
                end_line: 20,
            }],
            relationships: vec![
                Relationship {
                    kind: RelationshipKind::Imports,
                    from_symbol: None,
                    target: "./database".into(),
                    target_path: Some("src/database.ts".into()),
                    line: 1,
                },
                Relationship {
                    kind: RelationshipKind::Calls,
                    from_symbol: Some("authenticate".into()),
                    target: "query".into(),
                    target_path: None,
                    line: 12,
                },
            ],
        };

        assert_eq!(intelligence.of_kind(RelationshipKind::Imports).count(), 1);
        assert_eq!(intelligence.symbol_names(), vec!["authenticate"]);
        assert!(!intelligence.is_empty());
        assert!(FileIntelligence::default().is_empty());
    }
}
