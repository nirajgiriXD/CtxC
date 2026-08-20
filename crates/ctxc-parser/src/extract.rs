//! Turning a parse tree into symbols and relationships.
//!
//! One tree-sitter query per language does the work. Capture names are the
//! contract between the `.scm` files and this module:
//!
//! ```text
//! @def.<kind>  the node a definition spans, with @name inside it
//! @import      an import, with @path naming the target
//! @call        a call, with @callee naming what is called
//! ```
//!
//! Nothing here knows any language specifics — that all lives in the queries,
//! which is what makes adding a language a self-contained change.

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use ctxc_graph::model::{FileIntelligence, Relationship, RelationshipKind, Symbol, SymbolKind};

use crate::error::{ParserError, Result};
use crate::language::Language;

/// Parses source files into intelligence, reusing compiled queries.
///
/// tree-sitter parsers and queries are expensive to build and cheap to reuse,
/// but they are not thread safe; one registry per worker is the intended usage.
pub struct ParserRegistry {
    parsers: HashMap<Language, Parser>,
    queries: HashMap<Language, Query>,
}

impl Default for ParserRegistry {
    fn default() -> Self {
        ParserRegistry::new()
    }
}

impl ParserRegistry {
    pub fn new() -> Self {
        ParserRegistry {
            parsers: HashMap::new(),
            queries: HashMap::new(),
        }
    }

    /// Extract symbols and relationships from `source`.
    pub fn parse(&mut self, language: Language, source: &str) -> Result<FileIntelligence> {
        self.prepare(language)?;

        let parser = self.parsers.get_mut(&language).expect("parser prepared");
        let tree = parser
            .parse(source, None)
            .ok_or(ParserError::Failed { language })?;
        let query = self.queries.get(&language).expect("query prepared");

        let mut symbols = Vec::new();
        let mut relationships = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());

        while let Some(matched) = matches.next() {
            let mut anchor: Option<(&str, Node)> = None;
            let mut name: Option<Node> = None;
            let mut path: Option<Node> = None;
            let mut callee: Option<Node> = None;

            for capture in matched.captures {
                match query.capture_names()[capture.index as usize] {
                    "name" => name = Some(capture.node),
                    "path" => path = Some(capture.node),
                    "callee" => callee = Some(capture.node),
                    other => anchor = Some((other, capture.node)),
                }
            }

            let Some((kind, node)) = anchor else {
                continue;
            };

            if let Some(definition) = kind.strip_prefix("def.") {
                if let Some(name) = name {
                    symbols.push(Symbol {
                        name: text(name, source).to_string(),
                        kind: symbol_kind(definition),
                        start_line: line_of(node),
                        end_line: node.end_position().row as u32 + 1,
                    });
                }
                continue;
            }

            match kind {
                "import" => {
                    if let Some(path) = path {
                        relationships.push(Relationship {
                            kind: RelationshipKind::Imports,
                            from_symbol: None,
                            target: unquote(text(path, source)).to_string(),
                            target_path: None,
                            line: line_of(node),
                        });
                    }
                }
                "call" => {
                    if let Some(callee) = callee {
                        relationships.push(Relationship {
                            kind: RelationshipKind::Calls,
                            from_symbol: enclosing_definition(node, source),
                            target: text(callee, source).to_string(),
                            target_path: None,
                            line: line_of(node),
                        });
                    }
                }
                _ => {}
            }
        }

        deduplicate_symbols(&mut symbols);
        deduplicate_relationships(&mut relationships);

        Ok(FileIntelligence {
            symbols,
            relationships,
        })
    }

    /// Build the parser and query for a language, once.
    fn prepare(&mut self, language: Language) -> Result<()> {
        if self.parsers.contains_key(&language) {
            return Ok(());
        }

        let grammar = language.grammar();
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|source| ParserError::Grammar {
                language,
                message: source.to_string(),
            })?;

        let query =
            Query::new(&grammar, language.query_source()).map_err(|source| ParserError::Query {
                language,
                message: source.to_string(),
            })?;

        self.parsers.insert(language, parser);
        self.queries.insert(language, query);
        Ok(())
    }
}

