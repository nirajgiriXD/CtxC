//! Persistence for the code index.
//!
//! The unit of work is a file: its fingerprint, the symbols it defines and the
//! relationships it declares are written and replaced together. That is what
//! makes re-indexing one changed file safe — nothing else in the index is
//! touched, and a replayed index of the same file produces the same rows.

use rusqlite::{Connection, OptionalExtension};

use ctxc_core::Timestamp;
use ctxc_graph::model::{
    FileFingerprint, FileId, FileIntelligence, IndexedFile, Relationship, RelationshipKind,
    SymbolKind,
};
use ctxc_graph::Edge;

use crate::db::Database;
use crate::error::Result;

/// A symbol found by a search, with enough context to open it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolHit {
    pub path: String,
    pub language: Option<String>,
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: u32,
}

/// How much has been indexed under a root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexCounts {
    pub files: u64,
    pub symbols: u64,
    pub relationships: u64,
}

/// A file whose content matched a full-text query.
#[derive(Debug, Clone, PartialEq)]
pub struct TextHit {
    pub file: FileId,
    pub path: String,
    pub language: Option<String>,
    /// BM25 relevance. FTS5 reports smaller numbers as better matches; this is
    /// negated on the way out so that larger always means more relevant.
    pub relevance: f64,
    pub mtime_ms: i64,
    /// The stored content, for snippet extraction by the caller.
    pub content: String,
}

/// Read and write access to the code index.
pub trait IndexStore {
    /// The record for one file, if it has been indexed.
    fn file(&self, root: &str, path: &str) -> Result<Option<IndexedFile>>;

    /// Every path indexed under a root.
    fn file_paths(&self, root: &str) -> Result<Vec<String>>;

    /// Insert or update a file record, returning its id.
    fn upsert_file(
        &self,
        root: &str,
        path: &str,
        language: Option<&str>,
        fingerprint: &FileFingerprint,
        indexed_at: Timestamp,
    ) -> Result<FileId>;

    /// Replace everything derived from a file.
    fn replace_intelligence(&self, file: FileId, intelligence: &FileIntelligence) -> Result<()>;

    /// Move a file's record to a new path, keeping everything derived from it.
    ///
    /// A rename does not change content, so re-parsing would be wasted work;
    /// moving the record is what makes rename detection worth doing.
    fn rename_file(&self, root: &str, from: &str, to: &str) -> Result<bool>;

    /// Remove every file recorded under a directory prefix.
    ///
    /// Deleting a directory produces one event, not one per file, so the index
    /// has to sweep what was underneath it.
    fn delete_under(&self, root: &str, prefix: &str) -> Result<u64>;

    /// Remove a file and everything derived from it.
    fn delete_file(&self, root: &str, path: &str) -> Result<bool>;

    /// Symbols whose name contains `query`, case insensitively.
    fn search_symbols(&self, root: &str, query: &str, limit: u32) -> Result<Vec<SymbolHit>>;

    /// Make a file's text searchable, replacing whatever was indexed before.
    fn index_content(&self, file: FileId, path: &str, content: &str) -> Result<()>;

    /// The stored text of a file, if it was small enough to keep.
    fn file_content(&self, root: &str, path: &str) -> Result<Option<String>>;

    /// Files matching an FTS5 query, most relevant first.
    ///
    /// `query` is FTS5 syntax and must already be escaped by the caller; the
    /// store does not guess at what a user meant.
    fn search_text(&self, root: &str, query: &str, limit: u32) -> Result<Vec<TextHit>>;

    /// Relationships declared by one file.
    fn relationships(&self, root: &str, path: &str) -> Result<Vec<Relationship>>;

    /// File-to-file edges, for the dependency graph.
    fn edges(&self, root: &str) -> Result<Vec<Edge>>;

    /// Index size under a root.
    fn counts(&self, root: &str) -> Result<IndexCounts>;

    /// Record that a root finished indexing.
    fn record_index_run(&self, root: &str, at: Timestamp, counts: IndexCounts) -> Result<()>;

    /// When a root was last indexed.
    fn last_indexed_at(&self, root: &str) -> Result<Option<Timestamp>>;

    /// Roots that have been indexed.
    fn roots(&self) -> Result<Vec<String>>;
}

/// SQLite-backed [`IndexStore`].
pub struct SqliteIndexStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteIndexStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteIndexStore {
            conn: database.connection(),
        }
    }
}

