//! `ctxc optimize <input>` and `ctxc compile <input>...`
//!
//! These two commands produce content, not a report, so they split their output
//! deliberately:
//!
//! ```text
//! human  -> optimized content on stdout, summary on stderr
//! json   -> one document on stdout carrying both
//! quiet  -> optimized content on stdout, nothing else
//! ```
//!
//! That keeps `ctxc optimize file | agent` correct in every format while still
//! showing a person what was saved.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_core::optimization::{OptimizationResult, SavingsByStage};
use ctxc_core::{Context, ContextSource, TokenBudget};
use ctxc_engine::{Compilation, Engine};
use ctxc_metrics::{MetricEvent, Operation};
use ctxc_store::{ContextStore, SqliteContextStore};

use crate::app::App;
use crate::cli::OptimizeOptions;
use crate::output::{human_count, human_delta, human_percent, OutputFormat, Printer, Render};

/// Result of optimizing a single input.
#[derive(Debug, Serialize)]
pub struct OptimizeReport {
    pub source: String,
    /// Reference that resolves back to the original content.
    pub reference: String,
    /// Whether the original was kept in the context database.
    pub stored: bool,
    /// Exit code of the captured command, when the input came from one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub content: String,
    pub result: OptimizationResult,
}

impl Render for OptimizeReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        render_summary(out, &Summary::from_result(&self.result))?;
        if let Some(code) = self.exit_code {
            writeln!(out, "Exit code:        {code}")?;
        }
        writeln!(out, "Reference:        {}", self.reference)?;
        if !self.stored {
            writeln!(out, "                  (original not stored)")?;
        }
        Ok(())
    }
}

/// Result of compiling several inputs.
#[derive(Debug, Serialize)]
pub struct CompileReport {
    pub stored: bool,
    #[serde(flatten)]
    pub compilation: Compilation,
}

impl Render for CompileReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        render_summary(out, &Summary::from_compilation(&self.compilation))?;
        writeln!(out)?;
        writeln!(out, "Sections:")?;
        for section in &self.compilation.sections {
            writeln!(
                out,
                "  {:<28}{} -> {} tokens",
                section.source,
                human_count(section.result.original_tokens),
                human_count(section.result.optimized_tokens)
            )?;
        }
        Ok(())
    }
}

/// The numbers common to both commands.
struct Summary {
    original_tokens: u32,
    optimized_tokens: u32,
    reduction_ratio: f64,
    estimated: bool,
    savings: SavingsByStage,
}

impl Summary {
    fn from_result(result: &OptimizationResult) -> Summary {
        Summary {
            original_tokens: result.original_tokens,
            optimized_tokens: result.optimized_tokens,
            reduction_ratio: result.reduction_ratio,
            estimated: result.estimated,
            savings: result.savings_by_stage,
        }
    }

    fn from_compilation(compilation: &Compilation) -> Summary {
        Summary {
            original_tokens: compilation.original_tokens,
            optimized_tokens: compilation.optimized_tokens,
            reduction_ratio: compilation.reduction_ratio,
            estimated: compilation
                .sections
                .first()
                .map(|section| section.result.estimated)
                .unwrap_or(true),
            savings: compilation.savings_by_stage,
        }
    }
}