/// Map a `def.<kind>` capture suffix onto the shared vocabulary.
fn symbol_kind(capture: &str) -> SymbolKind {
    match capture {
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

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn line_of(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Strip the quotes tree-sitter includes in string literals.
fn unquote(text: &str) -> &str {
    text.trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
}

/// The named definition a node sits inside, so a call can be attributed to the
/// function that makes it rather than only to the file.
fn enclosing_definition(node: Node, source: &str) -> Option<String> {
    const DEFINITION_KINDS: [&str; 8] = [
        "function_item",
        "function_definition",
        "function_declaration",
        "method_declaration",
        "method_definition",
        "generator_function_declaration",
        "class_definition",
        "macro_definition",
    ];

    let mut current = node.parent();
    while let Some(parent) = current {
        if DEFINITION_KINDS.contains(&parent.kind()) {
            if let Some(name) = parent.child_by_field_name("name") {
                return Some(text(name, source).to_string());
            }
        }
        current = parent.parent();
    }
    None
}

/// Keep one symbol per name and position.
///
/// Overlapping query patterns are intentional — a Go `type_spec` matches both
/// the specific struct pattern and the general type pattern — so the more
/// specific kind wins rather than both being recorded.
fn deduplicate_symbols(symbols: &mut Vec<Symbol>) {
    symbols.sort_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| specificity(right.kind).cmp(&specificity(left.kind)))
    });
    symbols.dedup_by(|later, earlier| {
        later.name == earlier.name && later.start_line == earlier.start_line
    });
}

/// How specific a kind is, for resolving overlapping matches.
fn specificity(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::TypeAlias | SymbolKind::Variable => 0,
        _ => 1,
    }
}

