//! Incremental indexing.
//!
//! Walk a project, parse what changed, and write the result. The "what changed"
//! is the whole point: re-indexing a repository where nothing moved must cost a
//! `stat` per file, not a parse per file.
//!
//! ```text
//! walk -> unchanged?  -> skip
//!      -> same bytes? -> refresh metadata only
//!      -> otherwise   -> parse, resolve imports, replace rows
//! ```
//!
//! A full pass runs that decision in two halves. Reading, hashing, parsing and
//! resolving imports touch no shared state, so they run across every core at
//! once; writing is then a serial drain of the results, in walk order, inside
//! the caller's transaction. SQLite has one writer either way, and the writes
//! were never the expensive half.
//!
//! Deletions are handled by comparing what the walk saw against what the index
//! holds, so a file removed while CtxC was not running still disappears.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use ctxc_context::walk::{self, WalkEntry, WalkOptions};
use ctxc_core::{id, Timestamp};
use ctxc_graph::model::{FileFingerprint, FileIntelligence};
use ctxc_graph::DependencyGraph;
use ctxc_parser::{resolve_import, Language, ParserRegistry};
use ctxc_store::{IndexCounts, IndexStore, StoredFingerprint};
use ctxc_watcher::{Change, ChangeKind};

use crate::error::{EngineError, Result};

/// How many files one parallel batch prepares before they are written.
///
/// The prepared results hold each file's text, so an unbounded batch would
/// hold the whole project in memory at once. A batch large enough to keep
/// every core busy and small enough to bound that is the whole trade-off.
const BATCH: usize = 256;

/// Largest file whose text is kept for full-text search. Beyond this a file
/// is still indexed and its symbols recorded; only its body stops being
/// searchable, which keeps a vendored bundle from bloating the database.
const MAX_SEARCHABLE_BYTES: usize = 512 * 1024;

/// How to run an index pass.
#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    pub walk: WalkOptions,
    /// Re-parse every file, even ones that look unchanged.
    pub force: bool,
}

/// What one index pass did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReport {
    pub root: PathBuf,
    /// Files the walk returned.
    pub scanned: u64,
    /// Files parsed and written.
    pub indexed: u64,
    /// Files skipped because they had not changed.
    pub unchanged: u64,
    /// Files dropped from the index because they are gone.
    pub removed: u64,
    /// Paths the ignore rules excluded.
    pub ignored: u64,
    /// Files skipped for being too large to index.
    pub too_large: u64,
    /// Files whose language CtxC cannot parse; recorded, but not analysed.
    pub unparsed: u64,
    /// Files embedded this pass. Zero when embeddings are off, and lower than
    /// `indexed` when a file's text was already embedded unchanged.
    #[serde(default)]
    pub embedded: u64,
    pub symbols: u64,
    pub relationships: u64,
    pub duration_ms: u64,
}

impl IndexReport {
    /// An empty report for a root, used when accumulating targeted work.
    fn empty(root: &Path) -> IndexReport {
        IndexReport {
            root: root.to_path_buf(),
            scanned: 0,
            indexed: 0,
            unchanged: 0,
            removed: 0,
            ignored: 0,
            too_large: 0,
            unparsed: 0,
            embedded: 0,
            symbols: 0,
            relationships: 0,
            duration_ms: 0,
        }
    }

    /// Whether the pass changed anything in the index.
    pub fn is_up_to_date(&self) -> bool {
        self.indexed == 0 && self.removed == 0
    }
}

/// What applying a batch of changes did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeReport {
    pub root: PathBuf,
    /// Changes in the batch.
    pub considered: u64,
    pub indexed: u64,
    pub unchanged: u64,
    pub removed: u64,
    pub renamed: u64,
    pub symbols: u64,
    pub duration_ms: u64,
}

impl ChangeReport {
    /// Whether anything in the index actually moved.
    pub fn changed_anything(&self) -> bool {
        self.indexed > 0 || self.removed > 0 || self.renamed > 0
    }
}

/// Told how far an index pass has got.
///
/// A pass over a large repository takes long enough that silence is
/// indistinguishable from a hang. The engine reports; deciding whether anyone
/// is watching, and what a progress line should look like, belongs to whatever
/// owns the terminal.
pub trait IndexProgress {
    /// Files walked so far, and how many of those were parsed and written.
    fn advance(&self, seen: u64, indexed: u64);

