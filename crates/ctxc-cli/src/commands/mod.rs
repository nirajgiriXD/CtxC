//! Command implementations.
//!
//! Each command builds a serializable report and hands it to the printer, so
//! human and machine output can never drift apart.

pub mod analyze;
pub mod catalog;
pub mod completions;
pub mod config;
pub mod daemon;
pub mod dashboard;
pub mod doctor;
pub mod index;
pub mod init;
pub mod input;
pub mod integrations;
pub mod mcp;
pub mod metrics;
pub mod optimize;
pub mod project;
pub mod search;
pub mod similar;
pub mod status;
pub mod stop;
pub mod update;
pub mod version;

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;

use crate::app::App;
use crate::cli::{Command, DaemonAction, OptimizeOptions, SearchOptions};
use crate::error::CliError;
use crate::output::Printer;

/// Record a long-running process so `ctxc stop` can find it later.
///
/// Only the commands that outlive their invocation need this: a daemon in the
/// foreground, and an MCP server an agent spawned and may never reap. Failing
/// to record costs a warning and nothing else — the process still runs, it is
/// only harder to stop.
pub(crate) fn record(app: &App, command: &str) -> Option<ctxc_core::processes::Registration> {
    ctxc_core::processes::register(app.paths().data_dir(), command)
        .inspect_err(|err| {
            tracing::warn!(error = %err, "could not record this process; `ctxc stop` will not find it")
        })
        .ok()
}

/// Dispatch a parsed command.
///
/// The first arm of each group is the advertised command; the arms below the
/// divider are the names CtxC answered to before the command tree was grouped,
/// and they route to exactly the same work.
pub fn dispatch<W: Write>(command: &Command, app: &App, printer: &mut Printer<W>) -> Result<()> {
    match command {
        Command::Init {
            path,
            no_index,
            no_agents,
            start,
            yes,
        } => init::run(
            app,
            path.as_ref(),
            *no_index,
            *no_agents,
            *start,
            *yes,
            printer,
        ),

        Command::Optimize {
            inputs,
            dry_run,
            command,
            source,
            options,
        } => optimize_input(
            app,
            inputs,
            *dry_run,
            command,
            source.from.as_deref(),
            options,
            printer,
        ),

        Command::Find {
            query,
            path,
            similar,
            options,
        } => find(app, query, path.as_ref(), *similar, options, printer),

        Command::Project { action } => project::run(app, action, printer),
        Command::Start { detach } => daemon::start(app, *detach, printer),
        Command::Stop { all } => stop::run(app, *all, printer),
        Command::Doctor => doctor::run(app, printer),

        Command::Status {
            daemon: only_daemon,
            metrics: saving,
            options,
        } => {
            if *only_daemon {
                daemon::status(app, printer)
            } else if *saving {
                metrics::run(app, options, printer)
            } else {
                status::run(app, printer)
            }
        }

        Command::Config { action } => config::run(app, action.as_ref(), printer),
        Command::Dashboard { no_open } => dashboard::run(app, !no_open, printer),
        Command::Update { options } => update::run(app, options, printer),
        Command::Completions { shell } => completions::run(*shell, printer),
        Command::Mcp => mcp::run(app, printer),

        // ---- names kept so older scripts keep working ----
        Command::Analyze { input, source } => {
            analyze::run(app, input.as_deref(), source.from.as_deref(), printer)
        }
        Command::Compile { inputs, options } => optimize::compile(app, inputs, options, printer),
        Command::Capture { command, options } => optimize::capture(app, command, options, printer),
        Command::Similar { query, path, limit } => similar::run(
            app,
            query,
            &index::root_or_current(path.as_ref()),
            *limit,
            printer,
        ),
        Command::Retrieve { reference } => search::retrieve(app, reference, printer),
        Command::Index { path, force } => {
            index::index(app, &index::root_or_current(path.as_ref()), *force, printer)
        }
        Command::Graph { path, file, limit } => index::graph(
            app,
            &index::root_or_current(path.as_ref()),
            file.as_deref(),
            *limit,
            printer,
        ),
        Command::Metrics { options } => metrics::run(app, options, printer),
        Command::Integrations { action } => integrations::run(app, action.as_ref(), printer),
        Command::Daemon { action } => match action {
            Some(DaemonAction::Start { detach }) => daemon::start(app, *detach, printer),
            Some(DaemonAction::Stop { all }) => stop::run(app, *all, printer),
            Some(DaemonAction::Status) | None => daemon::status(app, printer),
        },
        Command::Version => version::run(printer),
    }
}

/// Route `ctxc optimize` to the work its arguments describe.
///
/// One command covers four shapes of the same job, because from the outside
/// they are the same job: take input, spend fewer tokens saying it. Which
/// optimizer runs is a detail of where the bytes came from.
fn optimize_input<W: Write>(
    app: &App,
    inputs: &[PathBuf],
    dry_run: bool,
    command: &[String],
    from: Option<&str>,
    options: &OptimizeOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    if !command.is_empty() {
        if !inputs.is_empty() {
            return Err(
                CliError::new("files and a command cannot be optimized together")
                    .with_hint("run `ctxc optimize` once for the files and once for the command")
                    .into(),
            );
        }
        if dry_run {
            return Err(
                CliError::new("--dry-run cannot describe output that does not exist yet")
                    .with_hint("the command has to run before there is anything to measure")
                    .into(),
            );
        }
        return optimize::capture(app, command, options, printer);
    }

    if dry_run {
        if inputs.len() > 1 {
            return Err(CliError::new("--dry-run describes one input at a time")
                .with_hint("pass a single file, or drop --dry-run to compile them all")
                .into());
        }
        return analyze::run(app, inputs.first().map(PathBuf::as_path), from, printer);
    }

    if inputs.len() > 1 {
        return optimize::compile(app, inputs, options, printer);
    }

    optimize::optimize(
        app,
        inputs.first().map(PathBuf::as_path),
        from,
        options,
        printer,
    )
}

/// Route `ctxc find` to the retrieval it describes.
fn find<W: Write>(
    app: &App,
    query: &str,
    path: Option<&PathBuf>,
    similar_only: bool,
    options: &SearchOptions,
    printer: &mut Printer<W>,
) -> Result<()> {
    // A reference is not a question: it names one stored context exactly, so
    // there is nothing to rank and no project to rank it against.
    if query.starts_with(ctxc_core::id::URI_PREFIX) {
        if similar_only {
            return Err(CliError::new("a reference has nothing to compare against")
                .with_hint("drop --similar to recover what the reference points at")
                .into());
        }
        return search::retrieve(app, query, printer);
    }

    let root = index::root_or_current(path);
    if similar_only {
        return similar::run(app, query, &root, options.limit, printer);
    }
    search::search(app, query, &root, options, printer)
}
