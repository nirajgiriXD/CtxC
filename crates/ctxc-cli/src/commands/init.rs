//! `ctxc init`.
//!
//! Everything a new project needs, in the order it needs it: register, index,
//! tell the agents that are here about CtxC, and offer to leave a daemon
//! running. Each of those is already a command; what was missing was knowing
//! that they exist and what order they go in.
//!
//! Every step is idempotent, so running `ctxc init` twice is a way to check
//! the state of a project rather than a way to damage it.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use ctxc_project::Registry;
use ctxc_store::{IndexStore, SqliteIndexStore, SqliteProjectStore};

use crate::app::App;
use crate::commands::integrations::AppliedChange;
use crate::output::{human_count, OutputFormat, Printer, Render};

/// What one `ctxc init` run set up.
#[derive(Debug, Serialize)]
pub struct InitReport {
    pub path: String,
    pub project: String,
    /// True when this run registered the project, false when it already was.
    pub registered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexSummary>,
    pub agents: Vec<AppliedChange>,
    pub daemon: DaemonOutcome,
}

/// What the index pass found, in the terms `init` cares about.
#[derive(Debug, Serialize)]
pub struct IndexSummary {
    pub files: u64,
    pub symbols: u64,
    pub duration_ms: u64,
}

/// Where the daemon ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonOutcome {
    /// One was already running.
    AlreadyRunning,
    /// This run started one.
    Started,
    /// It was left alone, and `ctxc start` will start it.
    NotStarted,
}

impl Render for InitReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out)?;
        writeln!(out, "{} is set up.", self.project)?;
        writeln!(out)?;
        writeln!(out, "Next:")?;
        writeln!(
            out,
            "  ctxc find \"<question>\"        find the code that answers it"
        )?;
        writeln!(
            out,
            "  ctxc optimize -- cargo test   shrink a command's output"
        )?;
        writeln!(
            out,
            "  ctxc status                   see where things stand"
        )?;
        if self.daemon == DaemonOutcome::NotStarted {
            writeln!(out)?;
            writeln!(
                out,
                "  ctxc start --detach           keep the index up to date as you work"
            )?;
        }
        Ok(())
    }
}

pub fn run<W: Write>(
    app: &App,
    path: Option<&PathBuf>,
    no_index: bool,
    no_agents: bool,
    start: bool,
    yes: bool,
    printer: &mut Printer<W>,
) -> Result<()> {
    let root = super::index::root_or_current(path);
    let human = printer.format() == OutputFormat::Human;

    // Step 1: the registry. Adding a directory that is already registered
    // returns what is there, so this is safe to repeat.
    let database = app.open_database()?;
    let store = SqliteProjectStore::new(&database);
    let registration = Registry::new(&store).add(&root)?;
    let project = registration.project;
    step(
        human,
        1,
        if registration.added {
            format!("registered {}", project.name)
        } else {
            format!("{} is already registered", project.name)
        },
    );

    // Step 2: the index. Without it there is nothing for `ctxc find` to search
    // and nothing for an agent to be pointed at.
    let indexed = if no_index {
        step(human, 2, "skipped indexing (--no-index)".to_string());
        None
    } else {
        step(human, 2, format!("indexing {}...", project.path));
        let report = super::index::perform(app, Path::new(&project.path), false)?;
        let at = ctxc_core::Timestamp::now();
        Registry::new(&store).record_indexed(&project.id, at)?;

        // What the index holds, not what this pass changed. A second `init`
        // re-parses nothing, and reporting that as "0 symbols" would read as
        // an empty index rather than an up-to-date one.
        let counts = SqliteIndexStore::new(&database).counts(&project.path)?;
        step(
            human,
            2,
            format!(
                "indexed {} files, {} symbols in {} ms",
                human_count(counts.files as u32),
                human_count(counts.symbols as u32),
                human_count(report.duration_ms as u32)
            ),
        );
        Some(IndexSummary {
            files: counts.files,
            symbols: counts.symbols,
            duration_ms: report.duration_ms,
        })
    };

    // Step 3: the agents. Only the ones that look like they are in use here —
    // writing into the instruction file of an agent nobody runs is noise.
    let agents = if no_agents {
        step(human, 3, "skipped agent setup (--no-agents)".to_string());
        Vec::new()
    } else {
        let (changes, _) = super::integrations::perform(app, None, Some(&root), true, true)?;
        step(
            human,
            3,
            if changes.is_empty() {
                "no agents detected here".to_string()
            } else {
                format!(
                    "told {} about CtxC",
                    changes
                        .iter()
                        .map(|change| change.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        );
        changes
    };

    // Step 4: the daemon. It is what keeps the index current, so it is worth
    // offering — but starting a background process nobody asked for is not
    // something to do quietly.
    let daemon = if ctxc_daemon::status(app.paths().data_dir())?.is_running() {
        step(human, 4, "the daemon is already running".to_string());
        DaemonOutcome::AlreadyRunning
    } else if start || yes || (human && confirm("Start the CtxC daemon now?")) {
        super::daemon::start_in_background(app)?;
        step(human, 4, "started the daemon".to_string());
        DaemonOutcome::Started
    } else {
        step(human, 4, "left the daemon stopped".to_string());
        DaemonOutcome::NotStarted
    };

    printer.emit(&InitReport {
        path: root.display().to_string(),
        project: project.name,
        registered: registration.added,
        index: indexed,
        agents,
        daemon,
    })?;
    Ok(())
}

/// Report progress while the work happens.
///
/// On stderr, and only for human output: `ctxc --format json init` has to stay
/// one parseable document, and a step line is a diagnostic, not a result.
fn step(human: bool, number: u8, message: String) {
    if human {
        let _ = writeln!(io::stderr(), "  {number}/4  {message}");
    }
}

/// Ask a yes-or-no question, defaulting to no.
///
/// Only asked when someone is there to answer: with no terminal on both ends
/// there is nobody to read the question, and a script that hangs waiting for
/// an answer is worse than one that does slightly less.
fn confirm(question: &str) -> bool {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return false;
    }

    let _ = write!(io::stderr(), "  {question} [y/N] ");
    let _ = io::stderr().flush();

    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Printer;

    fn render(daemon: DaemonOutcome) -> String {
        let report = InitReport {
            path: ".".into(),
            project: "app".into(),
            registered: true,
            index: Some(IndexSummary {
                files: 12,
                symbols: 40,
                duration_ms: 8,
            }),
            agents: Vec::new(),
            daemon,
        };
        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .emit(&report)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn the_summary_ends_with_commands_to_try() {
        let output = render(DaemonOutcome::Started);
        assert!(output.contains("app is set up."), "{output}");
        assert!(output.contains("ctxc find"), "{output}");
        assert!(output.contains("ctxc optimize"), "{output}");
    }

    #[test]
    fn a_stopped_daemon_is_the_only_case_that_suggests_starting_one() {
        assert!(render(DaemonOutcome::NotStarted).contains("ctxc start --detach"));
        assert!(!render(DaemonOutcome::Started).contains("ctxc start --detach"));
        assert!(!render(DaemonOutcome::AlreadyRunning).contains("ctxc start --detach"));
    }
}
