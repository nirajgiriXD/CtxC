//! `ctxc index`, `ctxc graph` and `ctxc search`.
//!
//! The three views of the code index: build it, look at how files depend on
//! each other, and find where something is defined.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_engine::index::{self, IndexOptions, IndexReport, Indexer};
use ctxc_engine::ProjectEmbedder;
use ctxc_metrics::{MetricEvent, Operation};
use ctxc_semantic::SemanticOptions;
use ctxc_store::{IndexStore, SqliteEmbeddingStore, SqliteIndexStore};

use crate::app::App;
use crate::output::{human_count, Printer, Render};

impl Render for IndexReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "{}", self.root.display())?;
        writeln!(out)?;
        writeln!(out, "Scanned:       {}", human_count(self.scanned as u32))?;
        writeln!(out, "Indexed:       {}", human_count(self.indexed as u32))?;
        writeln!(out, "Unchanged:     {}", human_count(self.unchanged as u32))?;
        if self.removed > 0 {
            writeln!(out, "Removed:       {}", human_count(self.removed as u32))?;
        }
        if self.embedded > 0 {
            writeln!(out, "Embedded:      {}", human_count(self.embedded as u32))?;
        }
        writeln!(out, "Symbols:       {}", human_count(self.symbols as u32))?;
        writeln!(
            out,
            "Relationships: {}",
            human_count(self.relationships as u32)
        )?;
        writeln!(out, "Ignored:       {}", human_count(self.ignored as u32))?;
        writeln!(
            out,
            "Duration:      {} ms",
            human_count(self.duration_ms as u32)
        )
    }
}

/// One file's place in the dependency graph.
#[derive(Debug, Serialize)]
pub struct FileGraph {
    pub path: String,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
    /// Imports that did not resolve to a file in this project: other crates,
    /// packages, and anything the resolver could not follow.
    pub unresolved_imports: Vec<String>,
}

/// A file and how many others depend on it.
#[derive(Debug, Serialize)]
pub struct GraphEntry {
    pub path: String,
    pub dependents: usize,
}

/// The dependency graph, or one file's corner of it.
#[derive(Debug, Serialize)]
pub struct GraphReport {
    pub root: String,
    pub files: u64,
    pub nodes: usize,
    pub edges: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileGraph>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub most_depended_on: Vec<GraphEntry>,
}

impl Render for GraphReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if let Some(file) = &self.file {
            writeln!(out, "{}", file.path)?;
            writeln!(out)?;
            write_list(out, "Depends on", &file.dependencies)?;
            write_list(out, "Depended on by", &file.dependents)?;
            write_list(out, "Unresolved imports", &file.unresolved_imports)?;
            return Ok(());
        }

        writeln!(out, "{}", self.root)?;
        writeln!(out)?;
        writeln!(out, "Files:  {}", human_count(self.files as u32))?;
        writeln!(out, "Nodes:  {}", human_count(self.nodes as u32))?;
        writeln!(out, "Edges:  {}", human_count(self.edges as u32))?;

        if self.most_depended_on.is_empty() {
            writeln!(out)?;
            return writeln!(out, "No resolved dependencies between files yet.");
        }

        writeln!(out)?;
        writeln!(out, "Most depended on:")?;
        for entry in &self.most_depended_on {
            writeln!(
                out,
                "  {:<48}{} dependents",
                entry.path,
                human_count(entry.dependents as u32)
            )?;
        }
        Ok(())
    }
}

fn write_list(out: &mut dyn Write, title: &str, items: &[String]) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(out, "{title}:")?;
    for item in items {
        writeln!(out, "  {item}")?;
    }
    writeln!(out)
}

