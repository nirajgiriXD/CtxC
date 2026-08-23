//! `ctxc mcp`
//!
//! Runs the Model Context Protocol server on stdin and stdout. This is not a
//! command a person types — an agent spawns it and speaks JSON-RPC down the
//! pipe — so it prints nothing, blocks until the client hangs up, and puts
//! every diagnostic on stderr.
//!
//! stdout is the protocol. A single stray line of human-readable output on it
//! corrupts the stream and the client disconnects, which is why this command
//! ignores the printer entirely rather than being careful with it.

use std::io::Write;

use anyhow::{Context as _, Result};

use ctxc_mcp::{Server, Tools};

use crate::app::App;
use crate::output::Printer;

pub fn run<W: Write>(app: &App, _printer: &mut Printer<W>) -> Result<()> {
    let tools = Tools::new(
        app.config.clone(),
        app.database_path(),
        ctxc_mcp::tools::working_directory(),
    );

    // An agent spawned this and may never reap it, so it records itself for
    // `ctxc stop` to find. The guard clears the record when the session ends.
    let _recorded = crate::commands::record(app, "mcp");

    tracing::info!(
        database = %app.database_path().display(),
        "MCP server ready on stdin"
    );

    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    Server::new(tools)
        .serve(stdin, &mut stdout)
        .context("the MCP session ended badly")?;

    tracing::info!("MCP session closed");
    Ok(())
}
