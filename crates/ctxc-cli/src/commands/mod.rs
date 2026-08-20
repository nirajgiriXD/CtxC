//! Command implementations.
//!
//! Each command builds a serializable report and hands it to the printer, so
//! human and machine output can never drift apart.

pub mod analyze;
pub mod config;
pub mod daemon;
pub mod dashboard;
pub mod index;
pub mod input;
pub mod integrations;
pub mod mcp;
pub mod metrics;
pub mod optimize;
pub mod project;
pub mod search;
pub mod similar;
pub mod status;
pub mod update;
pub mod version;

use std::io::Write;

use anyhow::Result;

use crate::app::App;
use crate::cli::{Command, DaemonAction};
use crate::output::Printer;

/// Dispatch a parsed command.
pub fn dispatch<W: Write>(command: &Command, app: &App, printer: &mut Printer<W>) -> Result<()> {
    match command {
        Command::Analyze { input, source } => {
            analyze::run(app, input.as_deref(), source.from.as_deref(), printer)
        }
        Command::Optimize {
            input,
            source,
            options,
        } => optimize::optimize(
            app,
            input.as_deref(),
            source.from.as_deref(),
            options,
            printer,
        ),
        Command::Capture { command, options } => optimize::capture(app, command, options, printer),
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
        Command::Search {
            query,
            path,
            options,
        } => search::search(
            app,
            query,
            &index::root_or_current(path.as_ref()),
            options,
            printer,
        ),
        Command::Similar { query, path, limit } => similar::run(
            app,
            query,
            &index::root_or_current(path.as_ref()),
            *limit,
            printer,
        ),
        Command::Retrieve { reference } => search::retrieve(app, reference, printer),
        Command::Compile { inputs, options } => optimize::compile(app, inputs, options, printer),
        Command::Project { action } => project::run(app, action, printer),
        Command::Start { detach } => daemon::start(app, *detach, printer),
        Command::Stop => daemon::stop(app, printer),
        Command::Daemon { action } => match action {
            Some(DaemonAction::Start) => daemon::start(app, true, printer),
            Some(DaemonAction::Stop) => daemon::stop(app, printer),
            Some(DaemonAction::Status) | None => daemon::status(app, printer),
        },
        Command::Integrations { action } => integrations::run(app, action.as_ref(), printer),
        Command::Mcp => mcp::run(app, printer),
        Command::Metrics { options } => metrics::run(app, options, printer),
        Command::Dashboard { no_open } => dashboard::run(app, !no_open, printer),
        Command::Version => version::run(printer),
        Command::Update { options } => update::run(app, options, printer),

        Command::Status => status::run(app, printer),
        Command::Config { action } => config::run(app, action.as_ref(), printer),
    }
}