/// Collapse repeated relationships, keeping the first occurrence.
fn deduplicate_relationships(relationships: &mut Vec<Relationship>) {
    let mut seen = std::collections::HashSet::new();
    relationships.retain(|edge| {
        seen.insert((
            edge.kind,
            edge.from_symbol.clone(),
            edge.target.clone(),
            edge.line,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(language: Language, source: &str) -> FileIntelligence {
        ParserRegistry::new().parse(language, source).unwrap()
    }

    fn names_of(intelligence: &FileIntelligence, kind: SymbolKind) -> Vec<String> {
        let mut names: Vec<String> = intelligence
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == kind)
            .map(|symbol| symbol.name.clone())
            .collect();
        names.sort();
        names
    }

    fn imports_of(intelligence: &FileIntelligence) -> Vec<String> {
        let mut targets: Vec<String> = intelligence
            .of_kind(RelationshipKind::Imports)
            .map(|edge| edge.target.clone())
            .collect();
        targets.sort();
        targets
    }

    #[test]
    fn rust_definitions_imports_and_calls() {
        let source = "\
use crate::database::Connection;
use std::collections::HashMap;

pub struct Session {
    pub id: String,
}

pub trait Authenticator {
    fn verify(&self) -> bool;
}

pub fn authenticate(token: &str) -> bool {
    let connection = Connection::open();
    connection.query(token)
}
";
        let intelligence = parse(Language::Rust, source);

        assert!(names_of(&intelligence, SymbolKind::Struct).contains(&"Session".to_string()));
        assert!(names_of(&intelligence, SymbolKind::Trait).contains(&"Authenticator".to_string()));
        assert!(names_of(&intelligence, SymbolKind::Function).contains(&"authenticate".to_string()));

        assert_eq!(
            imports_of(&intelligence),
            vec!["crate::database::Connection", "std::collections::HashMap"]
        );

        let calls: Vec<&Relationship> = intelligence.of_kind(RelationshipKind::Calls).collect();
        assert!(calls.iter().any(|edge| edge.target == "query"));
        assert!(
            calls
                .iter()
                .any(|edge| edge.from_symbol.as_deref() == Some("authenticate")),
            "calls are attributed to the function that makes them"
        );
    }

    #[test]
    fn typescript_definitions_and_imports() {
        let source = "\
import { Database } from './database';
import express from 'express';

export interface User {
  id: string;
}

export type UserId = string;

export class UserService {
  find(id: UserId): User | undefined {
    return this.db.lookup(id);
  }
}

export const authenticate = async (token: string) => {
  return verify(token);
};
";
        let intelligence = parse(Language::TypeScript, source);

        assert_eq!(
            names_of(&intelligence, SymbolKind::Interface),
            vec!["User".to_string()]
        );
        assert_eq!(
            names_of(&intelligence, SymbolKind::TypeAlias),
            vec!["UserId".to_string()]
        );
        assert_eq!(
            names_of(&intelligence, SymbolKind::Class),
            vec!["UserService".to_string()]
        );
        assert!(names_of(&intelligence, SymbolKind::Method).contains(&"find".to_string()));
        assert!(names_of(&intelligence, SymbolKind::Function).contains(&"authenticate".to_string()));

        assert_eq!(imports_of(&intelligence), vec!["./database", "express"]);
    }

    #[test]
    fn python_definitions_and_imports() {
        let source = "\
import os
from .database import Connection
from typing import Optional

class SessionStore:
    def get(self, key):
        return self.cache.lookup(key)

def authenticate(token):
    store = SessionStore()
    return store.get(token)
";
        let intelligence = parse(Language::Python, source);

        assert_eq!(
            names_of(&intelligence, SymbolKind::Class),
            vec!["SessionStore".to_string()]
        );
        assert_eq!(
            names_of(&intelligence, SymbolKind::Function),
            vec!["authenticate".to_string(), "get".to_string()]
        );
        assert_eq!(imports_of(&intelligence), vec![".database", "os", "typing"]);
    }

    #[test]
    fn go_definitions_and_imports() {
        let source = "\
package main

import (
    \"fmt\"
    \"github.com/example/db\"
)

type User struct {
    ID string
}

type Store interface {
    Find(id string) (*User, error)
}

type UserID string

func Authenticate(token string) bool {
    fmt.Println(token)
    return true
}

func (u *User) Name() string {
    return u.ID
}
";
        let intelligence = parse(Language::Go, source);

        assert_eq!(
            names_of(&intelligence, SymbolKind::Struct),
            vec!["User".to_string()]
        );
        assert_eq!(
            names_of(&intelligence, SymbolKind::Interface),
            vec!["Store".to_string()]
        );
        assert_eq!(
            names_of(&intelligence, SymbolKind::TypeAlias),
            vec!["UserID".to_string()],
            "the specific struct and interface kinds must win over the general one"
        );
        assert!(names_of(&intelligence, SymbolKind::Method).contains(&"Name".to_string()));
        assert_eq!(
            imports_of(&intelligence),
            vec!["fmt", "github.com/example/db"]
        );
    }

    #[test]
    fn javascript_handles_jsx_free_modules() {
        let source = "\
import { render } from './render.js';

export function mount(node) {
  return render(node);
}

export class Widget {
  update() {}
}
";
        let intelligence = parse(Language::JavaScript, source);

        assert!(names_of(&intelligence, SymbolKind::Function).contains(&"mount".to_string()));
        assert!(names_of(&intelligence, SymbolKind::Class).contains(&"Widget".to_string()));
        assert_eq!(imports_of(&intelligence), vec!["./render.js"]);
    }

    #[test]
    fn symbols_carry_their_line_range() {
        let intelligence = parse(
            Language::Rust,
            "\nfn first() {}\n\nfn second() {\n    ()\n}\n",
        );
        let second = intelligence
            .symbols
            .iter()
            .find(|symbol| symbol.name == "second")
            .expect("second is defined");

        assert_eq!(second.start_line, 4);
        assert_eq!(second.end_line, 6);
    }

    #[test]
    fn broken_source_still_yields_what_parsed() {
        // Tree-sitter recovers from errors; a half-written file must not stop
        // the rest of the repository from being indexed.
        let intelligence = parse(Language::Rust, "fn good() {}\n\nfn broken( {\n");
        assert!(intelligence
            .symbols
            .iter()
            .any(|symbol| symbol.name == "good"));
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(parse(Language::Rust, "").is_empty());
    }

    #[test]
    fn the_registry_reuses_prepared_languages() {
        let mut registry = ParserRegistry::new();
        registry.parse(Language::Rust, "fn a() {}").unwrap();
        let second = registry.parse(Language::Rust, "fn b() {}").unwrap();

        assert_eq!(second.symbols.len(), 1);
        assert_eq!(registry.parsers.len(), 1);
    }
}
