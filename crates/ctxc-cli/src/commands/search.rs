//! `ctxc find`, and the reference recovery it also answers.
//!
//! Search answers "what should an agent read to work on this?" — full-text
//! matches, symbol matches, and whatever the dependency graph says those files
//! lean on, ranked together. With `--compile` it stops describing the answer
//! and produces it: the highest-ranked files, optimized, inside a token budget.
//!
//! Retrieve is the other half of reversible optimization: every optimized
//! output carries a `ctxc://context/<id>`, and this is what turns one back into
//! the bytes it came from.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_core::{ContentType, Context, ContextId, ContextSource, TokenBudget};
use ctxc_engine::{index, Engine, ProjectEmbedder};
use ctxc_metrics::{MetricEvent, Operation};
use ctxc_retrieval::{Query, Retrieval, RetrievalOptions, Retriever};
use ctxc_semantic::SemanticOptions;
use ctxc_store::{
    ContextStore, IndexStore, SqliteContextStore, SqliteEmbeddingStore, SqliteIndexStore,
};

use crate::app::App;
use crate::cli::SearchOptions;
use crate::error::CliError;
use crate::output::{human_count, human_percent, OutputFormat, Printer, Render};

impl Render for Retrieval {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.files.is_empty() {
            return writeln!(
                out,
                "Nothing indexed under {} matches {:?}.",
                self.root, self.query
            );
        }

        for file in &self.files {
            let location = match file.line {
                Some(line) => format!("{}:{}", file.path, line),
                None => file.path.clone(),
            };
            writeln!(out, "{location}  ({:.2})", file.score)?;

            if !file.matched_symbols.is_empty() {
                writeln!(out, "  defines  {}", file.matched_symbols.join(", "))?;
            }
            if let ctxc_retrieval::Reason::Related { to, hops } = &file.reason {
                writeln!(out, "  related  {to} ({hops} hop away)")?;
            }
            if let Some(snippet) = &file.snippet {
                for line in snippet.lines() {
                    writeln!(out, "  | {line}")?;
                }
            }
            writeln!(out)?;
        }

        writeln!(
            out,
            "{} of {} candidates shown",
            human_count(self.files.len() as u32),
            human_count(self.considered as u32)
        )
    }
}

/// The context a query selected, assembled and optimized.
#[derive(Debug, Serialize)]
pub struct CompiledContextReport {
    pub query: String,
    /// Whether the cited originals were kept, so the references resolve.
    pub stored: bool,
    pub root: String,
    /// Files chosen, in rank order.
    pub selected: Vec<String>,
    pub original_tokens: u32,
    pub optimized_tokens: u32,
    pub reduction_ratio: f64,
    pub budget: u32,
    pub content: String,
}

impl Render for CompiledContextReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "Query:            {}", self.query)?;
        writeln!(out, "Files selected:   {}", self.selected.len())?;
        for path in &self.selected {
            writeln!(out, "  {path}")?;
        }
        writeln!(out)?;
        writeln!(
            out,
            "Original tokens:  {}",
            human_count(self.original_tokens)
        )?;
        writeln!(
            out,
            "Optimized tokens: {}  of {} budget",
            human_count(self.optimized_tokens),
            human_count(self.budget)
        )?;
        writeln!(
            out,
            "Reduction:        {}",
            human_percent(self.reduction_ratio)
        )
    }
}

/// A context recovered from a reference.
#[derive(Debug, Serialize)]
pub struct RetrieveReport {
    pub id: String,
    pub reference: String,
    pub source: String,
    pub content_type: String,
    pub bytes: u64,
    pub content: String,
}

impl Render for RetrieveReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "Reference:    {}", self.reference)?;
        writeln!(out, "Source:       {}", self.source)?;
        writeln!(out, "Type:         {}", self.content_type)?;
        writeln!(
            out,
            "Size:         {} bytes",
            human_count(self.bytes as u32)
        )
    }
}