    /// The pass is over; leave the terminal as it was found.
    fn finish(&self);
}

/// Indexes projects into an [`IndexStore`].
pub struct Indexer<'a> {
    store: &'a dyn IndexStore,
    parsers: ParserRegistry,
    embedder: Option<&'a crate::embed::ProjectEmbedder<'a>>,
    progress: Option<&'a dyn IndexProgress>,
}

impl<'a> Indexer<'a> {
    pub fn new(store: &'a dyn IndexStore) -> Self {
        Indexer {
            store,
            parsers: ParserRegistry::new(),
            embedder: None,
            progress: None,
        }
    }

    /// Report progress as the pass runs.
    pub fn with_progress(mut self, progress: &'a dyn IndexProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Also embed each file's text as it is indexed.
    ///
    /// Passed in rather than read from configuration here: the indexer holds no
    /// database handle of its own, and the caller already has both halves.
    pub fn with_embedder(mut self, embedder: &'a crate::embed::ProjectEmbedder<'a>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Index everything under `root`.
    ///
    /// The caller is expected to wrap this in a transaction: a pass writes
    /// thousands of rows, and a half-applied index is worse than none.
    pub fn index(&mut self, root: &Path, options: &IndexOptions) -> Result<IndexReport> {
        let started = Instant::now();
        let root_key = root_key(root);
        let found = walk::walk(root, &options.walk)?;

        // Import resolution asks "does this file exist?", and the walk already
        // answered that for every file in the project.
        let known: HashSet<&str> = found
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        let mut report = IndexReport {
            root: root.to_path_buf(),
            scanned: found.entries.len() as u64,
            indexed: 0,
            unchanged: 0,
            removed: 0,
            ignored: found.ignored,
            too_large: found.too_large,
            unparsed: 0,
            embedded: 0,
            symbols: 0,
            relationships: 0,
            duration_ms: 0,
        };

        // One query for the whole root. The alternative — asking SQLite about
        // each walked file in turn — is the cost a warm re-index is supposed
        // not to have.
        let stored = self.store.fingerprints(&root_key)?;

        let mut seen: HashSet<&str> = HashSet::with_capacity(found.entries.len());
        for entry in &found.entries {
            seen.insert(entry.path.as_str());
        }

        let mut seen_files = 0u64;

        // Prepare a batch across every core, then write it here. `map_init`
        // hands each worker its own parser registry, which is what tree-sitter
        // requires: cheap to reuse, not safe to share.
        for batch in found.entries.chunks(BATCH) {
            let prepared: Vec<Result<Prepared>> = batch
                .par_iter()
                .map_init(ParserRegistry::new, |parsers, entry| {
                    prepare(
                        parsers,
                        entry,
                        stored.get(&entry.path),
                        &known,
                        options.force,
                    )
                })
                .collect();

            for (entry, outcome) in batch.iter().zip(prepared) {
                self.write_prepared(&root_key, entry, outcome?, &mut report)?;
            }

            // Once a batch, not once a file: a line that redraws thousands of
            // times a second costs more than the work it is describing.
            seen_files += batch.len() as u64;
            if let Some(progress) = self.progress {
                progress.advance(seen_files, report.indexed);
            }
        }

        if let Some(progress) = self.progress {
            progress.finish();
        }

        // The same map answers "what did the index hold that the walk did not
        // see?", so the deletion sweep needs no second listing either.
        for path in stored.keys() {
            if !seen.contains(path.as_str()) {
                self.store.delete_file(&root_key, path)?;
                // A vector for a file that is gone would keep turning up in
                // similarity results for content nobody can open.
                if let Some(embedder) = self.embedder {
                    embedder.forget(path)?;
                }
                report.removed += 1;
            }
        }

        let counts = self.store.counts(&root_key)?;
        self.store
            .record_index_run(&root_key, Timestamp::now(), counts)?;

        report.duration_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    /// Write what [`prepare`] decided about one file.
    ///
    /// Everything here touches the database and nothing here is expensive, so
    /// it runs on one thread inside the caller's transaction.
    fn write_prepared(
        &mut self,
        root_key: &str,
        entry: &WalkEntry,
        prepared: Prepared,
        report: &mut IndexReport,
    ) -> Result<()> {
        let relative = entry.path.as_str();

        match prepared {
            Prepared::Vanished => Ok(()),

            Prepared::Unchanged => {
                report.unchanged += 1;
                Ok(())
            }

            Prepared::Touched {
                fingerprint,
                language,
                indexed_at,
            } => {
                self.store.upsert_file(
                    root_key,
                    relative,
                    language.map(Language::as_str),
                    &fingerprint,
                    indexed_at,
                )?;
                report.unchanged += 1;
                Ok(())
            }

            Prepared::Parsed {
                fingerprint,
                language,
                intelligence,
                searchable,
                unparsed,
            } => {
                if unparsed {
                    report.unparsed += 1;
                }

                let id = self.store.upsert_file(
                    root_key,
                    relative,
                    language.map(Language::as_str),
                    &fingerprint,
                    Timestamp::now(),
                )?;
                self.store.replace_intelligence(id, &intelligence)?;

                // Text is made searchable whether or not CtxC can parse it: a
                // README answers as many questions as a source file does.
                if let Some(text) = &searchable {
                    self.store.index_content(id, relative, text)?;

                    if let Some(embedder) = self.embedder {
                        if embedder.embed_file(relative, text, &fingerprint.content_hash)? {
                            report.embedded += 1;
                        }
                    }
                }

                report.indexed += 1;
                report.symbols += intelligence.symbols.len() as u64;
                report.relationships += intelligence.relationships.len() as u64;
                Ok(())
            }
        }
    }

    /// Index one file named by a change, reading its stored state as it goes.
    ///
    /// The targeted path through the same decision: one file, one fingerprint
    /// lookup, no batch to prepare.
    fn index_one(
        &mut self,
        root_key: &str,
        entry: &WalkEntry,
        known: &HashSet<&str>,
        report: &mut IndexReport,
    ) -> Result<()> {
        let stored = self
            .store
            .file(root_key, &entry.path)?
            .map(|file| StoredFingerprint {
                fingerprint: file.fingerprint,
                indexed_at: file.indexed_at,
            });

        let prepared = prepare(&mut self.parsers, entry, stored.as_ref(), known, false)?;
        self.write_prepared(root_key, entry, prepared, report)
    }

    /// Apply a settled batch of filesystem changes.
    ///
    /// This is the difference continuous mode makes: instead of walking the
    /// project to discover what moved, the watcher already said, so the work is
    /// proportional to what changed rather than to the size of the repository.
    ///
    /// Applying the same batch twice is safe. Every operation is keyed by path
    /// and re-derives its result from the file on disk, so a replayed or
    /// duplicated event cannot corrupt the index.
    pub fn apply(&mut self, root: &Path, changes: &[Change]) -> Result<ChangeReport> {
        let started = Instant::now();
        let root_key = root_key(root);

        let mut report = ChangeReport {
            root: root.to_path_buf(),
            considered: changes.len() as u64,
            ..ChangeReport::default()
        };

        // Import resolution asks whether a path exists; for a targeted update
        // the index itself is the cheapest available answer.
        let indexed_paths = self.store.file_paths(&root_key)?;
        let known: HashSet<&str> = indexed_paths.iter().map(String::as_str).collect();

        for change in changes {
            match &change.kind {
                ChangeKind::Deleted => {
                    if self.store.delete_file(&root_key, &change.path)? {
                        report.removed += 1;
                    }
                    // A directory deletion arrives as one event for the folder.
                    let swept = self.store.delete_under(&root_key, &change.path)?;
                    report.removed += swept;
                }
                ChangeKind::Renamed { from } => {
                    if self.store.rename_file(&root_key, from, &change.path)? {
                        report.renamed += 1;
                    }
                    self.index_path(&root_key, root, &change.path, &known, &mut report)?;
                }
                ChangeKind::Created | ChangeKind::Modified => {
                    self.index_path(&root_key, root, &change.path, &known, &mut report)?;
                }
            }
        }

        report.duration_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    /// Index one path named by a change, reading its current state from disk.
    fn index_path(
        &mut self,
        root_key: &str,
        root: &Path,
        relative: &str,
        known: &HashSet<&str>,
        report: &mut ChangeReport,
    ) -> Result<()> {
        let absolute = root.join(relative);
        let Ok(metadata) = std::fs::metadata(&absolute) else {
            // The path is gone. That is a deletion, whatever the event said —
            // events and reality race, and reality wins.
            if self.store.delete_file(root_key, relative)? {
                report.removed += 1;
            }
            return Ok(());
        };
        if metadata.is_dir() {
            return Ok(());
        }

        let entry = WalkEntry {
            path: relative.to_owned(),
            size: metadata.len(),
            mtime_ms: mtime_ms(&metadata),
            absolute,
        };

        let mut pass = IndexReport::empty(root);
        self.index_one(root_key, &entry, known, &mut pass)?;

        report.indexed += pass.indexed;
        report.unchanged += pass.unchanged;
        report.symbols += pass.symbols;
        Ok(())
    }
}

/// What preparing one file decided, before anything is written.
#[derive(Debug)]
enum Prepared {
    /// Size and mtime both match: the file has not been touched.
    Unchanged,
    /// The bytes match but the metadata moved — editors and checkouts touch
    /// files constantly. Refresh the fingerprint so the cheap check works next
    /// time, but do not re-parse.
    Touched {
        fingerprint: FileFingerprint,
        language: Option<Language>,
        indexed_at: Timestamp,
    },
    /// New content: everything derived from this file has to be replaced.
    Parsed {
        fingerprint: FileFingerprint,
        language: Option<Language>,
        intelligence: FileIntelligence,
        /// The text to make searchable, when it is text and small enough.
        searchable: Option<String>,
        /// True when CtxC has no parser for this file, so it is recorded
        /// without symbols.
        unparsed: bool,
    },
    /// The file went away between being walked and being read.
    Vanished,
}

/// Decide what one file needs, without touching the database.
///
/// This is the expensive half of an index pass — a read, a hash, a parse and
/// import resolution — and it holds no shared state, which is what lets a full
/// pass run it across every core at once.
fn prepare(
    parsers: &mut ParserRegistry,
    entry: &WalkEntry,
    stored: Option<&StoredFingerprint>,
    known: &HashSet<&str>,
    force: bool,
) -> Result<Prepared> {
    let relative = entry.path.as_str();

    // Cheap check first: size and mtime unchanged means untouched.
    if !force {
        if let Some(stored) = stored {
            if stored.fingerprint.size == entry.size
                && stored.fingerprint.mtime_ms == entry.mtime_ms
            {
                return Ok(Prepared::Unchanged);
            }
        }
    }

    let bytes = match std::fs::read(&entry.absolute) {
        Ok(bytes) => bytes,
        // A file that vanished between being noticed and being read is simply
        // not indexed; its deletion event will follow.
        Err(err) => {
            tracing::debug!(path = %relative, error = %err, "skipping unreadable file");
            return Ok(Prepared::Vanished);
        }
    };

    let fingerprint = FileFingerprint {
        size: entry.size,
        mtime_ms: entry.mtime_ms,
        content_hash: id::content_hash(&bytes),
    };
    let language = Language::from_path(Path::new(relative));

    if !force {
        if let Some(stored) = stored {
            if stored.fingerprint.content_matches(&fingerprint) {
                return Ok(Prepared::Touched {
                    fingerprint,
                    language,
                    indexed_at: stored.indexed_at,
                });
            }
        }
    }

    let text = std::str::from_utf8(&bytes).ok();
    let (intelligence, unparsed) = match (language, text) {
        (Some(language), Some(source)) => {
            let mut parsed = parsers.parse(language, source)?;
            resolve_imports(language, relative, &mut parsed, known);
            (parsed, false)
        }
        // Files CtxC cannot parse are still recorded: the index is the list of
        // what a project contains, not only of what it can read.
        _ => (FileIntelligence::default(), true),
    };

    Ok(Prepared::Parsed {
        searchable: text
            .filter(|text| text.len() <= MAX_SEARCHABLE_BYTES)
            .map(str::to_owned),
        fingerprint,
        language,
        intelligence,
        unparsed,
    })
}

/// Modification time in epoch milliseconds, or zero when unavailable.
fn mtime_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|delta| delta.as_millis() as i64)
        .unwrap_or(0)
}

/// Attach resolved file paths to the imports a file declares.
fn resolve_imports(
    language: Language,
    from: &str,
    intelligence: &mut FileIntelligence,
    known: &HashSet<&str>,
) {
    let lookup = |candidate: &str| known.contains(candidate);
    for edge in &mut intelligence.relationships {
        if edge.kind == ctxc_graph::RelationshipKind::Imports {
            edge.target_path = resolve_import(language, from, &edge.target, &lookup);
        }
    }
}

/// The key a root is stored under.
///
/// Canonicalized where the filesystem allows it, so `.` and `./project` and an
/// absolute path all describe the same index rather than three.
pub fn root_key(root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let text = canonical.to_string_lossy();
    // Windows canonicalization produces a `\\?\` prefix, which is correct but
    // unreadable, and would make the same project look like two.
    text.strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/")
}

/// Build the dependency graph for an indexed root.
pub fn load_graph(store: &dyn IndexStore, root: &Path) -> Result<DependencyGraph> {
    let edges = store.edges(&root_key(root))?;
    Ok(DependencyGraph::from_edges(edges))
}

/// Index totals for a root.
pub fn index_counts(store: &dyn IndexStore, root: &Path) -> Result<IndexCounts> {
    Ok(store.counts(&root_key(root))?)
}

/// Whether a root has ever been indexed.
pub fn last_indexed_at(store: &dyn IndexStore, root: &Path) -> Result<Option<Timestamp>> {
    Ok(store.last_indexed_at(&root_key(root))?)
}

impl From<ctxc_context::ContextError> for EngineError {
    fn from(source: ctxc_context::ContextError) -> Self {
        EngineError::Context(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_store::{Database, SqliteIndexStore};

    struct Project(PathBuf);

    impl Project {
        fn new(name: &str) -> Project {
            let path = std::env::temp_dir()
                .join("ctxc-index-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Project(path)
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn remove(&self, relative: &str) {
            std::fs::remove_file(self.0.join(relative)).unwrap();
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn typescript_project(name: &str) -> Project {
        let project = Project::new(name);
        project.write(
            "src/database.ts",
            "export class Database {\n  query(sql: string) {}\n}\n",
        );
        project.write(
            "src/auth.ts",
            "import { Database } from './database';\n\
             export function authenticate(token: string) {\n  return new Database().query(token);\n}\n",
        );
        project.write("README.md", "# project\n");
        project
    }

    #[test]
    fn indexing_records_files_symbols_and_resolved_imports() {
        let project = typescript_project("basic");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let report = Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        assert_eq!(report.scanned, 3);
        assert_eq!(report.indexed, 3);
        assert_eq!(
            report.unparsed, 1,
            "the markdown file is recorded, not parsed"
        );
        assert!(report.symbols >= 3);

        let root = root_key(&project.0);
        let hits = store.search_symbols(&root, "authenticate", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/auth.ts");
        assert_eq!(hits[0].language.as_deref(), Some("typescript"));

        let edges = store.edges(&root).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "src/auth.ts");
        assert_eq!(edges[0].to, "src/database.ts");
    }

    #[test]
    fn re_indexing_an_untouched_project_parses_nothing() {
        let project = typescript_project("unchanged");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();
        let second = Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        assert_eq!(second.indexed, 0);
        assert_eq!(second.unchanged, 3);
        assert!(second.is_up_to_date());
    }

    #[test]
    fn only_the_changed_file_is_re_indexed() {
        let project = typescript_project("changed");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        project.write(
            "src/auth.ts",
            "import { Database } from './database';\n\
             export function authenticate(token: string) {}\n\
             export function logout() {}\n",
        );

        let second = Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        assert_eq!(second.indexed, 1);
        assert_eq!(second.unchanged, 2);

        let root = root_key(&project.0);
        assert_eq!(store.search_symbols(&root, "logout", 10).unwrap().len(), 1);
    }

    #[test]
    fn a_touched_but_identical_file_is_not_re_parsed() {
        let project = typescript_project("touched");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        // Rewrite the same bytes, which moves the modification time.
        let contents = std::fs::read_to_string(project.0.join("README.md")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        project.write("README.md", &contents);

        let second = Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        assert_eq!(
            second.indexed, 0,
            "identical content must not be re-indexed just because the mtime moved"
        );
        assert_eq!(second.unchanged, 3);
    }

    #[test]
    fn deleted_files_leave_the_index() {
        let project = typescript_project("deleted");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();
        project.remove("src/database.ts");

        let second = Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        assert_eq!(second.removed, 1);
        let root = root_key(&project.0);
        assert_eq!(store.counts(&root).unwrap().files, 2);
        assert!(store
            .search_symbols(&root, "Database", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn forcing_re_indexes_everything() {
        let project = typescript_project("forced");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();
        let forced = Indexer::new(&store)
            .index(
                &project.0,
                &IndexOptions {
                    force: true,
                    ..IndexOptions::default()
                },
            )
            .unwrap();

        assert_eq!(forced.indexed, 3);
        assert_eq!(forced.unchanged, 0);
    }

    #[test]
    fn incremental_and_full_indexes_agree() {
        let project = typescript_project("agreement");
        let incremental_db = Database::open_in_memory().unwrap();
        let incremental = SqliteIndexStore::new(&incremental_db);

        Indexer::new(&incremental)
            .index(&project.0, &IndexOptions::default())
            .unwrap();
        project.write("src/session.ts", "export function refresh() {}\n");
        project.remove("README.md");
        Indexer::new(&incremental)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        let full_db = Database::open_in_memory().unwrap();
        let full = SqliteIndexStore::new(&full_db);
        Indexer::new(&full)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        let root = root_key(&project.0);
        assert_eq!(
            incremental.file_paths(&root).unwrap(),
            full.file_paths(&root).unwrap()
        );
        assert_eq!(
            incremental.counts(&root).unwrap(),
            full.counts(&root).unwrap()
        );
        assert_eq!(
            incremental.edges(&root).unwrap(),
            full.edges(&root).unwrap()
        );
    }

    #[test]
    fn ignored_directories_are_never_indexed() {
        let project = Project::new("ignored");
        project.write("src/app.ts", "export const x = 1;\n");
        project.write("node_modules/react/index.js", "module.exports = {};\n");
        project.write("target/debug/build.rs", "fn main() {}\n");

        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let report = Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        assert_eq!(report.scanned, 1);
        assert!(report.ignored >= 2);
    }

    #[test]
    fn the_graph_is_built_from_resolved_imports() {
        let project = typescript_project("graph");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        let graph = load_graph(&store, &project.0).unwrap();
        assert_eq!(
            graph.dependencies_of("src/auth.ts"),
            vec!["src/database.ts"]
        );
        assert_eq!(graph.dependents_of("src/database.ts"), vec!["src/auth.ts"]);
    }

    #[test]
    fn index_runs_are_recorded() {
        let project = typescript_project("recorded");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);

        assert!(last_indexed_at(&store, &project.0).unwrap().is_none());
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();
        assert!(last_indexed_at(&store, &project.0).unwrap().is_some());
        assert_eq!(index_counts(&store, &project.0).unwrap().files, 3);
    }

    #[test]
    fn root_keys_are_stable_and_use_forward_slashes() {
        let project = typescript_project("keys");
        let direct = root_key(&project.0);
        let indirect = root_key(&project.0.join("src").join(".."));

        assert_eq!(direct, indirect, "the same directory is one root");
        assert!(!direct.contains('\\'), "root keys are normalized: {direct}");
        assert!(!direct.starts_with(r"\\?\"));
    }

    #[test]
    fn a_changed_file_is_re_indexed_without_walking_the_project() {
        let project = typescript_project("apply-modify");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        project.write("src/auth.ts", "export function logout() {}\n");

        let report = Indexer::new(&store)
            .apply(&project.0, &[Change::modified("src/auth.ts")])
            .unwrap();

        assert_eq!(report.considered, 1);
        assert_eq!(report.indexed, 1);
        assert!(report.changed_anything());

        let root = root_key(&project.0);
        assert_eq!(store.search_symbols(&root, "logout", 10).unwrap().len(), 1);
        assert!(
            store
                .search_symbols(&root, "authenticate", 10)
                .unwrap()
                .is_empty(),
            "the old symbols are replaced, not added to"
        );
    }

    #[test]
    fn a_new_file_is_picked_up_from_its_change_alone() {
        let project = typescript_project("apply-create");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        project.write("src/session.ts", "export function refresh() {}\n");
        let report = Indexer::new(&store)
            .apply(&project.0, &[Change::created("src/session.ts")])
            .unwrap();

        assert_eq!(report.indexed, 1);
        let root = root_key(&project.0);
        assert_eq!(store.counts(&root).unwrap().files, 4);
        assert_eq!(store.search_symbols(&root, "refresh", 10).unwrap().len(), 1);
    }

    #[test]
    fn a_deleted_file_leaves_the_index() {
        let project = typescript_project("apply-delete");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        project.remove("src/database.ts");
        let report = Indexer::new(&store)
            .apply(&project.0, &[Change::deleted("src/database.ts")])
            .unwrap();

        assert_eq!(report.removed, 1);
        let root = root_key(&project.0);
        assert_eq!(store.counts(&root).unwrap().files, 2);
        assert!(store
            .search_symbols(&root, "Database", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_renamed_file_moves_its_record_instead_of_being_re_parsed() {
        let project = typescript_project("apply-rename");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        let contents = std::fs::read_to_string(project.0.join("src/database.ts")).unwrap();
        project.remove("src/database.ts");
        project.write("src/db.ts", &contents);

        let report = Indexer::new(&store)
            .apply(
                &project.0,
                &[Change::renamed("src/database.ts", "src/db.ts")],
            )
            .unwrap();

        assert_eq!(report.renamed, 1);
        assert_eq!(
            report.indexed, 0,
            "the content did not change, so nothing needed parsing"
        );

        let root = root_key(&project.0);
        let hits = store.search_symbols(&root, "Database", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/db.ts", "the symbol moved with the file");
    }

    #[test]
    fn deleting_a_directory_removes_everything_under_it() {
        let project = typescript_project("apply-delete-tree");
        project.write("src/nested/one.ts", "export function one() {}\n");
        project.write("src/nested/two.ts", "export function two() {}\n");

        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();
        assert_eq!(store.counts(&root_key(&project.0)).unwrap().files, 5);

        std::fs::remove_dir_all(project.0.join("src/nested")).unwrap();
        let report = Indexer::new(&store)
            .apply(&project.0, &[Change::deleted("src/nested")])
            .unwrap();

        assert_eq!(report.removed, 2, "one event, two files swept");
        assert_eq!(store.counts(&root_key(&project.0)).unwrap().files, 3);
    }

    #[test]
    fn applying_the_same_batch_twice_changes_nothing_the_second_time() {
        let project = typescript_project("apply-idempotent");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        project.write("src/auth.ts", "export function logout() {}\n");
        let batch = [Change::modified("src/auth.ts")];

        let first = Indexer::new(&store).apply(&project.0, &batch).unwrap();
        let second = Indexer::new(&store).apply(&project.0, &batch).unwrap();

        assert_eq!(first.indexed, 1);
        assert_eq!(second.indexed, 0, "a replayed event must be a no-op");
        assert_eq!(second.unchanged, 1);
        assert!(!second.changed_anything());
    }

    #[test]
    fn a_change_for_a_file_that_is_already_gone_is_treated_as_a_deletion() {
        let project = typescript_project("apply-race");
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        Indexer::new(&store)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        // The event says "modified", but by the time it is applied the file is
        // gone: events and reality race, and reality wins.
        project.remove("src/database.ts");
        let report = Indexer::new(&store)
            .apply(&project.0, &[Change::modified("src/database.ts")])
            .unwrap();

        assert_eq!(report.removed, 1);
        assert_eq!(store.counts(&root_key(&project.0)).unwrap().files, 2);
    }

    #[test]
    fn targeted_updates_agree_with_a_full_re_index() {
        let project = typescript_project("apply-agreement");
        let watched_db = Database::open_in_memory().unwrap();
        let watched = SqliteIndexStore::new(&watched_db);
        Indexer::new(&watched)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        // A working session: edit, add, move, delete.
        project.write("src/auth.ts", "export function login() {}\n");
        project.write("src/session.ts", "export function refresh() {}\n");
        let contents = std::fs::read_to_string(project.0.join("README.md")).unwrap();
        project.remove("README.md");
        project.write("GUIDE.md", &contents);

        Indexer::new(&watched)
            .apply(
                &project.0,
                &[
                    Change::modified("src/auth.ts"),
                    Change::created("src/session.ts"),
                    Change::renamed("README.md", "GUIDE.md"),
                ],
            )
            .unwrap();

        let full_db = Database::open_in_memory().unwrap();
        let full = SqliteIndexStore::new(&full_db);
        Indexer::new(&full)
            .index(&project.0, &IndexOptions::default())
            .unwrap();

        let root = root_key(&project.0);
        assert_eq!(
            watched.file_paths(&root).unwrap(),
            full.file_paths(&root).unwrap()
        );
        assert_eq!(watched.counts(&root).unwrap(), full.counts(&root).unwrap());
    }
    #[test]
    fn indexing_a_missing_directory_fails_clearly() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteIndexStore::new(&database);
        let error = Indexer::new(&store)
            .index(Path::new("no-such-project-here"), &IndexOptions::default())
            .unwrap_err();

        assert!(error.hint().is_some());
    }
}