fn render_summary(out: &mut dyn Write, summary: &Summary) -> io::Result<()> {
    writeln!(
        out,
        "Original tokens:  {}",
        human_count(summary.original_tokens)
    )?;
    writeln!(
        out,
        "Optimized tokens: {}",
        human_count(summary.optimized_tokens)
    )?;
    writeln!(
        out,
        "Reduction:        {}{}",
        human_percent(summary.reduction_ratio),
        if summary.estimated {
            "  (token counts are estimates)"
        } else {
            ""
        }
    )?;

    let savings = &summary.savings;
    if savings.total() != 0 {
        writeln!(out)?;
        for (stage, tokens) in [
            ("filtering", savings.filtering),
            ("deduplication", savings.deduplication),
            ("compression", savings.compression),
            ("selection", savings.selection),
        ] {
            if tokens != 0 {
                writeln!(out, "  {stage:<16}{}", human_delta(tokens))?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

pub fn optimize<W: Write>(
    app: &App,
    input: Option<&Path>,
    from: Option<&str>,
    options: &OptimizeOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    let context = super::input::load_from(input, from)?;
    run_one(app, context, options, printer, None)
}

/// Run a command and optimize what it printed.
pub fn capture<W: Write>(
    app: &App,
    command: &[String],
    options: &OptimizeOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    let (program, args) = command.split_first().expect("clap requires a command");
    let captured = crate::process::capture(program, args)?;

    let context = ctxc_context::ingest::from_text(
        ContextSource::Command {
            command: captured.command_line.clone(),
        },
        &captured.output,
        None,
    );

    run_one(app, context, options, printer, captured.exit_code)
}

/// Optimize one context and report it.
fn run_one<W: Write>(
    app: &App,
    context: Context,
    options: &OptimizeOptions,
    printer: &mut Printer<W>,
    exit_code: Option<i32>,
) -> Result<()> {
    let engine = Engine::from_config(&app.config);
    let started = std::time::Instant::now();
    let optimized = engine
        .optimize(&context, options.budget.map(TokenBudget::new))
        .context("failed to optimize input")?;

    let source = ctxc_context::ingest::label(&context.metadata.source);
    app.record(
        MetricEvent::from_result(Operation::Optimize, &source, &optimized.result)
            .took(started.elapsed()),
    );

    let stored = store_originals(app, std::slice::from_ref(&context), options.no_store)?;

    let report = OptimizeReport {
        source,
        reference: optimized.reference(),
        stored,
        exit_code,
        content: optimized.content,
        result: optimized.result,
    };

    emit(printer, &report, &report.content)
}

pub fn compile<W: Write>(
    app: &App,
    inputs: &[std::path::PathBuf],
    options: &OptimizeOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    let mut contexts = Vec::with_capacity(inputs.len());
    for input in inputs {
        contexts.push(super::input::load(Some(input))?);
    }

    let engine = Engine::from_config(&app.config);
    let started = std::time::Instant::now();
    let compilation = engine
        .compile(&contexts, options.budget.map(TokenBudget::new))
        .context("failed to compile the given inputs")?;

    // One event for the compilation, not one per section: a compilation is the
    // operation someone ran, and its sections are how it did it.
    let mut event = MetricEvent::new(Operation::Compile, format!("{} inputs", contexts.len()))
        .with_tokens(compilation.original_tokens, compilation.optimized_tokens)
        .took(started.elapsed());
    event.savings_by_stage = compilation.savings_by_stage;
    event.estimated = compilation
        .sections
        .first()
        .map_or(true, |section| section.result.estimated);
    app.record(event);

    let stored = store_originals(app, &contexts, options.no_store)?;
    let report = CompileReport {
        stored,
        compilation,
    };

    let content = report.compilation.content.clone();
    emit(printer, &report, &content)
}

/// Send the content and the report to the right streams for the format.
fn emit<W: Write, T: Render>(printer: &mut Printer<W>, report: &T, content: &str) -> Result<()> {
    match printer.format() {
        OutputFormat::Human => {
            printer.write_content(content)?;
            report.render_human(&mut io::stderr())?;
        }
        OutputFormat::Json | OutputFormat::Jsonl => printer.emit(report)?,
        OutputFormat::Quiet => printer.write_content(content)?,
    }
    Ok(())
}

/// Keep the originals so optimized output stays reversible.
///
/// Storage failures are reported rather than swallowed: silently losing the
/// only copy of what was removed would make the reference a lie.
pub(crate) fn store_originals(app: &App, contexts: &[Context], skip: bool) -> Result<bool> {
    if skip {
        return Ok(false);
    }

    let database = app.open_database()?;
    let store = SqliteContextStore::new(&database);
    for context in contexts {
        store
            .save_context(context)
            .with_context(|| format!("failed to store context {}", context.id))?;
    }
    Ok(true)
}