pub fn search<W: Write>(
    app: &App,
    query: &str,
    root: &Path,
    options: &SearchOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    let database = app.open_database()?;
    let store = SqliteIndexStore::new(&database);
    let key = index::root_key(root);

    if store.counts(&key)?.files == 0 {
        return Err(
            CliError::new(format!("{} has not been indexed", root.display()))
                .with_hint(format!(
                    "run `ctxc project index {}`, or `ctxc init {}` to set it up",
                    root.display(),
                    root.display()
                ))
                .into(),
        );
    }

    let retrieval_options = RetrievalOptions {
        limit: options.limit,
        ..RetrievalOptions::from_config(&app.config)
    };
    let parsed = Query::parse(query);
    let started = std::time::Instant::now();

    // Embedding similarity is an extra ranking signal, not a different search.
    // It contributes nothing unless `ranking.semantic` is above zero, so
    // turning embeddings on never silently changes what search returns.
    let semantic = SemanticOptions::from_config(&app.config);
    let embeddings = SqliteEmbeddingStore::new(&database);
    let embedder = ProjectEmbedder::from_options(&semantic, &embeddings, &key)?;
    let similarity = embedder.as_ref().map(|embedder| embedder.for_query(query));

    let retriever = Retriever::new(&store, retrieval_options);
    let retriever = match &similarity {
        Some(similarity) => retriever.with_similarity(similarity),
        None => retriever,
    };

    let retrieval = retriever
        .retrieve(&key, &parsed, ctxc_core::Timestamp::now().as_millis())
        .with_context(|| format!("failed to search {key}"))?;

    app.record(
        MetricEvent::new(Operation::Search, query)
            .for_project_opt(super::metrics::project_for_root(&database, &key))
            .took(started.elapsed()),
    );

    if !options.compile {
        let empty = retrieval.files.is_empty();
        printer.emit(&retrieval)?;
        if empty {
            printer.hint(&[
                "Nothing matched. Try:".to_string(),
                format!("  ctxc find {query:?} --similar        rank by similarity instead"),
                format!("  ctxc find {query:?} --limit 50       look further down the ranking"),
                format!(
                    "  ctxc project index {} --force   re-read the code",
                    root.display()
                ),
            ]);
        }
        return Ok(());
    }

    compile_selection(app, &store, &key, root, retrieval, options, printer)
}

/// Turn ranked results into an AI-ready document inside a budget.
fn compile_selection<W: Write>(
    app: &App,
    store: &dyn IndexStore,
    key: &str,
    root: &Path,
    retrieval: Retrieval,
    options: &SearchOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    let budget = TokenBudget::new(options.budget.unwrap_or(app.config.budget.default));
    // The same tokenizer the engine will compile with: selecting files against
    // one count and then compiling against another would fill the budget by
    // one measure and overflow it by the other.
    let tokenizer = app.config.tokenizer();

    // Selection reads content through the index rather than the filesystem, so
    // what gets compiled is exactly what was searched.
    let selected = retrieval.select_within_budget(budget, tokenizer.as_ref(), |path| {
        store.file_content(key, path).ok().flatten()
    });

    let mut contexts = Vec::with_capacity(selected.len());
    for path in &selected {
        let Some(content) = store.file_content(key, path)? else {
            continue;
        };
        contexts.push(Context::new(
            ContextSource::File {
                path: root.join(path),
            },
            ContentType::Code,
            content,
        ));
    }

    if contexts.is_empty() {
        return Err(CliError::new(format!(
            "nothing matching {:?} fits a budget of {} tokens",
            retrieval.query,
            budget.total()
        ))
        .with_hint("raise --budget, or search for something narrower")
        .into());
    }

    let engine = Engine::from_config(&app.config);
    let compilation = engine
        .compile(&contexts, Some(budget))
        .context("failed to compile the selected context")?;

    // The compiled document cites each file by reference, and a reference that
    // resolves to nothing is worse than no reference at all: store the
    // originals so `ctxc find <REFERENCE>` can honour them.
    let stored = super::optimize::store_originals(app, &contexts, options.no_store)?;

    let report = CompiledContextReport {
        stored,
        query: retrieval.query,
        root: key.to_string(),
        selected,
        original_tokens: compilation.original_tokens,
        optimized_tokens: compilation.optimized_tokens,
        reduction_ratio: compilation.reduction_ratio,
        budget: budget.total(),
        content: compilation.content,
    };

    // Like optimize and compile, the product is the content: it goes to stdout
    // so it can be piped, and the summary goes to stderr.
    match printer.format() {
        OutputFormat::Human => {
            printer.write_content(&report.content)?;
            report.render_human(&mut io::stderr())?;
        }
        OutputFormat::Json | OutputFormat::Jsonl => printer.emit(&report)?,
        OutputFormat::Quiet => printer.write_content(&report.content)?,
    }
    Ok(())
}

pub fn retrieve<W: Write>(app: &App, reference: &str, printer: &mut Printer<W>) -> Result<()> {
    let id = ContextId::parse(reference)
        .with_context(|| format!("{reference} is not a context reference"))?;

    let database = app.open_database()?;
    let store = SqliteContextStore::new(&database);

    let context = store
        .get_context(&id)
        .context("failed to read the context database")?
        .ok_or_else(|| {
            CliError::new(format!("no context is stored for {}", id.to_uri())).with_hint(
                "references expire only when the database is cleared; check the id, \
                 or re-run the command that produced it",
            )
        })?;

    app.record(MetricEvent::new(Operation::Retrieve, id.to_string()).with_cache_hit(true));

    let report = RetrieveReport {
        id: id.to_string(),
        reference: context.uri(),
        source: ctxc_context::ingest::label(&context.metadata.source),
        content_type: context.metadata.content_type.as_str().to_string(),
        bytes: context.metadata.byte_len,
        content: context.content,
    };

    match printer.format() {
        OutputFormat::Human => {
            printer.write_content(&report.content)?;
            report.render_human(&mut io::stderr())?;
        }
        OutputFormat::Json | OutputFormat::Jsonl => printer.emit(&report)?,
        OutputFormat::Quiet => printer.write_content(&report.content)?,
    }
    Ok(())
}