pub fn index<W: Write>(
    app: &App,
    root: &Path,
    force: bool,
    printer: &mut Printer<W>,
) -> Result<()> {
    let database = app.open_database()?;
    let store = SqliteIndexStore::new(&database);
    let options = IndexOptions {
        force,
        ..IndexOptions::default()
    };

    // Embeddings are computed in the same pass rather than a second one: the
    // file's text is already read and its hash already known.
    let semantic = SemanticOptions::from_config(&app.config);
    let embeddings = SqliteEmbeddingStore::new(&database);
    let embedder = ProjectEmbedder::from_options(&semantic, &embeddings, &index::root_key(root))?;

    // One transaction for the whole pass: thousands of small writes, and an
    // interrupted run should leave the previous index intact.
    let report = database
        .transaction(|| {
            let mut indexer = Indexer::new(&store);
            if let Some(embedder) = &embedder {
                indexer = indexer.with_embedder(embedder);
            }
            indexer.index(root, &options)
        })
        .with_context(|| format!("failed to index {}", root.display()))?;

    app.record(index_event(&report, &index::root_key(root), &database));

    printer.emit(&report)?;
    Ok(())
}

/// Describe an index pass as a metric.
///
/// Files skipped because they had not changed are the index cache doing its
/// job, so they are recorded as hits against the files the pass looked at —
/// which is what makes "cache hit rate" mean something for indexing.
pub fn index_event(
    report: &IndexReport,
    root_key: &str,
    database: &ctxc_store::Database,
) -> MetricEvent {
    let looked_at = report.indexed.saturating_add(report.unchanged);

    MetricEvent::new(Operation::Index, root_key)
        .for_project_opt(super::metrics::project_for_root(database, root_key))
        .took(std::time::Duration::from_millis(report.duration_ms))
        .with_cache(
            report.unchanged.min(u32::MAX as u64) as u32,
            looked_at.min(u32::MAX as u64) as u32,
        )
}

pub fn graph<W: Write>(
    app: &App,
    root: &Path,
    file: Option<&str>,
    limit: usize,
    printer: &mut Printer<W>,
) -> Result<()> {
    let database = app.open_database()?;
    let store = SqliteIndexStore::new(&database);
    let key = index::root_key(root);

    let counts = store.counts(&key)?;
    if counts.files == 0 {
        return Err(crate::error::CliError::new(format!(
            "{} has not been indexed",
            root.display()
        ))
        .with_hint("run `ctxc index` first")
        .into());
    }

    let graph = index::load_graph(&store, root)?;
    let file = match file {
        Some(path) => Some(FileGraph {
            dependencies: graph
                .dependencies_of(path)
                .into_iter()
                .map(String::from)
                .collect(),
            dependents: graph
                .dependents_of(path)
                .into_iter()
                .map(String::from)
                .collect(),
            unresolved_imports: unresolved_imports(&store, &key, path)?,
            path: path.to_string(),
        }),
        None => None,
    };

    let report = GraphReport {
        root: key,
        files: counts.files,
        nodes: graph.node_count(),
        edges: graph.edge_count(),
        most_depended_on: if file.is_none() {
            graph
                .most_depended_on(limit)
                .into_iter()
                .map(|(path, dependents)| GraphEntry {
                    path: path.to_string(),
                    dependents,
                })
                .collect()
        } else {
            Vec::new()
        },
        file,
    };

    printer.emit(&report)?;
    Ok(())
}

/// Imports of a file that did not resolve to another file in the project.
fn unresolved_imports(store: &dyn IndexStore, root: &str, path: &str) -> Result<Vec<String>> {
    let mut targets: Vec<String> = store
        .relationships(root, path)?
        .into_iter()
        .filter(|edge| {
            edge.kind == ctxc_graph::RelationshipKind::Imports && edge.target_path.is_none()
        })
        .map(|edge| edge.target)
        .collect();
    targets.sort();
    targets.dedup();
    Ok(targets)
}

/// The directory a command should work on, defaulting to the current one.
pub fn root_or_current(path: Option<&PathBuf>) -> PathBuf {
    path.cloned().unwrap_or_else(|| PathBuf::from("."))
}