impl IndexStore for SqliteIndexStore<'_> {
    fn file(&self, root: &str, path: &str) -> Result<Option<IndexedFile>> {
        let file = self
            .conn
            .prepare_cached(
                "SELECT id, language, size, mtime_ms, content_hash, indexed_at
                 FROM files WHERE root = ?1 AND path = ?2",
            )?
            .query_row([root, path], |row| {
                Ok(IndexedFile {
                    id: FileId(row.get(0)?),
                    root: root.into(),
                    path: path.to_owned(),
                    language: row.get(1)?,
                    fingerprint: FileFingerprint {
                        size: row.get::<_, i64>(2)? as u64,
                        mtime_ms: row.get(3)?,
                        content_hash: row.get(4)?,
                    },
                    indexed_at: Timestamp::from_millis(row.get(5)?),
                })
            })
            .optional()?;
        Ok(file)
    }

    fn file_paths(&self, root: &str) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT path FROM files WHERE root = ?1 ORDER BY path")?;
        let paths = statement
            .query_map([root], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(paths)
    }

    fn upsert_file(
        &self,
        root: &str,
        path: &str,
        language: Option<&str>,
        fingerprint: &FileFingerprint,
        indexed_at: Timestamp,
    ) -> Result<FileId> {
        // RETURNING gives back the id whether the row was inserted or updated,
        // which last_insert_rowid does not after an upsert.
        let id: i64 = self
            .conn
            .prepare_cached(
                "INSERT INTO files (root, path, language, size, mtime_ms, content_hash, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(root, path) DO UPDATE SET
                 language     = excluded.language,
                 size         = excluded.size,
                 mtime_ms     = excluded.mtime_ms,
                 content_hash = excluded.content_hash,
                 indexed_at   = excluded.indexed_at
             RETURNING id",
            )?
            .query_row(
                rusqlite::params![
                    root,
                    path,
                    language,
                    fingerprint.size as i64,
                    fingerprint.mtime_ms,
                    fingerprint.content_hash,
                    indexed_at.as_millis(),
                ],
                |row| row.get(0),
            )?;
        Ok(FileId(id))
    }

    fn replace_intelligence(&self, file: FileId, intelligence: &FileIntelligence) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM symbols WHERE file_id = ?1")?
            .execute([file.0])?;
        self.conn
            .prepare_cached("DELETE FROM relationships WHERE file_id = ?1")?
            .execute([file.0])?;

        {
            let mut insert = self.conn.prepare_cached(
                "INSERT INTO symbols (file_id, name, kind, start_line, end_line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for symbol in &intelligence.symbols {
                insert.execute(rusqlite::params![
                    file.0,
                    symbol.name,
                    symbol.kind.as_str(),
                    symbol.start_line,
                    symbol.end_line,
                ])?;
            }
        }

        let mut insert = self.conn.prepare_cached(
            "INSERT INTO relationships (file_id, kind, from_symbol, target, target_path, line)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for edge in &intelligence.relationships {
            insert.execute(rusqlite::params![
                file.0,
                edge.kind.as_str(),
                edge.from_symbol,
                edge.target,
                edge.target_path,
                edge.line,
            ])?;
        }
        Ok(())
    }

    fn rename_file(&self, root: &str, from: &str, to: &str) -> Result<bool> {
        // The search index keys on the file's rowid, so only its stored path
        // needs correcting; symbols and relationships hang off the same id and
        // do not move.
        let moved = self
            .conn
            .prepare_cached("UPDATE files SET path = ?3 WHERE root = ?1 AND path = ?2")?
            .execute([root, from, to])?;
        if moved > 0 {
            self.conn
                .prepare_cached(
                    "UPDATE file_search SET path = ?1
                     WHERE rowid IN (SELECT id FROM files WHERE root = ?2 AND path = ?1)",
                )?
                .execute([to, root])?;
        }
        Ok(moved > 0)
    }

    fn delete_under(&self, root: &str, prefix: &str) -> Result<u64> {
        let pattern = format!("{}/%", prefix.trim_end_matches('/'));
        self.conn
            .prepare_cached(
                "DELETE FROM file_search WHERE rowid IN
                     (SELECT id FROM files WHERE root = ?1 AND path LIKE ?2)",
            )?
            .execute([root, &pattern])?;
        let removed = self
            .conn
            .prepare_cached("DELETE FROM files WHERE root = ?1 AND path LIKE ?2")?
            .execute([root, &pattern])?;
        Ok(removed as u64)
    }

    fn delete_file(&self, root: &str, path: &str) -> Result<bool> {
        // The search index is a virtual table, so foreign key cascades do not
        // reach it; its row has to go first, while the file id is still there.
        self.conn
            .prepare_cached(
                "DELETE FROM file_search WHERE rowid IN
                     (SELECT id FROM files WHERE root = ?1 AND path = ?2)",
            )?
            .execute([root, path])?;
        let removed = self
            .conn
            .prepare_cached("DELETE FROM files WHERE root = ?1 AND path = ?2")?
            .execute([root, path])?;
        Ok(removed > 0)
    }

    fn search_symbols(&self, root: &str, query: &str, limit: u32) -> Result<Vec<SymbolHit>> {
        let pattern = format!("%{}%", escape_like(query));
        let mut statement = self.conn.prepare_cached(
            "SELECT files.path, files.language, symbols.name, symbols.kind, symbols.start_line
             FROM symbols
             JOIN files ON files.id = symbols.file_id
             WHERE files.root = ?1 AND symbols.name LIKE ?2 ESCAPE '\\'
             ORDER BY
                 -- Exact matches first, then prefixes, then the rest.
                 CASE
                     WHEN symbols.name = ?3 THEN 0
                     WHEN symbols.name LIKE ?4 ESCAPE '\\' THEN 1
                     ELSE 2
                 END,
                 length(symbols.name),
                 files.path,
                 symbols.start_line
             LIMIT ?5",
        )?;

        let hits = statement
            .query_map(
                rusqlite::params![
                    root,
                    pattern,
                    query,
                    format!("{}%", escape_like(query)),
                    limit
                ],
                |row| {
                    Ok(SymbolHit {
                        path: row.get(0)?,
                        language: row.get(1)?,
                        name: row.get(2)?,
                        kind: SymbolKind::from_str_lossy(&row.get::<_, String>(3)?),
                        start_line: row.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<SymbolHit>>>()?;
        Ok(hits)
    }

    fn index_content(&self, file: FileId, path: &str, content: &str) -> Result<()> {
        // FTS5 has no upsert, so a re-index replaces the row outright.
        self.conn
            .prepare_cached("DELETE FROM file_search WHERE rowid = ?1")?
            .execute([file.0])?;
        self.conn
            .prepare_cached("INSERT INTO file_search (rowid, path, content) VALUES (?1, ?2, ?3)")?
            .execute(rusqlite::params![file.0, path, content])?;
        Ok(())
    }

    fn file_content(&self, root: &str, path: &str) -> Result<Option<String>> {
        let content = self
            .conn
            .prepare_cached(
                "SELECT file_search.content
                 FROM file_search
                 JOIN files ON files.id = file_search.rowid
                 WHERE files.root = ?1 AND files.path = ?2",
            )?
            .query_row([root, path], |row| row.get::<_, String>(0))
            .optional()?;
        Ok(content)
    }

    fn search_text(&self, root: &str, query: &str, limit: u32) -> Result<Vec<TextHit>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT files.id, files.path, files.language, files.mtime_ms,
                    bm25(file_search), file_search.content
             FROM file_search
             JOIN files ON files.id = file_search.rowid
             WHERE file_search MATCH ?1 AND files.root = ?2
             ORDER BY bm25(file_search)
             LIMIT ?3",
        )?;

        let hits = statement
            .query_map(rusqlite::params![query, root, limit], |row| {
                Ok(TextHit {
                    file: FileId(row.get(0)?),
                    path: row.get(1)?,
                    language: row.get(2)?,
                    mtime_ms: row.get(3)?,
                    // Negated so that callers can treat bigger as better.
                    relevance: -row.get::<_, f64>(4)?,
                    content: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<TextHit>>>()?;
        Ok(hits)
    }

    fn relationships(&self, root: &str, path: &str) -> Result<Vec<Relationship>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT relationships.kind, relationships.from_symbol, relationships.target,
                    relationships.target_path, relationships.line
             FROM relationships
             JOIN files ON files.id = relationships.file_id
             WHERE files.root = ?1 AND files.path = ?2
             ORDER BY relationships.line, relationships.target",
        )?;

        let edges = statement
            .query_map([root, path], |row| {
                Ok(Relationship {
                    kind: RelationshipKind::from_str_lossy(&row.get::<_, String>(0)?),
                    from_symbol: row.get(1)?,
                    target: row.get(2)?,
                    target_path: row.get(3)?,
                    line: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<Relationship>>>()?;
        Ok(edges)
    }

    fn edges(&self, root: &str) -> Result<Vec<Edge>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT files.path, relationships.target_path, relationships.kind
             FROM relationships
             JOIN files ON files.id = relationships.file_id
             WHERE files.root = ?1 AND relationships.target_path IS NOT NULL
             ORDER BY files.path, relationships.target_path",
        )?;

        let edges = statement
            .query_map([root], |row| {
                Ok(Edge {
                    from: row.get(0)?,
                    to: row.get(1)?,
                    kind: RelationshipKind::from_str_lossy(&row.get::<_, String>(2)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<Edge>>>()?;
        Ok(edges)
    }

    fn counts(&self, root: &str) -> Result<IndexCounts> {
        let counts = self
            .conn
            .prepare_cached(
                "SELECT
                     (SELECT COUNT(*) FROM files WHERE root = ?1),
                     (SELECT COUNT(*) FROM symbols
                          JOIN files ON files.id = symbols.file_id WHERE files.root = ?1),
                     (SELECT COUNT(*) FROM relationships
                          JOIN files ON files.id = relationships.file_id WHERE files.root = ?1)",
            )?
            .query_row([root], |row| {
                Ok(IndexCounts {
                    files: row.get::<_, i64>(0)? as u64,
                    symbols: row.get::<_, i64>(1)? as u64,
                    relationships: row.get::<_, i64>(2)? as u64,
                })
            })?;
        Ok(counts)
    }

    fn record_index_run(&self, root: &str, at: Timestamp, counts: IndexCounts) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO index_state (root, last_indexed_at, file_count, symbol_count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(root) DO UPDATE SET
                 last_indexed_at = excluded.last_indexed_at,
                 file_count      = excluded.file_count,
                 symbol_count    = excluded.symbol_count",
            )?
            .execute(rusqlite::params![
                root,
                at.as_millis(),
                counts.files as i64,
                counts.symbols as i64
            ])?;
        Ok(())
    }

    fn last_indexed_at(&self, root: &str) -> Result<Option<Timestamp>> {
        let at = self
            .conn
            .prepare_cached("SELECT last_indexed_at FROM index_state WHERE root = ?1")?
            .query_row([root], |row| row.get::<_, i64>(0))
            .optional()?;
        Ok(at.map(Timestamp::from_millis))
    }

    fn roots(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT root FROM index_state ORDER BY root")?;
        let roots = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(roots)
    }
}

/// Escape the wildcards SQL `LIKE` would otherwise interpret, so a search for
/// `get_%` looks for that name rather than everything starting with `get_`.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_graph::model::Symbol;

    const ROOT: &str = "/repo";

    fn fingerprint(hash: &str) -> FileFingerprint {
        FileFingerprint {
            size: 120,
            mtime_ms: 1_700_000_000_000,
            content_hash: hash.into(),
        }
    }

    fn intelligence() -> FileIntelligence {
        FileIntelligence {
            symbols: vec![
                Symbol {
                    name: "authenticate".into(),
                    kind: SymbolKind::Function,
                    start_line: 10,
                    end_line: 20,
                },
                Symbol {
                    name: "Session".into(),
                    kind: SymbolKind::Struct,
                    start_line: 3,
                    end_line: 6,
                },
            ],
            relationships: vec![Relationship {
                kind: RelationshipKind::Imports,
                from_symbol: None,
                target: "crate::database".into(),
                target_path: Some("src/database.rs".into()),
                line: 1,
            }],
        }
    }

    fn store_with_file(database: &Database) -> FileId {
        let store = SqliteIndexStore::new(database);
        let id = store
            .upsert_file(
                ROOT,
                "src/auth.rs",
                Some("rust"),
                &fingerprint("hash-1"),
                Timestamp::from_millis(1_700_000_000_000),
            )
            .unwrap();
        store.replace_intelligence(id, &intelligence()).unwrap();
        id
    }

    #[test]
    fn files_round_trip() {
        let database = Database::open_in_memory().unwrap();
        store_with_file(&database);
        let store = SqliteIndexStore::new(&database);

        let file = store.file(ROOT, "src/auth.rs").unwrap().unwrap();
        assert_eq!(file.language.as_deref(), Some("rust"));
        assert_eq!(file.fingerprint.content_hash, "hash-1");
        assert!(store.file(ROOT, "src/missing.rs").unwrap().is_none());
    }

    #[test]
    fn re_indexing_updates_rather_than_duplicates() {
        let database = Database::open_in_memory().unwrap();
        let first = store_with_file(&database);
        let store = SqliteIndexStore::new(&database);

        let second = store
            .upsert_file(
                ROOT,
                "src/auth.rs",
                Some("rust"),
                &fingerprint("hash-2"),
                Timestamp::from_millis(1_700_000_001_000),
            )
            .unwrap();

        assert_eq!(first, second, "the same file keeps its id");
        assert_eq!(store.counts(ROOT).unwrap().files, 1);
        assert_eq!(
            store
                .file(ROOT, "src/auth.rs")
                .unwrap()
                .unwrap()
                .fingerprint
                .content_hash,
            "hash-2"
        );
    }

    #[test]
    fn replacing_intelligence_leaves_no_stale_rows() {
        let database = Database::open_in_memory().unwrap();
        let id = store_with_file(&database);
        let store = SqliteIndexStore::new(&database);

        store
            .replace_intelligence(
                id,
                &FileIntelligence {
                    symbols: vec![Symbol {
                        name: "verify".into(),
                        kind: SymbolKind::Function,
                        start_line: 1,
                        end_line: 2,
                    }],
                    relationships: Vec::new(),
                },
            )
            .unwrap();

        let counts = store.counts(ROOT).unwrap();
        assert_eq!(counts.symbols, 1);
        assert_eq!(counts.relationships, 0);
        assert!(store
            .search_symbols(ROOT, "authenticate", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn deleting_a_file_removes_its_symbols_and_relationships() {
        let database = Database::open_in_memory().unwrap();
        store_with_file(&database);
        let store = SqliteIndexStore::new(&database);

        assert!(store.delete_file(ROOT, "src/auth.rs").unwrap());
        assert_eq!(store.counts(ROOT).unwrap(), IndexCounts::default());
        assert!(!store.delete_file(ROOT, "src/auth.rs").unwrap());
    }

    #[test]
    fn search_ranks_exact_matches_first() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let id = store
            .upsert_file(
                ROOT,
                "src/a.rs",
                Some("rust"),
                &fingerprint("h"),
                Timestamp::now(),
            )
            .unwrap();

        store
            .replace_intelligence(
                id,
                &FileIntelligence {
                    symbols: ["reauthenticate", "authenticate_user", "authenticate"]
                        .iter()
                        .map(|name| Symbol {
                            name: (*name).into(),
                            kind: SymbolKind::Function,
                            start_line: 1,
                            end_line: 2,
                        })
                        .collect(),
                    relationships: Vec::new(),
                },
            )
            .unwrap();

        let hits = store.search_symbols(ROOT, "authenticate", 10).unwrap();
        let names: Vec<&str> = hits.iter().map(|hit| hit.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["authenticate", "authenticate_user", "reauthenticate"]
        );
    }

    #[test]
    fn search_treats_wildcards_as_text() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let id = store
            .upsert_file(
                ROOT,
                "src/a.rs",
                Some("rust"),
                &fingerprint("h"),
                Timestamp::now(),
            )
            .unwrap();
        store
            .replace_intelligence(
                id,
                &FileIntelligence {
                    symbols: vec![Symbol {
                        name: "get_user".into(),
                        kind: SymbolKind::Function,
                        start_line: 1,
                        end_line: 2,
                    }],
                    relationships: Vec::new(),
                },
            )
            .unwrap();

        assert_eq!(store.search_symbols(ROOT, "get_user", 10).unwrap().len(), 1);
        assert!(
            store.search_symbols(ROOT, "%", 10).unwrap().is_empty(),
            "a bare wildcard must not match everything"
        );
    }

    #[test]
    fn searches_are_scoped_to_their_root() {
        let database = Database::open_in_memory().unwrap();
        store_with_file(&database);
        let store = SqliteIndexStore::new(&database);

        assert!(store
            .search_symbols("/other-repo", "authenticate", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn resolved_relationships_become_graph_edges() {
        let database = Database::open_in_memory().unwrap();
        store_with_file(&database);
        let store = SqliteIndexStore::new(&database);

        let edges = store.edges(ROOT).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "src/auth.rs");
        assert_eq!(edges[0].to, "src/database.rs");

        let relationships = store.relationships(ROOT, "src/auth.rs").unwrap();
        assert_eq!(relationships[0].target, "crate::database");
    }

    #[test]
    fn index_runs_are_recorded_per_root() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let at = Timestamp::from_millis(1_700_000_000_000);

        store
            .record_index_run(
                ROOT,
                at,
                IndexCounts {
                    files: 4,
                    symbols: 40,
                    relationships: 12,
                },
            )
            .unwrap();
        store
            .record_index_run(
                ROOT,
                at,
                IndexCounts {
                    files: 5,
                    symbols: 50,
                    relationships: 15,
                },
            )
            .unwrap();

        assert_eq!(store.last_indexed_at(ROOT).unwrap(), Some(at));
        assert_eq!(store.roots().unwrap(), vec![ROOT.to_string()]);
        assert_eq!(store.last_indexed_at("/never").unwrap(), None);
    }

    #[test]
    fn text_search_ranks_by_relevance() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        let files = [
            (
                "src/auth.rs",
                "fn authenticate() { session timeout handling }",
            ),
            ("src/session.rs", "fn refresh() { timeout }"),
            ("src/unrelated.rs", "fn render() { colors }"),
        ];
        for (path, content) in files {
            let id = store
                .upsert_file(
                    ROOT,
                    path,
                    Some("rust"),
                    &fingerprint("h"),
                    Timestamp::now(),
                )
                .unwrap();
            store.index_content(id, path, content).unwrap();
        }

        let hits = store
            .search_text(ROOT, "\"timeout\" OR \"authenticate\"", 10)
            .unwrap();

        assert_eq!(hits.len(), 2, "only matching files come back");
        assert_eq!(
            hits[0].path, "src/auth.rs",
            "the file matching both terms ranks first"
        );
        assert!(hits[0].relevance > hits[1].relevance, "bigger is better");
        assert!(hits[0].content.contains("authenticate"));
    }

    #[test]
    fn text_search_is_scoped_to_its_root() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let id = store
            .upsert_file(
                ROOT,
                "a.rs",
                Some("rust"),
                &fingerprint("h"),
                Timestamp::now(),
            )
            .unwrap();
        store.index_content(id, "a.rs", "authentication").unwrap();

        assert!(store
            .search_text("/other", "\"authentication\"", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn re_indexing_content_replaces_the_old_text() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let id = store
            .upsert_file(
                ROOT,
                "a.rs",
                Some("rust"),
                &fingerprint("h"),
                Timestamp::now(),
            )
            .unwrap();

        store.index_content(id, "a.rs", "authentication").unwrap();
        store.index_content(id, "a.rs", "rendering").unwrap();

        assert!(store
            .search_text(ROOT, "\"authentication\"", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            store.search_text(ROOT, "\"rendering\"", 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn deleting_a_file_removes_it_from_search() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let id = store
            .upsert_file(
                ROOT,
                "a.rs",
                Some("rust"),
                &fingerprint("h"),
                Timestamp::now(),
            )
            .unwrap();
        store.index_content(id, "a.rs", "authentication").unwrap();

        store.delete_file(ROOT, "a.rs").unwrap();
        assert!(store
            .search_text(ROOT, "\"authentication\"", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn underscored_identifiers_are_found_by_their_parts() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let id = store
            .upsert_file(
                ROOT,
                "a.rs",
                Some("rust"),
                &fingerprint("h"),
                Timestamp::now(),
            )
            .unwrap();
        store
            .index_content(id, "a.rs", "fn get_user_session() {}")
            .unwrap();

        assert_eq!(store.search_text(ROOT, "\"session\"", 10).unwrap().len(), 1);
        assert_eq!(
            store.search_text(ROOT, "\"get_user\"", 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn file_paths_are_listed_for_a_root() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        for path in ["src/b.rs", "src/a.rs"] {
            store
                .upsert_file(
                    ROOT,
                    path,
                    Some("rust"),
                    &fingerprint("h"),
                    Timestamp::now(),
                )
                .unwrap();
        }

        assert_eq!(
            store.file_paths(ROOT).unwrap(),
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
    }
}
