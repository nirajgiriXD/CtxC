//! `ctxc project index` and `ctxc project graph`.
//!
//! The three views of the code index: build it, look at how files depend on
//! each other, and find where something is defined.

use std::cell::Cell;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_engine::index::{self, IndexOptions, IndexProgress, IndexReport, Indexer};
use ctxc_engine::ProjectEmbedder;
use ctxc_metrics::{MetricEvent, Operation};
use ctxc_semantic::SemanticOptions;
use ctxc_store::{IndexStore, SqliteEmbeddingStore, SqliteIndexStore};

use crate::app::App;
use crate::output::{human_count, OutputFormat, Printer, Render};
use crate::style::Palette;

impl Render for IndexReport {
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        writeln!(out, "{}", palette.path(self.root.display()))?;
        writeln!(out)?;
        let mut field = |name: &str, value: String| -> io::Result<()> {
            writeln!(out, "{} {}", palette.label(format!("{name:<14}")), value)
        };
        field(
            "Scanned:",
            palette.number(human_count(self.scanned as u32)).to_string(),
        )?;
        field(
            "Indexed:",
            palette.number(human_count(self.indexed as u32)).to_string(),
        )?;
        field(
            "Unchanged:",
            palette.dim(human_count(self.unchanged as u32)).to_string(),
        )?;
        if self.removed > 0 {
            field(
                "Removed:",
                palette.warn(human_count(self.removed as u32)).to_string(),
            )?;
        }
        if self.embedded > 0 {
            field(
                "Embedded:",
                palette
                    .number(human_count(self.embedded as u32))
                    .to_string(),
            )?;
        }
        field(
            "Symbols:",
            palette.number(human_count(self.symbols as u32)).to_string(),
        )?;
        field(
            "Relationships:",
            palette
                .number(human_count(self.relationships as u32))
                .to_string(),
        )?;
        field(
            "Ignored:",
            palette.dim(human_count(self.ignored as u32)).to_string(),
        )?;
        field(
            "Duration:",
            format!(
                "{} {}",
                palette.number(human_count(self.duration_ms as u32)),
                palette.dim("ms")
            ),
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
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        if let Some(file) = &self.file {
            writeln!(out, "{}", palette.path(&file.path))?;
            writeln!(out)?;
            write_list(out, "Depends on", &file.dependencies, palette, true)?;
            write_list(out, "Depended on by", &file.dependents, palette, true)?;
            // Unresolved imports are names, not files in this project, so they
            // are not painted as paths — the reader would go looking for them.
            write_list(
                out,
                "Unresolved imports",
                &file.unresolved_imports,
                palette,
                false,
            )?;
            return Ok(());
        }

        writeln!(out, "{}", palette.path(&self.root))?;
        writeln!(out)?;
        writeln!(
            out,
            "{} {}",
            palette.label("Files: "),
            palette.number(human_count(self.files as u32))
        )?;
        writeln!(
            out,
            "{} {}",
            palette.label("Nodes: "),
            palette.number(human_count(self.nodes as u32))
        )?;
        writeln!(
            out,
            "{} {}",
            palette.label("Edges: "),
            palette.number(human_count(self.edges as u32))
        )?;

        if self.most_depended_on.is_empty() {
            writeln!(out)?;
            return writeln!(out, "No resolved dependencies between files yet.");
        }

        writeln!(out)?;
        writeln!(out, "{}", palette.heading("Most depended on:"))?;
        for entry in &self.most_depended_on {
            writeln!(
                out,
                "  {:<48}{} {}",
                palette.path(&entry.path),
                palette.number(human_count(entry.dependents as u32)),
                palette.dim("dependents")
            )?;
        }
        Ok(())
    }
}

fn write_list(
    out: &mut dyn Write,
    title: &str,
    items: &[String],
    palette: Palette,
    are_paths: bool,
) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(out, "{}", palette.heading(format!("{title}:")))?;
    for item in items {
        if are_paths {
            writeln!(out, "  {}", palette.path(item))?;
        } else {
            writeln!(out, "  {}", palette.symbol(item))?;
        }
    }
    writeln!(out)
}

pub fn index<W: Write>(
    app: &App,
    root: &Path,
    force: bool,
    printer: &mut Printer<W>,
) -> Result<()> {
    let report = perform(app, root, force, printer.format() == OutputFormat::Human)?;
    printer.emit(&report)?;
    Ok(())
}

/// Run an index pass and record what it cost, without reporting it.
///
/// `ctxc init` runs the same pass as part of a longer sequence, and a command
/// that emits one document must not have a second one printed from inside it.
///
/// `show_progress` only asks: a line is drawn when stderr is a terminal, so
/// piped output is unchanged whatever the caller wanted.
pub fn perform(app: &App, root: &Path, force: bool, show_progress: bool) -> Result<IndexReport> {
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
    let progress = TerminalProgress::start(show_progress);
    let report = database
        .transaction(|| {
            let mut indexer = Indexer::new(&store);
            if let Some(embedder) = &embedder {
                indexer = indexer.with_embedder(embedder);
            }
            if let Some(progress) = &progress {
                indexer = indexer.with_progress(progress);
            }
            indexer.index(root, &options)
        })
        .with_context(|| format!("failed to index {}", root.display()))?;

    app.record(index_event(&report, &index::root_key(root), &database));

    Ok(report)
}

/// One rewriting line on stderr, while an index pass runs.
///
/// Only when stderr is a terminal: a redrawn line is control characters and a
/// carriage return, which is exactly what nobody wants in a log file or on the
/// far end of a pipe. Nothing here is on the critical path, so a write that
/// fails is dropped rather than reported.
struct TerminalProgress {
    started: Instant,
    /// Width of the last line drawn, so the next one can cover it completely.
    /// A shorter line over a longer one would otherwise leave its tail behind.
    width: Cell<usize>,
}

impl TerminalProgress {
    /// A reporter, or `None` when nobody is watching.
    fn start(wanted: bool) -> Option<TerminalProgress> {
        (wanted && io::stderr().is_terminal()).then(|| TerminalProgress {
            started: Instant::now(),
            width: Cell::new(0),
        })
    }

    fn draw(&self, line: &str) {
        let padding = self.width.get().saturating_sub(line.chars().count());
        let _ = write!(io::stderr(), "\r{line}{:padding$}", "");
        let _ = io::stderr().flush();
        self.width.set(line.chars().count());
    }
}

impl IndexProgress for TerminalProgress {
    fn advance(&self, seen: u64, indexed: u64) {
        self.draw(&format!(
            "  {} files seen, {} parsed, {:.1}s",
            human_count(seen as u32),
            human_count(indexed as u32),
            self.started.elapsed().as_secs_f64()
        ));
    }

    fn finish(&self) {
        // Clear the line rather than leaving it above the report: the report
        // says the same things, in full.
        let _ = write!(io::stderr(), "\r{:width$}\r", "", width = self.width.get());
        let _ = io::stderr().flush();
    }
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
        .with_hint("run `ctxc project index` first")
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
