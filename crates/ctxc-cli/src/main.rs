//! CtxC command line entry point.

mod app;
mod cli;
mod commands;
mod error;
mod logging;
mod output;
mod process;
mod style;
mod terminate;

use std::process::ExitCode;

use clap::Parser;

use crate::app::App;
use crate::cli::Cli;
use crate::output::Printer;
use crate::style::{Palette, Stream};

/// Exit code for a failed command. Clap already uses 2 for usage errors.
const FAILURE: u8 = 1;

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init(cli.verbosity());

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let palette = Palette::for_stream(Stream::Stderr, cli.color);
            eprintln!("{}", error::paint(&error::report(&err), palette));
            ExitCode::from(FAILURE)
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let app = App::bootstrap(cli)?;
    let mut printer = Printer::stdout(cli.format, cli.color);

    // Metrics are written after the command, whether or not it succeeded: a
    // failed optimization is exactly the kind of thing worth having counted.
    let outcome = commands::dispatch(&cli.command, &app, &mut printer);
    app.flush_metrics();
    outcome
}
