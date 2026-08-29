//! `ctxc optimize <input>...`, and the capture behind `ctxc optimize -- ...`
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
use ctxc_store::{
    CachedOptimization, ContextStore, OptimizationKey, OptimizationStore, SqliteContextStore,
    SqliteOptimizationStore,
};

use crate::app::App;
use crate::cli::OptimizeOptions;
use crate::output::{human_count, human_delta, human_percent, OutputFormat, Printer, Render};
use crate::style::Palette;

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
    /// True when this document was remembered from an earlier identical run
    /// rather than produced now.
    pub cached: bool,
    pub content: String,
    pub result: OptimizationResult,
}

impl Render for OptimizeReport {
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        render_summary(out, &Summary::from_result(&self.result), palette)?;
        if let Some(code) = self.exit_code {
            let code = if code == 0 {
                palette.good(code).to_string()
            } else {
                palette.bad(code).to_string()
            };
            writeln!(out, "{} {code}", palette.label("Exit code:       "))?;
        }
        writeln!(
            out,
            "{} {}",
            palette.label("Reference:       "),
            palette.reference(&self.reference)
        )?;
        if self.cached {
            writeln!(
                out,
                "                  {}",
                palette.dim("(reused from an identical earlier run)")
            )?;
        }
        if !self.stored {
            writeln!(
                out,
                "                  {}",
                palette.dim("(original not stored)")
            )?;
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
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        render_summary(out, &Summary::from_compilation(&self.compilation), palette)?;
        writeln!(out)?;
        writeln!(out, "{}", palette.heading("Sections:"))?;
        for section in &self.compilation.sections {
            writeln!(
                out,
                "  {:<28}{} {} {} {}",
                palette.path(&section.source),
                palette.number(human_count(section.result.original_tokens)),
                palette.dim("->"),
                palette.number(human_count(section.result.optimized_tokens)),
                palette.dim("tokens")
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

fn render_summary(out: &mut dyn Write, summary: &Summary, palette: Palette) -> io::Result<()> {
    writeln!(
        out,
        "{} {}",
        palette.label("Original tokens: "),
        palette.number(human_count(summary.original_tokens))
    )?;
    writeln!(
        out,
        "{} {}",
        palette.label("Optimized tokens:"),
        palette.number(human_count(summary.optimized_tokens))
    )?;
    writeln!(
        out,
        "{} {}{}",
        palette.label("Reduction:       "),
        palette.good(human_percent(summary.reduction_ratio)),
        if summary.estimated {
            format!("  {}", palette.dim("(token counts are estimates)"))
        } else {
            String::new()
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
                // A stage that costs tokens is not a saving, and the colour is
                // the fastest way to see which kind of line this is.
                let delta = human_delta(tokens);
                let delta = if tokens < 0 {
                    palette.warn(delta).to_string()
                } else {
                    palette.good(delta).to_string()
                };
                writeln!(out, "  {:<16}{delta}", palette.label(stage))?;
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
    let budget = options
        .budget
        .map(TokenBudget::new)
        .unwrap_or_else(|| engine.options().default_budget);
    let started = std::time::Instant::now();

    // Optimization is deterministic, so an identical earlier run is the answer
    // rather than a hint about it. Agents re-run the same commands constantly,
    // which makes this the common path and not the clever one.
    let (optimized, cached) = match reuse(app, &engine, &context, budget, options.no_cache) {
        Some(remembered) => (remembered, true),
        None => {
            let produced = engine
                .optimize(&context, Some(budget))
                .context("failed to optimize input")?;
            remember(app, &engine, &context, budget, &produced, options.no_cache);
            (produced, false)
        }
    };

    let source = ctxc_context::ingest::label(&context.metadata.source);
    app.record(
        MetricEvent::from_result(Operation::Optimize, &source, &optimized.result)
            .took(started.elapsed())
            .with_cache_hit(cached),
    );

    let stored = store_originals(app, std::slice::from_ref(&context), options.no_store)?;

    let report = OptimizeReport {
        source,
        reference: optimized.reference(),
        stored,
        exit_code,
        cached,
        content: optimized.content,
        result: optimized.result,
    };

    emit(printer, &report, &report.content)
}

/// What an identical earlier run produced, if there was one.
///
/// Every failure here is a miss: the work can always be done again, and a
/// cache that can break a command is worse than no cache.
fn reuse(
    app: &App,
    engine: &Engine,
    context: &Context,
    budget: TokenBudget,
    skip: bool,
) -> Option<ctxc_core::OptimizedContext> {
    if skip {
        return None;
    }

    let settings = engine.settings_fingerprint();
    let optimizer = engine.optimizer_for(context);
    let hash = ctxc_core::id::content_hash(context.content.as_bytes());

    let database = app.open_database().ok()?;
    let found = SqliteOptimizationStore::new(&database)
        .cached(OptimizationKey {
            content_hash: &hash,
            optimizer: optimizer.name(),
            budget: budget.total(),
            settings: &settings,
        })
        .inspect_err(|err| tracing::debug!(error = %err, "could not read the optimization cache"))
        .ok()??;

    Some(ctxc_core::OptimizedContext {
        source_id: context.id.clone(),
        content: found.content,
        result: found.result,
    })
}

/// Remember what this run produced, so the next identical one is free.
fn remember(
    app: &App,
    engine: &Engine,
    context: &Context,
    budget: TokenBudget,
    optimized: &ctxc_core::OptimizedContext,
    skip: bool,
) {
    if skip {
        return;
    }

    let settings = engine.settings_fingerprint();
    let optimizer = engine.optimizer_for(context);
    let hash = ctxc_core::id::content_hash(context.content.as_bytes());

    let write = || -> anyhow::Result<()> {
        let database = app.open_database()?;
        SqliteOptimizationStore::new(&database).remember(
            OptimizationKey {
                content_hash: &hash,
                optimizer: optimizer.name(),
                budget: budget.total(),
                settings: &settings,
            },
            &CachedOptimization {
                content: optimized.content.clone(),
                result: optimized.result.clone(),
            },
        )?;
        Ok(())
    };

    // Nothing about the command's own result depends on this having worked.
    if let Err(err) = write() {
        tracing::debug!(error = %err, "could not remember this optimization");
    }
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
            report.render_human(&mut io::stderr(), printer.stderr_palette())?;
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
