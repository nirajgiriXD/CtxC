//! Languages CtxC can read.
//!
//! Support is modular by construction: a language is an entry in this enum, a
//! grammar, and a query file. Adding one touches nothing else.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A language CtxC has a grammar and a query for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    JavaScript,
    TypeScript,
    /// TypeScript with JSX, which is a separate grammar.
    Tsx,
    Python,
    Go,
}

impl Language {
    /// Every supported language.
    pub const ALL: [Language; 6] = [
        Language::Rust,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::Python,
        Language::Go,
    ];

    /// Stable name used in storage and output.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::Python => "python",
            Language::Go => "go",
        }
    }

    /// The language a file extension implies.
    pub fn from_path(path: &Path) -> Option<Language> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        let language = match extension.as_str() {
            "rs" => Language::Rust,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "py" | "pyi" => Language::Python,
            "go" => Language::Go,
            _ => return None,
        };
        Some(language)
    }

    /// Parse a stored language name.
    pub fn from_str_opt(value: &str) -> Option<Language> {
        Language::ALL
            .iter()
            .copied()
            .find(|language| language.as_str() == value)
    }

    /// The tree-sitter grammar for this language.
    pub fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// The query describing what to extract from this language.
    pub fn query_source(self) -> &'static str {
        match self {
            Language::Rust => include_str!("queries/rust.scm"),
            Language::JavaScript => include_str!("queries/javascript.scm"),
            Language::TypeScript | Language::Tsx => include_str!("queries/typescript.scm"),
            Language::Python => include_str!("queries/python.scm"),
            Language::Go => include_str!("queries/go.scm"),
        }
    }

    /// File extensions this language claims, used when resolving imports.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Language::Rust => &["rs"],
            Language::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Language::TypeScript => &["ts", "tsx", "mts", "cts", "js", "jsx"],
            Language::Tsx => &["tsx", "ts", "jsx", "js"],
            Language::Python => &["py", "pyi"],
            Language::Go => &["go"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extensions_map_to_languages() {
        let cases = [
            ("src/main.rs", Language::Rust),
            ("src/app.ts", Language::TypeScript),
            ("src/App.tsx", Language::Tsx),
            ("src/index.js", Language::JavaScript),
            ("scripts/build.py", Language::Python),
            ("cmd/server/main.go", Language::Go),
        ];
        for (path, expected) in cases {
            assert_eq!(Language::from_path(&PathBuf::from(path)), Some(expected));
        }
    }

    #[test]
    fn unsupported_files_have_no_language() {
        for path in ["README.md", "Cargo.toml", "image.png", "Makefile"] {
            assert_eq!(Language::from_path(&PathBuf::from(path)), None);
        }
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(
            Language::from_path(&PathBuf::from("MAIN.RS")),
            Some(Language::Rust)
        );
    }

    #[test]
    fn names_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_str_opt(language.as_str()), Some(language));
        }
        assert_eq!(Language::from_str_opt("cobol"), None);
    }

    #[test]
    fn every_language_has_a_grammar_and_a_valid_query() {
        for language in Language::ALL {
            let grammar = language.grammar();
            tree_sitter::Query::new(&grammar, language.query_source())
                .unwrap_or_else(|err| panic!("{} query is invalid: {err}", language.as_str()));
        }
    }
}
